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
}

#[derive(Deserialize)]
pub struct Query {
   pub search_term : String,
}

#[derive(Serialize, Deserialize)]
pub struct SendToTransmission {
   pub magnet : String,
}