//! The six core rules of `prpc/1` (spec §12), run by the mechanism itself.
//!
//! These six name no method of any profile. They need the manifest's `command`, the envelope
//! schemas, the profile's `protocolVersion` string, and a way to send a method name the profile
//! does not declare and a line that is not JSON. That is why the mechanism can run them for every
//! profile and why a profile author writes no code for them (§12 item 7).
//!
//! The two probes that send a bad request drive the plugin over their own pipes rather than through
//! [`crate::process::Plugin`], and that is deliberate: the typed client drops a line it cannot
//! parse, which is right for a host and wrong for a suite checking whether a plugin answers a
//! malformed line at all. §12 item 6 gives a suite its own process and pipes for exactly this, and
//! both profiles that drove this design already do it that way.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::process::{HandshakeMethod, Plugin, PluginConfig, StartError, Timeouts};
use crate::{Call, Loss, METHOD_NOT_FOUND, PARSE_ERROR};
use serde_json::Value;

use super::{core_rules, ProfileSpec, Report, Verdict};

/// The method name the `unknown-method` rule sends. It is the mechanism's own, not a profile's, so
/// a profile cannot make the rule vacuous by declaring it: [`CoreCheck::unknown_method`] keeps
/// appending a suffix while the name happens to be in the profile's method set.
const UNKNOWN_METHOD_BASE: &str = "prpc-not-a-method";

/// A line that is not JSON, for `framing-intact` (spec §4, §5).
const MALFORMED_LINE: &str = "{\"jsonrpc\": \"2.0\", oops";

/// One run of the core rules against one plugin.
#[derive(Debug, Clone)]
pub struct CoreCheck<'a> {
    /// The profile the plugin is tested against: it fixes the `protocolVersion` the handshake must
    /// answer with, and the method vocabulary `unknown-method` picks a name outside of.
    pub profile: &'a ProfileSpec,
    /// The manifest's `command`, run verbatim (§11).
    pub command: Vec<String>,
    /// Extra variables for the plugin's process.
    pub env: Vec<(String, String)>,
    /// The directory to start the plugin in.
    pub cwd: Option<PathBuf>,
    /// How long to wait for anything the plugin controls: the handshake, a refusal, the next answer.
    pub deadline: Duration,
}

impl CoreCheck<'_> {
    fn config(&self) -> PluginConfig {
        let mut config = PluginConfig::new(self.command.clone());
        for (key, value) in &self.env {
            config = config.with_env(key, value);
        }
        if let Some(cwd) = &self.cwd {
            config = config.with_cwd(cwd.clone());
        }
        config.timeouts = Timeouts {
            handshake_timeout: self.deadline,
            request_timeout: self.deadline,
        };
        config
    }

    /// A method name the profile does not declare.
    fn unknown_method(&self) -> String {
        let mut name = UNKNOWN_METHOD_BASE.to_string();
        while self.profile.methods.contains_key(&name) {
            name.push_str("-prpc");
        }
        name
    }
}

