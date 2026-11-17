//! The pluginrpc wire mechanism (`prpc/1`): one JSON message per line over a plugin's stdio.
//!
//! The framing helpers are shared by the host client and by plugins written in Rust, so a
//! plugin author does not reimplement line-delimited framing. Nothing here names a method: a
//! profile defines its method vocabulary as a type that implements [`MethodName`], and the
//! mechanism carries only the wire string. See `spec/spec.md` for the normative rules and §1 of
//! that file for the vocabulary this crate's names come from: a *message* is one line, the
//! *envelope* is the fields every message carries regardless of profile, a *refusal* is a
//! response carrying `error`, and a *loss* is the host receiving no answer.

pub mod process;

use std::io::{BufRead, BufReader, BufWriter, Read, Write};

use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

/// JSON-RPC parse error (spec §5).
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC invalid request (spec §5).
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC method not found (spec §5).
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Application-level failure inside the plugin (spec §5).
pub const PLUGIN_ERROR: i64 = -32000;

/// The [`Loss::Profile`] reason the mechanism reports when a response carried a result the
/// profile's own type could not read (spec §8). A profile that names its own reason for that case
/// replaces this one.
pub const UNUSABLE_RESULT: &str = "unusable-result";

/// The largest id a JavaScript plugin can carry in a double (spec §4). The host assigns ids
/// at or below this bound; a host that cannot guarantee it has outlived its profile.
pub const MAX_ID: u64 = 9_007_199_254_740_991;
/// The lowest id a host may assign. The host never sends 0, because 0 is the id a plugin uses
/// for a refusal whose request id it could not determine (spec §4, §5).
pub const MIN_HOST_ID: u64 = 1;
/// The id a plugin echoes when it cannot determine which request a refusal answers (spec §5).
pub const UNCORRELATED_ID: u64 = 0;

/// A profile's method vocabulary. The mechanism transports `name()` and knows nothing else;
/// `from_wire` is what a plugin server uses to decide a -32601.
pub trait MethodName {
    fn name(&self) -> &'static str;
    fn from_wire(name: &str) -> Option<Self>
    where
        Self: Sized;
}

/// A request message. `id` is assigned by the host and echoed on the matching response
/// (spec §4).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub method: String,
    /// Omitted, not `null`, when the method takes no parameters (spec §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl Request {
    pub fn new(id: u64, method: impl MethodName, params: impl Serialize) -> Self {
        Request {
            jsonrpc: JsonRpcVersion::V2,
            id,
            method: method.name().to_string(),
            params: Some(serde_json::to_value(params).expect("request params are serializable")),
        }
    }

    /// A method with no parameters.
    pub fn empty(id: u64, method: impl MethodName) -> Self {
        Request {
            jsonrpc: JsonRpcVersion::V2,
            id,
            method: method.name().to_string(),
            params: None,
        }
    }
}

/// A request message must carry an id the host could have assigned: 0 is reserved for a plugin's
/// uncorrelated refusal, and an id above [`MAX_ID`] is not exact for a JavaScript plugin.
impl<'de> Deserialize<'de> for Request {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            jsonrpc: JsonRpcVersion,
            id: u64,
            method: String,
            #[serde(default)]
            params: Option<Value>,
        }

        let wire = Wire::deserialize(deserializer)?;
        check_id(wire.id, MIN_HOST_ID).map_err(de::Error::custom)?;
        if wire.method.is_empty() {
            return Err(de::Error::custom(
                "`method` is empty; a request must name the method it calls",
            ));
        }
        Ok(Request {
            jsonrpc: wire.jsonrpc,
            id: wire.id,
            method: wire.method,
            params: wire.params,
        })
    }
}

/// The only JSON-RPC revision this mechanism speaks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum JsonRpcVersion {
    #[default]
    #[serde(rename = "2.0")]
    V2,
}

/// A response whose envelope carries `result`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Success {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub result: Value,
}

/// The `error` object of a refusal. A profile that needs more detail than the code and the text
/// puts a typed reason in `data` (spec §5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// A response whose envelope carries `error`: a refusal. A refusal is an answer, not a loss
/// (spec §8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub error: ErrorObject,
}

/// A response message, discriminated by the presence of `result` or `error` (spec §4).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Response {
    Success(Success),
    Failure(Failure),
}

