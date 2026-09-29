//! 1337x through FlareSolverr.
//!
//! Every 1337x mirror sits behind a Cloudflare browser check that Lodestarr
//! (plain HTTP) can't pass, so with `FLARESOLVERR_URL` set anorak searches
//! 1337x itself: the search page is loaded by FlareSolverr's browser and the
//! result table parsed here. Search pages carry no magnet; results link to
//! their details page, and the magnet on that page is fetched (again through
//! FlareSolverr) only when a result is sent or its files are listed.

use crate::flaresolverr::{self, failure};
use crate::lodestarr::Indexer;
use crate::models::Item;

use anyhow::Result;
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use log::{info, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const ID: &str = "1337x";
/// 20 results a page; the second page is only asked for when the first is full.
const PAGES: u32 = 2;
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<Item>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// Index into the mirror list of the last mirror that worked.
static MIRROR: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(0));

/// anorak searches this source itself (instead of asking Lodestarr).
pub fn handles(indexer_id: &str) -> bool {
    indexer_id == ID && flaresolverr::enabled()
}

fn mirrors(source: &Indexer) -> Vec<String> {
    let from_env: Vec<String> = std::env::var("X1337X_MIRRORS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| s.starts_with("http"))
        .collect();
    if !from_env.is_empty() {
        return from_env;
    }
    let mut list: Vec<String> = source
        .links
        .iter()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| s.starts_with("http"))
        .collect();
    if list.is_empty() {
        list = vec!["https://1337x.to".into(), "https://1337x.st".into(), "https://x1337x.ws".into()];
    }
    list
}

