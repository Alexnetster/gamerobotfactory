# 로봇팔 컨베이어 게임

4족 보행 로봇이 공장 컨베이어 라인을 돌며 작업을 돕는 팩토리 시뮬레이션. **백엔드/서버 설계 역량을 보여주는 포트폴리오 프로젝트**로, 게임성은 2차 목표다 — 결정적 시뮬레이션 코어, WebSocket 델타 동기화 프로토콜, 동시성 안전한 틱 루프가 이 프로젝트의 실제 핵심이다.

전체 설계 배경과 트레이드오프는 [`docs/robot-arm-conveyor-game-design.md`](docs/robot-arm-conveyor-game-design.md)에, 지금까지 뭘 만들었고 뭐가 남았는지는 [`docs/KANBAN.md`](docs/KANBAN.md)에 있다.

## 핵심 엔지니어링 결정

- **결정적 병렬 틱**: 매 틱 `rayon`으로 로봇들을 병렬 갱신하되, 직전 틱의 스냅샷만 읽는 더블 버퍼링 + 로봇 ID 기반 타이브레이크로 동시 이동 충돌을 결정적으로 해소한다. 몇 번을 돌려도 같은 입력이면 같은 결과가 나온다 — `tick_is_deterministic`/`tick_never_produces_collisions` proptest로 검증.
- **장애 격리**: 로봇 한 대의 갱신 로직이 패닉해도(`catch_unwind`) 그 틱은 스킵될 뿐 나머지 로봇과 다른 클라이언트 연결에는 영향이 없다. `safe_tick`이 스킵된 틱 수를 `tick_panics_total` 메트릭으로 노출한다 — "조용히 멈추는" 실패를 관측 가능하게 만든 것.
- **클라이언트별 델타 동기화 + 무손실 재접속**: 매 틱 전체 상태 대신 바뀐 로봇만 보내고(`Delta`), 연결이 브로드캐스트 채널 용량을 넘겨 뒤처지면(Lagged) 끊는 대신 전체 스냅샷으로 재동기화한다. 세션 토큰(UUID)으로 30초 유예시간 내 재접속을 지원.
- **성능 목표를 실측 가능하게**: 틱 처리시간(락 획득~스냅샷/델타 계산)을 `tick_duration_seconds` Prometheus 히스토그램으로 노출한다 — 목표(p99 < 10ms, 20Hz 틱 예산 50ms)를 정확히 버킷 경계로 잡아둬서 `histogram_quantile`로 바로 조회 가능. "성능 목표를 문서에만 적어두고 측정 수단이 없는" 흔한 함정을 피한 것.
- **리뷰로 잡아낸 실질 버그들**: 통합테스트 중 하나가 "그럴듯해 보이지만 실제로는 아무것도 검증하지 않는" 공허한 테스트였음을 뮤테이션 테스트(동작을 일부러 깨고 테스트가 여전히 통과하는지 확인)로 발견해 재작성했고, `SetRobotCount`에 상한이 없어 무제한 메모리 할당으로 서버를 멈출 수 있었던 DoS 버그도 리뷰 과정에서 잡았다. 자세한 내용은 [`docs/KANBAN.md`](docs/KANBAN.md) 참고.

## 아키텍처

```
sim_core (라이브러리, 네트워크 의존성 없음)
  그리드 · A* 경로탐색 · 결정적 병렬 틱(더블 버퍼링+ID 타이브레이크) · 패닉 격리
  프로시저럴 보행 · 2-본 팔 IK · 자세-IK 연결 · 결정적 생산량 집계

server (바이너리, sim_core에 의존)
  GameState(컨베이어/로봇수/선택/팔동작) · JSON 와이어 프로토콜(버전 필드,
  변경분만 담는 델타) · 20Hz 틱 루프 + 브로드캐스트 · axum WebSocket 핸들러
  세션/재접속 + Lagged 리싱크 · SQLite 영속화 · REST 설정 API ·
  Prometheus 메트릭 · tracing 구조화 로깅
```

두 부분 다 같은 Cargo 패키지(`gamerobotfactory-server`) 안에서 라이브러리 타깃(`sim_core`)과 바이너리 타깃(`server`)으로 나뉘어 있다.

## 개발 환경

