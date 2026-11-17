//! The committed envelope schemas and the Rust reference types must accept the same messages.
//!
//! `tests/messages.json` is the one table of cases. This file asks the Rust types for each
//! verdict, and `scripts/check_envelope_schemas.py` asks the committed schemas. A rule the
//! schemas enforce and the types do not shows up there; a rule the types enforce and the schemas
//! do not shows up here. A plugin author in another language implements against the schemas and
//! tests against the types, so the two answers have to match.

use plugin_rpc::*;
use serde::Deserialize;
use serde_json::Value;

const TABLE: &str = include_str!("messages.json");

#[derive(Deserialize)]
struct Table {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    schema: String,
    expect: String,
    message: Value,
}

/// The verdict the reference types give, reached through the same functions a host and a plugin
/// call on the wire.
fn verdict(case: &Case) -> &'static str {
    let accepted = match case.schema.as_str() {
        "request" => parse_request(&case.message.to_string()).is_ok(),
        "response" => parse_response(&case.message.to_string()).is_ok(),
        "handshake" => serde_json::from_value::<Handshake>(case.message.clone()).is_ok(),
        other => panic!("case `{}` names `{other}`, which is no envelope schema", case.name),
    };
    if accepted {
        "accept"
    } else {
        "reject"
    }
}

fn table() -> Table {
    serde_json::from_str(TABLE).expect("crates/plugin-rpc/tests/messages.json is readable")
}

#[test]
fn the_reference_types_give_every_case_the_verdict_the_mechanism_states() {
    let wrong: Vec<String> = table()
        .cases
        .iter()
        .filter(|case| verdict(case) != case.expect)
        .map(|case| {
            format!(
                "  {}: the reference types {} it; the mechanism says {}",
                case.name,
                verdict(case),
                case.expect
            )
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "the reference types disagree with the verdicts in tests/messages.json:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_envelope_schema_has_cases() {
    // A schema with no case in the table is unchecked, which is how a constraint comes back
    // without anyone noticing it went.
    let cases = table().cases;
    for schema in ["request", "response", "handshake"] {
        assert!(
            cases.iter().any(|case| case.schema == schema),
            "`{schema}` has no case in tests/messages.json"
        );
    }
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
