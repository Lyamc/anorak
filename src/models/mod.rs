use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Rss {
    pub channel: Channel,
}

#[derive(Debug, Deserialize)]
pub struct Channel {
    #[serde(default)]
    pub item: Vec<Item>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Item {
    pub title: String,
    pub guid: String,
    /// Torznab/Jackett often put the magnet here (guid is frequently a page URL).
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub comments: String,
    #[serde(rename = "pubDate")]
    pub pub_date: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub files: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: Vec<String>,
    /// Injected from torznab:attr before XML deserialize.
    #[serde(default)]
    pub seeders: u32,
    /// Injected from torznab:attr before XML deserialize.
    #[serde(default)]
    pub peers: u32,
    /// Injected from torznab:attr before XML deserialize.
    #[serde(default)]
    pub magneturl: String,
}

impl Item {
    /// Prefer real magnets over guid (which is often a details-page URL).
    /// `link` is preferred over `magneturl` because link is XML-decoded by
    /// serde, while magneturl is lifted from attr values that may still
    /// contain `&amp;` entities.
    pub fn magnet_link(&self) -> String {
        let candidates = [self.link.as_str(), self.magneturl.as_str(), self.guid.as_str()];
        for candidate in candidates {
            let decoded = decode_basic_entities(candidate.trim());
            if decoded.to_ascii_lowercase().starts_with("magnet:") {
                return decoded;
            }
        }
        for candidate in candidates {
            let decoded = decode_basic_entities(candidate.trim());
            if !decoded.is_empty() {
                return decoded;
            }
        }
        String::new()
    }
}

fn decode_basic_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}


#[derive(Deserialize)]
pub struct Query {
    pub search_term: String,
}

#[derive(Serialize, Deserialize)]
pub struct SendToTransmission {
    pub magnet: String,
    /// Optional Newznab/Torznab category id (may be empty when unknown).
    #[serde(default)]
    pub category: Option<String>,
}

/// Pick one Torznab category id from a list of category strings.
/// Prefers 5070 (Anime); else the most specific (non-thousand) id; else the first parseable.
pub fn prefer_torznab_category(categories: &[String]) -> Option<u32> {
    let parsed: Vec<u32> = categories.iter().filter_map(|c| c.trim().parse().ok()).collect();
    if parsed.is_empty() {
        return None;
    }
    if parsed.iter().any(|&c| c == 5070) {
        return Some(5070);
    }
    // Prefer subcategory (non-round thousands) over generic bucket.
    parsed
        .iter()
        .copied()
        .max_by_key(|c| (if c % 1000 == 0 { 0u8 } else { 1u8 }, *c))
}
