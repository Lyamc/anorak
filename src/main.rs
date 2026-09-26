mod config;
mod models;
mod routes;
mod utils;

mod app_error;
use axum::{
    extract::Request,
    http::{header, HeaderValue},
    middleware::{map_request, map_response},
    response::Response,
    routing::{get, post},
    Router,
};
use log::info;
use minijinja::{path_loader, Environment};
use once_cell::sync::Lazy;
use pretty_env_logger;
use tower_http::services::ServeDir;

use crate::config::CONFIG;

pub static ENV: Lazy<Environment<'static>> = Lazy::new(|| {
    let mut env = Environment::new();
    env.set_loader(path_loader("assets"));
    env
});

#[tokio::main]
pub async fn main() {
    pretty_env_logger::init();

    let app = Router::new()
        .route("/query/", post(routes::query::endpoint))
        .route(
            "/api/query",
            get(routes::api::query_get).post(routes::api::query_post),
        )
        .route("/send-to-rqbit/", post(routes::send_to_rqbit::endpoint))
        // Optional GPUI WebAssembly client (anorak-gpui/, `trunk build --release`),
        // served same-origin so it can call /api/query and /send-to-rqbit/
        // without CORS. ServeDir sends `.wasm` as application/wasm.
        .nest_service("/gpui", ServeDir::new(gpui_dist()))
        .nest_service("/", ServeDir::new("assets"))
        .layer(map_request(strip_conditional_headers))
        .layer(map_response(no_stale_cache));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", CONFIG.port))
        .await
        .unwrap();
    info!("Anorak running on http://localhost:{}", CONFIG.port);
    axum::serve(listener, app).await.unwrap();
}

/// Directory holding the Trunk output of anorak-gpui. `ANORAK_GPUI_DIST`
/// overrides it; if the directory is missing, /gpui/ simply returns 404.
fn gpui_dist() -> String {
    std::env::var("ANORAK_GPUI_DIST").unwrap_or_else(|_| "anorak-gpui/dist".to_string())
}

// Assets are served from the Nix store, where every file's mtime is the epoch.
// ServeDir then sends `Last-Modified: 1970`, so browsers cache heuristically
// for years and answer every revalidation with 304 -> old HTML/CSS after a
// deploy. Drop the useless validator and ask clients to refetch.
async fn strip_conditional_headers(mut req: Request) -> Request {
    req.headers_mut().remove(header::IF_MODIFIED_SINCE);
    req.headers_mut().remove(header::IF_UNMODIFIED_SINCE);
    req
}

async fn no_stale_cache(mut res: Response) -> Response {
    let headers = res.headers_mut();
    headers.remove(header::LAST_MODIFIED);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    res
}
