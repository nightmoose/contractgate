package io.datacontractgate.connect.client;

import com.github.tomakehurst.wiremock.WireMockServer;
import com.github.tomakehurst.wiremock.core.WireMockConfiguration;
import org.junit.jupiter.api.AfterAll;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Nested;
import org.junit.jupiter.api.Test;

import static com.github.tomakehurst.wiremock.client.WireMock.aResponse;
import static com.github.tomakehurst.wiremock.client.WireMock.equalTo;
import static com.github.tomakehurst.wiremock.client.WireMock.okJson;
import static com.github.tomakehurst.wiremock.client.WireMock.post;
import static com.github.tomakehurst.wiremock.client.WireMock.postRequestedFor;
import static com.github.tomakehurst.wiremock.client.WireMock.urlEqualTo;
import static com.github.tomakehurst.wiremock.client.WireMock.urlPathEqualTo;
import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * WireMock tests for {@link ContractGateClient}.
 *
 * <p>The production bug this locks: HTTP 422 (all events failed) is a
 * validation outcome, not an outage. The SMT must parse the body and let
 * DLQ / TAG_AND_PASS run. Fail-open is reserved for transport and
 * non-validation statuses.</p>
 */
class ContractGateClientTest {

    private static final String CONTRACT_ID = "11111111-1111-1111-1111-111111111111";
    private static final String FAIL_BODY = "{"
        + "\"total\":1,\"passed\":0,\"failed\":1,\"dry_run\":false,"
        + "\"resolved_version\":\"1.0.0\",\"version_pin_source\":\"default_stable\","
        + "\"results\":[{"
        + "\"passed\":false,"
        + "\"contract_version\":\"1.0.0\","
        + "\"validation_us\":12,"
        + "\"violations\":[{\"field\":\"event_type\",\"kind\":\"enum_violation\","
        + "\"message\":\"value 'checkout' not in allowed enum\"}]"
        + "}]}";
    private static final String PASS_BODY = "{"
        + "\"total\":1,\"passed\":1,\"failed\":0,\"dry_run\":false,"
        + "\"resolved_version\":\"1.0.0\",\"version_pin_source\":\"default_stable\","
        + "\"results\":[{\"passed\":true,\"contract_version\":\"1.0.0\","
        + "\"validation_us\":8,\"violations\":[]}"
        + "]}";

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

    private ContractGateClient client() {
        return client("", false);
    }

    private ContractGateClient client(String version, boolean dryRun) {
        return new ContractGateClient(
            wireMock.baseUrl(),
            CONTRACT_ID,
            "cg_live_test",
            version,
            dryRun,
            2_000,
            5_000
        );
    }

    @Nested
    @DisplayName("Validation HTTP statuses")
    class ValidationStatuses {

        @Test
        @DisplayName("HTTP 422 with results is a rejected event, not an API failure")
        void http422IsValidationFailure() throws Exception {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(aResponse().withStatus(422).withHeader("Content-Type", "application/json")
                    .withBody(FAIL_BODY)));

            IngestResponse resp = client().validate("{\"event_type\":\"checkout\"}", 5_000);

            assertThat(resp.failed).isEqualTo(1);
            assertThat(resp.singleResult().passed).isFalse();
            assertThat(resp.singleResult().violationSummary()).contains("enum_violation");
        }

        @Test
        @DisplayName("HTTP 207 with results is a mixed-batch validation outcome")
        void http207IsValidationOutcome() throws Exception {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(aResponse().withStatus(207).withHeader("Content-Type", "application/json")
                    .withBody(FAIL_BODY)));

            IngestResponse resp = client().validate("{\"event_type\":\"checkout\"}", 5_000);
            assertThat(resp.singleResult().passed).isFalse();
        }

        @Test
        @DisplayName("HTTP 200 with results is a passing event")
        void http200IsPass() throws Exception {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(okJson(PASS_BODY)));

            IngestResponse resp = client().validate("{\"event_type\":\"click\"}", 5_000);
            assertThat(resp.singleResult().passed).isTrue();
        }
    }

    @Nested
    @DisplayName("Non-validation statuses stay fail-open at the SMT")
    class TransportFailures {

        @Test
        @DisplayName("HTTP 500 is an API failure")
        void http500IsApiFailure() {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(aResponse().withStatus(500).withBody("internal error")));

            assertThatThrownBy(() -> client().validate("{\"event_type\":\"click\"}", 5_000))
                .isInstanceOf(ContractGateClient.ContractGateApiException.class)
                .hasMessageContaining("HTTP 500");
        }

        @Test
        @DisplayName("HTTP 401 is an API failure")
        void http401IsApiFailure() {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(aResponse().withStatus(401).withBody("{\"error\":\"unauthorized\"}")));

            assertThatThrownBy(() -> client().validate("{\"event_type\":\"click\"}", 5_000))
                .isInstanceOf(ContractGateClient.ContractGateApiException.class)
                .hasMessageContaining("HTTP 401");
        }

        @Test
        @DisplayName("HTTP 422 without a results array is an API failure")
        void http422WithoutResultsIsApiFailure() {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(aResponse().withStatus(422)
                    .withHeader("Content-Type", "application/json")
                    .withBody("{\"error\":\"idempotency conflict\"}")));

            assertThatThrownBy(() -> client().validate("{\"event_type\":\"click\"}", 5_000))
                .isInstanceOf(ContractGateClient.ContractGateApiException.class)
                .hasMessageContaining("no per-event results");
        }
    }

    @Nested
    @DisplayName("v1 URL construction")
    class UrlConstruction {

        @Test
        @DisplayName("Posts to /v1/ingest/{id} with the API key header")
        void postsToV1Ingest() throws Exception {
            wireMock.stubFor(post(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .willReturn(okJson(PASS_BODY)));

            client().validate("{\"event_type\":\"click\"}", 5_000);

            wireMock.verify(postRequestedFor(urlPathEqualTo("/v1/ingest/" + CONTRACT_ID))
                .withHeader("x-api-key", equalTo("cg_live_test"))
                .withHeader("Content-Type", equalTo("application/json")));
        }

        @Test
        @DisplayName("Version pin and dry_run become query parameters")
        void versionAndDryRunQueryParams() throws Exception {
            wireMock.stubFor(post(urlEqualTo(
                    "/v1/ingest/" + CONTRACT_ID + "?dry_run=true&version=1.2.0"))
                .willReturn(okJson(PASS_BODY)));

            client("1.2.0", true).validate("{\"event_type\":\"click\"}", 5_000);

            wireMock.verify(postRequestedFor(urlEqualTo(
                "/v1/ingest/" + CONTRACT_ID + "?dry_run=true&version=1.2.0")));
        }

        @Test
        @DisplayName("buildUrl uses /v1/ingest and omits empty optional params")
        void buildUrlDefaults() {
            assertThat(client().buildUrl())
                .isEqualTo(wireMock.baseUrl() + "/v1/ingest/" + CONTRACT_ID);
            assertThat(client("1.2.0", false).buildUrl())
                .isEqualTo(wireMock.baseUrl() + "/v1/ingest/" + CONTRACT_ID + "?version=1.2.0");
        }
    }
}
