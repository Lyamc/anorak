//! Lodestarr's native JSON API, next to its Torznab feed.
//!
//! The Torznab feed (`JACKETT_URL`) has no source tag on results, ignores
//! `cat=`, and labels every result without a category as 5000 (TV). The
//! native API lists the installed sources (`/api/native/local`) and searches
//! one source at a time (`/api/native/search?q=&indexer=`), returning each
//! result's source and its real categories (empty when the source gave none).
//! When the native API isn't there (e.g. JACKETT_URL points at Jackett),
//! `indexers()` returns an empty list and callers fall back to Torznab.

use crate::config::CONFIG;
use crate::models::{self, Item};

use anyhow::{anyhow, Result};
use chrono::DateTime;
use log::warn;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Indexer {
    pub id: String,
    pub name: String,
    /// Torznab category ids the source says it carries.
    #[serde(default)]
    pub categories: Vec<u32>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing)]
    pub links: Vec<String>,
    #[serde(default, skip_serializing)]
    pub legacylinks: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Indexer {
    /// The one top-level category (e.g. 6000 for an XXX-only source), if the
    /// source declares exactly one. Used to label results that have none.
    pub fn only_category(&self) -> Option<u32> {
        let mut tops: Vec<u32> = self
            .categories
            .iter()
            .filter(|c| **c < 100_000)
            .map(|c| c / 1000 * 1000)
            .collect();
        tops.sort_unstable();
        tops.dedup();
        match tops.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    /// Host names of the source's sites (for validating .torrent links).
    pub fn hosts(&self) -> Vec<String> {
        self.links
            .iter()
            .chain(self.legacylinks.iter())
            .filter_map(|l| reqwest::Url::parse(l).ok())
            .filter_map(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
            .collect()
    }
}

#[derive(Deserialize)]
struct LocalList {
    #[serde(default)]
    indexers: Vec<Indexer>,
}

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("reqwest client")
});

static INDEXERS: Lazy<Mutex<Option<(Instant, Vec<Indexer>)>>> = Lazy::new(|| Mutex::new(None));
const INDEXER_TTL: Duration = Duration::from_secs(60);

/// scheme://host:port of JACKETT_URL (Lodestarr serves both APIs there).
pub fn base_url() -> Option<String> {
    let url = reqwest::Url::parse(&CONFIG.jackett_url).ok()?;
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

/// Installed sources, cached for a minute. Empty if the native API isn't
/// available (the failure is cached too, so searches don't keep waiting on it).
pub async fn indexers() -> Vec<Indexer> {
    {
        let cached = INDEXERS.lock().await;
        if let Some((at, list)) = cached.as_ref() {
            if at.elapsed() < INDEXER_TTL {
                return list.clone();
            }
        }
    }
    let list = match fetch_indexers().await {
        Ok(list) => list,
        Err(err) => {
            warn!("Lodestarr source list unavailable, using Torznab only: {err}");
            Vec::new()
        }
    };
    *INDEXERS.lock().await = Some((Instant::now(), list.clone()));
    list
}

async fn fetch_indexers() -> Result<Vec<Indexer>> {
    let base = base_url().ok_or_else(|| anyhow!("JACKETT_URL has no host"))?;
    let response = CLIENT
        .get(format!("{base}/api/native/local"))
        .timeout(Duration::from_secs(10))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(anyhow!("/api/native/local returned {}", response.status()));
    }
    let body = response.text().await?;
    let list: LocalList = serde_json::from_str(&body)?;
    Ok(list.indexers)
}

