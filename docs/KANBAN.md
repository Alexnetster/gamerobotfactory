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

### Plan 3 — 영속화 + REST API + 관측가능성 + 하드닝 (`docs/superpowers/plans/2026-07-15-persistence-observability-plan.md`)
계획서(`285ca15`, 관측가능성 보강 `dd65136`, `connected_clients` 배선 보강 `4df0e5b`). 10개 태스크 전부 완료: (1)세션 재접속 실배선+Lagged 리싱크, (2)틱 루프 패닉 방어(`safe_tick`), (3)SQLite 영속화, (4)`AppConfig`+`/api/config`, (5)Prometheus `/metrics`(+`tick_panics_total`), (6)tracing 구조화 로깅, (7)전부 `main.rs`+`ws.rs`에 배선(`connected_clients` RAII 가드 포함), (8)Lagged 통합테스트, (9)Resume 통합테스트, (10)REST/영속화/메트릭 통합테스트.

- **Task 1 완료** — 세션 재접속 실배선 + Lagged 리싱크 (`7db4e37`, 문서 보강 `fa2fb1c`) — 실제 WS 클라이언트로 구현자·리뷰어 각자 독립 검증됨(초기 스냅샷에 진짜 session_id, 유효/무효 Resume 각각 정확히 응답). Plan 2 종료 시 남겨뒀던 하드닝 갭 3개 중 2개(재접속 배선, Lagged 처리) 해소.
- **Task 2 완료** — 틱 루프 패닉 방어 `safe_tick` (`d39b265`, 문서 보강 `10afdbe`) — Plan 2 이후 남은 하드닝 갭 3개 전부 해소. 리뷰에서 "패닉 시 조용히 멈추는 게 관측 안 됨" 지적이 나와, 아직 실행 전인 Task 5/7에 `tick_panics_total` 카운터를 미리 반영해둠(`dd65136`).
- **Task 3 완료** — `persistence.rs` SQLite 영속화 (`9b0945b`) — `session.rs` 때와 같은 이유로 `#![allow(dead_code)]`(아직 미배선, Task 7에서 연결). 스키마 버전/마이그레이션 없음은 의도적으로 남겨둔 갭(포트폴리오 스코프에서 지금 만들 필요 없음).
- **Task 4 완료** — `config.rs` `AppConfig` + `GET`/`POST /api/config` (`170deb3`, TODO 주석 보강 `84fcdfe`) — 필드가 하나뿐이라 지금은 전체 교체 방식, 필드 늘어나면 부분 업데이트 방식으로 바꿔야 한다는 점을 주석으로 남겨둠.
- **Task 5 완료** — `metrics.rs` Prometheus 레지스트리(`ticks_total`/`connected_clients`/`robot_count`/`tick_panics_total`) (`0b26862`, 주석 정확도 수정 `4df0e5b`) — 리뷰에서 "`connected_clients`를 `ws.rs`의 여러 exit 지점마다 수동으로 inc/dec하면 하나라도 빠뜨려 게이지가 샌다"는 지적이 나와, 아직 실행 전인 Task 7에 RAII 가드(`ConnectionGuard`) 배선을 미리 설계해둠(`4df0e5b`).
- **Task 6 완료** — `tracing`/`tracing-subscriber` 구조화 로깅으로 `eprintln!` 전체 교체 (`1e75e00`) — 코드 품질 리뷰에서 "`EnvFilter::from_default_env()`는 `RUST_LOG` 미설정 시 ERROR만 통과시켜, 방금 추가한 `warn!` 로그 6개 중 5개가 `cargo run`만으로는 전혀 안 보인다"는 지적이 나와, `try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))`로 수정(`19b2237`) — `RUST_LOG` 미설정 상태에서 WARN 로그가 실제로 출력되는 것까지 직접 재확인됨.
- **Task 7 완료** — 영속화/설정/메트릭을 `main.rs`+`ws.rs`에 실배선 (`9c6ccff`, 코드 품질 리뷰 수정 `088b3bd`) — `/api/config`가 틱 루프에 실시간 반영되는 것(POST로 `persist_every_n_ticks` 바꾸면 다음 틱부터 즉시 적용), `/api/stats/history`에 실제로 행이 쌓이는 것, `/metrics`의 `gamerobotfactory_connected_clients`가 실제 WS 접속/해제에 따라 오르내리는 것을 전부 실제 클라이언트로 재확인함. 코드 품질 리뷰에서 나온 지적 3개(DB 뮤텍스 poisoning 시 무한 조용한 실패, `stats_history`의 `.expect()`가 panic으로 요청을 죽임, `ConnectionGuard`의 불필요한 라이프타임/clippy 경고)를 전부 수정 후 재검증 완료(clippy 경고 0개, 75/75 통과). 의도적으로 지금 안 고친 것: 틱 루프 몸체 전체(`safe_tick` 바깥)에는 여전히 패닉 가드가 없음 — 아래 Backlog에 남겨둠.
- **Task 8 완료** — Lagged 리싱크 테스트 (`1f3335e`, 뮤테이션 테스트로 공허함 발견 후 재작성 `9c7df4e`) — **코드 품질 리뷰에서 뮤테이션 테스트로 잡아낸 실질 버그**: 처음 짠 통합테스트는 클라이언트가 3초간 소켓을 안 읽으면 서버가 Lagged를 겪을 거라 가정했지만, 실제로는 서버 쪽 브로드캐스트 수신 루프가 계속 `.recv()`를 부르고 있어서(막히는 건 OS 소켓 버퍼가 찰 때뿐인데 3초/60개 델타로는 어림도 없음) `Lagged` 핸들링을 통째로 `break`로 바꿔도 테스트가 그대로 통과함(진짜 공허 테스트, Plan 1의 vacuous proptest 사례와 같은 패턴). 수정: `handle_socket`의 `Ok`/`Lagged`/`Closed` 분기 로직을 `decide_broadcast_update` 순수 함수로 추출하고, 실제로 `broadcast::channel(32)`를 40개 메시지로 넘치게 한 뒤 진짜 `Err(Lagged(_))`를 받아 검증하는 결정적 단위테스트로 교체 — 리뷰어가 직접 `Lagged` 분기를 깨서 새 테스트가 실패하는 것, 되돌려서 다시 통과하는 것까지 재확인함(78/78 통과, clippy 경고 0개).
- **Task 9 완료** — Resume 통합테스트 2개(유효/무효 session_id) (`910b61b`) — Task 8의 교훈을 반영해 구현자·리뷰어 모두 뮤테이션 테스트로 검증: `resumed`를 강제로 `true`/`false` 고정시켜봐도 두 테스트 중 하나는 반드시 실패함을 직접 확인(둘이 짝을 이뤄야 진짜 검증이 된다는 것 재확인). 리뷰에서 나온 지적은 전부 Minor(중복된 `uuid` dev-dependency 한 줄, 타임아웃 가드 없음 — 실제 행 위험은 없다고 리뷰어가 직접 확인)라 수정 없이 승인(80/80 통과, clippy 경고 0개).
- **Task 10 완료** — REST/영속화/메트릭 통합테스트 (`ab7f002`, 구현자 자체 발견으로 메트릭 테스트 보강 `a2396f2`) — `config.rs`/`persistence.rs`/`metrics.rs`를 실제 서버 바이너리(테스트별 격리된 임시 SQLite 경로) + `reqwest`로 검증. 구현자가 스스로 "메트릭 이름만 확인하고 값은 확인 안 함"이라는 공허 위험을 발견해 `ticks_total` 실제 값이 0보다 큰지 파싱해서 확인하도록 자체 보강. 리뷰에서 `insert_stats`를 no-op으로, `ticks_total.inc()` 호출을 주석 처리로 각각 뮤테이션해 두 테스트가 실제로 실패하는 것/되돌리면 통과하는 것 재확인. Minor만 남음(임시 DB 파일이 assert 실패 시 `Drop` 가드 없이 정리 안 될 수 있음, stats_history 테스트가 행 개수만 보고 필드 값은 검증 안 함, `tick_panics_total`을 실제로 건드리는 REST 레벨 테스트는 없음) — 전부 Backlog로 이관.

