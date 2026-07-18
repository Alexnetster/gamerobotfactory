// client/scripts/perf-check.mjs
import { parseTickDurationP99 } from './perf-metrics.mjs'
import WebSocket from 'ws'

const BASE_URL = process.argv[2]
if (!BASE_URL) {
  console.error('사용법: node client/scripts/perf-check.mjs <배포된 URL, 예: https://gamerobotfactory.fly.dev>')
  process.exit(1)
}

async function main() {
  const wsUrl = `${BASE_URL.replace(/^http/, 'ws')}/ws`
  const ws = new WebSocket(wsUrl)
  await new Promise((resolve, reject) => {
    ws.once('open', resolve)
    ws.once('error', reject)
  })
  ws.send(JSON.stringify({ type: 'SetRobotCount', count: 50 }))
  ws.close()

  console.log('로봇 50대 반영 대기(10초)...')
  await new Promise((r) => setTimeout(r, 10000))

  const metricsText = await (await fetch(`${BASE_URL}/metrics`)).text()
  const p99 = parseTickDurationP99(metricsText)
  const robotCountMatch = /gamerobotfactory_robot_count (\d+)/.exec(metricsText)
  const tickCountMatch = /gamerobotfactory_ticks_total (\d+)/.exec(metricsText)

  console.log(`robot_count=${robotCountMatch?.[1] ?? '알 수 없음'}`)
  console.log(`ticks_total=${tickCountMatch?.[1] ?? '알 수 없음'}`)
  console.log(`tick_duration_seconds p99 근사치=${p99 ?? '버킷 없음'}s (목표: <0.01s)`)
}

main()
