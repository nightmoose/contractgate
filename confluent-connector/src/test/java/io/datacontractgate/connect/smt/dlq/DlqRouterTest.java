package io.datacontractgate.connect.smt.dlq;

import io.datacontractgate.connect.client.ViolationDetail;
import org.apache.kafka.common.config.ConfigException;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Nested;
import org.junit.jupiter.api.Test;

import java.util.List;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Unit tests for {@link DlqRouter} and {@link DlqRoutingConfig}.
 *
 * <p>Tests rule matching logic, severity mapping, top-to-bottom evaluation,
 * default fallback, and config parsing error cases.</p>
 */
class DlqRouterTest {

    // ── Helpers ───────────────────────────────────────────────────────────────

    private static ViolationDetail violation(String kind, String field) {
        ViolationDetail v = new ViolationDetail();
        v.kind  = kind;
        v.field = field;
        return v;
    }

    private static DlqRouter router(String rulesJson, String defaultTopic) {
        DlqRoutingConfig cfg = DlqRoutingConfig.parse(rulesJson, defaultTopic, "localhost:9092");
        return new DlqRouter(cfg.rules(), cfg.defaultTopic());
    }

    // ── Severity mapping ──────────────────────────────────────────────────────

    @Nested
    @DisplayName("Severity mapping")
    class SeverityMappingTests {

        @Test
        @DisplayName("Hard violation kinds map to 'error'")
        void hardViolationsMapToError() {
            for (String kind : List.of(
                    "missing_required_field", "type_mismatch", "pattern_mismatch",
                    "enum_violation", "range_violation", "length_violation",
                    "metric_range_violation")) {
                assertThat(DlqRouter.toSeverity(kind))
                    .as("Expected severity 'error' for kind: " + kind)
                    .isEqualTo("error");
            }
        }

        @Test
        @DisplayName("undeclared_field maps to 'warn'")
        void undeclaredFieldMapsToWarn() {
            assertThat(DlqRouter.toSeverity("undeclared_field")).isEqualTo("warn");
        }

        @Test
        @DisplayName("Unknown kind maps to 'warn'")
        void unknownKindMapsToWarn() {
            assertThat(DlqRouter.toSeverity("future_unknown_kind")).isEqualTo("warn");
        }

        @Test
        @DisplayName("Null kind maps to 'error'")
        void nullKindMapsToError() {
            assertThat(DlqRouter.toSeverity(null)).isEqualTo("error");
        }
    }

    // ── Rule matching ─────────────────────────────────────────────────────────

    @Nested
    @DisplayName("Rule matching")
    class RuleMatchingTests {

        @Test
        @DisplayName("Exact severity match routes to correct topic")
        void severityMatchRoutesToCorrectTopic() {
            // Two severity rules
            String rules =
                "[{\"match\":{\"severity\":\"error\"},\"topic\":\"dlq.errors\"}," +
                 "{\"match\":{\"severity\":\"warn\"}, \"topic\":\"dlq.warnings\"}]";
            DlqRouter r = router(rules, "dlq.fallback");

            ViolationDetail errorViolation = violation("missing_required_field", "user_id");
            ViolationDetail warnViolation  = violation("undeclared_field", "extra");

            assertThat(r.route(errorViolation, "contract-uuid")).isEqualTo("dlq.errors");
            assertThat(r.route(warnViolation,  "contract-uuid")).isEqualTo("dlq.warnings");
        }

        @Test
        @DisplayName("Top-to-bottom evaluation: first match wins")
        void firstMatchWins() {
            // More specific rule before less specific — specific rule must win
            String rules =
                "[{\"match\":{\"severity\":\"error\",\"type\":\"missing_required_field\"},\"topic\":\"dlq.missing\"}," +
                 "{\"match\":{\"severity\":\"error\"},\"topic\":\"dlq.errors\"}]";
            DlqRouter r = router(rules, "dlq.fallback");

            ViolationDetail missing = violation("missing_required_field", "user_id");
            ViolationDetail range   = violation("range_violation", "amount");

            assertThat(r.route(missing, "c")).isEqualTo("dlq.missing");
            assertThat(r.route(range,   "c")).isEqualTo("dlq.errors");
        }

        @Test
        @DisplayName("Default topic returned when no rule matches")
        void defaultTopicReturnedOnNoMatch() {
            String rules = "[{\"match\":{\"severity\":\"warn\"},\"topic\":\"dlq.warnings\"}]";
            DlqRouter r = router(rules, "dlq.fallback");
            ViolationDetail error = violation("enum_violation", "status");
            assertThat(r.route(error, "c")).isEqualTo("dlq.fallback");
        }

        @Test
        @DisplayName("Multi-field match: all conditions must be satisfied")
        void multiFieldMatchAllConditions() {
            String rules = "[{\"match\":{\"severity\":\"error\",\"field\":\"amount\"},\"topic\":\"dlq.amount\"}]";
            DlqRouter r = router(rules, "dlq.fallback");

            ViolationDetail amountError = violation("range_violation", "amount");
            ViolationDetail otherError  = violation("range_violation", "price");

            assertThat(r.route(amountError, "c")).isEqualTo("dlq.amount");
            assertThat(r.route(otherError,  "c")).isEqualTo("dlq.fallback");
        }

