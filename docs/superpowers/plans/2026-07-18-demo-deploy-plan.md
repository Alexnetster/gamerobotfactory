# 데모/배포 구현 계획 (Plan 5)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 서버+클라이언트를 단일 Docker 컨테이너로 패키징하고, Fly.io 배포 설정과 CI 스모크 테스트를 갖추고, Playwright로 데모 영상을 자동 녹화하고, 배포 후 실행할 성능 실측 스크립트를 준비한다.

**Architecture:** 서버에 바인드 주소 환경변수화 + 정적 파일 서빙(클라이언트 빌드 산출물)을 추가해 "단일 컨테이너, 단일 포트"를 가능하게 하고, 클라이언트는 같은 오리진에서 WS 주소를 자동 유도하도록 고친다. 그 위에 멀티스테이지 `Dockerfile`, `docker-compose.yml`, `fly.toml`, CI `docker build` 잡, Playwright 데모 녹화 스크립트, 원격 URL 대상 성능 스크립트를 쌓는다.

**Tech Stack:** Rust(서버, 기존) + `tower-http`(정적 파일 서빙, 신규), TypeScript(클라이언트, 기존), Docker/Docker Compose, Fly.io, Playwright(기존 devDependency 재사용), Node 스크립트(신규, `scripts/`).

**설계 근거:** `docs/superpowers/specs/2026-07-18-demo-deploy-design.md` (이 계획의 모든 결정은 그 문서를 따른다)

---

### Task 1: 서버 — 바인드 주소 환경변수화

**Files:**
- Modify: `server/src/main.rs`

- [ ] **Step 1: 실패하는 테스트 작성**

`server/src/main.rs`의 `#[cfg(test)] mod tests` 블록 끝에 추가:

```rust
    #[test]
    fn resolve_bind_addr_defaults_to_loopback_random_port_when_unset() {
        assert_eq!(resolve_bind_addr(None), "127.0.0.1:0");
    }

    #[test]
    fn resolve_bind_addr_uses_env_value_when_set() {
        assert_eq!(resolve_bind_addr(Some("0.0.0.0:8080")), "0.0.0.0:8080");
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test --manifest-path server/Cargo.toml resolve_bind_addr`
Expected: FAIL — `error[E0425]: cannot find function 'resolve_bind_addr' in this scope`

- [ ] **Step 3: `resolve_bind_addr` 함수 추가 + `main()` 배선**

`server/src/main.rs`의 `pub fn build_app(...)` 함수(266-287번째 줄) 바로 뒤, 기존 `main()` 앞 doc comment(289-292번째 줄) 앞에 추가:

```rust
/// 바인드 주소를 env var 값에서 직접 읽지 않고 파라미터로 받는 순수
/// 함수로 분리한다 — `main()` 안에서 `std::env::var`를 직접 부르면
/// 유닛테스트가 프로세스 전역 상태(env var)를 건드리게 되고, Rust
/// 테스트는 기본적으로 병렬 실행되므로 다른 테스트와 레이스가 생길 수
/// 있다. 기본값은 기존 동작과 동일한 `127.0.0.1:0`(로컬 개발/테스트에서
/// 여러 서버 인스턴스를 동시에 띄워도 포트 충돌이 없도록) — Docker/배포
/// 환경에서만 `GAMEROBOTFACTORY_BIND_ADDR`로 오버라이드한다.
fn resolve_bind_addr(env_value: Option<&str>) -> String {
    env_value.unwrap_or("127.0.0.1:0").to_string()
}
```

`main()`(294-325번째 줄)에서 다음 두 줄을 교체:

```rust
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind 127.0.0.1:0");
```

다음으로:

```rust
    let bind_addr = resolve_bind_addr(std::env::var("GAMEROBOTFACTORY_BIND_ADDR").ok().as_deref());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {bind_addr}: {e}"));
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: PASS (전체 스위트 — 기존 135개 + 새 2개 = 137개)

Run: `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`
Expected: 경고 0개

- [ ] **Step 5: 기존 통합테스트가 기본값 유지로 영향받지 않는지 재확인**

Run: `cargo test --manifest-path server/Cargo.toml --test ws_integration --test rest_integration`
Expected: PASS — `GAMEROBOTFACTORY_BIND_ADDR`를 아무도 안 건드리므로 기존처럼 `127.0.0.1:<임의포트>`로 계속 뜬다.

- [ ] **Step 6: 커밋**

```bash
git add server/src/main.rs
git commit -m "feat: 서버 바인드 주소를 GAMEROBOTFACTORY_BIND_ADDR로 환경변수화"
```

---

### Task 2: 서버 — 정적 파일 서빙 (단일 컨테이너의 핵심)

**Files:**
- Modify: `server/Cargo.toml`
- Modify: `server/src/main.rs`
- Create: `server/tests/static_serving_integration.rs`

- [ ] **Step 1: 의존성 추가**

`server/Cargo.toml`의 `[dependencies]` 블록(14-24번째 줄)에 추가:

```toml
tower-http = { version = "0.5", features = ["fs"] }
```

- [ ] **Step 2: 실패하는 유닛테스트 작성**

`server/src/main.rs`의 `#[cfg(test)] mod tests` 블록 끝에 추가:

```rust
    #[test]
    fn resolve_static_dir_defaults_to_client_dist_when_unset() {
        assert_eq!(resolve_static_dir(None), "client/dist");
    }

    #[test]
    fn resolve_static_dir_uses_env_value_when_set() {
        assert_eq!(resolve_static_dir(Some("/app/client-dist")), "/app/client-dist");
    }
```

- [ ] **Step 3: 테스트 실패 확인**

