//! File list of one torrent, looked up on demand (never during a search).
//!
//! Neither Lodestarr's Torznab feed nor its native API reports files, so the
//! list comes from the .torrent metadata itself:
//! * a direct .torrent link is fetched through Lodestarr's
//!   `/api/v2.0/indexers/{id}/dl?link=` proxy (only for hosts of that source);
//! * a magnet is resolved by rqbit's `POST /torrents/resolve_magnet`, which
//!   returns the .torrent bytes and adds nothing to the session;
//! * a result with only a details-page link (0Magnet) has that page fetched
//!   through the same proxy and the magnet on it resolved as above.
//! Results are cached in memory and at most a few lookups run at once.

use crate::config::CONFIG;
use crate::lodestarr;
use crate::models;

use anyhow::{anyhow, bail, Result};
use log::info;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    /// Small supporting file (nfo/txt/image/sample, or under 1% of the total).
    pub extra: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub count: usize,
    pub total: u64,
    pub main_count: usize,
    pub main_total: u64,
    pub extras_count: usize,
    pub extras_total: u64,
    pub largest: Option<FileEntry>,
    /// Largest first; at most MAX_LISTED entries.
    pub files: Vec<FileEntry>,
    /// "torrent" (fetched .torrent) or "magnet" (resolved by rqbit).
    pub via: &'static str,
}

const MAX_LISTED: usize = 200;
const MAX_TORRENT_BYTES: usize = 20 * 1024 * 1024;
const CACHE_CAP: usize = 1000;

static CACHE: Lazy<Mutex<HashMap<String, FileInfo>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static LIMIT: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(4));
static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .expect("reqwest client")
});

pub fn is_magnet(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.starts_with("magnet:?") && lower.contains("xt=urn:btih:")
}

