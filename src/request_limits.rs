//! Bounds on the waits that can stall a request without logging.
//!
//! Prod 2026-10-16 ~10:58–11:03 UTC: `POST .../promote` hung ~4.6 minutes,
//! applied nothing, and left no error, panic, or timeout line (the machine
//! did not reboot; the only nearby log was a JWKS refresh). Promote holds no
//! transaction and no advisory lock — one autocommit `UPDATE`. The shape is
//! an unbounded wait: pool checkout, `lock_timeout = 0`, or a stale socket
//! whose read sits in the kernel until TCP gives up.
//!
//! Each limit is inside the 30s request budget. When one fires, the log line
//! carries the request route and the stage that was in progress
//! (`pool_acquire`, `db_statement`, `lock_wait`, or `handler`).

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, PgPool};

/// Outer budget for every request except the demo SSE streams.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Checkout, including a bounded ping of an idle connection. Inside the HTTP budget
/// so a pool wait is attributed to `pool_acquire` instead of the outer timer.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(8);
/// Drop an idle connection whose ping does not return. A dead socket's ping
/// otherwise blocks until TCP retransmission (~minutes) with no error line.
pub const PING_TIMEOUT: Duration = Duration::from_secs(2);
/// Close idle connections before a pooler or NAT silently drops them.
/// sqlx's default is 10 minutes, which is long enough to hand out a dead socket.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
/// Server-side. A query that never reaches Postgres is caught by [`REQUEST_TIMEOUT`].
pub const STATEMENT_TIMEOUT: Duration = Duration::from_secs(15);
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
pub const IDLE_IN_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(15);

const STAGE_HANDLER: u8 = 0;
const STAGE_POOL_ACQUIRE: u8 = 1;
const STAGE_DB_STATEMENT: u8 = 2;

/// Routes that are allowed to stay open. Both are demo SSE streams with a 15s keepalive.
const UNBOUNDED_PATHS: &[&str] = &["/demo/stream", "/demo/events"];

tokio::task_local! {
    static STAGE: Arc<AtomicU8>;
    static REQUEST_ROUTE: Arc<str>;
}

pub fn stage_name(code: u8) -> &'static str {
    match code {
        STAGE_POOL_ACQUIRE => "pool_acquire",
        STAGE_DB_STATEMENT => "db_statement",
        _ => "handler",
    }
}

pub fn current_stage() -> &'static str {
    STAGE
        .try_with(|s| stage_name(s.load(Ordering::Relaxed)))
        .unwrap_or("handler")
}

pub fn current_route() -> String {
    REQUEST_ROUTE
        .try_with(|r| r.to_string())
        .unwrap_or_else(|_| "-".to_string())
}

fn set_stage(code: u8) {
    let _ = STAGE.try_with(|s| s.store(code, Ordering::Relaxed));
}

fn set_pool_acquire() {
    set_stage(STAGE_POOL_ACQUIRE);
}

fn set_db_statement() {
    set_stage(STAGE_DB_STATEMENT);
}

/// Why a database error is a timed-out wait rather than a 500.
pub struct DbTimeout {
    pub stage: &'static str,
    pub message: &'static str,
}

/// Pool checkout, `lock_timeout` (`55P03`), and `statement_timeout` (`57014`).
pub fn db_timeout(error: &sqlx::Error) -> Option<DbTimeout> {
    if matches!(error, sqlx::Error::PoolTimedOut) {
        return Some(DbTimeout {
            stage: "pool_acquire",
            message: "timed out acquiring a database connection",
        });
    }
    let code = error.as_database_error().and_then(|db| db.code());
    match code.as_deref() {
        Some("55P03") => Some(DbTimeout {
            stage: "lock_wait",
            message: "timed out waiting for a database lock",
        }),
        Some("57014") => Some(DbTimeout {
            stage: "db_statement",
            message: "database statement timed out",
        }),
        _ => None,
    }
}

pub fn is_unbounded_path(path: &str) -> bool {
    UNBOUNDED_PATHS.contains(&path)
}

/// Axum layer: 30s budget, 504 JSON, and an error log with route + stage.
pub async fn request_timeout(req: Request, next: Next) -> Response {
    enforce_request_timeout(REQUEST_TIMEOUT, req, next).await
}

