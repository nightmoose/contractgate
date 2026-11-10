//! In-process TTL cache for anonymous public catalog reads.
//!
//! `GET /catalog`, `GET /public-contracts`, `GET /public-contract-families`,
//! and `GET /public-contracts/{slug}` (including `/yaml`, `/odcs`, and
//! `/family`) otherwise query Postgres on every request and send no
//! `Cache-Control`. This cache stores the successful anonymous response for
//! [`TTL`] (60s), keyed by path and query string, and stamps
//! [`PUBLIC_CACHE_CONTROL`] so a shared cache can hold it longer.
//!
//! Requests that carry `Authorization` or `x-api-key` are never stored and
//! are marked `private, no-store`. The response for those routes does not
//! depend on the caller, but a credentialed request must not be reused as a
//! public cache entry.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{Method, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt;

/// Browser freshness. Matches the in-process TTL.
pub const TTL: Duration = Duration::from_secs(60);

/// `Cache-Control` on a successful anonymous catalog read.
///
/// `max-age` matches [`TTL`]. `s-maxage` lets a shared cache (Fly's proxy or
/// a CDN) keep the object for five minutes and serve a stale copy for ten
/// while it revalidates, so a cold dashboard does not stampede Postgres.
pub const PUBLIC_CACHE_CONTROL: &str =
    "public, max-age=60, s-maxage=300, stale-while-revalidate=600";

/// Credentialed catalog reads must not enter a shared cache.
const PRIVATE_CACHE_CONTROL: &str = "private, no-store";

/// Working set is the seeded catalog plus a modest number of query variants.
const MAX_ENTRIES: usize = 512;

/// Bodies larger than this are still returned with `Cache-Control`, but not
/// kept in the process. Catalog payloads are far smaller.
const MAX_BODY_BYTES: usize = 1024 * 1024;

struct Entry {
    headers: HeaderMap,
    body: Vec<u8>,
    inserted_at: Instant,
}

/// Bounded map of anonymous `200` catalog responses.
pub struct PublicReadCache {
    inner: Mutex<HashMap<String, Entry>>,
}

impl Default for PublicReadCache {
    fn default() -> Self {
        PublicReadCache {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

impl PublicReadCache {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.inner.lock().unwrap_or_else(|e| {
            tracing::error!("public-read cache Mutex was poisoned — recovering inner value");
            e.into_inner()
        })
    }

    /// Fresh hit, if any. A stale entry is dropped so the next fill replaces it.
    pub fn get(&self, key: &str) -> Option<Response> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &str, now: Instant) -> Option<Response> {
        let mut map = self.lock();
        let fresh = match map.get(key) {
            Some(entry) if now.saturating_duration_since(entry.inserted_at) < TTL => true,
            Some(_) => false,
            None => return None,
        };
        if !fresh {
            map.remove(key);
            return None;
        }
        let entry = map.get(key)?;
        Some(response_from_parts(
            entry.headers.clone(),
            entry.body.clone(),
        ))
    }

    fn insert_at(&self, key: String, headers: HeaderMap, body: Vec<u8>, now: Instant) {
        if body.len() > MAX_BODY_BYTES {
            return;
        }
        let mut map = self.lock();
        map.retain(|_, entry| now.saturating_duration_since(entry.inserted_at) < TTL);
        if map.len() >= MAX_ENTRIES && !map.contains_key(&key) {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, entry)| entry.inserted_at)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(
            key,
            Entry {
                headers,
                body,
                inserted_at: now,
            },
        );
    }
}

/// True for the anonymous catalog GETs this cache is allowed to store.
pub fn is_public_catalog_path(path: &str) -> bool {
    path == "/catalog"
        || path == "/public-contracts"
        || path == "/public-contract-families"
        || path.starts_with("/public-contracts/")
        || path.starts_with("/public-contract-families/")
}

/// Path plus query. `/catalog` and `/catalog?limit=20` are different entries.
pub fn cache_key(uri: &Uri) -> String {
    uri.path_and_query()
        .map(|pq| pq.as_str().to_owned())
        .unwrap_or_else(|| uri.path().to_owned())
}

/// `Authorization` or `x-api-key` with a value. An opaque (non-UTF-8) header
/// counts as a credential so the response is not stored.
pub fn request_is_anonymous(headers: &HeaderMap) -> bool {
    !credential_present(headers, header::AUTHORIZATION) && !credential_present(headers, "x-api-key")
}

