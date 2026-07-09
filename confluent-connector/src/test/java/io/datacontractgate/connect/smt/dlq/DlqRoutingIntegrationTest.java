package io.datacontractgate.connect.smt.dlq;

import io.datacontractgate.connect.client.ContractGateClient;
import io.datacontractgate.connect.client.IngestResponse;
import io.datacontractgate.connect.client.ViolationDetail;
import io.datacontractgate.connect.smt.ContractGateValidator;
import io.datacontractgate.connect.smt.ContractGateValidatorConfig;
import org.apache.kafka.clients.producer.MockProducer;
import org.apache.kafka.clients.producer.ProducerRecord;
import org.apache.kafka.common.serialization.ByteArraySerializer;
import org.apache.kafka.connect.data.Schema;
import org.apache.kafka.connect.errors.DataException;
import org.apache.kafka.connect.header.ConnectHeaders;
import org.apache.kafka.connect.sink.SinkRecord;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.mockito.Mock;
import org.mockito.junit.jupiter.MockitoExtension;

import java.lang.reflect.Field;
import java.util.Collections;
import java.util.List;
import java.util.Map;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;
import static org.mockito.ArgumentMatchers.anyInt;
import static org.mockito.ArgumentMatchers.anyString;
import static org.mockito.Mockito.when;

/**
 * Integration test for per-violation DLQ routing (RFC-064 AC#3).
 *
 * <p>Wires the full {@link ContractGateValidator} → {@link DlqRouter} →
 * {@link KafkaDlqProducer} pipeline.  Uses:</p>
 * <ul>
 *   <li>Mockito stub for {@link ContractGateClient} (no real network calls)</li>
 *   <li>{@link MockProducer} from kafka-clients for the internal DLQ producer
 *       (no Kafka broker needed — sends captured in memory)</li>
 * </ul>
 *
 * <p>Satisfies RFC-064 acceptance criterion #3: "3 violations route to 3
 * different topics."</p>
 */
@ExtendWith(MockitoExtension.class)
class DlqRoutingIntegrationTest {

    private static final String API_URL     = "http://localhost:8080";
    private static final String CONTRACT_ID = "test-contract-uuid";

    // 3-rule config matching the RFC-064 spec example (compile-time constant, Java 11 safe)
    private static final String THREE_RULES_JSON =
        "[{\"match\":{\"severity\":\"error\",\"type\":\"pii_leak\"},\"topic\":\"audit.pii_failures\"}," +
         "{\"match\":{\"severity\":\"error\"},\"topic\":\"dlq.errors\"}," +
         "{\"match\":{\"severity\":\"warn\"}, \"topic\":\"dlq.warnings\"}]";

    @Mock
    private ContractGateClient mockClient;

    private ContractGateValidator<SinkRecord> smt;
    private MockProducer<byte[], byte[]> mockProducer;

