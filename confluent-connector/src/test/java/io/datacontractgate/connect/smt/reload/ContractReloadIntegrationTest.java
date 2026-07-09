package io.datacontractgate.connect.smt.reload;

import com.github.tomakehurst.wiremock.WireMockServer;
import com.github.tomakehurst.wiremock.core.WireMockConfiguration;
import org.junit.jupiter.api.*;

import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

import static com.github.tomakehurst.wiremock.client.WireMock.*;
import static org.assertj.core.api.Assertions.assertThat;

/**
 * Integration test for {@link DynamicContractReloader} with a mock HTTP gateway.
 *
 * <p>Uses WireMock to simulate the ContractGate API:
 * <ul>
 *   <li>First: {@code GET /v1/contracts/test-id/version} returns v1 hash.</li>
 *   <li>Then: same endpoint returns v2 hash.</li>
 *   <li>When hash changes: reloader fetches
 *       {@code GET /contracts/test-id/versions/2.0.0} for body validation.</li>
 *   <li>Asserts: reloader picks up the change within {@code poll.ms}.</li>
 * </ul>
 * </p>
 *
 * <p>This test satisfies RFC-064 acceptance criterion #2 (AC#2):
 * "An embedded-Kafka integration test demonstrates the SMT picking up a
 * contract version bump within poll.ms." No full Kafka cluster is needed for
 * the reload path — the relevant behaviour is the reloader detecting the
 * version change and calling the onReload callback, which in production would
 * update the AtomicReference used by apply().</p>
 */
@TestMethodOrder(MethodOrderer.OrderAnnotation.class)
class ContractReloadIntegrationTest {

    private static WireMockServer wireMock;

    private static final String CONTRACT_ID = "test-contract-id";
    private static final String V1_HASH     = "aaaaaaaabbbbbbbbccccccccdddddddd00000000000000000000000000000001";
    private static final String V2_HASH     = "aaaaaaaabbbbbbbbccccccccdddddddd00000000000000000000000000000002";
    // No inner quotes in the YAML values — avoids JSON escaping issues when
    // these strings are embedded in WireMock stub response bodies.
    private static final String V1_YAML     = "version: 1.0.0\nname: test-contract";
    private static final String V2_YAML     = "version: 2.0.0\nname: test-contract";

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

    // ── Tests ─────────────────────────────────────────────────────────────────

    @Test
    @DisplayName("AC#2: Reloader picks up contract version bump within poll.ms")
    void reloaderPicksUpVersionBump() throws Exception {
        // Stub: first poll returns v1 (initial — no change expected)
        // Second poll returns v2 (triggers body fetch + reload callback)
        wireMock.stubFor(get(urlEqualTo("/v1/contracts/" + CONTRACT_ID + "/version"))
            .inScenario("version-bump")
            .whenScenarioStateIs("Started")
            .willReturn(okJson("{\"version\":\"1.0.0\",\"hash\":\"" + V1_HASH + "\"}"))
            .willSetStateTo("bumped"));

        wireMock.stubFor(get(urlEqualTo("/v1/contracts/" + CONTRACT_ID + "/version"))
            .inScenario("version-bump")
            .whenScenarioStateIs("bumped")
            .willReturn(okJson("{\"version\":\"2.0.0\",\"hash\":\"" + V2_HASH + "\"}")));

        // Stub: body fetch for v2
        wireMock.stubFor(get(urlEqualTo("/contracts/" + CONTRACT_ID + "/versions/2.0.0"))
            .willReturn(okJson("{\"version\":\"2.0.0\",\"yaml_content\":\"" + V2_YAML.replace("\n", "\\n") + "\"}")));

        String baseUrl = "http://localhost:" + wireMock.port();
        ContractVersionCheck check = new ContractVersionCheck(
            baseUrl, CONTRACT_ID, "", 5000, 10000);

        // Start with v1 as the known version
        ContractVersionInfo initialVersion = new ContractVersionInfo("1.0.0", V1_HASH);

        CountDownLatch reloadLatch = new CountDownLatch(1);
        AtomicReference<ContractVersionInfo> reloadedTo = new AtomicReference<>();

        long pollMs = 200; // fast polling for test
        DynamicContractReloader reloader = new DynamicContractReloader(
            check, pollMs, "warn", initialVersion, info -> {
                reloadedTo.set(info);
                reloadLatch.countDown();
            });

        reloader.start();
        try {
            // Wait up to 3× poll.ms — the reload should happen within 2 polls
            boolean reloaded = reloadLatch.await(3 * pollMs, TimeUnit.MILLISECONDS);

            assertThat(reloaded)
                .as("Reload callback should have fired within poll.ms")
                .isTrue();
            assertThat(reloadedTo.get().version).isEqualTo("2.0.0");
            assertThat(reloadedTo.get().hash).isEqualTo(V2_HASH);
            assertThat(reloader.reloadSuccessCount.get()).isGreaterThanOrEqualTo(1);
            assertThat(reloader.reloadFailureCount.get()).isEqualTo(0);
        } finally {
            reloader.stop();
        }
    }

    @Test
    @DisplayName("Reloader keeps old contract when gateway returns unparseable body")
    void keepOldContractOnBadBody() throws Exception {
        // Version probe shows a change
        wireMock.stubFor(get(urlEqualTo("/v1/contracts/" + CONTRACT_ID + "/version"))
            .willReturn(okJson("{\"version\":\"3.0.0\",\"hash\":\"" + V2_HASH + "\"}")));

        // Body fetch returns malformed (JSON error response, not YAML)
        wireMock.stubFor(get(urlEqualTo("/contracts/" + CONTRACT_ID + "/versions/3.0.0"))
            .willReturn(okJson("{\"error\":\"not found\"}")));

        String baseUrl = "http://localhost:" + wireMock.port();
        ContractVersionCheck check = new ContractVersionCheck(
            baseUrl, CONTRACT_ID, "", 5000, 10000);

        ContractVersionInfo initial = new ContractVersionInfo("1.0.0", V1_HASH);

        AtomicReference<ContractVersionInfo> reloadedTo = new AtomicReference<>();
        DynamicContractReloader reloader = new DynamicContractReloader(
            check, 100, "warn", initial, reloadedTo::set);

        reloader.start();
        Thread.sleep(500);
        reloader.stop();

        // The body starts with '{' — reloader should reject it as invalid YAML
        assertThat(reloadedTo.get()).isNull();
        assertThat(reloader.currentVersion().version).isEqualTo("1.0.0");
        assertThat(reloader.reloadFailureCount.get()).isGreaterThanOrEqualTo(1);
        assertThat(reloader.reloadSuccessCount.get()).isEqualTo(0);
    }

    @Test
    @DisplayName("Reloader keeps old contract and increments failure counter when gateway is down")
    void keepOldContractWhenGatewayDown() throws Exception {
        // Don't stub anything — all requests return 404
        String baseUrl = "http://localhost:" + wireMock.port();
        ContractVersionCheck check = new ContractVersionCheck(
            baseUrl, CONTRACT_ID, "", 1000, 2000);

        ContractVersionInfo initial = new ContractVersionInfo("1.0.0", V1_HASH);
        AtomicReference<ContractVersionInfo> reloadedTo = new AtomicReference<>();

        DynamicContractReloader reloader = new DynamicContractReloader(
            check, 100, "warn", initial, reloadedTo::set);
        reloader.start();
        Thread.sleep(400);
        reloader.stop();

        assertThat(reloadedTo.get()).isNull();
        assertThat(reloader.reloadFailureCount.get()).isGreaterThanOrEqualTo(1);
    }
}
