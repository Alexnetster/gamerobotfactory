import { chromium } from '@playwright/test'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'

const run = promisify(execFile)
const BASE_URL = process.argv[2] ?? 'http://localhost:8080'

async function main() {
  const browser = await chromium.launch()
  const context = await browser.newContext({
    viewport: { width: 1000, height: 700 },
    recordVideo: { dir: 'demo-recordings', size: { width: 1000, height: 700 } },
  })
  const page = await context.newPage()

  try {
    await recordScenario(page)
    console.log('recorded to demo-recordings/')
  } finally {
    // 위에서 무엇이 실패하든(셀렉터 타임아웃 등) 브라우저/컨텍스트는 반드시 정리한다 —
    // 안 그러면 헤드리스 Chromium 프로세스가 남고 녹화 영상도 불완전하게 남는다.
    await context.close().catch(() => {})
    await browser.close().catch(() => {})
  }
}

async function recordScenario(page) {
  await page.goto(BASE_URL) // ?ws= 없이 — 같은 오리진 자동 유도 확인
  await page.waitForSelector('.connection-status')

  const incButton = page.locator('.sidebar button', { hasText: '+' })
  for (let i = 0; i < 5; i++) {
    await incButton.click()
    await page.waitForTimeout(300)
  }

  await page.locator('.sidebar button', { hasText: '컨베이어' }).click()
  await page.waitForTimeout(3000) // 순찰 로봇의 보행/경로탐색 움직임이 화면에 충분히 보이도록

  const canvas = page.locator('canvas')
  const box = await canvas.boundingBox()
  if (!box) {
    throw new Error('canvas has no bounding box')
  }
  // 로봇이 순찰 중이라(Task 7) 고정 좌표(캔버스 중앙 근처)를 클릭하는 방식은
  // 더 이상 안 통한다 — 클릭 시점에 로봇이 스폰 지점에서 이미 여러 칸
  // 이동해 있어(실제로 재현: 5마리 추가 + 컨베이어 토글 대기 후 클릭하면
  // "Picking button not found" 예외) 클릭 판정 반경(24px, main.ts) 안에
  // 아무 로봇도 없다. 대신 canvas 픽셀을 직접 스캔해 로봇 몸체 그라디언트
  // 색(#ffd27a~#d99a2e, canvas.ts의 drawRobot)이 실제로 그려진 화면 좌표를
  // 찾아 그 자리를 클릭한다 — 로봇이 어디로 순찰 이동했든 항상 맞는다.
  async function findRobotClickPoint() {
    return page.evaluate(() => {
      const c = document.querySelector('canvas')
      const ctx = c.getContext('2d')
      const { width, height } = c
      const data = ctx.getImageData(0, 0, width, height).data
      const isBodyColor = (r, g, b) => r > 190 && g >= 130 && g <= 225 && b < 140

      let seed = null
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

      // seed 주변 좁은 창(로봇 몸체 22x16보다 약간 넓은 ±15px)만 재스캔해서
      // 무게중심을 구한다 — 여러 로봇이 화면에 있어도 이 창 안에는 보통
      // 한 로봇의 몸체 픽셀만 있으므로, 서로 다른 로봇 사이에서 평균이
      // 흐트러져 엉뚱한 빈 공간을 가리키는 일이 없다.
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

  // 로봇이 계속 순찰 이동 중이라 스캔~클릭 사이의 짧은 지연 동안 살짝
  // 어긋날 수 있으므로 몇 번 재시도한다.
  let pickingClicked = false
  for (let attempt = 0; attempt < 5 && !pickingClicked; attempt++) {
    const point = await findRobotClickPoint()
    if (!point) {
      throw new Error('canvas에서 로봇 몸체 픽셀을 찾지 못함 — 순찰 중인 로봇이 화면에 없는 것으로 보임')
    }
    await page.mouse.click(box.x + point.x, box.y + point.y)
    // 사이드바의 선택 패널은 서버 틱(20Hz, server/src/main.rs의 TICK_INTERVAL)마다
    // innerHTML을 통째로 재생성하므로(client/src/main.ts의 renderSidebar), 버튼이
    // ~50ms마다 detach/재생성된다. Playwright의 locator.click()(force 포함)은
    // "안정된 상태"를 기다리다 계속 detached/not-visible로 재시도만 반복해
    // 타임아웃한다(실제로 재현 확인) — page.evaluate로 그 순간 DOM에 있는 버튼에
    // 직접 클릭 이벤트를 디스패치해 이 경쟁을 우회한다.
    pickingClicked = await page.evaluate(() => {
      const button = Array.from(document.querySelectorAll('button')).find((b) => b.textContent === 'Picking')
      if (!button) return false
      button.click()
      return true
    }) // 팔 IK 동작
    if (!pickingClicked) {
      await page.waitForTimeout(100)
    }
  }
  if (!pickingClicked) {
    throw new Error('Picking button not found after retries — is a robot selected?')
  }
  await page.waitForTimeout(1500)

  // 재접속 시나리오: 컨테이너를 실제로 내렸다 올려서 WS 연결이 진짜로 끊기게 한다
  // (setOffline은 이미 열린 WS를 안 닫으므로 쓸 수 없음 — 위 설계 결정 참고).
  // stop과 start는 반드시 짝을 이뤄야 한다 — 중간의 waitForSelector가 타임아웃 등으로
  // 던지더라도 컨테이너를 내린 채로 스크립트가 끝나 개발 환경이 조용히 망가지면 안 된다.
  try {
    await run('docker', ['compose', 'stop', 'app'])
    await page.waitForSelector('.connection-status:has-text("재연결")', { timeout: 10000 })
    await page.waitForTimeout(1000)
  } finally {
    await run('docker', ['compose', 'start', 'app'])
  }
  // 컨테이너가 실제로 다시 떠서 재연결됐는지 확인하는 단계 — "반드시 복구한다"는
  // 보장과는 별개라 위 finally 밖에 둔다.
  await page.waitForSelector('.connection-status:has-text("연결됨")', { timeout: 15000 })
  await page.waitForTimeout(1000)
}

main().catch((err) => {
  console.error(err)
  process.exitCode = 1
})
