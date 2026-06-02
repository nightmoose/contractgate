package io.datacontractgate.connect.client;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
import com.fasterxml.jackson.annotation.JsonProperty;

import java.util.Collections;
import java.util.List;

/**
 * Top-level response from {@code POST /ingest/{contractId}}.
 *
 * <p>Mirrors the {@code BatchIngestResponse} struct in the Rust backend.
 * Unknown fields are silently ignored so older connector versions remain
 * compatible with newer server responses.</p>
 */
@JsonIgnoreProperties(ignoreUnknown = true)
public class IngestResponse {

    /** Total number of events in the request batch (always 1 for the SMT). */
    public int total;

    /** Number of events that passed validation. */
    public int passed;

    /** Number of events that failed validation. */
    public int failed;

    /**
     * Whether the request was processed in dry-run mode.
     * When {@code true} the server validated but did not write to the audit log.
     */
    @JsonProperty("dry_run")
    public boolean dryRun;

    /**
     * Contract version that was resolved and used for validation.
     * Either the pinned version supplied by the caller or the latest stable
     * version resolved by the server.
     */
    @JsonProperty("resolved_version")
    public String resolvedVersion;

    /**
     * How the contract version was determined: {@code "pinned"}, {@code "latest"}, etc.
     * Informational — used for header enrichment.
     */
    @JsonProperty("version_pin_source")
    public String versionPinSource;

    /**
     * Per-event results. For single-event SMT calls this list always has
     * exactly one entry.
     */
    public List<IngestEventResult> results = Collections.emptyList();

    // ── Convenience helpers ───────────────────────────────────────────────────

    /**
     * Returns the single {@link IngestEventResult} for the record the SMT sent.
     * Throws {@link IllegalStateException} if the server returned an unexpected
     * number of results (should never happen in normal operation).
     */
    public IngestEventResult singleResult() {
        if (results == null || results.isEmpty()) {
            throw new IllegalStateException(
                "ContractGate returned no results for a single-record ingest call");
        }
        return results.get(0);
    }

    // ── Inner class ───────────────────────────────────────────────────────────

    /**
     * Validation result for one event within a batch.
     * Mirrors {@code IngestEventResult} in the Rust backend.
     */
    @JsonIgnoreProperties(ignoreUnknown = true)
    public static class IngestEventResult {

        /** {@code true} if the event passed all contract rules. */
        public boolean passed;

        /** Ordered list of rule violations; empty when {@link #passed} is {@code true}. */
        public List<ViolationDetail> violations = Collections.emptyList();

        /**
         * Wall-clock validation time in microseconds as measured by the server.
         * Useful for latency budgeting and SLA tracking.
         */
        @JsonProperty("validation_us")
        public long validationUs;

        /** Contract version that produced this result (may differ per event in mixed batches). */
        @JsonProperty("contract_version")
        public String contractVersion;

        // ── Convenience helpers ───────────────────────────────────────────

        /**
         * Returns a compact, single-line summary for logging and DLQ exception messages.
         * Example: {@code "3 violation(s): field [kind]: message; ..."}
         */
        public String violationSummary() {
            if (violations == null || violations.isEmpty()) {
                return "no violations";
            }
            StringBuilder sb = new StringBuilder();
            sb.append(violations.size()).append(" violation(s): ");
            for (int i = 0; i < violations.size(); i++) {
                if (i > 0) sb.append("; ");
                sb.append(violations.get(i));
            }
            return sb.toString();
        }
    }
}
