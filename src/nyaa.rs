//! Nyaa and sukebei through their own RSS feeds (`/?page=rss&q=`).
//!
//! Lodestarr returns Nyaa/sukebei results without categories (Torznab says
//! 5000 for all, the native API an empty list), although every RSS item
//! carries `nyaa:categoryId` ("1_2") and `nyaa:category` ("Anime -
//! English-translated"), plus the info-hash, seeders, leechers and size.
//! anorak runs in the same network namespace as Lodestarr, so it asks the
//! site itself and falls back to Lodestarr if the feed can't be had.

use crate::lodestarr::Indexer;
use crate::models::Item;

use anyhow::{anyhow, Result};
use log::warn;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const TRACKERS: &[&str] = &[
    "http://nyaa.tracker.wf:7777/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
];

/// Mirrors tried per search (each has the client timeout).
const MAX_MIRRORS: usize = 3;

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<Item>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// Per source, the mirror that answered last.
static LAST_GOOD: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .expect("reqwest client")
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    Nyaa,
    Sukebei,
}

impl Site {
    pub fn from_indexer(id: &str) -> Option<Site> {
        match id {
            "nyaasi" => Some(Site::Nyaa),
            "sukebeinyaasi" => Some(Site::Sukebei),
            _ => None,
        }
    }
    /// Value for rqbit's `category_source`.
    pub fn key(self) -> &'static str {
        match self {
            Site::Nyaa => "nyaa",
            Site::Sukebei => "sukebei",
        }
    }
    fn default_links(self) -> Vec<String> {
        match self {
            Site::Nyaa => vec!["https://nyaa.si".into()],
            Site::Sukebei => vec!["https://sukebei.nyaa.si".into()],
        }
    }
}

/// anorak reads this source's RSS itself (set ANORAK_NYAA_RSS=0 to use Lodestarr).
pub fn handles(indexer_id: &str) -> bool {
    Site::from_indexer(indexer_id).is_some() && std::env::var("ANORAK_NYAA_RSS").map_or(true, |v| v != "0")
}

/// One site category: id, full name (as the RSS feed and result rows give
/// it), short badge text, Torznab id.
pub struct SiteCategory {
    pub id: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub torznab: u32,
}

const fn cat(id: &'static str, name: &'static str, short: &'static str, torznab: u32) -> SiteCategory {
    SiteCategory { id, name, short, torznab }
}

/// Nyaa categories (ids and names checked against the live site, Oct 2026).
/// `X_0` ids are parent categories, used as filters but never on a result.
pub const NYAA: &[SiteCategory] = &[
    cat("1_0", "Anime", "Anime", 5070),
    cat("1_1", "Anime - Anime Music Video", "Anime AMV", 5070),
    cat("1_2", "Anime - English-translated", "Anime EN", 5070),
    cat("1_3", "Anime - Non-English-translated", "Anime Non-EN", 5070),
    cat("1_4", "Anime - Raw", "Anime Raw", 5070),
    cat("2_0", "Audio", "Audio", 3000),
    cat("2_1", "Audio - Lossless", "Audio Lossless", 3040),
    cat("2_2", "Audio - Lossy", "Audio Lossy", 3000),
    cat("3_0", "Literature", "Literature", 7000),
    cat("3_1", "Literature - English-translated", "Lit. EN", 7000),
    cat("3_2", "Literature - Non-English-translated", "Lit. Non-EN", 7000),
    cat("3_3", "Literature - Raw", "Lit. Raw", 7000),
    cat("4_0", "Live Action", "Live Action", 5000),
    cat("4_1", "Live Action - English-translated", "Live EN", 5000),
    cat("4_2", "Live Action - Idol/Promotional Video", "Live Idol/PV", 5000),
    cat("4_3", "Live Action - Non-English-translated", "Live Non-EN", 5000),
    cat("4_4", "Live Action - Raw", "Live Raw", 5000),
    cat("5_0", "Pictures", "Pictures", 8000),
    cat("5_1", "Pictures - Graphics", "Graphics", 8000),
    cat("5_2", "Pictures - Photos", "Photos", 8000),
    cat("6_0", "Software", "Software", 4000),
    cat("6_1", "Software - Applications", "Apps", 4000),
    cat("6_2", "Software - Games", "Games", 4050),
];

