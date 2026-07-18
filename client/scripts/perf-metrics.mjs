// client/scripts/perf-metrics.mjs — 순수 함수, 유닛테스트 대상(perf-metrics.test.mjs)
export function parseTickDurationP99(metricsText) {
  const buckets = []
  for (const line of metricsText.split('\n')) {
    const match = /^gamerobotfactory_tick_duration_seconds_bucket\{le="([^"]+)"\}\s+(\d+)/.exec(line)
    if (match) {
      buckets.push({ le: match[1] === '+Inf' ? Infinity : Number(match[1]), count: Number(match[2]) })
    }
  }
  if (buckets.length === 0) {
    return null
  }
  buckets.sort((a, b) => a.le - b.le)
  const total = buckets[buckets.length - 1].count
  const target = total * 0.99
  const p99Bucket = buckets.find((b) => b.count >= target)
  return p99Bucket ? p99Bucket.le : null
}
