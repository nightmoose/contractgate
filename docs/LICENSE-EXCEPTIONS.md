# License exceptions

DataContractGate and AISchemaGate allow MIT, ISC, BSD-2-Clause, BSD-3-Clause, and 0BSD.

| Ecosystem | Check | Allow |
|---|---|---|
| Rust | `cargo deny check licenses` (`deny.toml`) | `MIT`, `ISC`, `BSD-2-Clause`, `BSD-3-Clause`, `0BSD` |
| npm | `license-checker-rseidelsohn --production --onlyAllow` | `MIT;ISC;BSD-2-Clause;BSD-3-Clause;0BSD` |
| Python | `pip-licenses --allow-only` | classifier names below, plus the SPDX strings the tool prints |

Python uses the classifier tails `pip-licenses` reads from PyPI: `MIT License`, `BSD License` (this is the classifier for both BSD-2-Clause and BSD-3-Clause), `ISC License (ISCL)`, and `Zero-Clause BSD (0BSD)`. The same command also allows `MIT`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, and `0BSD`, because a distribution that sets `License-Expression` is reported as that SPDX string instead. `idna` and `httpcore` report `BSD-3-Clause`. `httpx` reports `BSD License`.

A Rust crate licensed `MIT OR …` passes, because MIT can be selected. `Apache-2.0` and the expression `MIT OR Apache-2.0` are not on the allow list. Each package below is a reviewed exception. A new package outside the allow list fails the check until it is added here and to the matching file under `scripts/license/`.

`about.toml` and `docs/data-room/dependency-licenses.md` are the wider diligence inventory. They are not this allow list.

Private npm packages (`dashboard`, `zapier`, `ops/dogfood`) set `"license": "MIT"`. The checker still labels a `private: true` package `UNLICENSED`, so the npm job passes `--excludePrivatePackages`. Those packages are this repository, under the MIT `LICENSE`.

Versions below are what the checkers reported when this list was written. npm exceptions are by package name, so a later patch of the same package stays excluded. A new package name does not.

## Counts

| Ecosystem | Exceptions |
|---|---|
| Rust | 25 crates |
| npm | 15 packages |
| Python | 2 distributions, in each of the three projects |

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
| `ring` 0.17.14 | Apache-2.0 AND ISC | Accepted. rustls crypto; no MIT or ISC provider is viable. | none found | large |
| `ryu` 1.0.23 | Apache-2.0 OR BSL-1.0 | Float formatting inside `csv`, `serde_urlencoded`, and `serde_yaml` | none found | large |
| `foldhash` 0.1.5 and 0.2.0 | Zlib | Hasher selected by hashbrown. Every crates.io release checked is Zlib | none found | large |
| `sketches-ddsketch` 0.3.1 | Apache-2.0 | Accepted. Via the metrics exporter; every version is Apache-2.0. | none found | large |
| `sync_wrapper` 1.0.2 | Apache-2.0 | Accepted. axum and tower stack. | none found | large |
| `webpki-roots` 0.26.11 and 1.0.10 | CDLA-Permissive-2.0 | Bundled Mozilla CA set. reqwest feature `rustls-tls` and sqlx feature `runtime-tokio-rustls` (which selects `tls-rustls-ring-webpki`) depend on it. `rustls-native-certs` 0.8 is `Apache-2.0 OR ISC OR MIT` on crates.io, so MIT can be selected. reqwest has `rustls-tls-native-roots`. sqlx has `tls-rustls-ring-native-roots` (used with `runtime-tokio`, not with `runtime-tokio-rustls`). That switch drops this crate and still uses `ring` | `rustls-native-certs` (OS trust store, MIT-selectable) | medium |

## npm

`npm ci --omit=dev --omit=peer`, then a prune of packages that are only devDependencies or peerDependencies, then `license-checker-rseidelsohn` 5.0.1 with `--production` and `--onlyAllow 'MIT;ISC;BSD-2-Clause;BSD-3-Clause;0BSD'`. That drops optional peers such as Playwright and the TypeScript peer of `valibot`. The checker treats an SPDX `AND` as satisfied when every license in it is allowed, so `sha.js` (`MIT AND BSD-3-Clause`) is not an exception. `sdks/aischemagate/typescript` has no committed lockfile. The linux x64 optional binaries are the ones this CI image installs. Other `@img/sharp-*` platform packages are not installed here, so they were not scanned.

