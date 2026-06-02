package io.datacontractgate.connect.smt.dlq;

import io.datacontractgate.connect.client.ViolationDetail;

import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Evaluates DLQ routing rules against a violation and returns the target topic.
 *
 * <h2>Rule evaluation</h2>
 * <p>Rules are evaluated top-to-bottom; the first rule whose {@code match} map
 * is a subset of the violation context wins.  If no rule matches, the default
 * topic is returned.</p>
 *
 * <h2>Severity mapping</h2>
 * <p>ContractGate violation {@code kind} values are mapped to a severity
 * string for the {@code "severity"} match field:</p>
 * <ul>
 *   <li>{@code "error"} — {@code missing_required_field}, {@code type_mismatch},
 *       {@code pattern_mismatch}, {@code enum_violation}, {@code range_violation},
 *       {@code length_violation}, {@code metric_range_violation}, {@code pii_leak}</li>
 *   <li>{@code "warn"} — {@code undeclared_field} and any unknown kind</li>
 * </ul>
 */
public class DlqRouter {

    private final List<DlqRule> rules;
    private final String defaultTopic;

    public DlqRouter(List<DlqRule> rules, String defaultTopic) {
        this.rules        = rules;
        this.defaultTopic = defaultTopic;
    }

    /**
     * Returns the DLQ topic for the given violation.
     *
     * @param violation    the first (worst) violation from the validation result
     * @param contractId   the contract UUID string (used for {@code contract} match field)
     * @return the target DLQ topic name; never {@code null}
     */
    public String route(ViolationDetail violation, String contractId) {
        Map<String, String> ctx = buildContext(violation, contractId);

        for (DlqRule rule : rules) {
            if (matches(rule.match, ctx)) {
                return rule.topic;
            }
        }
        return defaultTopic;
    }

    /**
     * Routes using all violations; delegates to the worst (first) violation
     * for topic selection.  If the list is empty, returns the default topic.
     */
    public String routeFirst(List<ViolationDetail> violations, String contractId) {
        if (violations == null || violations.isEmpty()) {
            return defaultTopic;
        }
        return route(violations.get(0), contractId);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /**
     * Builds the violation context map used for rule matching.
     *
     * <p>Keys: {@code severity}, {@code type}, {@code field}, {@code contract}.</p>
     */
    static Map<String, String> buildContext(ViolationDetail v, String contractId) {
        Map<String, String> ctx = new HashMap<>();
        ctx.put("severity", toSeverity(v.kind));
        if (v.kind       != null) ctx.put("type",     v.kind);
        if (v.field      != null) ctx.put("field",    v.field);
        if (contractId   != null) ctx.put("contract", contractId);
        return ctx;
    }

    /**
     * Returns {@code true} when all entries in {@code match} are present in
     * {@code ctx} with equal values.
     */
    static boolean matches(Map<String, String> match, Map<String, String> ctx) {
        for (Map.Entry<String, String> e : match.entrySet()) {
            if (!e.getValue().equals(ctx.get(e.getKey()))) {
                return false;
            }
        }
        return true;
    }

    /**
     * Maps a ContractGate violation kind to a severity string.
     *
     * <ul>
     *   <li>{@code "error"} — hard violations: missing/type/pattern/enum/range/
     *       length/metric/pii_leak</li>
     *   <li>{@code "warn"} — advisory: undeclared_field and any unknown kind</li>
     * </ul>
     */
    static String toSeverity(String kind) {
        if (kind == null) return "error";
        switch (kind) {
            case "missing_required_field":
            case "type_mismatch":
            case "pattern_mismatch":
            case "enum_violation":
            case "range_violation":
            case "length_violation":
            case "metric_range_violation":
            case "pii_leak":           // PII violations are hard errors
                return "error";
            default:
                return "warn";
        }
    }
}
