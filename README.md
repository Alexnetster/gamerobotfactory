# 로봇팔 컨베이어 게임

4족 보행 로봇이 공장 컨베이어 라인을 돌며 작업을 돕는 팩토리 시뮬레이션. **백엔드/서버 설계 역량을 보여주는 포트폴리오 프로젝트**로, 게임성은 2차 목표다 — 결정적 시뮬레이션 코어, WebSocket 델타 동기화 프로토콜, 동시성 안전한 틱 루프가 이 프로젝트의 실제 핵심이다.

전체 설계 배경과 트레이드오프는 [`docs/robot-arm-conveyor-game-design.md`](docs/robot-arm-conveyor-game-design.md)에, 지금까지 뭘 만들었고 뭐가 남았는지는 [`docs/KANBAN.md`](docs/KANBAN.md)에 있다.

## 아키텍처

```
sim_core (라이브러리, 네트워크 의존성 없음)
  그리드 · A* 경로탐색 · 결정적 병렬 틱(더블 버퍼링+ID 타이브레이크) · 패닉 격리
  프로시저럴 보행 · 2-본 팔 IK · 자세-IK 연결 · 결정적 생산량 집계

server (바이너리, sim_core에 의존)
  GameState(컨베이어/로봇수/선택/팔동작) · JSON 와이어 프로토콜(버전 필드,
  변경분만 담는 델타) · 20Hz 틱 루프 + 브로드캐스트 · axum WebSocket 핸들러
```

두 부분 다 같은 Cargo 패키지(`gamerobotfactory-server`) 안에서 라이브러리 타깃(`sim_core`)과 바이너리 타깃(`server`)으로 나뉘어 있다.

## 빠른 시작

```bash
# 서버 실행 (포트는 OS가 임의로 골라 표준출력에 LISTENING_PORT=<번호>로 알려줌)
cargo run --manifest-path server/Cargo.toml

# 테스트 전체 실행
cargo test --manifest-path server/Cargo.toml

# 린트
cargo clippy --manifest-path server/Cargo.toml --all-targets
```

서버가 뜨면 WebSocket으로 `ws://127.0.0.1:<포트>/ws`에 접속해 커맨드를 보낼 수 있다:

```json
{"type": "SetRobotCount", "count": 3}
{"type": "ToggleConveyor"}
{"type": "SelectRobot", "robot_id": 0}
{"type": "TriggerArmAction", "robot_id": 0, "task": "Picking"}
```

접속하면 즉시 전체 스냅샷을 받고, 이후 20Hz로 변경분만 담은 델타 메시지를 받는다.

## 지금까지 만든 것

- **Plan 1 — 결정적 시뮬레이션 코어**: 그리드/A* 경로탐색, 로봇 ID 기반 결정적 타이브레이크가 있는 병렬 틱, 로봇 하나가 패닉해도 나머지는 정상 갱신되는 격리, 트롯 보행 애니메이션, 2-본 IK(몸체 자세와 연결됨), 부동소수점 합산 순서에 영향받지 않는 생산량 집계.
- **Plan 2 — WS 프로토콜 & 네트워킹**: 실제로 뜨는 axum 서버, 커맨드 검증(존재하지 않는 로봇 거부, `SetRobotCount` 상한 클램프), 클라이언트별로 바뀐 로봇만 보내는 델타 동기화, 재접속 유예시간 로직(아직 실배선은 안 됨).

**현재: 67개 테스트 통과, clippy 경고 0개.**

## 다음 단계

- **Plan 3**: SQLite 영속화, REST API, 관측가능성(`/metrics`, tracing) — 여기서 재접속 실배선과 틱 루프 패닉 주입 테스트도 함께 다룰 예정.
- **Plan 4**: 클라이언트 렌더링 (Vite+TS+Canvas, 아이소메트릭 투영) — 아직 `client/` 디렉토리 자체가 없다.
- **Plan 5**: 데모/배포 (Docker, 라이브 URL, 성능 목표 실측).

상세 계획은 [`docs/superpowers/plans/`](docs/superpowers/plans/)에 있다.
