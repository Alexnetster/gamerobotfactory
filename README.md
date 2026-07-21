# 로봇팔 컨베이어 게임

4족 보행 로봇이 공장 컨베이어 라인을 돌며 작업을 돕는 팩토리 시뮬레이션. **백엔드/서버 설계 역량을 보여주는 포트폴리오 프로젝트**로, 게임성은 2차 목표다 — 결정적 시뮬레이션 코어, WebSocket 델타 동기화 프로토콜, 동시성 안전한 틱 루프가 이 프로젝트의 실제 핵심이다.

전체 설계 배경과 트레이드오프는 [`docs/robot-arm-conveyor-game-design.md`](docs/robot-arm-conveyor-game-design.md)에 있다.

**이어서 작업할 때는 이 문서 대신 [`docs/KANBAN.md`](docs/KANBAN.md)부터 봐야 한다** — 완료/진행/남은 작업이 커밋 SHA와 함께 정리돼 있고, "다음 카드"가 뭔지도 거기 나와 있다. 이 README는 외부 공개용 요약이라 최신 작업 상태 추적용으로는 KANBAN.md보다 부정확하다.

## 퀵스타트

**라이브 데모**: https://gamerobotfactory.fly.dev 를 열면 클론/설치 없이 바로 체험 가능(Fly.io에 배포된 실제 인스턴스 — 배포 절차 자체는 아래 "배포" 절 참고).

**로컬에서**: 저장소를 클론한 뒤

```bash
docker compose up
```

만 실행하면 `http://localhost:8080`에서 서버+클라이언트가 하나의 컨테이너로 뜬다.

## 핵심 엔지니어링 결정

- **결정적 병렬 틱**: 매 틱 `rayon`으로 로봇들을 병렬 갱신하되, 직전 틱의 스냅샷만 읽는 더블 버퍼링 + 로봇 ID 기반 타이브레이크로 동시 이동 충돌을 결정적으로 해소한다. 몇 번을 돌려도 같은 입력이면 같은 결과가 나온다 — `tick_is_deterministic`/`tick_never_produces_collisions` proptest로 검증.
- **장애 격리**: 로봇 한 대의 갱신 로직이 패닉해도(`catch_unwind`) 그 틱은 스킵될 뿐 나머지 로봇과 다른 클라이언트 연결에는 영향이 없다. `safe_tick`이 스킵된 틱 수를 `tick_panics_total` 메트릭으로 노출한다 — "조용히 멈추는" 실패를 관측 가능하게 만든 것.
- **클라이언트별 델타 동기화 + 무손실 재접속**: 매 틱 전체 상태 대신 바뀐 로봇만 보내고(`Delta`), 연결이 브로드캐스트 채널 용량을 넘겨 뒤처지면(Lagged) 끊는 대신 전체 스냅샷으로 재동기화한다. 세션 토큰(UUID)으로 30초 유예시간 내 재접속을 지원.
- **성능 목표를 실측 가능하게**: 틱 처리시간(락 획득~스냅샷/델타 계산)을 `tick_duration_seconds` Prometheus 히스토그램으로 노출한다 — 목표(p99 < 10ms, 20Hz 틱 예산 50ms)를 정확히 버킷 경계로 잡아둬서 `histogram_quantile`로 바로 조회 가능. "성능 목표를 문서에만 적어두고 측정 수단이 없는" 흔한 함정을 피한 것. 배포된 인스턴스 실측치가 필요하면 `node client/scripts/perf-check.mjs https://gamerobotfactory.fly.dev`로 직접 측정할 수 있다.
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
- 클라이언트는 `client/`에 있다(Vite+TS+Canvas2D, Node.js 필요). Node.js가 이미 있다면:

```bash
cd client
npm install
npm run dev
# → http://localhost:5173/?ws=ws://127.0.0.1:<서버가 알려준 포트>/ws 로 접속
```

```bash
# 서버 빌드
cargo build --manifest-path server/Cargo.toml

# 서버 테스트 전체 실행 (143개, 인테그레이션 테스트 일부는 실제 서버 바이너리를 띄워 수 초 소요)
cargo test --manifest-path server/Cargo.toml

# 서버 린트
cargo clippy --manifest-path server/Cargo.toml --all-targets
```

