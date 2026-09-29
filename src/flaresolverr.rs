//! Minimal FlareSolverr client (`FLARESOLVERR_URL`, e.g. http://127.0.0.1:8191).
//!
//! FlareSolverr drives a headless Chromium that passes Cloudflare's browser
//! check. anorak uses the page HTML FlareSolverr returns directly; it never
//! replays the `cf_clearance` cookie itself (Cloudflare ties the cookie to
//! the browser's TLS/HTTP fingerprint, so a replay from reqwest gets a 403).
//!
//! One FlareSolverr session is kept so only the first request pays for the
//! challenge (~12 s); later pages load in 1-2 s. Requests through the session
//! are serialized (it is a single browser tab) and the session is closed
//! after `IDLE` without use, which frees its Chromium.

use crate::config::CONFIG;

use anyhow::{anyhow, bail, Result};
use log::{info, warn};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const SESSION: &str = "anorak";
/// FlareSolverr's own limit for one page (challenge included).
const MAX_TIMEOUT: Duration = Duration::from_secs(50);
/// Close the browser session after this long without a request.
const IDLE: Duration = Duration::from_secs(30 * 60);

/// A page as the browser saw it.
pub struct Page {
    pub status: u16,
    pub html: String,
    pub title: String,
}

/// Why a page couldn't be had. The label is shown to the user next to the
/// source name, e.g. "1337x (blocked by Cloudflare)".
#[derive(Debug)]
pub struct Failure(pub String);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Failure {}

pub fn failure(label: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Failure(label.into()))
}

pub fn enabled() -> bool {
    CONFIG.flaresolverr_url.is_some()
}

struct State {
    session: bool,
    last_used: Instant,
    reaper: bool,
}

static STATE: Lazy<Mutex<State>> =
    Lazy::new(|| Mutex::new(State { session: false, last_used: Instant::now(), reaper: false }));

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(MAX_TIMEOUT + Duration::from_secs(20))
        .build()
        .expect("reqwest client")
});

async fn call(body: Value) -> Result<Value> {
    let base = CONFIG.flaresolverr_url.as_deref().ok_or_else(|| anyhow!("FLARESOLVERR_URL is not set"))?;
    let url = format!("{}/v1", base.trim_end_matches('/'));
    let response = CLIENT
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|err| {
        if err.is_connect() {
            failure("FlareSolverr not running")
        } else if err.is_timeout() {
            failure("timed out passing Cloudflare")
        } else {
            anyhow!("FlareSolverr: {err}")
        }
    })?;
    let text = response.text().await.map_err(|err| anyhow!("FlareSolverr answer: {err}"))?;
    let value: Value = serde_json::from_str(&text).map_err(|err| anyhow!("FlareSolverr answer: {err}"))?;
    Ok(value)
}

fn message(v: &Value) -> String {
    v.get("message").and_then(Value::as_str).unwrap_or("").to_string()
}

/// GET `url` in the shared browser session.
pub async fn get(url: &str) -> Result<Page> {
    let mut state = STATE.lock().await;
    if !state.reaper {
        state.reaper = true;
        tokio::spawn(reap_idle());
    }
    for attempt in 0..2 {
        if !state.session {
            let v = call(json!({"cmd": "sessions.create", "session": SESSION})).await?;
            let msg = message(&v);
            if v.get("status").and_then(Value::as_str) != Some("ok") && !msg.to_ascii_lowercase().contains("already exists") {
                bail!("FlareSolverr couldn't open a browser session: {msg}");
            }
            info!("FlareSolverr session opened");
            state.session = true;
        }
        let started = Instant::now();
        let v = call(json!({
            "cmd": "request.get",
            "url": url,
            "session": SESSION,
            "maxTimeout": MAX_TIMEOUT.as_millis() as u64,
            "disableMedia": true,
        }))
        .await?;
        state.last_used = Instant::now();
        let msg = message(&v);
        if v.get("status").and_then(Value::as_str) != Some("ok") {
            let lower = msg.to_ascii_lowercase();
            if attempt == 0 && lower.contains("session") {
                // FlareSolverr restarted and forgot the session: open a new one.
                warn!("FlareSolverr session lost ({msg}); reopening");
                state.session = false;
                continue;
            }
            if lower.contains("challenge") || lower.contains("captcha") || lower.contains("timeout") {
                return Err(failure("blocked by Cloudflare").context(msg));
            }
            bail!("FlareSolverr: {msg}");
        }
        let solution = v.get("solution").cloned().unwrap_or(Value::Null);
        let status = solution.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
        let html = solution.get("response").and_then(Value::as_str).unwrap_or("").to_string();
        let title = page_title(&html);
        info!("FlareSolverr {url}: {status} in {} ms ({msg})", started.elapsed().as_millis());
        return Ok(Page { status, html, title });
    }
    bail!("FlareSolverr session kept failing")
}

async fn reap_idle() {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        let mut state = STATE.lock().await;
        if state.session && state.last_used.elapsed() > IDLE {
            let _ = call(json!({"cmd": "sessions.destroy", "session": SESSION})).await;
            state.session = false;
            info!("FlareSolverr session closed after {} min idle", IDLE.as_secs() / 60);
        }
    }
}

pub fn page_title(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title>").map(|i| i + 7) else { return String::new() };
    let end = lower[start..].find("</title>").map(|i| start + i).unwrap_or(start);
    crate::x1337::decode_entities(html[start..end].trim())
}

/// Cloudflare's interstitial ("Just a moment...") or block page.
pub fn is_cloudflare_page(status: u16, title: &str, html: &str) -> bool {
    let t = title.to_ascii_lowercase();
    if t.contains("just a moment") || t.contains("attention required") || t.contains("verify you are human") {
        return true;
    }
    (status == 403 || status == 503 || status == 429)
        && (html.contains("challenge-platform") || html.contains("cf-chl") || html.contains("cf_chl_opt"))
}
