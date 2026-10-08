//! "Send to rqbit": hand a magnet / .torrent link to the torrent client.
//!
//! Waiting is never an error. Only a link that is actually bad (a malformed
//! magnet, rqbit rejecting it as invalid, a .torrent URL that isn't a
//! torrent) ends in the red "invalid" state; rqbit being down or failing for
//! its own reasons gets the amber "unreachable" / "failed" states.
//!
//! * The link is checked here first (magnet: scheme, xt=urn:btih: with a
//!   40-hex or 32-base32 info hash, or an http(s) URL).
//! * Magnets are added with `defer_metadata=true`: rqbit queues them at once
//!   (listed as "Resolving metadata") and answers `resolving: true`.
//!   `magnet_timeout_secs` is set to a year so rqbit keeps looking for peers
//!   instead of marking the placeholder "Metadata failed" after its default
//!   15 minutes.
//! * The request to rqbit runs in a background task that is never dropped
//!   (dropping an add request cancels it in rqbit). The HTTP handler waits
//!   at most [`HANDOFF_WAIT`]; if rqbit hasn't answered by then it answers
//!   `pending` with a job id and the page polls `GET /api/send/{job}`.
//! * If rqbit's add times out (an rqbit that ignores `defer_metadata` and
//!   waits for metadata, a slow .torrent download), the add is sent again
//!   (with `defer_metadata` for magnets) and the send stays `pending`.
//! * A send of an info hash (or link) that is already in flight joins it.

use crate::models::SendToTransmission;
use crate::torrent_files;
use crate::config::CONFIG;

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
/// rqbit's own limit for one add request (`timeout_ms`, capped by rqbit at 1 h).
const RQBIT_ADD_TIMEOUT_MS: u64 = 3_600_000;
/// anorak's limit for one add request; a bit longer than rqbit's.
const RQBIT_REQUEST_TIMEOUT: Duration = Duration::from_secs(3_600 + 60);
/// How long rqbit may look for a deferred magnet's metadata (a year: in
/// practice until the torrent is removed).
const MAGNET_RESOLVE_SECS: u64 = 365 * 24 * 3_600;
/// How long finished sends stay available to `GET /api/send/{job}`.
const KEEP_FINISHED: Duration = Duration::from_secs(6 * 3_600);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendState {
    /// Handed to rqbit, which hasn't confirmed yet. Not an error.
    Pending,
    /// rqbit queued the magnet and is fetching its metadata in the background.
    Queued,
    /// rqbit added the torrent.
    Sent,
    /// rqbit already had this torrent.
    Already,
    /// The link is bad (malformed magnet, rejected as invalid, not a torrent).
    Invalid,
    /// rqbit couldn't be reached.
    Unreachable,
    /// rqbit refused for a reason that isn't the link's fault.
    Failed,
}