pub(crate) async fn enforce_request_timeout(limit: Duration, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    if is_unbounded_path(&path) {
        return next.run(req).await;
    }

    let method = req.method().clone();
    let route: Arc<str> = Arc::from(format!("{method} {path}"));
    let stage = Arc::new(AtomicU8::new(STAGE_HANDLER));
    let slot = tracker().insert(route.to_string(), Arc::clone(&stage), limit);

    let run = REQUEST_ROUTE.scope(
        Arc::clone(&route),
        STAGE.scope(Arc::clone(&stage), next.run(req)),
    );
    match tokio::time::timeout(limit, run).await {
        Ok(response) => response,
        Err(_) => timeout_response(&slot.0),
    }
}

fn timeout_response(slot: &Inflight) -> Response {
    log_timeout(slot);
    let stage = stage_name(slot.stage.load(Ordering::Relaxed));
    let message = format!("request timed out during {stage}");
    (
        axum::http::StatusCode::GATEWAY_TIMEOUT,
        Json(serde_json::json!({
            "error": message,
            "status": 504,
            "stage": stage,
        })),
    )
        .into_response()
}

fn log_timeout(slot: &Inflight) {
    if slot.reported.swap(true, Ordering::AcqRel) {
        return;
    }
    tracing::error!(
        route = %slot.route,
        stage = stage_name(slot.stage.load(Ordering::Relaxed)),
        timeout_secs = slot.timeout.as_secs(),
        "request timed out"
    );
}

struct Inflight {
    route: String,
    stage: Arc<AtomicU8>,
    deadline: Instant,
    timeout: Duration,
    reported: AtomicBool,
}

struct InflightGuard(Arc<Inflight>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        let mut slots = tracker().slots.lock().unwrap_or_else(|e| e.into_inner());
        slots.retain(|slot| !Arc::ptr_eq(slot, &self.0));
    }
}

struct Tracker {
    slots: Mutex<Vec<Arc<Inflight>>>,
}

fn tracker() -> &'static Tracker {
    static TRACKER: OnceLock<Tracker> = OnceLock::new();
    TRACKER.get_or_init(|| {
        std::thread::Builder::new()
            .name("request-deadline".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(1));
                sweep();
            })
            .expect("spawn request-deadline watchdog");
        Tracker {
            slots: Mutex::new(Vec::new()),
        }
    })
}

fn sweep() {
    let slots = tracker().slots.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    for slot in slots.iter() {
        if now >= slot.deadline {
            log_timeout(slot);
        }
    }
}

impl Tracker {
    fn insert(&self, route: String, stage: Arc<AtomicU8>, timeout: Duration) -> InflightGuard {
        let slot = Arc::new(Inflight {
            route,
            stage,
            deadline: Instant::now() + timeout,
            timeout,
            reported: AtomicBool::new(false),
        });
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(Arc::clone(&slot));
        InflightGuard(slot)
    }
}

