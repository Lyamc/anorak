use crate::models::SendToTransmission;
use crate::torrent_files;
use crate::{app_error::AppError, config::CONFIG};

use anyhow::{anyhow, Result};
use log::{info, warn};
use axum::response::{Html, IntoResponse};
use axum::{debug_handler, Form};
use urlencoding::decode;

#[debug_handler]
pub async fn endpoint(Form(payload): Form<SendToTransmission>) -> Result<impl IntoResponse, AppError> {
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
    let category = payload
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok());
    let link = match payload.indexer.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(indexer) if torrent_files::is_page_link(&decoded) => {
            // A details page (0Magnet) is not something the client can add;
            // send the magnet printed on it. A .torrent is left as is.
            match torrent_files::magnet_for_link(&decoded, indexer).await {
                Ok(Some(magnet)) => {
                    info!("sending the magnet from {decoded}");
                    magnet
                }
                Ok(None) => decoded,
                Err(err) => {
                    warn!("no magnet from {decoded}: {err}");
                    decoded
                }
            }
        }
        _ => decoded,
    };
    send_rqbit(link, category).await?;
    Ok(Html("<span class=\"fa-solid fa-check\"></span>"))
}

async fn send_rqbit(link: String, torznab_category: Option<u32>) -> Result<String> {
    let base = format!("{}/torrents", CONFIG.rqbit_url.trim_end_matches('/'));
    let url = match torznab_category {
        Some(id) => {
            let sep = if base.contains('?') { '&' } else { '?' };
            format!("{base}{sep}torznab_category={id}")
        }
        None => base,
    };
    let response = reqwest::Client::new().post(url).body(link).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(text)
    } else {
        Err(anyhow!("rqbit returned {status}: {text}"))
    }
}
