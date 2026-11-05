//! Bounded, batched background-write queue for the ingest/egress hot paths.
//!
//! The ingest handler used to `tokio::spawn` one untracked task per request for
//! each of its audit, quarantine, and forward writes — up to three new tasks on
//! every call.  Under load that is unbounded concurrency: nothing caps how many
//! writer tasks (and therefore DB connections) pile up when Postgres is slow,
//! and a graceful shutdown cannot wait for in-flight writes it does not track.
//!
//! This module replaces that with a single bounded queue:
//!
//!   * one [`tokio::sync::mpsc`] channel with a fixed capacity,
//!   * one worker task that pulls jobs, coalesces them by kind, and writes in
//!     batches (flushing when it has `batch_max` jobs or `flush_interval`
//!     elapses — whichever comes first),
//!   * [`WriteQueue::submit`] uses `try_send`, so a full queue drops the job and
//!     bumps `contractgate_write_queue_dropped_total{kind}` instead of blocking
//!     the request, and
//!   * [`WriteQueue::shutdown`] drains everything still queued within a bounded
//!     timeout so an orderly restart does not silently lose durable writes.
//!
//! The DB side sits behind the [`WriteSink`] trait so the batching, drop, and
//! drain behaviour can be unit-tested without a Postgres connection.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::storage::{AuditEntryInsert, ForwardEventInsert, QuarantineEventInsert};

/// A single durable-write request handed to the background worker.
///
/// Rows are already batched per HTTP request; the worker coalesces jobs of the
/// same kind across requests into one DB round-trip.
#[derive(Debug)]
pub enum WriteJob {
    /// Audit-log rows plus the usage-metering follow-up for the originating org.
    Audit {
        rows: Vec<AuditEntryInsert>,
        store_payloads: bool,
        meter_org: Option<Uuid>,
    },
    /// Quarantine rows for failing events.
    Quarantine {
        rows: Vec<QuarantineEventInsert>,
        store_payloads: bool,
    },
    /// Forward rows for passing events.
    Forward {
        rows: Vec<ForwardEventInsert>,
        store_payloads: bool,
    },
}

impl WriteJob {
    /// Stable metric/log label for this job kind.
    pub fn kind(&self) -> &'static str {
        match self {
            WriteJob::Audit { .. } => "audit",
            WriteJob::Quarantine { .. } => "quarantine",
            WriteJob::Forward { .. } => "forward",
        }
    }
}