/// sukebei categories; all of them are Torznab XXX.
pub const SUKEBEI: &[SiteCategory] = &[
    cat("1_0", "Art", "Art", 6000),
    cat("1_1", "Art - Anime", "Art Anime", 6000),
    cat("1_2", "Art - Doujinshi", "Doujinshi", 6000),
    cat("1_3", "Art - Games", "Art Games", 6000),
    cat("1_4", "Art - Manga", "Manga", 6000),
    cat("1_5", "Art - Pictures", "Art Pictures", 6000),
    cat("2_0", "Real Life", "Real Life", 6000),
    cat("2_1", "Real Life - Photobooks / Pictures", "RL Photos", 6000),
    cat("2_2", "Real Life - Videos", "RL Videos", 6000),
];

impl Site {
    pub fn from_key(key: &str) -> Option<Site> {
        match key {
            "nyaa" => Some(Site::Nyaa),
            "sukebei" => Some(Site::Sukebei),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Site::Nyaa => "Nyaa",
            Site::Sukebei => "sukebei",
        }
    }
    fn categories(self) -> &'static [SiteCategory] {
        match self {
            Site::Nyaa => NYAA,
            Site::Sukebei => SUKEBEI,
        }
    }
}

pub fn category(site: Site, id: &str) -> Option<&'static SiteCategory> {
    site.categories().iter().find(|c| c.id == id)
}

