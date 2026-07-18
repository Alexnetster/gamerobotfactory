// 배포 환경(Docker 단일 컨테이너)에서는 서버가 클라이언트 정적 파일과
// /ws를 같은 오리진(같은 호스트:포트)에서 서빙하므로, ?ws= 쿼리
// 파라미터 없이도 안전하게 기본값을 유도할 수 있다. 로컬 npm run dev
// (Vite 5173 vs 서버 임의 포트, 서로 다른 오리진)에서는 여전히 ?ws=
// 오버라이드가 필요하므로 그대로 남겨둔다.
export function resolveWsUrl(search: string, protocol: string, host: string): string {
  const override = new URLSearchParams(search).get('ws')
  if (override) {
    return override
  }
  const wsProtocol = protocol === 'https:' ? 'wss' : 'ws'
  return `${wsProtocol}://${host}/ws`
}
