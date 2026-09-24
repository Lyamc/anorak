use crate::models::SendToTransmission;
use crate::{app_error::AppError, config::CONFIG};

use anyhow::{anyhow, Result};
use axum::response::{Html, IntoResponse};
use axum::{debug_handler, Form};
use urlencoding::decode;

#[debug_handler]
pub async fn endpoint(Form(payload): Form<SendToTransmission>) -> Result<impl IntoResponse, AppError> {
    let decoded = decode(&payload.magnet)?.into_owned();
    let category = payload
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u32>().ok());
    send_rqbit(decoded, category).await?;
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
