# License exceptions

DataContractGate and AISchemaGate enforce an MIT-only dependency policy.

| Ecosystem | Check | Allow |
|---|---|---|
| Rust | `cargo deny check licenses` (`deny.toml`) | `MIT` |
| npm | `license-checker-rseidelsohn --production --onlyAllow MIT` | `MIT` |
| Python | `pip-licenses --allow-only 'MIT;MIT License'` | `MIT` |

`MIT License` is included for Python because `pip-licenses` prints that classifier string for some MIT distributions (`h11`, `PyYAML`, the SDKs). It is the MIT license, not a second grant.

A Rust crate licensed `MIT OR …` passes, because MIT can be selected. `Apache-2.0` and the expression `MIT OR Apache-2.0` are not on the allow list. ISC, BSD-2-Clause, BSD-3-Clause, and 0BSD are not on the npm allow list either. Each package below is a reviewed exception. A new non-MIT package fails the check until it is added here and to the matching file under `scripts/license/`.

`about.toml` and `docs/data-room/dependency-licenses.md` are the wider diligence inventory. They are not this allow list.

Private npm packages (`dashboard`, `zapier`, `ops/dogfood`) set `"license": "MIT"`. The checker still labels a `private: true` package `UNLICENSED`, so the npm job passes `--excludePrivatePackages`. Those packages are this repository, under the MIT `LICENSE`.

Versions below are what the checkers reported when this list was written. npm exceptions are by package name, so a later patch of the same package stays excluded. A new package name does not.

## Counts

| Ecosystem | Exceptions |
|---|---|
| Rust | 33 crates |
| npm | 29 packages |
| Python | 5 distributions, in each of the three projects |

`ops/dogfood` has no production dependencies. The checker reports that no packages were found and exits 0.

## Rust

`cargo deny` against the workspace lockfile. Parents are the shortest path from that report.

ICU4X note, for every Unicode-3.0 row: crates.io lists `idna` 0.5.0 as `MIT OR Apache-2.0`, and that release does not use these crates. `url` 2.5 depends on `idna` 1.x, which does. That older `idna` is not a drop-in. Effort for the set is large.

| Package | License | Why it is used | MIT replacement candidates | Effort |
|---|---|---|---|---|
| `icu_collections` 2.2.0 | Unicode-3.0 | ICU4X data used by `idna` 1.x via `url` | none found | large |
| `icu_locale_core` 2.2.0 | Unicode-3.0 | ICU4X locale types on that same `idna` path | none found | large |
| `icu_normalizer` 2.2.0 | Unicode-3.0 | ICU4X normalizer on that same `idna` path | none found | large |
| `icu_normalizer_data` 2.2.0 | Unicode-3.0 | Normalizer data tables for `icu_normalizer` | none found | large |
| `icu_properties` 2.2.0 | Unicode-3.0 | Unicode property data on that same `idna` path | none found | large |
| `icu_properties_data` 2.2.0 | Unicode-3.0 | Property data tables for `icu_properties` | none found | large |
| `icu_provider` 2.2.0 | Unicode-3.0 | ICU4X data provider on that same `idna` path | none found | large |
| `litemap` 0.8.2 | Unicode-3.0 | ICU4X map used by the crates above | none found | large |
| `potential_utf` 0.1.5 | Unicode-3.0 | ICU4X UTF helper used by `icu_collections` | none found | large |
| `tinystr` 0.8.3 | Unicode-3.0 | ICU4X string type used by `icu_locale_core` | none found | large |
| `writeable` 0.6.3 | Unicode-3.0 | ICU4X write trait used by the crates above | none found | large |
| `yoke` 0.8.3 | Unicode-3.0 | ICU4X zero-copy yoke used by the crates above | none found | large |
| `yoke-derive` 0.8.2 | Unicode-3.0 | Derive for `yoke` | none found | large |
| `zerofrom` 0.1.8 | Unicode-3.0 | ICU4X zero-from helper used by the crates above | none found | large |
| `zerofrom-derive` 0.1.7 | Unicode-3.0 | Derive for `zerofrom` | none found | large |
| `zerotrie` 0.2.4 | Unicode-3.0 | ICU4X trie used by the crates above | none found | large |
| `zerovec` 0.11.6 | Unicode-3.0 | ICU4X vector used by the crates above | none found | large |
| `zerovec-derive` 0.11.3 | Unicode-3.0 | Derive for `zerovec` | none found | large |
| `unicode-ident` 1.0.24 | `(MIT OR Apache-2.0) AND Unicode-3.0` | Identifier tables for `syn` and other macros. MIT covers the code; the tables add Unicode-3.0 | none found | large |
| `matchit` 0.8.4 | MIT AND BSD-3-Clause | Router used by axum 0.8. crates.io lists 0.7.0 as MIT; 0.8 and later are MIT AND BSD-3-Clause. axum 0.8 depends on 0.8 | none found that axum 0.8 can use | large |
| `metrics-exporter-prometheus` 0.18.3 | MIT AND Apache-2.0 | Direct dependency. Prometheus scrape endpoint. crates.io lists 0.18.0 and 0.18.1 as MIT; 0.18.3 is MIT AND Apache-2.0 | `metrics-exporter-prometheus` 0.18.1 (MIT on crates.io) | small |
| `ring` 0.17.14 | Apache-2.0 AND ISC | rustls crypto provider used by reqwest and sqlx. crates.io lists current `aws-lc-rs` as `ISC AND (Apache-2.0 OR ISC)`, which is not MIT | none found | large |
| `rustls-webpki` 0.103.16 | ISC | Certificate verification for that rustls stack. crates.io listings for 0.103 and 0.104 alphas are ISC | none found | large |
| `untrusted` 0.9.0 | ISC | Parser input type used by `ring` and `rustls-webpki` | none found | large |
| `ryu` 1.0.23 | Apache-2.0 OR BSL-1.0 | Float formatting inside `csv`, `serde_urlencoded`, and `serde_yaml` | none found | large |
| `curve25519-dalek` 4.1.3 | BSD-3-Clause | Curve25519 used by `ed25519-dalek` via jsonwebtoken. crates.io listings, including 5.0.0, are BSD-3-Clause | none found | large |
| `ed25519-dalek` 2.2.0 | BSD-3-Clause | Ed25519 verify path inside jsonwebtoken. crates.io listings, including 3.0.0, are BSD-3-Clause. `ed25519-consensus` 2.1.0 is MIT OR Apache-2.0 and is a different API; jsonwebtoken does not use it | none found | large |
| `simple_asn1` 0.6.4 | ISC | ASN.1 parser inside jsonwebtoken. Every crates.io release checked is ISC | none found | large |
| `subtle` 2.6.1 | BSD-3-Clause | Constant-time helpers used by the dalek crates and rustls | none found | large |
| `foldhash` 0.1.5 and 0.2.0 | Zlib | Hasher selected by hashbrown. Every crates.io release checked is Zlib | none found | large |
| `sketches-ddsketch` 0.3.1 | Apache-2.0 | Histogram sketch used by `metrics-exporter-prometheus`. crates.io releases are Apache-2.0 | none found | large |
| `sync_wrapper` 1.0.2 | Apache-2.0 | Task wrapper used by axum, tower, and reqwest. Every crates.io release checked is Apache-2.0 | none found | large |
| `webpki-roots` 0.26.11 and 1.0.10 | CDLA-Permissive-2.0 | Bundled Mozilla CA set. reqwest feature `rustls-tls` and sqlx feature `runtime-tokio-rustls` (which selects `tls-rustls-ring-webpki`) depend on it. `rustls-native-certs` 0.8 is `Apache-2.0 OR ISC OR MIT` on crates.io, so MIT can be selected. reqwest has `rustls-tls-native-roots`. sqlx has `tls-rustls-ring-native-roots` (used with `runtime-tokio`, not with `runtime-tokio-rustls`). That switch drops this crate and still uses `ring` | `rustls-native-certs` (OS trust store, MIT-selectable) | medium |

