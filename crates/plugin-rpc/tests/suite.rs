//! The suite contract of §12 item 6: what the mechanism puts in the suite's directory, and what it
//! requires back. `plugin_rpc::conformance::run_suite` is the reference driver, and these tests are
//! the first thing that ever ran it.

use plugin_rpc::conformance::{
    core::CoreCheck, run_defect, run_suite, Defect, DefectRun, Manifest, Outcome, ProfileSpec,
    SuiteError,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    let report = run_suite(&demo_spec(), &demo_manifest(), &fixtures(), &out, &[])
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
        &[],
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
        &[],
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

/// A base check with nothing in it but the deadline: a defect brings its own command and its own
/// variables, and the base's environment is what the runner was given for the plugin.
fn base(spec: &ProfileSpec) -> CoreCheck<'_> {
    CoreCheck {
        profile: spec,
        command: Vec::new(),
        env: Vec::new(),
        cwd: None,
        deadline: Duration::from_millis(700),
    }
}

#[test]
fn a_scripted_defect_is_run_and_what_it_fired_is_what_the_rule_is_worth() {
    // §12 item 4, done rather than assumed: the mechanism starts the plugin with the defect the
    // profile names and looks for the violation, and the same command with the defect taken out
    // proves the report would otherwise be claiming a run nobody did.
    let spec = demo_spec();
    let manifest = demo_manifest();
    let fired = run_defect(
        &spec.conformance.rules[0],
        &spec,
        &manifest,
        &fixtures(),
        &out_dir("defect-fires"),
        &base(&spec),
    );
    assert_eq!(fired, DefectRun::Fired, "{fired:?}");

    let mut correct = demo_spec();
    let Defect::Scripted {
        run,
        expect_violation,
        ..
    } = &correct.conformance.rules[0].defect
    else {
        panic!("the demo fixture's first rule carries a scripted defect");
    };
    correct.conformance.rules[0].defect = Defect::Scripted {
        run: run.clone(),
        env: Vec::new(),
        expect_violation: expect_violation.clone(),
    };
    let silent = run_defect(
        &correct.conformance.rules[0],
        &correct,
        &manifest,
        &fixtures(),
        &out_dir("defect-silent"),
        &base(&correct),
    );
    assert_eq!(
        silent,
        DefectRun::Silent {
            missing: vec!["ping-schema".to_string()]
        },
        "a correct plugin fires nothing, so the claim is not proof: {silent:?}"
    );

    // The same defect, against a manifest whose plugin is a different program: the claim belongs
    // to the profile's own run, and this run neither proves it nor takes the mark back.
    let other = Manifest {
        command: vec!["/bin/sh".to_string(), "tests/fixtures/other.sh".to_string()],
        ..demo_manifest()
    };
    let foreign = run_defect(
        &spec.conformance.rules[0],
        &spec,
        &other,
        &fixtures(),
        &out_dir("defect-other-plugin"),
        &base(&spec),
    );
    assert_eq!(foreign, DefectRun::OtherPlugin, "{foreign:?}");
}

/// Copy the fixture schemas under `into/schema`, so a profile spec written into a temporary
/// directory still names files its loader can open (§10 item 2).
fn copy_schemas(from: &Path, into: &Path) -> std::io::Result<()> {
    let dest = into.join("schema");
    std::fs::create_dir_all(&dest)?;
    for entry in std::fs::read_dir(from)? {
        let path = entry?.path();
        std::fs::copy(&path, dest.join(path.file_name().expect("a schema file")))?;
    }
    Ok(())
}

#[test]
fn the_command_takes_back_the_mark_of_a_rule_whose_defect_did_not_fire() {
    // The report's `unverified` column is worth exactly the defect run behind it, and `prpc-core`
    // is what does that run, so the mark is tested through the command and not the library.
    let dir = out_dir("command-mark");
    std::fs::create_dir_all(&dir).expect("the suite directory is creatable");
    copy_schemas(&fixtures().join("schema"), &dir).expect("the schemas copy");
    let mut spec = demo_spec();
    let Defect::Scripted {
        run,
        expect_violation,
        ..
    } = &spec.conformance.rules[0].defect
    else {
        panic!("the demo fixture's first rule carries a scripted defect");
    };
    spec.conformance.rules[0].defect = Defect::Scripted {
        run: run.clone(),
        env: Vec::new(),
        expect_violation: expect_violation.clone(),
    };
    let spec_path = dir.join("profile.json");
    std::fs::write(
        &spec_path,
        serde_json::to_string(&spec).expect("the spec serializes"),
    )
    .expect("the spec is writable");
    let manifest_path = dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string(&demo_manifest()).expect("the manifest serializes"),
    )
    .expect("the manifest is writable");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_prpc-core"))
        .arg(&spec_path)
        .arg(&manifest_path)
        .arg("--suite")
        // The fixture plugin and the fixture suite program are named beside the crate, which is
        // where the command started from a plugin author's checkout would find them.
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("the command runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ping-schema: its scripted defect left"),
        "the command says which rule's defect did nothing: {stderr}"
    );
    assert!(
        stdout.contains("unverified: no scripted defect fires this rule"),
        "and the report says so too: {stdout}"
    );
}
