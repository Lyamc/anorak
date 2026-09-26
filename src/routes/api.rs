//! JSON API for non-browser clients (e.g. the `anorak-gpui` desktop app).
//!
//! `GET /api/query?search_term=...` or `POST /api/query` (form body
//! `search_term=...`) returns the same items, in the same default order, as
//! the HTML fragment served by `POST /query/`.

use crate::app_error::AppError;
use crate::models;
use crate::routes::query::gather_items;
use crate::utils;

use axum::extract::Query;
use axum::{debug_handler, Form, Json};
use chrono::DateTime;
use log::info;
use serde::Serialize;

#[derive(Serialize)]
pub struct ApiResponse {
    pub search_term: String,
    pub count: usize,
    pub items: Vec<ApiItem>,
}

#[derive(Serialize)]
pub struct ApiItem {
    pub title: String,
    pub guid: String,
    /// Magnet (or best available link); empty when the indexer gave none.
    pub magnet: String,
    /// Preferred Torznab category id as a string ("" when unknown); pass it
    /// back unchanged as `category` to `POST /send-to-rqbit/`.
    pub category: String,
    pub seeders: u32,
    pub peers: u32,
    /// Size in bytes.
    pub size: u64,
    /// Human readable size, same text as the web UI.
    pub size_format: String,
    /// Publish date as Unix seconds, or null if the indexer's date was unparseable.
    pub date: Option<i64>,
    /// Display date, same text as the web UI.
    pub date_format: String,
    /// rqbit already has a torrent whose name contains this title.
    pub already_added: bool,
}

#[debug_handler]
pub async fn query_get(Query(payload): Query<models::Query>) -> Result<Json<ApiResponse>, AppError> {
    respond(payload.search_term).await
}

#[debug_handler]
pub async fn query_post(Form(payload): Form<models::Query>) -> Result<Json<ApiResponse>, AppError> {
    respond(payload.search_term).await
}

async fn respond(search_term: String) -> Result<Json<ApiResponse>, AppError> {
    info!("api: {}", &search_term);
    let gathered = gather_items(&search_term).await?;
    let items: Vec<ApiItem> = gathered
        .items
        .iter()
        .map(|it| ApiItem {
            already_added: gathered.is_known(it),
            title: it.title.clone(),
            guid: it.guid.clone(),
            magnet: it.magnet_link(),
            category: models::prefer_torznab_category(&it.category)
                .map(|c| c.to_string())
                .unwrap_or_default(),
            seeders: it.seeders,
            peers: it.peers,
            size: it.size,
            size_format: utils::format_bytes(it.size),
            date: DateTime::parse_from_str(&it.pub_date, "%a, %d %b %Y %H:%M:%S %z")
                .ok()
                .map(|d| d.timestamp()),
            date_format: utils::format_date(&it.pub_date),
        })
        .collect();
    Ok(Json(ApiResponse {
        search_term,
        count: items.len(),
        items,
    }))
}