    @BeforeEach
    void setUp() throws Exception {
        mockProducer = new MockProducer<>(true, new ByteArraySerializer(), new ByteArraySerializer());

        smt = new ContractGateValidator<>();
        smt.configure(Map.of(
            ContractGateValidatorConfig.API_URL_CONFIG,     API_URL,
            ContractGateValidatorConfig.CONTRACT_ID_CONFIG, CONTRACT_ID,
            // Enable DLQ routing with 3 rules
            ContractGateValidatorConfig.DLQ_ROUTING_ENABLED_CONFIG,  "true",
            ContractGateValidatorConfig.DLQ_ROUTING_RULES_CONFIG,    THREE_RULES_JSON,
            ContractGateValidatorConfig.DLQ_ROUTING_DEFAULT_CONFIG,  "dlq.fallback",
            ContractGateValidatorConfig.DLQ_ROUTING_PRODUCER_BOOTSTRAP_SERVERS_CONFIG, "localhost:9092"
        ));

        // Inject mock HTTP client via reflection (avoids test-only constructor)
        Field clientField = ContractGateValidator.class.getDeclaredField("client");
        clientField.setAccessible(true);
        clientField.set(smt, mockClient);

        // Inject mock Kafka producer into the KafkaDlqProducer
        // KafkaDlqProducer.producer is volatile and initialized lazily;
        // setting it here pre-empts the real KafkaProducer from being opened.
        Field dlqProducerField = ContractGateValidator.class.getDeclaredField("dlqProducer");
        dlqProducerField.setAccessible(true);
        KafkaDlqProducer dlqProducer = (KafkaDlqProducer) dlqProducerField.get(smt);

        Field innerProducerField = KafkaDlqProducer.class.getDeclaredField("producer");
        innerProducerField.setAccessible(true);
        innerProducerField.set(dlqProducer, mockProducer);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private static SinkRecord jsonRecord(String json) {
        return new SinkRecord(
            "source-topic", 0,
            Schema.STRING_SCHEMA, "key",
            Schema.STRING_SCHEMA, json,
            42L,
            System.currentTimeMillis(),
            org.apache.kafka.common.record.TimestampType.CREATE_TIME,
            new ConnectHeaders()
        );
    }

    private static IngestResponse failingWith(ViolationDetail violation) {
        IngestResponse resp = new IngestResponse();
        resp.total = 1; resp.passed = 0; resp.failed = 1;
        IngestResponse.IngestEventResult r = new IngestResponse.IngestEventResult();
        r.passed          = false;
        r.violations      = List.of(violation);
        r.contractVersion = "1.0.0";
        resp.results      = List.of(r);
        return resp;
    }

    private static ViolationDetail violation(String kind, String field) {
        ViolationDetail v = new ViolationDetail();
        v.kind  = kind;
        v.field = field;
        return v;
    }

    // ── AC#3: 3 violations → 3 different topics ───────────────────────────────

    @Test
    @DisplayName("AC#3 part-1: PII violation routes to audit.pii_failures")
    void piiViolationRoutesToAuditTopic() throws Exception {
        when(mockClient.validate(anyString(), anyInt()))
            .thenReturn(failingWith(violation("pii_leak", "email")));

        assertThatThrownBy(() -> smt.apply(jsonRecord("{\"email\":\"user@example.com\"}")))
            .isInstanceOf(DataException.class);

        List<ProducerRecord<byte[], byte[]>> sent = mockProducer.history();
        assertThat(sent).hasSize(1);
        assertThat(sent.get(0).topic()).isEqualTo("audit.pii_failures");
    }

    @Test
    @DisplayName("AC#3 part-2: Generic error violation routes to dlq.errors")
    void genericErrorViolationRoutesToDlqErrors() throws Exception {
        when(mockClient.validate(anyString(), anyInt()))
            .thenReturn(failingWith(violation("missing_required_field", "user_id")));

        assertThatThrownBy(() -> smt.apply(jsonRecord("{}")))
            .isInstanceOf(DataException.class);

        List<ProducerRecord<byte[], byte[]>> sent = mockProducer.history();
        assertThat(sent).hasSize(1);
        assertThat(sent.get(0).topic()).isEqualTo("dlq.errors");
    }

    @Test
    @DisplayName("AC#3 part-3: Warning violation routes to dlq.warnings")
    void warnViolationRoutesToDlqWarnings() throws Exception {
        when(mockClient.validate(anyString(), anyInt()))
            .thenReturn(failingWith(violation("undeclared_field", "extra_field")));

        assertThatThrownBy(() -> smt.apply(jsonRecord("{\"extra_field\":\"value\"}")))
            .isInstanceOf(DataException.class);

        List<ProducerRecord<byte[], byte[]>> sent = mockProducer.history();
        assertThat(sent).hasSize(1);
        assertThat(sent.get(0).topic()).isEqualTo("dlq.warnings");
    }

    @Test
    @DisplayName("AC#3 combined: 3 sequential violations each route to correct topic")
    void threeViolationsThreeTopics() throws Exception {
        when(mockClient.validate(anyString(), anyInt()))
            .thenReturn(failingWith(violation("pii_leak",              "email")))
            .thenReturn(failingWith(violation("missing_required_field", "user_id")))
            .thenReturn(failingWith(violation("undeclared_field",       "extra")));

        for (int i = 0; i < 3; i++) {
            try {
                smt.apply(jsonRecord("{\"n\":" + i + "}"));
            } catch (DataException ignored) { /* expected */ }
        }

        List<ProducerRecord<byte[], byte[]>> sent = mockProducer.history();
        assertThat(sent).hasSize(3);
        assertThat(sent.get(0).topic()).isEqualTo("audit.pii_failures");
        assertThat(sent.get(1).topic()).isEqualTo("dlq.errors");
        assertThat(sent.get(2).topic()).isEqualTo("dlq.warnings");
    }

    @Test
    @DisplayName("DataException still thrown after routed DLQ send (Connect error-handling still fires)")
    void dataExceptionStillThrownAfterDlqSend() throws Exception {
        when(mockClient.validate(anyString(), anyInt()))
            .thenReturn(failingWith(violation("enum_violation", "status")));

        assertThatThrownBy(() -> smt.apply(jsonRecord("{\"status\":\"bad\"}")))
            .isInstanceOf(DataException.class)
            .hasMessageContaining("enum_violation");
    }

    @Test
    @DisplayName("Passing records are never routed to any DLQ topic")
    void passingRecordNotRouted() throws Exception {
        IngestResponse passingResp = new IngestResponse();
        passingResp.total = 1; passingResp.passed = 1; passingResp.failed = 0;
        IngestResponse.IngestEventResult r = new IngestResponse.IngestEventResult();
        r.passed = true; r.violations = Collections.emptyList(); r.contractVersion = "1.0.0";
        passingResp.results = List.of(r);

        when(mockClient.validate(anyString(), anyInt())).thenReturn(passingResp);

        SinkRecord result = smt.apply(jsonRecord("{\"event_type\":\"click\"}"));
        assertThat(result).isNotNull();
        assertThat(mockProducer.history()).isEmpty();
    }
}