Run: `cargo test --manifest-path server/Cargo.toml resolve_static_dir`
Expected: FAIL — `error[E0425]: cannot find function 'resolve_static_dir' in this scope`

- [ ] **Step 4: `resolve_static_dir` 추가 + `build_app`/`main()` 배선**

`server/src/main.rs` 상단 `use` 절에 추가:

```rust
use tower_http::services::ServeDir;
```

`resolve_bind_addr` 함수(Task 1에서 추가됨) 바로 뒤에 추가:

```rust
/// 클라이언트 빌드 산출물(`client/dist/`)을 서빙할 디렉토리 경로도 같은
/// 이유(env var 직접 읽기 금지, 병렬 테스트 안전)로 파라미터화한다.
/// 기본값 `client/dist`는 로컬 `cargo run`을 저장소 루트에서 실행할 때
/// 상대경로로 맞는 값 — Docker에서는 `GAMEROBOTFACTORY_STATIC_DIR`로
/// 절대경로(`/app/client-dist`)를 넘긴다.
fn resolve_static_dir(env_value: Option<&str>) -> String {
    env_value.unwrap_or("client/dist").to_string()
}
```

`pub fn build_app(...)`(266-287번째 줄)를 교체 — `static_dir` 파라미터 추가 + `.fallback_service(...)` 추가:

```rust
pub fn build_app(
    state: SharedState,
    broadcaster: Broadcaster,
    sessions: ws::SessionHandle,
    db: DbHandle,
    config: ConfigHandle,
    metrics: MetricsHandle,
    static_dir: String,
) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ws", get(ws_route))
        .route("/api/stats/history", get(stats_history))
        .route("/api/robots/failures", get(robot_failures))
        .route("/api/config", get(get_config).post(post_config))
        .route("/metrics", get(metrics_route))
        .fallback_service(ServeDir::new(static_dir))
        .with_state(state)
        .layer(axum::extract::Extension(broadcaster))
        .layer(axum::extract::Extension(sessions))
        .layer(axum::extract::Extension(db))
        .layer(axum::extract::Extension(config))
        .layer(axum::extract::Extension(metrics))
}
```

`main()`에서 `build_app` 호출부를 교체:

```rust
    let static_dir = resolve_static_dir(std::env::var("GAMEROBOTFACTORY_STATIC_DIR").ok().as_deref());
    let app = build_app(state, broadcaster, sessions, db, config, metrics, static_dir);
```

- [ ] **Step 5: 유닛테스트 통과 확인**

Run: `cargo test --manifest-path server/Cargo.toml --lib resolve_static_dir`
Expected: PASS

- [ ] **Step 6: 정적 서빙 통합테스트 작성**

`server/tests/static_serving_integration.rs` (기존 `ws_integration.rs`/`rest_integration.rs`와 동일한 `ServerProcess`/포트-announce 패턴 — 파일마다 이 헬퍼를 복제하는 게 이 코드베이스의 기존 관례):

```rust
use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

struct ServerProcess {
    child: Child,
    port: u16,
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn spawn_server_with_static_dir(db_path: &std::path::Path, static_dir: &std::path::Path) -> ServerProcess {
    let mut child = Command::new(env!("CARGO_BIN_EXE_server"))
        .env("GAMEROBOTFACTORY_DB_PATH", db_path)
        .env("GAMEROBOTFACTORY_STATIC_DIR", static_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to start server binary");

    let stdout: ChildStdout = child.stdout.take().expect("child stdout was not piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).expect("failed to read announce line from server stdout");
    let port: u16 = line
        .trim()
        .strip_prefix("LISTENING_PORT=")
        .unwrap_or_else(|| panic!("unexpected server announce line: {line:?}"))
        .parse()
        .expect("LISTENING_PORT value was not a valid port number");

    ServerProcess { child, port }
}

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("gamerobotfactory-static-test-{name}-{}", uuid::Uuid::new_v4()))
}

#[tokio::test]
async fn serves_static_files_and_still_answers_health() {
    let db_path = temp_path("db").with_extension("sqlite3");
    let static_dir = temp_path("dist");
    std::fs::create_dir_all(&static_dir).expect("failed to create temp static dir");
    std::fs::write(static_dir.join("index.html"), "<html>hello from static test</html>")
        .expect("failed to write temp index.html");

    let server = spawn_server_with_static_dir(&db_path, &static_dir);
    let base = format!("http://127.0.0.1:{}", server.port);

    let index_body = reqwest::get(format!("{base}/")).await.unwrap().text().await.unwrap();
    assert!(index_body.contains("hello from static test"));

    let health_status = reqwest::get(format!("{base}/health")).await.unwrap().status();
    assert!(health_status.is_success());

    let _ = std::fs::remove_dir_all(&static_dir);
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn missing_static_dir_returns_404_without_crashing_other_routes() {
    // static_dir 자체가 없어도(로컬 cargo run에서 client/dist를 안 만든
    // 경우와 동일한 상황) 서버가 안 죽고 기존 API는 정상 동작해야 한다.
    let db_path = temp_path("db-missing").with_extension("sqlite3");
    let nonexistent_static_dir = temp_path("does-not-exist");

    let server = spawn_server_with_static_dir(&db_path, &nonexistent_static_dir);
    let base = format!("http://127.0.0.1:{}", server.port);

    let root_status = reqwest::get(format!("{base}/")).await.unwrap().status();
    assert_eq!(root_status.as_u16(), 404);

    let health_status = reqwest::get(format!("{base}/health")).await.unwrap().status();
    assert!(health_status.is_success());

    let _ = std::fs::remove_file(&db_path);
}
```

- [ ] **Step 7: 통합테스트 실행**

Run: `cargo test --manifest-path server/Cargo.toml --test static_serving_integration`
Expected: PASS (2개 테스트)

