//! "Send to rqbit": hand a magnet / .torrent link to the torrent client.
//!
//! rqbit's `POST /torrents` normally answers only once the torrent is fully
//! added. For a magnet that means after its metadata has been fetched from
//! DHT/peers: seconds for a well-seeded torrent, minutes (up to rqbit's
//! 10-minute request timeout) for a poorly seeded one. The button used to
//! spin for all of that time.
//!
//! Now:
//! * magnets are added with `defer_metadata=true`, so rqbit queues them at
//!   once (listed as "Resolving metadata") and answers `resolving: true`;
//! * the request to rqbit runs in a background task that is never dropped
//!   (dropping an add request cancels it in rqbit). The HTTP handler waits
//!   at most [`HANDOFF_WAIT`]; if rqbit hasn't answered by then (an older
//!   rqbit without `defer_metadata`, a slow .torrent download, a disk-slot
//!   wait) it answers `pending` with a job id, and the page polls
//!   `GET /api/send/{job}` until rqbit answers;
//! * a second send of the same torrent (same info hash, or same link) while
//!   one is still in flight joins that send instead of adding it again.

use crate::models::{self, SendToTransmission};
use crate::torrent_files;
use crate::{app_error::AppError, config::CONFIG};

use anyhow::{anyhow, Result};
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{debug_handler, Form, Json};
use log::{info, warn};
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex};
use urlencoding::decode;

/// Longest the send request itself waits for rqbit before answering `pending`.
const HANDOFF_WAIT: Duration = Duration::from_secs(8);
/// Upper bound for the background request to rqbit (rqbit itself gives up
/// on an add after 10 minutes by default).
const RQBIT_ADD_TIMEOUT: Duration = Duration::from_secs(11 * 60);
/// How long finished sends stay available to `GET /api/send/{job}`.
const KEEP_FINISHED: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendState {
    /// Handed to rqbit, which hasn't answered yet.
    Pending,
    /// rqbit queued the magnet and is fetching its metadata in the background.
    Queued,
    /// rqbit added the torrent.
    Sent,
    /// rqbit already had this torrent.
    Already,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct SendStatus {
    pub job: String,
    pub state: SendState,
    pub message: String,
    /// rqbit's torrent id once known.
    pub torrent_id: Option<u64>,
    pub elapsed_ms: u64,
    /// rqbit's add stage while pending (e.g. "resolving_metadata").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

#[derive(Clone, Debug)]
struct Outcome {
    state: SendState,
    message: String,
    torrent_id: Option<u64>,
}

struct Job {
    key: String,
    rqbit_job: String,
    started: Instant,
    finished: Option<Instant>,
    rx: watch::Receiver<Option<Outcome>>,
}

static JOBS: Lazy<Mutex<HashMap<String, Job>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn new_job_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("anorak-{:x}-{n}", nanos as u64)
}

#[debug_handler]
pub async fn endpoint(Form(payload): Form<SendToTransmission>) -> Result<Response, AppError> {
    // The form body is already URL-decoded. Decoding a magnet again would
    // unescape its dn/tr values (%26 -> '&' splits the magnet), so only
    // fall back to decoding for a value that arrives still escaped
    // ("magnet%3A%3Fxt=...").
    let raw = payload.magnet.trim();
    let lower = raw.to_ascii_lowercase();
    let decoded = if lower.starts_with("magnet:") || lower.starts_with("http://") || lower.starts_with("https://") {
        raw.to_string()
    } else {
        decode(raw)?.into_owned()
    };
    if decoded.trim().is_empty() {
        return Err(anyhow!("no magnet or .torrent link given").into());
    }
    let category = payload
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok());
    let indexer = payload
        .indexer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (job, rx) = start_or_join(decoded, category, indexer).await;
    let status = wait_status(&job, rx, HANDOFF_WAIT).await;
    match status.state {
        SendState::Error => Err(anyhow!("{}", status.message).into()),
        SendState::Pending => Ok((StatusCode::ACCEPTED, Json(status)).into_response()),
        _ => Ok(Json(status).into_response()),
    }
}

