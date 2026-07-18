# 데모/배포 설계 (Plan 5)

브레인스토밍 세션(2026-07-18)에서 확정된 설계. Docker 단일 컨테이너로 서버+클라이언트를 패키징하고, Fly.io 배포 설정과 README 퀵스타트를 준비하며, Playwright로 데모 영상을 자동 녹화하고, 성능 목표 실측 절차(부하 스크립트)를 만든다.

## 왜 이 기능인가

Plan 1~4로 "서버 권위 + 결정적 코어 + 델타 동기화 + 관측가능성 + 클라이언트 렌더링"이 전부 만들어졌지만, 마스터 설계문서의 "발표/데모 전략"(line 126-133)이 요구하는 마지막 조각 — 리뷰어가 로컬 클론 없이 브라우저에서 바로 체험할 수 있는 라이브 URL, 30초짜리 데모 영상, 실측 성능 수치 — 은 아직 없다. 이게 없으면 지금까지 만든 것의 상당수(재접속, 델타 동기화, IK/보행)가 "코드는 있지만 아무도 못 보는" 상태로 남는다.

## 스코프 (v1)

- **서버 코드 변경**: 바인드 주소 환경변수화, 정적 파일 서빙(빌드된 클라이언트를 같은 컨테이너/포트에서 서빙).
- **클라이언트 코드 변경**: WS 접속 대상을 같은 오리진에서 자동 유도(라이브 URL에 `?ws=` 파라미터를 수동으로 안 붙여도 되게).
- **Docker**: 단일 컨테이너 멀티스테이지 `Dockerfile` + 로컬 실행용 `docker-compose.yml`.
- **Fly.io 배포 설정**: `fly.toml` + README 배포 안내. **실제 `flyctl deploy` 실행과 계정 설정은 사용자 몫**(브레인스토밍에서 확정된 스코프 경계).
- **CI 보강**: `docker build`가 계속 되는지 확인하는 스모크 잡.
- **데모 영상**: Playwright로 로컬 `docker compose up` 대상 자동 녹화(mp4/webm — 이 환경엔 ffmpeg가 없어 GIF 변환은 안 함, 스코프 밖).
- **성능 실측**: 원격 URL을 인자로 받는 부하 스크립트 + `/metrics` 파싱(순수 함수, 유닛테스트 대상) + 실행 절차 문서화. **실제 실행(배포된 URL 대상)은 배포 완료 후**.
- **README**: 퀵스타트, 배포 안내, 데모 영상 삽입, 실측치 자리 표시(배포 후 채움).
- **서버 — 로봇 순찰 목표 배정**(Task 7 데모 녹화 중 발견해 추가): 실제 커맨드 체계 어디에도 로봇에게 목표(`goal`)를 주는 기능이 없어서, 라이브로 서버를 띄워도 로봇이 절대 걸어다니지 않는다는 사실이 데모 영상 녹화 과정에서 발견됨(아래 "서버 쪽 추가 변경" 절 참고). 이 프로젝트의 핵심 증명 포인트(결정적 병렬 경로탐색+충돌회피)가 라이브 데모에서 전혀 안 보이는 문제라, 작은 순찰 기능을 추가하기로 확정(사용자 결정, 2026-07-19).

## 서버 쪽 추가 변경 — 로봇 순찰 목표 배정 (Task 7 진행 중 발견)

### 발견된 사실

`server/src/*.rs` 전체를 검색한 결과, `Robot.goal` 필드는 `Robot::new`(스폰 시점, `pos`와 동일하게 설정)와 테스트 코드 외에는 어디에서도 설정되지 않는다. 클라이언트 커맨드(`SelectRobot`/`ReleaseRobot`/`ToggleConveyor`/`SetRobotCount`/`TriggerArmAction`/`RepairRobot`/`Resume`) 중 로봇에게 이동 목표를 주는 것은 하나도 없다 — Plan 1의 결정적 A* 경로탐색/충돌회피는 proptest의 합성 시나리오(테스트가 직접 `goal`을 다르게 설정)로만 검증됐을 뿐, 실제 서버를 띄우고 조작하는 경로로는 한 번도 트리거되지 않는다.