- Docker/Fly.io 배포 관련 파일도 저장소 루트/`client/`에 있다: `Dockerfile`(3단계 멀티스테이지 빌드), `docker-compose.yml`(로컬에서 배포 이미지와 동일하게 실행), `fly.toml`(Fly.io 배포 설정), `.dockerignore`. `client/scripts/`엔 애플리케이션 코드가 아니라 1회성 도구 스크립트가 있다: `record-demo.mjs`(Playwright로 데모 영상 자동 녹화), `perf-metrics.mjs`/`perf-metrics.test.mjs`(성능 메트릭 파싱 순수 함수 + `node --test` 유닛테스트), `perf-check.mjs`(배포된 URL에 실제로 접속해 성능을 실측하는 I/O 래퍼).

## 동작 환경

```bash
cargo run --manifest-path server/Cargo.toml
```

- **포트**: 고정 포트를 쓰지 않는다(`127.0.0.1:0`, OS가 임의 할당) — 실제 바인딩된 포트를 표준출력의 `LISTENING_PORT=<번호>` 줄로 알려준다. 여러 인스턴스를 동시에 띄워도 충돌하지 않는다. 단, Docker/배포 환경에서는 `GAMEROBOTFACTORY_BIND_ADDR`로 고정 포트를 쓴다(아래 '배포' 절 참고).
- **환경 변수**:
  - `GAMEROBOTFACTORY_DB_PATH` — SQLite 파일 경로. 기본값은 실행 디렉토리의 `gamerobotfactory.sqlite3`(`.gitignore`에 등록됨).
  - `RUST_LOG` — `tracing` 로그 레벨(예: `RUST_LOG=debug`). 미설정 시 기본값은 `info`.
  - `GAMEROBOTFACTORY_BIND_ADDR` — 바인드 주소. 기본값은 위와 같이 `127.0.0.1:0`이며, Docker/배포 환경에서는 `0.0.0.0:<고정포트>`로 오버라이드한다(`Dockerfile`에서 `0.0.0.0:8080`으로 설정).
- **상태**: 서버 프로세스 하나가 인메모리 시뮬레이션 상태 + SQLite 커넥션 하나를 갖는다 — 별도의 외부 DB나 캐시 서버는 필요 없다.
- **v1 스코프**: 단일 오퍼레이터 세션 전제(한 번에 하나의 클라이언트만 조작 권한을 가짐 — 관전용 다중 접속은 v2 백로그). 자세한 이유는 설계문서 참고.

## 배포

