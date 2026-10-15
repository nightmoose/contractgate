# Vendored ODCS JSON Schema

`odcs-json-schema-v3.1.0.json` is the official Open Data Contract Standard
v3.1.0 JSON Schema, copied unmodified from
<https://github.com/bitol-io/open-data-contract-standard/blob/main/schema/odcs-json-schema-v3.1.0.json>
(Bitol project, Linux Foundation AI & Data). It is byte-identical to the copy
CsvPath Framework (`csvpath==0.0.630`) bundles for its ODCS loader.

Licensed under the Apache License 2.0; see `LICENSE` in this directory.
Used only by tests (`src/tests.rs`, `odcs_strict_tests`) to check that DCG
exports validate against the strict schema.
