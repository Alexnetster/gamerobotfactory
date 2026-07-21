import { test, expect } from '@playwright/test'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import WebSocket from 'ws'
import { parseServerMessage } from '../../src/net/protocol'
import { applyServerMessage, createEmptyMirror } from '../../src/state/mirror'
import type { MirrorState } from '../../src/state/mirror'
import { computeRenderRobots } from '../../src/state/interpolation'
import type { TickSnapshot } from '../../src/state/interpolation'
import { gridToScreen, RENDER_SCALE } from '../../src/render/projection'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

function backendPort(): number {
  const info = JSON.parse(readFileSync(path.resolve(__dirname, '.server-info.json'), 'utf-8')) as { port: number }
  return info.port
}

// globalSetup은 파일당 한 번이 아니라 이 테스트 실행(invocation) 전체에
// 걸쳐 딱 한 번만 뜬다 — 서로 다른 spec 파일들은 서로 다른 워커 프로세스에서
// 동시에 실행되더라도 전부 같은 서버 인스턴스/상태를 공유한다(실측: 임시로
// 두 번째 spec 파일을 추가해 Playwright를 돌려서 직접 확인함). 서버의
// 로봇 수는 커넥션을 넘어 서버 프로세스에 살아있는 전역 상태라, 앞선
// 테스트(같은 파일이든 다른 파일이든)가 로봇을 스폰해 두면 다음 테스트가
// 접속했을 때부터 이미 그 로봇이 보인다. 그래서 "+" 클릭 후 카운트를
// 고정값 '1'로 단언하면 실행 순서에 따라 우연히 통과(혹은 실패)하는
// 깨지기 쉬운 테스트가 된다 (실측: 두 번째 테스트가 첫 테스트가 남긴 로봇
// 때문에 시작부터 이미 1이었고, 클릭 이후 실제로는 2가 되어야 정상인데도
// 타이밍에 따라 assertion이 '1'을 잡아버려 클릭이 아무 효과가 없어도
// 통과할 뻔한 적이 있었다). 클릭 전 카운트를 읽어 "정확히 +1 됐는지"를
// 검증하면 이전 테스트의 잔여 상태와 무관하게 결정적이고, 클릭이 실제로
// 효과를 냈다는 것도 확실히 검증된다.
//
// 주의: 앞으로 이 디렉토리에 다른 e2e spec 파일을 추가할 경우, 그 파일도
// 같은 서버 프로세스/같은 로봇 카운터를 공유하게 된다. 로봇 수를 변경하는
// (SetRobotCount 등) 새 spec을 추가하려면, 이 파일과 마찬가지로 "절대값"이
// 아니라 "호출 전후 델타"로 검증하거나, 이 파일의 상태와 명시적으로
// 조율해야 한다 — 그렇지 않으면 서로 다른 파일의 동시 실행이 같은
// 서버측 카운터를 두고 경쟁해 간헐적으로만 재현되는 flaky 테스트가 된다.
async function currentRobotCount(page: import('@playwright/test').Page): Promise<number> {
  const text = await page.locator('.robot-count').textContent()
  return Number(text)
}