[Fly.io](https://fly.io)에 단일 컨테이너(Docker)로 배포한다. 이 저장소엔 앱이 이미 등록돼 있으므로(`fly.toml`, 앱 이름 `gamerobotfactory`, 리전 `nrt`) **0~1단계는 이 앱을 처음부터 새로 만드는 게 아니라면 건너뛰어도 된다** — 다른 계정/새 앱으로 배포하려는 경우에만 필요.

### 0) flyctl CLI 설치 (최초 1회, 컴퓨터당 1번)

| OS | 명령 |
|---|---|
| Windows (PowerShell) | `pwsh -Command "iwr https://fly.io/install.ps1 -useb \| iex"` |
| macOS | `brew install flyctl` |
| Linux | `curl -L https://fly.io/install.sh \| sh` |

설치 확인: `flyctl version`

### 1) Fly.io 로그인 (최초 1회, 계정당 1번)

```bash
flyctl auth login   # 브라우저가 열리며 로그인/가입(계정 없으면 무료로 만들 수 있음)
```

### 2) 앱 등록 (이 저장소는 이미 돼 있음 — 새 앱을 만들 때만)

```bash
flyctl launch --no-deploy   # fly.toml이 이미 있으므로 기존 설정 그대로 쓸지 물어보면 예
flyctl volumes create data --size 1   # 반드시 "data" — fly.toml의 [[mounts]] source와 일치해야 함
```

### 3) 배포

**수동으로 한 번 배포**:
```bash
flyctl deploy
```

**또는 자동배포(CI/CD, 권장)** — `.github/workflows/fly-deploy.yml`이 `main`에 push할 때마다 알아서 `flyctl deploy --remote-only`를 실행하도록 이미 구성돼 있다. 동작하려면 GitHub 저장소 시크릿에 `FLY_API_TOKEN`을 한 번만 등록하면 된다:

```bash
flyctl tokens create deploy   # 배포 전용 토큰 발급
```
발급된 토큰을 GitHub 저장소 **Settings → Secrets and variables → Actions → New repository secret**에 이름 `FLY_API_TOKEN`으로 등록한다. 이 시크릿이 없으면 `flyctl deploy` 단계가 인증 실패로 즉시(수 초 안에) 실패한다 — 실제로 최초 push(`2218dc0`)에서 이 이유로 실패한 이력이 있다(`docs/KANBAN.md` Backlog 참고). 등록 후에는 아무 커밋이나 `main`에 push하면(또는 GitHub Actions 탭에서 워크플로를 수동 재실행하면) 자동으로 재시도된다.

### 4) 접속 확인

이 앱의 주소는 **https://gamerobotfactory.fly.dev** 다(Fly.io는 `fly.toml`의 `app` 이름을 그대로 `<앱이름>.fly.dev` 도메인에 매핑한다 — `flyctl deploy`/`flyctl status` 출력에도 같은 주소가 찍힌다). 브라우저로 열면 바로 체험 가능하다(별도 쿼리 파라미터 불필요 — 클라이언트가 같은 오리진에서 자동으로 WS에 접속한다).

```bash
flyctl status   # 배포 상태 + 위 URL 재확인
flyctl logs     # 문제가 생기면 실시간 로그 확인
```

### 로컬에서 배포 이미지와 동일하게 실행 (Fly.io 계정 불필요)

```bash
docker compose up
```

`http://localhost:8080`에서 배포 환경과 동일한 빌드로 바로 체험 가능하다.

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
| `RepairRobot` | `robot_id: number` | 고장난(`Failed`) 로봇 수리 시작 (오퍼레이터 트리거) |
| `Resume` | `session_id: string(UUID)` | 30초 유예시간 내 재접속인지 확인 |

**서버 → 클라이언트** (`kind` 태그로 구분):

| 메시지 | 필드 | 설명 |
|---|---|---|
| `Snapshot` | `v, tick, session_id, conveyor, robots[]` | 최초 접속 시 + Lagged 리싱크 시 전체 상태 |
| `Delta` | `v, tick, conveyor?, changed_robots[], removed_robot_ids[]` | 매 틱, 바뀐 것만 |
| `ResumeAck` | `v, session_id, resumed: boolean` | `Resume` 커맨드에 대한 응답 |

`robots[]`/`changed_robots[]`의 각 로봇 항목(`RobotView`)은 `id, pos, pose, leg_cycle_progress, task`에 더해 다음 필드를 담는다:

- `path` — 현재 A* 경로(칸 좌표 배열). 렌더링에서는 디버그 오버레이용, 실제 이동은 서버가 이미 계산해 `pos`로 반영한다.
- `facing`(`North`/`East`/`South`/`West`) — 로봇이 실제로 이동한 마지막 방향. 타이브레이크에서 져서 제자리에 머문 로봇은 갱신되지 않는다.
- `arm_pose` — 몸체-팔 단일 기구학 체인의 순전파(forward kinematics) 결과(어깨/팔꿈치/손목 로컬 좌표). `task`가 `Idle`이면 고정된 서 있는 자세, 그 외에는 목표 지점을 향한 실제 2-본 IK(`solve_two_bone_ik`) 풀이 결과 — Plan 4 이전에는 이 IK 코드가 자기 자신의 유닛테스트에서만 호출되던 죽은 코드였고, 클라이언트가 처음으로 실제 런타임에서 이걸 소비한다.
- `status`(`Operational`/`Failed`/`Repairing{remaining_ticks}`)와 `durability_remaining`(내구도 잔량 0.0~1.0, 델타 대역폭 절감을 유지하려고 5% 단위로 양자화됨 — `Repairing.remaining_ticks`는 진행률 표시 가치가 커서 양자화하지 않음).

잘못된 JSON이나 알 수 없는 커맨드는 로그로만 남고 연결을 끊지 않는다. 브로드캐스트 채널(버퍼 32개, ~1.6초분)을 넘겨 뒤처진 연결은 끊기지 않고 `Snapshot`으로 재동기화된다.

### REST

| 엔드포인트 | 설명 |
|---|---|
| `GET /health` | 헬스체크 |
| `GET`/`POST /api/config` | 런타임 설정(`persist_every_n_ticks`) 조회/변경. `0`은 400으로 거부 |
| `GET /api/stats/history` | SQLite에 주기적으로 쌓인 생산 통계 최근 50건 |
| `GET /api/robots/failures` | SQLite에 쌓인 로봇 고장/수리완료 이력 최근 50건 |
| `GET /metrics` | Prometheus 텍스트 포맷(`ticks_total`, `connected_clients`, `robot_count`, `tick_panics_total`, `tick_duration_seconds` 히스토그램, `robot_failures_total`, `robots_repairing`) |

## 플레이 안내

### 브라우저 클라이언트로 (권장)

```bash
# 1) 서버 실행 — LISTENING_PORT=54321 같은 줄이 뜬다
cargo run --manifest-path server/Cargo.toml

# 2) 클라이언트 실행 (다른 터미널)
cd client
npm install   # 최초 1회
npm run dev
```

브라우저에서 `http://localhost:5173/?ws=ws://127.0.0.1:54321/ws`를 열면(포트 번호는 위 1번 단계에서 실제로 뜬 `LISTENING_PORT` 값으로 교체) 아이소메트릭 캔버스와 우측 사이드바가 뜬다. 사이드바의 `+`/`-`로 로봇 수 조절, 컨베이어 on/off 토글, 캔버스에서 로봇을 클릭해 선택 후 `Picking`/`Placing`/`Idle` 버튼으로 작업 지시, 고장(`Failed`)난 로봇은 `수리` 버튼으로 복구할 수 있다.

### WS 프로토콜을 직접 확인하고 싶다면

GUI 없이 프로토콜 자체를 눈으로 보고 싶을 때는 `wscat` 같은 CLI WS 클라이언트나 브라우저 개발자도구의 WebSocket 콘솔도 여전히 쓸 수 있다:

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
- **로봇 내구도/고장/복구**: 작업(픽/플레이스)마다 로봇 마모가 쌓이고, 마모율에 비례한 확률로 매 틱 고장(`Failed`) 여부를 판정한다 — 매 틱 `rayon`으로 로봇을 병렬 갱신하는 무공유 모델의 순수성을 지키려고 상태 있는 RNG 대신 (로봇 ID, 틱 번호)를 해싱하는 순수 함수로 필요할 때마다 같은 결과를 재계산할 수 있는 결정적 확률 판정을 쓴 것. 고장난 로봇은 이동이 멈추고, 오퍼레이터가 `RepairRobot` 커맨드로 수리를 트리거하면 일정 틱 후 자동 복구된다. `robot_failures_total`/`robots_repairing` 메트릭과 SQLite `robot_failure_events` 이력(`GET /api/robots/failures`)으로 관측 가능. 고장/복구 중인 로봇이 섞여 있어도 기존의 결정성·충돌-회피 proptest가 여전히 성립함을 별도 케이스로 검증.
- **Plan 4 — 클라이언트 렌더링**: Vite+TS+Canvas2D 웹 클라이언트(`client/`, 프레임워크 없이 vanilla DOM). 그리드 좌표를 아이소메트릭 투영으로 그리고(바닥/장식용 U자 컨베이어/z-order 정렬된 4족 로봇+팔), 서버 틱(20Hz)과 렌더 프레임레이트 차이를 선형 보간+짧은 외삽(최대 100ms)으로 매끄럽게 잇는다. 우측 사이드바로 컨베이어 on/off, 로봇 수 조절, 로봇 선택 후 팔 동작/수리 지시가 가능. 이 작업이 서버 쪽 `RobotView`에 `path`/`facing`/`arm_pose`를 실제로 배선한 계기가 됐고, Plan 1 때부터 있었지만 자기 테스트에서만 불리던 `ik.rs`/`posture.rs`가 처음으로 실제 런타임에서 호출되기 시작했다. 3계층 테스트로 검증: vitest 단위(순수 로직, 뮤테이션 테스트 병행), vitest 통합(실 서버 바이너리 스폰 + 진짜 `ws` 클라이언트), Playwright E2E(실 서버 + 빌드된 클라이언트 + 실제 Chromium, 캔버스 픽셀 샘플링과 DOM 단언). 리뷰 과정에서 실제 버그 여러 건을 잡았다 — 재접속 타이머가 명시적 `close()` 이후에도 취소 안 되던 레이스, `<canvas>`의 자동 min-width 때문에 사이드바가 화면 밖으로 밀려나던 flex 레이아웃 버그(E2E로만 잡을 수 있었음), 픽셀 좌표가 팔 색과 우연히 겹쳐 공허했던 E2E 테스트, Windows에서 `process.kill()` 직후 SQLite 파일 핸들이 안 풀려 생기던 `EBUSY` 정리 실패. 로봇 50대로 실측한 평균 프레임 시간은 약 16.2~16.3ms(≈61~62fps, 실제 서버+`npm run dev` 클라이언트+Chromium으로 30프레임 표본 3회 측정) — 설계문서 성능 목표(60fps 근처, 16.7ms)를 만족.
- **Plan 5 — 데모/배포**: 서버+클라이언트를 단일 Docker 컨테이너로 패키징(서버가 바인드 주소를 `GAMEROBOTFACTORY_BIND_ADDR`로 환경변수화하고, `tower-http::ServeDir`로 클라이언트 빌드 산출물까지 서빙 — 지금까지는 서버가 `127.0.0.1:0` 임의 포트에만 바인드하고 정적 파일을 서빙할 방법이 아예 없어 컨테이너화 자체가 불가능했다), 클라이언트는 같은 오리진에서 WS 주소를 자동 유도해 `?ws=` 쿼리 파라미터 없이도 배포 환경에서 바로 접속되게 했다. 그 위에 `fly.toml`(Fly.io 배포 설정), CI `docker build` 스모크 잡, Playwright로 데모 영상을 자동 녹화하는 스크립트(`client/scripts/record-demo.mjs`), 배포된 URL을 인자로 받아 실제 성능(로봇 수/틱 수/`tick_duration_seconds` p99)을 실측하는 스크립트(`client/scripts/perf-check.mjs`, 파싱 로직은 `node --test`로 별도 유닛테스트)를 갖췄다. 데모 영상을 녹화하는 과정에서 이 프로젝트의 핵심 증명 포인트(결정적 병렬 경로탐색+충돌회피)가 실제로는 전혀 눈에 안 보인다는 걸 발견했다 — 실제 커맨드 체계 어디에도 로봇에게 이동 목표를 주는 기능이 없어서, 라이브 서버를 띄워도 로봇이 스폰된 자리에서 한 발짝도 안 움직였던 것. `sim_core`에 로봇 id로부터 결정적으로 계산되는 두 지점 사이를 영원히 왕복하는 순찰 목표 배정(`patrol_points`/`next_patrol_goal`)을 추가해 해소했고, 그 여파로 "로봇은 스폰 위치에서 안 움직인다"를 전제하던 Plan 4의 Playwright E2E 스위트가 조용히 깨진 것을 전체 회귀 검증 중 발견해 다시 고쳤다 — 몸체색 평균 검증 테스트는 데모 녹화 스크립트와 같은 캔버스 픽셀 스캔 방식을 연결-성분(최대 블롭) 필터로 강화했고, 클릭 선택 테스트는 헤드리스 크로미움의 캔버스 리드백이 실제 상태보다 뒤처지는 문제 때문에 픽셀 스캔 자체를 버리고 실제 서버 권위 상태(병렬 WS 연결 + 프로덕션 미러/보간/투영 모듈을 직접 import)로 클릭 좌표를 계산하는 방식으로 바꿨다(구현자 20회 연속 + 독립 리뷰어 15회 연속 실행 전부 통과로 검증 — 최초 수정 때의 "3회 연속 통과" 판단은 이후 독립 재검증에서 6/8로 드러나 틀렸었다).
- **로봇 외형 리디자인**: 라이브 데모를 직접 확인한 사용자 피드백("로봇 같지 않다")을 반영해 클라이언트 렌더링을 평평한 사각형+일자 다리에서 Spot 스타일 4족 로봇으로 다시 그렸다. 바닥 타일은 기존 아이소메트릭 투영을 유지하되 로봇 캐릭터만 3/4 정면 각도로 그려 실루엣을 뚜렷하게 하고, 다리 4개는 엉덩이→무릎→발을 하나의 연속된 stroke path로 그려 이음매 없이 몸통에 부착시켰다(비주얼 컴패니언 목업 반복 중 뒷다리가 몸통에서 떨어져 보이는 실제 버그를 발견해 수정). 걸음은 디딤(60%, 무릎 편 상태)/흔듦(40%, 무릎 굽힘)을 비대칭 타이밍으로 나눈 순수 함수(`client/src/render/gait.ts`)로 매핑해 "미끄러지듯 이동한다"는 인상을 줄였다. 서버/프로토콜 변경 없음.

**현재: 서버 145개 테스트 통과 + 클라이언트 vitest 단위 64개/통합 3개, Playwright E2E 2개, `node --test` 성능 스크립트 파싱 테스트 4개 통과. clippy(`--all-targets`, `-D warnings` 없이)는 이 플랜과 무관한 기존 unknown-lint 경고 1건(`sim.rs:285`, 상세는 KANBAN Backlog)만 남고 그 외 0개 — `-D warnings`로 돌리면 그 알림이 컴파일 에러로 승격돼 빌드 자체가 실패한다.**

## 다음 단계

Plan 1~5로 계획됐던 작업은 전부 완료됐고, https://gamerobotfactory.fly.dev 로 실제 배포도 끝났다. 남은 개선 아이디어는 [`docs/KANBAN.md`](docs/KANBAN.md)의 Backlog 절 참고.

상세 계획은 [`docs/superpowers/plans/`](docs/superpowers/plans/)에 있다.