/// Badge text for a result's site category ("Anime EN"); the feed's own
/// name when the id isn't in the table.
pub fn short_name(source: &str, id: &str, name: &str) -> String {
    Site::from_key(source)
        .and_then(|site| category(site, id))
        .map(|c| c.short.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Tooltip for a result's site category, e.g.
/// "Anime - English-translated (Nyaa 1_2) · TV/Anime (5070)".
pub fn tooltip(source: &str, id: &str, name: &str, torznab: Option<u32>) -> String {
    let site = Site::from_key(source).map(Site::label).unwrap_or(source);
    let mut text = format!("{name} ({site} {id})");
    if let Some(t) = torznab {
        text.push_str(&format!(" · {} ({t})", crate::models::category_name(t)));
    }
    text
}

/// The query Lodestarr's Nyaa definition also sends: leading zeros dropped
/// from one-digit episode numbers ("frieren 05" -> "frieren 5"). None when
/// that changes nothing.
pub fn without_episode_zeros(query: &str) -> Option<String> {
    let chars: Vec<char> = query.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = String::with_capacity(query.len());
    let mut changed = false;
    let mut i = 0;
    while i < chars.len() {
        let starts_word = i == 0 || !word(chars.get(i - 1));
        if starts_word
            && chars[i] == '0'
            && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())
            && !word(chars.get(i + 2))
        {
            out.push(chars[i + 1]);
            i += 2;
            changed = true;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    changed.then_some(out)
}

pub async fn search(query: &str, source: &Indexer) -> Result<Vec<Item>> {
    let site = Site::from_indexer(&source.id).ok_or_else(|| anyhow!("not a Nyaa source"))?;
    let query = query.trim();
    let key = format!("{}\n{}", source.id, query.to_lowercase());
    if let Some((at, items)) = CACHE.lock().await.get(&key) {
        if at.elapsed() < CACHE_TTL {
            return Ok(items.clone());
        }
    }
    let mut items = search_once(query, site, source).await?;
    if let Some(alt) = without_episode_zeros(query) {
        match search_once(&alt, site, source).await {
            Ok(more) => {
                let seen: std::collections::HashSet<String> =
                    items.iter().filter_map(|i| crate::models::info_hash(&i.magnet_link())).collect();
                items.extend(
                    more.into_iter()
                        .filter(|i| crate::models::info_hash(&i.magnet_link()).map_or(true, |h| !seen.contains(&h))),
                );
            }
            Err(err) => warn!("{} RSS for {alt:?} failed: {err:#}", source.name),
        }
    }
    CACHE.lock().await.insert(key, (Instant::now(), items.clone()));
    Ok(items)
}

/// Mirrors in order, starting with the one that answered last.
async fn mirrors(site: Site, source: &Indexer) -> Vec<String> {
    let mut links: Vec<String> = source
        .links
        .iter()
        .map(|l| l.trim().trim_end_matches('/').to_string())
        .filter(|l| l.starts_with("http"))
        .collect();
    if links.is_empty() {
        links = site.default_links();
    }
    links.truncate(MAX_MIRRORS);
    if let Some(good) = LAST_GOOD.lock().await.get(&source.id) {
        if let Some(pos) = links.iter().position(|l| l == good) {
            let good = links.remove(pos);
            links.insert(0, good);
        }
    }
    links
}

async fn search_once(query: &str, site: Site, source: &Indexer) -> Result<Vec<Item>> {
    let mut last_err = anyhow!("no mirrors");
    for base in mirrors(site, source).await {
        let url = format!("{base}/?page=rss&f=0&c=0_0&q={}", urlencoding::encode(query));
        match fetch(&url).await {
            Ok(body) if body.contains("<rss") => {
                LAST_GOOD.lock().await.insert(source.id.clone(), base);
                return Ok(parse(&body, site, source));
            }
            Ok(_) => last_err = anyhow!("{base} did not answer with RSS"),
            Err(err) => last_err = err.context(base.clone()),
        }
        warn!("{} RSS via {base} failed: {last_err:#}", source.name);
    }
    Err(last_err)
}

async fn fetch(url: &str) -> Result<String> {
    let response = CLIENT.get(url).header(reqwest::header::USER_AGENT, UA).send().await?;
    if !response.status().is_success() {
        return Err(anyhow!("returned {}", response.status()));
    }
    Ok(response.text().await?)
}

/// Parse a Nyaa/sukebei RSS feed. Tags are matched by local name, since
/// sukebei binds the `nyaa:` prefix to its own namespace URI.
pub fn parse(body: &str, site: Site, source: &Indexer) -> Vec<Item> {
    body.split("<item>").skip(1).filter_map(|chunk| {
        let chunk = chunk.split("</item>").next()?;
        let title = tag(chunk, "title")?;
        let hash = tag(chunk, "nyaa:infoHash")?.to_ascii_lowercase();
        let page = tag(chunk, "guid").unwrap_or_default();
        let site_id = tag(chunk, "nyaa:categoryId").unwrap_or_default();
        let feed_name = tag(chunk, "nyaa:category").unwrap_or_default();
        let known = category(site, &site_id);
        let label = if feed_name.is_empty() { known.map(|k| k.name.to_string()).unwrap_or_default() } else { feed_name };
        let mut magnet = format!("magnet:?xt=urn:btih:{hash}&dn={}", urlencoding::encode(&title));
        for tr in TRACKERS {
            magnet.push_str(&format!("&tr={}", urlencoding::encode(tr)));
        }
        let num = |name: &str| tag(chunk, name).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        Some(Item {
            title,
            guid: page.clone(),
            link: magnet,
            comments: page,
            pub_date: tag(chunk, "pubDate").unwrap_or_default(),
            size: tag(chunk, "nyaa:size").map(|s| parse_size(&s)).unwrap_or(0),
            files: 0,
            description: String::new(),
            category: known.map(|k| vec![k.torznab.to_string()]).unwrap_or_default(),
            seeders: num("nyaa:seeders"),
            peers: num("nyaa:leechers"),
            magneturl: String::new(),
            sources: vec![source.name.clone()],
            source_ids: vec![source.id.clone()],
            category_inferred: false,
            category_label: label,
            category_source: site.key().to_string(),
            category_site_id: site_id,
        })
    })
    .collect()
}

fn tag(chunk: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    // Skip longer names sharing the prefix (`nyaa:category` vs `nyaa:categoryId`).
    let mut from = 0;
    let rest = loop {
        let start = chunk[from..].find(&open)? + from + open.len();
        let next = chunk[start..].chars().next()?;
        if next == '>' || next.is_whitespace() || next == '/' {
            break &chunk[start..];
        }
        from = start;
    };
    let body_start = rest.find('>')? + 1;
    let end = rest.find(&format!("</{name}>"))?;
    let raw = rest.get(body_start..end)?.trim();
    let raw = raw.strip_prefix("<![CDATA[").and_then(|r| r.strip_suffix("]]>")).unwrap_or(raw);
    Some(unescape(raw))
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// "1.4 GiB" -> bytes.
fn parse_size(s: &str) -> u64 {
    let mut parts = s.split_whitespace();
    let n: f64 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0.0);
    let mult = match parts.next().unwrap_or("") {
        "KiB" | "KB" => 1024f64,
        "MiB" | "MB" => 1024f64.powi(2),
        "GiB" | "GB" => 1024f64.powi(3),
        "TiB" | "TB" => 1024f64.powi(4),
        _ => 1.0,
    };
    (n * mult) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::category_group;

    fn src(id: &str) -> Indexer {
        serde_json::from_str(&format!(r#"{{"id":"{id}","name":"{id}","categories":[],"enabled":true,"links":[]}}"#)).unwrap()
    }

    const NYAA_FEED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<rss xmlns:atom="http://www.w3.org/2005/Atom" xmlns:nyaa="https://nyaa.si/xmlns/nyaa" version="2.0"><channel>
<title>Nyaa - "one punch" - Torrent File RSS</title>
<item><title>[Group] One Punch Man - 05 [1080p] &amp; more</title>
<link>https://nyaa.si/download/100.torrent</link>
<guid isPermaLink="true">https://nyaa.si/view/100</guid>
<pubDate>Wed, 07 Oct 2026 10:00:00 -0000</pubDate>
<nyaa:seeders>120</nyaa:seeders><nyaa:leechers>4</nyaa:leechers><nyaa:downloads>900</nyaa:downloads>
<nyaa:infoHash>0123456789abcdef0123456789abcdef01234567</nyaa:infoHash>
<nyaa:categoryId>1_2</nyaa:categoryId><nyaa:category>Anime - English-translated</nyaa:category>
<nyaa:size>1.4 GiB</nyaa:size><nyaa:comments>0</nyaa:comments><nyaa:trusted>Yes</nyaa:trusted><nyaa:remake>No</nyaa:remake>
<description><![CDATA[<a href="https://nyaa.si/view/100">#100 | x</a>]]></description></item>
<item><title><![CDATA[Ubuntu 26.04 <amd64>]]></title>
<link>https://nyaa.si/download/101.torrent</link><guid isPermaLink="true">https://nyaa.si/view/101</guid>
<pubDate>Tue, 06 Oct 2026 09:00:00 -0000</pubDate>
<nyaa:seeders>3</nyaa:seeders><nyaa:leechers>0</nyaa:leechers>
<nyaa:infoHash>FEDCBA9876543210FEDCBA9876543210FEDCBA98</nyaa:infoHash>
<nyaa:categoryId>6_1</nyaa:categoryId><nyaa:category>Software - Applications</nyaa:category>
<nyaa:size>650.0 MiB</nyaa:size></item>
<item><title>New category</title><guid>https://nyaa.si/view/102</guid><pubDate>Tue, 06 Oct 2026 09:00:00 -0000</pubDate>
<nyaa:infoHash>1111111111111111111111111111111111111111</nyaa:infoHash>
<nyaa:categoryId>7_1</nyaa:categoryId><nyaa:category>Something - New</nyaa:category><nyaa:size>10 KiB</nyaa:size></item>
<item><title>No hash</title><guid>https://nyaa.si/view/103</guid></item>
</channel></rss>"#;

    const SUKEBEI_FEED: &str = r#"<rss xmlns:atom="http://www.w3.org/2005/Atom" xmlns:nyaa="https://sukebei.nyaa.si/xmlns/nyaa" version="2.0"><channel>
<item><title>Some &amp; Title</title><link>https://sukebei.nyaa.si/download/1.torrent</link>
<guid isPermaLink="true">https://sukebei.nyaa.si/view/1</guid><pubDate>Wed, 07 Oct 2026 10:00:00 -0000</pubDate>
<nyaa:seeders>12</nyaa:seeders><nyaa:leechers>3</nyaa:leechers><nyaa:downloads>9</nyaa:downloads>
<nyaa:infoHash>ABCDEF0123456789ABCDEF0123456789ABCDEF01</nyaa:infoHash><nyaa:categoryId>2_1</nyaa:categoryId>
<nyaa:category>Real Life - Photobooks / Pictures</nyaa:category><nyaa:size>1.5 GiB</nyaa:size></item></channel></rss>"#;

    #[test]
    fn parses_nyaa_feed() {
        let items = parse(NYAA_FEED, Site::Nyaa, &src("nyaasi"));
        assert_eq!(items.len(), 3, "the item without an info-hash is skipped");

        let a = &items[0];
        assert_eq!(a.title, "[Group] One Punch Man - 05 [1080p] & more");
        assert_eq!(a.category, vec!["5070".to_string()]);
        assert_eq!((a.category_source.as_str(), a.category_site_id.as_str()), ("nyaa", "1_2"));
        assert_eq!(a.category_label, "Anime - English-translated");
        assert!(!a.category_inferred);
        assert_eq!((a.seeders, a.peers), (120, 4));
        assert_eq!(a.size, (1.4 * 1024f64.powi(3)) as u64);
        assert_eq!(a.pub_date, "Wed, 07 Oct 2026 10:00:00 -0000");
        assert_eq!(a.guid, "https://nyaa.si/view/100");
        assert_eq!(a.sources, vec!["nyaasi".to_string()]);
        let magnet = a.magnet_link();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn="));
        assert!(magnet.contains("&tr="));
        assert_eq!(crate::models::info_hash(&magnet).as_deref(), Some("0123456789abcdef0123456789abcdef01234567"));
        assert_eq!(a.torrent_url(), "https://nyaa.si/download/100.torrent");
        assert_eq!(a.display_category(), Some(5070));
        assert_eq!(a.grab_category(), "5070");

        let b = &items[1];
        assert_eq!(b.title, "Ubuntu 26.04 <amd64>");
        assert_eq!(b.category, vec!["4000".to_string()]);
        assert_eq!(b.category_label, "Software - Applications");
        assert_eq!(b.size, 650 * 1024 * 1024);
        assert!(b.magnet_link().contains("btih:fedcba9876543210fedcba9876543210fedcba98"));

        // An id that isn't in the table keeps the feed's name, without a Torznab id.
        let c = &items[2];
        assert!(c.category.is_empty());
        assert_eq!((c.category_site_id.as_str(), c.category_label.as_str()), ("7_1", "Something - New"));
        assert_eq!(c.grab_category(), "5000");
    }

    #[test]
    fn parses_sukebei_feed() {
        // sukebei binds the nyaa: prefix to its own namespace URI.
        let items = parse(SUKEBEI_FEED, Site::Sukebei, &src("sukebeinyaasi"));
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.title, "Some & Title");
        assert_eq!(it.category, vec!["6000".to_string()]);
        assert_eq!(it.category_site_id, "2_1");
        assert_eq!(it.category_label, "Real Life - Photobooks / Pictures");
        assert_eq!(it.category_source, "sukebei");
        assert_eq!((it.seeders, it.peers), (12, 3));
        assert_eq!(it.size, (1.5 * 1024f64.powi(3)) as u64);
        assert!(it.magnet_link().starts_with("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01"));
        assert_eq!(it.torrent_url(), "https://sukebei.nyaa.si/download/1.torrent");
        assert_eq!(it.grab_category(), "6000");
    }

    #[test]
    fn empty_and_broken_feeds() {
        assert!(parse(r#"<rss version="2.0"><channel><title>x</title></channel></rss>"#, Site::Nyaa, &src("nyaasi")).is_empty());
        assert!(parse("<html>Service unavailable</html>", Site::Nyaa, &src("nyaasi")).is_empty());
    }

    #[test]
    fn tags_match_whole_names() {
        let chunk = "<nyaa:categoryId>1_2</nyaa:categoryId><nyaa:category>Anime - Raw</nyaa:category>";
        assert_eq!(tag(chunk, "nyaa:category").as_deref(), Some("Anime - Raw"));
        assert_eq!(tag(chunk, "nyaa:categoryId").as_deref(), Some("1_2"));
        assert_eq!(tag(chunk, "nyaa:size"), None);
        assert_eq!(tag(r#"<guid isPermaLink="true">u</guid>"#, "guid").as_deref(), Some("u"));
    }

    #[test]
    fn nyaa_table() {
        let ids: Vec<&str> = NYAA.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                "1_0", "1_1", "1_2", "1_3", "1_4", "2_0", "2_1", "2_2", "3_0", "3_1", "3_2", "3_3", "4_0", "4_1",
                "4_2", "4_3", "4_4", "5_0", "5_1", "5_2", "6_0", "6_1", "6_2"
            ]
        );
        let torznab = |id| category(Site::Nyaa, id).unwrap().torznab;
        for id in ["1_0", "1_1", "1_2", "1_3", "1_4"] {
            assert_eq!(torznab(id), 5070);
        }
        assert_eq!((torznab("2_0"), torznab("2_1"), torznab("2_2")), (3000, 3040, 3000));
        for id in ["3_0", "3_1", "3_2", "3_3"] {
            assert_eq!(torznab(id), 7000);
        }
        for id in ["4_0", "4_1", "4_2", "4_3", "4_4"] {
            assert_eq!(torznab(id), 5000);
        }
        for id in ["5_0", "5_1", "5_2"] {
            assert_eq!(torznab(id), 8000);
        }
        assert_eq!((torznab("6_0"), torznab("6_1"), torznab("6_2")), (4000, 4000, 4050));
        assert_eq!(category(Site::Nyaa, "1_2").unwrap().name, "Anime - English-translated");
        assert_eq!(category(Site::Nyaa, "4_2").unwrap().name, "Live Action - Idol/Promotional Video");
        assert!(category(Site::Nyaa, "9_9").is_none());
        // The filter groups the UI offers.
        assert_eq!(category_group(Some(torznab("1_2"))), "anime");
        assert_eq!(category_group(Some(torznab("2_1"))), "music");
        assert_eq!(category_group(Some(torznab("3_1"))), "books");
        assert_eq!(category_group(Some(torznab("4_1"))), "tv");
        assert_eq!(category_group(Some(torznab("5_1"))), "other");
        assert_eq!(category_group(Some(torznab("6_1"))), "software");
        assert_eq!(category_group(Some(torznab("6_2"))), "games");
    }

    #[test]
    fn sukebei_table() {
        let ids: Vec<&str> = SUKEBEI.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["1_0", "1_1", "1_2", "1_3", "1_4", "1_5", "2_0", "2_1", "2_2"]);
        assert!(SUKEBEI.iter().all(|c| c.torznab == 6000));
        assert_eq!(category(Site::Sukebei, "1_1").unwrap().name, "Art - Anime");
        assert_eq!(category(Site::Sukebei, "2_1").unwrap().name, "Real Life - Photobooks / Pictures");
        assert_eq!(category_group(Some(6000)), "xxx");
    }

    #[test]
    fn badge_and_tooltip() {
        assert!(NYAA.iter().chain(SUKEBEI.iter()).all(|c| !c.short.is_empty() && c.short.len() <= 14));
        assert_eq!(short_name("nyaa", "1_2", "Anime - English-translated"), "Anime EN");
        assert_eq!(short_name("sukebei", "1_2", "Art - Doujinshi"), "Doujinshi");
        assert_eq!(short_name("nyaa", "7_1", "Something - New"), "Something - New");
        assert_eq!(
            tooltip("nyaa", "1_2", "Anime - English-translated", Some(5070)),
            "Anime - English-translated (Nyaa 1_2) · TV/Anime (5070)"
        );
        assert_eq!(tooltip("sukebei", "1_5", "Art - Pictures", Some(6000)), "Art - Pictures (sukebei 1_5) · XXX (6000)");
        assert_eq!(tooltip("nyaa", "7_1", "Something - New", None), "Something - New (Nyaa 7_1)");
    }

    #[test]
    fn episode_zero_retry() {
        assert_eq!(without_episode_zeros("frieren 05").as_deref(), Some("frieren 5"));
        assert_eq!(without_episode_zeros("05 frieren 07").as_deref(), Some("5 frieren 7"));
        assert_eq!(without_episode_zeros("show - 03 [1080p]").as_deref(), Some("show - 3 [1080p]"));
        assert_eq!(without_episode_zeros("one punch"), None);
        assert_eq!(without_episode_zeros("s01e05"), None);
        assert_eq!(without_episode_zeros("2005"), None);
        assert_eq!(without_episode_zeros("show 012"), None);
        assert_eq!(without_episode_zeros("show 10"), None);
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("1.0 KiB"), 1024);
        assert_eq!(parse_size("2 TiB"), 2 * 1024u64.pow(4));
        assert_eq!(parse_size("17 Bytes"), 17);
        assert_eq!(parse_size("junk"), 0);
    }
}