impl SendState {
    fn http_status(self) -> StatusCode {
        match self {
            SendState::Pending => StatusCode::ACCEPTED,
            SendState::Queued | SendState::Sent | SendState::Already => StatusCode::OK,
            SendState::Invalid => StatusCode::UNPROCESSABLE_ENTITY,
            SendState::Unreachable | SendState::Failed => StatusCode::BAD_GATEWAY,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SendStatus {
    pub job: String,
    pub state: SendState,
    pub message: String,
    /// rqbit's torrent id once known.
    pub torrent_id: Option<u64>,
    pub elapsed_ms: u64,
    /// rqbit's add stage while pending (e.g. "resolving_metadata (40 s)").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Add attempts so far (more than 1 after an rqbit timeout).
    pub attempts: u32,
}

#[derive(Clone, Debug)]
struct Outcome {
    state: SendState,
    message: String,
    torrent_id: Option<u64>,
}

impl Outcome {
    fn new(state: SendState, message: impl Into<String>) -> Self {
        Outcome { state, message: message.into(), torrent_id: None }
    }
}

struct Job {
    key: String,
    started: Instant,
    finished: Option<Instant>,
    /// rqbit add_job_id of the current attempt.
    rqbit_job: Option<String>,
    attempts: u32,
    /// Why the send is still pending (e.g. rqbit's add timed out, re-added).
    note: Option<String>,
    rx: watch::Receiver<Option<Outcome>>,
}

#[derive(Default)]
struct Jobs {
    jobs: HashMap<String, Job>,
    /// Client-chosen job ids that joined another job.
    aliases: HashMap<String, String>,
}

static JOBS: Lazy<Mutex<Jobs>> = Lazy::new(|| Mutex::new(Jobs::default()));
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_suffix() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{n}", nanos as u64)
}

/// A job id the page may choose itself (so it can poll even if the POST's
/// answer never arrives).
fn valid_client_job(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// ---------------------------------------------------------------------------
// Link validation

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// A magnet with its BTv1 info hash as 40 lowercase hex characters.
    Magnet { url: String, info_hash: String },
    /// An http(s) URL of a .torrent (or a details page to look the magnet up on).
    Url(String),
}

impl Link {
    fn url(&self) -> &str {
        match self {
            Link::Magnet { url, .. } | Link::Url(url) => url,
        }
    }

    fn key(&self) -> String {
        match self {
            Link::Magnet { info_hash, .. } => info_hash.clone(),
            Link::Url(url) => url.clone(),
        }
    }
}

/// Checks a link before it goes to rqbit. The error says what's wrong.
pub fn validate_link(raw: &str) -> Result<Link, String> {
    let link = raw.trim();
    if link.is_empty() {
        return Err("Invalid link: it is empty".into());
    }
    let lower = link.to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        let info_hash = validate_magnet(link)?;
        return Ok(Link::Magnet { url: link.to_string(), info_hash });
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        let rest = &link[link.find("://").unwrap() + 3..];
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        if host.is_empty() || host.contains(char::is_whitespace) {
            return Err("Invalid .torrent URL: no host name".into());
        }
        return Ok(Link::Url(link.to_string()));
    }
    if link.len() == 40 && link.bytes().all(|b| b.is_ascii_hexdigit()) {
        let info_hash = link.to_ascii_lowercase();
        return Ok(Link::Magnet { url: format!("magnet:?xt=urn:btih:{info_hash}"), info_hash });
    }
    let scheme = link.split(':').next().filter(|s| s.len() < link.len() && s.len() <= 12);
    Err(match scheme {
        Some(s) => format!("Invalid link: \"{s}:\" isn't a magnet: link or an http(s) .torrent URL"),
        None => "Invalid link: expected a magnet: link or an http(s) .torrent URL".into(),
    })
}

/// Returns the BTv1 info hash (40 lowercase hex) of a magnet link.
fn validate_magnet(link: &str) -> Result<String, String> {
    let Some((_, query)) = link.split_once('?') else {
        return Err("Invalid magnet: missing infohash (no xt=urn:btih:… parameter)".into());
    };
    let mut btih: Option<String> = None;
    let mut has_v2 = false;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if k != "xt" {
            continue;
        }
        let v = decode(v).map(|c| c.into_owned()).unwrap_or_else(|_| v.to_string());
        let vl = v.to_ascii_lowercase();
        if let Some(hash) = vl.strip_prefix("urn:btih:") {
            btih.get_or_insert_with(|| v[v.len() - hash.len()..].to_string());
        } else if vl.starts_with("urn:btmh:") {
            has_v2 = true;
        } else {
            let shown: String = v.chars().take(40).collect();
            return Err(format!("Invalid magnet: unsupported xt={shown} (expected urn:btih:)"));
        }
    }
    let Some(hash) = btih else {
        return Err(if has_v2 {
            "Invalid magnet: only a BitTorrent v2 hash (urn:btmh:); rqbit needs xt=urn:btih:".into()
        } else {
            "Invalid magnet: missing infohash (no xt=urn:btih:… parameter)".into()
        });
    };
    let hash = hash.trim();
    if hash.len() == 40 {
        if hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(hash.to_ascii_lowercase());
        }
        return Err("Invalid magnet: infohash has characters that aren't hex (0-9, a-f)".into());
    }
    if hash.len() == 32 {
        return base32_to_hex(hash)
            .ok_or_else(|| "Invalid magnet: infohash isn't valid base32 (A-Z, 2-7)".to_string());
    }
    if hash.is_empty() {
        return Err("Invalid magnet: missing infohash (xt=urn:btih: is empty)".into());
    }
    Err(format!(
        "Invalid magnet: infohash must be 40 hex or 32 base32 characters, not {}",
        hash.chars().count()
    ))
}