| Package | License | Why it is used | MIT replacement candidates | Effort |
|---|---|---|---|---|
| `sharp` 0.35.5 | Apache-2.0 | Accepted. Next.js optional dependency, installed but not executed; images unoptimized. | none found | large |
| `@img/sharp-linux-x64` 0.35.5 | Apache-2.0 | Accepted. Next.js optional dependency, installed but not executed; images unoptimized. | none found | large |
| `@img/sharp-libvips-linux-x64` 1.3.4 | LGPL-3.0-or-later | Accepted. Next.js optional dependency, installed but not executed; images unoptimized. | none found | large |
| `@opentelemetry/api` 1.9.1 | Apache-2.0 | Accepted. Hard dependency of `@anthropic-ai/sdk`. | none found | large |
| `@swc/helpers` 0.5.23 | Apache-2.0 | Accepted. Hard dependency of Next.js. | none found | large |
| `baseline-browser-mapping` 2.11.24 | Apache-2.0 | Accepted. Hard dependency of Next.js. | none found | large |
| `caniuse-lite` 1.0.30001788 | CC-BY-4.0 | Browser-compatibility data shipped with `next` | none found | large |
| `argparse` 3.0.3 | PSF-2.0 | Dependency of dashboard `js-yaml` 5.5.0 | none found | large |
| `argparse` 2.0.1 | Python-2.0 | Dependency of `js-yaml` 4.3.2 in both TypeScript SDKs | none found | large |
| `fast-sha256` 1.3.0 | Unlicense | Dependency of `standardwebhooks`, used by `@anthropic-ai/sdk` and `resend`. `@noble/hashes` 2.4.0 on npm is MIT and is not the module those packages import | none found | large |
| `postal-mime` 2.7.6 | MIT-0 | Dependency of `resend`. MIT-0 is a different SPDX identifier | none found | large |
| `detect-libc` 2.1.2 | Apache-2.0 | Accepted. Next.js optional dependency, installed but not executed; images unoptimized. | none found | large |
| `zapier-platform-core` 19.1.0 | Zapier Platform Agreement (`SEE LICENSE IN LICENSE`; the checker reports `Custom: LICENSE`) | Accepted. Required for the Zapier integration. | none found | large |
| `zapier-platform-schema` 19.1.0 | Zapier Platform Agreement, same license file | Accepted. Required for the Zapier integration. | none found | large |
| `oauth-sign` 0.9.0 | Apache-2.0 | Accepted. Required for the Zapier integration. | none found | large |
| `json-metaschema` 1.3.0 | Public Domain | JSON Schema meta-schemas inside `zapier-platform-schema` | none found | large |

`argparse` is one excluded name and two license strings. The npm count of 15 is package names in `scripts/license/npm-exclude.txt`. `metrics-exporter-prometheus` is pinned to 0.18.1, which is MIT, so it is not an exception. Playwright and TypeScript are dev or peer dependencies and are not installed for this check.

## Python

`pip-licenses` 5.5.5 on a fresh venv. The root hooks package is installed after the local `sdks/python` tree, so the check sees this repository's SDK rather than an older index build. `sdks/aischemagate/python` is installed the same way. Required dependencies only. Optional extras (`langchain`, `llamaindex`, `instructor`) are not in the check.

The same two distributions appear in all three trees. `httpx` reports the classifier `BSD License`. `httpcore` and `idna` report `BSD-3-Clause`. Those are allowed. `aischemagate` and `contractgate-hooks` themselves report as MIT.

| Package | License string from pip-licenses | Why it is used | MIT replacement candidates | Effort |
|---|---|---|---|---|
| `certifi` 2026.10.23 | Mozilla Public License 2.0 (MPL 2.0) | CA bundle used by `httpx`. `truststore` 0.10.5 on PyPI is MIT and reads the OS trust store. It requires Python 3.10, and these packages still declare `requires-python >= 3.9`. `httpx` 0.28 still depends on `certifi` | `truststore` 0.10.5, after a Python floor change, and it still is not a drop-in for the `certifi` import inside `httpx` | large |
| `typing_extensions` 4.16.0 | PSF-2.0 | Backport pulled in for Python 3.9. The current PyPI release is PSF-2.0 | none found | large |

## What this list does not cover

- Optional Python extras on `aischemagate`.
- `@img/sharp-*` binaries for operating systems other than linux x64. The license workflow runs on `ubuntu-24.04`, which installs the linux x64 optional packages only.
- `docs/data-room/dependency-licenses.md`. It was not regenerated. The CI allow list is this page and `deny.toml`.
