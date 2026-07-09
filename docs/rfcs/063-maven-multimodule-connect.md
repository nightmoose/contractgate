# RFC-063 — Maven Multi-Module Restructure of `connect/`

**Status:** Deferred
**Date:** 2026-05-27
**Branch:** n/a — design only
**Addresses:** [RFC-059](059-open-core-split.md) §"Repo layout after the split", [RFC-060](060-license-manager-protocol.md)
**Depends on:** RFC-060

> Renames `confluent-connector/` → `connect/`, restructures it into a
> Maven multi-module project with three children:
> `connect-client` (Apache 2.0), `connect-community` (Apache 2.0),
> `connect-enterprise` (BSL 1.1). Includes the Java LicenseManager and
> the build/CI plumbing for two artifacts. No new SMT features — those
> land in RFC-064.

---

## ⚠️ Deferred — restructure of working code with no current payoff

The current single-module `confluent-connector/` works. Splitting it
into three Maven modules, a parent aggregator POM, two profiles, two
assemblies, two CI matrix entries, plus symlinks for back-compat, adds
real complexity to a project no one has asked to restructure. RFC-064
has been rewritten to ship in the existing single-module structure
without this restructure.

**Implement when** there is a paid Java SMT feature ready to ship
(i.e., when [RFC-059's build trigger](059-open-core-split.md#️-build-trigger--do-not-implement-yet)
has fired AND the design partner specifically wants something in the
Connect SMT, not the Rust server). At that point this restructure
becomes necessary; until then it's pure churn.

---

## Decision summary

- Rename `confluent-connector/` → `connect/`. Symlink for one minor
  version, then delete.
- Three Maven modules under a new parent POM:
  - `connect-client/` — Apache 2.0, shared HTTP client to the Rust ingest
    endpoint. Promoted out of the current SMT's `client/` package.
  - `connect-community/` — Apache 2.0, the existing SMT minus the
    HTTP client (now depends on `connect-client`).
  - `connect-enterprise/` — BSL 1.1, contains only the Java
    `LicenseManager` initially. Real enterprise features land in
    RFC-064.
- Two Maven profiles: `-Pcommunity` (default) and `-Penterprise`.
- Two release artifacts: `kafka-connect-contractgate-community-X.Y.Z.zip`
  and `kafka-connect-contractgate-enterprise-X.Y.Z.zip`. Community goes
  to Confluent Hub; enterprise stays on a private download page.

---

## Target layout

```
connect/                                           (was: confluent-connector/)
├── pom.xml                                        parent aggregator POM
├── README.md                                      "this is a multi-module Maven project"
├── LICENSE                                        Apache 2.0 (parent)
├── LICENSE-BSL                                    BSL 1.1 (referenced by connect-enterprise)
│
├── connect-client/
│   ├── pom.xml                                    artifactId: contractgate-connect-client
│   ├── src/main/java/io/datacontractgate/connect/client/
│   │   ├── ContractGateClient.java                (moved from current SMT)
│   │   ├── IngestRequest.java
│   │   ├── IngestResponse.java
│   │   └── ViolationDetail.java
│   └── src/test/java/...                          unit tests for the HTTP client
│
├── connect-community/
│   ├── pom.xml                                    artifactId: kafka-connect-contractgate-community
│   ├── src/main/java/io/datacontractgate/connect/smt/
│   │   ├── ContractGateValidator.java             (current SMT)
│   │   └── ContractGateValidatorConfig.java       (current config)
│   ├── src/main/resources/
│   │   └── META-INF/services/...                  SMT registration
│   ├── src/main/assembly/
│   │   └── package.xml                            confluent-hub layout zip
│   ├── manifest.json                              moved from current dir
│   ├── config/                                    moved from current dir
│   └── src/test/java/...                          existing SMT tests
│
├── connect-enterprise/
│   ├── pom.xml                                    artifactId: kafka-connect-contractgate-enterprise
│   ├── LICENSE-HEADER.txt                         BSL SPDX header template
│   ├── src/main/java/io/datacontractgate/connect/enterprise/
│   │   ├── license/
│   │   │   ├── LicenseManager.java                RFC-060 client
│   │   │   ├── LicenseState.java
│   │   │   ├── LicenseCache.java                  on-disk offline-token persistence
│   │   │   └── SignedTokenVerifier.java           Ed25519 verify
│   │   └── README.md                              "this is BSL-licensed"
│   ├── src/main/resources/
│   │   └── license-signing-key-2026.pub           Ed25519 public key
│   └── src/test/java/...                          LicenseManager unit + integration tests
```

The `kafka-connect-contractgate-0.1.0.zip` packaging assembly stays in
`connect-community/` (only the community artifact ships to Confluent
Hub). A separate assembly in `connect-enterprise/` produces the
enterprise zip, included via Maven assembly plugin.

---

## Parent POM (`connect/pom.xml`)

```xml
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>

  <groupId>io.datacontractgate</groupId>
  <artifactId>contractgate-connect-parent</artifactId>
  <version>0.2.0</version>
  <packaging>pom</packaging>

  <name>ContractGate Kafka Connect — Parent</name>
  <description>Parent POM for the ContractGate Kafka Connect modules.</description>
  <url>https://datacontractgate.com/docs/kafka-connect</url>

  <properties>
    <java.version>11</java.version>
    <maven.compiler.source>${java.version}</maven.compiler.source>
    <maven.compiler.target>${java.version}</maven.compiler.target>
    <project.build.sourceEncoding>UTF-8</project.build.sourceEncoding>

    <!-- Pinned versions used across modules -->
    <kafka.version>3.6.0</kafka.version>
    <jackson.version>2.15.2</jackson.version>
    <slf4j.version>2.0.9</slf4j.version>
    <junit.version>5.10.0</junit.version>
    <mockito.version>5.7.0</mockito.version>
  </properties>

  <modules>
    <module>connect-client</module>
    <module>connect-community</module>
  </modules>

  <profiles>
    <profile>
      <id>community</id>
      <activation>
        <activeByDefault>true</activeByDefault>
      </activation>
      <!-- community modules already listed above; nothing extra -->
    </profile>

    <profile>
      <id>enterprise</id>
      <modules>
        <module>connect-enterprise</module>
      </modules>
    </profile>
  </profiles>

  <build>
    <pluginManagement>
      <!-- shared plugin versions (compiler, surefire, assembly, etc.) -->
    </pluginManagement>
  </build>
</project>
```

**Why "default = community":** `mvn package` from a fresh checkout
builds only community modules. Enterprise code stays out of the default
dep graph and out of casual contributors' builds. RFC-059 invariant.

**Why enterprise activates via additional module, not classifier:**
classifiers would put enterprise classes inside the community jar. The
profile-adds-module pattern keeps the binary boundary clean — community
jar has zero `enterprise/` classes, period.

---

## Module POMs (sketch)

**`connect-client/pom.xml`:**
- `artifactId: contractgate-connect-client`
- `packaging: jar`
- Depends on: jackson-databind (provided), slf4j-api (provided),
  `java.net.http.HttpClient` (JDK 11+ stdlib — no Apache HttpClient
  dependency).
- License: Apache 2.0.

**`connect-community/pom.xml`:**
- `artifactId: kafka-connect-contractgate-community`
- `packaging: jar` (+ assembly for the Confluent Hub zip)
- Depends on:
  - `contractgate-connect-client` (compile)
  - `connect-api` (provided), `connect-transforms` (provided)
- License: Apache 2.0.
- Assembly produces `target/kafka-connect-contractgate-community-X.Y.Z.zip`
  with the Confluent Hub layout (`manifest.json` at root, `lib/`,
  `etc/`, etc.).

**`connect-enterprise/pom.xml`:**
- `artifactId: kafka-connect-contractgate-enterprise`
- `packaging: jar` (+ assembly for the private-download zip)
- Depends on:
  - `contractgate-connect-client` (compile)
  - `kafka-connect-contractgate-community` (compile — enterprise builds
    on top of community)
  - `connect-api` (provided)
  - **Ed25519 verify:** bouncycastle-bcprov-jdk18on (compile, ~5 MB —
    shaded into the enterprise jar so customers don't need to install
    BC themselves).
- License: BSL 1.1. POM `<licenses>` block points to the BSL text.
- Every source file gets an SPDX header (RFC-059 invariant).
- Assembly produces `target/kafka-connect-contractgate-enterprise-X.Y.Z.zip`.

---

## Java LicenseManager (this RFC)

`io.datacontractgate.connect.enterprise.license.LicenseManager` mirrors
the Rust version from RFC-061. Same protocol (RFC-060), same env-var
names, same cache semantics.

```java
public final class LicenseManager {
    public static LicenseManager fromEnv() throws LicenseConfigException;
    public LicenseState state();
    public boolean has(String feature);
    public void refreshNow() throws LicenseRefreshException;
    public void close();    // cancels the background refresh thread
}

public sealed interface LicenseState
        permits Valid, Grace, Invalid, Unconfigured {
    record Valid(String licenseId, Instant expiresAt, Set<String> features) implements LicenseState {}
    record Grace(Set<String> features, Instant graceUntil) implements LicenseState {}
    record Invalid(String reason) implements LicenseState {}
    record Unconfigured() implements LicenseState {}
}
```

**Single instance per Connect worker JVM.** Connect tasks share the
`LicenseManager` via a static holder; the holder lazy-initializes from
env on first access. Multiple tasks in the same worker only ping home
once per 24h, not once per task.

**Why not Spring Security:** Spring is overkill for one HTTP POST + JWT
verify + 24h timer. The JDK 11 `HttpClient`, Jackson, and Bouncy Castle
do everything we need with ~300 lines of focused code. Spring would add
40+ MB to the enterprise zip and a steep startup cost on every Connect
task. Reject.

`SignedTokenVerifier` parses the JWT manually (header.payload.signature
base64url-split), verifies the EdDSA signature with Bouncy Castle, and
checks `iss`, `aud`, `exp`, `kid`. Doesn't use a JWT library — most
Java JWT libs (nimbus, jose4j) don't support EdDSA out of the box
without extra setup, and the parse is ~30 lines.

---

## Backwards compatibility

The current single artifact is `kafka-connect-contractgate-0.1.0.zip`.
After this RFC the equivalent is
`kafka-connect-contractgate-community-0.2.0.zip`.

Steps to keep current users from breaking:

1. Symlink at repo root: `confluent-connector → connect`. Existing CI,
   Docker images, and external docs continue to resolve.
2. Symlink inside `connect/`: also publish the community zip under the
   old name as an alias for one release:
   `kafka-connect-contractgate-0.2.0.zip` → identical bytes to
   `-community-` zip, just renamed by the assembly. Maven assembly
   plugin supports producing two assemblies from one module.
3. SMT class name stays `io.datacontractgate.connect.smt.ContractGateValidator`.
   No customer config changes required.
4. Config keys unchanged.
5. The `0.1.0 → 0.2.0` version bump signals the structural change.
   Release notes: "structural refactor, no behavior change; old artifact
   name still produced for one release."

In the **next** minor release (0.3.0), drop both symlinks. Document in
that release's notes.

---

## CI changes

Existing CI presumably runs `mvn package` against
`confluent-connector/`. Replace with:

```yaml
- name: Build Connect (community)
  working-directory: connect
  run: mvn --batch-mode -Pcommunity verify

- name: Build Connect (enterprise)
  working-directory: connect
  run: mvn --batch-mode -Penterprise verify
```

Both must pass for the PR to merge. Same gating philosophy as RFC-061's
Rust side.

**Dependency-graph check (RFC-059 invariant):**

```yaml
- name: Verify community jar has no enterprise classes
  working-directory: connect
  run: |
    mvn -Pcommunity package -pl connect-community -am -q
    if unzip -l connect-community/target/kafka-connect-contractgate-community-*.jar \
       | grep -i 'enterprise/'; then
      echo "FAIL: enterprise classes leaked into community jar"; exit 1
    fi
```

---

## Removing old files

After the move, delete:
- `confluent-connector/src/` (contents migrated)
- `confluent-connector/pom.xml` (replaced by parent + children)
- `confluent-connector/manifest.json` (moved into `connect-community/`)
- `confluent-connector/config/` (moved into `connect-community/`)
- `confluent-connector/target/` (gitignored anyway)

Keep `confluent-connector/` as a symlink to `connect/` until 0.3.0
release.

**File deletes need confirmation per global rule** — Sonnet asks Alex
before running `git rm` on anything in `confluent-connector/`.

---

## Tests

- Unit: `SignedTokenVerifier` — happy path, expired, wrong `kid`, wrong
  signature, malformed base64.
- Unit: `LicenseCache` — write/read round-trip, corrupt file handling,
  missing-file handling.
- Integration: `LicenseManager.fromEnv()` against a WireMock server
  returning each documented response shape. Asserts state transitions,
  cache writes, 24h refresh scheduling.
- Integration: existing `ContractGateValidatorTest` still passes after
  the package split (regression).
- Assembly: the community zip and enterprise zip both unzip into a
  valid Confluent Hub layout (manifest.json validates).

---

## Out of scope

- Real enterprise SMT features (dynamic reload, DLQ routing) — RFC-064.
- Confluent Hub publishing automation — separate release-engineering
  RFC.
- Private artifact hosting for the enterprise jar — separate release-
  engineering RFC.
- Cross-version compatibility testing matrix — separate ops RFC if a
  customer reports a Kafka version mismatch.

---

## Acceptance Criteria

1. `mvn -f connect/pom.xml -Pcommunity verify` produces
   `kafka-connect-contractgate-community-*.zip` with no `enterprise/`
   classes inside.
2. `mvn -f connect/pom.xml -Penterprise verify` produces both community
   and enterprise zips.
3. The enterprise zip's `LicenseManager`, given a valid staging license
   key via env, logs `License validated for ...` within 5s of class load.
4. Without a license, the enterprise zip loads but
   `LicenseManager.has("anything")` returns false; community SMT
   behavior is unaffected.
5. Existing SMT integration test (`ContractGateValidatorTest`) still
   passes against the restructured `connect-community` module.
6. CI dependency-graph check (community jar has no enterprise classes)
   passes.
7. Symlinks `confluent-connector → connect` (top-level) and the alias
   zip name `kafka-connect-contractgate-0.2.0.zip` both exist.
8. `docs/connect-reference.md` updated to describe community vs
   enterprise artifacts (new section, not a new file — the existing
   `confluent-connector/README.md` content gets moved + expanded).

**Sonnet asks Alex before deleting any files** under
`confluent-connector/`.