fn base32_to_hex(s: &str) -> Option<String> {
    let mut bits: u64 = 0;
    let mut nbits = 0;
    let mut out = String::with_capacity(40);
    for c in s.bytes() {
        let v = match c.to_ascii_uppercase() {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | v as u64;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push_str(&format!("{:02x}", (bits >> nbits) & 0xff));
        }
    }
    (out.len() == 40).then_some(out)
}

// ---------------------------------------------------------------------------
// HTTP handlers

fn respond(status: SendStatus) -> Response {
    (status.state.http_status(), Json(status)).into_response()
}

#[debug_handler]
pub async fn endpoint(Form(payload): Form<SendToTransmission>) -> Response {
    // The form body is already URL-decoded. Decoding a magnet again would
    // unescape its dn/tr values (%26 -> '&' splits the magnet), so only
    // fall back to decoding for a value that arrives still escaped
    // ("magnet%3A%3Fxt=...").
    let raw = payload.magnet.trim();
    let lower = raw.to_ascii_lowercase();
    let decoded = if lower.starts_with("magnet:") || lower.starts_with("http://") || lower.starts_with("https://") {
        raw.to_string()
    } else {
        decode(raw).map(|c| c.into_owned()).unwrap_or_else(|_| raw.to_string())
    };
    let client_job = payload.job.as_deref().map(str::trim).filter(|j| valid_client_job(j)).map(str::to_string);
    let job_id = client_job.clone().unwrap_or_else(|| format!("anorak-{}", unique_suffix()));

    let link = match validate_link(&decoded) {
        Ok(link) => link,
        Err(message) => {
            info!("not sending {}: {message}", decoded.chars().take(120).collect::<String>());
            return respond(SendStatus {
                job: job_id,
                state: SendState::Invalid,
                message,
                torrent_id: None,
                elapsed_ms: 0,
                stage: None,
                attempts: 0,
            });
        }
    };
    let category = payload
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok());
    let site_category = SendCategory::from_form(
        payload.category_source.as_deref(),
        payload.category_id.as_deref(),
        payload.category_label.as_deref(),
    );
    let indexer = payload
        .indexer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let (job, rx) = start_or_join(job_id, link, category, site_category, indexer).await;
    respond(wait_status(&job, rx, HANDOFF_WAIT).await)
}

/// `GET /api/send/{job}`: where a send stands (poll while `pending`).
#[debug_handler]
pub async fn status(Path(job): Path<String>) -> Response {
    let found = {
        let jobs = JOBS.lock().await;
        let id = jobs.aliases.get(&job).cloned().unwrap_or(job);
        jobs.jobs.get(&id).map(|j| (id.clone(), j.rx.clone(), j.rqbit_job.clone()))
    };
    let Some((id, rx, rqbit_job)) = found else {
        return (StatusCode::NOT_FOUND, "no such send (anorak may have restarted)").into_response();
    };
    let mut status = wait_status(&id, rx, Duration::ZERO).await;
    if status.state == SendState::Pending {
        if let Some(rqbit_job) = rqbit_job {
            status.stage = rqbit_stage(&rqbit_job).await;
        }
    }
    respond(status)
}