/// Run the six core rules, in the order `spec/core-rules.json` states them.
///
/// Every path through the first three rules pushes exactly three verdicts, so a report always says
/// something about `spawn`, `protocol-version`, and `handshake-schema` rather than going silent on
/// them; the last three rules are decided after. A rule that could not be run is counted not
/// applicable with the reason that stopped it, never as a pass (§12 item 3).
pub fn run(check: &CoreCheck) -> Report {
    // The ids are positional here and nowhere else. `core_rule_ids_are_the_ones_the_runner_implements`
    // fails if the committed file and this function ever disagree about which six rules there are.
    let rules = core_rules();
    let ids: Vec<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
    if ids.len() != 6 {
        return Report::suite_error(format!(
            "the core-rules file states {} rules; this runner implements six, in the order spawn, \
             protocol-version, handshake-schema, unknown-method, framing-intact, process-alive",
            ids.len()
        ));
    }
    let [spawn, protocol_version, handshake_schema, unknown_method, framing_intact, process_alive]: [&str; 6] =
        ids.as_slice()
            .try_into()
            .expect("the length was just checked to be six");

    let mut report = Report::default();
    let connected = match Plugin::start(&check.config()) {
        Ok((plugin, handshake)) => {
            report.verdicts.push(Verdict::pass(spawn));
            if handshake.protocol_version == check.profile.protocol_version {
                report.verdicts.push(Verdict::pass(protocol_version));
            } else {
                report.verdicts.push(Verdict::violation(
                    protocol_version,
                    format!(
                        "the handshake answers `{}`; the profile spec declares `{}`",
                        handshake.protocol_version, check.profile.protocol_version
                    ),
                ));
            }
            // `Plugin::start` deserialized the answer into `crate::Handshake`, whose
            // deserializer enforces every constraint `schema/prpc-1/handshake.json` states — the
            // `<namespace>/<revision>` pattern, the two non-empty strings, and the capability
            // array's non-empty uniqueness — and `tests/envelope_parity.rs` holds the type and the
            // schema to the same verdicts. A handshake that got this far is an instance of it.
            report.verdicts.push(Verdict::pass(handshake_schema));
            Some(plugin)
        }
        // The process started and answered; the answer is not an instance of the envelope schema.
        // That is `handshake-schema`'s finding and not `spawn`'s, and the note is written in schema
        // terms rather than as a decoder's internal error (§12 item 2).
        Err(StartError::Lost(Loss::Profile { detail, .. })) => {
            report.verdicts.push(Verdict::pass(spawn));
            report.verdicts.push(Verdict::not_applicable(
                protocol_version,
                "the handshake is not an instance of the envelope schema, so its protocolVersion cannot be read",
            ));
            report
                .verdicts
                .push(Verdict::violation(handshake_schema, detail));
            None
        }
        // The plugin is running and declined to be a plugin for this host. `spawn`'s expectation is
        // that the command starts and its stdout accepts a request, which it did; refusing the
        // profile is `protocol-version`'s rule (§6).
        Err(StartError::Refused(refusal)) => {
            report.verdicts.push(Verdict::pass(spawn));
            report.verdicts.push(Verdict::violation(
                protocol_version,
                format!(
                    "the plugin refused the handshake ({}): {}",
                    refusal.code, refusal.message
                ),
            ));
            report.verdicts.push(Verdict::not_applicable(
                handshake_schema,
                "the plugin refused the handshake, so there is no handshake response to check",
            ));
            None
        }
        Err(StartError::Lost(loss)) => {
            report.verdicts.push(Verdict::violation(
                spawn,
                format!("the plugin gave no usable handshake: {loss}"),
            ));
            report.verdicts.push(Verdict::not_applicable(
                protocol_version,
                "there is no handshake response to read",
            ));
            report.verdicts.push(Verdict::not_applicable(
                handshake_schema,
                "there is no handshake response to check",
            ));
            None
        }
    };

    // The two probes each get a plugin of their own: a refusal and a malformed line are both allowed
    // to end a naive plugin's connection, and a shared process would turn one fault into two
    // violations and hide which rule the author has to fix.
    let probes = [
        (unknown_method, Probe::UnknownMethod),
        (framing_intact, Probe::Framing),
    ];
    for (index, (rule, probe)) in probes.into_iter().enumerate() {
        if connected.is_none() {
            report
                .verdicts
                .push(Verdict::not_applicable(rule, "no connection came up"));
            continue;
        }
        let mut raw = match Raw::start(check) {
            Ok(raw) => raw,
            Err(e) => {
                report.suite_error = Some(format!(
                    "the suite could not start its own plugin for `{rule}`: {e}"
                ));
                for pending in &ids[3 + index + 1..] {
                    report.verdicts.push(Verdict::not_applicable(
                        pending,
                        "the suite could not start its own plugin",
                    ));
                }
                return report;
            }
        };
        match probe.run(&mut raw, check) {
            Ok(verdict) => report.verdicts.push(verdict),
            Err(mut notes) => {
                // One violation per rule: the report is keyed on the rule id, and the first thing
                // that went wrong is the one the author acts on. The rest is in the note.
                let note = if notes.len() == 1 {
                    notes.remove(0)
                } else {
                    notes.join("; ")
                };
                report.verdicts.push(Verdict::violation(rule, note));
            }
        }
    }

    match &connected {
        None => report.verdicts.push(Verdict::not_applicable(
            process_alive,
            "no connection came up",
        )),
        Some(plugin) => {
            if let Some(loss) = plugin.lost() {
                report.verdicts.push(Verdict::violation(
                    process_alive,
                    format!("the plugin went during the run: {loss}"),
                ));
            } else {
                let again: Call<Value> =
                    plugin.request(HandshakeMethod::Handshake, None, check.deadline);
                if again.answered() {
                    report.verdicts.push(Verdict::pass(process_alive));
                } else {
                    report.verdicts.push(Verdict::violation(
                        process_alive,
                        "the plugin is running but no longer answers handshake",
                    ));
                }
            }
        }
    }

    report
}

/// The two checks that need raw pipes.
#[derive(Debug, Clone, Copy)]
enum Probe {
    UnknownMethod,
    Framing,
}

impl Probe {
    fn run(&self, raw: &mut Raw, check: &CoreCheck) -> Result<Verdict, Vec<String>> {
        match self {
            Probe::UnknownMethod => raw.unknown_method(check),
            Probe::Framing => raw.framing_intact(),
        }
    }
}

