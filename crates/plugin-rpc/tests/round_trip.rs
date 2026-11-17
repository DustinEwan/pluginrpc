//! The framing contract a non-Rust plugin is written against, asserted on the Rust side:
//! envelope shapes, the omitted-`params` rule, blank-line loss, and the open handshake.

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
    let ok = Response::ok(3, json!({ "ok": true }));
    let err = Response::err(4, METHOD_NOT_FOUND, "no such method");
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
    let line = json!({
        "protocolVersion": "afz/1",
        "name": "js-ts",
        "version": "0.3.0",
        "capabilities": ["coverage", "minimize"],
        "targetClass": "library",
        "discoverRequiresExecution": false
    });
    let h: Handshake = serde_json::from_value(line).expect("profile fields are ignored, not rejected");
    assert_eq!(h.protocol_version, "afz/1");
    assert_eq!(h.capabilities, vec!["coverage", "minimize"]);
}