### 설계 결정: 컨베이어 상태와 무관한 순수 순찰

`SimState`(`sim_core`)엔 컨베이어 개념이 아예 없다 — 컨베이어는 `game_state.rs`(서버 레이어)에만 존재한다(`lib.rs`의 "sim_core는 네트워크 의존성 없는 순수 라이브러리" 원칙과 일치). 순찰을 컨베이어 on/off에 연동하려면 `SimState`에 새 필드를 추가해 레이어 경계를 넘어야 하므로, v1은 **컨베이어 상태와 무관하게 항상 순찰**하는 것으로 범위를 최소화한다 — `sim_core` 안에서만 완결되는 변경이라 `game_state.rs`/`protocol.rs`는 건드릴 필요가 없다.

각 로봇은 자신의 id로부터 결정적으로 계산되는 두 지점(A, B) 사이를 오간다 — 목표에 도착할 때마다 반대쪽 지점으로 재배정한다:

```rust
// server/src/sim.rs
/// 로봇 id로부터 결정적으로 계산되는 순찰 지점 두 개. 그리드 폭/높이
/// 각각의 절반만큼(둘 중 1보다 큰 축만) 떨어뜨려서 두 지점이 항상
/// 서로 다르다는 걸 보장한다 — 실제 그리드 크기(프로덕션 10x10)뿐 아니라
/// 기존 유닛테스트가 쓰는 가늘고 긴 그리드(예: 5x1)에서도 안전하도록
/// 한쪽 축이 1이면(=이동 불가 축) 그 축은 안 건드리고 다른 축만으로
/// 구분한다.
fn patrol_points(id: u32, grid: &Grid) -> (CellId, CellId) {
    let w = grid.width.max(1);
    let h = grid.height.max(1);
    let a = ((id as i32 * 7).rem_euclid(w), (id as i32 * 3).rem_euclid(h));
    let dx = if w > 1 { w / 2 } else { 0 };
    let dy = if h > 1 { h / 2 } else { 0 };
    let b = ((a.0 + dx).rem_euclid(w), (a.1 + dy).rem_euclid(h));
    (a, b)
}

/// 로봇이 목표에 도착했을 때 다음 순찰 목표를 계산한다 — 현재 목표가
/// A면 B로, 그 외(B거나 스폰 시점의 초기 goal==pos)엔 A로.
fn next_patrol_goal(robot: &Robot, grid: &Grid) -> CellId {
    let (a, b) = patrol_points(robot.id, grid);
    if robot.goal == a { b } else { a }
}
```

`plan_robot`의 기존 "목표에 도착하면 그대로 반환" 분기(`if next.pos == next.goal { return next; }`)를 다음으로 교체 — 반환하지 않고 새 목표로 갱신한 뒤 그대로 흘려보내서, 같은 틱에 바로 새 목표를 향한 경로탐색이 시작될 수 있게 한다(로봇 내구도 기능의 "복구 완료 즉시 이동" 동틱 처리와 같은 패턴, 이미 이 프로젝트에서 의도적으로 받아들인 동작):

```rust
    if next.pos == next.goal {
        next.goal = next_patrol_goal(&next, grid);
    }
```

### 기존 테스트와의 충돌 — 의도적으로 재작성

이 변경은 "목표에 도착하면 정지한다"는 옛 의미를 "목표(순찰 지점)에 도착하면 다음 순찰 지점으로 향한다"로 바꾸므로, 그 옛 의미를 직접 검증하던 기존 테스트 2개가 새 동작과 충돌한다:

