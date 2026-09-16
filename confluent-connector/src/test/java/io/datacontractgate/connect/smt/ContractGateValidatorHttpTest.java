package io.datacontractgate.connect.smt;

import com.github.tomakehurst.wiremock.WireMockServer;
import com.github.tomakehurst.wiremock.core.WireMockConfiguration;
import org.apache.kafka.connect.data.Schema;
import org.apache.kafka.connect.errors.DataException;
import org.apache.kafka.connect.header.ConnectHeaders;
import org.apache.kafka.connect.sink.SinkRecord;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;

import java.util.Map;

import static com.github.tomakehurst.wiremock.client.WireMock.aResponse;
import static com.github.tomakehurst.wiremock.client.WireMock.okJson;
import static com.github.tomakehurst.wiremock.client.WireMock.post;
import static com.github.tomakehurst.wiremock.client.WireMock.urlPathEqualTo;
import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * End-to-end SMT tests against a mock gateway. Uses the real
 * {@link io.datacontractgate.connect.client.ContractGateClient} so HTTP 422
 * cannot be stubbed away at the client boundary.
 */
class ContractGateValidatorHttpTest {

    private static final String CONTRACT_ID = "11111111-1111-1111-1111-111111111111";
    private static final String FAIL_BODY = "{"
        + "\"total\":1,\"passed\":0,\"failed\":1,\"dry_run\":false,"
        + "\"resolved_version\":\"1.0.0\","
        + "\"results\":[{\"passed\":false,\"contract_version\":\"1.0.0\","
        + "\"violations\":[{\"field\":\"event_type\",\"kind\":\"enum_violation\","
        + "\"message\":\"value 'checkout' not in allowed enum\"}]}]}";
    private static final String PASS_BODY = "{"
        + "\"total\":1,\"passed\":1,\"failed\":0,\"dry_run\":false,"
        + "\"resolved_version\":\"1.0.0\","
        + "\"results\":[{\"passed\":true,\"contract_version\":\"1.0.0\","
        + "\"violations\":[]}]}";

    private static WireMockServer wireMock;

    @BeforeAll
    static void startWireMock() {
        wireMock = new WireMockServer(WireMockConfiguration.wireMockConfig().dynamicPort());
        wireMock.start();
    }

    @AfterAll
    static void stopWireMock() {
        wireMock.stop();
    }

    @BeforeEach
    void resetStubs() {
        wireMock.resetAll();
    }

    private ContractGateValidator<SinkRecord> smt(String onFailure) {
        ContractGateValidator<SinkRecord> s = new ContractGateValidator<>();
        s.configure(Map.of(
            ContractGateValidatorConfig.API_URL_CONFIG, wireMock.baseUrl(),
            ContractGateValidatorConfig.CONTRACT_ID_CONFIG, CONTRACT_ID,
            ContractGateValidatorConfig.API_KEY_CONFIG, "cg_live_test",
            ContractGateValidatorConfig.ON_FAILURE_CONFIG, onFailure
        ));
        return s;
    }

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

    @Test
    @DisplayName("HTTP 422 rejected event throws DataException in DLQ mode")
    void http422GoesToDlq() {
        wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
            .willReturn(aResponse().withStatus(422)
                .withHeader("Content-Type", "application/json")
                .withBody(FAIL_BODY)));

        ContractGateValidator<SinkRecord> s = smt("DLQ");
        try {
            assertThatThrownBy(() -> s.apply(jsonRecord("{\"event_type\":\"checkout\"}")))
                .isInstanceOf(DataException.class)
                .hasMessageContaining("enum_violation")
                .hasMessageContaining("event_type");
        } finally {
            s.close();
        }
    }

    @Test
    @DisplayName("HTTP 422 rejected event is tagged and passed in TAG_AND_PASS mode")
    void http422TagAndPass() {
        wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
            .willReturn(aResponse().withStatus(422)
                .withHeader("Content-Type", "application/json")
                .withBody(FAIL_BODY)));

        ContractGateValidator<SinkRecord> s = smt("TAG_AND_PASS");
        try {
            SinkRecord result = s.apply(jsonRecord("{\"event_type\":\"checkout\"}"));
            assertThat(result.headers().lastWithName("contractgate.passed").value())
                .isEqualTo("false");
            assertThat(result.headers().lastWithName("contractgate.violation.0.kind").value())
                .isEqualTo("enum_violation");
        } finally {
            s.close();
        }
    }

    @Test
    @DisplayName("HTTP 200 passing event continues downstream")
    void http200Passes() {
        wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
            .willReturn(okJson(PASS_BODY)));

        ContractGateValidator<SinkRecord> s = smt("DLQ");
        try {
            SinkRecord record = jsonRecord("{\"event_type\":\"click\"}");
            SinkRecord result = s.apply(record);
            assertThat(result.headers().lastWithName("contractgate.passed").value())
                .isEqualTo("true");
        } finally {
            s.close();
        }
    }

    @Test
    @DisplayName("HTTP 500 still fail-opens so a gateway outage does not halt the connector")
    void http500FailsOpen() {
        wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
            .willReturn(aResponse().withStatus(500).withBody("boom")));

        ContractGateValidator<SinkRecord> s = smt("DLQ");
        try {
            SinkRecord record = jsonRecord("{\"event_type\":\"click\"}");
            assertThat(s.apply(record)).isSameAs(record);
        } finally {
            s.close();
        }
    }
}