/// Joins an in-flight send of the same torrent (or the same job id), or
/// starts a new one.
async fn start_or_join(
    job_id: String,
    link: Link,
    category: Option<u32>,
    site_category: Option<SendCategory>,
    indexer: Option<String>,
) -> (String, watch::Receiver<Option<Outcome>>) {
    let key = link.key();
    let mut guard = JOBS.lock().await;
    let jobs = &mut *guard;
    jobs.jobs.retain(|_, j| j.finished.map_or(true, |at| at.elapsed() < KEEP_FINISHED));
    let live: std::collections::HashSet<String> = jobs.jobs.keys().cloned().collect();
    jobs.aliases.retain(|_, target| live.contains(target));

    // The page re-posting a job it already started (its first answer was lost).
    let existing = jobs.aliases.get(&job_id).cloned().unwrap_or_else(|| job_id.clone());
    if let Some(j) = jobs.jobs.get(&existing) {
        if j.key == key && j.rx.borrow().as_ref().map_or(true, |o| o.state != SendState::Invalid) {
            return (existing, j.rx.clone());
        }
    }
    if let Some((id, j)) = jobs
        .jobs
        .iter()
        .find(|(_, j)| j.key == key && j.finished.is_none() && j.rx.borrow().is_none())
    {
        info!("send of {key} already in flight as {id}; joining it");
        let (id, rx) = (id.clone(), j.rx.clone());
        if id != job_id {
            jobs.aliases.insert(job_id, id.clone());
        }
        return (id, rx);
    }
    let id = if jobs.jobs.contains_key(&job_id) { format!("anorak-{}", unique_suffix()) } else { job_id };
    let (tx, rx) = watch::channel(None);
    jobs.jobs.insert(
        id.clone(),
        Job {
            key: key.clone(),
            started: Instant::now(),
            finished: None,
            rqbit_job: None,
            attempts: 0,
            note: None,
            rx: rx.clone(),
        },
    );
    drop(guard);

    let job_id = id.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        let outcome = run_send(link, category, site_category, indexer, &job_id).await;
        let ms = started.elapsed().as_millis();
        match outcome.state {
            SendState::Invalid | SendState::Unreachable | SendState::Failed => {
                warn!("send {job_id} ({key}): {:?} after {ms} ms: {}", outcome.state, outcome.message)
            }
            state => info!("send {job_id} ({key}): {state:?} after {ms} ms"),
        }
        if let Some(j) = JOBS.lock().await.jobs.get_mut(&job_id) {
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
    let (elapsed_ms, attempts, note) = {
        let jobs = JOBS.lock().await;
        match jobs.jobs.get(job) {
            Some(j) => (
                match j.finished {
                    Some(finished) => finished.duration_since(j.started).as_millis() as u64,
                    None => j.started.elapsed().as_millis() as u64,
                },
                j.attempts,
                j.note.clone(),
            ),
            None => (0, 0, None),
        }
    };
    let current = rx.borrow().clone();
    let (state, message, torrent_id) = match current {
        Some(o) => (o.state, o.message, o.torrent_id),
        None => (
            SendState::Pending,
            note.unwrap_or_else(|| "rqbit hasn't confirmed yet; anorak keeps the request going".to_string()),
            None,
        ),
    };
    SendStatus { job: job.to_string(), state, message, torrent_id, elapsed_ms, stage: None, attempts }
}

async fn update_job(job_id: &str, f: impl FnOnce(&mut Job)) {
    if let Some(j) = JOBS.lock().await.jobs.get_mut(job_id) {
        f(j);
    }
}

// ---------------------------------------------------------------------------
// Talking to rqbit

/// Why one add attempt didn't succeed.
#[derive(Debug, PartialEq, Eq)]
enum AddError {
    /// Timed out / interrupted while rqbit was still working: add again.
    Waiting(String),
    Invalid(String),
    Unreachable(String),
    Failed(String),
}

async fn run_send(
    link: Link,
    category: Option<u32>,
    site_category: Option<SendCategory>,
    indexer: Option<String>,
    job_id: &str,
) -> Outcome {
    let link = match (&link, indexer) {
        (Link::Url(url), Some(indexer)) if torrent_files::is_page_link(url) => {
            // A details page (0Magnet, 1337x) is not something the client can
            // add; send the magnet printed on it. A .torrent is left as is.
            update_job(job_id, |j| j.note = Some("fetching the magnet from the result's page".to_string())).await;
            let found = torrent_files::magnet_for_link(url, &indexer).await;
            update_job(job_id, |j| j.note = None).await;
            match found {
                Ok(Some(magnet)) => match validate_link(&magnet) {
                    Ok(m) => {
                        info!("sending the magnet from {url}");
                        m
                    }
                    Err(message) => return Outcome::new(SendState::Invalid, format!("{message} (from {url})")),
                },
                Ok(None) => link,
                // A 1337x page is never a .torrent, so there is nothing to fall
                // back to: the site (not the magnet) is the problem. Retryable.
                Err(err) if crate::x1337::handles(&indexer) => {
                    warn!("no magnet from {url}: {err:#}");
                    return Outcome::new(SendState::Unreachable, format!("Couldn't get the magnet from 1337x: {err}"));
                }
                Err(err) => {
                    warn!("no magnet from {url}: {err}");
                    link
                }
            }
        }
        _ => link,
    };

    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let rqbit_job = format!("{}-a{attempt}", job_id.chars().take(100).collect::<String>());
        update_job(job_id, |j| {
            j.attempts = attempt;
            j.rqbit_job = Some(rqbit_job.clone());
        })
        .await;
        match add_once(&link, category, site_category.as_ref(), &rqbit_job).await {
            Ok(outcome) => return outcome,
            Err(AddError::Waiting(why)) => {
                let wait = Duration::from_secs((5 * attempt as u64).min(30));
                info!("send {job_id}: {why}; adding again in {} s (attempt {})", wait.as_secs(), attempt + 1);
                let note = match &link {
                    Link::Magnet { .. } => format!("{why}; still waiting for peers, re-added to rqbit (attempt {})", attempt + 1),
                    Link::Url(_) => format!("{why}; trying again (attempt {})", attempt + 1),
                };
                update_job(job_id, |j| {
                    j.note = Some(note);
                    // The old add job is over; don't show its stage.
                    j.rqbit_job = None;
                })
                .await;
                tokio::time::sleep(wait).await;
            }
            Err(AddError::Invalid(m)) => return Outcome::new(SendState::Invalid, m),
            Err(AddError::Unreachable(m)) => return Outcome::new(SendState::Unreachable, m),
            Err(AddError::Failed(m)) => return Outcome::new(SendState::Failed, m),
        }
    }
}

