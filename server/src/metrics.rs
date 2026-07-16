//! Prometheus metrics registry and `/metrics` encoder. Wired into `main.rs`'s
//! tick loop and route table, and into `ws.rs`'s connection lifecycle (via
//! `Extension<MetricsHandle>`).

use axum::extract::Extension;
use axum::response::IntoResponse;
use prometheus::{
    register_histogram_with_registry, register_int_counter_with_registry, register_int_gauge_with_registry, Encoder,
    Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder,
};
use std::sync::Arc;

pub struct Metrics {
    registry: Registry,
    pub ticks_total: IntCounter,
    pub connected_clients: IntGauge,
    pub robot_count: IntGauge,
    /// `safe_tick`(Task 2)이 패닉을 잡아낸 횟수. 이게 없으면 틱 루프가
    /// 매번 패닉해서 시뮬레이션이 조용히 멈춰도(서버 프로세스 자체는
    /// 살아있으니 `/health`는 여전히 "ok"를 반환) 밖에서 알아챌 방법이
    /// 없다 — Task 2 코드 리뷰에서 지적된 "조용한 멈춤" 관측 공백을
    /// 메우는 지표.
    pub tick_panics_total: IntCounter,
    /// 틱 하나를 처리하는 데 걸린 시간(초). 디자인 문서(설계 문서 ~101줄)의
    /// "틱 처리 시간 p99 < 10ms" 목표를 실제로 측정하기 위한 히스토그램 —
    /// `histogram_quantile(0.99, rate(gamerobotfactory_tick_duration_seconds_bucket[...]))`로
    /// Prometheus/Grafana에서 바로 p99를 뽑아낼 수 있도록 `_seconds` 접미사
    /// (Prometheus 베이스 단위 컨벤션)를 쓴다. 버킷 경계는 10ms 목표를 중심으로
    /// 그 아래/위 양쪽에 여유를 둬서 p99가 실제로 분해 가능하도록 잡았다.
    pub tick_duration_seconds: Histogram,
    /// 로봇이 Operational -> Failed로 전이할 때마다 증가 — 로봇 도메인
    /// 장애가 인프라 장애(tick_panics_total)와 같은 방식으로 관측
    /// 가능해지도록 하는 지표.
    pub robot_failures_total: IntCounter,
    /// 매 틱, 현재 Repairing 상태인 로봇 수로 갱신되는 게이지.
    pub robots_repairing: IntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let ticks_total = register_int_counter_with_registry!(
            "gamerobotfactory_ticks_total",
            "Total simulation ticks processed",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let connected_clients = register_int_gauge_with_registry!(
            "gamerobotfactory_connected_clients",
            "Currently connected WebSocket clients",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let robot_count = register_int_gauge_with_registry!(
            "gamerobotfactory_robot_count",
            "Current number of robots in the simulation",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let tick_panics_total = register_int_counter_with_registry!(
            "gamerobotfactory_tick_panics_total",
            "Total number of ticks where sim_core::sim::tick panicked and was skipped",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let tick_duration_seconds = register_histogram_with_registry!(
            HistogramOpts::new("gamerobotfactory_tick_duration_seconds", "Time spent processing a single simulation tick, in seconds")
                .buckets(vec![
                    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
                ]),
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let robot_failures_total = register_int_counter_with_registry!(
            "gamerobotfactory_robot_failures_total",
            "Total number of robot Operational -> Failed transitions",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");
        let robots_repairing = register_int_gauge_with_registry!(
            "gamerobotfactory_robots_repairing",
            "Current number of robots in the Repairing state",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 7 names are distinct and validly formed");

        Metrics {
            registry,
            ticks_total,
            connected_clients,
            robot_count,
            tick_panics_total,
            tick_duration_seconds,
            robot_failures_total,
            robots_repairing,
        }
    }

    pub fn encode(&self) -> (String, Vec<u8>) {
        let encoder = TextEncoder::new();
        let families = self.registry.gather();
        let mut buffer = Vec::new();
        encoder
            .encode(&families, &mut buffer)
            .expect("encoding a freshly-gathered metric family set does not fail");
        (encoder.format_type().to_string(), buffer)
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

pub type MetricsHandle = Arc<Metrics>;

pub async fn metrics_route(Extension(metrics): Extension<MetricsHandle>) -> impl IntoResponse {
    let (content_type, body) = metrics.encode();
    ([(axum::http::header::CONTENT_TYPE, content_type)], body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_encode_without_error_and_include_registered_names() {
        let metrics = Metrics::new();
        let (content_type, body) = metrics.encode();
        assert!(content_type.starts_with("text/plain"));
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_ticks_total"));
        assert!(text.contains("gamerobotfactory_connected_clients"));
        assert!(text.contains("gamerobotfactory_robot_count"));
        assert!(text.contains("gamerobotfactory_tick_panics_total"));
        assert!(text.contains("gamerobotfactory_tick_duration_seconds"));
        assert!(text.contains("gamerobotfactory_robot_failures_total"));
        assert!(text.contains("gamerobotfactory_robots_repairing"));
    }

    #[test]
    fn incrementing_a_counter_is_reflected_in_the_encoded_output() {
        let metrics = Metrics::new();
        metrics.ticks_total.inc();
        metrics.ticks_total.inc();
        let (_, body) = metrics.encode();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_ticks_total 2"));
    }

    #[test]
    fn observing_a_tick_duration_is_reflected_in_the_encoded_output() {
        let metrics = Metrics::new();
        let (_, fresh_body) = metrics.encode();
        let fresh_text = String::from_utf8(fresh_body).unwrap();
        assert!(fresh_text.contains("gamerobotfactory_tick_duration_seconds_count 0"));

        metrics.tick_duration_seconds.observe(0.002);
        metrics.tick_duration_seconds.observe(0.004);
        let (_, body) = metrics.encode();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_tick_duration_seconds_count 2"));
        assert!(text.contains("gamerobotfactory_tick_duration_seconds_sum 0.006"));
    }

    #[test]
    fn robot_failure_metrics_are_registered_and_reflect_updates() {
        let metrics = Metrics::new();
        metrics.robot_failures_total.inc();
        metrics.robots_repairing.set(2);
        let (_, body) = metrics.encode();
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("gamerobotfactory_robot_failures_total 1"));
        assert!(text.contains("gamerobotfactory_robots_repairing 2"));
    }
}