/// Exactly one of `result` or `error` is enforced rather than advised (spec §4). A response
/// carrying both is a plugin that assigned a result and then failed, and reading the result
/// would report success while discarding the failure; a response carrying neither is the same
/// fault in the other direction. The two arms are read from a field map rather than from a
/// derived struct so that `"result": null` counts as a present result: `null` is a payload the
/// profile may legitimately return, and a derived `Option<Value>` would read it as absent.
impl<'de> Deserialize<'de> for Response {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut fields = Map::<String, Value>::deserialize(deserializer)?;
        let jsonrpc: JsonRpcVersion = take_field(&mut fields, "jsonrpc")
            .map_err(de::Error::custom)?
            .ok_or_else(|| de::Error::custom("a response has no `jsonrpc` field"))?;
        let id: u64 = take_field(&mut fields, "id")
            .map_err(de::Error::custom)?
            .ok_or_else(|| de::Error::custom("a response has no `id` field"))?;
        check_id(id, UNCORRELATED_ID).map_err(de::Error::custom)?;
        let result: Option<Value> = take_field(&mut fields, "result").map_err(de::Error::custom)?;
        let error: Option<ErrorObject> =
            take_field(&mut fields, "error").map_err(de::Error::custom)?;
        match (result, error) {
            (Some(result), None) => Ok(Response::Success(Success {
                jsonrpc,
                id,
                result,
            })),
            (None, Some(error)) => Ok(Response::Failure(Failure { jsonrpc, id, error })),
            (Some(_), Some(_)) => Err(de::Error::custom(
                "a response carries exactly one of `result` or `error`; this one carries both",
            )),
            (None, None) => Err(de::Error::custom(
                "a response carries exactly one of `result` or `error`; this one carries neither",
            )),
        }
    }
}

impl Response {
    pub fn result(id: u64, value: impl Serialize) -> Self {
        Response::Success(Success {
            jsonrpc: JsonRpcVersion::V2,
            id,
            result: serde_json::to_value(value).expect("result is serializable"),
        })
    }

    /// A refusal: the plugin could not do the work. This is an answer, not a loss.
    pub fn refusal(id: u64, code: i64, text: impl Into<String>) -> Self {
        Response::Failure(Failure {
            jsonrpc: JsonRpcVersion::V2,
            id,
            error: ErrorObject {
                code,
                message: text.into(),
                data: None,
            },
        })
    }

    /// A refusal whose request id the plugin could not determine (spec §5).
    pub fn uncorrelated(code: i64, text: impl Into<String>) -> Self {
        Response::refusal(UNCORRELATED_ID, code, text)
    }

    pub fn id(&self) -> u64 {
        match self {
            Response::Success(s) => s.id,
            Response::Failure(f) => f.id,
        }
    }

    /// What the call ended as, with the payload read into the type the profile declared. A
    /// response is always an answer, so this never reports a loss for a refusal. It does report a
    /// loss when the result does not fit the declared type, because that is an answer the profile
    /// cannot use, and §8 files that under loss with the reason [`UNUSABLE_RESULT`].
    pub fn into_call<T: de::DeserializeOwned>(self) -> Call<T> {
        match self {
            Response::Success(s) => serde_json::from_value(s.result).map_or_else(
                |e| {
                    Call::Lost(Loss::Profile {
                        reason: UNUSABLE_RESULT.to_string(),
                        detail: e.to_string(),
                    })
                },
                Call::Result,
            ),
            Response::Failure(f) => Call::Refusal(f.error),
        }
    }
}

/// What one plugin call ended as (spec §8). The three endings ask three different things of the
/// host: a payload to use, a reason to report while keeping the process, or a process to restart.
///
/// A profile returns this from its own call method. A host that returns `Result<T, its own error>`
/// instead has nowhere to put a refusal, and reports an answer as a lost plugin; the two hosts
/// that drove this type had each written that alias, under the same name, before it existed.
///
/// This is a host-side value and not a message: it carries no `jsonrpc` and no `id`, so it is
/// never serialized and no envelope schema describes it.
#[derive(Debug, Clone, PartialEq)]
pub enum Call<T> {
    /// The plugin answered with a payload the profile could read.
    Result(T),
    /// The plugin answered that the work did not happen. The process is alive, so the host keeps
    /// it: a refusal is an answer, not a fault in the connection (spec §8).
    Refusal(ErrorObject),
    /// No answer arrived. This is the only case that says anything about the process.
    Lost(Loss),
}

impl<T> Call<T> {
    /// The payload, when the plugin produced one the profile could read.
    pub fn payload(self) -> Option<T> {
        match self {
            Call::Result(value) => Some(value),
            _ => None,
        }
    }

    /// The refusal, when the plugin answered with one.
    pub fn refusal(self) -> Option<ErrorObject> {
        match self {
            Call::Refusal(error) => Some(error),
            _ => None,
        }
    }