// Plan 5 Task 7(`server/src/sim.rs`의 로봇 순찰 목표 배정, `59aa06b`) 이후로는
// 로봇이 스폰 직후(goal==pos)부터 곧바로 순찰 이동을 시작한다. 게다가
// `client/src/state/interpolation.ts`의 `computeRenderRobots`가 매 애니메이션
// 프레임마다 이전 틱→현재 틱 위치를 계속 lerp하기 때문에, 로봇의 화면 좌표는
// 딱 멈춰 있는 스냅샷이 아니라 "그리는 순간에도 계속 미끄러지는" 값이다
// (한 칸 = TILE_WIDTH/HEIGHT 기준 화면 이동거리 약 35.8px를
// TICK_DURATION_MS=50ms 동안 이동 — 초당 대략 700px/s에 해당하는 속도).
// 아래 두 테스트는 이 전제 때문에 "로봇은 항상 씬 원점에 그려진다"던 예전
// 가정이 깨진 뒤 실제로 여러 번 재발한 flaky 테스트다 — 그 조사 과정을
// 기록해 둔다(다음에 또 건드릴 사람을 위해).
//
// (1차 재발, Task 13 회귀 이후) 실측: canvas를 스캔해 로봇 몸체 좌표를
// "찾아낸" 뒤 그 좌표를 들고 별도의 `page.evaluate`/`page.mouse.click` 호출로
// "그 좌표를 다시 쓰는" 방식은, 두 호출 사이에 몇~수십 ms의 실제 지연이
// 생기고 그 사이 로봇이 계속 이동하기 때문에 깨지기 쉽다 — 8회 연속 실행 중
// 2회 재현(avgR(59~75) < avgG(83~89): 이미 로봇이 떠난 빈 바닥 칸을 읽음,
// 클릭 판정 반경 24px 밖으로 이동해 5회 재시도 내내 선택 실패). "찾기"와
// "쓰기"를 한 evaluate 호출 안에서 원자적으로 처리(같은 getImageData 결과로
// 위치와 색/클릭을 동시에 끝냄)하도록 고쳤다.
//
// (2차 재발) 그런데도 클릭 테스트만 10회 중 3~5회로 여전히 실패했다 —
// 원자적으로 처리했는데도 클릭 직후 동기적으로 읽은 `.selected-robot-panel`이
// 이미 "선택된 로봇 없음"이었다(재시도 사이 지연이 아니라 그 클릭 자체가
// 이미 빗나갔다는 뜻). main.ts의 click 핸들러가 그 순간 `Math.hypot`으로
// 계산하는 실제 거리를 몽키패치로 직접 찍어보니, 가장 가까운 로봇조차 28~90px
// 떨어져 있었다(24px 임계값을 크게 초과) — 그 크기(로봇이 한 틱에 이동하는
// ~35.8px의 배수)로 보아, 우리가 `getImageData()`로 읽는 캔버스 내용 자체가
// "지금 진짜 상태"보다 뒤처져 있었다는 뜻이다. `requestAnimationFrame`으로
// 우리 콜백을 `frame()`(main.ts의 렌더 루프)과 같은 프레임 배치에 맞춰
// 실행되도록 정렬해도(이론상 그러면 두 콜백의 `performance.now()`가 거의
// 같아져야 함) 개선되지 않았다(오히려 10회 중 5회로 더 나빠짐) — 즉 지연의
// 원인이 우리 쪽 JS 타이밍이 아니라 그보다 아래 레이어(헤드리스 크로미움의
// GPU/컴포지터 리드백 등, `getImageData()`가 반환하는 프레임 버퍼 자체가
// "논리적으로 계산된 현재 상태"보다 뒤처지는 문제)에 있다는 뜻이다. 이건
// 아무리 JS 레벨에서 "스캔"과 "클릭"을 원자적으로 묶어도 고칠 수 없다.
//
// 그래서 캔버스 픽셀 스캔 자체를 클릭 테스트에서 버렸다. 대신
// `tests/integration/session.test.ts`와 같은 패턴으로 이 테스트 전용의
// 진짜 WS 연결을 하나 더 열어, 서버가 브로드캐스트하는 권위 있는
// Snapshot/Delta를 `client/src/state/mirror.ts`(main.ts와 동일 모듈)로
// 그대로 미러링하고, `client/src/state/interpolation.ts`의
// `computeRenderRobots` + `client/src/render/projection.ts`의 `gridToScreen`
// (역시 main.ts·canvas.ts가 실제로 쓰는 바로 그 함수들)으로 "지금 이 순간"의
// 화면 좌표를 직접 계산해 클릭한다. 우리 WS 연결과 페이지의 WS 연결은 같은
// 로컬호스트 서버로부터 사실상 동시에 같은 브로드캐스트를 받으므로, 각자
// 자신의 `performance.now()`로 잰 "메시지 수신 후 경과 시간"은 서로 잘
// 들어맞는다 — 캔버스가 실제로 뭘 그렸는지와 무관하게 항상 정확하다.
//
// 첫 번째 테스트(몸체색 평균 검증)는 여전히 캔버스 픽셀 스캔을 쓴다 — 이
// 테스트는 "그 순간의" 위치와 "그 순간의" 색을 같은 evaluate 호출 안에서
// 함께 얻으므로(재사용 지연이 없음) 위 문제의 영향을 받지 않고, 10회 연속
// 실행에서 안정적으로 통과했다(아래 기록 참고). 다만 위치를 찾는 로직
// 자체(래스터 스캔에서 처음 만난 색-일치 픽셀 하나만 믿고 그 주변 좁은 창을
// 평균 내는 것)는 별개의 취약점이었다 — 다른 로봇의 몸체 가장자리나 바닥
// 타일 경계의 앤티에일리어싱이 우연히 isBodyColor 조건을 몇 픽셀만 만족시킬
// 수 있고, 그 노이즈가 래스터 순서상 실제 몸체보다 먼저 걸리면 시드 자체가
// 잘못된 위치가 된다. 이를 색-일치 픽셀 전체를 4-연결 성분으로 묶어 가장 큰
// 덩어리(실제 몸체 사각형 26x20=520px)의 무게중심을 쓰는 방식으로
// 대체했다 — 노이즈는 최소 크기 기준(MIN_BODY_PIXELS)에 못 미쳐 걸러진다.
//
// isBodyColor는 canvas.ts::drawRobot의 새 몸체 그라디언트(#6b7480~#5a636e,
// 슬레이트 그레이 — 2026-07-21 로봇 외형 리디자인)와 대조 확인한다. 팔
// 스트로크(#8b95a0)나 다리(#454c54)는 R/G/B가 이 범위 밖이라 안 걸린다.
const MIN_BODY_PIXELS = 80

