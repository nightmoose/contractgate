//! Tests for `contractgate diff` (local contracts vs the deployed stable version).
//!
//! The gateway is a tiny in-process HTTP stub, so these run with no database
//! or network beyond loopback. Exit codes under test: 0 no drift, 1 drift,
//! 2 error.

use contractgate::cli::{
    commands::diff::{run, DiffArgs},
    config::{CliConfig, GatewayConfig},
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

const ORDERS_ID: &str = "11111111-1111-1111-1111-111111111111";
const EVENTS_ID: &str = "22222222-2222-2222-2222-222222222222";
const LEGACY_ID: &str = "33333333-3333-3333-3333-333333333333";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("cli")
}

/// What the stub gateway serves.
struct Stub {
    /// (id, name) rows for GET /contracts.
    contracts: Vec<(&'static str, &'static str)>,
    /// (id, version, yaml) for GET /contracts/:id/versions/latest-stable.
    /// An id with no entry answers 409 "has no stable version yet", as the
    /// real gateway does for a contract with only drafts.
    stable: Vec<(&'static str, String, String)>,
    /// Answer GET /contracts with a 500.
    list_fails: bool,
}

/// Serve the stub on a loopback port; returns the base URL.
fn serve(stub: Stub) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
            let (status, body) = if path == "/contracts" {
                if stub.list_fails {
                    ("500 Internal Server Error", "boom".to_string())
                } else {
                    let rows: Vec<_> = stub
                        .contracts
                        .iter()
                        .map(|(id, name)| serde_json::json!({"id": id, "name": name}))
                        .collect();
                    ("200 OK", serde_json::to_string(&rows).unwrap())
                }
            } else if let Some(id) = path
                .strip_prefix("/contracts/")
                .and_then(|p| p.strip_suffix("/versions/latest-stable"))
            {
                match stub.stable.iter().find(|(i, _, _)| *i == id) {
                    Some((_, version, yaml)) => (
                        "200 OK",
                        serde_json::json!({"version": version, "yaml_content": yaml}).to_string(),
                    ),
                    None => (
                        "409 Conflict",
                        format!(
                            "{{\"error\":\"Contract {id} has no stable version yet\",\"status\":409}}"
                        ),
                    ),
                }
            } else {
                ("404 Not Found", "{}".to_string())
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    url
}

fn orders_yaml() -> String {
    std::fs::read_to_string(fixtures_dir().join("valid_orders.yaml")).unwrap()
}

fn cfg(url: String) -> CliConfig {
    CliConfig {
        gateway: GatewayConfig { url },
        ..Default::default()
    }
}

fn args(dir: &std::path::Path) -> DiffArgs {
    DiffArgs {
        dir: Some(dir.to_path_buf()),
        name: None,
        fail_on_remote_only: false,
        json: true,
    }
}

/// A temp dir holding `orders.yaml` with the given text.
fn dir_with_orders(yaml: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("orders.yaml"), yaml).unwrap();
    d
}

fn stub_with_orders(deployed: String) -> Stub {
    Stub {
        contracts: vec![(ORDERS_ID, "orders")],
        stable: vec![(ORDERS_ID, "1.0".into(), deployed)],
        list_fails: false,
    }
}

#[test]
fn no_drift_exits_0() {
    let y = orders_yaml();
    let dir = dir_with_orders(&y);
    let url = serve(stub_with_orders(y));
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 0);
}

#[test]
fn line_endings_and_trailing_whitespace_are_not_drift() {
    let y = orders_yaml();
    let noisy = y.replace('\n', "  \r\n") + "\n\n";
    let dir = dir_with_orders(&noisy);
    let url = serve(stub_with_orders(y));
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 0);
}

#[test]
fn changed_field_exits_1() {
    let deployed = orders_yaml();
    let local = deployed.replace("\"cancelled\"]", "\"cancelled\", \"returned\"]");
    assert_ne!(local, deployed, "fixture edit must change the file");
    let dir = dir_with_orders(&local);
    let url = serve(stub_with_orders(deployed));
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 1);
}