fn credential_present(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> bool {
    match headers.get(name) {
        None => false,
        Some(value) => match value.to_str() {
            Ok(text) => !text.trim().is_empty(),
            Err(_) => true,
        },
    }
}

/// Serve a cached anonymous `200`, or remember one after the handler runs.
/// Anything that is not an anonymous catalog GET is passed through unchanged.
pub async fn handle(cache: &PublicReadCache, request: Request, next: Next) -> Response {
    if request.method() != Method::GET || !is_public_catalog_path(request.uri().path()) {
        return next.run(request).await;
    }
    let key = cache_key(request.uri());
    if !request_is_anonymous(request.headers()) {
        return mark_private(next.run(request).await);
    }
    if let Some(hit) = cache.get(&key) {
        return hit;
    }
    remember(cache, &key, next.run(request).await).await
}

fn mark_private(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(PRIVATE_CACHE_CONTROL),
    );
    response
}

async fn remember(cache: &PublicReadCache, key: &str, response: Response) -> Response {
    if response.status() != StatusCode::OK {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            tracing::error!("public catalog response body failed: {err}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "catalog response could not be read",
            )
                .into_response();
        }
    };
    parts.headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(PUBLIC_CACHE_CONTROL),
    );
    parts.headers.insert(
        header::VARY,
        HeaderValue::from_static("authorization, x-api-key"),
    );
    // A Set-Cookie on a shared catalog response would leak into every caller.
    parts.headers.remove(header::SET_COOKIE);
    if bytes.len() <= MAX_BODY_BYTES {
        cache.insert_at(
            key.to_owned(),
            parts.headers.clone(),
            bytes.to_vec(),
            Instant::now(),
        );
    }
    response_from_parts(parts.headers, bytes.to_vec())
}

