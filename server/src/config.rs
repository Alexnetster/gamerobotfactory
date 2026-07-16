//! Runtime configuration exposed via `GET`/`POST /api/config`. Deliberately
//! not wired into `main.rs`'s router yet — that wiring (and the tick loop
//! actually reading `persist_every_n_ticks` to decide when to call into
//! `persistence.rs`) happens in a later task in this plan. This module is
//! complete and tested on its own first, matching the same
//! write-then-wire-later pattern used for `session.rs`/`persistence.rs`: the
//! dead-code lint is suppressed here rather than forcing premature wiring
//! just to satisfy clippy.
#![allow(dead_code)]

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// REST로 조회/변경 가능한 런타임 설정. WS(실시간 게임 상태)와 REST(설정)의
/// 책임을 분리한다는 설계문서의 결정을 실제로 구현하는 지점.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub persist_every_n_ticks: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig { persist_every_n_ticks: 20 }
    }
}

pub type ConfigHandle = Arc<Mutex<AppConfig>>;

pub async fn get_config(Extension(config): Extension<ConfigHandle>) -> impl IntoResponse {
    let current = *config.lock().await;
    Json(current)
}

pub async fn post_config(
    Extension(config): Extension<ConfigHandle>,
    Json(update): Json<AppConfig>,
) -> impl IntoResponse {
    if update.persist_every_n_ticks == 0 {
        return (StatusCode::BAD_REQUEST, "persist_every_n_ticks must be at least 1").into_response();
    }
    let mut current = config.lock().await;
    *current = update;
    Json(update).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_a_sane_persist_interval() {
        assert_eq!(AppConfig::default().persist_every_n_ticks, 20);
    }
}
