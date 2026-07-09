package io.datacontractgate.connect.smt.reload;

import org.apache.kafka.connect.errors.ConnectException;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;

/**
 * Background polling thread that detects contract version changes and notifies
 * the SMT to swap its cached contract reference.
 *
 * <h2>Lifecycle</h2>
 * <ol>
 *   <li>{@link #start()} — called from
 *       {@code ContractGateValidator.configure()} when
 *       {@code contractgate.reload.enabled=true}.</li>
 *   <li>Polling loop runs every {@code pollMs} milliseconds.</li>
 *   <li>{@link #stop()} — called from {@code ContractGateValidator.close()}.</li>
 * </ol>
 *
 * <h2>Reload behaviour</h2>
 * <ol>
 *   <li>Poll {@code GET /v1/contracts/{id}/version} → {@link ContractVersionInfo}.</li>
 *   <li>If hash matches the cached hash: no-op.</li>
 *   <li>If hash changed: fetch the new YAML body via
 *       {@link ContractVersionCheck#fetchContractYaml(String)} to verify it
 *       is non-empty and well-formed (basic sanity check — full YAML parse
 *       happens server-side).</li>
 *   <li>On success: call {@code onReload} callback with the new
 *       {@link ContractVersionInfo}, increment success counter,
 *       log INFO with old → new version.</li>
 *   <li>On any error: keep old version, increment failure counter, log WARN.
 *       Never swaps to a broken or unverified contract.</li>
 * </ol>
 *
 * <h2>Failure action</h2>
 * <p>The {@code failureAction} parameter controls what happens when fetching
 * or parsing the new version fails:</p>
 * <ul>
 *   <li>{@code warn} (default) — log WARN, keep old contract, increment
 *       failure counter.</li>
 *   <li>{@code fail-task} — throw {@link ConnectException} which propagates
 *       out of the scheduled task and through {@code apply()} on the next
 *       call, causing the Kafka Connect task to fail. Use when contract
 *       integrity is a hard requirement and stale-contract processing is
 *       worse than task downtime.</li>
 * </ul>
 *
 * <h2>Metrics (exported as AtomicLong counters for external scraping)</h2>
 * <ul>
 *   <li>{@link #reloadSuccessCount}</li>
 *   <li>{@link #reloadFailureCount}</li>
 * </ul>
 * These are intentionally simple — Connect's own metrics framework can wrap
 * them in a future RFC.
 */
public class DynamicContractReloader {

    private static final Logger log = LoggerFactory.getLogger(DynamicContractReloader.class);

    // ── Metrics (contractgate.reload.*) ───────────────────────────────────────

    /** Counter incremented on each successful contract swap. */
    public final AtomicLong reloadSuccessCount = new AtomicLong(0);

    /** Counter incremented on each failed reload attempt. */
    public final AtomicLong reloadFailureCount = new AtomicLong(0);

    // ── State ─────────────────────────────────────────────────────────────────

    private final ContractVersionCheck versionCheck;
    private final long pollMs;
    private final String failureAction;
    private final AtomicReference<ContractVersionInfo> currentVersion;
    private final Consumer<ContractVersionInfo> onReload;

    private volatile ConnectException pendingTaskFailure = null;

    private ScheduledExecutorService scheduler;
    private ScheduledFuture<?> pollTask;

    /**
     * Creates a new reloader.
     *
     * @param versionCheck     the HTTP client for polling and body fetch
     * @param pollMs           polling interval in milliseconds (min 5000)
     * @param failureAction    {@code "warn"} or {@code "fail-task"}
     * @param initialVersion   the version info known at configure-time
     *                         (may be {@code null} if the version hasn't been
     *                         probed yet — first poll will set it)
     * @param onReload         callback invoked (on the polling thread) with the
     *                         new {@link ContractVersionInfo} after a successful
     *                         swap.  Must be thread-safe.
     */
    public DynamicContractReloader(
            ContractVersionCheck versionCheck,
            long pollMs,
            String failureAction,
            ContractVersionInfo initialVersion,
            Consumer<ContractVersionInfo> onReload) {
        this.versionCheck   = versionCheck;
        this.pollMs         = pollMs;
        this.failureAction  = failureAction;
        this.currentVersion = new AtomicReference<>(initialVersion);
        this.onReload       = onReload;
    }