- [ ] **Step 8: 전체 회귀 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: PASS (137개 + 이번에 추가한 2개 유닛 + 2개 통합 = 141개)

Run: `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`
Expected: 경고 0개

- [ ] **Step 9: 커밋**

```bash
git add server/Cargo.toml server/Cargo.lock server/src/main.rs server/tests/static_serving_integration.rs
git commit -m "feat: tower-http ServeDir로 클라이언트 정적 파일 서빙 추가"
```

---

### Task 3: 클라이언트 — WS URL 같은 오리진 자동 유도

**Files:**
- Create: `client/src/net/resolve-ws-url.ts`
- Create: `client/tests/unit/resolve-ws-url.test.ts`
- Modify: `client/src/main.ts`

`resolveWsUrl`은 `main.ts`(모듈 최하단에서 `main()`을 즉시 실행하는 부트스트랩 파일)가 아니라 별도 모듈로 분리한다 — 그래야 `main.ts`를 import할 때 딸려오는 부작용(`main()` 즉시 실행, `document.getElementById` 등 브라우저 전역 접근) 없이 순수 함수만 유닛테스트할 수 있다.

- [ ] **Step 1: 실패하는 테스트 작성**

`client/tests/unit/resolve-ws-url.test.ts`:

```ts
import { describe, it, expect } from 'vitest'
import { resolveWsUrl } from '../../src/net/resolve-ws-url'

describe('resolveWsUrl', () => {
  it('uses the ?ws= override when present, regardless of protocol', () => {
    expect(resolveWsUrl('?ws=ws://127.0.0.1:54321/ws', 'http:', 'localhost:5173')).toBe('ws://127.0.0.1:54321/ws')
  })

  it('derives wss:// from the same origin when protocol is https and no override is given', () => {
    expect(resolveWsUrl('', 'https:', 'gamerobotfactory.fly.dev')).toBe('wss://gamerobotfactory.fly.dev/ws')
  })

  it('derives ws:// from the same origin when protocol is http and no override is given', () => {
    expect(resolveWsUrl('', 'http:', 'localhost:8080')).toBe('ws://localhost:8080/ws')
  })
})
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cd client && npm test`
Expected: FAIL — `Cannot find module '../../src/net/resolve-ws-url'`

- [ ] **Step 3: `net/resolve-ws-url.ts` 구현**

```ts
// client/src/net/resolve-ws-url.ts
//
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
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd client && npm test`
Expected: PASS

- [ ] **Step 5: `main.ts` 배선 갱신**

`client/src/main.ts` 상단 import 절(1-11번째 줄)에 추가:

```ts
import { resolveWsUrl } from './net/resolve-ws-url'
```

`function resolveWsUrl(): string | null { ... }`(17-19번째 줄, 로컬 정의)를 **삭제** — 이제 `net/resolve-ws-url.ts`의 것을 쓴다.

`function main(): void`의 시작 부분(56-68번째 줄)을 교체:

```ts
function main(): void {
  const wsUrl = resolveWsUrl(location.search, location.protocol, location.host)
  const { canvas, sidebarContainer } = setupLayout()
  const ctx2d = canvas.getContext('2d')
  if (!ctx2d) {
    throw new Error('2D canvas context unavailable')
  }
  const ctx: CanvasRenderingContext2D = ctx2d

  let mirror: MirrorState = createEmptyMirror()
```

(`if (!wsUrl) { ... return }` 블록을 삭제한다 — `resolveWsUrl`이 이제 항상 유효한 문자열을 반환하므로 그 분기 자체가 불필요해졌다.)

- [ ] **Step 6: 타입체크 + 빌드 확인**

Run: `cd client && npm run typecheck`
Expected: 에러 없음(미사용 지역 함수/조건 분기를 다 지웠는지 확인 — `noUnusedLocals`가 켜져 있어 지우지 않으면 여기서 에러가 난다)

Run: `npm run build`
Expected: 에러 없음

- [ ] **Step 7: 커밋**

```bash
git add client/src/net/resolve-ws-url.ts client/tests/unit/resolve-ws-url.test.ts client/src/main.ts
git commit -m "feat: WS 접속 주소를 같은 오리진에서 자동 유도(배포 환경 지원)"
```

---

### Task 4: Docker 패키징

**Files:**
- Create: `Dockerfile` (저장소 루트)
- Create: `docker-compose.yml` (저장소 루트)
- Create: `.dockerignore` (저장소 루트)

- [ ] **Step 1: `.dockerignore` 작성**

```
server/target
client/node_modules
client/dist
client/test-results
client/playwright-report
client/tsconfig.tsbuildinfo
.git
.worktrees
```

- [ ] **Step 2: `Dockerfile` 작성**

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

- [ ] **Step 3: `docker-compose.yml` 작성**

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

- [ ] **Step 4: 로컬 빌드 + 실행 검증**

Run: `docker compose build`
Expected: 3단계 빌드 전부 성공, 에러 없음

Run: `docker compose up -d`
Expected: 컨테이너 기동

Run (몇 초 대기 후): `curl -s http://localhost:8080/health`
Expected: `ok`

Run: `curl -s http://localhost:8080/ | head -c 200`
Expected: 클라이언트 `index.html` 내용(빈 응답이나 404가 아님)

Run: `curl -s -X POST -H "Content-Type: application/json" -d '{"persist_every_n_ticks":10}' http://localhost:8080/api/config`
Expected: 200 OK — REST API가 정적 서빙 추가 후에도 정상 동작

브라우저에서 `http://localhost:8080` 접속 — `?ws=` 파라미터 없이 사이드바에 🟢 연결됨이 뜨는지 육안 확인(Task 3의 같은 오리진 자동 유도가 실제로 동작하는 증거).

