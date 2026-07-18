import { spawn } from 'node:child_process'
import type { ChildProcess } from 'node:child_process'
import { createInterface } from 'node:readline'
import path from 'node:path'
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'

export interface SpawnedServer {
  process: ChildProcess
  port: number
}

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

  return { process: child, port }
}

export function stopServer(server: SpawnedServer): void {
  server.process.kill()
}