/// `GET /api/send/{job}`: where a send stands (poll while `pending`).
#[debug_handler]
pub async fn status(Path(job): Path<String>) -> Response {
    let found = {
        let jobs = JOBS.lock().await;
        jobs.get(&job).map(|j| (j.rx.clone(), j.rqbit_job.clone()))
    };
    let Some((rx, rqbit_job)) = found else {
        return (StatusCode::NOT_FOUND, "no such send (anorak may have restarted)").into_response();
    };
    let mut status = wait_status(&job, rx, Duration::ZERO).await;
    if status.state == SendState::Pending {
        status.stage = rqbit_stage(&rqbit_job).await;
    }
    Json(status).into_response()
}

/// Joins an in-flight send of the same torrent, or starts a new one.
async fn start_or_join(
    link: String,
    category: Option<u32>,
    indexer: Option<String>,
) -> (String, watch::Receiver<Option<Outcome>>) {
    let key = models::info_hash(&link).unwrap_or_else(|| link.clone());
    let mut jobs = JOBS.lock().await;
    jobs.retain(|_, j| j.finished.map_or(true, |at| at.elapsed() < KEEP_FINISHED));
    // A finished job whose watch value is set but `finished` not yet
    // recorded counts as done too.
    if let Some((id, j)) = jobs
        .iter()
        .find(|(_, j)| j.key == key && j.finished.is_none() && j.rx.borrow().is_none())
    {
        info!("send of {key} already in flight as {id}; joining it");
        return (id.clone(), j.rx.clone());
    }
    let id = new_job_id();
    let (tx, rx) = watch::channel(None);
    jobs.insert(
        id.clone(),
        Job { key: key.clone(), rqbit_job: id.clone(), started: Instant::now(), finished: None, rx: rx.clone() },
    );
    drop(jobs);

    let job_id = id.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let outcome = match run_send(link, category, indexer, &job_id).await {
            Ok(outcome) => outcome,
            Err(err) => Outcome { state: SendState::Error, message: err.to_string(), torrent_id: None },
        };
        let ms = started.elapsed().as_millis();
        match outcome.state {
            SendState::Error => warn!("send {job_id} ({key}) failed after {ms} ms: {}", outcome.message),
            state => info!("send {job_id} ({key}): {state:?} after {ms} ms"),
        }
        if let Some(j) = JOBS.lock().await.get_mut(&job_id) {
            j.finished = Some(Instant::now());
        }
        let _ = tx.send(Some(outcome));
    });
    (id, rx)
}

