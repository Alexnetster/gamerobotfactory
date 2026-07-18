import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { createInterface } from 'node:readline'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'

export interface SpawnedServer {
  process: ChildProcess
  port: number
  dbDir: string
}

// vitest(vite-node)와 Playwright(순수 ESM 로더) 양쪽에서 다 쓰이는데,
// Playwright 쪽은 CJS `__dirname` 셈을 넣어주지 않으므로 `import.meta.url`
// 기반으로 직접 계산한다.
const __dirname = path.dirname(fileURLToPath(import.meta.url))

function resolveServerBinaryPath(): string {
  const exeName = process.platform === 'win32' ? 'server.exe' : 'server'
  return path.resolve(__dirname, '../../../server/target/debug', exeName)
}

/** 서버 바이너리를 임의 포트로 띄우고, 표준출력의 `LISTENING_PORT={port}`
 * 줄에서 실제 포트를 읽는다. 테스트마다 격리된 임시 SQLite 경로를 써서
 * 병렬로 돌려도 서로 간섭하지 않는다. */
export async function spawnServer(): Promise<SpawnedServer> {
  const dbDir = mkdtempSync(path.join(tmpdir(), 'gamerobotfactory-client-test-'))
  const dbPath = path.join(dbDir, 'test.sqlite3')

  const child = spawn(resolveServerBinaryPath(), [], {
    env: { ...process.env, GAMEROBOTFACTORY_DB_PATH: dbPath },
    stdio: ['ignore', 'pipe', 'ignore'],
  })

  const port = await new Promise<number>((resolve, reject) => {
    if (!child.stdout) {
      reject(new Error('server stdout was not piped'))
      return
    }
    const rl = createInterface({ input: child.stdout })
    const timeout = setTimeout(() => {
      rl.close()
      reject(new Error('timed out waiting for LISTENING_PORT announce line'))
    }, 10000)
    rl.on('line', (line) => {
      const match = /^LISTENING_PORT=(\d+)$/.exec(line.trim())
      if (match) {
        clearTimeout(timeout)
        rl.close()
        resolve(Number(match[1]))
      }
    })
    child.on('error', (err) => {
      clearTimeout(timeout)
      reject(err)
    })
  })

  return { process: child, port, dbDir }
}

/** 서버 프로세스를 종료하고 임시 SQLite 디렉토리를 정리한다.
 * Windows에서는 프로세스가 실제로 종료되어 파일 핸들을 놓기까지
 * `kill()` 호출 이후에도 시간이 걸리므로(EBUSY), exit 이벤트를 기다린 뒤
 * rmSync를 짧은 간격으로 재시도한다. 정리 실패는 테스트 결과에 영향을
 * 주지 않도록 최종적으로도 무시한다. */
export async function stopServer(server: SpawnedServer): Promise<void> {
  await new Promise<void>((resolve) => {
    if (server.process.exitCode !== null || server.process.signalCode !== null) {
      resolve()
      return
    }
    server.process.once('exit', () => resolve())
    server.process.kill()
  })

  const maxAttempts = 10
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    try {
      rmSync(server.dbDir, { recursive: true, force: true })
      return
    } catch {
      if (attempt === maxAttempts - 1) {
        // 정리 실패는 무시 — 테스트 결과에 영향 주지 않는다
        return
      }
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
  }
}
