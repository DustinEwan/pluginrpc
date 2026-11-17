//! End-to-end tests for the stdio-process transport. Each one starts a real child process, so
//! what they assert is the transport's behaviour under a plugin that misbehaves, not the shape of
//! a mock. `/bin/sh` stands in for the plugin: it can print a canned message, stay silent, refuse,
//! write to stderr, and exit with a chosen status, which is the whole set of things a host has to
//! survive.

#![cfg(unix)]

use std::time::Duration;

use plugin_rpc::process::{Plugin, PluginConfig, StartError, Timeouts};
use plugin_rpc::{Call, Loss, MethodName, PARSE_ERROR, PLUGIN_ERROR};

/// The profile these tests speak. The mechanism sends `handshake` itself, so the profile only
/// names its own method.
enum Work {
    Run,
}

impl MethodName for Work {
    fn name(&self) -> &'static str {
        "work"
    }

    fn from_wire(name: &str) -> Option<Work> {
        (name == "work").then_some(Work::Run)
    }
}

#[derive(serde::Deserialize, PartialEq, Debug)]
struct Note {
    note: String,
}

#[derive(serde::Serialize)]
struct Job {
    id: String,
}

/// The handshake answer, carrying the id the host assigns to its first request.
const HANDSHAKE: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"demo/1","name":"fake","version":"0.1.0","capabilities":["alpha"]}}"#;

/// Wait for one request line, so a canned answer is an answer to something. A plugin that wrote
/// its answer before the host had asked would not be a plugin, and the host is entitled to the
/// causality that a request precedes its response.
const ASKED: &str = "read request; ";

/// One `printf` of a message, so the shell quotes nothing itself.
fn say(message: &str) -> String {
    format!("printf '%s\\n' '{message}'; ")
}

/// A handshake carrying the fields a profile adds beside the core four. autofuzz adds
/// `targetClass` and `languages`; veridoc adds `languages`. The mechanism must deliver those to
/// the host and not drop them on the floor.
const HANDSHAKE_WITH_PROFILE_FIELDS: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"demo/1","name":"fake","version":"0.1.0","capabilities":["alpha"],"targetClass":"libfuzzer","languages":["c"],"discoverRequiresExecution":true}}"#;

fn plugin(script: &str) -> PluginConfig {
    PluginConfig::new(["/bin/sh", "-c", script])
}

fn start(script: &str) -> (Plugin, plugin_rpc::Handshake) {
    Plugin::start(&plugin(script)).expect("the plugin starts and shakes hands")
}

fn stderr_of(plugin: &Plugin) -> Vec<String> {
    plugin
        .stderr_lines
        .lock()
        .expect("the stderr lock is never poisoned")
        .clone()
}

/// Wait for the plugin's stderr to arrive, which is a thread crossing and not instantaneous.
fn stderr_containing(plugin: &Plugin, needle: &str) -> Vec<String> {
    for _ in 0..100 {
        let lines = stderr_of(plugin);
        if lines.iter().any(|line| line.contains(needle)) {
            return lines;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    stderr_of(plugin)
}

#[test]
fn start_shakes_hands_and_reports_the_declared_capabilities() {
    let (plugin, handshake) = start(&format!("{ASKED}{}sleep 5", say(HANDSHAKE)));
    assert_eq!(handshake.protocol_version, "demo/1");
    assert_eq!(handshake.capabilities, vec!["alpha".to_string()]);
    assert_eq!(plugin.lost(), None, "a live connection reports no loss");
}

#[test]
fn a_refused_handshake_is_a_refusal_and_not_a_loss() {
    // The plugin is up and answering; it only declines to be a plugin for this host. Calling that
    // a lost process sends an operator to look for a crash that never happened.
    let refusal =
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"not a demo plugin"}}"#;
    let error = Plugin::start(&plugin(&format!("{ASKED}{}sleep 5", say(refusal))))
        .expect_err("a refusal is not a connection");
    match &error {
        StartError::Refused(refused) => {
            assert_eq!(refused.code, PLUGIN_ERROR);
            assert_eq!(refused.message, "not a demo plugin");
        }
        StartError::Lost(loss) => panic!("a refusal was reported as a loss: {loss}"),
    }
    assert_eq!(
        error.to_string(),
        "the plugin refused the handshake (-32000): not a demo plugin"
    );
}

