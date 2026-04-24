package io.datacontractgate.connect.smt;

import org.apache.kafka.common.config.AbstractConfig;
import org.apache.kafka.common.config.ConfigDef;
import org.apache.kafka.common.config.ConfigDef.Importance;
import org.apache.kafka.common.config.ConfigDef.Type;

import java.util.Map;

/**
 * Configuration for {@link ContractGateValidator}.
 *
 * <p>All keys are namespaced under {@code contractgate.*} to avoid clashes with
 * other SMTs in a connector chain.</p>
 */
public class ContractGateValidatorConfig extends AbstractConfig {

    // ── Required ─────────────────────────────────────────────────────────────

    public static final String API_URL_CONFIG = "contractgate.api.url";
    private static final String API_URL_DOC =
        "Base URL of the ContractGate API server, e.g. https://contractgate-api.fly.dev. " +
        "No trailing slash.";

    public static final String CONTRACT_ID_CONFIG = "contractgate.contract.id";
    private static final String CONTRACT_ID_DOC =
        "UUID of the ContractGate contract to validate records against. " +
        "Obtain from the ContractGate dashboard or GET /contracts.";

    // ── Authentication ────────────────────────────────────────────────────────

    public static final String API_KEY_CONFIG = "contractgate.api.key";
    private static final String API_KEY_DOC =
        "x-api-key header value for the ContractGate API. " +
        "Leave blank only when the server runs without authentication (dev mode).";
    private static final String API_KEY_DEFAULT = "";

    // ── Contract version ─────────────────────────────────────────────────────

    public static final String CONTRACT_VERSION_CONFIG = "contractgate.contract.version";
    private static final String CONTRACT_VERSION_DOC =
        "Specific contract version to pin (e.g. '1.2.0'). " +
        "When blank the server resolves to the latest stable version automatically " +
        "(recommended — lets you promote new versions without redeploying connectors).";
    private static final String CONTRACT_VERSION_DEFAULT = "";

    // ── Validation mode ──────────────────────────────────────────────────────

    public static final String DRY_RUN_CONFIG = "contractgate.dry.run";
    private static final String DRY_RUN_DOC =
        "When true, validation results are NOT written to the ContractGate audit log " +
        "or quarantine store. Useful for high-throughput pipelines where you want " +
        "enforcement without DB write pressure. Default: false (full audit trail).";
    private static final boolean DRY_RUN_DEFAULT = false;

    // ── Failure action ────────────────────────────────────────────────────────

    public static final String ON_FAILURE_CONFIG = "contractgate.on.failure";
    private static final String ON_FAILURE_DOC =
        "What to do when a record fails validation.\n" +
        "  DLQ          — Throw a DataException so Kafka Connect routes the record to the " +
                          "dead-letter topic configured on the connector " +
                          "(errors.deadletterqueue.topic.name). Recommended.\n" +
        "  TAG_AND_PASS — Add violation headers and continue downstream. Records are " +
                          "never dropped; consumers decide what to do.";
    private static final String ON_FAILURE_DEFAULT = "DLQ";

    public enum OnFailure { DLQ, TAG_AND_PASS }

    // ── HTTP timeouts ─────────────────────────────────────────────────────────

    public static final String CONNECT_TIMEOUT_MS_CONFIG = "contractgate.connect.timeout.ms";
    private static final String CONNECT_TIMEOUT_MS_DOC =
        "TCP connection timeout to the ContractGate API in milliseconds. Default: 5000.";
    private static final int CONNECT_TIMEOUT_MS_DEFAULT = 5_000;

    public static final String REQUEST_TIMEOUT_MS_CONFIG = "contractgate.request.timeout.ms";
    private static final String REQUEST_TIMEOUT_MS_DOC =
        "Total HTTP request/response timeout in milliseconds. " +
        "Should be well below the Kafka Connect task timeout. Default: 10000.";
    private static final int REQUEST_TIMEOUT_MS_DEFAULT = 10_000;

    // ── Headers ───────────────────────────────────────────────────────────────

    public static final String ADD_RESULT_HEADERS_CONFIG = "contractgate.add.result.headers";
    private static final String ADD_RESULT_HEADERS_DOC =
        "When true, adds ContractGate result metadata as record headers on every record " +
        "(pass or fail). Headers: contractgate.passed, contractgate.contract.version, " +
        "contractgate.violations.count, and contractgate.violation.N.field/kind/message " +
        "for the first few violations. Default: true.";
    private static final boolean ADD_RESULT_HEADERS_DEFAULT = true;