- Rust **stable 1.85 이상** (edition 2021). `rust-toolchain.toml`은 따로 없음 — 특정 nightly 기능에 의존하지 않는다.
- 주요 의존성: `axum`(WS/HTTP) · `tokio`(비동기 런타임) · `rayon`(틱 병렬화) · `rusqlite`(bundled, 영속화) · `prometheus`(메트릭) · `tracing`(구조화 로깅) · `serde`/`serde_json`(와이어 포맷).
- OS 종속 코드 없음 — Windows/Linux/macOS 어디서든 `cargo build`만으로 빌드된다(개발은 Windows에서 진행, CI는 아직 없음 — Backlog 참고).
- `client/` 디렉토리는 아직 없다(Plan 4 이전).

```bash
# 빌드
cargo build --manifest-path server/Cargo.toml

# 테스트 전체 실행 (85개, 인테그레이션 테스트 일부는 실제 서버 바이너리를 띄워 수 초 소요)
cargo test --manifest-path server/Cargo.toml

# 린트
cargo clippy --manifest-path server/Cargo.toml --all-targets
```

## 동작 환경

```bash
cargo run --manifest-path server/Cargo.toml
```

- **포트**: 고정 포트를 쓰지 않는다(`127.0.0.1:0`, OS가 임의 할당) — 실제 바인딩된 포트를 표준출력의 `LISTENING_PORT=<번호>` 줄로 알려준다. 여러 인스턴스를 동시에 띄워도 충돌하지 않는다.
- **환경 변수**:
  - `GAMEROBOTFACTORY_DB_PATH` — SQLite 파일 경로. 기본값은 실행 디렉토리의 `gamerobotfactory.sqlite3`(`.gitignore`에 등록됨).
  - `RUST_LOG` — `tracing` 로그 레벨(예: `RUST_LOG=debug`). 미설정 시 기본값은 `info`.
- **상태**: 서버 프로세스 하나가 인메모리 시뮬레이션 상태 + SQLite 커넥션 하나를 갖는다 — 별도의 외부 DB나 캐시 서버는 필요 없다.
- **v1 스코프**: 단일 오퍼레이터 세션 전제(한 번에 하나의 클라이언트만 조작 권한을 가짐 — 관전용 다중 접속은 v2 백로그). 자세한 이유는 설계문서 참고.

## 프로토콜

WebSocket(`/ws`)과 REST가 역할을 분리한다 — WS는 실시간 게임 상태(20Hz), REST는 설정/통계처럼 느리게 바뀌는 데이터.

### WebSocket (`ws://127.0.0.1:<포트>/ws`)

접속하면 즉시 `Snapshot`(전체 상태 + 세션 ID)을 받고, 이후 20Hz로 `Delta`(바뀐 로봇만)를 받는다. 모든 서버 메시지는 `v` 필드로 프로토콜 버전(현재 `1`)을 명시한다.

**클라이언트 → 서버** (`type` 태그로 구분):

| 커맨드 | 필드 | 설명 |
|---|---|---|
| `SelectRobot` | `robot_id: number` | 조작 대상 로봇 선택 |
| `ReleaseRobot` | — | 선택 해제 |
| `ToggleConveyor` | — | 컨베이어 on/off |
| `SetRobotCount` | `count: number` | 로봇 수 조절 (상한 200으로 클램프) |
| `TriggerArmAction` | `robot_id: number, task: "Idle"\|"Picking"\|"Placing"` | 팔 동작 트리거 |
| `Resume` | `session_id: string(UUID)` | 30초 유예시간 내 재접속인지 확인 |

**서버 → 클라이언트** (`kind` 태그로 구분):

| 메시지 | 필드 | 설명 |
|---|---|---|
| `Snapshot` | `v, tick, session_id, conveyor, robots[]` | 최초 접속 시 + Lagged 리싱크 시 전체 상태 |
| `Delta` | `v, tick, conveyor?, changed_robots[], removed_robot_ids[]` | 매 틱, 바뀐 것만 |
| `ResumeAck` | `v, session_id, resumed: boolean` | `Resume` 커맨드에 대한 응답 |

잘못된 JSON이나 알 수 없는 커맨드는 로그로만 남고 연결을 끊지 않는다. 브로드캐스트 채널(버퍼 32개, ~1.6초분)을 넘겨 뒤처진 연결은 끊기지 않고 `Snapshot`으로 재동기화된다.

### REST

