# KANBAN

> 이 파일은 "지금 어디까지 왔나"를 한눈에 보기 위한 요약 보드다. 상세 스펙은 각 Plan 문서(`docs/superpowers/plans/`)에, 실제 근거는 git 커밋 이력에 있다 — 이 파일은 그 둘을 가리키는 인덱스에 가깝다.
>
> **갱신 규칙**: 태스크를 시작하면 Backlog → In Progress로, 리뷰(spec+code quality)까지 통과하면 In Progress → Done으로 옮긴다. Done 항목에는 커밋 SHA를 남긴다. **이 파일이 실제 상태보다 낡아지면 그 자체가 리뷰에서 지적당한 적이 있다 — 태스크 완료 시 바로바로 갱신할 것.**

## Done

### Plan 1 — 결정적 시뮬레이션 코어 (`docs/superpowers/plans/2026-07-15-sim-core-plan.md`)
전체 10개 태스크 + 후속 proptest 보강 2건 완료, PR #1로 `main`에 머지(`2744775`).
- 그리드/A* 경로탐색/결정적 병렬 틱(더블 버퍼링+ID 타이브레이크)/패닉 격리/프로시저럴 보행/2-본 팔 IK/자세-IK 연결/결정적 생산량 집계
- 후속: blocked-cell 경로탐색 proptest, goal-exception 실증 테스트, tick() 다중 로봇 무충돌/결정성 proptest

### Plan 2 — WS 프로토콜 & 네트워킹 (`docs/superpowers/plans/2026-07-15-ws-protocol-plan.md`)
- **Task 1** — `Robot.task` 필드 추가 (`4111ed7`)
- **Task 2** — 네트워킹 의존성(axum/tokio/serde/uuid) + 바이너리 타깃 (`8b03826`)
- **Task 3** — `GameState`(컨베이어/로봇수/선택/팔동작 커맨드) (`092daa8`, 수정 `a6e7f5d`)
- **Task 4** — `protocol.rs` 와이어 타입 + 스냅샷 (`bb11464`, 수정 `39b6971`)
- **Task 5** — `delta.rs` 변경분만 담는 델타 계산 (`4e3a35f`, 보강 `1830775`)
- **Task 6** — 최소 axum 서버 (health check) (`2e9dcc4`, 수정 `2582863`)
- **Task 7** — WebSocket 핸들러 (초기 스냅샷 + 커맨드 적용) (`2e25d1b`, 후속 로깅 수정 `e9870c8`) — 실제 WS 클라이언트로 구현자·리뷰어 각자 독립 검증됨
- **Task 8** — 20Hz 틱 루프 + 델타 브로드캐스트 (`5afecf3`, 문서 보강 `d0fe0c9`) — 실제 클라이언트로 주기적 브로드캐스트 확인됨
- **Task 9** — 세션/재접속 유예시간 순수 로직 (`492bd57`, 수정 `0609a39`) — 의도적으로 `ws.rs`에 미배선(스트레치 목표)
- **Task 10** — 통합테스트 (`acce46b`, 실제 서버 바이너리 + `tokio-tungstenite` 클라이언트) + 최종 리뷰 수정 (`f75d3fe`: `SetRobotCount` 상한 200으로 클램프, `subscribe()`를 스냅샷 전송보다 먼저 하도록 재정렬해 레이스 제거, 통합테스트 1번에 5초 타임아웃 추가) — 8회 반복 연결→커맨드→델타 사이클로 실제 재확인됨

**Plan 2 전체 완료.** 67개 테스트 통과, clippy 경고 0개.

## In Progress

(없음)

## Backlog

### Plan 2 마무리 후 명시적으로 남기는 갭 (최종 리뷰에서 발견, 의도적 이월)
- **재접속 실제 배선**: `session.rs`는 순수 로직만 있고 `ws.rs`에 배선 안 됨 — "단일 오퍼레이터" 보장이 코드로 강제되지 않음(문서상의 결정일 뿐). 재접속 통합테스트(연결 끊었다 재연결해서 델타 기준선이 이어지는지)도 없음.
- **틱 루프 패닉 주입 테스트**: 설계문서가 요구하는 "로봇 갱신 중 패닉 발생시켜도 다른 로봇은 정상 갱신" 검증이 `sim_core`(Plan 1) 레벨에는 있으나, 실제 서버 틱 루프 레벨에서는 없음.
- Lagged(지연) 브로드캐스트 수신자를 Closed와 동일하게 처리하는 단순화, 브로드캐스트 채널 용량(32)의 실측 근거 없음 — 코드에 주석으로만 남아 있고 실제 대응은 안 됨.

### Plan 3~5 (아직 계획 문서 없음, 설계문서 로드맵만 있음)
- **Plan 3** — 영속화(SQLite) + REST API + 관측가능성(`/metrics`, tracing) — 위 재접속/패닉주입/Lagged 처리 갭도 이 Plan에서 함께 다루는 게 자연스러움.
- **Plan 4** — 클라이언트 렌더링 (`client/` 디렉토리 자체가 아직 없음 — Vite+TS+Canvas, 아이소메트릭 투영)
- **Plan 5** — 데모/배포 (README, 데모 영상, Docker, 성능 목표 실측)

### 기타 (문서 위생, 급하지 않음)
- Plan 1/2 계획 문서의 태스크 체크박스(`- [ ]`)가 실제 완료 상태를 반영하지 못하고 있음 — 기능 영향 없는 문서 위생 이슈.
- `README.md`가 저장소에 아직 없음 — 설계문서의 "발표/데모 전략"이 v1부터 요구하는 항목인데 계속 미룸.

## 현재 건강도 스냅샷

- `cargo test --manifest-path server/Cargo.toml`: 67/67 통과
- `cargo clippy --all-targets`: 경고 0개
- `vitest`: 해당 없음 (`client/` 없음)