impl Raw {
    /// A request whose method is not in the profile must be refused with -32601 and the request's
    /// own id, and the connection must go on answering (spec §4, §5).
    fn unknown_method(&mut self, check: &CoreCheck) -> Result<Verdict, Vec<String>> {
        let method = check.unknown_method();
        let request = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":{}}}",
            serde_json::to_string(&method).expect("a method name is a JSON string")
        );
        let Some(frame) = self.request(1, &request) else {
            return Err(vec![format!(
                "`{method}` gave no answer at all; a refusal is expected"
            )]);
        };
        let mut notes = Vec::new();
        let code = frame
            .get("error")
            .and_then(Value::as_object)
            .and_then(|error| error.get("code"))
            .and_then(Value::as_i64);
        if code != Some(METHOD_NOT_FOUND) {
            notes.push(format!(
                "`{method}` got {} instead of a refusal carrying {METHOD_NOT_FOUND}",
                classify(&frame)
            ));
        }
        // The rule says the connection continues, so the next well-formed request has to be answered
        // by the same process and not merely refused correctly.
        if self.handshake(2).is_none() {
            notes.push("the connection did not answer the next handshake".to_string());
        }
        verdict("unknown-method", notes)
    }

    /// A malformed line must be refused with -32700 rather than unanswered, and the next
    /// well-formed request must still be answered (spec §4, §5).
    fn framing_intact(&mut self) -> Result<Verdict, Vec<String>> {
        let mut notes = Vec::new();
        if self.send(MALFORMED_LINE).is_err() {
            return Err(vec![
                "the pipe to the plugin closed on a malformed line".to_string()
            ]);
        }
        match self.await_any() {
            Some(frame) => {
                let code = frame
                    .get("error")
                    .and_then(Value::as_object)
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_i64);
                if code != Some(PARSE_ERROR) {
                    notes.push(format!(
                        "a malformed line got {} instead of a refusal carrying {PARSE_ERROR}",
                        classify(&frame)
                    ));
                }
            }
            None => notes.push("a malformed line got no answer at all".to_string()),
        }
        if self.handshake(1).is_none() {
            notes.push("the next well-formed request was not answered".to_string());
        }
        verdict("framing-intact", notes)
    }
}

/// One plugin's stdin and stdout, held by the suite rather than by a host, so a line the plugin
/// cannot parse reaches it as the byte sequence the suite chose.
struct Raw {
    child: Child,
    stdin: std::io::BufWriter<ChildStdin>,
    lines: Receiver<String>,
    deadline: Duration,
}

impl Raw {
    fn start(check: &CoreCheck) -> std::io::Result<Raw> {
        let Some(program) = check.command.first() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the plugin's command is empty",
            ));
        };
        let mut command = Command::new(program);
        command
            .args(check.command.iter().skip(1))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (key, value) in &check.env {
            command.env(key, value);
        }
        if let Some(cwd) = &check.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut reader = crate::buffered(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {
                        let frame = line.trim_end_matches(['\n', '\r']);
                        if frame.is_empty() {
                            continue;
                        }
                        if tx.send(frame.to_string()).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Ok(Raw {
            child,
            stdin: crate::buffered_writer(stdin),
            lines: rx,
            deadline: check.deadline,
        })
    }

    fn send(&mut self, line: &str) -> std::io::Result<()> {
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()
    }

    /// Write one request and take the answer that carries its id.
    fn request(&mut self, id: u64, request: &str) -> Option<Value> {
        self.send(request).ok()?;
        self.await_id(id)
    }

    /// A well-formed `handshake`, which is the one request the mechanism knows needs no parameters.
    /// It answers it as often as it is asked, so a probe can use it to ask "are you still there".
    fn handshake(&mut self, id: u64) -> Option<Value> {
        let request = format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"handshake\"}}");
        let frame = self.request(id, &request)?;
        frame.get("result").is_some().then_some(frame)
    }

    /// The next frame, whatever it says, within the deadline.
    fn await_any(&self) -> Option<Value> {
        match self.lines.recv_timeout(self.deadline) {
            Ok(line) => serde_json::from_str(&line).ok(),
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None,
        }
    }

    /// The next frame carrying this `id`, ignoring the frames beside it. A suite that took the first
    /// frame it saw would attribute another request's answer to this one.
    fn await_id(&self, id: u64) -> Option<Value> {
        let until = Instant::now() + self.deadline;
        loop {
            let left = until.checked_duration_since(Instant::now())?;
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    if let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(&line) {
                        if frame.get("id").and_then(Value::as_u64) == Some(id) {
                            return Some(Value::Object(frame));
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => {
                    return None
                }
            }
        }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a frame is, in the words a plugin author reads: the code it refused with, or that it
/// answered. Never the raw line, which is a wall of JSON in a report line.
fn classify(frame: &Value) -> String {
    match frame.get("error").and_then(Value::as_object) {
        Some(error) => format!(
            "a refusal with code {}",
            error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN)
        ),
        None if frame.as_object().is_some_and(|m| m.contains_key("result")) => {
            "an answer".to_string()
        }
        None => "a frame with neither `result` nor `error`".to_string(),
    }
}

fn verdict(rule: &str, notes: Vec<String>) -> Result<Verdict, Vec<String>> {
    if notes.is_empty() {
        Ok(Verdict::pass(rule))
    } else {
        Err(notes)
    }
}
