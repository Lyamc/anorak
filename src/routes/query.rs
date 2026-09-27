use crate::app_error::AppError;
use crate::config::CONFIG;
use crate::lodestarr;
use crate::models;
use crate::utils;
use crate::ENV;

use anyhow::Result;
use axum::response::Html;
use axum::Form;
use axum::{debug_handler, response::IntoResponse};
use log::{info, warn};
use minijinja::{context, Value};
use serde_xml_rs::from_str;

#[debug_handler]
pub async fn endpoint(Form(payload): Form<models::Query>) -> Result<impl IntoResponse, AppError> {
    info!("{} (sources: {})", &payload.search_term, display_sources(&payload.indexers));
    let filter = SearchFilter::from_query(&payload);
    let gathered = gather_items(&payload.search_term, &filter).await?;
    let items: Value = gathered
        .items
        .iter()
        .map(|it| {
            let display = it.display_category();
            let category_name = display.map(models::category_name).unwrap_or_default();
            let category_title = match (display, it.category_inferred) {
                (Some(_), true) => format!(
                    "{category_name} (inferred: {} only lists this category)",
                    it.sources.join(", ")
                ),
                (Some(id), false) => format!("{category_name} ({id})"),
                (None, _) => String::new(),
            };
            let magnet = it.magnet_link();
            context! {
                already_added => gathered.is_known(it),
                is_magnet => magnet.to_ascii_lowercase().starts_with("magnet:"),
                title => it.title,
                guid => it.guid,
                magnet => magnet,
                torrent => it.torrent_url(),
                category => it.grab_category(),
                category_name => category_name,
                category_title => category_title,
                category_group => models::category_group(display),
                category_inferred => it.category_inferred,
                sources => it.sources.join(", "),
                source_ids => it.source_ids.join(" "),
                seeders => it.seeders,
                peers => it.peers,
                pub_date => utils::format_date_unix(&it.pub_date),
                pub_date_format => utils::format_date(&it.pub_date),
                size => it.size,
                size_format => utils::format_bytes(it.size),
            }
        })
        .collect::<Vec<_>>()
        .into();
    let client_name = crate::client::name().await;
    let tmpl = ENV.get_template("query.html")?;
    let result = Html(tmpl.render(context!(
        items => items,
        failed_sources => gathered.failed_sources,
        client_name => client_name,
    ))?);
    Ok(result)
}

fn display_sources(indexers: &str) -> &str {
    if indexers.trim().is_empty() {
        "all"
    } else {
        indexers
    }
}

/// What to search: which sources (None = every enabled one) and which
/// category group to keep (None = all).
pub(crate) struct SearchFilter {
    pub indexers: Option<Vec<String>>,
    pub cat: Option<String>,
}

impl SearchFilter {
    pub fn from_query(query: &models::Query) -> Self {
        let ids: Vec<String> = query
            .indexers
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let cat = query.cat.trim().to_ascii_lowercase();
        SearchFilter {
            indexers: (!ids.is_empty()).then_some(ids),
            cat: (!cat.is_empty() && cat != "all").then_some(cat),
        }
    }
}

/// Search results in the default order (seeders desc, then peers desc), plus
/// the names rqbit already knows about. Shared by the HTML fragment
/// (`POST /query/`) and the JSON API (`/api/query`).
pub(crate) struct Gathered {
    pub items: Vec<models::Item>,
    pub known_torrents: Vec<String>,
    /// Results came from Lodestarr's native per-source search (source-tagged).
    pub native: bool,
    pub failed_sources: Vec<String>,
}

impl Gathered {
    pub fn is_known(&self, item: &models::Item) -> bool {
        self.known_torrents.iter().any(|t| t.contains(&item.title))
    }
}

