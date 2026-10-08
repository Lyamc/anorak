//! JSON API for non-browser clients (e.g. the `anorak-gpui` desktop app).
//!
//! * `GET /api/query?search_term=...` or `POST /api/query` (form body) returns
//!   the same items, in the same default order, as the HTML fragment served by
//!   `POST /query/`. Optional `indexers=id1,id2` (sources to search; default
//!   all) and `cat=<group>` (movies, tv, anime, music, books, games, software,
//!   xxx, other, unknown) work for both endpoints.
//! * `GET /api/indexers` lists the searchable sources.
//! * `GET /api/files?magnet=...&torrent=...&indexer=...` returns one result's
//!   file list (on demand; see `torrent_files`).

use crate::app_error::AppError;
use crate::client;
use crate::lodestarr;
use crate::models;
use crate::routes::query::{gather_items, SearchFilter};
use crate::torrent_files;
use crate::utils;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{debug_handler, Form, Json};
use chrono::DateTime;
use log::info;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ApiResponse {
    pub search_term: String,
    pub count: usize,
    /// Results came from Lodestarr's per-source search (so `sources` is set).
    pub native: bool,
    /// Sources that failed during this search.
    pub failed_sources: Vec<String>,
    pub items: Vec<ApiItem>,
}

#[derive(Serialize)]
pub struct ApiItem {
    pub title: String,
    pub guid: String,
    /// Magnet (or best available link); empty when the indexer gave none.
    pub magnet: String,
    /// Direct .torrent URL when known ("" otherwise).
    pub torrent: String,
    /// Torznab category id to pass back unchanged as `category` to
    /// `POST /send-to-rqbit/` ("" when unknown on the Torznab path).
    pub category: String,
    /// Display category: Torznab id, name and filter group ("unknown" if none).
    pub category_id: Option<u32>,
    pub category_name: String,
    pub category_group: String,
    /// The category was inferred from a single-category source.
    pub category_inferred: bool,
    /// Site category (Nyaa/sukebei): full name, short badge text, site
    /// ("nyaa" | "sukebei") and the site's id, e.g. "1_2" ("" when none).
    pub category_label: String,
    pub category_short: String,
    pub category_source: String,
    pub category_site_id: String,
    /// Source (indexer) names and ids; empty on the Torznab fallback path.
    pub sources: Vec<String>,
    pub source_ids: Vec<String>,
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
    respond(payload).await
}

#[debug_handler]
pub async fn query_post(Form(payload): Form<models::Query>) -> Result<Json<ApiResponse>, AppError> {
    respond(payload).await
}

async fn respond(payload: models::Query) -> Result<Json<ApiResponse>, AppError> {
    info!("api: {} (sources: {:?}, cat: {:?})", &payload.search_term, payload.indexers, payload.cat);
    let filter = SearchFilter::from_query(&payload);
    let gathered = gather_items(&payload.search_term, &filter).await?;
    let items: Vec<ApiItem> = gathered
        .items
        .iter()
        .map(|it| {
            let display = it.display_category();
            ApiItem {
                already_added: gathered.is_known(it),
                title: it.title.clone(),
                guid: it.guid.clone(),
                magnet: it.magnet_link(),
                torrent: it.torrent_url(),
                category: it.grab_category(),
                category_id: display,
                category_name: display.map(models::category_name).unwrap_or_default(),
                category_group: models::category_group(display).to_string(),
                category_inferred: it.category_inferred,
                category_label: it.category_label.clone(),
                category_short: if it.category_label.is_empty() {
                    String::new()
                } else {
                    crate::nyaa::short_name(&it.category_source, &it.category_site_id, &it.category_label)
                },
                category_source: it.category_source.clone(),
                category_site_id: it.category_site_id.clone(),
                sources: it.sources.clone(),
                source_ids: it.source_ids.clone(),
                seeders: it.seeders,
                peers: it.peers,
                size: it.size,
                size_format: utils::format_bytes(it.size),
                date: DateTime::parse_from_str(&it.pub_date, "%a, %d %b %Y %H:%M:%S %z")
                    .ok()
                    .map(|d| d.timestamp()),
                date_format: utils::format_date(&it.pub_date),
            }
        })
        .collect();
    Ok(Json(ApiResponse {
        search_term: payload.search_term,
        count: items.len(),
        native: gathered.native,
        failed_sources: gathered.failed_sources,
        items,
    }))
}

#[derive(Serialize)]
pub struct IndexersResponse {
    /// False when the backend can't list sources (e.g. plain Torznab/Jackett);
    /// then every search covers everything and `indexers` is empty.
    pub available: bool,
    pub indexers: Vec<lodestarr::Indexer>,
    pub category_groups: Vec<CategoryGroup>,
}

#[derive(Serialize)]
pub struct CategoryGroup {
    pub id: &'static str,
    pub name: &'static str,
}

#[debug_handler]
pub async fn indexers() -> Json<IndexersResponse> {
    let list: Vec<lodestarr::Indexer> = lodestarr::indexers()
        .await
        .into_iter()
        .filter(|i| i.enabled)
        .collect();
    Json(IndexersResponse {
        available: !list.is_empty(),
        indexers: list,
        category_groups: models::CATEGORY_GROUPS
            .iter()
            .map(|(id, name)| CategoryGroup { id, name })
            .collect(),
    })
}

#[derive(Deserialize)]
pub struct FilesQuery {
    #[serde(default)]
    pub magnet: String,
    #[serde(default)]
    pub torrent: String,
    #[serde(default)]
    pub indexer: String,
}

#[debug_handler]
pub async fn files(Query(q): Query<FilesQuery>) -> Response {
    match torrent_files::lookup(&q.magnet, &q.torrent, &q.indexer).await {
        Ok(info) => Json(info).into_response(),
        Err(err) => {
            log::warn!("file list failed ({}): {err}", q.indexer);
            let body = Json(serde_json::json!({ "error": err.to_string() }));
            (StatusCode::BAD_GATEWAY, body).into_response()
        }
    }
}

/// `GET /api/client`: the torrent client results are sent to.
#[debug_handler]
pub async fn client() -> Json<client::ClientInfo> {
    Json(client::info().await)
}
