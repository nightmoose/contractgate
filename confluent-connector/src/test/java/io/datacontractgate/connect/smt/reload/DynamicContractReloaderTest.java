package io.datacontractgate.connect.smt.reload;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.DisplayName;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.extension.ExtendWith;
import org.mockito.Mock;
import org.mockito.junit.jupiter.MockitoExtension;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

import static org.assertj.core.api.Assertions.assertThat;
import static org.mockito.Mockito.*;

/**
 * Unit tests for {@link DynamicContractReloader}.
 *
 * <p>Uses Mockito to stub {@link ContractVersionCheck} so no HTTP calls are made.
 * Tests cover:
 * <ul>
 *   <li>No-op when hash unchanged</li>
 *   <li>Swap on hash change</li>
 *   <li>No-swap + failure counter increment on check failure</li>
 *   <li>Reloader lifecycle (start → poll → stop)</li>
 * </ul>
 * </p>
 */
@ExtendWith(MockitoExtension.class)
class DynamicContractReloaderTest {

    @Mock
    private ContractVersionCheck versionCheck;

    private DynamicContractReloader reloader;

    @AfterEach
    void tearDown() {
        if (reloader != null) {
            reloader.stop();
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    private static ContractVersionInfo versionInfo(String version, String hash) {
        return new ContractVersionInfo(version, hash);
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    @Test
    @DisplayName("No callback triggered when hash is unchanged")
    void noCallbackWhenHashUnchanged() throws Exception {
        ContractVersionInfo initial = versionInfo("1.0.0", "aaaa");

        when(versionCheck.fetchCurrentVersion()).thenReturn(initial);

        List<ContractVersionInfo> reloadedVersions = new ArrayList<>();
        reloader = new DynamicContractReloader(
            versionCheck, 50, "warn", initial, reloadedVersions::add);
        reloader.start();

        // Wait for at least one poll
        Thread.sleep(200);
        reloader.stop();

        assertThat(reloadedVersions).isEmpty();
        assertThat(reloader.reloadSuccessCount.get()).isEqualTo(0);
        assertThat(reloader.reloadFailureCount.get()).isEqualTo(0);
    }

    @Test
    @DisplayName("Callback triggered and success counter incremented on hash change")
    void callbackOnHashChange() throws Exception {
        ContractVersionInfo v1 = versionInfo("1.0.0", "aaaa");
        ContractVersionInfo v2 = versionInfo("2.0.0", "bbbb");

        // First call returns v1 (set as initial) — treated as unchanged.
        // Second call returns v2 — triggers a swap.
        when(versionCheck.fetchCurrentVersion())
            .thenReturn(v1)   // poll 1 — unchanged
            .thenReturn(v2);  // poll 2 — changed
        when(versionCheck.fetchContractYaml("2.0.0")).thenReturn("version: \"2.0.0\"\nname: test");

        CountDownLatch reloadLatch = new CountDownLatch(1);
        AtomicReference<ContractVersionInfo> reloadedTo = new AtomicReference<>();

        reloader = new DynamicContractReloader(
            versionCheck, 50, "warn", v1, info -> {
                reloadedTo.set(info);
                reloadLatch.countDown();
            });
        reloader.start();

        boolean reloaded = reloadLatch.await(2, TimeUnit.SECONDS);
        reloader.stop();

        assertThat(reloaded).isTrue();
        assertThat(reloadedTo.get()).isNotNull();
        assertThat(reloadedTo.get().version).isEqualTo("2.0.0");
        assertThat(reloadedTo.get().hash).isEqualTo("bbbb");
        assertThat(reloader.reloadSuccessCount.get()).isGreaterThanOrEqualTo(1);
        assertThat(reloader.reloadFailureCount.get()).isEqualTo(0);
    }

    @Test
    @DisplayName("No swap and failure counter incremented when version check throws")
    void noSwapOnCheckFailure() throws Exception {
        ContractVersionInfo initial = versionInfo("1.0.0", "aaaa");

        when(versionCheck.fetchCurrentVersion())
            .thenThrow(new ContractVersionCheck.ContractCheckException("HTTP 503 from gateway"));

        List<ContractVersionInfo> reloadedVersions = new ArrayList<>();
        reloader = new DynamicContractReloader(
            versionCheck, 50, "warn", initial, reloadedVersions::add);
        reloader.start();

        Thread.sleep(300);
        reloader.stop();

        // Callback must never have been called
        assertThat(reloadedVersions).isEmpty();
        // Current version unchanged
        assertThat(reloader.currentVersion()).isEqualTo(initial);
        // Failure counter incremented at least once
        assertThat(reloader.reloadFailureCount.get()).isGreaterThanOrEqualTo(1);
        assertThat(reloader.reloadSuccessCount.get()).isEqualTo(0);
    }

    @Test
    @DisplayName("No swap when fetched YAML body is empty")
    void noSwapOnEmptyBody() throws Exception {
        ContractVersionInfo v1 = versionInfo("1.0.0", "aaaa");
        ContractVersionInfo v2 = versionInfo("2.0.0", "bbbb");

        when(versionCheck.fetchCurrentVersion()).thenReturn(v1).thenReturn(v2);
        when(versionCheck.fetchContractYaml("2.0.0")).thenReturn(""); // empty → reject

        List<ContractVersionInfo> reloads = new ArrayList<>();
        reloader = new DynamicContractReloader(
            versionCheck, 50, "warn", v1, reloads::add);
        reloader.start();

        Thread.sleep(300);
        reloader.stop();

        assertThat(reloads).isEmpty();
        assertThat(reloader.currentVersion().version).isEqualTo("1.0.0");
        assertThat(reloader.reloadFailureCount.get()).isGreaterThanOrEqualTo(1);
    }

    @Test
    @DisplayName("fail-task action: rethrowPendingFailureIfAny throws after a failed poll")
    void failTaskActionSetsPendingException() throws Exception {
        ContractVersionInfo initial = versionInfo("1.0.0", "aaaa");

        when(versionCheck.fetchCurrentVersion())
            .thenThrow(new ContractVersionCheck.ContractCheckException("server down"));

        reloader = new DynamicContractReloader(
            versionCheck, 50, "fail-task", initial, v -> {});
        reloader.start();

        // Wait for at least one failed poll
        Thread.sleep(300);
        reloader.stop();

        assertThat(reloader.reloadFailureCount.get()).isGreaterThanOrEqualTo(1);

        // rethrowPendingFailureIfAny() must now throw, simulating what apply() does
        org.assertj.core.api.Assertions.assertThatThrownBy(
            () -> reloader.rethrowPendingFailureIfAny())
            .isInstanceOf(org.apache.kafka.connect.errors.ConnectException.class)
            .hasMessageContaining("fail-task");
    }

    @Test
    @DisplayName("Stop is idempotent")
    void stopIsIdempotent() {
        reloader = new DynamicContractReloader(
            versionCheck, 100, "warn", null, v -> {});
        reloader.start();
        reloader.stop();
        reloader.stop(); // should not throw
    }
}