| 엔드포인트 | 설명 |
|---|---|
| `GET /health` | 헬스체크 |
| `GET`/`POST /api/config` | 런타임 설정(`persist_every_n_ticks`) 조회/변경. `0`은 400으로 거부 |
| `GET /api/stats/history` | SQLite에 주기적으로 쌓인 생산 통계 최근 50건 |
| `GET /metrics` | Prometheus 텍스트 포맷(`ticks_total`, `connected_clients`, `robot_count`, `tick_panics_total`, `tick_duration_seconds` 히스토그램) |

## 플레이 안내

터미널 두 개로 직접 확인해볼 수 있다. 하나는 서버, 하나는 클라이언트(아직 GUI가 없으므로 `wscat` 같은 CLI WS 클라이언트나 브라우저 개발자도구의 WebSocket 콘솔 사용):

```bash
# 1) 서버 실행 — LISTENING_PORT=54321 같은 줄이 뜬다
cargo run --manifest-path server/Cargo.toml

# 2) 접속 (wscat 예시: npm install -g wscat)
wscat -c ws://127.0.0.1:54321/ws
# → 접속 즉시 {"kind":"Snapshot", ..., "robots":[]} 수신 (아직 로봇 0마리)

# 3) 로봇 3마리 배치
> {"type":"SetRobotCount","count":3}
# → 다음 틱부터 {"kind":"Delta", "changed_robots":[...]} 로 로봇 3마리 등장

# 4) 컨베이어를 켜고 로봇 하나를 골라 작업시키기
> {"type":"ToggleConveyor"}
> {"type":"SelectRobot","robot_id":0}
> {"type":"TriggerArmAction","robot_id":0,"task":"Picking"}

# 5) 연결을 끊었다가(Ctrl+C) 30초 안에 재접속해 세션 이어가기
wscat -c ws://127.0.0.1:54321/ws
> {"type":"Resume","session_id":"<3)에서 받은 session_id>"}
# → {"kind":"ResumeAck","resumed":true}
```

다른 터미널에서 REST로 관측/설정도 함께 확인 가능:

```bash
curl http://127.0.0.1:54321/api/config
curl -X POST -H "Content-Type: application/json" \
  -d '{"persist_every_n_ticks":10}' http://127.0.0.1:54321/api/config
curl http://127.0.0.1:54321/api/stats/history
curl http://127.0.0.1:54321/metrics
```

## 지금까지 만든 것

- **Plan 1 — 결정적 시뮬레이션 코어**: 그리드/A* 경로탐색, 로봇 ID 기반 결정적 타이브레이크가 있는 병렬 틱, 로봇 하나가 패닉해도 나머지는 정상 갱신되는 격리, 트롯 보행 애니메이션, 2-본 IK(몸체 자세와 연결됨), 부동소수점 합산 순서에 영향받지 않는 생산량 집계.
- **Plan 2 — WS 프로토콜 & 네트워킹**: 실제로 뜨는 axum 서버, 커맨드 검증(존재하지 않는 로봇 거부, `SetRobotCount` 상한 클램프), 클라이언트별로 바뀐 로봇만 보내는 델타 동기화.
- **Plan 3 — 영속화 + REST API + 관측가능성 + 하드닝**: 세션 재접속 실배선(30초 유예시간) + 브로드캐스트 Lagged 리싱크(끊지 않고 스냅샷으로 재동기화), 틱 루프 패닉 격리(`safe_tick`), SQLite로 생산 통계 주기 영속화 + `GET /api/stats/history`, WS(실시간)와 분리된 `GET`/`POST /api/config`, `tracing` 구조화 로깅 + `/metrics` Prometheus 엔드포인트(틱 수/연결 수/로봇 수/틱 패닉 수). 통합테스트 전반에서 뮤테이션 테스트로 실제로 잡아낸 공허한 테스트를 재작성하는 등, 리뷰 과정에서 여러 실질 버그를 발견해 수정.
- **Plan 3~4 사이 보강**: 설계문서가 요구하는 틱 처리시간 p99 목표를 실제로 측정할 수 있도록 `tick_duration_seconds` 히스토그램 메트릭 추가.

**현재: 85개 테스트 통과, clippy 경고 0개.**

## 다음 단계

- **Plan 4**: 클라이언트 렌더링 (Vite+TS+Canvas, 아이소메트릭 투영) — 아직 `client/` 디렉토리 자체가 없다.
- **Plan 5**: 데모/배포 (Docker, 라이브 URL, 성능 목표 실측 — 측정 수단은 이제 있음).

상세 계획은 [`docs/superpowers/plans/`](docs/superpowers/plans/)에 있다.
