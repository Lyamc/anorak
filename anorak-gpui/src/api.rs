//! HTTP calls to the Anorak server.
//!
//! Native: blocking `ureq` calls run on GPUI's background executor.
//! Web: GPUI's `FetchHttpClient` (browser `fetch`, same origin as the page),
//! awaited on the UI thread; there are no threads in the single-threaded build.
//! Both targets expose the same `query_task` / `grab_task`.

use std::time::Duration;

use gpui::{App, Task};
use serde::Deserialize;
use web_time::Instant;

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

#[cfg(not(target_family = "wasm"))]
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .http_status_as_error(false)
        .build()
        .into()
}

#[cfg(not(target_family = "wasm"))]
fn query(server: &str, term: &str) -> Result<(Vec<ApiItem>, QueryTiming), String> {
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
    parse(&body, http)
}

fn parse(body: &str, http: Duration) -> Result<(Vec<ApiItem>, QueryTiming), String> {
    let t1 = Instant::now();
    let parsed: ApiResponse =
        serde_json::from_str(body).map_err(|e| format!("bad JSON from server: {e}"))?;
    let timing = QueryTiming {
        http,
        parse: t1.elapsed(),
        bytes: body.len(),
    };
    Ok((parsed.items, timing))
}

pub type QueryResult = Result<(Vec<ApiItem>, QueryTiming), String>;

#[cfg(not(target_family = "wasm"))]
pub fn query_task(server: String, term: String, cx: &App) -> Task<QueryResult> {
    cx.background_executor()
        .spawn(async move { query(&server, &term) })
}

#[cfg(not(target_family = "wasm"))]
pub fn grab_task(server: String, magnet: String, category: String, cx: &App) -> Task<Result<(), String>> {
    cx.background_executor()
        .spawn(async move { grab(&server, &magnet, &category) })
}

#[cfg(target_family = "wasm")]
mod fetch {
    use futures::AsyncReadExt as _;
    use http_client::{AsyncBody, HttpClient, http};

    pub fn encode(s: &str) -> String {
        String::from(js_sys::encode_uri_component(s))
    }

    /// Returns (status, body).
    pub async fn send(
        client: &dyn HttpClient,
        req: http::Request<AsyncBody>,
    ) -> Result<(http::StatusCode, String), String> {
        let url = req.uri().to_string();
        let mut resp = client
            .send(req)
            .await
            .map_err(|e| format!("request to {url} failed: {e:#}"))?;
        let mut body = Vec::new();
        resp.body_mut()
            .read_to_end(&mut body)
            .await
            .map_err(|e| format!("reading response failed: {e}"))?;
        Ok((resp.status(), String::from_utf8_lossy(&body).into_owned()))
    }
}

#[cfg(target_family = "wasm")]
pub fn query_task(server: String, term: String, cx: &App) -> Task<QueryResult> {
    use http_client::{AsyncBody, http};
    let client = cx.http_client();
    cx.foreground_executor().spawn(async move {
        let url = format!(
            "{}/api/query?search_term={}",
            server.trim_end_matches('/'),
            fetch::encode(&term)
        );
        let req = http::Request::get(url.as_str())
            .body(AsyncBody::empty())
            .map_err(|e| format!("bad request {url}: {e}"))?;
        let t0 = Instant::now();
        let (status, body) = fetch::send(client.as_ref(), req).await?;
        let http = t0.elapsed();
        if !status.is_success() {
            return Err(format!("server returned {status}: {}", body.trim()));
        }
        parse(&body, http)
    })
}

#[cfg(target_family = "wasm")]
pub fn grab_task(server: String, magnet: String, category: String, cx: &App) -> Task<Result<(), String>> {
    use http_client::{AsyncBody, http};
    let client = cx.http_client();
    cx.foreground_executor().spawn(async move {
        let url = format!("{}/send-to-rqbit/", server.trim_end_matches('/'));
        let mut form = format!("magnet={}", fetch::encode(&magnet));
        if !category.is_empty() {
            form.push_str(&format!("&category={}", fetch::encode(&category)));
        }
        let req = http::Request::post(url.as_str())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(AsyncBody::from(form))
            .map_err(|e| format!("bad request {url}: {e}"))?;
        let (status, body) = fetch::send(client.as_ref(), req).await?;
        if status.is_success() {
            Ok(())
        } else {
            Err(format!("server returned {status}: {}", body.trim()))
        }
    })
}

/// Same request the web UI's Grab button makes: form-encoded `magnet` and
/// optional `category` to `POST /send-to-rqbit/`.
#[cfg(not(target_family = "wasm"))]
fn grab(server: &str, magnet: &str, category: &str) -> Result<(), String> {
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
