use crate::models::SendToTransmission;
use crate::{app_error::AppError, config::CONFIG};

use anyhow::{anyhow, Result};
use axum::response::{Html, IntoResponse};
use axum::{debug_handler, Form};
use urlencoding::decode;

#[debug_handler]
pub async fn endpoint(Form(magnet): Form<SendToTransmission>) -> Result<impl IntoResponse, AppError> {
    let decoded = decode(&magnet.magnet)?.into_owned();
    send_rqbit(decoded).await?;
    Ok(Html("<span class=\"fa-solid fa-check\"></span>"))
}

async fn send_rqbit(link: String) -> Result<String> {
    let url = format!("{}/torrents", CONFIG.rqbit_url.trim_end_matches('/'));
    let response = reqwest::Client::new().post(url).body(link).send().await?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(text)
    } else {
        Err(anyhow!("rqbit returned {status}: {text}"))
    }
}