- `robot_stops_moving_once_at_goal` — 로봇을 목표 지점에 스폰(`pos==goal`)하고 한 틱 뒤에도 안 움직였는지 확인하던 테스트. 이제는 그 즉시 새 순찰 목표로 재배정되므로 더 이상 성립하지 않는다.
- `leg_cycle_progress_does_not_advance_once_at_goal` — 같은 이유로 충돌.

두 테스트 모두 "제자리 정지"가 아니라 "순찰 목표 재배정"을 검증하도록 재작성한다(아래 구현 계획 참고) — 낡은 assertion을 그냥 지우는 게 아니라, 바뀐 의도를 정확히 표현하는 새 assertion으로 바꾸는 것.

### 결정성 유지

`patrol_points`/`next_patrol_goal` 둘 다 `robot.id`/`robot.goal`/`grid` 크기만으로 계산되는 순수 함수라 `rand`류 상태 있는 RNG를 전혀 안 쓴다 — 기존 `tick_is_deterministic`/`tick_never_produces_collisions` proptest가 이 변경 후에도 그대로 성립해야 한다(구현 계획에서 재실행 확인).

## 스코프 밖

- 실제 `flyctl deploy` 실행, 실제 라이브 URL 발급(사용자 계정/결제 정보 필요 — 브레인스토밍에서 명시적으로 제외)
- GIF 변환(ffmpeg 필요 — 이 환경에 없음, 필요하면 후속으로)
- Postgres 전환(이미 마스터 설계문서 v2 백로그)
- Fly.io 외 다른 호스트(Railway 등) 설정 파일 — Fly.io로 확정(브레인스토밍에서 선택)

## 서버 쪽 변경

### 1) 바인드 주소 환경변수화

현재 `server/src/main.rs:320-322`가 `"127.0.0.1:0"`(루프백+임의 포트)에 하드코딩돼 있다 — 테스트 병렬 격리를 위한 의도적 설계였지만, 컨테이너 안에서는 외부에서 전혀 접근 불가능하다(컨테이너 네트워크 네임스페이스의 루프백은 호스트/외부에서 안 보이고, 포트가 임의라 `EXPOSE`/포트 매핑도 불가능).

환경 변수를 직접 `main()` 안에서 읽지 않고, 값을 파라미터로 받는 순수 함수로 분리한다 — env var를 함수 내부에서 직접 읽으면 유닛테스트에서 프로세스 전역 상태(env var)를 건드리게 되고, Rust 테스트는 기본적으로 병렬 실행되므로 다른 테스트와 레이스가 생길 수 있다(이 프로젝트가 이미 알고 있는 함정 패턴).

```rust
// server/src/main.rs
fn resolve_bind_addr(env_value: Option<&str>) -> String {
    env_value.unwrap_or("127.0.0.1:0").to_string()
}
```

`main()`에서:

```rust
let bind_addr = resolve_bind_addr(std::env::var("GAMEROBOTFACTORY_BIND_ADDR").ok().as_deref());
let listener = tokio::net::TcpListener::bind(&bind_addr)
    .await
    .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));
```

기본값이 기존과 동일(`127.0.0.1:0`)이므로 로컬 개발/기존 테스트(`ws_integration.rs`/`rest_integration.rs`의 `spawn_server`, 클라이언트 `spawn-server.ts`)는 전부 영향받지 않는다. Docker/배포 환경에서만 `GAMEROBOTFACTORY_BIND_ADDR=0.0.0.0:8080` 등으로 오버라이드한다.

### 2) 정적 파일 서빙 (단일 컨테이너의 핵심)

`server/Cargo.toml`에 `tower-http = { version = "0.5", features = ["fs"] }` 추가.

같은 패턴으로 정적 디렉토리 경로도 파라미터화:

```rust
fn resolve_static_dir(env_value: Option<&str>) -> String {
    env_value.unwrap_or("client/dist").to_string()
}
```