/// Where the worker actually persists batches.  Abstracted so tests can observe
/// batching/drain without a database.
pub trait WriteSink: Send + Sync + 'static {
    fn flush_audit(
        &self,
        rows: Vec<AuditEntryInsert>,
        store_payloads: bool,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn flush_quarantine(
        &self,
        rows: Vec<QuarantineEventInsert>,
        store_payloads: bool,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn flush_forward(
        &self,
        rows: Vec<ForwardEventInsert>,
        store_payloads: bool,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn meter(
        &self,
        org: Option<Uuid>,
        event_count: usize,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Production sink: writes to Postgres via the existing storage batch helpers.
struct PgSink {
    pool: PgPool,
}

impl WriteSink for PgSink {
    async fn flush_audit(&self, rows: Vec<AuditEntryInsert>, store_payloads: bool) {
        if let Err(e) =
            crate::storage::log_audit_entries_batch(&self.pool, &rows, store_payloads).await
        {
            tracing::warn!(
                "write queue: audit write failed ({} rows): {e:?}",
                rows.len()
            );
        }
    }

    async fn flush_quarantine(&self, rows: Vec<QuarantineEventInsert>, store_payloads: bool) {
        if let Err(e) =
            crate::storage::quarantine_events_batch(&self.pool, &rows, store_payloads).await
        {
            tracing::warn!(
                "write queue: quarantine write failed ({} rows): {e:?}",
                rows.len()
            );
        }
    }

    async fn flush_forward(&self, rows: Vec<ForwardEventInsert>, store_payloads: bool) {
        if let Err(e) =
            crate::storage::forward_events_batch(&self.pool, &rows, store_payloads).await
        {
            tracing::warn!(
                "write queue: forward write failed ({} rows): {e:?}",
                rows.len()
            );
        }
    }

    async fn meter(&self, org: Option<Uuid>, event_count: usize) {
        crate::metering::record_batch_usage_blocking(&self.pool, org, event_count).await;
    }
}

/// Tunables for the queue.  Defaults are sized for the ingest hot path.
#[derive(Clone, Copy, Debug)]
pub struct QueueConfig {
    /// Bounded channel capacity.  `submit` drops once this many jobs are queued.
    pub capacity: usize,
    /// Flush once this many jobs are buffered, without waiting for the timer.
    pub batch_max: usize,
    /// Flush a partially-filled buffer after this long.
    pub flush_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig {
            capacity: 1024,
            batch_max: 64,
            flush_interval: Duration::from_millis(50),
        }
    }
}

/// Handle stored in `AppState`.  Cheap to hold; cloning is not needed because it
/// is reached through `Arc<AppState>`.
pub struct WriteQueue {
    tx: mpsc::Sender<WriteJob>,
    shutdown: Arc<Notify>,
    worker: Mutex<Option<JoinHandle<()>>>,
    dropped: Arc<AtomicU64>,
}

impl WriteQueue {
    /// Build a queue backed by Postgres and spawn its worker.  Must be called
    /// from within a Tokio runtime (it is — `AppState::new` runs under `main`).
    pub fn new(pool: PgPool) -> Self {
        Self::with_sink(PgSink { pool }, QueueConfig::default())
    }

    fn with_sink<S: WriteSink>(sink: S, cfg: QueueConfig) -> Self {
        let (tx, rx) = mpsc::channel(cfg.capacity);
        let shutdown = Arc::new(Notify::new());
        let dropped = Arc::new(AtomicU64::new(0));
        let worker = tokio::spawn(run_worker(rx, Arc::clone(&shutdown), sink, cfg));
        WriteQueue {
            tx,
            shutdown,
            worker: Mutex::new(Some(worker)),
            dropped,
        }
    }

    /// Enqueue a write.  Never blocks: a full queue drops the job and records
    /// the loss on `contractgate_write_queue_dropped_total{kind}`.
    pub fn submit(&self, job: WriteJob) {
        let kind = job.kind();
        if self.tx.try_send(job).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            metrics::counter!(
                "contractgate_write_queue_dropped_total",
                "kind" => kind,
            )
            .increment(1);
            tracing::warn!(kind, "write queue full; dropping background {kind} write");
        }
    }

    /// Total jobs dropped since boot (mirrors the Prometheus counter; used in
    /// tests and for cheap introspection).
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Signal the worker to flush everything still queued and exit, waiting at
    /// most `timeout` for the drain to complete.  Called from the graceful
    /// shutdown path in `main`.
    pub async fn shutdown(&self, timeout: Duration) {
        self.shutdown.notify_one();
        let handle = self.worker.lock().unwrap().take();
        if let Some(handle) = handle {
            if tokio::time::timeout(timeout, handle).await.is_err() {
                tracing::warn!(
                    "write queue drain exceeded {:?}; some background writes may be lost",
                    timeout
                );
            }
        }
    }
}

/// Coalesce a buffer of jobs into the fewest DB calls and apply them.
///
/// Rows are grouped by `store_payloads` because the storage helpers apply that
/// flag uniformly to a batch; usage metering stays per-originating-request so
/// each org is credited the right event count.
async fn flush<S: WriteSink>(sink: &S, jobs: Vec<WriteJob>) {
    if jobs.is_empty() {
        return;
    }

    use std::collections::HashMap;
    let mut audit: HashMap<bool, Vec<AuditEntryInsert>> = HashMap::new();
    let mut quarantine: HashMap<bool, Vec<QuarantineEventInsert>> = HashMap::new();
    let mut forward: HashMap<bool, Vec<ForwardEventInsert>> = HashMap::new();
    let mut meter: Vec<(Option<Uuid>, usize)> = Vec::new();

    for job in jobs {
        match job {
            WriteJob::Audit {
                rows,
                store_payloads,
                meter_org,
            } => {
                let n = rows.len();
                if n > 0 {
                    meter.push((meter_org, n));
                    audit.entry(store_payloads).or_default().extend(rows);
                }
            }
            WriteJob::Quarantine {
                rows,
                store_payloads,
            } => {
                if !rows.is_empty() {
                    quarantine.entry(store_payloads).or_default().extend(rows);
                }
            }
            WriteJob::Forward {
                rows,
                store_payloads,
            } => {
                if !rows.is_empty() {
                    forward.entry(store_payloads).or_default().extend(rows);
                }
            }
        }
    }

    for (store_payloads, rows) in audit {
        sink.flush_audit(rows, store_payloads).await;
    }
    // Metering follows the audit write, mirroring the original per-request order.
    // Under-counts (e.g. an audit write that failed above) are repaired by the
    // periodic usage-reconcile task, so decoupling them here is safe.
    for (org, n) in meter {
        sink.meter(org, n).await;
    }
    for (store_payloads, rows) in quarantine {
        sink.flush_quarantine(rows, store_payloads).await;
    }
    for (store_payloads, rows) in forward {
        sink.flush_forward(rows, store_payloads).await;
    }
}

/// Drain everything currently queued (non-blocking) and flush it once.
async fn drain_remaining<S: WriteSink>(rx: &mut mpsc::Receiver<WriteJob>, sink: &S) {
    let mut buf = Vec::new();
    while let Ok(job) = rx.try_recv() {
        buf.push(job);
    }
    flush(sink, buf).await;
}

async fn run_worker<S: WriteSink>(
    mut rx: mpsc::Receiver<WriteJob>,
    shutdown: Arc<Notify>,
    sink: S,
    cfg: QueueConfig,
) {
    loop {
        // Block until the first job arrives, shutdown is signalled, or every
        // sender is dropped (process teardown).
        let first = tokio::select! {
            biased;
            _ = shutdown.notified() => {
                drain_remaining(&mut rx, &sink).await;
                return;
            }
            maybe = rx.recv() => match maybe {
                Some(job) => job,
                None => return,
            }
        };

        // Accumulate a batch, bounded by size or time.
        let mut buf = vec![first];
        let deadline = tokio::time::Instant::now() + cfg.flush_interval;
        while buf.len() < cfg.batch_max {
            tokio::select! {
                biased;
                _ = shutdown.notified() => {
                    while let Ok(job) = rx.try_recv() {
                        buf.push(job);
                    }
                    flush(&sink, buf).await;
                    drain_remaining(&mut rx, &sink).await;
                    return;
                }
                maybe = rx.recv() => match maybe {
                    Some(job) => buf.push(job),
                    None => {
                        flush(&sink, buf).await;
                        return;
                    }
                },
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        flush(&sink, buf).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AuditEntryInsert;
    use crate::transform::TransformedPayload;
    use std::sync::Mutex as StdMutex;

    /// Records every sink call so tests can assert on batching/drain.
    #[derive(Default)]
    struct RecordingSink {
        audit_batches: StdMutex<Vec<usize>>,
        quarantine_batches: StdMutex<Vec<usize>>,
        forward_batches: StdMutex<Vec<usize>>,
        meter_calls: StdMutex<Vec<(Option<Uuid>, usize)>>,
    }

    impl WriteSink for Arc<RecordingSink> {
        async fn flush_audit(&self, rows: Vec<AuditEntryInsert>, _store_payloads: bool) {
            self.audit_batches.lock().unwrap().push(rows.len());
        }
        async fn flush_quarantine(&self, rows: Vec<QuarantineEventInsert>, _store_payloads: bool) {
            self.quarantine_batches.lock().unwrap().push(rows.len());
        }
        async fn flush_forward(&self, rows: Vec<ForwardEventInsert>, _store_payloads: bool) {
            self.forward_batches.lock().unwrap().push(rows.len());
        }
        async fn meter(&self, org: Option<Uuid>, event_count: usize) {
            self.meter_calls.lock().unwrap().push((org, event_count));
        }
    }

    fn audit_row() -> AuditEntryInsert {
        AuditEntryInsert {
            contract_id: Uuid::nil(),
            org_id: None,
            contract_version: "1.0.0".to_string(),
            passed: true,
            violation_count: 0,
            violation_details: serde_json::Value::Array(vec![]),
            raw_event: TransformedPayload::from_stored(serde_json::json!({})),
            validation_us: 0,
            source_ip: None,
            source: "http".to_string(),
            pre_assigned_id: None,
            replay_of_quarantine_id: None,
            direction: "ingress".to_string(),
        }
    }

    fn audit_job(rows: usize, org: Option<Uuid>) -> WriteJob {
        WriteJob::Audit {
            rows: (0..rows).map(|_| audit_row()).collect(),
            store_payloads: true,
            meter_org: org,
        }
    }

    #[tokio::test]
    async fn flush_coalesces_jobs_by_kind() {
        let sink = Arc::new(RecordingSink::default());
        let org = Uuid::new_v4();
        let jobs = vec![audit_job(2, Some(org)), audit_job(3, Some(org))];
        flush(&Arc::clone(&sink), jobs).await;

        // Two audit jobs → one coalesced batch of 5 rows (same store_payloads).
        assert_eq!(*sink.audit_batches.lock().unwrap(), vec![5]);
        // Metering stays per-request: one call each.
        assert_eq!(
            *sink.meter_calls.lock().unwrap(),
            vec![(Some(org), 2), (Some(org), 3)]
        );
    }

    #[tokio::test]
    async fn flush_separates_distinct_store_payloads() {
        let sink = Arc::new(RecordingSink::default());
        let jobs = vec![
            WriteJob::Audit {
                rows: vec![audit_row()],
                store_payloads: true,
                meter_org: None,
            },
            WriteJob::Audit {
                rows: vec![audit_row(), audit_row()],
                store_payloads: false,
                meter_org: None,
            },
        ];
        flush(&Arc::clone(&sink), jobs).await;

        let mut batches = sink.audit_batches.lock().unwrap().clone();
        batches.sort_unstable();
        // Different store_payloads flags cannot share a batch.
        assert_eq!(batches, vec![1, 2]);
    }

    #[tokio::test]
    async fn submit_drops_and_counts_when_full() {
        let sink = Arc::new(RecordingSink::default());
        // Capacity 1, and a batch_max high enough that the worker parks on the
        // first recv long enough for the channel to back up.
        let cfg = QueueConfig {
            capacity: 1,
            batch_max: 64,
            flush_interval: Duration::from_secs(3600),
        };
        let q = WriteQueue::with_sink(Arc::clone(&sink), cfg);

        // Hammer the queue well past capacity. At least some sends must be
        // rejected, and every rejection must be counted.
        for _ in 0..200 {
            q.submit(audit_job(1, None));
        }
        assert!(
            q.dropped_count() > 0,
            "expected drops once the bounded queue filled"
        );
    }

    #[tokio::test]
    async fn shutdown_drains_queued_jobs() {
        let sink = Arc::new(RecordingSink::default());
        // Large flush interval so nothing flushes on its own — only the drain on
        // shutdown should move these rows.
        let cfg = QueueConfig {
            capacity: 256,
            batch_max: 1000,
            flush_interval: Duration::from_secs(3600),
        };
        let q = WriteQueue::with_sink(Arc::clone(&sink), cfg);

        for _ in 0..10 {
            q.submit(audit_job(1, None));
        }
        q.shutdown(Duration::from_secs(5)).await;

        let total: usize = sink.audit_batches.lock().unwrap().iter().sum();
        assert_eq!(total, 10, "all queued rows must be flushed on drain");
    }
}
