//! The message contract a non-Rust plugin is written against, asserted on the Rust side:
//! envelope shapes, the omitted-`params` rule, blank-line loss, the open envelope, the reserved
//! id 0, and the loss ordering.

use plugin_rpc::*;
use serde_json::json;

/// A profile's vocabulary, exactly as a profile implements it: the mechanism never sees it.
#[derive(Clone, Copy)]
enum Demo {
    Handshake,
    DoWork,
}

impl MethodName for Demo {
    fn name(&self) -> &'static str {
        match self {
            Demo::Handshake => "handshake",
            Demo::DoWork => "doWork",
        }
    }

    fn from_wire(name: &str) -> Option<Self> {
        match name {
            "handshake" => Some(Demo::Handshake),
            "doWork" => Some(Demo::DoWork),
            _ => None,
        }
    }
}

fn parse_handshake(value: serde_json::Value) -> Result<Handshake, serde_json::Error> {
    serde_json::from_value(value)
}

fn core_handshake() -> serde_json::Value {
    json!({
        "protocolVersion": "demo/1",
        "name": "mylang",
        "version": "0.3.0",
        "capabilities": ["alpha", "beta"]
    })
}

#[test]
fn request_round_trips_through_the_wire_shape() {
    let req = Request::new(7, Demo::DoWork, json!({ "deadlineMs": 1500 }));
    let line = serde_json::to_string(&req).expect("serializable");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).expect("json")["method"],
        "doWork"
    );
    assert_eq!(parse_request(&line).expect("parses"), req);
    assert!(Demo::from_wire(&req.method).is_some());
    assert!(Demo::from_wire("nope").is_none());
}

#[test]
fn params_is_omitted_not_null() {
    let req = Request::empty(1, Demo::Handshake);
    let value = serde_json::to_value(&req).expect("serializable");
    assert!(value.get("params").is_none(), "`params` must be absent: {value}");
}

#[test]
fn responses_round_trip_and_carry_the_id() {
    let ok = Response::result(3, json!({ "ok": true }));
    let err = Response::refusal(4, METHOD_NOT_FOUND, "no such method");
    for r in [ok, err] {
        let line = serde_json::to_string(&r).expect("serializable");
        let back = parse_response(&line).expect("parses");
        assert_eq!(back, r);
        assert_eq!(back.id(), r.id());
    }
}

#[test]
fn crlf_is_stripped_and_blank_line_is_loss() {
    let mut reader = buffered(&b"{\"a\":1}\r\n\n"[..]);
    let mut buf = String::new();
    assert_eq!(read_line(&mut reader, &mut buf).unwrap().as_deref(), Some("{\"a\":1}"));
    assert_eq!(read_line(&mut reader, &mut buf).unwrap(), None, "blank line is loss");
    assert_eq!(read_line(&mut reader, &mut buf).unwrap(), None, "eof is loss");
}

#[test]
fn handshake_envelope_accepts_profile_fields() {
    let mut value = core_handshake();
    value["targetClass"] = json!("library");
    value["extraFlag"] = json!(false);
    let h = parse_handshake(value).expect("profile fields are ignored, not rejected");
    assert_eq!(h.protocol_version, "demo/1");
    assert_eq!(h.capabilities, vec!["alpha", "beta"]);
}

#[test]
fn handshake_rejects_a_malformed_revision_and_a_duplicate_capability() {
    for (label, value) in [
        ("a revision that is not `<namespace>/<revision>`", {
            let mut v = core_handshake();
            v["protocolVersion"] = json!("AFZ_1");
            v
        }),
        ("a capability declared twice", {
            let mut v = core_handshake();
            v["capabilities"] = json!(["alpha", "alpha"]);
            v
        }),
    ] {
        assert!(
            parse_handshake(value).is_err(),
            "the reference type accepts a handshake the committed schema rejects: {label}"
        );
    }
}

