import { test, expect } from '@playwright/test'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

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
// 로봇이 스폰 직후(goal==pos)부터 곧바로 순찰 이동을 시작하므로, "로봇은 항상
// 씬 원점(스폰 좌표)에 그려진다"는 이 두 테스트의 예전 전제가 깨졌다 — 실제로
// Task 10 전체 회귀 검증 중 `npm run test:e2e`를 다시 돌려서 두 테스트 모두
// 실패하는 것으로 직접 재현했다(1번은 원점 픽셀 평균이 R<G로 나옴 — 로봇이
// 이미 다른 칸으로 이동해 그 자리가 비어 있었음, 2번은 클릭 판정 반경 24px
// 안에 아무 로봇도 없어 선택이 아예 안 됨). `client/scripts/record-demo.mjs`가
// 같은 문제(Task 8)를 캔버스 픽셀 스캔으로 해결한 것과 동일한 접근을 여기서도
// 쓴다 — 로봇이 실제로 어디에 있든 몸체 그라디언트 색(#ffd27a~#d99a2e, R>190
// && G∈[130,225] && B<140 — canvas.ts::drawRobot의 bodyGradient와 대조 확인,
// 팔 스트로크 색 #a06f1a는 R=160<190이라 이 임계값에 안 걸림)이 실제로 그려진
// 화면 좌표를 스캔해서 찾는다.
async function locateRobotBodyPixel(
  page: import('@playwright/test').Page,
): Promise<{ x: number; y: number } | null> {
  return page.evaluate(() => {
    const c = document.querySelector('canvas') as HTMLCanvasElement
    const ctx = c.getContext('2d')!
    const { width, height } = c
    const data = ctx.getImageData(0, 0, width, height).data
    const isBodyColor = (r: number, g: number, b: number) => r > 190 && g >= 130 && g <= 225 && b < 140

    let seed: { x: number; y: number } | null = null
    for (let y = 0; y < height && !seed; y++) {
      for (let x = 0; x < width; x++) {
        const i = (y * width + x) * 4
        if (isBodyColor(data[i], data[i + 1], data[i + 2])) {
          seed = { x, y }
          break
        }
      }
    }
    if (!seed) return null

    // seed 주변 좁은 창(몸체 22x16보다 약간 넓은 ±15px)만 재스캔해 무게중심을
    // 구한다 — 여러 로봇이 화면에 있어도 이 창 안에는 보통 한 로봇의 몸체
    // 픽셀만 있으므로 평균이 로봇 사이에서 흐트러지지 않는다.
    let sumX = 0
    let sumY = 0
    let count = 0
    const w = 15
    for (let y = Math.max(0, seed.y - w); y < Math.min(height, seed.y + w); y++) {
      for (let x = Math.max(0, seed.x - w); x < Math.min(width, seed.x + w); x++) {
        const i = (y * width + x) * 4
        if (isBodyColor(data[i], data[i + 1], data[i + 2])) {
          sumX += x
          sumY += y
          count += 1
        }
      }
    }
    return { x: sumX / count, y: sumY / count }
  })
}

// 로봇이 순찰 이동 중이라 스폰 직후 첫 프레임엔 아직 렌더링이 반영 안 됐을 수
// 있다 — 몇 번 재시도해서 몸체 픽셀이 나타날 때까지 기다린다.
async function waitForRobotBodyPixel(
  page: import('@playwright/test').Page,
): Promise<{ x: number; y: number }> {
  for (let attempt = 0; attempt < 20; attempt++) {
    const point = await locateRobotBodyPixel(page)
    if (point) return point
    await page.waitForTimeout(100)
  }
  throw new Error('20회 재시도 후에도 canvas에서 로봇 몸체 픽셀을 찾지 못함')
}

test.describe('client renders against a real server', () => {
  test('draws a spawned robot with body-colored pixels somewhere on the canvas', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const before = await currentRobotCount(page)
    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

    const point = await waitForRobotBodyPixel(page)

    // 몸체 사각형(22x16) 크기의 박스를 실제로 찾은 좌표 중심으로 평균 내면,
    // 얇은 다리/팔 선(전체 면적의 일부)의 영향이 희석되고 실제 몸체 채우기 색
    // (그라디언트: #ffd27a ~ #d99a2e, R>G)이 지배적으로 반영된다 — 단일
    // 픽셀만 보면 다리/팔 스트로크와 우연히 겹쳐 공허해질 수 있음(Task 13에서
    // 실제로 겪은 문제, 위 주석 참고).
    const avg = await page.evaluate(
      ({ x, y, w, h }) => {
        const c = document.querySelector('canvas') as HTMLCanvasElement
        const ctx = c.getContext('2d')!
        const data = ctx.getImageData(Math.max(0, x - w / 2), Math.max(0, y - h / 2), w, h).data
        let sumR = 0
        let sumG = 0
        const pixelCount = data.length / 4
        for (let i = 0; i < data.length; i += 4) {
          sumR += data[i]
          sumG += data[i + 1]
        }
        return { avgR: sumR / pixelCount, avgG: sumG / pixelCount }
      },
      { x: point.x, y: point.y, w: 22, h: 16 },
    )

    expect(avg.avgR).toBeGreaterThan(avg.avgG)
  })

  test('shows the selected robot info in the sidebar after clicking it', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const before = await currentRobotCount(page)
    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

    const canvas = page.locator('canvas')
    const box = await canvas.boundingBox()
    if (!box) throw new Error('canvas has no bounding box')

    // 로봇이 순찰 이동 중이라 스캔~클릭 사이의 짧은 지연 동안 살짝 어긋날 수
    // 있으므로 몇 번 재시도한다(record-demo.mjs와 동일한 패턴).
    let selected = false
    for (let attempt = 0; attempt < 5 && !selected; attempt++) {
      const point = await waitForRobotBodyPixel(page)
      await page.mouse.click(box.x + point.x, box.y + point.y)
      try {
        await expect(page.locator('.selected-robot-panel')).toContainText('로봇 #', { timeout: 1000 })
        selected = true
      } catch {
        // 클릭 순간과 로봇의 실제 위치가 어긋났을 수 있음 — 다음 시도로 재스캔
      }
    }
    expect(selected).toBe(true)
  })
})
