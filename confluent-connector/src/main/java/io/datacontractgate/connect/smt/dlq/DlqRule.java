package io.datacontractgate.connect.smt.dlq;

import java.util.Map;
import java.util.Objects;

/**
 * A single DLQ routing rule parsed from {@code contractgate.dlq.routing.rules}.
 *
 * <p>A rule matches when ALL keys in {@code match} match the corresponding
 * fields of the violation context.  Evaluation is top-to-bottom; first match
 * wins.</p>
 *
 * <h2>Match fields</h2>
 * <ul>
 *   <li>{@code severity} — {@code "error"} or {@code "warn"}.  ContractGate
 *       maps {@code kind} values to severity: missing/type/pattern/enum/range
 *       /length/metric violations are {@code "error"}; advisory violations
 *       (undeclared_field) are {@code "warn"}.</li>
 *   <li>{@code type} — the violation {@code kind} string from the gateway
 *       response, e.g. {@code "enum_violation"}, {@code "missing_required_field"}.</li>
 *   <li>{@code field} — the field path that violated, e.g. {@code "amount"}.</li>
 *   <li>{@code contract} — the contract UUID string (for future multi-contract
 *       setups).</li>
 * </ul>
 */
public final class DlqRule {

    /**
     * Map of field-name → expected-value.  All entries must match for the rule
     * to fire.
     */
    public final Map<String, String> match;

    /** The Kafka topic to route the failing record to when this rule fires. */
    public final String topic;

    public DlqRule(Map<String, String> match, String topic) {
        this.match = Objects.requireNonNull(match, "match");
        this.topic = Objects.requireNonNull(topic, "topic");
        if (topic.isBlank()) {
            throw new IllegalArgumentException("DlqRule topic must not be blank");
        }
    }

    @Override
    public String toString() {
        return "DlqRule{match=" + match + ", topic='" + topic + "'}";
    }
}