**Plan 3 전체 완료.** 84개 테스트 통과, clippy 경고 0개. Plan 2 종료 시 남겨뒀던 하드닝 갭 3개(재접속 실배선, Lagged 리싱크, 틱 루프 패닉 방어) 전부 코드로 구현되고 통합테스트로 검증됨. SQLite 생산 통계 영속화, 실시간(WS)과 분리된 `/api/config` 설정 채널, `tracing` 구조화 로깅 + `/metrics` Prometheus 엔드포인트 전부 실배선+검증 완료.

### Plan 3~4 사이 보강 — 틱 처리시간 p99 메트릭
설계문서(line 101)가 요구하는 "틱 처리 시간 p99 < 10ms" 목표를 측정할 수단이 없었던 갭을 상태 점검 중 발견해 별도 태스크로 처리.
- **완료** — `gamerobotfactory_tick_duration_seconds` Prometheus 히스토그램 추가 (`1d978fd`) — `state.lock()` 획득부터 스냅샷/델타 계산까지(브로드캐스트 전송·비동기 영속화 디스패치는 제외)의 실제 소요시간을 매 틱 측정. 10ms(p99 목표)와 50ms(20Hz 틱 예산)를 정확히 버킷 경계로 포함해 `histogram_quantile`로 바로 조회 가능. 구현자·리뷰어 모두 `.observe()` 호출을 제거해도 REST 통합테스트가 통과하는지 뮤테이션 테스트로 확인(둘 다 실제로 실패 → 되돌리면 통과, 85/85 통과, clippy 경고 0개).

## Backlog

