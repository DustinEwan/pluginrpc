//! The six core rules, run by the mechanism against the demo profile's reference plugin.
//!
//! Two claims are checked here, and the second is the one that makes the first worth anything. The
//! rules must pass a correct plugin, and each must have a scripted defect that makes exactly that
//! rule report a violation (§12 item 4). A rule with no defect has not been shown to detect the
//! failure it targets, which is the exact fault the audit of both real profiles found in their own
//! suites — one of them had no defect for any rule at all.

use std::path::{Path, PathBuf};
use std::time::Duration;

use plugin_rpc::conformance::{core, core_rules, CoreCheck, Outcome, ProfileSpec, Report};

/// The core rules, in the order `spec/core-rules.json` states them and the order the runner emits.
const CORE: [&str; 6] = [
    "spawn",
    "protocol-version",
    "handshake-schema",
    "unknown-method",
    "framing-intact",
    "process-alive",
];

/// One scripted defect per core rule, in the same order. `DEMO_DEFECT` selects it inside the one
/// reference plugin, so a defect cannot drift from the correct plugin and fire a rule by accident.
const DEFECTS: [(&str, &str); 6] = [
    ("spawn", "spawn"),
    ("version", "protocol-version"),
    ("schema", "handshake-schema"),
    ("unknown", "unknown-method"),
    ("framing", "framing-intact"),
    ("alive", "process-alive"),
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn profile() -> ProfileSpec {
    ProfileSpec::load(fixture("demo-1.profile.json")).expect("the demo profile spec loads")
}

fn run(defect: Option<&str>) -> Report {
    let profile = profile();
    let mut env = Vec::new();
    if let Some(defect) = defect {
        env.push(("DEMO_DEFECT".to_string(), defect.to_string()));
    }
    core::run(&CoreCheck {
        profile: &profile,
        command: vec![
            "/bin/sh".to_string(),
            fixture("demo.sh").display().to_string(),
        ],
        env,
        cwd: None,
        deadline: Duration::from_millis(700),
    })
}

fn rules(report: &Report) -> Vec<&str> {
    report
        .verdicts
        .iter()
        .map(|verdict| verdict.rule.as_str())
        .collect()
}

fn violations(report: &Report) -> Vec<&str> {
    report
        .verdicts
        .iter()
        .filter(|verdict| verdict.outcome == Outcome::Violation)
        .map(|verdict| verdict.rule.as_str())
        .collect()
}

#[test]
fn core_rule_ids_are_the_ones_the_runner_implements() {
    // The runner takes the six ids by position from the committed file. This is the check that keeps
    // the position honest, so a rule added to `spec/core-rules.json` cannot be silently skipped.
    let rules = core_rules();
    let ids: Vec<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
    assert_eq!(ids, CORE, "spec/core-rules.json and the runner disagree");
}

#[test]
fn a_correct_plugin_passes_every_core_rule() {
    let report = run(None);
    assert_eq!(
        rules(&report),
        CORE,
        "the report is the core rules, in the file's order\n{}",
        report.render()
    );
    assert!(
        report.passed(),
        "a correct plugin conforms\n{}",
        report.render()
    );
}

#[test]
fn every_core_rule_has_a_defect_that_fires_it_and_nothing_else() {
    for (defect, rule) in DEFECTS {
        let report = run(Some(defect));
        assert_eq!(
            violations(&report),
            vec![rule],
            "the `{defect}` defect must make `{rule}` report a violation and no other rule\n{}",
            report.render()
        );
        assert!(
            !report.passed(),
            "a plugin with a defect does not conform\n{}",
            report.render()
        );
    }
}

#[test]
fn a_plugin_that_never_starts_leaves_the_other_rules_not_applicable_and_never_a_pass() {
    // §12 item 3: a rule the suite could not run is counted not applicable. Reading the four rules
    // after `spawn` as passes is how a dead plugin scores 5/6.
    let report = run(Some("spawn"));
    assert_eq!(violations(&report), vec!["spawn"], "{}", report.render());
    let not_applicable: Vec<&str> = report
        .verdicts
        .iter()
        .filter(|verdict| verdict.outcome == Outcome::NotApplicable)
        .map(|verdict| verdict.rule.as_str())
        .collect();
    assert_eq!(
        not_applicable,
        vec![
            "protocol-version",
            "handshake-schema",
            "unknown-method",
            "framing-intact",
            "process-alive"
        ],
        "{}",
        report.render()
    );
    for verdict in &report.verdicts {
        assert!(
            verdict.reason.is_some() || verdict.note.is_some() || verdict.outcome == Outcome::Pass,
            "`{}` reports {} with nothing to say about it",
            verdict.rule,
            verdict.outcome
        );
    }
}

#[test]
fn a_plugin_answered_by_the_wrong_profile_is_named_in_the_report() {
    // The note has to tell the author which two versions disagreed, because "protocol-version
    // failed" without both strings is not actionable.
    let report = run(Some("version"));
    let note = &report.violations()[0].note;
    let note = note.as_deref().unwrap_or_default();
    assert!(note.contains("demo/2") && note.contains("demo/1"), "{note}");
}