        @Test
        @DisplayName("Contract field match")
        void contractFieldMatch() {
            String rules = "[{\"match\":{\"contract\":\"my-contract-uuid\"},\"topic\":\"dlq.mycontract\"}]";
            DlqRouter r = router(rules, "dlq.fallback");

            ViolationDetail v = violation("enum_violation", "type");
            assertThat(r.route(v, "my-contract-uuid")).isEqualTo("dlq.mycontract");
            assertThat(r.route(v, "other-uuid")).isEqualTo("dlq.fallback");
        }

        @Test
        @DisplayName("Empty rules list always returns default topic")
        void emptyRulesReturnsDefault() {
            DlqRouter r = router("[]", "dlq.default");
            ViolationDetail v = violation("missing_required_field", "id");
            assertThat(r.route(v, "c")).isEqualTo("dlq.default");
        }

        @Test
        @DisplayName("routeFirst with empty list returns default")
        void routeFirstEmptyList() {
            DlqRouter r = router("[]", "dlq.default");
            assertThat(r.routeFirst(List.of(), "c")).isEqualTo("dlq.default");
            assertThat(r.routeFirst(null, "c")).isEqualTo("dlq.default");
        }
    }

    // ── 3-rule routing (RFC-064 acceptance criterion AC#3) ────────────────────

    @Nested
    @DisplayName("3-rule routing (AC#3)")
    class ThreeRuleRoutingTests {

        // Compile-time constant — allowed in a non-static nested class in Java 11
        private static final String THREE_RULES =
            "[{\"match\":{\"severity\":\"error\",\"type\":\"pii_leak\"},\"topic\":\"audit.pii_failures\"}," +
             "{\"match\":{\"severity\":\"error\"},\"topic\":\"dlq.errors\"}," +
             "{\"match\":{\"severity\":\"warn\"}, \"topic\":\"dlq.warnings\"}]";

        @Test
        @DisplayName("3 violations route to 3 different topics")
        void threeViolationsThreeTopics() {
            DlqRouter r = router(THREE_RULES, "dlq.fallback");

            ViolationDetail piiLeak      = violation("pii_leak",              "email");
            ViolationDetail otherError   = violation("missing_required_field", "user_id");
            ViolationDetail warnViolation = violation("undeclared_field",      "extra");

            assertThat(r.route(piiLeak,       "c")).isEqualTo("audit.pii_failures");
            assertThat(r.route(otherError,    "c")).isEqualTo("dlq.errors");
            assertThat(r.route(warnViolation, "c")).isEqualTo("dlq.warnings");
        }
    }

    // ── Config parsing error cases ────────────────────────────────────────────

    @Nested
    @DisplayName("DlqRoutingConfig parsing")
    class ConfigParsingTests {

        @Test
        @DisplayName("Malformed JSON raises ConfigException")
        void malformedJsonRaisesConfigException() {
            assertThatThrownBy(() ->
                DlqRoutingConfig.parse("{not-valid-json", "dlq.default", "localhost:9092"))
                .isInstanceOf(ConfigException.class)
                .hasMessageContaining("valid JSON array");
        }

        @Test
        @DisplayName("Missing default topic raises ConfigException")
        void missingDefaultTopicRaisesConfigException() {
            assertThatThrownBy(() ->
                DlqRoutingConfig.parse("[]", "", "localhost:9092"))
                .isInstanceOf(ConfigException.class)
                .hasMessageContaining("contractgate.dlq.routing.default");
        }

        @Test
        @DisplayName("Missing bootstrap.servers raises ConfigException")
        void missingBootstrapServersRaisesConfigException() {
            assertThatThrownBy(() ->
                DlqRoutingConfig.parse("[]", "dlq.default", ""))
                .isInstanceOf(ConfigException.class)
                .hasMessageContaining("bootstrap.servers");
        }

        @Test
        @DisplayName("Rule with non-object match raises ConfigException")
        void nonObjectMatchRaisesConfigException() {
            assertThatThrownBy(() ->
                DlqRoutingConfig.parse(
                    "[{\"match\":\"not-an-object\",\"topic\":\"t\"}]",
                    "dlq.default", "localhost:9092"))
                .isInstanceOf(ConfigException.class)
                .hasMessageContaining("match must be a JSON object");
        }

        @Test
        @DisplayName("Rule with non-string topic raises ConfigException")
        void nonStringTopicRaisesConfigException() {
            assertThatThrownBy(() ->
                DlqRoutingConfig.parse(
                    "[{\"match\":{},\"topic\":42}]",
                    "dlq.default", "localhost:9092"))
                .isInstanceOf(ConfigException.class)
                .hasMessageContaining("topic must be a string");
        }
    }
}