`Router` 구성(`server/src/main.rs:274-286`)에 `.fallback_service(ServeDir::new(static_dir))` 추가 — 기존 `/health`/`/ws`/`/api/*`/`/metrics` 라우트에 안 걸리는 모든 요청(즉 `/`, `/assets/*` 등 클라이언트 정적 파일)이 이 서비스로 간다. `ServeDir`는 지연 평가라 디렉토리가 없어도 생성 시점에 패닉하지 않고 요청마다 404를 돌려줄 뿐이다 — `client/dist/`가 없는 로컬 `cargo run`에서도 서버가 깨지지 않는다(그냥 `/`가 404가 될 뿐, 기존 REST/WS 엔드포인트는 전혀 영향 없음).

### 테스트

- `resolve_bind_addr`/`resolve_static_dir` 유닛테스트: `None` → 기본값, `Some("x")` → 그 값 그대로. env var 자체를 건드리지 않으므로 병렬 테스트 안전.
- 기존 통합테스트(`ws_integration.rs` 등)가 여전히 그대로 통과하는지 확인 — 기본값 유지 검증.
- 신규 통합테스트 하나: 서버를 `GAMEROBOTFACTORY_STATIC_DIR`를 임시 디렉토리(더미 `index.html` 하나 있는)로 지정해 띄우고, `GET /`가 그 파일 내용을 돌려주는지 + `/health`는 여전히 정상인지 확인(정적 서빙 추가가 기존 API 라우트를 안 밀어냈다는 것의 증거).

## 클라이언트 쪽 변경 — WS URL 같은 오리진 자동 유도

지금 `client/src/main.ts::resolveWsUrl()`는 `?ws=` 쿼리 파라미터가 없으면 명확한 에러 메시지만 보여주고 끝난다(Plan 4에서 의도된 동작 — 서버가 항상 임의 포트였으므로 추측할 기본값이 없었다). 배포 환경에서는 서버가 클라이언트 정적 파일과 `/ws`를 **같은 오리진(같은 호스트:포트)**에서 서빙하므로, 이제는 안전하게 기본값을 유도할 수 있다.

기존 DI 원칙(Task 8의 `Connection`이 브라우저 API를 주입받은 것과 동일)을 따라, 브라우저 전역을 직접 읽지 않고 파라미터로 받는 순수 함수로 만든다:

```ts
// client/src/main.ts
export function resolveWsUrl(search: string, protocol: string, host: string): string {
  const override = new URLSearchParams(search).get('ws')
  if (override) {
    return override
  }
  const wsProtocol = protocol === 'https:' ? 'wss' : 'ws'
  return `${wsProtocol}://${host}/ws`
}
```

호출부: `resolveWsUrl(location.search, location.protocol, location.host)`. `?ws=` 오버라이드는 그대로 유지되므로(로컬 `npm run dev`처럼 클라이언트와 서버 포트가 다른 경우), 기존 Task 12/13의 통합/E2E 테스트(둘 다 `?ws=`를 명시적으로 붙여서 접속)는 전혀 영향받지 않는다. 배포된 단일 컨테이너에서는 `?ws=` 없이 그냥 URL만 열어도 같은 호스트의 `/ws`로 자동 접속된다 — "이 URL을 열면 바로 체험 가능"이라는 요구사항이 이걸로 충족된다.

### 테스트

`resolveWsUrl`에 대한 유닛테스트: `?ws=` 있으면 그 값 우선(https든 아니든), 없고 `protocol=https:`면 `wss://host/ws`, 없고 `protocol=http:`면 `ws://host/ws`.

## Docker 패키징

### `Dockerfile` (멀티스테이지, 단일 컨테이너)