fn is_http(s: &str) -> bool {
    let lower = s.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Look up the file list. `torrent` + `indexer` are tried first when given
/// (deterministic and fast), then the magnet. `magnet` may instead be the
/// result's http link (details page or .torrent) when it has no magnet.
pub async fn lookup(magnet: &str, torrent: &str, indexer: &str) -> Result<FileInfo> {
    let magnet = magnet.trim();
    let torrent = torrent.trim();
    let indexer = indexer.trim();
    let page = (!is_magnet(magnet) && is_http(magnet) && !indexer.is_empty()).then_some(magnet);
    let key = models::info_hash(magnet)
        .or_else(|| (!torrent.is_empty()).then(|| torrent.to_string()))
        .or_else(|| page.map(str::to_string))
        .ok_or_else(|| anyhow!("no magnet or .torrent link for this result"))?;
    if let Some(hit) = CACHE.lock().await.get(&key) {
        return Ok(hit.clone());
    }
    let _permit = LIMIT.acquire().await?;
    let mut errors = Vec::new();
    let mut found = None;
    if !torrent.is_empty() && !indexer.is_empty() {
        match fetch_torrent(torrent, indexer).await.and_then(|b| parse(&b, "torrent")) {
            Ok(info) => found = Some(info),
            Err(err) => errors.push(format!(".torrent: {err}")),
        }
    }
    if found.is_none() && torrent.is_empty() {
        if let Some(page) = page {
            match from_page(page, indexer).await {
                Ok(info) => found = Some(info),
                Err(err) => errors.push(format!("link: {err}")),
            }
        }
    }
    if found.is_none() && is_magnet(magnet) {
        match resolve_magnet(magnet).await.and_then(|b| parse(&b, "magnet")) {
            Ok(info) => found = Some(info),
            Err(err) => errors.push(format!("magnet: {err}")),
        }
    }
    let info = match found {
        Some(info) => info,
        None if errors.is_empty() => bail!("no magnet or .torrent link for this result"),
        None => bail!("{}", errors.join("; ")),
    };
    info!("file list for {key}: {} files via {}", info.count, info.via);
    let mut cache = CACHE.lock().await;
    if cache.len() >= CACHE_CAP {
        cache.clear();
    }
    cache.insert(key, info.clone());
    Ok(info)
}

/// The link is either a .torrent or a details page carrying a magnet.
async fn from_page(link: &str, indexer: &str) -> Result<FileInfo> {
    let body = fetch_proxied(link, indexer).await?;
    if body.first() == Some(&b'd') {
        if let Ok(info) = parse(&body, "torrent") {
            return Ok(info);
        }
    }
    let magnet = magnet_in_page(&body).ok_or_else(|| anyhow!("no magnet on the result's page"))?;
    let bytes = resolve_magnet(&magnet).await?;
    parse(&bytes, "magnet")
}

/// First `magnet:?xt=urn:btih:` link in an HTML page (entities decoded).
fn magnet_in_page(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    let lower = text.to_ascii_lowercase();
    let start = lower.find("magnet:?xt=urn:btih:")?;
    let end = text[start..]
        .find(|c: char| c == '"' || c == '\'' || c == '<' || c == '>' || c.is_whitespace())
        .map(|i| start + i)
        .unwrap_or(text.len());
    let magnet = text[start..end].replace("&amp;", "&");
    is_magnet(&magnet).then_some(magnet)
}

async fn fetch_torrent(link: &str, indexer: &str) -> Result<Vec<u8>> {
    let bytes = fetch_proxied(link, indexer).await?;
    if bytes.first() != Some(&b'd') {
        bail!("response is not a torrent file");
    }
    Ok(bytes)
}

/// GET a result link through Lodestarr's per-source download proxy, which
/// runs in the VPN namespace and knows the site's cookies. Only links on the
/// named source's own sites are accepted.
async fn fetch_proxied(link: &str, indexer: &str) -> Result<Vec<u8>> {
    let url = reqwest::Url::parse(link).map_err(|_| anyhow!("bad link"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        bail!("not an http(s) link");
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let sources = lodestarr::indexers().await;
    let source = sources
        .iter()
        .find(|s| s.id == indexer)
        .ok_or_else(|| anyhow!("unknown source {indexer}"))?;
    if !source.hosts().iter().any(|h| *h == host) {
        bail!("{host} is not a site of {}", source.name);
    }
    let base = lodestarr::base_url().ok_or_else(|| anyhow!("no Lodestarr URL"))?;
    let proxy = format!(
        "{base}/api/v2.0/indexers/{}/dl?link={}",
        urlencoding::encode(&source.id),
        urlencoding::encode(link)
    );
    let response = CLIENT.get(proxy).timeout(Duration::from_secs(30)).send().await?;
    read_body(response).await
}

async fn resolve_magnet(magnet: &str) -> Result<Vec<u8>> {
    let url = format!("{}/torrents/resolve_magnet", CONFIG.rqbit_url.trim_end_matches('/'));
    let response = CLIENT
        .post(url)
        .body(magnet.to_string())
        .timeout(Duration::from_secs(40))
        .send()
        .await
        .map_err(|err| {
            if err.is_timeout() {
                anyhow!("no peers sent the metadata within 40 s")
            } else {
                anyhow!("rqbit: {err}")
            }
        })?;
    let bytes = read_body(response).await?;
    if bytes.first() != Some(&b'd') {
        bail!("rqbit did not return a torrent file");
    }
    Ok(bytes)
}

async fn read_body(response: reqwest::Response) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        bail!("{status} {}", text.chars().take(160).collect::<String>());
    }
    if response.content_length().unwrap_or(0) as usize > MAX_TORRENT_BYTES {
        bail!("torrent file too large");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_TORRENT_BYTES {
        bail!("torrent file too large");
    }
    Ok(bytes.to_vec())
}

// ---------------------------------------------------------------- bencode

enum Bencode<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<Bencode<'a>>),
    Dict(Vec<(&'a [u8], Bencode<'a>)>),
}

impl<'a> Bencode<'a> {
    fn get(&self, key: &str) -> Option<&Bencode<'a>> {
        match self {
            Bencode::Dict(entries) => entries.iter().find(|(k, _)| *k == key.as_bytes()).map(|(_, v)| v),
            _ => None,
        }
    }
    fn int(&self) -> Option<i64> {
        match self {
            Bencode::Int(i) => Some(*i),
            _ => None,
        }
    }
    fn text(&self) -> Option<String> {
        match self {
            Bencode::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
    fn list(&self) -> Option<&[Bencode<'a>]> {
        match self {
            Bencode::List(items) => Some(items),
            _ => None,
        }
    }
}

fn decode(input: &[u8]) -> Result<Bencode<'_>> {
    let mut pos = 0;
    let value = decode_at(input, &mut pos, 0)?;
    Ok(value)
}

fn decode_at<'a>(b: &'a [u8], pos: &mut usize, depth: usize) -> Result<Bencode<'a>> {
    if depth > 64 {
        bail!("bencode nested too deeply");
    }
    match b.get(*pos) {
        Some(b'i') => {
            let end = find(b, *pos + 1, b'e')?;
            let n = std::str::from_utf8(&b[*pos + 1..end])?.parse()?;
            *pos = end + 1;
            Ok(Bencode::Int(n))
        }
        Some(b'l') => {
            *pos += 1;
            let mut items = Vec::new();
            while b.get(*pos) != Some(&b'e') {
                items.push(decode_at(b, pos, depth + 1)?);
            }
            *pos += 1;
            Ok(Bencode::List(items))
        }
        Some(b'd') => {
            *pos += 1;
            let mut entries = Vec::new();
            while b.get(*pos) != Some(&b'e') {
                let key = match decode_at(b, pos, depth + 1)? {
                    Bencode::Bytes(k) => k,
                    _ => bail!("bencode dict key is not a string"),
                };
                let value = decode_at(b, pos, depth + 1)?;
                entries.push((key, value));
            }
            *pos += 1;
            Ok(Bencode::Dict(entries))
        }
        Some(c) if c.is_ascii_digit() => {
            let colon = find(b, *pos, b':')?;
            let len: usize = std::str::from_utf8(&b[*pos..colon])?.parse()?;
            let start = colon + 1;
            let end = start.checked_add(len).filter(|e| *e <= b.len()).ok_or_else(|| anyhow!("bencode string runs past the end"))?;
            *pos = end;
            Ok(Bencode::Bytes(&b[start..end]))
        }
        _ => bail!("invalid bencode at byte {}", *pos),
    }
}