/// Search the given sources in parallel and merge the results (duplicates
/// by info-hash are folded into one row listing every source). Returns the
/// items and the sources that failed, as "name" or "name (reason)", e.g.
/// "1337x (blocked by Cloudflare)".
pub async fn search(query: &str, sources: &[Indexer]) -> (Vec<Item>, Vec<String>) {
    let mut set = tokio::task::JoinSet::new();
    for (index, source) in sources.iter().cloned().enumerate() {
        let query = query.to_string();
        set.spawn(async move {
            // Nyaa/sukebei: the site's RSS carries categories, Lodestarr's
            // results don't. Lodestarr is the fallback.
            let rss = if crate::nyaa::handles(&source.id) {
                match crate::nyaa::search(&query, &source).await {
                    Ok(items) => Some(items),
                    Err(err) => {
                        warn!("{} RSS failed ({err:#}); asking Lodestarr", source.name);
                        None
                    }
                }
            } else {
                None
            };
            let result = if crate::x1337::handles(&source.id) {
                crate::x1337::search(&query, &source).await
            } else if let Some(items) = rss {
                Ok(items)
            } else {
                match search_source(&query, &source).await {
                    // Lodestarr answers a blocked or unreachable site with an
                    // empty list; check the site so the user hears about it.
                    Ok(items) if items.is_empty() => match probe_site(&source).await {
                        Some(label) => Err(crate::flaresolverr::failure(label)),
                        None => Ok(items),
                    },
                    other => other,
                }
            };
            (index, source.name, result)
        });
    }
    let mut parts: Vec<(usize, Vec<Item>)> = Vec::new();
    let mut failed = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((index, _, Ok(items))) => parts.push((index, items)),
            Ok((_, name, Err(err))) => {
                warn!("source {name} failed: {err:#}");
                failed.push(failed_label(&name, &err));
            }
            Err(err) => warn!("source search task failed: {err}"),
        }
    }
    parts.sort_by_key(|(index, _)| *index);
    let items = dedupe(parts.into_iter().flat_map(|(_, items)| items).collect());
    failed.sort();
    (items, failed)
}

fn failed_label(name: &str, err: &anyhow::Error) -> String {
    match err.downcast_ref::<crate::flaresolverr::Failure>() {
        Some(f) => format!("{name} ({})", f.0),
        None => name.to_string(),
    }
}

/// Results of `probe_site`, per source id, for a few minutes.
static PROBES: Lazy<Mutex<HashMap<String, (Instant, Option<String>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
const PROBE_TTL: Duration = Duration::from_secs(10 * 60);
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// One GET of the source's home page (from the same VPN namespace as
/// Lodestarr) when it returned nothing: "blocked by Cloudflare" when the
/// site answers with Cloudflare's challenge, "site unreachable" when it
/// doesn't answer at all, else None (the search really found nothing).
async fn probe_site(source: &Indexer) -> Option<String> {
    let url = source.links.first()?.clone();
    if let Some((at, label)) = PROBES.lock().await.get(&source.id) {
        if at.elapsed() < PROBE_TTL {
            return label.clone();
        }
    }
    let label = match CLIENT
        .get(&url)
        .header(reqwest::header::USER_AGENT, BROWSER_UA)
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let challenged = response
                .headers()
                .get("cf-mitigated")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.eq_ignore_ascii_case("challenge"));
            let cloudflare = response
                .headers()
                .get(reqwest::header::SERVER)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.to_ascii_lowercase().contains("cloudflare"));
            let body = if cloudflare && !challenged && status >= 400 {
                response.text().await.unwrap_or_default()
            } else {
                String::new()
            };
            let title = crate::flaresolverr::page_title(&body);
            if challenged || (cloudflare && crate::flaresolverr::is_cloudflare_page(status, &title, &body)) {
                Some("blocked by Cloudflare".to_string())
            } else {
                None
            }
        }
        Err(err) if err.is_connect() || err.is_timeout() => Some("site unreachable".to_string()),
        Err(_) => None,
    };
    if let Some(label) = &label {
        warn!("{} returned nothing and {url} looks {label}", source.name);
    }
    PROBES.lock().await.insert(source.id.clone(), (Instant::now(), label.clone()));
    label
}

