package io.datacontractgate.connect.smt.dlq;

import com.fasterxml.jackson.core.type.TypeReference;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.apache.kafka.common.config.ConfigException;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * Parses and validates the DLQ routing configuration from connector props.
 *
 * <p>Reads:</p>
 * <ul>
 *   <li>{@code contractgate.dlq.routing.rules} — JSON array of
 *       {@code {"match": {...}, "topic": "..."}} objects.</li>
 *   <li>{@code contractgate.dlq.routing.default} — fallback topic (required
 *       when routing is enabled).</li>
 *   <li>{@code contractgate.dlq.routing.producer.bootstrap.servers} —
 *       required when routing is enabled; passed to the internal
 *       {@link KafkaDlqProducer}.</li>
 * </ul>
 *
 * <h2>Why a dedicated producer (not errantRecordReporter)?</h2>
 * <p>Kafka Connect's {@code ErrantRecordReporter} interface (3.6.0) routes all
 * errors to the single {@code errors.deadletterqueue.topic.name} configured on
 * the connector.  It does not support per-record topic override.  Therefore,
 * per-violation routing requires the SMT to open a dedicated
 * {@link KafkaDlqProducer} directly — a well-established pattern in third-party
 * SMTs (e.g. Debezium outbox, Lenses CQRS SMTs).</p>
 */
public class DlqRoutingConfig {

    private static final ObjectMapper MAPPER = new ObjectMapper();

    private final List<DlqRule> rules;
    private final String defaultTopic;
    private final String bootstrapServers;

    private DlqRoutingConfig(List<DlqRule> rules, String defaultTopic, String bootstrapServers) {
        this.rules            = rules;
        this.defaultTopic     = defaultTopic;
        this.bootstrapServers = bootstrapServers;
    }

    /**
     * Parses the DLQ routing config from raw connector props.
     *
     * @param rulesJson          raw JSON string from {@code contractgate.dlq.routing.rules}
     * @param defaultTopic       value of {@code contractgate.dlq.routing.default}
     * @param bootstrapServers   value of {@code contractgate.dlq.routing.producer.bootstrap.servers}
     * @throws ConfigException on malformed JSON or missing required fields
     */
    public static DlqRoutingConfig parse(
            String rulesJson,
            String defaultTopic,
            String bootstrapServers) throws ConfigException {

        if (defaultTopic == null || defaultTopic.isBlank()) {
            throw new ConfigException(
                "contractgate.dlq.routing.default is required when " +
                "contractgate.dlq.routing.enabled=true");
        }
        if (bootstrapServers == null || bootstrapServers.isBlank()) {
            throw new ConfigException(
                "contractgate.dlq.routing.producer.bootstrap.servers is required when " +
                "contractgate.dlq.routing.enabled=true");
        }

        List<DlqRule> rules = parseRules(rulesJson == null ? "[]" : rulesJson.trim());
        return new DlqRoutingConfig(rules, defaultTopic.trim(), bootstrapServers.trim());
    }

    /** Returns the ordered list of routing rules (may be empty). */
    public List<DlqRule> rules() { return rules; }

    /** Returns the fallback DLQ topic applied when no rule matches. */
    public String defaultTopic() { return defaultTopic; }

    /** Returns the {@code bootstrap.servers} for the internal DLQ producer. */
    public String bootstrapServers() { return bootstrapServers; }

    // ── Parsing helpers ───────────────────────────────────────────────────────

    /**
     * Parses the JSON array of rule objects.  Each object must have
     * {@code "match"} (object) and {@code "topic"} (string).
     *
     * <p>Example:
     * <pre>{@code
     * [
     *   {"match": {"severity": "error", "type": "pii_leak"}, "topic": "audit.pii_failures"},
     *   {"match": {"severity": "error"},                     "topic": "dlq.errors"},
     *   {"match": {"severity": "warn"},                      "topic": "dlq.warnings"}
     * ]
     * }</pre>
     * </p>
     */
    @SuppressWarnings("unchecked")
    private static List<DlqRule> parseRules(String json) throws ConfigException {
        List<Map<String, Object>> rawList;
        try {
            rawList = MAPPER.readValue(json, new TypeReference<List<Map<String, Object>>>() {});
        } catch (IOException e) {
            throw new ConfigException(
                "contractgate.dlq.routing.rules must be a valid JSON array: " + e.getMessage());
        }

        List<DlqRule> rules = new ArrayList<>(rawList.size());
        for (int i = 0; i < rawList.size(); i++) {
            Map<String, Object> raw = rawList.get(i);
            Object matchObj = raw.get("match");
            Object topicObj = raw.get("topic");

            if (!(matchObj instanceof Map)) {
                throw new ConfigException(
                    "Rule[" + i + "].match must be a JSON object");
            }
            if (!(topicObj instanceof String)) {
                throw new ConfigException(
                    "Rule[" + i + "].topic must be a string");
            }

            Map<String, Object> rawMatch = (Map<String, Object>) matchObj;
            Map<String, String> match = new java.util.LinkedHashMap<>();
            for (Map.Entry<String, Object> e : rawMatch.entrySet()) {
                if (!(e.getValue() instanceof String)) {
                    throw new ConfigException(
                        "Rule[" + i + "].match." + e.getKey() + " must be a string value");
                }
                match.put(e.getKey(), (String) e.getValue());
            }

            try {
                rules.add(new DlqRule(match, (String) topicObj));
            } catch (IllegalArgumentException e) {
                throw new ConfigException("Rule[" + i + "]: " + e.getMessage());
            }
        }
        return rules;
    }
}
