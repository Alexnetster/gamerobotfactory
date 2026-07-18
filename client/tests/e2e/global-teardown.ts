import { readFileSync, rmSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const INFO_PATH = path.resolve(__dirname, '.server-info.json')

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

export default async function globalTeardown(): Promise<void> {
  const info = JSON.parse(readFileSync(INFO_PATH, 'utf-8')) as { port: number; pid: number; dbDir: string }
  try {
    process.kill(info.pid)
  } catch {
    // 이미 종료된 경우 무시
  }

  // globalSetup은 별도 프로세스라 실제 ChildProcess 핸들(exit 이벤트)이
  // 여기까지 넘어오지 않는다 — PID만 남아있으므로 kill()이 비동기로
  // 처리되는 동안(Windows EBUSY) 잠시 대기한 뒤 삭제를 재시도한다. 실측 결과
  // sqlite 파일 자체는 금방 지워지지만, 마지막 남는 빈 디렉토리 노드는
  // (백신 실시간 검사 등으로 추정되는) 핸들 때문에 3초 정도로는 안 풀리는
  // 경우가 있어 재시도 총 시간을 넉넉히 잡는다.
  const maxAttempts = 20
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    await sleep(500)
    try {
      rmSync(info.dbDir, { recursive: true, force: true })
      break
    } catch {
      // 정리 실패는 무시 — 테스트 결과에 영향 주지 않는다. 마지막 시도까지
      // 실패해도 teardown 자체를 실패시키지 않는다.
    }
  }

  try {
    rmSync(INFO_PATH, { force: true })
  } catch {
    // 무시
  }
}
