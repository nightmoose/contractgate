//! RFC-089 — drift gate for the agent-facing integration playbook.
//!
//! `docs/llm-integration.md` is executed by coding agents, which makes it an API
//! surface: a route that gets renamed or removed turns the doc into confidently
//! wrong instructions. Two assertions keep it honest:
//!
//!   1. every `METHOD /path` cited in the playbook exists in the router
//!      (`src/main.rs` route table);
//!   2. the example contract YAML in the playbook parses and compiles through
//!      the real engine.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Every `/`-prefixed string literal in `src/main.rs` — a superset of the route
/// table (harmless: we only test membership), and immune to the router's
/// multi-line `.route(\n    "/path",\n ...)` formatting.
fn declared_paths() -> HashSet<String> {
    let src = fs::read_to_string(repo_path("src/main.rs")).expect("read src/main.rs");
    let mut out = HashSet::new();
    for line in src.lines() {
        // On a line with no escaped quotes, odd-indexed pieces are literals.
        for (i, piece) in line.split('"').enumerate() {
            if i % 2 == 1 && piece.starts_with('/') {
                out.insert(piece.to_string());
            }
        }
    }
    out
}

/// `METHOD /path` pairs cited in the playbook, normalized: markdown punctuation
/// and query strings stripped; wildcard forms (`/contracts/infer/*`) skipped
/// because they document a route group rather than a route.
fn cited_endpoints(doc: &str) -> Vec<(String, String)> {
    const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
    const TRIM: [char; 8] = ['`', '"', '\'', '(', ')', ',', '.', '|'];

    let tokens: Vec<&str> = doc.split_whitespace().collect();
    let mut out = Vec::new();

    for pair in tokens.windows(2) {
        let method = pair[0].trim_matches(|c: char| !c.is_ascii_alphabetic());
        if !METHODS.contains(&method) {
            continue;
        }
        let mut path = pair[1]
            .trim_matches(|c: char| TRIM.contains(&c))
            .to_string();
        if !path.starts_with('/') {
            continue;
        }
        if let Some(q) = path.find('?') {
            path.truncate(q);
        }
        if path.contains('*') || path.len() < 2 {
            continue;
        }
        out.push((method.to_string(), path));
    }
    out
}

#[test]
fn playbook_only_cites_routes_that_exist() {
    let doc = fs::read_to_string(repo_path("docs/llm-integration.md"))
        .expect("read docs/llm-integration.md");
    let declared = declared_paths();
    let cited = cited_endpoints(&doc);

    assert!(
        cited.len() >= 5,
        "extracted only {} endpoints from the playbook — the extractor is broken, \
         not the doc",
        cited.len()
    );

    let missing: Vec<String> = cited
        .iter()
        .filter(|(_, path)| !declared.contains(path))
        .map(|(m, p)| format!("{m} {p}"))
        .collect();

    assert!(
        missing.is_empty(),
        "docs/llm-integration.md cites routes that do not exist in src/main.rs: {missing:?}\n\
         Fix the doc (or restore the route) — agents execute this file verbatim."
    );
}

#[test]
fn playbook_example_contract_compiles() {
    let doc = fs::read_to_string(repo_path("docs/llm-integration.md"))
        .expect("read docs/llm-integration.md");

    let yaml = doc
        .split("```yaml")
        .skip(1)
        .filter_map(|block| block.split("```").next())
        .find(|block| block.contains("ontology:"))
        .expect("playbook must contain a ```yaml block with an ontology");

    let contract: contractgate::contract::Contract =
        serde_yaml::from_str(yaml).expect("playbook example contract must parse");

    contractgate::validation::CompiledContract::compile(contract)
        .expect("playbook example contract must compile");
}
