//! HTTP requests to rqbit.
//!
//! * Request bodies are zstd-compressed (`Content-Encoding: zstd`). rqbit
//!   decompresses request bodies on every endpoint.
//! * If rqbit answers 415 (it doesn't take that encoding; nothing was done),
//!   the same body is sent once more uncompressed, and every later body in
//!   this process goes out uncompressed.
//! * Every request carries `User-Agent: anorak/<version>` and
//!   `Accept-Encoding: zstd, br, gzip`; reqwest decodes compressed answers.

use crate::config::CONFIG;
use log::{debug, info, warn};
use once_cell::sync::Lazy;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE};
use reqwest::{RequestBuilder, Response, StatusCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const USER_AGENT: &str = concat!("anorak/", env!("CARGO_PKG_VERSION"));
pub const ACCEPT_ENCODINGS: &str = "zstd, br, gzip";
/// zstd level for request bodies (small JSON; level 3 is zstd's default).
const ZSTD_LEVEL: i32 = 3;

/// Set after rqbit answered 415 to a compressed body.
static PLAIN_BODIES: AtomicBool = AtomicBool::new(false);

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static(ACCEPT_ENCODINGS));
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client")
});

pub fn base() -> String {
    CONFIG.rqbit_url.trim_end_matches('/').to_string()
}

/// `GET {rqbit}{path}` asking for JSON.
pub async fn get(path: &str, timeout: Duration) -> reqwest::Result<Response> {
    CLIENT
        .get(format!("{}{path}", base()))
        .header(ACCEPT, "application/json")
        .timeout(timeout)
        .send()
        .await
}

/// `POST {rqbit}{path}` without a body.
pub async fn post_empty(path: &str, timeout: Duration) -> reqwest::Result<Response> {
    CLIENT.post(format!("{}{path}", base())).timeout(timeout).send().await
}