    /// Why no answer arrived, when none did.
    pub fn loss(self) -> Option<Loss> {
        match self {
            Call::Lost(loss) => Some(loss),
            _ => None,
        }
    }

    /// Whether the plugin answered. Only [`Call::Lost`] says anything about the process, so this
    /// is the test a host makes before deciding whether to restart.
    pub fn answered(&self) -> bool {
        !matches!(self, Call::Lost(_))
    }

    /// Put the payload in the shape the caller wants, leaving a refusal and a loss untouched.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Call<U> {
        match self {
            Call::Result(value) => Call::Result(f(value)),
            Call::Refusal(error) => Call::Refusal(error),
            Call::Lost(loss) => Call::Lost(loss),
        }
    }
}

/// The four core fields of a handshake response (spec §6). The object is open on purpose:
/// a profile adds top-level fields, serde ignores what it does not name, and the committed
/// envelope schema allows them, so a profile evolves its handshake without a mechanism
/// revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Handshake {
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    /// Opaque capability strings from the profile's vocabulary (spec §7).
    pub capabilities: Vec<String>,
}

/// The two constraints `handshake.json` places on the core fields are checked here so the
/// reference type and the committed schema accept the same handshakes (spec §6, §7):
/// `protocolVersion` matches `<namespace>/<revision>`, and `capabilities` carries no duplicates.
impl<'de> Deserialize<'de> for Handshake {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            protocol_version: String,
            name: String,
            version: String,
            capabilities: Vec<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        if !is_revision(&wire.protocol_version) {
            return Err(de::Error::custom(format!(
                "`protocolVersion` must be `<namespace>/<revision>`, as in \"demo/1\"; got {:?}",
                wire.protocol_version
            )));
        }
        if wire.capabilities.iter().any(String::is_empty) {
            return Err(de::Error::custom(
                "`capabilities` declares an empty capability; declare each with a non-empty string",
            ));
        }
        let mut sorted = wire.capabilities.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != wire.capabilities.len() {
            return Err(de::Error::custom(
                "`capabilities` declares the same capability twice; declare each once",
            ));
        }
        Ok(Handshake {
            protocol_version: wire.protocol_version,
            name: wire.name,
            version: wire.version,
            capabilities: wire.capabilities,
        })
    }
}

/// Whether `version` matches the `<namespace>/<revision>` form of `handshake.json`. Hand-rolled
/// so the reference type checks the pattern without a regular-expression dependency.
fn is_revision(version: &str) -> bool {
    let Some((namespace, revision)) = version.split_once('/') else {
        return false;
    };
    let namespace_ok = !namespace.is_empty()
        && namespace
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let revision_ok = !revision.is_empty() && revision.chars().all(|c| c.is_ascii_digit());
    namespace_ok && revision_ok
}

/// The reason the host received no answer (spec §8). A refusal is not a loss: a refusal means the
/// connection delivered the request and brought back an answer that says the work did not happen.
///
/// A profile adds a reason with [`Loss::Profile`] for the cases where the mechanism delivered an
/// answer the profile cannot use, such as a result that fails the profile's own schema. The
/// profile names the reason and says what the host does with the work that was waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "loss")]
pub enum Loss {
    /// The process could not be started.
    Spawn { detail: String },
    /// The plugin closed its stdout.
    Eof,
    /// The plugin process ended. `signal` is absent where the platform reports no signals.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// The host ended the plugin.
    Killed,
    /// The deadline the host set passed with the request still waiting.
    NoAnswer,
    /// A profile's own reason, named by the profile (spec §8).
    Profile { reason: String, detail: String },
}

impl Loss {
    /// The reason to record when `observed` arrives and `recorded` already holds one (spec §8).
    ///
    /// The first cause stands, because the observations that follow a cause are usually that
    /// cause's consequences. Preferring the later one mis-attributes the death: a `killed` recorded
    /// after an `exited` would credit the host with ending a process that had already gone, the
    /// opposite of what recording the kill first is for. Two reasons name what an observer noticed
    /// rather than what happened to the process — `eof` is what the reader saw, `no answer` is what
    /// the deadline saw — and a later reason replaces either of those.
    pub fn resolve(recorded: Option<Loss>, observed: Loss) -> Loss {
        match recorded {
            Some(previous) if previous.rank() >= observed.rank() => previous,
            _ => observed,
        }
    }

    /// Whether the host asked for this. Only [`Loss::Killed`] means the host ended the plugin, and
    /// a host classifies a kill with a request in flight differently from a death the plugin chose.
    pub fn host_requested(&self) -> bool {
        matches!(self, Loss::Killed)
    }