fn rqbit_base() -> String {
    CONFIG.rqbit_url.trim_end_matches('/').to_string()
}

/// A Nyaa/sukebei category, forwarded to rqbit as `category_source`,
/// `category_id` and `category` next to the numeric `torznab_category`.
/// Each part is only sent when it is valid.
#[derive(Clone, Debug, Default, PartialEq)]
struct SendCategory {
    source: Option<String>,
    id: Option<String>,
    label: Option<String>,
}

/// rqbit's limit for `category`.
const CATEGORY_LABEL_MAX: usize = 100;

impl SendCategory {
    fn from_form(source: Option<&str>, id: Option<&str>, label: Option<&str>) -> Option<SendCategory> {
        let source = source
            .map(str::trim)
            .filter(|s| matches!(*s, "nyaa" | "sukebei"))
            .map(str::to_string);
        let id = id.map(str::trim).filter(|s| is_site_category_id(s)).map(str::to_string);
        let label = label
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
            .map(|s| s.chars().take(CATEGORY_LABEL_MAX).collect::<String>().trim_end().to_string());
        let category = SendCategory { source, id, label };
        (category != SendCategory::default()).then_some(category)
    }
}

/// "1_2": digits, an underscore, digits.
fn is_site_category_id(s: &str) -> bool {
    match s.split_once('_') {
        Some((a, b)) => {
            !a.is_empty() && !b.is_empty() && a.len() <= 3 && b.len() <= 3
                && a.chars().chain(b.chars()).all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Category query parameters for rqbit's `POST /torrents`.
fn category_params(torznab_category: Option<u32>, site: Option<&SendCategory>) -> Vec<(&'static str, String)> {
    let mut params = Vec::new();
    if let Some(id) = torznab_category {
        params.push(("torznab_category", id.to_string()));
    }
    if let Some(site) = site {
        if let Some(label) = &site.label {
            params.push(("category", label.clone()));
        }
        if let Some(source) = &site.source {
            params.push(("category_source", source.clone()));
        }
        if let Some(id) = &site.id {
            params.push(("category_id", id.clone()));
        }
    }
    params
}

async fn add_once(
    link: &Link,
    torznab_category: Option<u32>,
    site_category: Option<&SendCategory>,
    rqbit_job: &str,
) -> Result<Outcome, AddError> {
    let mut params: Vec<(&str, String)> = category_params(torznab_category, site_category);
    if let Link::Magnet { .. } = link {
        // Return as soon as the magnet is queued; rqbit fetches the metadata
        // in the background and keeps trying for MAGNET_RESOLVE_SECS.
        params.push(("defer_metadata", "true".to_string()));
        params.push(("magnet_timeout_secs", MAGNET_RESOLVE_SECS.to_string()));
    }
    params.push(("timeout_ms", RQBIT_ADD_TIMEOUT_MS.to_string()));
    // Lets `GET /api/send/{job}` show rqbit's stage (/add_jobs/{id}).
    params.push(("add_job_id", rqbit_job.to_string()));
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
        .collect();
    let url = format!("{}/torrents?{}", rqbit_base(), query.join("&"));

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(RQBIT_REQUEST_TIMEOUT)
        .build()
        .map_err(|e| AddError::Failed(format!("couldn't set up the request: {e}")))?;
    let response = client
        .post(url)
        .body(link.url().to_string())
        .send()
        .await
        .map_err(|err| {
            if err.is_connect() {
                AddError::Unreachable(format!("nothing answered at {}", CONFIG.rqbit_url))
            } else if err.is_timeout() {
                AddError::Waiting("rqbit didn't answer within an hour".into())
            } else {
                AddError::Waiting(format!("the connection to rqbit was interrupted ({err})"))
            }
        })?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(classify_rqbit_error(status, &rqbit_error_text(&text)));
    }
    let mut outcome = outcome_from_rqbit(&text);
    if outcome.state == SendState::Already && response_flag(&text, "resolving") {
        if let Some(id) = outcome.torrent_id {
            if restart_if_failed(id).await {
                outcome.state = SendState::Queued;
                outcome.message =
                    "already in rqbit, which had stopped looking for its metadata; restarted the search for peers".into();
            }
        }
    }
    Ok(outcome)
}

fn response_flag(text: &str, key: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get(key).and_then(|b| b.as_bool()))
        .unwrap_or(false)
}

/// rqbit answers `{"id":…, "resolving":true, "already_managed":…}`.
fn outcome_from_rqbit(text: &str) -> Outcome {
    let value: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let flag = |k: &str| value.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let torrent_id = value.get("id").and_then(|v| v.as_u64());
    let (state, message) = if flag("already_managed") {
        (SendState::Already, "rqbit already has this torrent")
    } else if flag("resolving") {
        (SendState::Queued, "queued in rqbit; waiting for peers to send the torrent's metadata")
    } else {
        (SendState::Sent, "added to rqbit")
    };
    Outcome { state, message: message.to_string(), torrent_id }
}

/// Sorts an rqbit error answer into waiting / invalid / unreachable / failed.
fn classify_rqbit_error(status: reqwest::StatusCode, msg: &str) -> AddError {
    use reqwest::StatusCode;
    let lower = msg.to_ascii_lowercase();
    if lower.contains("timeout") || lower.contains("timed out") {
        return AddError::Waiting("rqbit's add timed out while it was still fetching the metadata".into());
    }
    if status == StatusCode::BAD_GATEWAY || status == StatusCode::SERVICE_UNAVAILABLE || status == StatusCode::GATEWAY_TIMEOUT {
        return AddError::Unreachable(format!("rqbit isn't available: {status}: {msg}"));
    }
    const INVALID: &[&str] = &[
        "not a valid magnet",
        "magnet link must be",
        "expected scheme",
        "expected xt",
        "infohash",
        "info hash",
        "btih",
        "btmh",
        "base32",
        "hex",
        "decoding torrent",
        "error decoding",
        "bencode",
        "unsupported url",
        "not a torrent",
        "invalid torrent",
        "invalid magnet",
    ];
    let bad_download = lower.starts_with("error adding torrent: get ") && lower.contains(" returned 4");
    if status.is_client_error() && !lower.contains("add_job_id") && (bad_download || INVALID.iter().any(|p| lower.contains(p))) {
        return AddError::Invalid(format!("rqbit rejected it as invalid: {msg}"));
    }
    AddError::Failed(format!("{status}: {msg}"))
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

/// A magnet placeholder rqbit gave up on ("Metadata failed") is started
/// again. Returns true if it was restarted.
async fn restart_if_failed(id: u64) -> bool {
    let client = reqwest::Client::new();
    let stats = async {
        let r = client
            .get(format!("{}/torrents/{id}/stats/v1", rqbit_base()))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .ok()?;
        serde_json::from_str::<serde_json::Value>(&r.text().await.ok()?).ok()
    }
    .await;
    let failed = stats
        .as_ref()
        .and_then(|s| s.get("state"))
        .and_then(|s| s.as_str())
        .map_or(false, |s| s.eq_ignore_ascii_case("error"));
    if !failed {
        return false;
    }
    let ok = client
        .post(format!("{}/torrents/{id}/start", rqbit_base()))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    info!("rqbit placeholder {id} had failed; restart {}", if ok { "requested" } else { "failed" });
    ok
}

async fn rqbit_stage(rqbit_job: &str) -> Option<String> {
    let url = format!("{}/add_jobs/{rqbit_job}", rqbit_base());
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
    if matches!(stage.as_str(), "cancelled" | "failed" | "added" | "already_managed" | "list_only") {
        return None;
    }
    let secs = value.get("stage_secs").and_then(|v| v.as_f64());
    Some(match secs {
        Some(s) if s >= 120.0 => format!("{stage} ({:.0} min)", s / 60.0),
        Some(s) => format!("{stage} ({s:.0} s)"),
        None => stage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_params_for_rqbit() {
        // Torznab id only (every non-Nyaa source).
        assert_eq!(category_params(Some(2040), None), vec![("torznab_category", "2040".to_string())]);
        assert!(category_params(None, None).is_empty());

        let site = SendCategory::from_form(Some("nyaa"), Some("1_2"), Some("Anime - English-translated"));
        assert_eq!(
            category_params(Some(5070), site.as_ref()),
            vec![
                ("torznab_category", "5070".to_string()),
                ("category", "Anime - English-translated".to_string()),
                ("category_source", "nyaa".to_string()),
                ("category_id", "1_2".to_string()),
            ]
        );
    }

    #[test]
    fn send_category_validation() {
        assert_eq!(SendCategory::from_form(None, None, None), None);
        assert_eq!(SendCategory::from_form(Some(""), Some(" "), Some("  ")), None);
        // Each part is checked on its own and dropped when invalid.
        let c = SendCategory::from_form(Some("piratebay"), Some("1_2"), Some("Art - Doujinshi")).unwrap();
        assert_eq!((c.source, c.id.as_deref(), c.label.as_deref()), (None, Some("1_2"), Some("Art - Doujinshi")));
        let c = SendCategory::from_form(Some(" sukebei "), Some("1-2"), None).unwrap();
        assert_eq!((c.source.as_deref(), c.id, c.label), (Some("sukebei"), None, None));
        for bad in ["12", "_2", "1_", "a_b", "1_2_3", "1234_1", "1_2 "] {
            assert!(!is_site_category_id(bad), "{bad}");
        }
        assert!(is_site_category_id("1_2") && is_site_category_id("6_2"));
        // Labels: whitespace folded, control characters refused, at most 100 chars.
        let c = SendCategory::from_form(None, None, Some(" Anime -\n English ")).unwrap();
        assert_eq!(c.label.as_deref(), Some("Anime - English"));
        assert_eq!(SendCategory::from_form(None, None, Some("bad\u{7}label")), None);
        let long = "x".repeat(150);
        let c = SendCategory::from_form(None, None, Some(&long)).unwrap();
        assert_eq!(c.label.unwrap().chars().count(), 100);
    }

    const H: &str = "c9e15763f722f23e98a29decdfae341b98d53056";

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

    #[test]
    fn validates_magnets() {
        let ok = validate_link(&format!("magnet:?xt=urn:btih:{}&dn=x&tr=udp%3A%2F%2Ft%3A1", H.to_uppercase())).unwrap();
        assert_eq!(ok.key(), H);
        // Base32 of the same hash.
        let b32 = validate_link("magnet:?dn=x&xt=urn:btih:ZHQVOY7XELZD5GFCTXWN7LRUDOMNKMCW").unwrap();
        assert_eq!(b32.key(), H);
        assert_eq!(validate_link(H).unwrap().key(), H);
        assert!(matches!(validate_link("https://nyaa.si/download/1.torrent"), Ok(Link::Url(_))));

        let err = |s: &str| validate_link(s).unwrap_err();
        assert_eq!(err("magnet:?dn=only-a-name"), "Invalid magnet: missing infohash (no xt=urn:btih:… parameter)");
        assert_eq!(err("magnet:"), "Invalid magnet: missing infohash (no xt=urn:btih:… parameter)");
        assert_eq!(err("magnet:?xt=urn:btih:abc123"), "Invalid magnet: infohash must be 40 hex or 32 base32 characters, not 6");
        assert!(err(&format!("magnet:?xt=urn:btih:{}zz", &H[..38])).contains("aren't hex"));
        assert!(err("magnet:?xt=urn:btih:ZHQVOY7XELZD5GFCTXWN7LRUDOMNKMC1").contains("base32"));
        assert!(err("magnet:?xt=urn:btih:").contains("empty"));
        assert!(err("magnet:?xt=urn:ed2k:abcdef").contains("unsupported xt"));
        assert!(err("magnet:?xt=urn:btmh:1220abcd").contains("v2"));
        assert!(err("magnett:?xt=urn:btih:x").contains("\"magnett:\""));
        assert!(err("ftp://example.com/a.torrent").contains("\"ftp:\""));
        assert!(err("hello").contains("expected a magnet"));
        assert!(err("https:///nohost").contains("no host"));
        assert!(err("   ").contains("empty"));
    }

    #[test]
    fn classifies_rqbit_errors() {
        use AddError::*;
        let c = |code: u16, m: &str| classify_rqbit_error(reqwest::StatusCode::from_u16(code).unwrap(), m);
        assert!(matches!(c(500, "timeout"), Waiting(_)));
        assert!(matches!(c(400, "error adding torrent: timed out after 900s waiting for torrent metadata from peers"), Waiting(_)));
        assert!(matches!(c(400, "error adding torrent: provided path is not a valid magnet URL"), Invalid(_)));
        assert!(matches!(c(400, "error adding torrent: error decoding torrent"), Invalid(_)));
        assert!(matches!(c(400, "error adding torrent: GET https://x/y.torrent returned 404 Not Found"), Invalid(_)));
        assert!(matches!(c(400, "error adding torrent: GET https://x/y.torrent returned 500 Internal Server Error"), Failed(_)));
        assert!(matches!(c(400, "invalid add_job_id (use 1-128 of [A-Za-z0-9_-])"), Failed(_)));
        assert!(matches!(c(500, "disk full"), Failed(_)));
        assert!(matches!(c(503, "starting up"), Unreachable(_)));
    }
}