#[test]
fn response_must_carry_exactly_one_of_result_and_error() {
    // A plugin that assigned a result and then failed must not be read as a clean success.
    let both = r#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":-32000,"message":"boom"}}"#;
    assert!(parse_response(both).is_err(), "a response carrying both is malformed");

    let neither = r#"{"jsonrpc":"2.0","id":1}"#;
    assert!(parse_response(neither).is_err(), "a response carrying neither is malformed");

    // `null` is a payload a profile may return, so a present `result` of null is a result.
    let nulls = parse_response(r#"{"jsonrpc":"2.0","id":1,"result":null}"#).expect("a null payload");
    assert_eq!(nulls.into_answer().value().unwrap(), serde_json::Value::Null);
}

#[test]
fn envelope_is_open_so_a_profile_may_declare_top_level_fields() {
    let req = parse_request(r#"{"jsonrpc":"2.0","id":1,"method":"doWork","shard":3}"#)
        .expect("a field the mechanism does not name is ignored, not rejected");
    assert_eq!(req.id, 1);

    let res = parse_response(r#"{"jsonrpc":"2.0","id":2,"result":1,"attempt":4}"#)
        .expect("a field the mechanism does not name is ignored, not rejected");
    assert_eq!(res.id(), 2);
}

#[test]
fn id_zero_is_reserved_for_an_uncorrelated_refusal() {
    let req = format!(r#"{{"jsonrpc":"2.0","id":{UNCORRELATED_ID},"method":"handshake"}}"#);
    assert!(parse_request(&req).is_err(), "a host never assigns id 0");

    let refusal = Response::uncorrelated(PARSE_ERROR, "the line was not json-rpc");
    assert_eq!(refusal.id(), UNCORRELATED_ID);
    let line = serde_json::to_string(&refusal).expect("serializable");
    assert_eq!(parse_response(&line).expect("a refusal with id 0 is valid").id(), UNCORRELATED_ID);
}

#[test]
fn id_above_the_javascript_safe_bound_is_rejected() {
    let req = format!(r#"{{"jsonrpc":"2.0","id":{},"method":"handshake"}}"#, u64::MAX);
    assert!(parse_request(&req).is_err(), "id above MAX_ID is not exact in JavaScript");
    let res = format!(r#"{{"jsonrpc":"2.0","id":{},"result":1}}"#, u64::MAX);
    assert!(parse_response(&res).is_err());
    let edge = format!(r#"{{"jsonrpc":"2.0","id":{MAX_ID},"result":1}}"#);
    assert!(parse_response(&edge).is_ok(), "MAX_ID itself is legal");
}

#[test]
fn a_refusal_is_an_answer_and_not_a_loss() {
    let refusal = Response::refusal(9, PLUGIN_ERROR, "the build failed");
    assert!(refusal.clone().into_answer().is_refusal());
    let error = refusal.into_answer().value().expect_err("a refusal has no payload");
    assert_eq!(error.code, PLUGIN_ERROR);
}

#[test]
fn the_more_specific_loss_reason_wins() {
    // The host's kill outranks the exit status it produced; the exit status outranks the closed
    // pipe the reader noticed first; no-answer is the fallback.
    let killed = Loss::Killed;
    let exited = Loss::Exited { code: Some(1), signal: None };
    let eof = Loss::Eof;
    assert_eq!(Loss::resolve(Some(killed.clone()), exited.clone()), killed);
    assert_eq!(Loss::resolve(Some(eof.clone()), exited.clone()), exited);
    assert_eq!(
        Loss::resolve(Some(Loss::NoAnswer), eof.clone()),
        eof,
        "a recorded reason outranks no answer"
    );
    assert_eq!(Loss::resolve(None, Loss::NoAnswer), Loss::NoAnswer);
}

#[test]
fn a_profile_adds_its_own_loss_reason() {
    let loss = Loss::Profile {
        reason: "result-schema".to_string(),
        detail: "`targets` is not an array".to_string(),
    };
    assert!(loss.to_string().contains("result-schema"));
    let round: Loss = serde_json::from_value(serde_json::to_value(&loss).expect("serializable"))
        .expect("a loss reason round trips into a report");
    assert_eq!(round, loss);
}