Run: `docker compose down -v` (검증 끝나면 볼륨까지 정리)

- [ ] **Step 5: 커밋**

```bash
git add Dockerfile docker-compose.yml .dockerignore
git commit -m "chore: 단일 컨테이너 Docker 패키징(Dockerfile+docker-compose)"
```

---

### Task 5: CI — Docker 빌드 스모크 잡

**Files:**
- Modify: `.github/workflows/rust-ci.yml`

- [ ] **Step 1: `docker` 잡 추가**

`.github/workflows/rust-ci.yml`의 `jobs:` 블록 끝에 추가:

```yaml
  docker:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build Docker image
        run: docker build -t gamerobotfactory:ci .
```

- [ ] **Step 2: 로컬에서 CI와 동일한 커맨드로 재확인**

Run: `docker build -t gamerobotfactory:ci .`
Expected: 성공(Task 4에서 이미 `docker compose build`로 확인했지만, CI가 실제로 쓰는 `docker build` 커맨드 그대로 한 번 더 확인)

- [ ] **Step 3: 커밋**

```bash
git add .github/workflows/rust-ci.yml
git commit -m "chore(ci): Docker 이미지 빌드 스모크 잡 추가"
```

---

### Task 6: Fly.io 배포 설정

**Files:**
- Create: `fly.toml` (저장소 루트)
- Modify: `README.md`

- [ ] **Step 1: `fly.toml` 작성**

```toml
app = "gamerobotfactory"  # 실제 배포 시 본인 Fly.io 앱 이름으로 교체
primary_region = "nrt"    # 실제 배포 시 원하는 리전으로 교체(예: nrt=도쿄)

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

- [ ] **Step 2: README에 배포 안내 절 추가**

`README.md`의 "동작 환경" 절(현재 로컬 `cargo run` 안내가 있는 부분) 뒤에 새 절 "## 배포" 추가:

```markdown
## 배포

