//! Name of the torrent client anorak sends to, for button labels.
//!
//! `TORRENT_CLIENT_NAME` overrides it; otherwise the client is asked
//! (rqbit's `GET /` answers `{"server": "rqbit", "version": ...}`). The
//! answer is cached; if the client can't be reached the generic
//! "torrent client" is used and the lookup is retried a minute later.

use crate::config::CONFIG;

use log::warn;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const FALLBACK: &str = "torrent client";

#[derive(Clone, Debug, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: Option<String>,
    /// "config", "client" (reported by the client) or "fallback".
    pub source: &'static str,
}

static CACHE: Lazy<Mutex<Option<(Instant, ClientInfo)>>> = Lazy::new(|| Mutex::new(None));
const FOUND_TTL: Duration = Duration::from_secs(600);
const MISSING_TTL: Duration = Duration::from_secs(60);

pub async fn info() -> ClientInfo {
    if let Some(name) = CONFIG.torrent_client_name.as_deref() {
        return ClientInfo { name: name.to_string(), version: None, source: "config" };
    }
    let mut cache = CACHE.lock().await;
    if let Some((at, info)) = cache.as_ref() {
        let ttl = if info.source == "client" { FOUND_TTL } else { MISSING_TTL };
        if at.elapsed() < ttl {
            return info.clone();
        }
    }
    let info = probe().await.unwrap_or_else(|| ClientInfo {
        name: FALLBACK.to_string(),
        version: None,
        source: "fallback",
    });
    *cache = Some((Instant::now(), info.clone()));
    info
}

pub async fn name() -> String {
    info().await.name
}

async fn probe() -> Option<ClientInfo> {
    let response = crate::rqbit::get("/", Duration::from_secs(3))
        .await
        .map_err(|err| warn!("torrent client name lookup failed: {err}"))
        .ok()?;
    let text = response.text().await.ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = clean(value.get("server")?.as_str()?)?;
    let version = value.get("version").and_then(|v| v.as_str()).and_then(clean);
    Some(ClientInfo { name, version, source: "client" })
}

/// Short, printable, single-line.
fn clean(s: &str) -> Option<String> {
    let s: String = s.chars().filter(|c| !c.is_control()).take(40).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}