fn find(b: &[u8], from: usize, byte: u8) -> Result<usize> {
    b.get(from..)
        .and_then(|rest| rest.iter().position(|c| *c == byte))
        .map(|i| from + i)
        .ok_or_else(|| anyhow!("truncated bencode"))
}

// ---------------------------------------------------------------- summary

fn parse(bytes: &[u8], via: &'static str) -> Result<FileInfo> {
    let root = decode(bytes)?;
    let info = root.get("info").ok_or_else(|| anyhow!("torrent has no info dictionary"))?;
    let name = info
        .get("name.utf-8")
        .or_else(|| info.get("name"))
        .and_then(Bencode::text)
        .unwrap_or_default();
    let mut files: Vec<(String, u64)> = Vec::new();
    if let Some(list) = info.get("files").and_then(Bencode::list) {
        for f in list {
            let size = f.get("length").and_then(Bencode::int).unwrap_or(0).max(0) as u64;
            let path = f
                .get("path.utf-8")
                .or_else(|| f.get("path"))
                .and_then(Bencode::list)
                .map(|parts| parts.iter().filter_map(Bencode::text).collect::<Vec<_>>().join("/"))
                .unwrap_or_default();
            // BEP 47 padding files are not real content.
            let is_pad = f.get("attr").and_then(Bencode::text).map_or(false, |a| a.contains('p'))
                || path.starts_with(".pad/");
            if !is_pad {
                files.push((path, size));
            }
        }
    } else if let Some(size) = info.get("length").and_then(Bencode::int) {
        files.push((name.clone(), size.max(0) as u64));
    } else {
        bail!("unsupported torrent layout (no file list)");
    }
    Ok(summarize(name, files, via))
}

fn is_extra_name(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    let ext = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(ext, "nfo" | "txt" | "sfv" | "md5" | "sha1" | "url" | "lnk" | "jpg" | "jpeg" | "png" | "gif" | "webp" | "db")
        || file.contains("sample")
        || lower.starts_with("sample/")
        || lower.contains("/sample/")
}

fn summarize(name: String, files: Vec<(String, u64)>, via: &'static str) -> FileInfo {
    let total: u64 = files.iter().map(|(_, s)| *s).sum();
    let tiny = total / 100; // under 1% of the whole torrent
    let mut entries: Vec<FileEntry> = files
        .into_iter()
        .map(|(path, size)| {
            let extra = size < tiny || is_extra_name(&path);
            FileEntry { path, size, extra }
        })
        .collect();
    // If every file looks like an extra (e.g. a single small file), none is.
    if entries.iter().all(|e| e.extra) {
        entries.iter_mut().for_each(|e| e.extra = false);
    }
    entries.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    let main: Vec<&FileEntry> = entries.iter().filter(|e| !e.extra).collect();
    let main_count = main.len();
    let main_total = main.iter().map(|e| e.size).sum();
    let extras_count = entries.len() - main_count;
    let extras_total = total - main_total;
    let largest = entries.first().cloned();
    let count = entries.len();
    entries.truncate(MAX_LISTED);
    FileInfo {
        name,
        count,
        total,
        main_count,
        main_total,
        extras_count,
        extras_total,
        largest,
        files: entries,
        via,
    }
}
