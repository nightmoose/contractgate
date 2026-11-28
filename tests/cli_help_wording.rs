//! The installed binary is `contractgate`, not `cg`: help text and
//! examples must say so. Regression for the clean-install report: several
//! `--help` examples said `cg synth` / `cg test` / `cg check-table` etc.,
//! which did not exist as a binary or alias. Also checks no `--help` output
//! contains an em dash (public copy uses plain ASCII punctuation).

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_contractgate")
}

fn help(args: &[&str]) -> String {
    let out = Command::new(bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("running {args:?}: {e}"));
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A standalone `cg` token (not part of a longer identifier like
/// `cg-{contract_id}-raw` or `CG_KAFKA_BROKER`, which are real product
/// names, not the old CLI alias).
fn has_bare_cg_command(text: &str) -> bool {
    text.split_whitespace().any(|w| {
        let trimmed = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        trimmed == "cg"
    })
}

#[test]
fn top_level_help_says_contractgate_not_cg() {
    let text = help(&["--help"]);
    assert!(!has_bare_cg_command(&text), "{text}");
    assert!(text.contains("contractgate"), "{text}");
    assert!(!text.contains('\u{2014}'), "em dash in --help:\n{text}");
}

#[test]
fn every_subcommand_help_says_contractgate_not_cg_and_has_no_em_dash() {
    let top = help(&["--help"]);
    let commands: Vec<&str> = top
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_whitespace().next())
        .filter(|c| *c != "help")
        .collect();
    assert!(!commands.is_empty(), "{top}");

    for cmd in commands {
        let text = help(&[cmd, "--help"]);
        assert!(
            !has_bare_cg_command(&text),
            "`{cmd} --help` still says bare `cg`:\n{text}"
        );
        assert!(
            !text.contains('\u{2014}'),
            "`{cmd} --help` contains an em dash:\n{text}"
        );
    }
}