```dockerfile
# ---- Stage 1: Rust 서버 빌드 ----
FROM rust:1.85-bookworm AS server-builder
WORKDIR /build
COPY server/Cargo.toml server/Cargo.lock ./
COPY server/src ./src
RUN cargo build --release

# ---- Stage 2: 클라이언트 빌드 ----
FROM node:22-bookworm-slim AS client-builder
WORKDIR /build
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/index.html client/vite.config.ts client/tsconfig.json ./
COPY client/src ./src
RUN npm run build

# ---- Stage 3: 런타임(슬림) ----
FROM debian:bookworm-slim
WORKDIR /app
COPY --from=server-builder /build/target/release/server ./server
COPY --from=client-builder /build/dist ./client-dist

ENV GAMEROBOTFACTORY_BIND_ADDR=0.0.0.0:8080
ENV GAMEROBOTFACTORY_STATIC_DIR=/app/client-dist
ENV GAMEROBOTFACTORY_DB_PATH=/data/gamerobotfactory.sqlite3
ENV RUST_LOG=info

EXPOSE 8080
VOLUME ["/data"]
ENTRYPOINT ["./server"]
```

`rusqlite`의 `bundled` 기능이 SQLite를 소스에서 컴파일하므로 빌드 스테이지에 C 컴파일러가 필요하다 — `rust:1.85-bookworm`(슬림 아님)은 이미 `build-essential`을 포함하므로 별도 설치가 필요 없다. 런타임 스테이지는 컴파일된 바이너리+정적 파일만 복사하므로 최종 이미지에는 Rust/Node 툴체인이 전혀 안 남는다.

### `docker-compose.yml` (로컬 실행)

```yaml
services:
  app:
    build: .
    ports:
      - "8080:8080"
    volumes:
      - gamerobotfactory-data:/data
    environment:
      - RUST_LOG=info

volumes:
  gamerobotfactory-data:
```

`docker compose up` 한 줄로 `http://localhost:8080`에서 바로 체험 가능 — SQLite 파일은 named volume에 저장돼 컨테이너를 내렸다 올려도 유지된다.

### CI 보강

`.github/workflows/rust-ci.yml`에 세 번째 잡 추가:

```yaml
  docker:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build Docker image
        run: docker build -t gamerobotfactory:ci .
```

Dockerfile이 썩는 것(의존성 버전 변경, 빌드 스텝 깨짐)을 회귀 없이 잡아낸다 — 실행(`docker run`)까지는 안 하고 빌드만 확인(포트/볼륨 검증은 로컬 `docker compose up`으로 사람이 확인).

## Fly.io 배포 설정

`fly.toml`:

```toml
app = "gamerobotfactory"  # 실제 배포 시 본인 앱 이름으로 교체
primary_region = "nrt"    # 실제 배포 시 원하는 리전으로 교체

[build]

[http_service]
  internal_port = 8080
  force_https = true
  auto_stop_machines = false
  auto_start_machines = true
  min_machines_running = 1

[[mounts]]
  source = "gamerobotfactory_data"
  destination = "/data"

[env]
  GAMEROBOTFACTORY_DB_PATH = "/data/gamerobotfactory.sqlite3"
  RUST_LOG = "info"
```

`min_machines_running = 1`/`auto_stop_machines = false`로 항상 켜둔다 — 무료 티어의 자동 절전은 WebSocket 장시간 연결과 "데모가 바로 반응해야 한다"는 요구사항에 안 맞는다(브레인스토밍에서 확정된 이유). README에 `flyctl volumes create gamerobotfactory_data --size 1`(최초 1회) + `flyctl deploy` 안내를 추가하되, 실행 자체는 사용자 몫.

## 데모 영상 — Playwright 자동 녹화

로컬 `docker compose up`으로 띄운 실제 컨테이너를 대상으로 녹화한다(배포 없이도 완결 가능, 배포 아티팩트와 동일한 이미지를 검증하는 효과도 있음).

`client/scripts/record-demo.mjs`(신규, `client/tests/`가 아니라 `client/scripts/` — 애플리케이션 테스트가 아니라 마케팅 자산 생성 스크립트이므로 vitest/playwright 설정 대상에서 제외. 저장소 루트가 아니라 `client/` 안에 두는 이유: Node의 모듈 해석은 스크립트 파일 자신의 경로 기준으로 `node_modules`를 찾으므로, 저장소 루트에 두면 `client/node_modules`의 `@playwright/test`/`ws`를 못 찾는다 — 계획 단계에서 실제로 재현 확인):