fn response_from_parts(headers: HeaderMap, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    *response.headers_mut() = headers;
    response
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use axum::routing::get;
    use axum::{middleware, Router};
    use axum_test::TestServer;

    use super::*;

    fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn catalog_paths_are_cached_and_other_public_routes_are_not() {
        assert!(is_public_catalog_path("/catalog"));
        assert!(is_public_catalog_path("/public-contracts"));
        assert!(is_public_catalog_path("/public-contract-families"));
        assert!(is_public_catalog_path("/public-contracts/stripe-charges"));
        assert!(is_public_catalog_path(
            "/public-contracts/stripe-charges/yaml"
        ));
        assert!(is_public_catalog_path(
            "/public-contracts/stripe-charges/odcs"
        ));
        assert!(is_public_catalog_path(
            "/public-contracts/stripe-charges/family"
        ));
        assert!(is_public_catalog_path(
            "/public-contract-families/stripe-charges-pipeline"
        ));
        assert!(!is_public_catalog_path("/health"));
        assert!(!is_public_catalog_path("/published/ref"));
        assert!(!is_public_catalog_path("/public-contracts-extra"));
        assert!(!is_public_catalog_path("/catalog/library"));
    }

    #[test]
    fn cache_key_includes_the_query_string() {
        let bare: Uri = "/catalog".parse().unwrap();
        let limited: Uri = "/catalog?limit=20".parse().unwrap();
        assert_eq!(cache_key(&bare), "/catalog");
        assert_eq!(cache_key(&limited), "/catalog?limit=20");
        assert_ne!(cache_key(&bare), cache_key(&limited));
    }

    #[test]
    fn credentials_are_not_anonymous() {
        assert!(request_is_anonymous(&HeaderMap::new()));
        assert!(request_is_anonymous(&header_map(&[("x-api-key", "  ")])));
        assert!(!request_is_anonymous(&header_map(&[(
            "x-api-key",
            "cg_live_example"
        )])));
        assert!(!request_is_anonymous(&header_map(&[(
            "authorization",
            "Bearer token"
        )])));
    }

    #[test]
    fn fresh_entry_is_served_and_stale_entry_is_dropped() {
        let cache = PublicReadCache::default();
        let headers = header_map(&[("content-type", "application/json")]);
        cache.insert_at(
            "/catalog".to_owned(),
            headers,
            br#"{"ok":true}"#.to_vec(),
            Instant::now() - Duration::from_secs(30),
        );
        let hit = cache
            .get_at("/catalog", Instant::now())
            .expect("fresh entry");
        assert_eq!(hit.status(), StatusCode::OK);
        assert_eq!(
            hit.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );

        cache.insert_at(
            "/catalog".to_owned(),
            HeaderMap::new(),
            b"stale".to_vec(),
            Instant::now() - TTL - Duration::from_secs(1),
        );
        assert!(cache.get_at("/catalog", Instant::now()).is_none());
    }

    #[test]
    fn insert_stays_bounded() {
        let cache = PublicReadCache::default();
        let now = Instant::now();
        for i in 0..(MAX_ENTRIES + 5) {
            cache.insert_at(
                format!("/public-contracts/{i}"),
                HeaderMap::new(),
                b"{}".to_vec(),
                now + Duration::from_millis(i as u64),
            );
        }
        let map = cache.lock();
        assert!(map.len() <= MAX_ENTRIES);
        assert!(!map.contains_key("/public-contracts/0"));
        assert!(map.contains_key(&format!("/public-contracts/{}", MAX_ENTRIES + 4)));
    }

    #[test]
    fn oversized_body_is_not_stored() {
        let cache = PublicReadCache::default();
        cache.insert_at(
            "/catalog".to_owned(),
            HeaderMap::new(),
            vec![0; MAX_BODY_BYTES + 1],
            Instant::now(),
        );
        assert!(cache.get("/catalog").is_none());
    }

    fn catalog_app(cache: Arc<PublicReadCache>, hits: Arc<AtomicUsize>) -> Router {
        let hits_catalog = hits.clone();
        let hits_yaml = hits.clone();
        Router::new()
            .route(
                "/catalog",
                get(move || {
                    let hits_catalog = hits_catalog.clone();
                    async move {
                        hits_catalog.fetch_add(1, Ordering::SeqCst);
                        (
                            StatusCode::OK,
                            [(header::CONTENT_TYPE, "application/json")],
                            r#"{"from":"handler"}"#,
                        )
                    }
                }),
            )
            .route(
                "/public-contracts/{id}/yaml",
                get(move || {
                    let hits_yaml = hits_yaml.clone();
                    async move {
                        hits_yaml.fetch_add(1, Ordering::SeqCst);
                        (StatusCode::OK, "contract: true")
                    }
                }),
            )
            .route("/health", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                cache,
                |axum::extract::State(cache): axum::extract::State<Arc<PublicReadCache>>,
                 request: Request,
                 next: Next| async move { handle(&cache, request, next).await },
            ))
    }

    #[tokio::test]
    async fn anonymous_get_is_cached_and_a_credentialed_get_is_not() {
        let cache = Arc::new(PublicReadCache::default());
        let hits = Arc::new(AtomicUsize::new(0));
        let server = TestServer::new(catalog_app(cache, hits.clone()));

        let first = server.get("/catalog").await;
        assert_eq!(first.status_code(), StatusCode::OK);
        assert_eq!(first.header("cache-control"), PUBLIC_CACHE_CONTROL);
        assert_eq!(first.header("vary"), "authorization, x-api-key");
        assert_eq!(first.text(), r#"{"from":"handler"}"#);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let second = server.get("/catalog?limit=1").await;
        assert_eq!(second.text(), r#"{"from":"handler"}"#);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "query string is a distinct key"
        );

        let cached = server.get("/catalog").await;
        assert_eq!(cached.text(), r#"{"from":"handler"}"#);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "a second anonymous GET is served from the cache"
        );

        let authed = server
            .get("/catalog")
            .add_header("x-api-key", "cg_live_example")
            .await;
        assert_eq!(authed.text(), r#"{"from":"handler"}"#);
        assert_eq!(authed.header("cache-control"), PRIVATE_CACHE_CONTROL);
        assert_eq!(hits.load(Ordering::SeqCst), 3);

        let still_cached = server.get("/catalog").await;
        assert_eq!(still_cached.text(), r#"{"from":"handler"}"#);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "the credentialed response must not replace the anonymous entry"
        );

        let health = server.get("/health").await;
        assert_eq!(health.text(), "ok");
        assert!(health.headers().get("cache-control").is_none());
    }

    #[tokio::test]
    async fn yaml_variant_is_cached() {
        let cache = Arc::new(PublicReadCache::default());
        let hits = Arc::new(AtomicUsize::new(0));
        let server = TestServer::new(catalog_app(cache.clone(), hits.clone()));
        let first = server.get("/public-contracts/stripe-charges/yaml").await;
        assert_eq!(first.status_code(), StatusCode::OK);
        assert_eq!(first.header("cache-control"), PUBLIC_CACHE_CONTROL);
        assert_eq!(first.text(), "contract: true");
        let second = server.get("/public-contracts/stripe-charges/yaml").await;
        assert_eq!(second.text(), "contract: true");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
