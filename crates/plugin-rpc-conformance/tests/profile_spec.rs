//! The committed profile-spec meta-schema and the Rust reference loader must accept the same files.
//!
//! `tests/profile_cases.json` is the one table. This half asks the loader; `scripts/check_profile_schema.py`
//! asks the meta-schema. A profile author validates their file with the schema and loads it with a
//! host's reader, so the two answers have to agree — the same argument that holds the envelope
//! schemas and `plugin_rpc`'s types together in `crates/plugin-rpc/tests/envelope_parity.rs`.

use serde::Deserialize;
use serde_json::Value;

use plugin_rpc_conformance::{Defect, ProfileSpec};

const TABLE: &str = include_str!("profile_cases.json");

#[derive(Deserialize)]
struct Table {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    expect: String,
    document: Value,
}

fn verdict(case: &Case) -> &'static str {
    let text = serde_json::to_string(&case.document).expect("a case document is serializable");
    if ProfileSpec::parse(&text).is_ok() {
        "accept"
    } else {
        "reject"
    }
}

fn table() -> Table {
    serde_json::from_str(TABLE).expect("tests/profile_cases.json is readable")
}

#[test]
fn the_reference_loader_gives_every_case_the_verdict_the_meta_schema_states() {
    let wrong: Vec<String> = table()
        .cases
        .iter()
        .filter(|case| verdict(case) != case.expect)
        .map(|case| {
            format!(
                "  {}: the loader {}s it; the meta-schema {}s it",
                case.name,
                verdict(case),
                case.expect
            )
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "the loader and schema/prpc-1/profile.json disagree:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_case_names_a_verdict() {
    for case in table().cases {
        assert!(
            matches!(case.expect.as_str(), "accept" | "reject"),
            "case `{}` expects {:?}, which is neither accept nor reject",
            case.name,
            case.expect
        );
    }
}

/// The three rules below are in the loader and deliberately not in the meta-schema, because stating
/// them there would copy `spec/core-rules.json` into a schema and freeze a list the mechanism owns.
#[test]
fn a_profile_that_restates_a_core_rule_is_refused() {
    let error = ProfileSpec::parse(&document(serde_json::json!({"id": "framing-intact"})))
        .expect_err("a profile adds rules beside the core rules and never repeats one");
    assert!(error.to_string().contains("framing-intact"), "{error}");
}

#[test]
fn a_rule_that_depends_on_a_rule_that_is_not_declared_before_it_is_refused() {
    // The not-applicable short-circuit is computed in one pass over the list, so a rule that
    // depends on a later one would be read before that rule has a verdict to look at. `first-rule`
    // names `second-rule`, which the list declares after it.
    let error = ProfileSpec::parse(&document(serde_json::json!([
        {"id": "first-rule", "dependsOn": ["second-rule"]},
        {"id": "second-rule"}
    ])))
    .expect_err("`second-rule` is declared after the rule that depends on it");
    assert!(error.to_string().contains("second-rule"), "{error}");
}

#[test]
fn a_defect_that_names_an_undeclared_rule_is_refused() {
    let error = ProfileSpec::parse(&document(
        serde_json::json!({"id": "late-rule", "defect": {"run": ["/bin/sh", "p.sh"], "expectViolation": ["nowhere"]}}),
    ))
    .expect_err("a defect has to name a rule that exists");
    assert!(error.to_string().contains("nowhere"), "{error}");
}

#[test]
fn a_profile_spec_for_another_mechanism_is_refused_at_load_time() {
    // §13: refused at load rather than run and misreported, because a runner for `prpc/1` cannot
    // say what a `prpc/2` plugin violated.
    let base: Value =
        serde_json::from_str(&serde_json::to_string(&table().cases[0].document).unwrap())
            .expect("the first case is a valid profile spec");
    let mut document = base.as_object().unwrap().clone();
    document.insert("mechanism".to_string(), serde_json::json!("prpc/2"));
    let error = ProfileSpec::parse(&serde_json::to_string(&document).unwrap())
        .expect_err("this host implements prpc/1");
    assert!(error.to_string().contains("prpc/2"), "{error}");
}

#[test]
fn the_demo_profile_spec_is_the_documented_example_and_loads() {
    let spec = ProfileSpec::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/demo-1.profile.json"
    ))
    .expect("the demo profile spec loads");
    assert_eq!(spec.protocol_version, "demo/1");
    assert_eq!(spec.methods.len(), 3);
    // `ready` takes no parameters, so it names no params schema (§3 omits the member from the
    // request rather than sending null, and a profile forced to invent a file would ship one that
    // nothing validates).
    assert_eq!(spec.methods["ready"].params, None);
    assert!(spec.methods["ping"].params.is_some());
    let verified: Vec<&str> = spec
        .conformance
        .rules
        .iter()
        .filter(|rule| rule.defect.verified())
        .map(|rule| rule.id.as_str())
        .collect();
    assert_eq!(verified, ["ping-schema"], "one rule has a scripted defect");
    let unverified = matches!(&spec.conformance.rules[1].defect, Defect::Unverified { .. });
    assert!(unverified, "the other states that none exists");
}

/// The first case's document, with one rule of the profile's own replaced, so the loader-only rules
/// are tested against an otherwise valid profile spec. The given object is merged over the first
/// rule, so a test states only the field it is about.
fn document(rule: Value) -> String {
    let table = table();
    let mut document = table.cases[0]
        .document
        .as_object()
        .expect("the first case is an object")
        .clone();
    let base_rules = document["conformance"]["rules"]
        .as_array()
        .expect("the first case has rules")
        .clone();
    let overrides: Vec<&Value> = match rule.as_array() {
        Some(list) => list.iter().collect(),
        None => vec![&rule],
    };
    let mut merged_rules: Vec<Value> = Vec::new();
    for (index, override_) in overrides.iter().enumerate() {
        let mut merged = base_rules
            .get(index)
            .cloned()
            .unwrap_or_else(|| base_rules[0].clone())
            .as_object()
            .expect("a base rule is an object")
            .clone();
        for (key, value) in override_.as_object().expect("a rule override is an object") {
            merged.insert(key.clone(), value.clone());
        }
        merged_rules.push(Value::Object(merged));
    }
    let conformance = document
        .get_mut("conformance")
        .and_then(Value::as_object_mut)
        .expect("the first case has a conformance block");
    conformance.insert("rules".to_string(), Value::Array(merged_rules));
    serde_json::to_string(&document).expect("a profile spec is serializable")
}