#[test]
fn a_silent_plugin_is_no_answer() {
    let (plugin, _handshake) = start(&format!("{ASKED}{}sleep 5", say(HANDSHAKE)));
    let call: Call<Note> = plugin.request(Work::Run, None, Duration::from_millis(300));
    assert_eq!(call.loss(), Some(Loss::NoAnswer));
    assert_eq!(plugin.lost(), None, "a slow plugin is still running");
}

#[test]
fn a_plugin_that_died_is_reported_as_exited_and_not_as_no_answer() {
    // The reader sees the closed pipe before the exit watcher learns the status. Without the
    // mechanism waiting briefly for the status, a crash would be reported as a closed pipe.
    let (plugin, _handshake) = start(&format!("{ASKED}{}exit 3", say(HANDSHAKE)));
    let call: Call<Note> = plugin.request(Work::Run, None, Duration::from_secs(2));
    assert_eq!(
        call.loss(),
        Some(Loss::Exited {
            code: Some(3),
            signal: None
        }),
        "the exit status outranks what the reader noticed"
    );
}

#[test]
fn a_refusal_is_an_answer_and_the_plugin_keeps_answering() {
    let refusal =
        r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"store unreachable"}}"#;
    let answer = r#"{"jsonrpc":"2.0","id":3,"result":{"note":"done"}}"#;
    let (plugin, _handshake) = start(&format!(
        "{ASKED}{}{ASKED}{}{ASKED}{}sleep 5",
        say(HANDSHAKE),
        say(refusal),
        say(answer)
    ));
    let first: Call<Note> = plugin.request(Work::Run, None, Duration::from_secs(5));
    let error = first
        .refusal()
        .expect("an error frame is a refusal, not a loss");
    assert_eq!(error.message, "store unreachable");
    assert_eq!(plugin.lost(), None, "a refusal leaves the process alive");

    let second: Call<Note> = plugin.request(Work::Run, None, Duration::from_secs(5));
    assert_eq!(
        second.payload(),
        Some(Note {
            note: "done".to_string()
        }),
        "the connection survived the refusal"
    );
}

#[test]
fn kill_records_the_hosts_own_reason() {
    let (plugin, _handshake) = start(&format!("{ASKED}{}sleep 30", say(HANDSHAKE)));
    plugin.kill();
    assert_eq!(plugin.lost(), Some(Loss::Killed));
}

#[test]
fn stderr_is_drained_so_the_plugin_never_blocks_on_it() {
    // Spec §3: a pipe nobody reads fills, and the plugin stops writing to it. The line has to
    // reach the host for a death to be attributable.
    let script = &format!(
        "echo 'plugin is unhappy' >&2; {ASKED}{}sleep 5",
        say(HANDSHAKE)
    );
    let (plugin, _handshake) = start(script);
    assert_eq!(
        stderr_containing(&plugin, "unhappy"),
        vec!["plugin is unhappy".to_string()]
    );
}

#[test]
fn a_refusal_the_plugin_cannot_correlate_is_kept_as_a_connection_fault() {
    // Spec §4: the host never discards a refusal whose id is 0, because it reports a fault on the
    // connection rather than an answer to a request. The plugin sends it unsolicited, which is
    // exactly why it carries the id no host can assign.
    let uncorrelated =
        r#"{"jsonrpc":"2.0","id":0,"error":{"code":-32700,"message":"malformed line"}}"#;
    let (plugin, _handshake) = start(&format!(
        "{ASKED}{}{}sleep 5",
        say(HANDSHAKE),
        say(uncorrelated)
    ));
    let mut fault = None;
    for _ in 0..100 {
        fault = plugin.connection_fault();
        if fault.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fault.expect("id 0 is never discarded").code,
        PARSE_ERROR,
        "an uncorrelated refusal is reported as a fault on the connection"
    );
}