/// `POST {rqbit}{path}` with a JSON body.
pub async fn post_json(
    path: &str,
    body: &serde_json::Value,
    timeout: Duration,
    headers: &[(&'static str, String)],
) -> reqwest::Result<Response> {
    let bytes = serde_json::to_vec(body).expect("serializing JSON");
    post_body(&base(), path, bytes, "application/json", timeout, headers).await
}

/// `POST {rqbit}{path}` with a plain-text body (endpoints that take a bare string).
pub async fn post_text(path: &str, body: &str, timeout: Duration) -> reqwest::Result<Response> {
    post_body(&base(), path, body.as_bytes().to_vec(), "text/plain; charset=utf-8", timeout, &[]).await
}

pub fn compress(body: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::bulk::compress(body, ZSTD_LEVEL)
}

async fn post_body(
    base: &str,
    path: &str,
    body: Vec<u8>,
    content_type: &'static str,
    timeout: Duration,
    headers: &[(&'static str, String)],
) -> reqwest::Result<Response> {
    let request = || {
        let mut r: RequestBuilder = CLIENT
            .post(format!("{base}{path}"))
            .header(CONTENT_TYPE, content_type)
            .timeout(timeout);
        for (name, value) in headers {
            r = r.header(*name, value.as_str());
        }
        r
    };
    if !PLAIN_BODIES.load(Ordering::Relaxed) {
        match compress(&body) {
            Ok(compressed) => {
                log_sizes(path, body.len(), Some(compressed.len()));
                let response = request()
                    .header(CONTENT_ENCODING, "zstd")
                    .body(compressed)
                    .send()
                    .await?;
                if response.status() != StatusCode::UNSUPPORTED_MEDIA_TYPE {
                    return Ok(response);
                }
                let accepts = response
                    .headers()
                    .get(ACCEPT_ENCODING)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("not given")
                    .to_string();
                PLAIN_BODIES.store(true, Ordering::Relaxed);
                warn!(
                    "rqbit answered 415 to a zstd body on POST {path} (it accepts: {accepts}); \
                     sending request bodies uncompressed from now on"
                );
            }
            Err(err) => warn!("zstd compression failed ({err}); sending POST {path} uncompressed"),
        }
    }
    log_sizes(path, body.len(), None);
    request().body(body).send().await
}

fn log_sizes(path: &str, plain: usize, compressed: Option<usize>) {
    let line = match compressed {
        Some(c) => format!("rqbit POST {path}: {plain} B body, {c} B zstd"),
        None => format!("rqbit POST {path}: {plain} B body, uncompressed"),
    };
    // Adds are rare and worth a line; anything else only in debug.
    if path == "/torrents" {
        info!("{line}");
    } else {
        debug!("{line}");
    }
}

/// Whether bodies currently go out uncompressed (after a 415).
#[cfg(test)]
fn plain_bodies() -> bool {
    PLAIN_BODIES.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn compression_round_trip() {
        let body = br#"{"url":"magnet:?xt=urn:btih:c9e15763f722f23e98a29decdfae341b98d53056"}"#;
        let c = compress(body).unwrap();
        assert_eq!(&c[..4], &[0x28, 0xb5, 0x2f, 0xfd], "zstd frame magic");
        assert_eq!(zstd::bulk::decompress(&c, 1 << 20).unwrap(), body);
    }

    #[test]
    fn user_agent_has_version() {
        assert!(USER_AGENT.starts_with("anorak/") && USER_AGENT.len() > "anorak/".len());
    }

    /// One HTTP/1.1 request read off a socket: head and body.
    fn read_request(stream: &mut std::net::TcpStream) -> (String, Vec<u8>) {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            buf.extend_from_slice(&chunk[..n]);
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..end]).to_string();
                let len = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                let mut body = buf[end + 4..].to_vec();
                while body.len() < len {
                    let n = stream.read(&mut chunk).unwrap();
                    body.extend_from_slice(&chunk[..n]);
                }
                return (head, body);
            }
            if n == 0 {
                panic!("connection closed");
            }
        }
    }

    /// A server that answers 415 to anything with Content-Encoding and 200 otherwise.
    #[tokio::test]
    async fn falls_back_to_plain_after_415_and_remembers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<(String, Vec<u8>)>>> = Arc::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let (head, body) = read_request(&mut stream);
                let encoded = head.to_ascii_lowercase().contains("content-encoding:");
                log.lock().unwrap().push((head, body));
                let reply = if encoded {
                    "HTTP/1.1 415 Unsupported Media Type\r\naccept-encoding: identity\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}"
                };
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        let json = br#"{"url":"magnet:?xt=urn:btih:c9e15763f722f23e98a29decdfae341b98d53056"}"#.to_vec();
        let t = Duration::from_secs(5);
        let extra = [("x-req-timeout-ms", "3600000".to_string())];
        let r = post_body(&base, "/torrents", json.clone(), "application/json", t, &extra).await.unwrap();
        assert_eq!(r.status(), 200);
        assert!(plain_bodies());
        let r = post_body(&base, "/torrents", json.clone(), "application/json", t, &extra).await.unwrap();
        assert_eq!(r.status(), 200);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "compressed, plain retry, then plain only");
        let lower = |i: usize| seen[i].0.to_ascii_lowercase();
        assert!(lower(0).contains("content-encoding: zstd"));
        assert_eq!(zstd::bulk::decompress(&seen[0].1, 1 << 20).unwrap(), json);
        for i in 1..3 {
            assert!(!lower(i).contains("content-encoding"));
            assert_eq!(seen[i].1, json, "same JSON");
        }
        for i in 0..3 {
            assert!(lower(i).contains("content-type: application/json"));
            assert!(lower(i).contains("x-req-timeout-ms: 3600000"));
            assert!(lower(i).contains(&format!("user-agent: {}", USER_AGENT.to_ascii_lowercase())));
            assert!(lower(i).contains("accept-encoding: zstd, br, gzip"));
        }
    }

    /// Run by hand against a real rqbit; adds nothing. Sends a compressed
    /// body with an unknown field and no link, which rqbit must refuse as
    /// `400 invalid_input`:
    /// `ANORAK_LIVE_RQBIT_URL=http://127.0.0.1:3030 cargo test live_rqbit -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn live_rqbit_refuses_unknown_field() {
        let base = std::env::var("ANORAK_LIVE_RQBIT_URL").expect("ANORAK_LIVE_RQBIT_URL");
        let base = base.trim_end_matches('/');
        let body = serde_json::to_vec(&serde_json::json!({"anorak_check_unknown_field": true})).unwrap();
        let r = post_body(base, "/torrents", body, "application/json", Duration::from_secs(10), &[])
            .await
            .unwrap();
        let status = r.status();
        let text = r.text().await.unwrap();
        println!("live rqbit: {status} {text}");
        assert!(!plain_bodies(), "rqbit refused the zstd body (415)");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["error_kind"], "invalid_input");
    }
}