## npm

`npm ci --omit=dev`, then `license-checker-rseidelsohn` 5.0.1 with `--production --onlyAllow MIT`. `sdks/aischemagate/typescript` has no lockfile (the release workflow deletes one) and is installed with `npm install --omit=dev --no-package-lock`. The linux x64 optional binaries are the ones this CI image installs. Other `@img/sharp-*` platform packages are not installed here, so they were not scanned.

| Package | License | Why it is used | MIT replacement candidates | Effort |
|---|---|---|---|---|
| `typescript` 5.9.3 | Apache-2.0 | Optional peer of `valibot`, which `tmcp` depends on. npm still installs it for a production install of `mcp`. The server does not import it | none found | large |
| `uri-template-matcher` 1.1.2 | ISC | RFC 6570 matching inside `tmcp` | `uri-template` 2.0.0 on npm is MIT. `tmcp` depends on `uri-template-matcher`, so it is not a drop-in | large |
| `sharp` 0.35.5 | Apache-2.0 | Optional dependency of `next` for image optimization | none found that `next` will call instead | large |
| `@img/sharp-linux-x64` 0.35.5 | Apache-2.0 | linux x64 build of `sharp`, installed on the CI image | none found | large |
| `@img/sharp-libvips-linux-x64` 1.3.4 | LGPL-3.0-or-later | libvips binary bundled for that `sharp` build | none found. `jimp` 1.6.1 on npm is MIT and is not a `next` image optimizer | large |
| `@opentelemetry/api` 1.9.1 | Apache-2.0 | Dependency of `@anthropic-ai/sdk` in the dashboard | none found | large |
| `@swc/helpers` 0.5.23 | Apache-2.0 | Dependency of `next` | none found | large |
| `baseline-browser-mapping` 2.11.24 | Apache-2.0 | Dependency of `next` | none found | large |
| `caniuse-lite` 1.0.30001788 | CC-BY-4.0 | Browser-compatibility data shipped with `next` | none found | large |
| `argparse` 3.0.3 | PSF-2.0 | Dependency of dashboard `js-yaml` 5.5.0 | none found | large |
| `argparse` 2.0.1 | Python-2.0 | Dependency of `js-yaml` 4.3.2 in both TypeScript SDKs | none found. The `yaml` package on npm is ISC, not MIT | large |
| `picocolors` 1.1.1 | ISC | Dependency of `postcss`, which `next` depends on. `kleur` 4.1.5 and `colorette` 2.0.20 on npm are MIT and are not what `postcss` imports | none found | large |
| `source-map-js` 1.2.2 | BSD-3-Clause | Dependency of `postcss` | none found | large |
| `tslib` 2.8.1 | 0BSD | TypeScript helper runtime used by `@swc/helpers` and the Supabase client | none found | large |
| `fast-sha256` 1.3.0 | Unlicense | Dependency of `standardwebhooks`, used by `@anthropic-ai/sdk` and `resend`. `@noble/hashes` 2.4.0 on npm is MIT and is not the module those packages import | none found | large |
| `postal-mime` 2.7.6 | MIT-0 | Dependency of `resend`. MIT-0 is a different SPDX identifier | none found | large |
| `semver` 7.8.5 and 7.7.2 | ISC | `sharp` (dashboard) and `zapier-platform-core` / its tree. `compare-versions` 6.1.1 on npm is MIT and does not replace the `semver` import | none found | large |
| `detect-libc` 2.1.2 | Apache-2.0 | libc detection inside `sharp` | none found | large |
| `@playwright/test` 1.64.1 | Apache-2.0 | Optional peer of `next`. Installed because the dashboard lockfile records it (also a dashboard devDependency) | none found | large |
| `playwright` 1.64.1 | Apache-2.0 | Dependency of `@playwright/test` | none found | large |
| `playwright-core` 1.64.1 | Apache-2.0 | Dependency of `playwright` | none found | large |
| `zapier-platform-core` 19.1.0 | Zapier Platform Agreement (`SEE LICENSE IN LICENSE`; the checker reports `Custom: LICENSE`) | Direct dependency of the Zapier integration | none found | large |
| `zapier-platform-schema` 19.1.0 | Zapier Platform Agreement, same license file | Dependency of `zapier-platform-core` | none found | large |
| `@zapier/secret-scrubber` 1.1.6 | ISC | Dependency of `zapier-platform-core` | none found | large |
| `dotenv` 17.2.1 | BSD-2-Clause | Dependency of `zapier-platform-core` | none found | large |
| `oauth-sign` 0.9.0 | Apache-2.0 | OAuth signing inside `zapier-platform-core` | none found | large |
| `json-metaschema` 1.3.0 | Public Domain | JSON Schema meta-schemas inside `zapier-platform-schema` | none found | large |
| `sha.js` 2.4.12 | MIT AND BSD-3-Clause | Hash implementation used by `@zapier/secret-scrubber` | none found | large |
| `inherits` 2.0.4 | ISC | Node `util.inherits` helper used under `@zapier/secret-scrubber` | none found | large |
| `webidl-conversions` 3.0.1 | BSD-2-Clause | Dependency of `whatwg-url`, which `node-fetch` inside `zapier-platform-core` uses | none found | large |

