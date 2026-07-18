import { test } from 'node:test'
import assert from 'node:assert/strict'
import { parseTickDurationP99 } from './perf-metrics.mjs'

const SAMPLE_METRICS = `
# HELP gamerobotfactory_tick_duration_seconds tick 처리시간
# TYPE gamerobotfactory_tick_duration_seconds histogram
gamerobotfactory_tick_duration_seconds_bucket{le="0.001"} 100
gamerobotfactory_tick_duration_seconds_bucket{le="0.005"} 500
gamerobotfactory_tick_duration_seconds_bucket{le="0.01"} 990
gamerobotfactory_tick_duration_seconds_bucket{le="0.05"} 999
gamerobotfactory_tick_duration_seconds_bucket{le="+Inf"} 1000
gamerobotfactory_robot_count 50
`

test('finds the bucket where cumulative count first reaches the 99th percentile', () => {
  // total=1000, target=990 -> le="0.01" 버킷(count=990)이 처음으로 990 이상
  assert.equal(parseTickDurationP99(SAMPLE_METRICS), 0.01)
})

test('returns null when no histogram buckets are present', () => {
  assert.equal(parseTickDurationP99('gamerobotfactory_robot_count 0\n'), null)
})

test('treats +Inf as the last bucket without breaking numeric sort', () => {
  const withOnlyInf = `
gamerobotfactory_tick_duration_seconds_bucket{le="0.001"} 10
gamerobotfactory_tick_duration_seconds_bucket{le="+Inf"} 10
`
  assert.equal(parseTickDurationP99(withOnlyInf), 0.001)
})

test('uses the 99th percentile, not the 95th (buckets chosen so they land in different buckets)', () => {
  // total=1000
  // p95 target=950 -> le="0.001"(900) < 950, le="0.005"(960) >= 950 => 0.005
  // p99 target=990 -> le="0.005"(960) < 990, le="0.01"(995) >= 990 => 0.01
  // 0.005 !== 0.01이므로, 구현이 0.95를 쓰면 이 테스트는 실패한다.
  const finegrained = `
gamerobotfactory_tick_duration_seconds_bucket{le="0.001"} 900
gamerobotfactory_tick_duration_seconds_bucket{le="0.005"} 960
gamerobotfactory_tick_duration_seconds_bucket{le="0.01"} 995
gamerobotfactory_tick_duration_seconds_bucket{le="+Inf"} 1000
`
  assert.equal(parseTickDurationP99(finegrained), 0.01)
})