### 로봇 내구도/고장/복구 (설계 완료, 구현 대기)
사용시간 누적 → 고장 확률 증가 → 오퍼레이터가 부품 교체(복구) → 장애 감지+조치 프로세스를 로봇 도메인에도 적용하자는 아이디어(브레인스토밍 2026-07-16).
- **설계 완료** — `docs/superpowers/specs/2026-07-16-robot-durability-failure-design.md` (`ba148eb`) — 단일 내구도 값, `(robot_id, tick_count)` 시드 결정적 해시로 병렬 틱과 공존하는 고장 판정, 별도 `status` 필드(기존 `task`와 분리), `RepairRobot` 커맨드, `robot_failures_total`/`robots_repairing` 메트릭, `robot_failure_events` SQLite 테이블.
- **완결성 리뷰 + 갭 수정 완료** (`740d280`) — 독립 리뷰어가 스펙을 코드베이스와 대조 검증. High 2건 실질 발견: (1) `durability_remaining`을 원값 그대로 노출하면 델타 압축(이 프로토콜의 핵심 셀링포인트)이 작업 중인 로봇마다 무력화됨 → 5% 단위 반올림으로 해결(단, `Repairing`의 실시간 카운트다운은 의도적으로 반올림 안 함, 진행률 표시 목적+짧은 지속시간이라 비용 무시 가능). (2) 테스트 전략이 "자연 마모로 확률적 고장을 기다리는" 플레이키 테스트 함정을 명시적으로 막지 못함 → Task 8 사례처럼 되지 않도록 결정적 시딩 방식을 테스트 전략에 명문화. 그 외 Medium/Low: 마모율 계산 중복(→ `wear_ratio()` 단일 소스로 통합), 상태전이 위치 서술 오류(Failed→Repairing은 tick() 밖 ws.rs에서 일어남) 수정, `SetRobotCount`+고장/복구중 로봇 처리 방침 명문화, `selected_robot` 영향 없음 명시, 라인 인용 오탈자 수정. **사용자 스펙 리뷰 승인 대기 중** — 승인되면 writing-plans로 구현 계획 작성.

### Plan 4~5 (아직 계획 문서 없음, 설계문서 로드맵만 있음)
- **Plan 4** — 클라이언트 렌더링 (`client/` 디렉토리 자체가 아직 없음 — Vite+TS+Canvas, 아이소메트릭 투영)
- **Plan 5** — 데모/배포 (데모 영상, Docker, 성능 목표 실측 — README 자체는 완료됨)

### 관측가능성/견고성 (Plan 3 리뷰에서 나왔으나 지금은 안 고친 것)
- `spawn_tick_loop`의 틱 루프 몸체는 `safe_tick`(`sim_core::sim::tick` 호출부)만 패닉 격리돼 있고, 그 바깥(생산량 집계/스냅샷 변환/델타 계산 등)에서 패닉이 나면 틱 루프 태스크 자체가 죽어 서버가 조용히 멈춘다(`/health`는 여전히 "ok"). 지금 당장 고치기엔 설계 판단(전체를 `catch_unwind`로 감쌀지, 태스크 재시작 감독자를 둘지)이 필요해 보류. (Task 7 리뷰)
- DB 영속화 실패(뮤텍스 poisoning, `insert_stats` 에러)를 구분해서 보여줄 전용 메트릭(`persist_failures_total` 같은 것)이 없음 — 지금은 `tracing::error!` 로그로만 남음. (Task 7 리뷰)
- `tick_panics_total`을 실제로 건드리는 REST/통합 레벨 테스트가 없음(`sim`/`safe_tick` 유닛 레벨에서만 검증됨) — 배선 자체는 됐지만 엔드투엔드로는 미검증. (Task 10 리뷰)
- `stats_history_reflects_persisted_rows_after_running` 테스트는 행이 "있는지"만 확인하고 필드 값(tick 번호, robot_count 등)이 맞는지는 확인 안 함 — 값이 그럴듯하지만 틀린 영속화 버그는 못 잡음. (Task 10 리뷰)
- `server/tests/rest_integration.rs`의 임시 SQLite 파일 정리가 각 테스트 끝에 있는 평범한 `remove_file` 호출이라, 그 전에 assert가 패닉하면 파일이 안 지워짐(`ServerProcess`의 프로세스 kill은 `Drop`이라 안전한데 파일 정리는 아님) — 실질적 위험은 낮음(UUID 이름, OS 임시 폴더). (Task 10 리뷰)

### 기타 (문서 위생, 급하지 않음)
- Plan 1/2/3 계획 문서 3개 전부(스텝 총 100+개) 태스크 체크박스(`- [ ]`)가 실제 완료 상태를 반영하지 못하고 있음 — 기능 영향 없는 문서 위생 이슈. (상태 점검 중 Plan 3도 해당됨을 추가 확인)
- 설계문서가 v1 스코프에 "CI"를 명시하고 있으나(`docs/robot-arm-conveyor-game-design.md` 백엔드 요약 표) `.github/workflows` 자체가 없음 — 지금은 로컬 `cargo test`/`clippy`로만 검증됨.

## 현재 건강도 스냅샷

- `cargo test --manifest-path server/Cargo.toml`: 85/85 통과
- `cargo clippy --all-targets`: 경고 0개
- `vitest`: 해당 없음 (`client/` 없음)
