//! Blocking HTTP calls to the Anorak server. Always run these on GPUI's
//! background executor, never on the UI thread.

use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiResponse {
    #[allow(dead_code)]
    pub search_term: String,
    #[allow(dead_code)]
    pub count: usize,
    pub items: Vec<ApiItem>,
}

/// One search result, as returned by the server's `GET /api/query`.
#[derive(Debug, Clone, Deserialize)]
pub struct ApiItem {
    pub title: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub guid: String,
    #[serde(default)]
    pub magnet: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub seeders: u32,
    #[serde(default)]
    pub peers: u32,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub size_format: String,
    #[serde(default)]
    pub date: Option<i64>,
    #[serde(default)]
    pub date_format: String,
    #[serde(default)]
    pub already_added: bool,
}

/// Timing breakdown for one search, used by `--bench`.
#[derive(Debug, Clone, Default)]
pub struct QueryTiming {
    pub http: Duration,
    pub parse: Duration,
    pub bytes: usize,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .http_status_as_error(false)
        .build()
        .into()
}

pub fn query(server: &str, term: &str) -> Result<(Vec<ApiItem>, QueryTiming), String> {
    let url = format!("{}/api/query", server.trim_end_matches('/'));
    let t0 = Instant::now();
    let mut resp = agent()
        .get(&url)
        .query("search_term", term)
        .call()
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("reading response failed: {e}"))?;
    let http = t0.elapsed();
    if !status.is_success() {
        return Err(format!("server returned {status}: {}", body.trim()));
    }
    let t1 = Instant::now();
    let parsed: ApiResponse =
        serde_json::from_str(&body).map_err(|e| format!("bad JSON from server: {e}"))?;
    let timing = QueryTiming {
        http,
        parse: t1.elapsed(),
        bytes: body.len(),
    };
    Ok((parsed.items, timing))
}

/// Same request the web UI's Grab button makes: form-encoded `magnet` and
/// optional `category` to `POST /send-to-rqbit/`.
pub fn grab(server: &str, magnet: &str, category: &str) -> Result<(), String> {
    let url = format!("{}/send-to-rqbit/", server.trim_end_matches('/'));
    let mut form = vec![("magnet", magnet)];
    if !category.is_empty() {
        form.push(("category", category));
    }
    let mut resp = agent()
        .post(&url)
        .send_form(form)
        .map_err(|e| format!("request to {url} failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        Err(format!("server returned {status}: {}", body.trim()))
    }
}