/// Pool used by the server and the one-shot CLI subcommands.
///
/// `test_before_acquire` is off: sqlx's built-in ping has no deadline of its
/// own, so a stale socket blocks the checkout until `acquire_timeout` (and,
/// if that timer is not polled, until TCP gives up). The `before_acquire`
/// hook pings with [`PING_TIMEOUT`] and discards the connection on expiry.
pub async fn connect_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    tracing::info!(
        acquire_timeout_secs = ACQUIRE_TIMEOUT.as_secs(),
        ping_timeout_secs = PING_TIMEOUT.as_secs(),
        idle_timeout_secs = IDLE_TIMEOUT.as_secs(),
        statement_timeout_secs = STATEMENT_TIMEOUT.as_secs(),
        lock_timeout_secs = LOCK_TIMEOUT.as_secs(),
        "postgres pool limits"
    );

    PgPoolOptions::new()
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .idle_timeout(IDLE_TIMEOUT)
        .max_lifetime(MAX_LIFETIME)
        .test_before_acquire(false)
        .before_acquire(|conn, _meta| {
            Box::pin(async move {
                set_pool_acquire();
                match tokio::time::timeout(PING_TIMEOUT, conn.ping()).await {
                    Ok(Ok(())) => {
                        set_db_statement();
                        Ok(true)
                    }
                    Ok(Err(error)) => {
                        tracing::warn!(
                            route = %current_route(),
                            stage = "pool_acquire",
                            %error,
                            "discarding database connection after failed ping"
                        );
                        Err(error)
                    }
                    Err(_) => {
                        tracing::warn!(
                            route = %current_route(),
                            stage = "pool_acquire",
                            timeout_ms = PING_TIMEOUT.as_millis() as u64,
                            "discarding stale database connection: ping timed out"
                        );
                        Ok(false)
                    }
                }
            })
        })
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                set_pool_acquire();
                // Session settings, so they survive for the life of this sqlx
                // connection (session pooler or direct). Startup `options=-c`
                // is not used: a transaction-mode pooler rejects that packet.
                let sql = format!(
                    "SET statement_timeout = '{}ms'; \
                     SET lock_timeout = '{}ms'; \
                     SET idle_in_transaction_session_timeout = '{}ms'",
                    STATEMENT_TIMEOUT.as_millis(),
                    LOCK_TIMEOUT.as_millis(),
                    IDLE_IN_TRANSACTION_TIMEOUT.as_millis(),
                );
                Executor::execute(&mut *conn, sql.as_str()).await?;
                set_db_statement();
                Ok(())
            })
        })
        .connect(database_url)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use std::borrow::Cow;
    use tower::ServiceExt;

    #[test]
    fn timeouts_sit_inside_the_request_budget() {
        assert!(PING_TIMEOUT < ACQUIRE_TIMEOUT);
        assert!(ACQUIRE_TIMEOUT < REQUEST_TIMEOUT);
        assert!(LOCK_TIMEOUT < STATEMENT_TIMEOUT);
        assert!(STATEMENT_TIMEOUT < REQUEST_TIMEOUT);
        assert!(IDLE_IN_TRANSACTION_TIMEOUT <= STATEMENT_TIMEOUT);
        assert!(IDLE_TIMEOUT < Duration::from_secs(10 * 60));
    }

    #[test]
    fn db_timeout_names_the_wait() {
        assert_eq!(
            db_timeout(&sqlx::Error::PoolTimedOut).unwrap().stage,
            "pool_acquire"
        );
        assert_eq!(db_timeout(&db_err("55P03")).unwrap().stage, "lock_wait");
        assert_eq!(db_timeout(&db_err("57014")).unwrap().stage, "db_statement");
        assert!(db_timeout(&db_err("23505")).is_none());
    }

    #[derive(Debug)]
    struct FakeDb(&'static str);

    impl std::fmt::Display for FakeDb {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("fake")
        }
    }

    impl std::error::Error for FakeDb {}

    impl sqlx::error::DatabaseError for FakeDb {
        fn message(&self) -> &str {
            "fake"
        }
        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.0))
        }
        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
    }

    fn db_err(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDb(code)))
    }

    fn router(limit: Duration) -> Router {
        Router::new()
            .route(
                "/contracts/{id}/versions/{version}/promote",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    StatusCode::OK
                }),
            )
            .route(
                "/slow-db",
                get(|| async {
                    super::set_db_statement();
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    StatusCode::OK
                }),
            )
            .route(
                "/demo/stream",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    StatusCode::OK
                }),
            )
            .route("/fast", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn(move |req, next| {
                let limit = limit;
                async move { enforce_request_timeout(limit, req, next).await }
            }))
    }

    async fn call(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        let response = app
            .oneshot(Request::get(path).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, json)
    }

    #[tokio::test]
    async fn slow_handler_returns_504_for_the_handler_stage() {
        let (status, body) = call(
            router(Duration::from_millis(40)),
            "/contracts/abc/versions/1.0.0/promote",
        )
        .await;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body["status"], 504);
        assert_eq!(body["stage"], "handler");
        assert_eq!(body["error"], "request timed out during handler");
    }

    #[tokio::test]
    async fn timeout_reports_the_db_statement_stage() {
        let (status, body) = call(router(Duration::from_millis(40)), "/slow-db").await;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(body["stage"], "db_statement");
        assert_eq!(body["error"], "request timed out during db_statement");
    }

    #[tokio::test]
    async fn demo_stream_is_not_cut_at_the_request_budget() {
        let (status, _) = call(router(Duration::from_millis(40)), "/demo/stream").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn fast_handler_is_unchanged() {
        let (status, _) = call(router(Duration::from_millis(40)), "/fast").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[test]
    fn timeout_log_names_route_and_stage_once() {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let make = {
            let buf = Arc::clone(&buf);
            move || Capture(Arc::clone(&buf))
        };
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::ERROR)
            .with_ansi(false)
            .with_writer(make)
            .without_time()
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let slot = Inflight {
                route: "POST /contracts/x/versions/1.0.0/promote".into(),
                stage: Arc::new(AtomicU8::new(STAGE_DB_STATEMENT)),
                deadline: Instant::now(),
                timeout: Duration::from_secs(30),
                reported: AtomicBool::new(false),
            };
            log_timeout(&slot);
            log_timeout(&slot);
        });
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            text.contains("POST /contracts/x/versions/1.0.0/promote"),
            "{text}"
        );
        assert!(text.contains("db_statement"), "{text}");
        assert_eq!(text.matches("request timed out").count(), 1, "{text}");
    }

    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
