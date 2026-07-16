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

## In Progress

### 로봇 내구도/고장/복구 (`docs/superpowers/plans/2026-07-16-robot-durability-failure-plan.md`)
사용시간 누적 → 고장 확률 증가 → 오퍼레이터가 부품 교체(복구) → 장애 감지+조치 프로세스를 로봇 도메인에도 적용하자는 아이디어(브레인스토밍 2026-07-16). 설계 완료(`docs/superpowers/specs/2026-07-16-robot-durability-failure-design.md`, `ba148eb`, 완결성 리뷰 갭 수정 `740d280` — 델타 압축 회귀 방지용 내구도 5% 반올림, 플레이키 테스트 함정 방지용 결정적 시딩 테스트 전략 등), 구현 계획 작성 완료(`cf72051`). 9개 태스크: (1)~~sim_core RobotStatus/마모/결정적 고장/이동정지~~ ✅, (2)~~game_state.rs 커맨드 검증+RepairRobot~~ ✅, (3)~~protocol.rs+delta.rs 와이어 프로토콜~~ ✅, (4)~~ws.rs 통합테스트(배선 자체는 Task 3에서 선반영됨)~~ ✅, (5)~~metrics.rs~~ ✅, (6)~~persistence.rs~~ ✅, (7)~~main.rs 전이감지+배선+REST~~ ✅, (8)~~tick_properties.rs 결정성 proptest~~ ✅, (9)전체 검증+문서 갱신.

