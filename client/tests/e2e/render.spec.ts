import { test, expect } from '@playwright/test'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

function backendPort(): number {
  const info = JSON.parse(readFileSync(path.resolve(__dirname, '.server-info.json'), 'utf-8')) as { port: number }
  return info.port
}

// 두 테스트가 같은 globalSetup 서버 인스턴스를 공유한다(파일당 한 번만
// 뜬다) — 서버의 로봇 수는 커넥션을 넘어 서버 프로세스에 살아있는 전역
// 상태라, 앞선 테스트가 로봇을 스폰해 두면 다음 테스트가 접속했을 때부터
// 이미 그 로봇이 보인다. 그래서 "+" 클릭 후 카운트를 고정값 '1'로 단언하면
// 실행 순서에 따라 우연히 통과(혹은 실패)하는 깨지기 쉬운 테스트가 된다
// (실측: 두 번째 테스트가 첫 테스트가 남긴 로봇 때문에 시작부터 이미 1이었고,
// 클릭 이후 실제로는 2가 되어야 정상인데도 타이밍에 따라 assertion이 '1'을
// 잡아버려 클릭이 아무 효과가 없어도 통과할 뻔한 적이 있었다). 클릭 전
// 카운트를 읽어 "정확히 +1 됐는지"를 검증하면 이전 테스트의 잔여 상태와
// 무관하게 결정적이고, 클릭이 실제로 효과를 냈다는 것도 확실히 검증된다.
async function currentRobotCount(page: import('@playwright/test').Page): Promise<number> {
  const text = await page.locator('.robot-count').textContent()
  return Number(text)
}

test.describe('client renders against a real server', () => {
  test('draws a spawned robot at its projected screen position', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 700 })
    await page.goto(`/?ws=ws://127.0.0.1:${backendPort()}/ws`)

    const before = await currentRobotCount(page)
    const incButton = page.locator('.sidebar button', { hasText: '+' })
    await incButton.click()
    await expect(page.locator('.robot-count')).toHaveText(String(before + 1), { timeout: 5000 })

    const canvas = page.locator('canvas')
    const box = await canvas.boundingBox()
    if (!box) throw new Error('canvas has no bounding box')

    // 로봇 몸체(fillRect(-11, -bodyLift-8, 22, 16))는 씬 원점(캔버스
    // translate 기준점, drawScene의 ctx.translate(canvasWidth/2, 40))
    // 기준으로 절대 좌표 x∈[center-11, center+11), y∈[20, 36)에 그려진다.
    // 단일 픽셀(예: 몸체 중앙점)은 다리 스트로크(y 12~4 구간, x=±5/±11
    // 부근)나 팔 스트로크의 시작점(정확히 로컬 (0, -bodyLift), 즉 절대
    // (center, 28))과 우연히 겹칠 수 있다 — 실제로 겹쳤었다: 팔 색
    // (#a06f1a, R>G 고정값)이 몸체 그라디언트 대신 잡혀서, 몸체 색을
    // 파란색(R<G)으로 바꿔도 여전히 R>G로 나와 뮤테이션 테스트에서
    // 잡히지 않는 공허한 검증이 됐다. 몸체 사각형 전체를 평균 내면
    // 얇은 다리/팔 선(전체 면적의 일부)의 영향이 희석되고, 실제 몸체
    // 채우기 색(그라디언트: #ffd27a ~ #d99a2e, R>G)이 지배적으로 반영된다.
    const avg = await page.evaluate(
      ({ x, y, w, h }) => {
        const c = document.querySelector('canvas') as HTMLCanvasElement
        const ctx = c.getContext('2d')!
        const data = ctx.getImageData(x, y, w, h).data
        let sumR = 0
        let sumG = 0
        const pixelCount = data.length / 4
        for (let i = 0; i < data.length; i += 4) {
          sumR += data[i]
          sumG += data[i + 1]
        }
        return { avgR: sumR / pixelCount, avgG: sumG / pixelCount }
      },
      { x: Math.round(box.width / 2) - 11, y: 20, w: 22, h: 16 },
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

    await page.mouse.click(box.x + box.width / 2, box.y + 40)

    await expect(page.locator('.selected-robot-panel')).toContainText('로봇 #', { timeout: 5000 })
  })
})