pub async fn search(query: &str, source: &Indexer) -> Result<Vec<Item>> {
    let query = query.trim();
    // 1337x refuses searches shorter than 3 characters.
    if query.chars().count() < 3 {
        return Ok(Vec::new());
    }
    let key = query.to_lowercase();
    if let Some((at, items)) = CACHE.lock().await.get(&key) {
        if at.elapsed() < CACHE_TTL {
            return Ok(items.clone());
        }
    }
    let list = mirrors(source);
    let start = *MIRROR.lock().await % list.len();
    let started = Instant::now();
    let mut last_err = None;
    // Try at most two mirrors so a bad day can't hold the search up for long.
    for step in 0..list.len().min(2) {
        let index = (start + step) % list.len();
        let base = &list[index];
        match search_mirror(base, query, source).await {
            Ok(items) => {
                *MIRROR.lock().await = index;
                info!("1337x: {} results for {query:?} from {base} in {} ms", items.len(), started.elapsed().as_millis());
                let mut cache = CACHE.lock().await;
                if cache.len() > 200 {
                    cache.clear();
                }
                cache.insert(key, (Instant::now(), items.clone()));
                return Ok(items);
            }
            Err(err) => {
                warn!("1337x: {base} failed: {err:#}");
                // FlareSolverr itself being down is the same on every mirror.
                let down = err.downcast_ref::<flaresolverr::Failure>().is_some_and(|f| f.0.contains("not running"));
                last_err = Some(err);
                if down {
                    break;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| failure("no mirror answered")))
}

async fn search_mirror(base: &str, query: &str, source: &Indexer) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for page in 1..=PAGES {
        let url = format!("{base}/sort-search/{}/seeders/desc/{page}/", urlencoding::encode(query));
        let fetched = flaresolverr::get(&url).await?;
        let parsed = parse_search(&fetched, base, source)?;
        let full = parsed.len() >= 20;
        items.extend(parsed);
        if !full || !fetched.html.contains(&format!("/seeders/desc/{}/", page + 1)) {
            break;
        }
    }
    Ok(items)
}

/// Details page (for the magnet on it).
pub async fn fetch_page(url: &str) -> Result<Vec<u8>> {
    let page = flaresolverr::get(url).await?;
    if flaresolverr::is_cloudflare_page(page.status, &page.title, &page.html) {
        return Err(failure("blocked by Cloudflare"));
    }
    if page.status >= 400 {
        return Err(failure(format!("1337x answered {}", page.status)));
    }
    Ok(page.html.into_bytes())
}

fn parse_search(page: &flaresolverr::Page, base: &str, source: &Indexer) -> Result<Vec<Item>> {
    let html = &page.html;
    if flaresolverr::is_cloudflare_page(page.status, &page.title, html) {
        return Err(failure("blocked by Cloudflare"));
    }
    if html.contains("No results were returned") {
        return Ok(Vec::new());
    }
    if page.status >= 400 {
        return Err(failure(format!("site answered {}", page.status)));
    }
    let Some(body) = between(html, "<tbody>", "</tbody>") else {
        if page.title.to_ascii_lowercase().contains("torrents") {
            return Ok(Vec::new());
        }
        return Err(failure(format!("unexpected page \"{}\"", page.title.chars().take(60).collect::<String>())));
    };
    Ok(body.split("<tr").skip(1).filter_map(|row| parse_row(row, base, source)).collect())
}

fn parse_row(row: &str, base: &str, source: &Indexer) -> Option<Item> {
    let at = row.find("href=\"/torrent/")?;
    let rest = &row[at + 6..];
    let href = &rest[..rest.find('"')?];
    let title = decode_entities(between(rest, ">", "</a>")?.trim());
    if title.is_empty() {
        return None;
    }
    let link = format!("{base}{href}");
    let sub: Option<u32> = between(row, "href=\"/sub/", "/").and_then(|s| s.parse().ok());
    let number = |class: &str| -> u32 {
        between(row, &format!("class=\"{class}\">"), "<")
            .and_then(|s| s.trim().replace(',', "").parse().ok())
            .unwrap_or(0)
    };
    let size = between(row, "class=\"coll-4 size", "<span")
        .and_then(|s| s.split('>').nth(1))
        .map(parse_size)
        .unwrap_or(0);
    let date = between(row, "class=\"coll-date\">", "<").map(parse_date).unwrap_or_default();
    Some(Item {
        title,
        guid: link.clone(),
        link: link.clone(),
        comments: link,
        pub_date: date,
        size,
        files: 0,
        description: String::new(),
        category: sub.and_then(torznab_category).map(|c| vec![c.to_string()]).unwrap_or_default(),
        seeders: number("coll-2 seeds"),
        peers: number("coll-3 leeches"),
        magneturl: String::new(),
        sources: vec![source.name.clone()],
        source_ids: vec![source.id.clone()],
        category_inferred: false,
    })
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)? + i;
    Some(&s[i..j])
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest[..rest.len().min(10)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// "1.6 GB" -> bytes (1337x uses binary units).
fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let mut parts = s.split_whitespace();
    let (Some(num), Some(unit)) = (parts.next(), parts.next()) else { return 0 };
    let Ok(num) = num.replace(',', "").parse::<f64>() else { return 0 };
    let mult: f64 = match unit.to_ascii_uppercase().as_str() {
        "B" => 1.0,
        "KB" | "KIB" => 1024.0,
        "MB" | "MIB" => 1024.0f64.powi(2),
        "GB" | "GIB" => 1024.0f64.powi(3),
        "TB" | "TIB" => 1024.0f64.powi(4),
        _ => return 0,
    };
    (num * mult) as u64
}

/// 1337x dates: "Jan. 10th '26", "Sep. 27th" (this year), "10:39am" (today).
/// Returned as an RSS date; empty when unrecognised.
fn parse_date(s: &str) -> String {
    let s = s.trim();
    let now = Utc::now();
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let lower = s.to_ascii_lowercase();
    let month = months.iter().position(|m| lower.starts_with(m)).map(|i| i as u32 + 1);
    let date = if let Some(month) = month {
        let mut words = lower.split_whitespace().skip(1);
        let day: Option<u32> = words.next().map(|d| d.trim_end_matches(|c: char| c.is_ascii_alphabetic())).and_then(|d| d.parse().ok());
        let year = match words.next() {
            Some(y) => y.trim_start_matches('\'').parse::<i32>().ok().map(|y| if y < 100 { 2000 + y } else { y }),
            None => Some(now.year()),
        };
        match (day, year) {
            (Some(day), Some(year)) => {
                let d = NaiveDate::from_ymd_opt(year, month, day);
                // "Dec. 30th" seen in January means last year.
                d.map(|d| if d > now.date_naive() + chrono::Duration::days(1) { d.with_year(year - 1).unwrap_or(d) } else { d })
            }
            _ => None,
        }
    } else if lower.ends_with("am") || lower.ends_with("pm") {
        Some(now.date_naive())
    } else {
        None
    };
    match date.and_then(|d| d.and_hms_opt(0, 0, 0)) {
        Some(dt) => Utc.from_utc_datetime(&dt).format("%a, %d %b %Y %H:%M:%S %z").to_string(),
        None => String::new(),
    }
}

/// 1337x sub-category id -> Torznab category (from Jackett's 1337x definition).
fn torznab_category(sub: u32) -> Option<u32> {
    Some(match sub {
        28 | 78 | 79 | 80 | 81 => 5070,
        22 => 3010,
        23 => 3040,
        25 => 3020,
        27 => 3050,
        24 | 26 | 53 | 58 | 59 | 60 | 68 | 69 => 3000,
        52 => 3030,
        1 => 2070,
        2 => 2030,
        4 => 2010,
        42 | 54 | 70 => 2040,
        66 => 2060,
        76 => 2045,
        3 | 55 | 73 => 2000,
        41 => 5040,
        75 => 5030,
        9 => 5080,
        5 | 6 | 7 | 71 | 74 => 5000,
        18 | 20 | 21 => 4000,
        19 => 4030,
        56 => 4070,
        57 => 4060,
        17 => 4040,
        10 => 4050,
        11 | 15 | 43 => 1080,
        12 => 1020,
        13 => 1040,
        14 => 1050,
        16 | 46 | 82 => 1090,
        44 => 1030,
        45 => 1010,
        72 => 1110,
        77 => 1180,
        48 => 6010,
        49 => 6060,
        50 | 51 | 67 => 6000,
        34 => 7000,
        36 => 7020,
        39 => 7030,
        40 => 8010,
        33 | 35 | 37 | 38 | 47 => 8000,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Indexer {
        serde_json::from_str(r#"{"id":"1337x","name":"1337x","categories":[],"enabled":true,"links":["https://1337x.to/"]}"#).unwrap()
    }

    const ROWS: &str = r#"<html><head><title>Download ubuntu Torrents | 1337x</title></head><body><table><thead><tr><th class="coll-1 name">name</th></tr></thead><tbody>
<tr>
  <td class="coll-1 name"><a href="/sub/34/0/" class="icon"><i class="flaticon-tutorial"></i></a><a href="/torrent/6563285/Udemy-Deploy-Django-on-Ubuntu/">Udemy - Deploy Django on Ubuntu - Nginx &amp; SSL</a></td>
  <td class="coll-2 seeds">9</td>
  <td class="coll-3 leeches">10</td>
  <td class="coll-date">Jan. 10th '26</td>
  <td class="coll-4 size mob-uploader">1.6 GB<span class="seeds">9</span></td>
  <td class="coll-5 uploader"><a href="/user/freecoursewb/">freecoursewb</a></td>
</tr>
<tr>
  <td class="coll-1 name"><a href="/sub/36/0/" class="icon"><i class="flaticon-ebook"></i></a><a href="/torrent/6536505/Ubuntu-Linux-Bible/">Ubuntu Linux Bible 11E (epub)</a><span class="comments"><i class="flaticon-message"></i>3</span></td>
  <td class="coll-2 seeds">1,093</td>
  <td class="coll-3 leeches">3</td>
  <td class="coll-date">Nov. 27th '25</td>
  <td class="coll-4 size mob-user">8.1 MB<span class="seeds">93</span></td>
  <td class="coll-5 user"><a href="/user/Bilbo76/">Bilbo76</a></td>
</tr>
</tbody></table></body></html>"#;

    fn page(status: u16, html: &str) -> flaresolverr::Page {
        flaresolverr::Page { status, html: html.to_string(), title: flaresolverr::page_title(html) }
    }

    #[test]
    fn parses_result_rows() {
        let items = parse_search(&page(200, ROWS), "https://1337x.to", &source()).unwrap();
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!(a.title, "Udemy - Deploy Django on Ubuntu - Nginx & SSL");
        assert_eq!(a.link, "https://1337x.to/torrent/6563285/Udemy-Deploy-Django-on-Ubuntu/");
        assert_eq!((a.seeders, a.peers), (9, 10));
        assert_eq!(a.size, (1.6 * 1024f64.powi(3)) as u64);
        assert_eq!(a.category, vec!["7000".to_string()]);
        assert_eq!(a.pub_date, "Sat, 10 Jan 2026 00:00:00 +0000");
        assert_eq!(a.source_ids, vec!["1337x".to_string()]);
        let b = &items[1];
        assert_eq!(b.seeders, 1093);
        assert_eq!(b.category, vec!["7020".to_string()]);
        assert_eq!(b.size, (8.1 * 1024f64.powi(2)) as u64);
    }

    #[test]
    fn empty_and_blocked_pages() {
        let none = page(200, "<title>Search - 1337x</title><p>No results were returned. Please refine your search.</p>");
        assert!(parse_search(&none, "https://1337x.to", &source()).unwrap().is_empty());
        let cf = page(403, "<title>Just a moment...</title><script src=\"/cdn-cgi/challenge-platform/x\"></script>");
        let err = parse_search(&cf, "https://1337x.to", &source()).unwrap_err();
        assert_eq!(err.downcast_ref::<flaresolverr::Failure>().unwrap().0, "blocked by Cloudflare");
        let busy = page(503, "<title>503 Service Unavailable</title>");
        let err = parse_search(&busy, "https://1337x.to", &source()).unwrap_err();
        assert_eq!(err.downcast_ref::<flaresolverr::Failure>().unwrap().0, "site answered 503");
    }

    #[test]
    fn sizes_dates_entities() {
        assert_eq!(parse_size("657.8 MB"), (657.8 * 1048576.0) as u64);
        assert_eq!(parse_size("2 TB"), 2 * 1024u64.pow(4));
        assert_eq!(parse_size("junk"), 0);
        assert_eq!(parse_date("May. 7th '23"), "Sun, 07 May 2023 00:00:00 +0000");
        assert_eq!(parse_date("Feb. 2nd '26"), "Mon, 02 Feb 2026 00:00:00 +0000");
        assert!(!parse_date("10:39am").is_empty());
        assert_eq!(parse_date("yesterday-ish"), "");
        assert_eq!(decode_entities("Tom &amp; Jerry &#039;99 &#x41; &bogus; &"), "Tom & Jerry '99 A &bogus; &");
    }
}