async fn search_source(query: &str, source: &Indexer) -> Result<Vec<Item>> {
    let base = base_url().ok_or_else(|| anyhow!("JACKETT_URL has no host"))?;
    let url = format!(
        "{base}/api/native/search?q={}&indexer={}",
        urlencoding::encode(query),
        urlencoding::encode(&source.id)
    );
    let response = CLIENT.get(url).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("returned {}", response.status()));
    }
    let body = response.text().await?;
    let value: Value = serde_json::from_str(&body)?;
    let results = value
        .as_array()
        .or_else(|| value.get("results").and_then(Value::as_array))
        .ok_or_else(|| anyhow!("unexpected response shape"))?;
    Ok(results.iter().filter_map(|r| to_item(r, source)).collect())
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn number(value: &Value, key: &str) -> u64 {
    match value.get(key) {
        Some(v) => v
            .as_u64()
            .or_else(|| v.as_f64().filter(|f| *f > 0.0).map(|f| f as u64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            .unwrap_or(0),
        None => 0,
    }
}

fn to_item(value: &Value, source: &Indexer) -> Option<Item> {
    let title = text(value, "title");
    if title.is_empty() {
        return None;
    }
    let link = text(value, "link");
    let magnet = text(value, "magnet");
    let comments = text(value, "comments");
    let guid = [text(value, "guid"), comments.clone(), link.clone(), magnet.clone()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_else(|| title.clone());
    // The rest of the app expects RSS-style dates.
    let raw_date = text(value, "publish_date");
    let pub_date = DateTime::parse_from_rfc3339(&raw_date)
        .map(|d| d.format("%a, %d %b %Y %H:%M:%S %z").to_string())
        .unwrap_or(raw_date);
    let mut category: Vec<String> = value
        .get("categories")
        .and_then(Value::as_array)
        .map(|cats| {
            cats.iter()
                .filter_map(|c| c.as_u64().or_else(|| c.as_str().and_then(|s| s.parse().ok())))
                .map(|c| c.to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut category_inferred = false;
    if category.is_empty() {
        if let Some(only) = source.only_category() {
            category.push(only.to_string());
            category_inferred = true;
        }
    }
    let source_name = Some(text(value, "indexer"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| source.name.clone());
    Some(Item {
        title,
        guid,
        link,
        comments,
        pub_date,
        size: number(value, "size"),
        files: 0,
        description: String::new(),
        category,
        seeders: number(value, "seeders").min(u32::MAX as u64) as u32,
        peers: number(value, "leechers").min(u32::MAX as u64) as u32,
        magneturl: magnet,
        sources: vec![source_name],
        source_ids: vec![source.id.clone()],
        category_inferred,
        category_label: String::new(),
        category_source: String::new(),
        category_site_id: String::new(),
    })
}

/// Fold results with the same info-hash into one, keeping every source.
fn dedupe(items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    let mut seen: HashMap<String, usize> = HashMap::new();
    for item in items {
        let hash = models::info_hash(&item.magnet_link());
        let Some(hash) = hash else {
            out.push(item);
            continue;
        };
        if let Some(&at) = seen.get(&hash) {
            let kept = &mut out[at];
            for (name, id) in item.sources.iter().zip(item.source_ids.iter()) {
                if !kept.source_ids.contains(id) {
                    kept.sources.push(name.clone());
                    kept.source_ids.push(id.clone());
                }
            }
            kept.seeders = kept.seeders.max(item.seeders);
            kept.peers = kept.peers.max(item.peers);
            if (kept.category.is_empty() || kept.category_inferred) && !item.category.is_empty() && !item.category_inferred {
                kept.category = item.category;
                kept.category_inferred = false;
            }
            if kept.category_label.is_empty() && !item.category_label.is_empty() {
                kept.category_label = item.category_label;
                kept.category_source = item.category_source;
                kept.category_site_id = item.category_site_id;
            }
        } else {
            seen.insert(hash, out.len());
            out.push(item);
        }
    }
    out
}