`argparse` is one excluded name and two license strings. `semver` is one excluded name. The table lists both reviewed versions. The npm count of 29 is package names in `scripts/license/npm-exclude.txt`.

## Python

`pip-licenses` 5.5.5 on a fresh venv. The root hooks package is installed after the local `sdks/python` tree, so the check sees this repository's SDK rather than an older index build. `sdks/aischemagate/python` is installed the same way. Required dependencies only. Optional extras (`langchain`, `llamaindex`, `instructor`) are not in the check.

The same five distributions appear in all three trees. `aischemagate` and `contractgate-hooks` themselves report as MIT.

| Package | License string from pip-licenses | Why it is used | MIT replacement candidates | Effort |
|---|---|---|---|---|
| `httpx` 0.28.1 | BSD License | HTTP client for the Python SDK (and therefore the root hooks package and the AISchemaGate package) | none found. `urllib3` 2.8.0 on PyPI is MIT and does not provide this async client | large |
| `httpcore` 1.0.9 | BSD-3-Clause | Transport used by `httpx` | none found | large |
| `idna` 3.20 | BSD-3-Clause | Internationalized domain names for `httpx`. The current PyPI release is BSD-3-Clause | none found | large |
| `certifi` 2026.10.23 | Mozilla Public License 2.0 (MPL 2.0) | CA bundle used by `httpx`. `truststore` 0.10.5 on PyPI is MIT and reads the OS trust store. It requires Python 3.10, and these packages still declare `requires-python >= 3.9`. `httpx` 0.28 still depends on `certifi` | `truststore` 0.10.5, after a Python floor change, and it still is not a drop-in for the `certifi` import inside `httpx` | large |
| `typing_extensions` 4.16.0 | PSF-2.0 | Backport pulled in for Python 3.9. The current PyPI release is PSF-2.0 | none found | large |

## What this list does not cover

- Optional Python extras on `aischemagate`.
- `@img/sharp-*` binaries for operating systems other than linux x64. The license workflow runs on `ubuntu-24.04`, which installs the linux x64 optional packages only.
- `docs/data-room/dependency-licenses.md`. It was not regenerated. The CI allow list is this page and `deny.toml`.