#[test]
fn comment_only_change_exits_1() {
    let deployed = orders_yaml();
    let local = format!("# reviewed\n{deployed}");
    let dir = dir_with_orders(&local);
    let url = serve(stub_with_orders(deployed));
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 1);
}

#[test]
fn contract_not_deployed_exits_1() {
    let dir = dir_with_orders(&orders_yaml());
    let url = serve(Stub {
        contracts: vec![],
        stable: vec![],
        list_fails: false,
    });
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 1);
}

#[test]
fn contract_without_stable_version_exits_1() {
    let dir = dir_with_orders(&orders_yaml());
    let url = serve(Stub {
        contracts: vec![(ORDERS_ID, "orders")],
        stable: vec![],
        list_fails: false,
    });
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 1);
}

#[test]
fn remote_only_is_exit_0_unless_flagged() {
    let y = orders_yaml();
    let dir = dir_with_orders(&y);
    let stub = || Stub {
        contracts: vec![(ORDERS_ID, "orders"), (LEGACY_ID, "legacy")],
        stable: vec![(ORDERS_ID, "1.0".into(), y.clone())],
        list_fails: false,
    };
    let url = serve(stub());
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 0);

    let url = serve(stub());
    let mut a = args(dir.path());
    a.fail_on_remote_only = true;
    assert_eq!(run(&a, &cfg(url), "k").unwrap(), 1);
}

#[test]
fn name_filter_checks_only_that_contract() {
    let y = orders_yaml();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("orders.yaml"), &y).unwrap();
    std::fs::copy(
        fixtures_dir().join("valid_user_events.yaml"),
        dir.path().join("events.yaml"),
    )
    .unwrap();
    // user_events is not deployed, so without --name this is drift.
    let stub = || Stub {
        contracts: vec![(ORDERS_ID, "orders"), (EVENTS_ID, "other")],
        stable: vec![(ORDERS_ID, "1.0".into(), y.clone())],
        list_fails: false,
    };
    let url = serve(stub());
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 1);

    let url = serve(stub());
    let mut a = args(dir.path());
    a.name = Some("orders".into());
    assert_eq!(run(&a, &cfg(url), "k").unwrap(), 0);

    let url = serve(stub());
    a.name = Some("missing".into());
    assert_eq!(run(&a, &cfg(url), "k").unwrap(), 2);
}

#[test]
fn unparseable_local_file_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixtures_dir().join("invalid_bad_yaml.yaml"),
        dir.path().join("bad.yaml"),
    )
    .unwrap();
    let url = serve(Stub {
        contracts: vec![],
        stable: vec![],
        list_fails: false,
    });
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 2);
}

#[test]
fn error_wins_over_drift() {
    let dir = dir_with_orders(&format!("# drift\n{}", orders_yaml()));
    std::fs::copy(
        fixtures_dir().join("invalid_bad_yaml.yaml"),
        dir.path().join("bad.yaml"),
    )
    .unwrap();
    let url = serve(stub_with_orders(orders_yaml()));
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 2);
}

#[test]
fn gateway_error_exits_2() {
    let dir = dir_with_orders(&orders_yaml());
    let url = serve(Stub {
        contracts: vec![],
        stable: vec![],
        list_fails: true,
    });
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 2);
}

#[test]
fn unreachable_gateway_exits_2() {
    let dir = dir_with_orders(&orders_yaml());
    // Port 1 on loopback refuses connections.
    let url = "http://127.0.0.1:1".to_string();
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 2);
}

#[test]
fn empty_directory_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let url = serve(Stub {
        contracts: vec![],
        stable: vec![],
        list_fails: false,
    });
    assert_eq!(run(&args(dir.path()), &cfg(url), "k").unwrap(), 2);
}
