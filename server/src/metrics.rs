//! Prometheus metrics registry and `/metrics` encoder. Deliberately not wired
//! into `main.rs`'s tick loop yet — Task 7 in this plan will call into
//! `Metrics` (via `Extension<MetricsHandle>`) from the tick loop and route
//! table. This module is complete and tested on its own first, matching the
//! same write-then-wire-later pattern used for `session.rs`/`persistence.rs`/
//! `config.rs` (see those modules' history for the precedent): the dead-code
//! lint is suppressed here rather than forcing premature wiring just to
//! satisfy clippy.
#![allow(dead_code)]

use axum::extract::Extension;
use axum::response::IntoResponse;
use prometheus::{
    register_int_counter_with_registry, register_int_gauge_with_registry, Encoder, IntCounter, IntGauge, Registry,
    TextEncoder,
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
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();
        let ticks_total = register_int_counter_with_registry!(
            "gamerobotfactory_ticks_total",
            "Total simulation ticks processed",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 4 names are distinct and validly formed");
        let connected_clients = register_int_gauge_with_registry!(
            "gamerobotfactory_connected_clients",
            "Currently connected WebSocket clients",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 4 names are distinct and validly formed");
        let robot_count = register_int_gauge_with_registry!(
            "gamerobotfactory_robot_count",
            "Current number of robots in the simulation",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 4 names are distinct and validly formed");
        let tick_panics_total = register_int_counter_with_registry!(
            "gamerobotfactory_tick_panics_total",
            "Total number of ticks where sim_core::sim::tick panicked and was skipped",
            registry
        )
        .expect("registration only fails on a duplicate/invalid metric name; these 4 names are distinct and validly formed");

        Metrics { registry, ticks_total, connected_clients, robot_count, tick_panics_total }
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
}
