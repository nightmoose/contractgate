package io.datacontractgate.connect.smt.reload;

import com.fasterxml.jackson.annotation.JsonIgnoreProperties;
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
 * Makes HTTP calls to the ContractGate gateway to check the current contract
 * version and fetch the contract body.
 *
 * <h2>Endpoints used</h2>
 * <ul>
 *   <li>{@code GET /v1/contracts/{id}/version} — returns
 *       {@code {version: string, hash: string}}.  Cheap poll used on every
 *       tick; body fetch is triggered only on hash change.</li>
 *   <li>{@code GET /contracts/{id}/versions/{version}} — returns the full
 *       {@code VersionResponse}.  Called only when the hash changes to verify
 *       the new version is well-formed before committing the swap.</li>
 * </ul>
 *
 * <p>Thread-safe; the underlying {@link HttpClient} is shared and can be
 * called concurrently from the background reloader thread.</p>
 */
public class ContractVersionCheck {

    private static final Logger log = LoggerFactory.getLogger(ContractVersionCheck.class);

    /** Jackson type for the version probe response. */
    @JsonIgnoreProperties(ignoreUnknown = true)
    static class VersionProbeResponse {
        public String version;
        public String hash;
    }

    /** Jackson type for the version body response (minimal fields needed). */
    @JsonIgnoreProperties(ignoreUnknown = true)
    static class VersionBodyResponse {
        public String yaml_content;
        public String version;
    }

    private final HttpClient httpClient;
    private final ObjectMapper mapper;
    private final String baseUrl;
    private final String contractId;
    private final String apiKey;
    private final int requestTimeoutMs;

    public ContractVersionCheck(
            String baseUrl,
            String contractId,
            String apiKey,
            int connectTimeoutMs,
            int requestTimeoutMs) {
        this.baseUrl = baseUrl;
        this.contractId = contractId;
        this.apiKey = apiKey;
        this.requestTimeoutMs = requestTimeoutMs;
        this.mapper = new ObjectMapper();
        this.httpClient = HttpClient.newBuilder()
            .connectTimeout(Duration.ofMillis(connectTimeoutMs))
            .build();
    }

    /**
     * Polls {@code GET /v1/contracts/{id}/version} and returns the current
     * version info.
     *
     * @throws ContractCheckException on HTTP error or parse failure
     */
    public ContractVersionInfo fetchCurrentVersion() throws ContractCheckException {
        String url = baseUrl + "/v1/contracts/" + contractId + "/version";
        HttpRequest req = buildGet(url);

        HttpResponse<String> resp = send(req, url);
        checkStatus(resp, url);

        try {
            VersionProbeResponse body = mapper.readValue(resp.body(), VersionProbeResponse.class);
            if (body.version == null || body.hash == null) {
                throw new ContractCheckException(
                    "Version probe returned null version or hash from " + url);
            }
            return new ContractVersionInfo(body.version, body.hash);
        } catch (IOException e) {
            throw new ContractCheckException(
                "Failed to parse version probe response from " + url + ": " + e.getMessage(), e);
        }
    }

    /**
     * Fetches and returns the YAML body for a specific contract version.
     * Called only when {@link #fetchCurrentVersion()} reports a changed hash.
     *
     * @param version the version string to fetch (e.g. {@code "2.1.0"})
     * @throws ContractCheckException on HTTP error or parse failure
     */
    public String fetchContractYaml(String version) throws ContractCheckException {
        String url = baseUrl + "/contracts/" + contractId + "/versions/" + version;
        HttpRequest req = buildGet(url);

        HttpResponse<String> resp = send(req, url);
        checkStatus(resp, url);

        try {
            VersionBodyResponse body = mapper.readValue(resp.body(), VersionBodyResponse.class);
            if (body.yaml_content == null || body.yaml_content.isBlank()) {
                throw new ContractCheckException(
                    "Version body response had no yaml_content from " + url);
            }
            return body.yaml_content;
        } catch (IOException e) {
            throw new ContractCheckException(
                "Failed to parse version body response from " + url + ": " + e.getMessage(), e);
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private HttpRequest buildGet(String url) {
        HttpRequest.Builder b = HttpRequest.newBuilder()
            .uri(URI.create(url))
            .timeout(Duration.ofMillis(requestTimeoutMs))
            .GET()
            .header("Accept", "application/json");
        if (apiKey != null && !apiKey.isEmpty()) {
            b.header("x-api-key", apiKey);
        }
        return b.build();
    }

    private HttpResponse<String> send(HttpRequest req, String url)
            throws ContractCheckException {
        try {
            return httpClient.send(req, HttpResponse.BodyHandlers.ofString());
        } catch (IOException e) {
            throw new ContractCheckException(
                "I/O error calling " + url + ": " + e.getMessage(), e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new ContractCheckException("Interrupted while calling " + url, e);
        }
    }

    private void checkStatus(HttpResponse<String> resp, String url)
            throws ContractCheckException {
        int status = resp.statusCode();
        if (status < 200 || status >= 300) {
            throw new ContractCheckException(
                "HTTP " + status + " from " + url + ": " + truncate(resp.body(), 200));
        }
    }

    private static String truncate(String s, int max) {
        if (s == null) return "<null>";
        return s.length() <= max ? s : s.substring(0, max) + "…";
    }

    // ── Exception ─────────────────────────────────────────────────────────────

    public static class ContractCheckException extends Exception {
        public ContractCheckException(String message) { super(message); }
        public ContractCheckException(String message, Throwable cause) { super(message, cause); }
    }
}
