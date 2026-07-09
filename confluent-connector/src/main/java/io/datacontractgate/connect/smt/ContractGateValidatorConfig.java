package io.datacontractgate.connect.smt;

import org.apache.kafka.common.config.AbstractConfig;
import org.apache.kafka.common.config.ConfigDef;
import org.apache.kafka.common.config.ConfigDef.Importance;
import org.apache.kafka.common.config.ConfigDef.Type;
import org.apache.kafka.common.config.ConfigException;

import java.util.Map;

/**
 * Configuration for {@link ContractGateValidator}.
 *
 * <p>All keys are namespaced under {@code contractgate.*} to avoid clashes with
 * other SMTs in a connector chain.</p>
 *
 * <h2>RFC-064 additions</h2>
 * <ul>
 *   <li>Dynamic contract reload: {@code contractgate.reload.*}</li>
 *   <li>Per-violation DLQ routing: {@code contractgate.dlq.routing.*}</li>
 * </ul>
 * Both features are opt-in via {@code *.enabled=false} defaults.
 * With both disabled, behavior is byte-identical to the pre-RFC-064 baseline.
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

    // ── RFC-064: Dynamic contract reload ──────────────────────────────────────

    public static final String RELOAD_ENABLED_CONFIG = "contractgate.reload.enabled";
    private static final String RELOAD_ENABLED_DOC =
        "Enable dynamic contract reload via polling. When false (default), the contract " +
        "reference is fixed at task start — same behaviour as before RFC-064. " +
        "When true, a background thread polls GET /v1/contracts/{id}/version every " +
        "contractgate.reload.poll.ms milliseconds and hot-swaps the contract on change.";
    private static final boolean RELOAD_ENABLED_DEFAULT = false;

    public static final String RELOAD_POLL_MS_CONFIG = "contractgate.reload.poll.ms";
    private static final String RELOAD_POLL_MS_DOC =
        "How often the background reloader polls the ContractGate gateway for a " +
        "contract version change, in milliseconds. Minimum 5000. Default: 30000.";
    private static final int RELOAD_POLL_MS_DEFAULT = 30_000;
    private static final int RELOAD_POLL_MS_MIN     = 5_000;

    public static final String RELOAD_FAILURE_ACTION_CONFIG = "contractgate.reload.failure.action";
    private static final String RELOAD_FAILURE_ACTION_DOC =
        "What to do when a contract reload fails (e.g. server unreachable, unparseable YAML).\n" +
        "  warn      — Log WARN, keep old contract (default). The SMT continues " +
                        "processing with the last good version.\n" +
        "  fail-task — Mark the Connect task as failed. Use when running with a " +
                        "stale contract is worse than downtime (strict compliance scenarios).";
    private static final String RELOAD_FAILURE_ACTION_DEFAULT = "warn";

    // ── RFC-064: Per-violation DLQ routing ───────────────────────────────────

    public static final String DLQ_ROUTING_ENABLED_CONFIG = "contractgate.dlq.routing.enabled";
    private static final String DLQ_ROUTING_ENABLED_DOC =
        "Enable per-violation DLQ routing. When false (default), all violations are " +
        "routed to the single errors.deadletterqueue.topic.name configured on the connector. " +
        "When true, violations are routed according to contractgate.dlq.routing.rules.";
    private static final boolean DLQ_ROUTING_ENABLED_DEFAULT = false;

    public static final String DLQ_ROUTING_RULES_CONFIG = "contractgate.dlq.routing.rules";
    private static final String DLQ_ROUTING_RULES_DOC =
        "JSON array of routing rules evaluated top-to-bottom (first match wins). " +
        "Each rule: {\"match\": {\"severity\": \"error\", \"type\": \"pii_leak\"}, " +
        "\"topic\": \"audit.pii_failures\"}. " +
        "Match fields: severity (error|warn), type (violation kind), " +
        "field (field path), contract (contract UUID). " +
        "Default: [] (all violations go to contractgate.dlq.routing.default).";
    private static final String DLQ_ROUTING_RULES_DEFAULT = "[]";

    public static final String DLQ_ROUTING_DEFAULT_CONFIG = "contractgate.dlq.routing.default";
    private static final String DLQ_ROUTING_DEFAULT_DOC =
        "Fallback DLQ topic when no routing rule matches. " +
        "Required when contractgate.dlq.routing.enabled=true.";
    private static final String DLQ_ROUTING_DEFAULT_DEFAULT = "";

    public static final String DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_CONFIG =
        "contractgate.dlq.routing.producer.bootstrap.servers";
    private static final String DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_DOC =
        "bootstrap.servers for the internal DLQ routing producer. " +
        "Required when contractgate.dlq.routing.enabled=true. " +
        "Use the same value as the Connect worker's bootstrap.servers. " +
        "Additional producer settings can be passed as " +
        "contractgate.dlq.routing.producer.<key>=<value>.";
    private static final String DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_DEFAULT = "";

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
                Importance.LOW, MAX_VIOLATION_HEADERS_DOC)
        // RFC-064: Dynamic contract reload
        .define(RELOAD_ENABLED_CONFIG,
                Type.BOOLEAN, RELOAD_ENABLED_DEFAULT,
                Importance.MEDIUM, RELOAD_ENABLED_DOC)
        .define(RELOAD_POLL_MS_CONFIG,
                Type.INT, RELOAD_POLL_MS_DEFAULT,
                ConfigDef.Range.atLeast(RELOAD_POLL_MS_MIN),
                Importance.LOW, RELOAD_POLL_MS_DOC)
        .define(RELOAD_FAILURE_ACTION_CONFIG,
                Type.STRING, RELOAD_FAILURE_ACTION_DEFAULT,
                ConfigDef.ValidString.in("warn", "fail-task"),
                Importance.LOW, RELOAD_FAILURE_ACTION_DOC)
        // RFC-064: Per-violation DLQ routing
        .define(DLQ_ROUTING_ENABLED_CONFIG,
                Type.BOOLEAN, DLQ_ROUTING_ENABLED_DEFAULT,
                Importance.MEDIUM, DLQ_ROUTING_ENABLED_DOC)
        .define(DLQ_ROUTING_RULES_CONFIG,
                Type.STRING, DLQ_ROUTING_RULES_DEFAULT,
                Importance.MEDIUM, DLQ_ROUTING_RULES_DOC)
        .define(DLQ_ROUTING_DEFAULT_CONFIG,
                Type.STRING, DLQ_ROUTING_DEFAULT_DEFAULT,
                Importance.MEDIUM, DLQ_ROUTING_DEFAULT_DOC)
        .define(DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_CONFIG,
                Type.STRING, DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_DEFAULT,
                Importance.MEDIUM, DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_DOC);

    // ── Constructor ───────────────────────────────────────────────────────────

    public ContractGateValidatorConfig(Map<String, ?> props) {
        super(CONFIG_DEF, props);
        validate();
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

    // RFC-064 accessors

    public boolean reloadEnabled() {
        return getBoolean(RELOAD_ENABLED_CONFIG);
    }

    public int reloadPollMs() {
        return getInt(RELOAD_POLL_MS_CONFIG);
    }

    public String reloadFailureAction() {
        return getString(RELOAD_FAILURE_ACTION_CONFIG);
    }

    public boolean dlqRoutingEnabled() {
        return getBoolean(DLQ_ROUTING_ENABLED_CONFIG);
    }

    public String dlqRoutingRules() {
        return getString(DLQ_ROUTING_RULES_CONFIG);
    }

    public String dlqRoutingDefault() {
        return getString(DLQ_ROUTING_DEFAULT_CONFIG).trim();
    }

    public String dlqRoutingProducerBootstrapServers() {
        return getString(DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_CONFIG).trim();
    }

    // ── Cross-field validation ────────────────────────────────────────────────

    private void validate() {
        // dlq.routing: validate that required fields are present when enabled.
        // Full JSON parse is deferred to DlqRoutingConfig.parse() in configure().
        if (dlqRoutingEnabled()) {
            if (dlqRoutingDefault().isEmpty()) {
                throw new ConfigException(DLQ_ROUTING_DEFAULT_CONFIG,
                    "required when contractgate.dlq.routing.enabled=true");
            }
            if (dlqRoutingProducerBootstrapServers().isEmpty()) {
                throw new ConfigException(DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_CONFIG,
                    "required when contractgate.dlq.routing.enabled=true");
            }
        }
    }
}
