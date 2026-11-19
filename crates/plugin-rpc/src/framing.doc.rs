//! The claims about the response envelope in `src/lib.rs`, and the scenario each one carries.
//!
//! A scenario is code in this file. It calls `parse_response` directly, so no action name and no
//! fixture registry stands between the claim and the call it makes, and the harness example links
//! this module (`examples/veridoc.rs`).

use veridoc_claims::{Assert, Channel, Claim, Kind, Outcome, Scenario, Subject, Type};

/// The one value the run claim declares, with the type its assertion is checked against.
const VALUES: &[(&str, Type)] = &[("refused", Type::Boolean)];

/// `parse_response` reading a response envelope that carries both `result` and `error`.
fn both_fields() -> Result<Outcome, String> {
    let refused = plugin_rpc::parse_response(
        r#"{"jsonrpc":"2.0","id":1,"result":null,"error":{"code":-32000,"message":"failed"}}"#,
    )
    .is_err();
    Ok(Outcome::new().value("refused", refused))
}

/// The claims about the response envelope.
pub fn claims() -> Vec<Claim> {
    vec![Claim {
        kind: Kind::Ensures,
        subject: Subject::symbol("parse_response", "crates/plugin-rpc/src/lib.rs"),
        concept: "pluginrpc",
        predicate: "refuses-a-response-with-both-fields",
        text:
            "parse_response refuses a response whose envelope carries both `result` and `error`, \
               because reading the result would report success while discarding the failure.",
        channel: Channel::run(Assert::eq("refused", true)),
        scenario: Some(Scenario {
            given: "a response line that carries both `result` and `error`",
            when: "parse_response reads the line",
            returns: VALUES,
            run: both_fields,
        }),
    }]
}