[Fly.io](https://fly.io)에 단일 컨테이너로 배포하도록 `fly.toml`을 준비해뒀다. 실제 배포는 본인 Fly.io 계정으로 진행한다:

\`\`\`bash
# 최초 1회
flyctl launch --no-deploy   # fly.toml이 이미 있으므로 기존 설정 그대로 사용할지 물어보면 예
flyctl volumes create gamerobotfactory_data --size 1

# 배포
flyctl deploy
\`\`\`

배포가 끝나면 `flyctl status`로 나온 URL을 열면 바로 체험 가능하다(별도 쿼리 파라미터 불필요 — 클라이언트가 같은 오리진에서 자동으로 WS에 접속한다).

### 로컬에서 배포 이미지와 동일하게 실행

\`\`\`bash
docker compose up
\`\`\`

`http://localhost:8080`에서 배포 환경과 동일한 빌드로 바로 체험 가능하다.
```

- [ ] **Step 3: 커밋**

```bash
git add fly.toml README.md
git commit -m "docs: Fly.io 배포 설정(fly.toml) + README 배포 안내 추가"
```

---

### Task 7: 서버 — 로봇 순찰 목표 배정

**Files:**
- Modify: `server/src/sim.rs`

Task 8(데모 영상 녹화) 도중 발견된 문제를 고친다: 실제 커맨드 체계 어디에도 로봇에게 목표(`goal`)를 주는 기능이 없어서, 라이브 서버를 띄워도 로봇이 걸어다니지 않는다 — 이 프로젝트의 핵심 증명 포인트(결정적 병렬 경로탐색+충돌회피)가 데모에서 전혀 안 보인다. 설계 근거: `docs/superpowers/specs/2026-07-18-demo-deploy-design.md`의 "서버 쪽 추가 변경 — 로봇 순찰 목표 배정" 절.

각 로봇은 자신의 id로부터 결정적으로 계산되는 두 지점(A, B) 사이를 영원히 왕복한다 — 목표에 도착할 때마다 반대쪽 지점으로 재배정한다. `SimState`는 컨베이어를 모르므로(`game_state.rs`에만 존재) 순찰은 컨베이어 on/off와 무관하게 항상 동작한다.

이 변경은 기존 테스트 3개와 충돌한다("도착하면 정지"라는 옛 전제가 깨지므로):
- `robot_stops_moving_once_at_goal`(`server/src/sim.rs:343-350`) → `robot_picks_a_new_patrol_goal_and_moves_on_the_same_tick_it_arrives`로 교체
- `leg_cycle_progress_does_not_advance_once_at_goal`(`server/src/sim.rs:413-420`) → `leg_cycle_progress_advances_when_patrol_reassignment_causes_movement`로 교체
- `facing_holds_last_direction_while_stationary`(`server/src/sim.rs:601-625`) → 이름은 유지하되 마지막 단계를 goal 수동 대입 대신 `RobotStatus::Failed`로 바꿔 재작성(도착 즉시 순찰 재배정되어 다시 움직이므로, "완전히 멈춤"을 보장하려면 Failed가 필요함)

- [ ] **Step 1: 실패하는 테스트 작성 (2개 신규 + 3개 교체)**

`server/src/sim.rs`의 `mod tests` 블록에 다음 2개를 새로 추가한다(위치는 임의 — 다른 `#[test]` 사이):

```rust
    #[test]
    fn patrol_points_are_always_distinct_for_a_reasonably_sized_grid() {
        let grid = Grid::new(10, 10);
        for id in 0..20u32 {
            let (a, b) = patrol_points(id, &grid);
            assert_ne!(a, b, "patrol points must differ for id {id}");
        }
    }

    #[test]
    fn next_patrol_goal_alternates_between_the_two_patrol_points() {
        let grid = Grid::new(10, 10);
        let mut robot = Robot::new(1, (0, 0), (0, 0));
        let (a, b) = patrol_points(1, &grid);
        robot.goal = a;
        assert_eq!(next_patrol_goal(&robot, &grid), b);
        robot.goal = b;
        assert_eq!(next_patrol_goal(&robot, &grid), a);
    }
```

`server/src/sim.rs:342-350`의 기존 `robot_stops_moving_once_at_goal` 전체를 다음으로 교체:

```rust
    #[test]
    fn robot_picks_a_new_patrol_goal_and_moves_on_the_same_tick_it_arrives() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (2, 0), (2, 0)));

        let next = tick(&state);

        assert_ne!(next.robots[0].goal, (2, 0), "arriving at a patrol point should immediately assign the next one");
        assert_eq!(next.robots[0].pos, (3, 0), "the robot should already be moving toward the new patrol goal");
    }
```

`server/src/sim.rs:412-420`의 기존 `leg_cycle_progress_does_not_advance_once_at_goal` 전체를 다음으로 교체:

```rust
    #[test]
    fn leg_cycle_progress_advances_when_patrol_reassignment_causes_movement() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (2, 0), (2, 0)));

        let next = tick(&state);

        assert!(next.robots[0].leg_cycle_progress > 0.0, "moving toward the new patrol goal should advance the gait cycle");
    }
```

`server/src/sim.rs:601-625`의 기존 `facing_holds_last_direction_while_stationary` 전체를 다음으로 교체(앞부분 두 단계는 그대로, 마지막 단계만 Failed로 바꿈):

```rust
    #[test]
    fn facing_holds_last_direction_while_stationary() {
        let mut state = simple_state(5, 1);
        state.robots.push(Robot::new(1, (0, 0), (3, 0)));
        state = tick(&state); // 동쪽으로 한 칸 이동 -> facing = East
        assert_eq!(state.robots[0].facing, Direction::East);

        // 목표를 직접 바꿀 때는 남아 있는 경로/재계획 타이머도 함께 지워야 한다
        // — 그러지 않으면 plan_robot()이 새 목표를 무시하고 옛 경로(동쪽)를
        // 계속 따라간다. 실제 프로덕션 코드에는 이렇게 goal만 단독으로
        // 바꾸는 경로가 없다(Robot::new에서 한 번만 설정됨) — 이 테스트가
        // 그 시나리오를 시뮬레이션하려면 tick()의 타이브레이크 패배 분기와
        // 동일하게 경로를 초기화해줘야 한다.
        state.robots[0].goal = (0, 0); // 이제 서쪽으로
        state.robots[0].path.clear();
        state.robots[0].ticks_until_repath = 0;
        state = tick(&state);
        assert_eq!(state.robots[0].facing, Direction::West);

        // 정지 상태에서도 마지막 방향을 유지해야 한다 — 이제 "목표 도착"은
        // 곧바로 다음 순찰 목표로 재배정되어 다시 움직이므로 더 이상
        // "정지"를 의미하지 않는다. 진짜로 멈춘 상태를 만들려면 Failed로
        // 만든다 — plan_robot()이 이동/재계획/순찰 재배정을 전부 건너뛴다.
        state.robots[0].status = RobotStatus::Failed;
        let held = tick(&state);
        assert_eq!(held.robots[0].facing, Direction::West);
    }
```

- [ ] **Step 2: 테스트 실패 확인**

Run: `cargo test --manifest-path server/Cargo.toml patrol_points`
Expected: FAIL — `error[E0425]: cannot find function 'patrol_points' in this scope`

- [ ] **Step 3: `patrol_points`/`next_patrol_goal` 구현 + `plan_robot` 배선**

`server/src/sim.rs:230`(`fn plan_robot(...)` 바로 앞)에 추가:

```rust
/// 로봇 id로부터 결정적으로 계산되는 순찰 지점 두 개. 그리드 폭/높이 중
/// 1보다 큰 축만 절반만큼 떨어뜨려서 두 지점이 항상 서로 다르다는 걸
/// 보장한다 — 실제 그리드 크기(프로덕션 10x10)뿐 아니라 기존
/// 유닛테스트가 쓰는 가늘고 긴 그리드(예: 5x1)에서도 안전하다.
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

`server/src/sim.rs:241-243`의 기존 코드:

```rust
    if next.pos == next.goal {
        return next;
    }
```

다음으로 교체(반환하지 않고 새 목표로 갱신한 뒤 흘려보내서, 같은 틱에 바로 새 목표를 향한 경로탐색이 시작되게 한다 — 로봇 내구도 기능의 "복구 완료 즉시 이동" 동틱 처리와 같은 패턴):

```rust
    if next.pos == next.goal {
        next.goal = next_patrol_goal(&next, grid);
    }
```

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: PASS — 전체 141개 중 2개 교체 대상 이름이 사라지고 5개(교체 3 + 신규 2)가 새로 들어와 순증가 +2, 총 **143개 PASS**.

Run: `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`
Expected: 경고 0개

- [ ] **Step 5: 결정성/충돌회피 proptest 재확인**

Run: `cargo test --manifest-path server/Cargo.toml --test tick_properties`
Expected: PASS — `patrol_points`/`next_patrol_goal` 둘 다 `robot.id`/`robot.goal`/`grid` 크기만으로 계산되는 순수 함수라 `tick_is_deterministic`/`tick_never_produces_collisions`가 이 변경 후에도 그대로 성립해야 한다.

- [ ] **Step 6: 커밋**

```bash
git add server/src/sim.rs
git commit -m "feat: sim_core에 로봇 순찰 목표 배정 추가 — 결정적 왕복 이동"
```

---

### Task 8: 데모 영상 — Playwright 자동 녹화

**Files:**
- Create: `client/scripts/record-demo.mjs`

이 스크립트는 애플리케이션 테스트가 아니라 마케팅 자산(데모 영상)을 생성하는 1회성 도구다 — `client/tests/`가 아니라 `client/scripts/`에 두고, vitest/playwright 설정 대상에서 제외한다(`client/vitest.config.ts`/`client/playwright.config.ts`는 `tests/**`만 본다).

**중요**: `client/` **밖**(저장소 루트)에 두지 않는다 — Node의 모듈 해석은 실행 시점의 작업 디렉토리(cwd)가 아니라 **스크립트 파일 자신의 경로**를 기준으로 `node_modules`를 찾아 올라간다. 저장소 루트 `scripts/`에 두면 `client/node_modules`의 `@playwright/test`를 절대 못 찾는다(실제로 재현해서 확인함 — `cd client && node ../scripts/x.mjs`로 실행해도 `ERR_MODULE_NOT_FOUND`). `client/scripts/`에 두면 파일 자신의 조상 디렉토리에 `client/node_modules`가 있으므로 저장소 루트에서 실행하든 어디서 실행하든 항상 올바르게 resolve된다.

- [ ] **Step 1: 스크립트 작성**

**중요한 설계 결정 (Task 8 최초 실행에서 발견 후 수정)**: 재접속 시나리오는 `context.setOffline(true/false)`로 만들면 안 된다 — Playwright/CDP의 `setOffline`은 새 네트워크 요청만 막을 뿐 **이미 열려 있는 WebSocket 연결은 닫지 않는다**(실제로 재현해서 확인: 영상을 프레임 단위로 뜯어봐도 연결 끊김 UI가 전혀 나타나지 않음). 실제로 서버와의 연결을 끊으려면 서버 프로세스 자체를 내렸다 올려야 한다 — `docker compose stop/start app`으로 컨테이너를 재시작해 클라이언트의 재연결 로직(지수 백오프 재시도)이 실제로 트리거되게 한다. 이 방식이 더 현실적이기도 하다(실제 배포 환경에서 재접속이 일어나는 원인은 서버 재배포/재시작이지, 클라이언트 쪽 네트워크 단절이 아니다).

`client/scripts/record-demo.mjs`:

```js
import { chromium } from '@playwright/test'
import { execFile } from 'node:child_process'
import { promisify } from 'node:util'

const run = promisify(execFile)
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
  await page.waitForTimeout(3000) // 순찰 로봇의 보행/경로탐색 움직임이 화면에 충분히 보이도록

  const canvas = page.locator('canvas')
  const box = await canvas.boundingBox()
  if (!box) {
    throw new Error('canvas has no bounding box')
  }
  await page.mouse.click(box.x + box.width / 2, box.y + 40)
  // 사이드바의 선택 패널은 서버 틱(20Hz)마다 innerHTML을 통째로 재생성하므로
  // 버튼이 ~50ms마다 detach/재생성된다 — locator.click()은 "안정된 상태"를
  // 기다리다 타임아웃하므로(실제로 재현 확인), page.evaluate로 그 순간
  // DOM에 있는 버튼에 직접 클릭 이벤트를 디스패치해 이 경쟁을 우회한다.
  await page.evaluate(() => {
    const button = Array.from(document.querySelectorAll('button')).find((b) => b.textContent === 'Picking')
    if (!button) {
      throw new Error('Picking button not found — is a robot selected?')
    }
    button.click()
  }) // 팔 IK 동작
  await page.waitForTimeout(1500)

  // 재접속 시나리오: 컨테이너를 실제로 내렸다 올려서 WS 연결이 진짜로 끊기게 한다
  // (setOffline은 이미 열린 WS를 안 닫으므로 쓸 수 없음 — 위 설계 결정 참고).
  await run('docker', ['compose', 'stop', 'app'])
  await page.waitForSelector('.connection-status:has-text("재연결")', { timeout: 10000 })
  await page.waitForTimeout(1000)
  await run('docker', ['compose', 'start', 'app'])
  await page.waitForSelector('.connection-status:has-text("연결됨")', { timeout: 15000 })
  await page.waitForTimeout(1000)

  await context.close()
  await browser.close()
  console.log('recorded to demo-recordings/')
}

main()
```

- [ ] **Step 2: 산출물 디렉토리를 저장소 루트 `.gitignore`에 추가**

`recordVideo.dir`(`demo-recordings`)은 Playwright가 **프로세스 cwd** 기준 상대경로로 해석한다 — 스크립트를 저장소 루트에서 실행하므로(Step 3) `demo-recordings/`는 저장소 루트에 생긴다. 저장소 루트 `.gitignore`(현재 Rust 전용 항목만 있음)에 추가:

```
demo-recordings/
```

- [ ] **Step 3: 실행해서 실제 데모 영상 생성**

Run: `docker compose up -d` (Task 4에서 만든 이미지 재사용) — 스크립트 자신은 `docker compose stop/start app`(재접속 시나리오)만 호출하고 최초 기동/최종 정리는 안 하므로, 실행 전후로 직접 올리고 내려야 한다.
Run (미설치 시): `cd client && npx playwright install chromium`
Run (**저장소 루트에서** — `docker compose` 하위 명령이 `docker-compose.yml`을 cwd 기준으로 찾으므로 반드시 저장소 루트에서 실행): `node client/scripts/record-demo.mjs`
Expected: `demo-recordings/*.webm` 파일 생성(저장소 루트 기준), 콘솔에 에러 없음
Run: `docker compose down`

생성된 영상을 재생해서 육안으로 확인 — 이전 시도(Task 8 최초 실행, 커밋 `bac3657`)에서 셀프리뷰만으로는 두 가지 실제 결함(로봇이 안 움직임, 재접속 UI가 안 보임)을 놓쳤던 전례가 있으므로, "스크립트가 에러 없이 끝났다"가 아니라 **영상 프레임을 실제로 확인**해서 다음이 전부 실제로 화면에 보이는지 검증한다: 로봇 배치 → 순찰 보행/경로탐색으로 로봇이 실제로 이동 → 컨베이어 토글 → 팔 Picking 동작 → 연결 끊김(🔴 재연결 중... 표시) → 복구(🟢 연결됨).

- [ ] **Step 4: 커밋**

`record-demo.mjs`는 이미 이전 시도(커밋 `bac3657`)에서 생성됐으므로, 이번 변경은 재접속 시나리오 수정 + 로봇 순찰 반영이다:

```bash
git add client/scripts/record-demo.mjs
git commit -m "fix: 데모 녹화 스크립트 — setOffline 대신 컨테이너 재시작으로 재접속 시나리오 수정"
```

(영상 파일 자체는 `.gitignore`에 있으므로 커밋 대상 아님 — README에는 로컬에 생성된 파일 경로를 안내하거나, 별도로 호스팅한 링크를 붙인다.)

---

### Task 9: 성능 실측 스크립트

**Files:**
- Create: `client/scripts/perf-metrics.mjs`
- Create: `client/scripts/perf-metrics.test.mjs`
- Create: `client/scripts/perf-check.mjs`

파싱 로직(순수 함수)과 네트워크 I/O를 분리한다 — 순수 함수만 유닛테스트 대상으로 삼는다(이 프로젝트의 일관된 원칙).

**중요한 설계 결정 1**: Task 8과 같은 이유로 `scripts/`는 저장소 루트가 아니라 **`client/scripts/`**에 둔다 — `perf-check.mjs`가 `ws` 패키지를 import하는데, Node의 모듈 해석은 스크립트 파일 자신의 경로를 기준으로 `node_modules`를 찾아 올라가지 cwd를 보지 않는다(직접 재현해서 확인: 저장소 루트 `scripts/`에 두고 `cd client && node ../scripts/x.mjs`로 실행해도 `client/node_modules`를 못 찾고 `ERR_MODULE_NOT_FOUND`가 난다). `client/scripts/`에 두면 파일 조상 경로에 `client/node_modules`가 있으므로 어디서 실행하든 항상 resolve된다.

**중요한 설계 결정 2**: 유닛테스트는 `client/`의 vitest가 아니라 **Node 내장 테스트 러너**(`node --test`, Node 18+ 표준 내장, 별도 의존성 불필요)로 돌린다 — `client/vitest.config.ts`는 `tests/unit/**`만 보므로 `client/scripts/`는 그 스캔 대상 밖이고, `perf-metrics.mjs`는 순수 함수라 어떤 테스트 러너로 돌려도 상관없어 가장 가벼운 선택(의존성 추가 없음)을 한다.

- [ ] **Step 1: 실패하는 테스트 작성**

`client/scripts/perf-metrics.test.mjs`:

```js
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
```

- [ ] **Step 2: 테스트 실패 확인**

Run (저장소 루트에서): `node --test client/scripts/perf-metrics.test.mjs`
Expected: FAIL — `Cannot find module './perf-metrics.mjs'`

- [ ] **Step 3: `client/scripts/perf-metrics.mjs` 구현**

```js
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
```

- [ ] **Step 4: 테스트 통과 확인**

Run (저장소 루트에서): `node --test client/scripts/perf-metrics.test.mjs`
Expected: PASS (3개 테스트)

- [ ] **Step 5: `client/scripts/perf-check.mjs` 작성(I/O 래퍼)**

```js
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
```

- [ ] **Step 6: 로컬 스모크 실행**

`client/package.json`에 이미 있는 `ws` devDependency를 그대로 재사용한다(별도 `package.json`을 새로 만들지 않음 — `client/scripts/`에 둔 덕분에 `node_modules` 해석이 자동으로 된다).

Run (로컬 스모크 — 배포 없이 로컬 Docker 컨테이너 대상으로 스크립트 자체가 동작하는지만 확인, 로봇 50대의 실제 p99는 로컬 머신 스펙이라 배포 환경과 다를 수 있음을 감안. **저장소 루트에서 실행**):

```bash
docker compose up -d
node client/scripts/perf-check.mjs http://localhost:8080
docker compose down
```

Expected: `robot_count=50`, `ticks_total=<0보다 큰 값>`, p99 값이 출력됨(에러 없이 스크립트 자체가 끝까지 실행되는 것을 확인 — 목표 수치 자체는 로컬 실측이라 참고용).

- [ ] **Step 7: 커밋**

```bash
git add client/scripts/perf-metrics.mjs client/scripts/perf-metrics.test.mjs client/scripts/perf-check.mjs
git commit -m "feat: 배포된 URL 대상 성능 실측 스크립트 추가(+ 파싱 로직 유닛테스트)"
```

**실제 배포된 URL 대상 실행은 배포 완료 후 진행** — 사용자가 배포 URL을 알려주면 그때 같이 실행해서 README "성능 목표" 절에 실측치를 반영한다(다음 Task 10의 README 갱신에서 자리 표시만 해두고, 실측치는 배포 후 별도로 채운다).

---

### Task 10: 문서 갱신 + 전체 회귀 검증

**Files:**
- Modify: `README.md`
- Modify: `docs/robot-arm-conveyor-game-design.md`
- Modify: `docs/KANBAN.md`

- [ ] **Step 1: 서버 전체 회귀 확인**

Run: `cargo test --manifest-path server/Cargo.toml`
Expected: PASS — 143개(Task 1-2가 추가한 4개 유닛 + 2개 통합으로 135→141, Task 7의 로봇 순찰 기능이 순증가 +2로 141→143).

Run: `cargo clippy --manifest-path server/Cargo.toml --all-targets -- -D warnings`
Expected: 경고 0개

- [ ] **Step 2: 클라이언트 전체 회귀 확인**

```bash
cd client
npm test               # 단위 — Task 3이 추가한 3개 포함(기존 46 + 3 = 49). Task 9의 perf-metrics 테스트는
                        # node --test로 별도 실행하므로 여기 포함 안 됨(아래 별도 확인).
npm run test:integration
npm run test:e2e
npm run typecheck
```
Expected: 전부 PASS/에러 없음.

Run (저장소 루트에서, Task 9의 Node 테스트도 회귀 확인): `node --test client/scripts/perf-metrics.test.mjs`
Expected: PASS (4개 — Task 9 코드 품질 리뷰에서 p99/p95 버킷이 실제로 다른 케이스 1개가 추가되어 3→4)

CI에도 이제 이 테스트가 연결돼 있다(Task 9 리뷰에서 `.github/workflows/rust-ci.yml`의 `client` job에 `npm run test:scripts` 스텝 추가) — `cd client && npm run test:scripts`로도 동일하게 확인 가능.

- [ ] **Step 3: README 갱신**

- **퀵스타트** 절 추가(README 최상단 근처): "이 URL을 열면 바로 체험 가능"(배포 URL은 배포 후 채움 자리 표시) + `docker compose up` 한 줄.
- **개발 환경** 절에 Docker/Fly.io 관련 새 파일(`Dockerfile`, `docker-compose.yml`, `fly.toml`, `.dockerignore`, `client/scripts/`) 존재 언급.
- **성능 목표** 관련 절에 클라이언트 프레임 시간(Plan 4 실측, 이미 있음) 옆에 "배포 환경 실측치: 배포 완료 후 기록 예정" 자리 표시 추가.
- **지금까지 만든 것**에 "Plan 5 — 데모/배포" 항목 추가: 단일 컨테이너 Docker 패키징(서버가 정적 파일까지 서빙), Fly.io 배포 설정, CI Docker 빌드 스모크, Playwright 자동 데모 녹화, 원격 URL 대상 성능 실측 스크립트.
- **다음 단계**에서 "Plan 5" 항목 제거(모든 계획된 Plan 완료).
- 테스트 개수 갱신(서버 143개, 클라이언트 vitest 단위 49개 + `node --test` 4개).

- [ ] **Step 4: 마스터 설계문서 각주 추가**

`docs/robot-arm-conveyor-game-design.md`의 "발표/데모 전략" 절(126-133번째 줄) 뒤에 Plan 4 각주(124번째 줄)와 같은 패턴으로 추가:

```markdown
> **각주(Plan 5, 2026-07-18 완료)**: 단일 컨테이너 Docker 패키징(서버가 `tower-http::ServeDir`로 클라이언트 정적 파일까지 서빙, `GAMEROBOTFACTORY_BIND_ADDR`로 바인드 주소 설정 가능), `fly.toml`(Fly.io 배포 설정), CI `docker build` 스모크 잡, Playwright 기반 데모 영상 자동 녹화(`client/scripts/record-demo.mjs`), 원격 URL 대상 성능 실측 스크립트(`client/scripts/perf-check.mjs`)가 이 시점에 추가됨. 실제 `flyctl deploy` 실행과 라이브 URL 발급은 사용자 계정으로 별도 진행.
```

- [ ] **Step 5: KANBAN.md 갱신**

`docs/KANBAN.md`의 Backlog에 있던 "Plan 5" 항목을 Done으로 옮기고, 이 계획 문서(`docs/superpowers/plans/2026-07-18-demo-deploy-plan.md`) 경로와 10개 태스크 전체 완료를 요약한다(기존 Plan들과 같은 서술 밀도로 — 서버가 `127.0.0.1:0` 하드코딩+정적 서빙 부재로 컨테이너화 자체가 불가능했다는 발견, 클라이언트 WS URL 자동 유도, Task 7 진행 중 발견한 로봇 순찰 기능 필요성 등 실제 코드 변경 내역 포함). "현재 건강도 스냅샷"도 갱신.

- [ ] **Step 6: 최종 커밋**

```bash
git add README.md docs/robot-arm-conveyor-game-design.md docs/KANBAN.md
git commit -m "docs: Plan 5(데모/배포) 완료 반영 — README/설계문서/KANBAN 갱신"
```

---

## 참고 — 배포 후 후속 작업 (이 계획의 범위 밖, 별도 진행)

Task 9에서 만든 `client/scripts/perf-check.mjs`는 사용자가 실제로 `flyctl deploy`를 실행해 라이브 URL을 얻은 뒤, 그 URL을 인자로 다시 한번(저장소 루트에서 `node client/scripts/perf-check.mjs <배포 URL>`) 실행해서 README의 "배포 환경 실측치" 자리 표시를 실제 수치로 채우는 후속 작업이 남아있다. 이건 이 계획(Task 1-10)이 완료된 후, 사용자가 배포를 마치면 별도로 진행한다.

## 참고 — 각 태스크 완료 시 KANBAN.md도 함께 갱신

프로젝트 관행상 태스크 하나가 끝날 때마다 `docs/KANBAN.md`의 In Progress 항목에 커밋 SHA를 남기는 `docs:` 커밋이 뒤따른다. 이 계획의 Task 1-9 각각도 완료 직후 그렇게 갱신하고, Task 10에서 전체를 한 번에 정리한다.
