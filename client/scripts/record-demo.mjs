import { chromium } from '@playwright/test'

const BASE_URL = process.argv[2] ?? 'http://localhost:8080'

async function main() {
  const browser = await chromium.launch()
  const context = await browser.newContext({
    viewport: { width: 1000, height: 700 },
    recordVideo: { dir: 'demo-recordings', size: { width: 1000, height: 700 } },
  })
  const page = await context.newPage()

  await page.goto(BASE_URL) // ?ws= 없이 — 같은 오리진 자동 유도 확인
  await page.waitForSelector('.connection-status')

  const incButton = page.locator('.sidebar button', { hasText: '+' })
  for (let i = 0; i < 5; i++) {
    await incButton.click()
    await page.waitForTimeout(300)
  }

  await page.locator('.sidebar button', { hasText: '컨베이어' }).click()
  await page.waitForTimeout(2000) // 보행/경로탐색 움직임이 화면에 보이도록

  const canvas = page.locator('canvas')
  const box = await canvas.boundingBox()
  if (!box) {
    throw new Error('canvas has no bounding box')
  }
  await page.mouse.click(box.x + box.width / 2, box.y + 40)
  // 사이드바의 선택 패널은 서버 틱(20Hz, server/src/main.rs의 TICK_INTERVAL)마다
  // innerHTML을 통째로 재생성하므로(client/src/main.ts의 renderSidebar), 버튼이
  // ~50ms마다 detach/재생성된다. Playwright의 locator.click()(force 포함)은
  // "안정된 상태"를 기다리다 계속 detached/not-visible로 재시도만 반복해
  // 타임아웃한다(실제로 재현 확인) — page.evaluate로 그 순간 DOM에 있는 버튼에
  // 직접 클릭 이벤트를 디스패치해 이 경쟁을 우회한다.
  await page.evaluate(() => {
    const button = Array.from(document.querySelectorAll('button')).find((b) => b.textContent === 'Picking')
    if (!button) {
      throw new Error('Picking button not found — is a robot selected?')
    }
    button.click()
  }) // 팔 IK 동작
  await page.waitForTimeout(1500)

  // 재접속 시나리오: 네트워크를 잠깐 끊었다 복구
  await context.setOffline(true)
  await page.waitForTimeout(1000)
  await context.setOffline(false)
  await page.waitForSelector('.connection-status:has-text("연결됨")', { timeout: 10000 })
  await page.waitForTimeout(1000)

  await context.close()
  await browser.close()
  console.log('recorded to demo-recordings/')
}

main()
