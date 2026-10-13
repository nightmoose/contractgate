//! Global response-security headers (RFC-100).
//!
//! Applied as the outermost middleware on `build_router` so the headers land
//! on every response — including 404s, 429s, CORS pre-flights, and
//! `/metrics` 401s. Before `44887e0` the Next.js edge set these; after the
//! API host went direct-to-Fly it was setting none.
//!
//! Headers written:
//!
//! | Header                      | Value |
//! |-----------------------------|-------|
//! | Strict-Transport-Security   | `max-age=31536000; includeSubDomains` (1 year, no preload) |
//! | X-Content-Type-Options      | `nosniff` |
//! | X-Frame-Options             | `DENY` (API never framed) |
//! | Content-Security-Policy     | `default-src 'none'; frame-ancestors 'none'` (API-only, no scripts) |
//! | Referrer-Policy             | `strict-origin-when-cross-origin` |
//!
//! Only inserted when the header isn't already present, so a handler can
//! override a specific response.

use axum::{
    extract::Request,
    http::header::{HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};

/// (name, value) pairs applied to every response.
const HEADERS: &[(HeaderName, &str)] = &[
    (
        axum::http::header::STRICT_TRANSPORT_SECURITY,
        "max-age=31536000; includeSubDomains",
    ),
    (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (axum::http::header::X_FRAME_OPTIONS, "DENY"),
    (
        axum::http::header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'",
    ),
    (
        axum::http::header::REFERRER_POLICY,
        "strict-origin-when-cross-origin",
    ),
];

/// Axum middleware. Runs the inner handler, then stamps the security headers
/// on the response if the handler didn't already set them.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    for (name, value) in HEADERS {
        if !headers.contains_key(name) {
            // HeaderValue::from_static is const-safe for our ASCII literals.
            headers.insert(name.clone(), HeaderValue::from_static(value));
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{middleware, response::IntoResponse, routing::get, Router};

    #[test]
    fn every_header_value_parses() {
        // Guard against a typo introducing an invalid header value, which
        // would panic at request time rather than at compile time.
        for (_, value) in HEADERS {
            HeaderValue::from_static(value);
        }
    }

    fn app() -> Router {
        Router::new()
            .route("/ping", get(|| async { "pong" }))
            .route(
                "/404ish",
                get(|| async { (axum::http::StatusCode::NOT_FOUND, "nope").into_response() }),
            )
            .layer(middleware::from_fn(security_headers))
    }

    const EXPECTED: &[(&str, &str)] = &[
        (
            "strict-transport-security",
            "max-age=31536000; includeSubDomains",
        ),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ),
        ("referrer-policy", "strict-origin-when-cross-origin"),
    ];

    #[tokio::test]
    async fn headers_on_200() {
        let server = axum_test::TestServer::new(app());
        let resp = server.get("/ping").await;
        assert_eq!(resp.status_code(), 200);
        for (name, want) in EXPECTED {
            let got = resp
                .headers()
                .get(*name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(got, *want, "header `{name}`");
        }
    }

    #[tokio::test]
    async fn headers_on_404() {
        // RFC-100: headers must land on non-200 responses too.
        let server = axum_test::TestServer::new(app());
        let resp = server.get("/404ish").await;
        assert_eq!(resp.status_code(), 404);
        for (name, want) in EXPECTED {
            let got = resp
                .headers()
                .get(*name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            assert_eq!(got, *want, "header `{name}` on 404");
        }
    }
}
