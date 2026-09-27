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
    /// Source (indexer) display names; only known via Lodestarr's native API.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Source (indexer) ids, parallel to `sources`.
    #[serde(default)]
    pub source_ids: Vec<String>,
    /// `category` came from the source's only declared category, not the result.
    #[serde(default)]
    pub category_inferred: bool,
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

    /// A direct .torrent URL, if the source gave one (or, for Nyaa-style
    /// sites, the download URL that goes with a `/view/<id>` page).
    pub fn torrent_url(&self) -> String {
        let link = decode_basic_entities(self.link.trim());
        let lower = link.to_ascii_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://"))
            && (lower.ends_with(".torrent") || lower.contains("/download/"))
        {
            return link;
        }
        for page in [self.guid.as_str(), self.comments.as_str()] {
            if let Ok(url) = reqwest::Url::parse(page.trim()) {
                let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
                let path = url.path();
                if host.contains("nyaa") {
                    if let Some(id) = path.strip_prefix("/view/") {
                        let id = id.trim_end_matches('/');
                        if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                            return format!("{}://{}/download/{}.torrent", url.scheme(), host, id);
                        }
                    }
                }
            }
        }
        String::new()
    }

    /// Best Torznab category id for display and filtering (None = unknown).
    pub fn display_category(&self) -> Option<u32> {
        prefer_torznab_category(&self.category)
    }

    /// Category forwarded to rqbit on Grab. Unknown categories stay "5000",
    /// which is what Lodestarr's Torznab feed has always reported for them,
    /// so where grabs land doesn't change. Inferred categories are display-only.
    pub fn grab_category(&self) -> String {
        if self.sources.is_empty() {
            // Torznab path: forward what the feed said, as before.
            return prefer_torznab_category(&self.category)
                .map(|c| c.to_string())
                .unwrap_or_default();
        }
        if self.category_inferred {
            return "5000".to_string();
        }
        prefer_torznab_category(&self.category)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "5000".to_string())
    }
}

fn decode_basic_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Lower-case hex/base32 info-hash from a magnet link.
pub fn info_hash(magnet: &str) -> Option<String> {
    let lower = magnet.to_ascii_lowercase();
    let start = lower.find("xt=urn:btih:")? + "xt=urn:btih:".len();
    let hash: String = lower[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (!hash.is_empty()).then_some(hash)
}

#[derive(Deserialize)]
pub struct Query {
    pub search_term: String,
    /// Comma-separated source (indexer) ids to search; empty = all enabled.
    #[serde(default)]
    pub indexers: String,
    /// Category group to keep (movies, tv, anime, music, books, games,
    /// software, xxx, other, unknown); empty = all. Applied by anorak, since
    /// Lodestarr ignores Torznab `cat=`.
    #[serde(default)]
    pub cat: String,
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

/// Category groups offered by the web UI's Category filter.
pub const CATEGORY_GROUPS: &[(&str, &str)] = &[
    ("movies", "Movies"),
    ("tv", "TV"),
    ("anime", "Anime"),
    ("music", "Music"),
    ("books", "Books"),
    ("games", "Games"),
    ("software", "Software"),
    ("xxx", "XXX"),
    ("other", "Other"),
    ("unknown", "Unknown"),
];

pub fn category_group(id: Option<u32>) -> &'static str {
    match id {
        None => "unknown",
        Some(5070) => "anime",
        Some(4050) => "games",
        Some(c) if c >= 100_000 => "unknown",
        Some(c) => match c / 1000 {
            1 => "games",
            2 => "movies",
            3 => "music",
            4 => "software",
            5 => "tv",
            6 => "xxx",
            7 => "books",
            8 => "other",
            _ => "unknown",
        },
    }
}

/// Standard Torznab category names (as listed by Lodestarr's caps).
pub fn category_name(id: u32) -> String {
    const NAMES: &[(u32, &str)] = &[
        (1000, "Console"), (1010, "Console/NDS"), (1020, "Console/PSP"), (1030, "Console/Wii"),
        (1040, "Console/Xbox"), (1050, "Console/Xbox360"), (1060, "Console/WiiWare"),
        (1070, "Console/XBOX 360 DLC"), (1080, "Console/PS3"), (1090, "Console/Other"),
        (1110, "Console/3DS"), (1120, "Console/PS Vita"), (1130, "Console/WiiU"),
        (1140, "Console/XboxOne"), (1180, "Console/PS4"),
        (2000, "Movies"), (2010, "Movies/Foreign"), (2020, "Movies/Other"), (2030, "Movies/SD"),
        (2040, "Movies/HD"), (2045, "Movies/UHD"), (2050, "Movies/BluRay"), (2060, "Movies/3D"),
        (2070, "Movies/DVD"), (2080, "Movies/WEB-DL"), (2090, "Movies/x265"),
        (3000, "Audio"), (3010, "Audio/MP3"), (3020, "Audio/Video"), (3030, "Audio/Audiobook"),
        (3040, "Audio/Lossless"), (3050, "Audio/Other"), (3060, "Audio/Foreign"),
        (4000, "PC"), (4010, "PC/0day"), (4020, "PC/ISO"), (4030, "PC/Mac"),
        (4040, "PC/Mobile-Other"), (4050, "PC/Games"), (4060, "PC/Mobile-iOS"),
        (4070, "PC/Mobile-Android"),
        (5000, "TV"), (5010, "TV/WEB-DL"), (5020, "TV/Foreign"), (5030, "TV/SD"), (5040, "TV/HD"),
        (5045, "TV/UHD"), (5050, "TV/Other"), (5060, "TV/Sport"), (5070, "TV/Anime"),
        (5080, "TV/Documentary"), (5090, "TV/x265"),
        (6000, "XXX"), (6010, "XXX/DVD"), (6020, "XXX/WMV"), (6030, "XXX/XviD"), (6040, "XXX/x264"),
        (6045, "XXX/UHD"), (6050, "XXX/Other"), (6060, "XXX/ImageSet"), (6070, "XXX/Other"),
        (6080, "XXX/SD"), (6090, "XXX/WEB-DL"),
        (7000, "Books"), (7010, "Books/Mags"), (7020, "Books/EBook"), (7030, "Books/Comics"),
        (7040, "Books/Technical"), (7050, "Books/Other"), (7060, "Books/Foreign"),
        (8000, "Other"), (8010, "Other/Misc"), (8020, "Other/Hashed"),
    ];
    if let Some((_, name)) = NAMES.iter().find(|(c, _)| *c == id) {
        return name.to_string();
    }
    if let Some((_, name)) = NAMES.iter().find(|(c, _)| *c == id / 1000 * 1000) {
        return name.to_string();
    }
    id.to_string()
}
