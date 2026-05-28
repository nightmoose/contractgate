package io.datacontractgate.connect.smt.reload;

import java.util.Objects;

/**
 * Immutable snapshot of a contract's current version as returned by
 * {@code GET /v1/contracts/{id}/version}.
 *
 * <p>The {@code hash} is a SHA-256 hex digest of the YAML content, computed
 * server-side.  The reloader compares hashes on each poll; a changed hash is
 * the signal to fetch the full contract body and swap the SMT's reference.</p>
 */
public final class ContractVersionInfo {

    /** Semver version string, e.g. {@code "2.1.0"}. */
    public final String version;

    /**
     * SHA-256 hex digest of the YAML content.  Two responses with the same
     * hash mean the contract body has not changed.
     */
    public final String hash;

    public ContractVersionInfo(String version, String hash) {
        this.version = Objects.requireNonNull(version, "version");
        this.hash    = Objects.requireNonNull(hash,    "hash");
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (!(o instanceof ContractVersionInfo)) return false;
        ContractVersionInfo other = (ContractVersionInfo) o;
        return Objects.equals(version, other.version) && Objects.equals(hash, other.hash);
    }

    @Override
    public int hashCode() {
        return Objects.hash(version, hash);
    }

    @Override
    public String toString() {
        return "ContractVersionInfo{version='" + version + "', hash='" + hash.substring(0, 8) + "...'}";
    }
}
