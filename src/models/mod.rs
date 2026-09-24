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
}