/// Waits up to `wait` for the send to finish and reports where it stands.
async fn wait_status(job: &str, mut rx: watch::Receiver<Option<Outcome>>, wait: Duration) -> SendStatus {
    if !wait.is_zero() && rx.borrow().is_none() {
        let _ = tokio::time::timeout(wait, async {
            while rx.borrow_and_update().is_none() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
    }
    let times = JOBS.lock().await.get(job).map(|j| (j.started, j.finished));
    let elapsed_ms = match times {
        Some((started, Some(finished))) => finished.duration_since(started).as_millis() as u64,
        Some((started, None)) => started.elapsed().as_millis() as u64,
        None => 0,
    };
    let current = rx.borrow().clone();
    match current {
        Some(o) => SendStatus {
            job: job.to_string(),
            state: o.state,
            message: o.message,
            torrent_id: o.torrent_id,
            elapsed_ms,
            stage: None,
        },
        None => SendStatus {
            job: job.to_string(),
            state: SendState::Pending,
            message: "rqbit hasn't answered yet; anorak keeps the request going in the background".to_string(),
            torrent_id: None,
            elapsed_ms,
            stage: None,
        },
    }
}

async fn run_send(link: String, category: Option<u32>, indexer: Option<String>, job_id: &str) -> Result<Outcome> {
    let link = match indexer {
        Some(indexer) if torrent_files::is_page_link(&link) => {
            // A details page (0Magnet) is not something the client can add;
            // send the magnet printed on it. A .torrent is left as is.
            match torrent_files::magnet_for_link(&link, &indexer).await {
                Ok(Some(magnet)) => {
                    info!("sending the magnet from {link}");
                    magnet
                }
                Ok(None) => link,
                Err(err) => {
                    warn!("no magnet from {link}: {err}");
                    link
                }
            }
        }
        _ => link,
    };
    send_rqbit(link, category, job_id).await
}

async fn send_rqbit(link: String, torznab_category: Option<u32>, job_id: &str) -> Result<Outcome> {
    let base = format!("{}/torrents", CONFIG.rqbit_url.trim_end_matches('/'));
    let mut params: Vec<(&str, String)> = Vec::new();
    if let Some(id) = torznab_category {
        params.push(("torznab_category", id.to_string()));
    }
    let is_magnet = link.trim_start().to_ascii_lowercase().starts_with("magnet:");
    if is_magnet {
        // Return as soon as the magnet is queued; rqbit fetches the metadata
        // in the background. Older rqbit builds ignore unknown parameters.
        params.push(("defer_metadata", "true".to_string()));
    }
    // Lets `GET /api/send/{job}` show rqbit's stage (/add_jobs/{id}).
    params.push(("add_job_id", job_id.to_string()));
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect();
    let sep = if base.contains('?') { '&' } else { '?' };
    let url = format!("{base}{sep}{}", query.join("&"));

    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(RQBIT_ADD_TIMEOUT)
        .build()?
        .post(url)
        .body(link)
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                anyhow!("rqbit didn't answer within {} minutes", RQBIT_ADD_TIMEOUT.as_secs() / 60)
            } else if err.is_connect() {
                anyhow!("can't reach rqbit at {}", CONFIG.rqbit_url)
            } else {
                anyhow!("request to rqbit failed: {err}")
            }
        })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let text = rqbit_error_text(&text);
        return Err(anyhow!("rqbit returned {status}: {text}"));
    }
    Ok(outcome_from_rqbit(&text))
}

/// rqbit answers `{"id":…, "resolving":true, "already_managed":…}`.
fn outcome_from_rqbit(text: &str) -> Outcome {
    let value: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let flag = |k: &str| value.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let torrent_id = value.get("id").and_then(|v| v.as_u64());
    let (state, message) = if flag("already_managed") {
        (SendState::Already, "rqbit already has this torrent")
    } else if flag("resolving") {
        (SendState::Queued, "queued in rqbit; it is fetching the torrent's metadata from peers")
    } else {
        (SendState::Sent, "added to rqbit")
    };
    Outcome { state, message: message.to_string(), torrent_id }
}

/// rqbit errors look like `{"human_readable": "...", ...}`; fall back to the body.
fn rqbit_error_text(text: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let msg = ["human_readable", "error", "message"]
        .iter()
        .find_map(|k| value.get(*k).and_then(|v| v.as_str()).map(str::to_string))
        .unwrap_or_else(|| text.trim().to_string());
    msg.chars().take(300).collect()
}

async fn rqbit_stage(rqbit_job: &str) -> Option<String> {
    let url = format!("{}/add_jobs/{rqbit_job}", CONFIG.rqbit_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&response.text().await.ok()?).ok()?;
    let stage = value.get("stage")?.as_str()?.to_string();
    let secs = value.get("stage_secs").and_then(|v| v.as_f64());
    Some(match secs {
        Some(s) => format!("{stage} ({s:.0} s)"),
        None => stage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_rqbit_answers() {
        assert_eq!(outcome_from_rqbit(r#"{"id":7,"resolving":true}"#).state, SendState::Queued);
        assert_eq!(outcome_from_rqbit(r#"{"id":7,"resolving":true,"already_managed":true}"#).state, SendState::Already);
        assert_eq!(outcome_from_rqbit(r#"{"id":7,"already_managed":true}"#).state, SendState::Already);
        let sent = outcome_from_rqbit(r#"{"id":7,"details":{}}"#);
        assert_eq!(sent.state, SendState::Sent);
        assert_eq!(sent.torrent_id, Some(7));
        assert_eq!(outcome_from_rqbit("not json").state, SendState::Sent);
    }

    #[test]
    fn error_text() {
        assert_eq!(rqbit_error_text(r#"{"human_readable":"bad magnet","id":null}"#), "bad magnet");
        assert_eq!(rqbit_error_text("plain"), "plain");
    }
}
