package io.datacontractgate.connect.smt;

import io.datacontractgate.connect.client.ContractGateClient;
import io.datacontractgate.connect.client.IngestResponse;
import io.datacontractgate.connect.client.ViolationDetail;
import org.apache.kafka.connect.data.Schema;
import org.apache.kafka.connect.errors.DataException;
import org.apache.kafka.connect.header.ConnectHeaders;
import org.apache.kafka.connect.sink.SinkRecord;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Nested;
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
 * Unit tests for {@link ContractGateValidator}.
 *
 * <p>Uses Mockito to stub {@link ContractGateClient} so no real network calls
 * are made. The SMT's {@code client} field is injected via reflection to avoid
 * needing to make it package-private or add a test-only constructor.</p>
 */
@ExtendWith(MockitoExtension.class)
class ContractGateValidatorTest {

    private static final String API_URL     = "http://localhost:8080";
    private static final String CONTRACT_ID = "test-contract-uuid";

    @Mock
    private ContractGateClient mockClient;

    private ContractGateValidator<SinkRecord> smt;

    @BeforeEach
    void setUp() throws Exception {
        smt = new ContractGateValidator<>();
        smt.configure(Map.of(
            ContractGateValidatorConfig.API_URL_CONFIG, API_URL,
            ContractGateValidatorConfig.CONTRACT_ID_CONFIG, CONTRACT_ID
        ));
        // Inject mock client via reflection (avoids test-only constructor)
        Field clientField = ContractGateValidator.class.getDeclaredField("client");
        clientField.setAccessible(true);
        clientField.set(smt, mockClient);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private static SinkRecord jsonRecord(String json) {
        return new SinkRecord(
            "test-topic", 0,
            Schema.STRING_SCHEMA, "key",
            Schema.STRING_SCHEMA, json,
            42L,
            System.currentTimeMillis(),
            org.apache.kafka.common.record.TimestampType.CREATE_TIME,
            new ConnectHeaders()
        );
    }

    private static IngestResponse passingResponse(String version) {
        IngestResponse resp = new IngestResponse();
        resp.total = 1; resp.passed = 1; resp.failed = 0;
        resp.resolvedVersion = version;

        IngestResponse.IngestEventResult r = new IngestResponse.IngestEventResult();
        r.passed = true;
        r.violations = Collections.emptyList();
        r.contractVersion = version;
        resp.results = List.of(r);
        return resp;
    }

    private static IngestResponse failingResponse(String version, ViolationDetail... violations) {
        IngestResponse resp = new IngestResponse();
        resp.total = 1; resp.passed = 0; resp.failed = 1;
        resp.resolvedVersion = version;

        IngestResponse.IngestEventResult r = new IngestResponse.IngestEventResult();
        r.passed = false;
        r.violations = List.of(violations);
        r.contractVersion = version;
        resp.results = List.of(r);
        return resp;
    }

    private static ViolationDetail violation(String field, String kind, String message) {
        ViolationDetail v = new ViolationDetail();
        v.field = field; v.kind = kind; v.message = message;
        return v;
    }

    // ── Test cases ────────────────────────────────────────────────────────────

    @Nested
    @DisplayName("Tombstone handling")
    class TombstoneTests {

        @Test
        @DisplayName("Null-value tombstone is passed through without calling API")
        void tombstoneIsPassedThrough() {
            SinkRecord tombstone = new SinkRecord(
                "test-topic", 0,
                Schema.STRING_SCHEMA, "key",
                null, null, 42L
            );
            SinkRecord result = smt.apply(tombstone);
            assertThat(result).isSameAs(tombstone);
            // No interactions with client expected
        }
    }

    @Nested
    @DisplayName("Passing records (DLQ mode)")
    class PassingTests {

        @Test
        @DisplayName("Passing record is returned unchanged when addResultHeaders=false")
        void passingRecordReturnedUnchanged() throws Exception {
            // Re-configure with headers disabled
            smt = new ContractGateValidator<>();
            smt.configure(Map.of(
                ContractGateValidatorConfig.API_URL_CONFIG, API_URL,
                ContractGateValidatorConfig.CONTRACT_ID_CONFIG, CONTRACT_ID,
                ContractGateValidatorConfig.ADD_RESULT_HEADERS_CONFIG, "false"
            ));
            Field cf = ContractGateValidator.class.getDeclaredField("client");
            cf.setAccessible(true);
            cf.set(smt, mockClient);

            when(mockClient.validate(anyString(), anyInt()))
                .thenReturn(passingResponse("1.0.0"));

            SinkRecord record = jsonRecord("{\"event_type\":\"click\"}");
            SinkRecord result = smt.apply(record);

            assertThat(result).isSameAs(record);
        }

        @Test
        @DisplayName("Passing record gets contractgate.passed=true header when addResultHeaders=true")
        void passingRecordGetsPassedHeader() throws Exception {
            when(mockClient.validate(anyString(), anyInt()))
                .thenReturn(passingResponse("2.1.0"));

            SinkRecord record = jsonRecord("{\"event_type\":\"view\"}");
            SinkRecord result = smt.apply(record);

            assertThat(result).isNotNull();
            var passedHeader = result.headers().lastWithName("contractgate.passed");
            assertThat(passedHeader).isNotNull();
            assertThat(passedHeader.value()).isEqualTo("true");

            var versionHeader = result.headers().lastWithName("contractgate.contract.version");
            assertThat(versionHeader).isNotNull();
            assertThat(versionHeader.value()).isEqualTo("2.1.0");
        }
    }

    @Nested
    @DisplayName("Failing records — DLQ mode")
    class DlqModeTests {

        @Test
        @DisplayName("Single violation throws DataException with violation detail in message")
        void singleViolationThrowsDataException() throws Exception {
            when(mockClient.validate(anyString(), anyInt()))
                .thenReturn(failingResponse("1.0.0",
                    violation("event_type", "enum_violation", "value 'checkout' not in allowed enum")));

            SinkRecord record = jsonRecord("{\"event_type\":\"checkout\"}");

            assertThatThrownBy(() -> smt.apply(record))
                .isInstanceOf(DataException.class)
                .hasMessageContaining("enum_violation")
                .hasMessageContaining("event_type")
                .hasMessageContaining("test-contract-uuid");
        }

        @Test
        @DisplayName("Multiple violations all appear in the exception message")
        void multipleViolationsInMessage() throws Exception {
            when(mockClient.validate(anyString(), anyInt()))
                .thenReturn(failingResponse("1.0.0",
                    violation("user_id", "missing_required_field", "required field missing"),
                    violation("amount", "range_violation", "value -5 below minimum 0")));

            SinkRecord record = jsonRecord("{\"event_type\":\"purchase\"}");

            assertThatThrownBy(() -> smt.apply(record))
                .isInstanceOf(DataException.class)
                .hasMessageContaining("2 violation(s)")
                .hasMessageContaining("missing_required_field")
                .hasMessageContaining("range_violation");
        }
    }

    @Nested
    @DisplayName("Failing records — TAG_AND_PASS mode")
    class TagAndPassModeTests {

        @BeforeEach
        void setUpTagAndPass() throws Exception {
            smt = new ContractGateValidator<>();
            smt.configure(Map.of(
                ContractGateValidatorConfig.API_URL_CONFIG, API_URL,
                ContractGateValidatorConfig.CONTRACT_ID_CONFIG, CONTRACT_ID,
                ContractGateValidatorConfig.ON_FAILURE_CONFIG, "TAG_AND_PASS"
            ));
            Field cf = ContractGateValidator.class.getDeclaredField("client");
            cf.setAccessible(true);
            cf.set(smt, mockClient);
        }

        @Test
        @DisplayName("Failing record is returned (not thrown) with contractgate.passed=false")
        void failingRecordReturnedWithHeader() throws Exception {
            when(mockClient.validate(anyString(), anyInt()))
                .thenReturn(failingResponse("1.0.0",
                    violation("amount", "type_mismatch", "expected number got string")));

            SinkRecord record = jsonRecord("{\"amount\":\"oops\"}");
            SinkRecord result = smt.apply(record);

            assertThat(result).isNotNull();
            assertThat(result.headers().lastWithName("contractgate.passed").value())
                .isEqualTo("false");
            assertThat(result.headers().lastWithName("contractgate.violations.count").value())
                .isEqualTo("1");
        }

        @Test
        @DisplayName("Violation detail headers are stamped for each violation up to max")
        void violationHeadersStamped() throws Exception {
            when(mockClient.validate(anyString(), anyInt()))
                .thenReturn(failingResponse("1.0.0",
                    violation("user_id", "missing_required_field", "required field missing"),
                    violation("event_type", "enum_violation", "bad value")));

            SinkRecord record = jsonRecord("{\"ts\":1234}");
            SinkRecord result = smt.apply(record);

            assertThat(result.headers().lastWithName("contractgate.violation.0.field").value())
                .isEqualTo("user_id");
            assertThat(result.headers().lastWithName("contractgate.violation.0.kind").value())
                .isEqualTo("missing_required_field");
            assertThat(result.headers().lastWithName("contractgate.violation.1.field").value())
                .isEqualTo("event_type");
        }
    }

    @Nested
    @DisplayName("API failure handling")
    class ApiFailureTests {

        @Test
        @DisplayName("When API is unreachable, record passes through with a warning (fail-open)")
        void apiUnavailableFailsOpen() throws Exception {
            when(mockClient.validate(anyString(), anyInt()))
                .thenThrow(new ContractGateClient.ContractGateApiException(
                    "Connection refused: localhost:8080"));

            SinkRecord record = jsonRecord("{\"event_type\":\"click\"}");
            // Should NOT throw — fail-open behaviour
            SinkRecord result = smt.apply(record);
            assertThat(result).isSameAs(record);
        }
    }
}