```js
import { chromium } from 'playwright'

const BASE_URL = process.argv[2] ?? 'http://localhost:8080'

async function main() {
  const browser = await chromium.launch()
  const context = await browser.newContext({
    viewport: { width: 1000, height: 700 },
    recordVideo: { dir: 'demo-recordings', size: { width: 1000, height: 700 } },
  })
  const page = await context.newPage()

  await page.goto(BASE_URL) // ?ws= 없이 — 같은 오리진 자동 유도 확인
  await page.waitForSelector('.connection-status')

  const incButton = page.locator('.sidebar button', { hasText: '+' })
  for (let i = 0; i < 5; i++) {
    await incButton.click()
    await page.waitForTimeout(300)
  }

  await page.locator('.sidebar button', { hasText: '컨베이어' }).click()
  await page.waitForTimeout(2000) // 보행/경로탐색 움직임이 화면에 보이도록

  const canvas = page.locator('canvas')
  const box = await canvas.boundingBox()
  await page.mouse.click(box.x + box.width / 2, box.y + 40)
  await page.locator('button', { hasText: 'Picking' }).click() // 팔 IK 동작
  await page.waitForTimeout(1500)

  // 재접속 시나리오: 네트워크를 잠깐 끊었다 복구
  await context.setOffline(true)
  await page.waitForTimeout(1000)
  await context.setOffline(false)
  await page.waitForSelector('.connection-status:has-text("연결됨")', { timeout: 10000 })
  await page.waitForTimeout(1000)

  await context.close()
  await browser.close()
  console.log('recorded to demo-recordings/')
}

main()
```

실행(저장소 루트에서): `docker compose up -d` → `node client/scripts/record-demo.mjs` → `docker compose down`. 산출물은 `demo-recordings/*.webm`(Playwright 기본 포맷) — README에 상대경로로 링크하거나, 필요시 사용자가 직접 mp4/GIF로 변환(이 환경엔 ffmpeg가 없어 자동 변환은 스코프 밖).

## 성능 실측

### 부하 스크립트

`client/scripts/perf-check.mjs`(신규) — 원격 URL을 인자로 받아 로봇 수를 늘리고 `/metrics`를 폴링한다. 파싱 로직은 순수 함수로 분리해 유닛테스트 대상으로 삼는다(Node 내장 `node --test`로, 저장소 루트가 아니라 `client/scripts/`에 두는 이유는 위와 동일):

```js
// client/scripts/perf-metrics.mjs — 순수 함수, 유닛테스트 대상
export function parseTickDurationP99(metricsText) {
  // Prometheus 히스토그램 텍스트에서 gamerobotfactory_tick_duration_seconds_bucket 라인들을 파싱해
  // 누적 카운트 기준 99번째 백분위에 해당하는 버킷 상한(le)을 근사치로 반환한다.
  const buckets = []
  for (const line of metricsText.split('\n')) {
    const match = /^gamerobotfactory_tick_duration_seconds_bucket\{le="([^"]+)"\}\s+(\d+)/.exec(line)
    if (match) {
      buckets.push({ le: match[1] === '+Inf' ? Infinity : Number(match[1]), count: Number(match[2]) })
    }
  }
  if (buckets.length === 0) return null
  buckets.sort((a, b) => a.le - b.le)
  const total = buckets[buckets.length - 1].count
  const target = total * 0.99
  const p99Bucket = buckets.find((b) => b.count >= target)
  return p99Bucket ? p99Bucket.le : null
}
```

`client/scripts/perf-check.mjs`(I/O 래퍼, 순수 함수 재사용):