- **Task 1 완료** — `sim_core`에 `RobotStatus`(Operational/Failed/Repairing) + `worn_ticks` + `wear_ratio()` + `(robot_id, tick_count)` 시드 결정적 해시 고장 판정 + 이동 정지 (`e0a16fc`, 코드 품질 리뷰 수정 `947afd6`) — 기존 `occupied` 스냅샷 메커니즘이 고장난 로봇을 별도 코드 없이 자동으로 장애물 취급하는 것을 활용. **코드 품질 리뷰에서 뮤테이션 테스트로 잡아낸 실질 문제**: 최초의 "고장난 로봇이 장애물 역할을 하는지" 테스트는 한 틱만 확인해서, Failed 여부와 무관하게 이미 존재하던 "한 틱짜리 점유 스냅샷" 규칙만으로도 통과해버리는 공허한 검증이었음(Task 8의 Lagged 사례와 같은 패턴). 목표 로봇을 향해 이동해야 할 로봇을 고장 상태로 만들고 10틱 반복 검증하는 방식으로 재작성 — 리뷰어가 직접 이동정지 게이트를 무력화해서 mover 쪽 단언만 분리해도 실패하는 것까지 확인(정상이라면 그 지점에서 고장 로봇이 비켜서 mover가 전진했을 것이므로). 45/45(lib) 통과, clippy 경고 0개.
- **Task 2 완료** — `game_state.rs`에 `TriggerArmAction` 상태 가드 + `repair_robot` 메서드 (`c0fd1ee`) — `CommandError`에 `RobotNotOperational`/`RobotNotFailed` 추가하며 clippy `enum_variant_names` 경고가 새로 뜬 것을 `#[allow(...)]`로 좁게 억제(변수명 rename보다 계획서와의 일관성 우선 — 리뷰어는 "rename이 실제로는 파급 범위가 거의 없다"는 반대 의견을 냈지만 둘 다 정당한 선택이라고 판단, 강제 아님). 101/101 통과, clippy 경고 0개. 리뷰에서 나온 지적은 전부 Minor(억제 주석의 "ws.rs/protocol.rs도 이 이름을 참조한다"는 문구가 사실과 다름, `set_robot_count`+`Repairing` 회귀테스트가 "지금의" 동작이 아니라 "미래의" 실수를 막는 가드에 가깝다는 설명 보강 필요)라 수정 없이 승인.
- **Task 3 완료** — `protocol.rs`에 `WireStatus` + `RobotView.status`/`durability_remaining`(5% 반올림) + `RepairRobot` 커맨드, `delta.rs` 테스트 헬퍼 수정 (`f7d440c`) — `ClientCommand::RepairRobot` 추가로 `ws.rs::apply_command`의 소진적 매치가 깨지는 것을 막기 위해 Task 4 Step 1(정확히 같은 코드)을 선반영함을 커밋 메시지에 명시, 스펙 리뷰어가 계획서 Task 4 텍스트와 바이트 단위로 대조해 확인. 코드 품질 리뷰에서 `quantize_durability`가 실제로 델타 압축을 되살리는지 직접 계산 검증(`worn_ticks` 0~20까지 전부 `durability_remaining=1.0`으로 동일, 약 100틱에 한 번만 바뀜 — 설계문서 주장과 일치). 105/105 통과, clippy 경고 0개. **부수 발견**: 설계문서(`docs/superpowers/specs/...design.md`)의 수식 설명이 `1.0 -` 반전을 빠뜨려서 문자 그대로 읽으면 새 로봇이 내구도 0%로 나오는 오류였음(실제 구현/계획 문서 코드는 처음부터 맞았음) — 별도 커밋(`4b71d98`)으로 수정.
- **Task 4 완료** — WS 통합테스트 1개 추가(`aac1eae`, 코드 리뷰 후속 주석 보강 `c2aed3d`) — 배선 자체는 Task 3에서 이미 끝나 있어(`f7d440c`) 테스트만 추가. 코드 품질 리뷰가 이 테스트 하나만으로는 "RepairRobot이 실제로 파싱돼서 거부됐다"와 "JSON이 애초에 안 파싱됐다"를 구분 못 함을 직접 뮤테이션으로 확인(둘 다 연결은 안 죽으므로) — 다만 `protocol.rs`의 파싱 증명 테스트 + `game_state.rs`의 거부 사유 증명 테스트와 합쳐지면 전체 주장은 성립한다고 판단해 승인, 그 구성 관계를 명시하는 주석만 보강. 106/106 통과, clippy 경고 0개.
- **Task 5 완료** — `metrics.rs`에 `robot_failures_total`/`robots_repairing` 추가 (`2864597`) — 기존 5개 메트릭과 같은 "write-then-wire-later" 패턴(아직 어디서도 안 읽음/안 증가시킴, Task 7에서 배선). `.expect()` 문구 5→7개 이름으로 전부 일관되게 갱신됐는지, 캐시 없이 clean build로 dead-code 경고가 안 뜨는지 리뷰어가 직접 확인. 107/107 통과, clippy 경고 0개. Minor 하나(게이지 이름 `robots_repairing`이 다른 형제 게이지들의 "명사_count" 네이밍과 살짝 다름)만 나와 수정 없이 승인.
- **Task 6 완료** — `persistence.rs`에 `robot_failure_events` 테이블 + `FailureEvent`/`insert_failure_event`/`recent_failure_events` (`43c02ae`) — 기존 `stats_history`와 같은 패턴, 아직 미배선(Task 7). 이번엔 파일 전체가 아니라 새로 추가된 3개 항목에만 `#[allow(dead_code)]`를 좁게 붙임(이미 배선된 `stats_history` 쪽 코드의 dead-code 검사는 그대로 살아있게) — `session.rs`/`persistence.rs`가 예전에 썼던 파일 전체 억제 방식에서 한 단계 더 정교해진 것. 110/110 통과, clippy 경고 0개. Minor만 남음(향후 실제 배선 시 `event_type`에 CHECK 제약 고려해볼 만함, 모듈 최상단 문서 주석이 아직 이 새 테이블을 언급 안 함) — 수정 없이 승인.
- **Task 7 완료** — `main.rs`에 `detect_status_transitions` 순수 함수 + 틱 루프 배선(메트릭 증가/게이지 갱신, `spawn_blocking`으로 이벤트 영속화) + `GET /api/robots/failures` (`50dcd2c`, 코드 품질 리뷰 수정 `0b0bbe6`) — `persistence.rs`의 `#[allow(dead_code)]` 3개도 이제 실제로 쓰이므로 제거. 리뷰에서 실질 테스트 커버리지 갭 2개 발견: (1) 로봇이 이미 Failed/Repairing인 상태가 유지될 때 이벤트가 매 틱 다시 안 나가는지 검증하는 테스트가 없었음(뮤테이션 테스트로 "이전 상태 무시하고 현재 상태만 보고 발화"하도록 깨봐도 기존 5개 테스트가 다 통과하는 것으로 확인) → 회귀 테스트 2개 추가. (2) 새 REST 테스트 2개가 로봇이 0마리인 서버에서만 검증해서 "고장 없음"이 진짜 관찰 결과가 아니라 로봇이 아예 없어서 당연한 것이었음 → 서버 시작 전에 SQLite 파일에 이벤트를 직접 시딩(스키마를 `rusqlite`로 직접 복제 — `persistence` 모듈이 바이너리 크레이트 전용이라 통합테스트에서 직접 import 불가한 게 확인된 제약) 후 REST로 정확히 그 행이 돌아오는지 검증하는 진짜 라운드트립 테스트로 교체. 리뷰어가 두 수정 모두 독립적으로 뮤테이션 재현해서 검증. 120/120 통과, clippy 경고 0개.
- **Task 8 완료** — `tick_properties.rs`에 Failed/Repairing 로봇이 섞인 상태에 대한 proptest 3개(충돌 없음/결정성/영구 정지) 추가 (`160f419`) — **proptest가 실제 동작 하나를 새로 찾아냄**: `remaining_ticks: 1`인 복구 중 로봇은 그 틱에 `Operational`로 전이된 뒤 같은 틱 안에서 바로 이동까지 할 수 있음(`update_status`가 `plan_robot`의 이동정지 체크보다 먼저 실행되므로) — proptest가 실제로 이 경우를 찾아내 축소된 반례를 `.proptest-regressions`에 남김. 구현자가 "복구 끝나면 하던 일을 바로 이어간다"는 설계 의도에 맞는 의도된 동작으로 판단해 프로덕션 코드 대신 테스트의 불변식만 좁힘(`remaining_ticks > 1`만 "반드시 안 움직임" 대상), 스펙 리뷰어와 코드 품질 리뷰어 둘 다 각자 독립적으로 뮤테이션 재현(반례를 되살려서 실패 재현 → 되돌려서 통과 재확인)까지 해서 동의. 스펙이 이 타이밍에 대해 원래 침묵하고 있었다는 것도 확인됨 — 설계문서에 명시적으로 의도된 동작이라고 기록(`2aada9e`). 이 프로젝트 마지막 콘텐츠 태스크. clippy 경고 0개.

## Backlog

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

- `cargo test --manifest-path server/Cargo.toml`: 123/123 통과 (진행 중인 로봇 내구도 기능 Task 1~8 반영, 총계는 태스크가 진행되며 계속 늘어남)
- `cargo clippy --all-targets`: 경고 0개
- `vitest`: 해당 없음 (`client/` 없음)
