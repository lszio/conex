//! Same-origin connected landing page assets.
//!
//! Only the three explicitly supported paths are mounted. Assets are read and
//! validated at startup so requests cannot turn the configured root into an
//! arbitrary file server.
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Router};
use conex_core::CallError;

const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self' ws: wss:; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";

#[derive(Clone)]
pub struct WebAssets {
    index: Arc<Vec<u8>>,
    app: Arc<Vec<u8>>,
    style: Arc<Vec<u8>>,
    /// Machine-readable site summary for LLM crawlers. Optional: a build
    /// without it must still serve the human page.
    llms: Option<Arc<Vec<u8>>>,
}

impl WebAssets {
    /// Load exactly the files used by the landing page and reject missing or
    /// escaping paths before the host starts listening.
    pub fn load(root: &Path) -> Result<Self, CallError> {
        let root = root
            .canonicalize()
            .map_err(|error| invalid(format!("web_root {}: {error}", root.display())))?;
        if !root.is_dir() {
            return Err(invalid(format!(
                "web_root is not a directory: {}",
                root.display()
            )));
        }
        Ok(Self {
            index: Arc::new(read_asset(&root, "index.html")?),
            app: Arc::new(read_asset(&root, "app.js")?),
            style: Arc::new(read_asset(&root, "style.css")?),
            llms: read_asset(&root, "llms.txt").ok().map(Arc::new),
        })
    }

    pub fn router(self) -> Router {
        let llms = self.llms.clone();
        let mut router = Router::new()
            .route("/", get(index))
            .route("/app.js", get(app))
            .route("/style.css", get(style));
        if llms.is_some() {
            // llms.txt is the llmstxt.org convention; /llm.txt is the older
            // spelling and both must resolve for crawlers that use either.
            router = router
                .route("/llms.txt", get(llms_txt))
                .route("/llm.txt", get(llms_txt));
        }
        router.layer(Extension(self))
    }
}

fn read_asset(root: &Path, name: &str) -> Result<Vec<u8>, CallError> {
    let path = root.join(name);
    let canonical = path
        .canonicalize()
        .map_err(|error| invalid(format!("missing web asset {}: {error}", path.display())))?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(invalid(format!("web asset escapes web_root: {name}")));
    }
    std::fs::read(&canonical)
        .map_err(|error| invalid(format!("read web asset {}: {error}", canonical.display())))
}

async fn index(Extension(assets): Extension<WebAssets>) -> Response {
    asset_response(&assets.index, "text/html; charset=utf-8", true)
}

async fn app(Extension(assets): Extension<WebAssets>) -> Response {
    asset_response(&assets.app, "text/javascript; charset=utf-8", false)
}

async fn style(Extension(assets): Extension<WebAssets>) -> Response {
    asset_response(&assets.style, "text/css; charset=utf-8", false)
}

async fn llms_txt(Extension(assets): Extension<WebAssets>) -> Response {
    // The route only exists when the file loaded, so the missing arm cannot
    // fire; serving an empty body is still safer than an HTML fallback.
    let bytes: &[u8] = assets.llms.as_deref().map(|b| b.as_slice()).unwrap_or(&[]);
    asset_response(bytes, "text/plain; charset=utf-8", false)
}

fn asset_response(bytes: &[u8], content_type: &'static str, html: bool) -> Response {
    let mut response = (StatusCode::OK, Body::from(bytes.to_vec())).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if html {
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
    }
    response
}

fn invalid(message: impl std::fmt::Display) -> CallError {
    CallError::new(conex_proto::ErrorCode::Internal, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[test]
    fn rejects_missing_fixed_asset() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "ok").unwrap();
        assert!(WebAssets::load(dir.path()).is_err());
    }

    #[test]
    fn loads_only_fixed_assets() {
        let dir = tempdir().unwrap();
        for name in ["index.html", "app.js", "style.css"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let assets = WebAssets::load(dir.path()).unwrap();
        assert_eq!(&*assets.index, b"index.html");
        assert_eq!(&*assets.app, b"app.js");
        assert_eq!(&*assets.style, b"style.css");
    }

    #[tokio::test]
    async fn llms_txt_is_served_under_both_spellings_when_present() {
        let dir = tempdir().unwrap();
        for name in ["index.html", "app.js", "style.css"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        std::fs::write(dir.path().join("llms.txt"), "# conex\n\nsummary").unwrap();
        let app = WebAssets::load(dir.path()).unwrap().router();
        for path in ["/llms.txt", "/llm.txt"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/plain; charset=utf-8"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body");
            assert_eq!(body.as_ref(), b"# conex\n\nsummary");
        }
    }

    #[tokio::test]
    async fn missing_llms_txt_does_not_break_the_page() {
        let dir = tempdir().unwrap();
        for name in ["index.html", "app.js", "style.css"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        let app = WebAssets::load(dir.path()).unwrap().router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        // No file means no route: /llms.txt must 404, not serve HTML.
        let app = WebAssets::load(dir.path()).unwrap().router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/llms.txt")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