```js
import { parseTickDurationP99 } from './perf-metrics.mjs'

const BASE_URL = process.argv[2]
if (!BASE_URL) {
  console.error('사용법: node client/scripts/perf-check.mjs <배포된 URL>')
  process.exit(1)
}

async function main() {
  const ws = new (await import('ws')).default(`${BASE_URL.replace(/^http/, 'ws')}/ws`)
  await new Promise((resolve, reject) => { ws.once('open', resolve); ws.once('error', reject) })
  ws.send(JSON.stringify({ type: 'SetRobotCount', count: 50 }))
  ws.close()

  console.log('로봇 50대 반영 대기(10초)...')
  await new Promise((r) => setTimeout(r, 10000))

  const metricsText = await (await fetch(`${BASE_URL}/metrics`)).text()
  const p99 = parseTickDurationP99(metricsText)
  const robotCountMatch = /gamerobotfactory_robot_count (\d+)/.exec(metricsText)
  console.log(`robot_count=${robotCountMatch?.[1]}, tick_duration_seconds p99 근사치=${p99}s (목표: <0.01s)`)
}

main()
```

### 테스트

`parseTickDurationP99`는 순수 함수라 고정된 Prometheus 텍스트 샘플(버킷 여러 개, `+Inf` 포함)로 유닛테스트한다 — 실제 `/metrics` 응답 형식은 이미 `server/src/metrics.rs`가 정의하므로 그 포맷을 그대로 고정 샘플로 사용.

### 실행 절차 (배포 완료 후)

README에 다음을 명시(저장소 루트에서 실행): `node client/scripts/perf-check.mjs https://<배포된 URL>` 실행 → 출력된 `robot_count`/p99 값을 README "성능 목표" 절에 실측치로 기록. **이 스크립트 자체는 이번 세션에서 완성하지만, 실제 실행(배포된 URL 대상)은 배포 완료 후 진행** — 사용자가 배포 URL을 알려주면 그때 같이 실행해서 README에 반영한다.

## README 갱신 항목

- **퀵스타트**: "이 URL을 열면 바로 체험 가능" (배포 URL은 실제 배포 후 채움) + `docker compose up` 한 줄 로컬 실행법.
- **배포 안내**: `flyctl launch`/`flyctl volumes create`/`flyctl deploy` 절차, `fly.toml` 참고.
- **데모 영상**: `demo-recordings/`에 녹화된 영상 링크(또는 GitHub에 임베드 가능한 형식으로 안내).
- **성능 목표**: 서버 p99(Plan 3~4 사이 보강분, 이미 있음)에 클라이언트 프레임 시간(Plan 4 실측)과 이번에 배포 환경에서 실측한 p99/틱레이트를 나란히 기록(배포 후 채움).

## 테스트 전략 요약

1. **서버 유닛(Rust)**: `resolve_bind_addr`/`resolve_static_dir` — env 값 유무에 따른 분기, 프로세스 env var 안 건드림(병렬 안전).
2. **서버 통합(Rust)**: 정적 파일 서빙이 기존 API 라우트를 안 밀어내는지 실제 서버 기동 + `GET /`/`GET /health` 둘 다 확인.
3. **클라이언트 유닛(vitest)**: `resolveWsUrl`의 3가지 분기(오버라이드/https/http).
4. **CI**: `docker build` 스모크 잡.
5. **성능 스크립트 유닛(vitest 또는 plain node — 결정은 계획 단계에서)**: `parseTickDurationP99`를 고정 Prometheus 텍스트 샘플로 검증.
6. **수동/1회성**: `docker compose up` 로컬 실행 확인, Playwright 데모 녹화 실행(산출물 존재 확인), 배포 후 `perf-check.mjs` 실행.

## 문서 갱신 의무

- README(퀵스타트/배포/데모/성능 — 위 항목).
- `docs/robot-arm-conveyor-game-design.md`의 "발표/데모 전략" 절에 실제로 어떻게 구현됐는지 각주 추가(Plan 4 때와 같은 패턴).
- `docs/KANBAN.md` Plan 5 항목을 Done으로.
