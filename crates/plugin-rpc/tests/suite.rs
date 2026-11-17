//! The suite contract of §12 item 6: what the mechanism puts in the suite's directory, and what it
//! requires back. `plugin_rpc::conformance::run_suite` is the reference driver, and these tests are
//! the first thing that ever ran it.

use plugin_rpc::conformance::{run_suite, Manifest, Outcome, ProfileSpec, SuiteError};
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn demo_spec() -> ProfileSpec {
    ProfileSpec::load(fixtures().join("demo-1.profile.json")).expect("the demo profile spec loads")
}

/// The demo profile spec with a different suite program, so a test states only the program it runs.
/// Parsing rather than loading is deliberate: the schemas stay where the committed fixture put them,
/// and the test hands `run_suite` that same directory.
fn spec_with_suite(argv: &[&str]) -> ProfileSpec {
    let text = std::fs::read_to_string(fixtures().join("demo-1.profile.json"))
        .expect("the demo fixture is readable");
    let mut document: serde_json::Value = text.parse().expect("the demo fixture is JSON");
    document["conformance"]["suite"] = serde_json::json!(argv);
    ProfileSpec::parse(&document.to_string()).expect("only the suite changed")
}

fn demo_manifest() -> Manifest {
    Manifest {
        name: "demo".to_string(),
        install: vec![],
        build: vec![],
        command: vec![
            "/bin/sh".to_string(),
            fixtures().join("demo.sh").display().to_string(),
        ],
        conformance: None,
    }
}

fn out_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn the_suite_is_handed_the_schemas_the_profile_spec_names() {
    let out = out_dir("suite-clean");
    let report = run_suite(&demo_spec(), &demo_manifest(), &fixtures(), &out)
        .expect("the demo suite reports both rules");

    // The suite program reads `schema/ping.json` out of the directory it was given and exits 2
    // without it, so a report here already proves the copy landed. The copies are what make §10
    // item 2's paths mean anything to a program that was never given the profile's checkout.
    assert!(out.join("profile.json").is_file());
    assert!(out.join("manifest.json").is_file());
    assert!(out.join("schema/ping.json").is_file());
    assert!(out.join("schema/prepare-req.json").is_file());

    assert_eq!(report.verdicts.len(), 2, "one verdict per declared rule");
    assert_eq!(report.verdicts[0].rule, "ping-schema");
    assert_eq!(report.verdicts[0].outcome, Outcome::Pass);
    assert_eq!(report.verdicts[1].outcome, Outcome::NotApplicable);
    assert!(
        report.verdicts[1].unverified,
        "the rule says no scripted defect exists, so a pass on it is not proof"
    );
    assert!(
        report.passed(),
        "a not-applicable rule does not fail the run"
    );
}

#[test]
fn the_scripted_defect_a_profile_declares_makes_its_own_rule_report_a_violation() {
    // §12 item 4, for a rule of the profile's own: the defect the spec names has to make exactly
    // that rule report a violation. The defect reaches the reference plugin through the wrapper
    // script rather than this process's environment, which other tests run in.
    let out = out_dir("suite-defect");
    let report = run_suite(
        &spec_with_suite(&["/bin/sh", "tests/fixtures/suite-defect.sh"]),
        &demo_manifest(),
        &fixtures(),
        &out,
    )
    .expect("the demo suite reports both rules");

    assert_eq!(
        report.verdicts[0].outcome,
        Outcome::Violation,
        "the defect the spec names must fire the rule it names"
    );
    assert_eq!(
        report
            .verdicts
            .iter()
            .filter(|verdict| verdict.outcome == Outcome::Violation)
            .count(),
        1,
        "the defect fires one rule and no other"
    );
    assert!(!report.passed());
    let note = report.verdicts[0]
        .note
        .as_ref()
        .expect("a violation names what it saw");
    assert!(note.contains("pong"), "{note}");
}

#[test]
fn a_rule_the_suite_did_not_report_is_an_error_and_never_a_pass() {
    // This suite program writes a verdict for `ping-schema` and nothing else.
    let dir = out_dir("suite-partial");
    std::fs::create_dir_all(&dir).expect("the suite directory is creatable");
    let script = dir.join("one-verdict.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do case \"$1\" in --out) out=\"$2\"; shift 2;; *) shift;; esac; done\nprintf '[{\"rule\":\"ping-schema\",\"outcome\":\"pass\"}]\\n' >\"$out/verdicts.json\"\n",
    )
    .expect("the suite script is writable");

    let error = match run_suite(
        &spec_with_suite(&["/bin/sh", &script.display().to_string()]),
        &demo_manifest(),
        &fixtures(),
        &dir,
    ) {
        Ok(report) => panic!("a suite that reported one of two rules produced {report:?}"),
        Err(error) => error,
    };
    assert!(matches!(error, SuiteError::Missing { rule: _ }), "{error}");
    assert!(
        error.to_string().contains("prepare-after-ping"),
        "the error names the rule the suite skipped: {error}"
    );
}
