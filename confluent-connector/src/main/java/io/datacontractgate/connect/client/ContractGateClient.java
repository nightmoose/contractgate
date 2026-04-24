package io.datacontractgate.connect.client;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Duration;

/**
 * Thread-safe HTTP client for the ContractGate ingest API.
 *
 * <p>Uses the Java 11 built-in {@link java.net.http.HttpClient} — no extra
 * runtime dependencies. A single instance is safe to share across all Kafka
 * Connect worker threads.</p>
 *
 * <p>Every call sends exactly one record wrapped in a JSON array so the server
 * always returns exactly one {@link IngestResponse.IngestEventResult}. The
 * SMT retrieves it via {@link IngestResponse#singleResult()}.</p>
 */
public class ContractGateClient {

    private static final Logger log = LoggerFactory.getLogger(ContractGateClient.class);

    private final HttpClient httpClient;
    private final ObjectMapper mapper;

    private final String baseUrl;
    private final String contractId;
    private final String apiKey;
    private final String contractVersion; // empty string → let server resolve latest
    private final boolean dryRun;

    // ── Constructor ───────────────────────────────────────────────────────────

    /**
     * Creates a new client with the given configuration.
     *
     * @param baseUrl         ContractGate API base URL (no trailing slash)
     * @param contractId      UUID of the contract to validate against
     * @param apiKey          {@code x-api-key} header value; empty string skips the header
     * @param contractVersion specific version pin; empty string → latest stable
     * @param dryRun          when {@code true} the server validates without writing audit rows
     * @param connectTimeoutMs TCP connection timeout in milliseconds
     * @param requestTimeoutMs total request/response timeout in milliseconds
     */
    public ContractGateClient(
            String baseUrl,
            String contractId,
            String apiKey,
            String contractVersion,
            boolean dryRun,
            int connectTimeoutMs,
            int requestTimeoutMs) {

        this.baseUrl = baseUrl;
        this.contractId = contractId;
        this.apiKey = apiKey;
        this.contractVersion = contractVersion;
        this.dryRun = dryRun;

        this.mapper = new ObjectMapper();

        this.httpClient = HttpClient.newBuilder()
            .connectTimeout(Duration.ofMillis(connectTimeoutMs))
            .build();

        log.info("ContractGateClient initialised — url={} contract={} version={} dryRun={}",
            baseUrl, contractId, contractVersion.isEmpty() ? "latest" : contractVersion, dryRun);
    }

    // ── Public API ────────────────────────────────────────────────────────────

    /**
     * Validates {@code recordJson} against the configured contract.
     *
     * <p>The record is wrapped in a JSON array and posted to
     * {@code POST /ingest/{contractId}[?version=...][&dry_run=true]}.</p>
     *
     * @param recordJson JSON string representing a single Kafka record value
     * @param requestTimeoutMs per-call override (same value used at construction time)
     * @return parsed {@link IngestResponse}; never {@code null}
     * @throws ContractGateApiException on non-2xx HTTP status or I/O failure
     */
    public IngestResponse validate(String recordJson, int requestTimeoutMs)
            throws ContractGateApiException {

        String url = buildUrl();
        String body = wrapInArray(recordJson);

        HttpRequest.Builder requestBuilder = HttpRequest.newBuilder()
            .uri(URI.create(url))
            .timeout(Duration.ofMillis(requestTimeoutMs))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");

        if (!apiKey.isEmpty()) {
            requestBuilder.header("x-api-key", apiKey);
        }

        HttpRequest request = requestBuilder
            .POST(HttpRequest.BodyPublishers.ofString(body))
            .build();

        HttpResponse<String> response;
        try {
            response = httpClient.send(request, HttpResponse.BodyHandlers.ofString());
        } catch (IOException e) {
            throw new ContractGateApiException(
                "I/O error calling ContractGate at " + url + ": " + e.getMessage(), e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new ContractGateApiException(
                "Interrupted while calling ContractGate at " + url, e);
        }

        int status = response.statusCode();
        if (status < 200 || status >= 300) {
            throw new ContractGateApiException(
                "ContractGate returned HTTP " + status + " for contract " + contractId +
                ". Body: " + truncate(response.body(), 500));
        }

        try {
            return mapper.readValue(response.body(), IngestResponse.class);
        } catch (JsonProcessingException e) {
            throw new ContractGateApiException(
                "Failed to parse ContractGate response: " + e.getMessage() +
                ". Raw body: " + truncate(response.body(), 300), e);
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /**
     * Builds the full ingest URL, appending query parameters as configured.
     *
     * <p>Example outputs:
     * <ul>
     *   <li>{@code https://api.contractgate.io/ingest/abc-123}</li>
     *   <li>{@code https://api.contractgate.io/ingest/abc-123?version=1.2.0&dry_run=true}</li>
     * </ul>
     * </p>
     */
    private String buildUrl() {
        StringBuilder sb = new StringBuilder(baseUrl)
            .append("/ingest/")
            .append(contractId);

        boolean hasQuery = false;

        if (!contractVersion.isEmpty()) {
            sb.append("?version=").append(contractVersion);
            hasQuery = true;
        }
        if (dryRun) {
            sb.append(hasQuery ? "&" : "?").append("dry_run=true");
        }
        return sb.toString();
    }

    /**
     * Wraps a single JSON value in an array so the server's batch endpoint
     * returns exactly one {@link IngestResponse.IngestEventResult}.
     *
     * <p>If the record is already a JSON array we send it as-is (the server
     * treats array input as a batch). For all other values we wrap in {@code []}.</p>
     */
    private static String wrapInArray(String json) {
        String trimmed = json.trim();
        if (trimmed.startsWith("[")) {
            return trimmed;
        }
        return "[" + trimmed + "]";
    }

    /** Truncates a string to {@code maxLen} characters for safe log/exception messages. */
    private static String truncate(String s, int maxLen) {
        if (s == null) return "<null>";
        return s.length() <= maxLen ? s : s.substring(0, maxLen) + "…";
    }

    // ── Exception type ────────────────────────────────────────────────────────

    /**
     * Checked exception thrown when the ContractGate API cannot be reached or
     * returns an unexpected response. The calling SMT maps this to either a
     * {@code DataException} (for DLQ routing) or a log warning (for
     * TAG_AND_PASS with degraded-service semantics).
     */
    public static class ContractGateApiException extends Exception {
        public ContractGateApiException(String message) {
            super(message);
        }
        public ContractGateApiException(String message, Throwable cause) {
            super(message, cause);
        }
    }
}