#[test]
fn the_plugin_is_told_the_deadline_the_host_will_not_wait_past() {
    // Spec §9: the host's wait is longer than the deadline the plugin is given, so a plugin that
    // cuts itself off in time still delivers its answer.
    let given = Timeouts::plugin_deadline(Duration::from_secs(2));
    assert_eq!(given, 1_750);
    assert!(
        Duration::from_millis(given) < Duration::from_secs(2),
        "the plugin's deadline is strictly inside the host's wait"
    );
}

#[test]
fn a_command_that_cannot_be_started_is_a_spawn_loss_naming_the_program() {
    let config = PluginConfig::new(["/nonexistent/plugin-server"]);
    let error = Plugin::start(&config).expect_err("nothing started");
    let StartError::Lost(Loss::Spawn { detail }) = error else {
        panic!("a failed spawn was reported as {error}");
    };
    assert!(
        detail.contains("/nonexistent/plugin-server"),
        "the detail names the program that would not start: {detail}"
    );
}

#[test]
fn an_empty_command_is_a_spawn_loss_naming_the_config_field() {
    let error = Plugin::start(&PluginConfig::default()).expect_err("nothing to run");
    assert_eq!(
        error,
        StartError::Lost(Loss::Spawn {
            detail: "the plugin's command is empty".to_string()
        })
    );
}

#[test]
fn typed_params_reach_the_wire_under_the_profiles_own_names() {
    // The plugin echoes the request it is answering to stderr, so this asserts what went over the
    // pipe rather than what the conversion was meant to build.
    let refusal = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"not now"}}"#;
    let script = &format!(
        "{ASKED}{}{ASKED}printf '%s\\n' \"$request\" >&2; {}sleep 5",
        say(HANDSHAKE),
        say(refusal)
    );
    let (plugin, _handshake) = start(script);
    let call: Call<Note> = plugin.call(
        Work::Run,
        &Job {
            id: "job-7".to_string(),
        },
        Duration::from_secs(5),
    );
    assert!(
        call.refusal().is_some(),
        "a refusal still resolves the call it answers"
    );
    let echoed = stderr_containing(&plugin, "job-7").join("\n");
    assert!(
        echoed.contains(r#""params":{"id":"job-7"}"#),
        "the request carried its params: {echoed}"
    );
    assert!(
        echoed.contains(r#""method":"work""#),
        "the request named the profile's method: {echoed}"
    );
}

#[test]
fn the_handshake_carries_the_profile_fields_beside_the_four_the_mechanism_reads() {
    // §5 makes the envelope open, so a profile's fields are already in the one `handshake` answer.
    // A host that has to send `handshake` twice to learn its own profile is reading the mechanism
    // wrong, and both hosts did exactly that before `Handshake::result` existed.
    let (plugin, handshake) = start(&format!(
        "{ASKED}{}sleep 5",
        say(HANDSHAKE_WITH_PROFILE_FIELDS)
    ));
    assert_eq!(
        handshake.protocol_version, "demo/1",
        "the mechanism still read the core fields it owns"
    );
    assert_eq!(
        handshake.result["targetClass"], "libfuzzer",
        "the profile's own field reached the host: {}",
        handshake.result
    );
    assert_eq!(handshake.result["discoverRequiresExecution"], true);
    assert!(
        plugin.lost().is_none(),
        "fields the mechanism does not name are not a protocol violation"
    );

    let serialized = serde_json::to_value(&handshake).expect("a handshake serializes");
    assert!(
        serialized.get("result").is_none(),
        "`result` is not a wire field, so it is never re-sent nested: {serialized}"
    );
    assert_eq!(serialized["capabilities"], serde_json::json!(["alpha"]));
}