pub(crate) async fn gather_items(search_query: &str, filter: &SearchFilter) -> Result<Gathered> {
    let enabled: Vec<lodestarr::Indexer> = lodestarr::indexers()
        .await
        .into_iter()
        .filter(|i| i.enabled)
        .collect();

    let mut native = false;
    let mut failed_sources = Vec::new();
    let mut items: Option<Vec<models::Item>> = None;
    if !enabled.is_empty() {
        let chosen: Vec<lodestarr::Indexer> = match &filter.indexers {
            None => enabled,
            Some(ids) => enabled.into_iter().filter(|i| ids.contains(&i.id)).collect(),
        };
        let (found, failed) = lodestarr::search(search_query, &chosen).await;
        // If every source failed on an unfiltered search, the native API is
        // probably broken: fall back to the Torznab feed rather than show
        // nothing. A hand-picked subset is honoured (and its failures shown)
        // because the Torznab feed would bring back unchecked sources.
        if chosen.is_empty() || failed.len() < chosen.len() || filter.indexers.is_some() {
            native = true;
            failed_sources = failed;
            items = Some(found);
        } else {
            warn!("every source failed in the native search; falling back to Torznab");
        }
    }
    let mut items = match items {
        Some(items) => items,
        None => {
            let contents = query_jackett(search_query).await?;
            process_xml(&contents).unwrap_or_default()
        }
    };

    if let Some(cat) = &filter.cat {
        items.retain(|it| models::category_group(it.display_category()) == cat.as_str());
    }

    let known_torrents = request_rqbit_known_torrents().await;

    // Default sort: highest seeders first, then peers.
    items.sort_by(|a, b| {
        b.seeders
            .cmp(&a.seeders)
            .then_with(|| b.peers.cmp(&a.peers))
    });

    Ok(Gathered {
        items,
        known_torrents,
        native,
        failed_sources,
    })
}

async fn request_rqbit_known_torrents() -> Vec<String> {
    let url = format!("{}/torrents", CONFIG.rqbit_url.trim_end_matches('/'));
    let response = match reqwest::get(&url).await {
        Ok(response) => response,
        Err(err) => {
            warn!("rqbit torrent list failed: {err}");
            return Vec::new();
        }
    };
    if !response.status().is_success() {
        warn!("rqbit torrent list returned {}", response.status());
        return Vec::new();
    }
    let body = match response.text().await {
        Ok(body) => body,
        Err(err) => {
            warn!("rqbit torrent list body failed: {err}");
            return Vec::new();
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&body) {
        Ok(value) => value,
        Err(err) => {
            warn!("rqbit torrent list was not JSON: {err}");
            return Vec::new();
        }
    };
    let items = value
        .as_array()
        .cloned()
        .or_else(|| value.get("torrents").and_then(|t| t.as_array()).cloned())
        .unwrap_or_default();
    items
        .into_iter()
        .filter_map(|item| {
            item.get("name")
                .and_then(|name| name.as_str())
                .map(str::to_string)
        })
        .collect()
}

fn format_query_url(search_query: &str) -> String {
    // JACKETT_URL is the Torznab results path. Lodestarr accepts that path
    // with `/api` appended, which is what this app has always requested.
    format!(
        "{}/api?apikey={}&t=search&q={}",
        CONFIG.jackett_url.trim_end_matches('/'),
        urlencoding::encode(&CONFIG.jackett_apikey),
        urlencoding::encode(search_query)
    )
}

async fn query_jackett(search_query: &str) -> Result<String> {
    let response = reqwest::get(format_query_url(search_query)).await?;
    let body = response.text().await?;
    Ok(body)
}

/// serde-xml-rs rejects repeated `<torznab:attr .../>` as duplicate field `attr`.
/// Lift seeders/peers/magneturl into real elements and drop the rest before parsing.
fn normalize_torznab_xml(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<torznab:attr ") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let end = match after.find("/>") {
            Some(i) => i + 2,
            None => {
                out.push_str(after);
                return out;
            }
        };
        let tag = &after[..end];
        if let (Some(name), Some(value)) = (xml_attr(tag, "name"), xml_attr(tag, "value")) {
            if name == "seeders" || name == "peers" || name == "magneturl" {
                out.push('<');
                out.push_str(name);
                out.push('>');
                out.push_str(value);
                out.push('<');
                out.push('/');
                out.push_str(name);
                out.push('>');
            } else if name == "category" {
                // Supplement <category> elements from torznab:attr.
                out.push_str("<category>");
                out.push_str(value);
                out.push_str("</category>");
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn xml_attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("{key}=\"");
    let start = tag.find(&pattern)? + pattern.len();
    let end = tag[start..].find('"')? + start;
    Some(&tag[start..end])
}

fn process_xml(xml: &str) -> Result<Vec<models::Item>> {
    let normalized = normalize_torznab_xml(xml.trim());
    match from_str::<models::Rss>(&normalized) {
        Ok(rss) => Ok(rss.channel.item),
        Err(err) => {
            warn!("Torznab XML parse failed: {err}");
            Err(err.into())
        }
    }
}
