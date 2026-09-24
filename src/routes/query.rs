use crate::app_error::AppError;
use crate::config::CONFIG;
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
    info!("{}", &payload.search_term);
    let items = gather_items_json(&payload.search_term).await?;
    let tmpl = ENV.get_template("query.html")?;
    let result = Html(tmpl.render(context!(items => items))?);
    Ok(result)
}

async fn gather_items_json(search_query: &str) -> Result<Value> {
    let contents = query_jackett(search_query).await?;
    let mut items: Vec<models::Item> = process_xml(&contents).unwrap_or_default();
    let known_torrents = request_rqbit_known_torrents().await;

    // Default sort: highest seeders first, then peers.
    items.sort_by(|a, b| {
        b.seeders
            .cmp(&a.seeders)
            .then_with(|| b.peers.cmp(&a.peers))
    });

    let contexts: Value = items
        .iter()
        .map(|it| {
            let magnet = it.magnet_link();
            context! {
                already_added => known_torrents.iter().any(|t| t.contains(&it.title)),
                title => it.title,
                guid => it.guid,
                magnet => magnet,
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
    Ok(contexts)
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