    /** Start the background polling thread.  Idempotent. */
    public synchronized void start() {
        if (scheduler != null && !scheduler.isShutdown()) {
            return; // already running
        }
        scheduler = Executors.newSingleThreadScheduledExecutor(r -> {
            Thread t = new Thread(r, "contractgate-reload-poller");
            t.setDaemon(true);
            return t;
        });
        pollTask = scheduler.scheduleAtFixedRate(
            this::poll, pollMs, pollMs, TimeUnit.MILLISECONDS);
        log.info("DynamicContractReloader started — poll.ms={}", pollMs);
    }

    /** Stop the background polling thread.  Safe to call multiple times. */
    public synchronized void stop() {
        if (scheduler != null) {
            scheduler.shutdownNow();
            scheduler = null;
            log.info("DynamicContractReloader stopped");
        }
    }

    /**
     * Returns the most recently confirmed {@link ContractVersionInfo}, or
     * {@code null} if the first probe has not yet succeeded.
     */
    public ContractVersionInfo currentVersion() {
        return currentVersion.get();
    }

    /**
     * If the {@code fail-task} action was triggered by a background poll,
     * this method re-throws the recorded exception so {@code apply()} can
     * surface it to Kafka Connect and fail the task.
     *
     * <p>Call at the top of {@code ContractGateValidator.apply()} when
     * {@code failureAction = fail-task}.</p>
     */
    public void rethrowPendingFailureIfAny() {
        ConnectException e = pendingTaskFailure;
        if (e != null) {
            throw e;
        }
    }

    // ── Polling loop ──────────────────────────────────────────────────────────

    private void poll() {
        try {
            ContractVersionInfo probe = versionCheck.fetchCurrentVersion();
            ContractVersionInfo known = currentVersion.get();

            // No change — fast path.
            if (known != null && known.hash.equals(probe.hash)) {
                log.trace("Contract version unchanged (hash={}…)", abbrev(probe.hash));
                return;
            }

            // Hash changed — fetch body to validate before swapping.
            String yaml = versionCheck.fetchContractYaml(probe.version);
            if (yaml == null || yaml.isBlank()) {
                throw new ContractVersionCheck.ContractCheckException(
                    "Fetched contract body was empty for version " + probe.version);
            }

            // Basic sanity: check the YAML is plausibly a contract
            // (starts with YAML content, not an error JSON).
            if (yaml.trim().startsWith("{") || yaml.length() < 10) {
                throw new ContractVersionCheck.ContractCheckException(
                    "Fetched contract body looks invalid for version " + probe.version);
            }

            // Commit the swap.
            currentVersion.set(probe);
            reloadSuccessCount.incrementAndGet();
            log.info("Contract reloaded — old={} new={} hash={}…",
                known != null ? known.version : "<none>",
                probe.version,
                abbrev(probe.hash));

            // Notify the SMT.
            onReload.accept(probe);

        } catch (ContractVersionCheck.ContractCheckException e) {
            reloadFailureCount.incrementAndGet();
            log.warn("Contract reload failed: {} (keeping existing contract; failure#={})",
                e.getMessage(), reloadFailureCount.get());

            if ("fail-task".equalsIgnoreCase(failureAction)) {
                pendingTaskFailure = new ConnectException(
                    "contractgate.reload.failure.action=fail-task triggered: " + e.getMessage(), e);
                log.error("fail-task action set — SMT task will fail on next apply() call");
            }
        } catch (Exception e) {
            // Unexpected runtime exception — treat same as ContractCheckException.
            reloadFailureCount.incrementAndGet();
            log.warn("Unexpected error in contract reload poll: {}", e.getMessage(), e);
        }
    }

    /** Returns first 8 chars of hash for log output, or the full hash if shorter. */
    private static String abbrev(String hash) {
        return hash.length() > 8 ? hash.substring(0, 8) : hash;
    }
}