    public static final String MAX_VIOLATION_HEADERS_CONFIG = "contractgate.max.violation.headers";
    private static final String MAX_VIOLATION_HEADERS_DOC =
        "Maximum number of individual violations to include as record headers. " +
        "High violation counts can bloat headers; cap at a useful number. Default: 5.";
    private static final int MAX_VIOLATION_HEADERS_DEFAULT = 5;

    // ── Config definition ─────────────────────────────────────────────────────

    public static final ConfigDef CONFIG_DEF = new ConfigDef()
        // Required
        .define(API_URL_CONFIG,
                Type.STRING, ConfigDef.NO_DEFAULT_VALUE,
                Importance.HIGH, API_URL_DOC)
        .define(CONTRACT_ID_CONFIG,
                Type.STRING, ConfigDef.NO_DEFAULT_VALUE,
                Importance.HIGH, CONTRACT_ID_DOC)
        // Auth
        .define(API_KEY_CONFIG,
                Type.PASSWORD, API_KEY_DEFAULT,
                Importance.HIGH, API_KEY_DOC)
        // Version
        .define(CONTRACT_VERSION_CONFIG,
                Type.STRING, CONTRACT_VERSION_DEFAULT,
                Importance.MEDIUM, CONTRACT_VERSION_DOC)
        // Validation mode
        .define(DRY_RUN_CONFIG,
                Type.BOOLEAN, DRY_RUN_DEFAULT,
                Importance.MEDIUM, DRY_RUN_DOC)
        // Failure action
        .define(ON_FAILURE_CONFIG,
                Type.STRING, ON_FAILURE_DEFAULT,
                ConfigDef.ValidString.in("DLQ", "TAG_AND_PASS"),
                Importance.HIGH, ON_FAILURE_DOC)
        // Timeouts
        .define(CONNECT_TIMEOUT_MS_CONFIG,
                Type.INT, CONNECT_TIMEOUT_MS_DEFAULT,
                ConfigDef.Range.atLeast(100),
                Importance.LOW, CONNECT_TIMEOUT_MS_DOC)
        .define(REQUEST_TIMEOUT_MS_CONFIG,
                Type.INT, REQUEST_TIMEOUT_MS_DEFAULT,
                ConfigDef.Range.atLeast(100),
                Importance.LOW, REQUEST_TIMEOUT_MS_DOC)
        // Headers
        .define(ADD_RESULT_HEADERS_CONFIG,
                Type.BOOLEAN, ADD_RESULT_HEADERS_DEFAULT,
                Importance.LOW, ADD_RESULT_HEADERS_DOC)
        .define(MAX_VIOLATION_HEADERS_CONFIG,
                Type.INT, MAX_VIOLATION_HEADERS_DEFAULT,
                ConfigDef.Range.between(0, 50),
                Importance.LOW, MAX_VIOLATION_HEADERS_DOC);

    // ── Constructor ───────────────────────────────────────────────────────────

    public ContractGateValidatorConfig(Map<String, ?> props) {
        super(CONFIG_DEF, props);
    }

    // ── Typed accessors ───────────────────────────────────────────────────────

    public String apiUrl() {
        return getString(API_URL_CONFIG).replaceAll("/+$", ""); // strip trailing slashes
    }

    public String contractId() {
        return getString(CONTRACT_ID_CONFIG).trim();
    }

    public String apiKey() {
        return getPassword(API_KEY_CONFIG).value();
    }

    public String contractVersion() {
        return getString(CONTRACT_VERSION_CONFIG).trim();
    }

    public boolean dryRun() {
        return getBoolean(DRY_RUN_CONFIG);
    }

    public OnFailure onFailure() {
        return OnFailure.valueOf(getString(ON_FAILURE_CONFIG));
    }

    public int connectTimeoutMs() {
        return getInt(CONNECT_TIMEOUT_MS_CONFIG);
    }

    public int requestTimeoutMs() {
        return getInt(REQUEST_TIMEOUT_MS_CONFIG);
    }

    public boolean addResultHeaders() {
        return getBoolean(ADD_RESULT_HEADERS_CONFIG);
    }

    public int maxViolationHeaders() {
        return getInt(MAX_VIOLATION_HEADERS_CONFIG);
    }
}