test.describe('client renders against a real server', () => {
  test('draws a spawned robot with body-colored pixels somewhere on the canvas', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const before = await currentRobotCount(page)
    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

    // 로봇이 순찰 이동 중이라 스폰 직후 첫 프레임엔 아직 렌더링이 반영 안 됐을
    // 수 있다 — 몇 번 재시도해서 몸체 픽셀이 나타날 때까지 기다린다. 위치와
    // 색 평균을 한 evaluate 호출 안에서 함께 계산하므로(아래), 재시도 사이의
    // 지연은 문제가 안 된다 — 매번 "그 순간의" 캔버스를 한 번만 읽어 위치와
    // 색을 함께 뽑아내기 때문이다.
    let result: { avgR: number; avgG: number } | null = null
    for (let attempt = 0; attempt < 20 && !result; attempt++) {
      result = await page.evaluate((minBodyPixels) => {
        const c = document.querySelector('canvas') as HTMLCanvasElement
        const ctx = c.getContext('2d')!
        const { width, height } = c
        const data = ctx.getImageData(0, 0, width, height).data
        // b >= r는 슬레이트 그레이 그라디언트의 의도(B가 항상 R 이상)를
        // 표현하려고 남겨뒀지만, 실측 뮤테이션 테스트(이 클로저에서 해당
        // 절만 제거하고 5회 연속 실행) 결과 이 테스트를 통과/실패시키는 데
        // 실제로는 기여하지 않는다 — 나머지 R/G/B 범위 경계만으로 이미
        // 다리(#454c54)와 팔(#8b95a0) 색을 걸러내기 충분하기 때문. 안전망
        // 차원에서 남겨두는 것이니, 나중에 지워도 안전하다고 가정하지 말고
        // 지우기 전에 뮤테이션 테스트를 다시 돌려볼 것.
        const isBodyColor = (r: number, g: number, b: number) =>
          r >= 80 && r <= 130 && g >= 90 && g <= 140 && b >= 95 && b <= 145 && b >= r
        const matches = (x: number, y: number): boolean => {
          const i = (y * width + x) * 4
          return isBodyColor(data[i], data[i + 1], data[i + 2])
        }

        const visited = new Uint8Array(width * height)
        let best: { sumX: number; sumY: number; count: number } | null = null

        for (let y = 0; y < height; y++) {
          for (let x = 0; x < width; x++) {
            const startIdx = y * width + x
            if (visited[startIdx]) continue
            visited[startIdx] = 1
            if (!matches(x, y)) continue

            // 이 픽셀에서 시작하는 연결 성분 전체를 반복적 flood fill로 훑는다
            // (재귀는 큰 성분에서 스택 오버플로 위험이 있어 명시적 스택 사용).
            let sumX = 0
            let sumY = 0
            let count = 0
            const stackX: number[] = [x]
            const stackY: number[] = [y]
            while (stackX.length > 0) {
              const cx = stackX.pop()!
              const cy = stackY.pop()!
              sumX += cx
              sumY += cy
              count += 1
              const neighbors: Array<[number, number]> = [
                [cx - 1, cy], [cx + 1, cy], [cx, cy - 1], [cx, cy + 1],
              ]
              for (const [nx, ny] of neighbors) {
                if (nx < 0 || nx >= width || ny < 0 || ny >= height) continue
                const nIdx = ny * width + nx
                if (visited[nIdx]) continue
                visited[nIdx] = 1
                if (!matches(nx, ny)) continue
                stackX.push(nx)
                stackY.push(ny)
              }
            }

            if (!best || count > best.count) {
              best = { sumX, sumY, count }
            }
          }
        }

        if (!best || best.count < minBodyPixels) return null
        const cx = best.sumX / best.count
        const cy = best.sumY / best.count

        // 같은 data 버퍼에서 바로 몸체 사각형(26x20) 크기 박스의 색을
        // 평균 낸다 — 위치를 반환한 뒤 별도 호출로 색을 "다시" 읽으면 그
        // 사이 로봇이 이동해 버려 엉뚱한 빈 칸을 읽을 수 있다(위 파일 상단
        // 주석 참고, 실측 재현됨).
        const w = 26
        const h = 20
        const x0 = Math.max(0, Math.round(cx - w / 2))
        const y0 = Math.max(0, Math.round(cy - h / 2))
        let sumR = 0
        let sumG = 0
        let pixelCount = 0
        for (let y = y0; y < Math.min(height, y0 + h); y++) {
          for (let x = x0; x < Math.min(width, x0 + w); x++) {
            const i = (y * width + x) * 4
            sumR += data[i]
            sumG += data[i + 1]
            pixelCount += 1
          }
        }
        return { avgR: sumR / pixelCount, avgG: sumG / pixelCount }
      }, MIN_BODY_PIXELS)

      if (!result) await page.waitForTimeout(100)
    }
    if (!result) throw new Error('20회 재시도 후에도 canvas에서 로봇 몸체 픽셀을 찾지 못함')

    // 몸체 채우기 색(그라디언트: #6b7480(R107,G116,B128) ~ #5a636e(R90,G99,B110),
    // 슬레이트 그레이로 B가 R보다 항상 크거나 같음)이 26x20 박스 평균에
    // 지배적으로 반영되는지 확인. R 범위 하나만 보면 다른 회색 톤과
    // 겹칠 여지가 있어, 그라디언트의 G 채널(약 99~116)도 별도로 확인해
    // 판별력을 보강한다.
    expect(result.avgR).toBeGreaterThan(70)
    expect(result.avgR).toBeLessThan(140)
    expect(result.avgG).toBeGreaterThan(95)
    expect(result.avgG).toBeLessThan(135)
  })

  test('shows the selected robot info in the sidebar after clicking it', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })

    // main.ts와 완전히 같은 프로토콜/미러/보간 모듈을 재사용하는 관찰 전용
    // WS 연결 — 서버 권위 위치를 캔버스 대신 직접 계산하는 이유는 파일 상단
    // 주석 참고.
    const ws = new WebSocket(`ws://127.0.0.1:${backendPort()}/ws`)
    await new Promise<void>((resolve, reject) => {
      ws.once('open', () => resolve())
      ws.once('error', reject)
    })

    let mirror: MirrorState = createEmptyMirror()
    let prevSnapshot: TickSnapshot | null = null
    let currSnapshot: TickSnapshot | null = null
    ws.on('message', (data: Buffer) => {
      const message = parseServerMessage(data.toString())
      if (!message) return
      mirror = applyServerMessage(mirror, message)
      if (message.kind === 'Snapshot' || message.kind === 'Delta') {
        prevSnapshot = currSnapshot
        currSnapshot = { mirror, receivedAtMs: performance.now() }
      }
    })

    try {
      await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

      const before = await currentRobotCount(page)
      const incButton = page.locator('.sidebar button', { hasText: '+' })
      await incButton.click()
      await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

      // 우리 mirror도 서버로부터 같은 브로드캐스트를 받아 곧 같은 로봇 수를
      // 반영한다 — 페이지 쪽 DOM 갱신과 별개로, 우리 쪽 WS 연결이 따라잡을
      // 때까지 기다린다.
      const deadline = Date.now() + 5000
      while (mirror.robots.size < before + 1 && Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, 20))
      }
      expect(mirror.robots.size).toBeGreaterThanOrEqual(before + 1)

      const canvas = page.locator('canvas')
      const canvasBox = await canvas.boundingBox()
      if (!canvasBox) throw new Error('canvas has no bounding box')
      // canvas.width(비트맵 폭)는 canvas.clientWidth와 1:1이므로(main.ts
      // resizeCanvas, devicePixelRatio 스케일 없음) canvasBox.width로 대신
      // 써도 되지만, 그 가정이 나중에 깨지는 일을 막기 위해 실제 값을 한 번
      // 읽어 둔다(로봇이 움직이는 것과 무관한, 시간에 안 민감한 값이라 별도
      // 호출로 읽어도 안전하다).
      const canvasWidth = await page.evaluate(() => (document.querySelector('canvas') as HTMLCanvasElement).width)

      // 서버 브로드캐스트를 받는 순간과 클릭을 계산하는 순간 사이에는
      // (page.mouse.click의 CDP 왕복 정도의) 아주 작은 지연만 남는다 — 로봇
      // 속도(~700px/s) 기준으로도 무시할 수 있는 수준이라 재시도는 "아직
      // 로봇이 하나도 안 보임" 같은 초기 타이밍만 대비하면 된다.
      let selected = false
      for (let attempt = 0; attempt < 10 && !selected; attempt++) {
        if (!currSnapshot) {
          await new Promise((resolve) => setTimeout(resolve, 50))
          continue
        }
        const rendered = computeRenderRobots(prevSnapshot, currSnapshot, performance.now())
        const target = rendered[0]
        if (!target) {
          await new Promise((resolve) => setTimeout(resolve, 50))
          continue
        }
        const screen = gridToScreen(target.renderPos.x, target.renderPos.y)
        // canvas.ts::drawScene의 `ctx.translate(canvasWidth / 2, 40)` 다음
        // `ctx.scale(RENDER_SCALE)`과 대칭되는 역연산 — gridToScreen은
        // 미확대(unscaled) 좌표를 돌려주므로, 실제 화면에 렌더링된 위치를
        // 클릭하려면 RENDER_SCALE을 곱해줘야 한다.
        const pageX = canvasBox.x + canvasWidth / 2 + screen.x * RENDER_SCALE
        const pageY = canvasBox.y + 40 + screen.y * RENDER_SCALE
        await page.mouse.click(pageX, pageY)
        try {
          await expect(page.locator('.selected-robot-panel')).toContainText('로봇 #', { timeout: 1000 })
          selected = true
        } catch {
          // 클릭 순간과 로봇의 실제 위치가 어긋났을 수 있음 — 다음 시도로 재계산
        }
      }
      expect(selected).toBe(true)
    } finally {
      ws.close()
    }
  })

  test('renders a cargo icon on a robot after it completes a pickup', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const before = await currentRobotCount(page)
    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

    // 컨베이어는 서버 기본값으로 이미 켜져 있다 — 작업 사이클이 자동으로
    // 시작돼 픽업을 완료하면 화물을 든다. 이동 거리 + PICK_TICKS(20틱,
    // 약 1초) 감안해 8초 동안 재시도.
    const cargoColor = { r: 0xc9, g: 0x76, b: 0x2f }
    const hasCargoColor = async () => page.evaluate((color) => {
      const c = document.querySelector('canvas') as HTMLCanvasElement
      const ctx = c.getContext('2d')!
      const { width, height } = c
      const data = ctx.getImageData(0, 0, width, height).data
      for (let i = 0; i < data.length; i += 4) {
        if (Math.abs(data[i] - color.r) < 10 && Math.abs(data[i + 1] - color.g) < 10 && Math.abs(data[i + 2] - color.b) < 10) {
          return true
        }
      }
      return false
    }, cargoColor)

    // 화물이 처음부터 항상 그려지는 회귀(robot.carrying 가드가 빠지거나
    // 뒤집히는 경우)를 잡기 위해, 픽업이 끝나기 전에는 화물 아이콘이
    // 아직 안 보이는지부터 확인한다. 스폰 직후(PICK_TICKS=20틱, 약 1초
    // 미만 경과) 시점이라 이 시점엔 아직 픽업이 끝났을 리 없다.
    //
    // `.robot-count` 텍스트는 사이드바 DOM 갱신(서버 브로드캐스트 수신 시
    // 바로 반영)만으로 확정되고, 캔버스는 별도의 requestAnimationFrame
    // 루프가 그 다음 프레임에 그린다 — 그래서 텍스트 확인 직후 곧바로
    // `hasCargoColor()`를 호출하면 새로 스폰된 로봇이 아직 캔버스에 한 번도
    // 그려지기 전이라 화물 유무와 무관하게 항상 false가 나오는 레이스가
    // 있었다(뮤테이션 `if (true || robot.carrying)`으로 실측: 3회 중 1회
    // 이 경합 때문에 뮤턴트를 못 잡고 조용히 통과함). 최소 두 번의 실제
    // 페인트가 끝난 뒤 상태를 읽도록 두 번의 `requestAnimationFrame`을
    // 명시적으로 기다려서, "아직 안 그려짐"이 아니라 "그려졌는데 화물이
    // 없음"을 검증하게 고쳤다.
    await page.evaluate(() => new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()))
    }))
    expect(await hasCargoColor()).toBe(false)

    let found = false
    for (let attempt = 0; attempt < 40 && !found; attempt++) {
      found = await hasCargoColor()
      if (!found) await page.waitForTimeout(200)
    }
    expect(found).toBe(true)
  })
})