    /// What happened to the process outranks what an observer noticed, and a passed deadline
    /// outranks nothing. Two reasons of the same kind tie, and a tie keeps the first.
    fn rank(&self) -> u8 {
        match self {
            Loss::NoAnswer => 0,
            Loss::Eof => 1,
            _ => 2,
        }
    }
}

impl std::fmt::Display for Loss {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Loss::Spawn { detail } => write!(f, "cannot start the plugin: {detail}"),
            Loss::Eof => write!(f, "the plugin closed the pipe"),
            Loss::Exited {
                code: Some(code),
                signal: None,
            } => write!(f, "the plugin exited with code {code}"),
            Loss::Exited {
                code: None,
                signal: Some(signal),
            } => write!(f, "the plugin was ended by signal {signal}"),
            Loss::Exited { .. } => write!(f, "the plugin exited"),
            Loss::Killed => write!(f, "the host killed the plugin"),
            Loss::NoAnswer => write!(f, "the plugin did not answer before its deadline"),
            Loss::Profile { reason, detail } => {
                write!(f, "the plugin answered unusably ({reason}): {detail}")
            }
        }
    }
}

/// Anything that can go wrong while reading or writing one message. A lost plugin is
/// [`MessageError::Eof`], not a failure to report: callers map it onto [`Loss`] (spec §8).
#[derive(Debug, Error)]
pub enum MessageError {
    #[error("io on the plugin pipe: {0}")]
    Io(#[from] std::io::Error),
    #[error("message is not serializable: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("malformed message: {0}")]
    Malformed(String),
    #[error("plugin closed the pipe")]
    Eof,
}

/// Write one message as a single line (spec §3).
pub fn write_line(writer: &mut impl Write, msg: &impl Serialize) -> Result<(), MessageError> {
    let mut line = serde_json::to_string(msg)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// Read one message. `Ok(None)` means the peer closed the pipe or sent a blank line, which is
/// loss, not an error (spec §3).
pub fn read_line(
    reader: &mut impl BufRead,
    buf: &mut String,
) -> Result<Option<String>, MessageError> {
    buf.clear();
    if reader.read_line(buf)? == 0 {
        return Ok(None);
    }
    let trimmed = buf.trim_end_matches(['\n', '\r']).to_string();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed))
}

/// Parse one response message. A receiver that cannot decode a message drops it and keeps the
/// connection open; a dropped response ends its request at the deadline (spec §3).
pub fn parse_response(line: &str) -> Result<Response, MessageError> {
    serde_json::from_str(line).map_err(|e| MessageError::Malformed(format!("{e}: {line}")))
}

/// Parse one request message, as a plugin server does. A plugin answers a message it cannot
/// decode with a refusal carrying [`PARSE_ERROR`], and [`Response::uncorrelated`] when it cannot
/// determine the id (spec §3, §5).
pub fn parse_request(line: &str) -> Result<Request, MessageError> {
    serde_json::from_str(line).map_err(|e| MessageError::Malformed(format!("{e}: {line}")))
}

/// A line reader over a byte stream, sized so a large payload does not thrash.
pub fn buffered<R: Read>(read: R) -> BufReader<R> {
    BufReader::with_capacity(1 << 16, read)
}

/// A line writer over a byte stream.
pub fn buffered_writer<W: Write>(write: W) -> BufWriter<W> {
    BufWriter::with_capacity(1 << 16, write)
}

/// Whether `id` is inside the range the envelope schemas allow. `lowest` is [`MIN_HOST_ID`] for a
/// request, which a host assigns, and [`UNCORRELATED_ID`] for a response, where 0 is legal.
fn check_id(id: u64, lowest: u64) -> Result<(), String> {
    if id < lowest {
        return Err(format!(
            "id {id} is below the lowest legal id {lowest}; 0 is reserved for a refusal whose \
             request id the plugin could not determine"
        ));
    }
    if id > MAX_ID {
        return Err(format!(
            "id {id} is above {MAX_ID}, which a plugin written in JavaScript cannot represent exactly"
        ));
    }
    Ok(())
}

/// Remove one envelope field and decode it, so the presence of a field is decided by the map and
/// not by whether its value happens to be `null`.
fn take_field<V: serde::de::DeserializeOwned>(
    fields: &mut Map<String, Value>,
    name: &str,
) -> Result<Option<V>, serde_json::Error> {
    fields.remove(name).map(serde_json::from_value).transpose()
}
