//! The pluginrpc wire mechanism (`prpc/1`): one JSON message per line over a plugin's stdio.
//!
//! The framing helpers are shared by the host client and by plugins written in Rust, so a
//! plugin author does not reimplement line-delimited framing. Nothing here names a method: a
//! profile defines its method vocabulary as a type that implements [`MethodName`], and the
//! mechanism carries only the wire string. See `spec/spec.md` for the normative rules; the
//! doc comments cite the section they implement.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use thiserror::Error;

/// JSON-RPC parse error (spec §4).
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC invalid request (spec §4).
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC method not found (spec §4).
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Application-level failure inside the plugin (spec §4).
pub const PLUGIN_ERROR: i64 = -32000;
/// The largest id a JavaScript plugin can carry in a double (spec §3). The host assigns ids
/// at or below this bound; a host that cannot guarantee it has outlived its profile.
pub const MAX_ID: u64 = 9_007_199_254_740_991;

/// A profile's method vocabulary. The mechanism transports `name()` and knows nothing else;
/// `from_wire` is what a plugin server uses to decide a -32601.
pub trait MethodName {
    fn name(&self) -> &'static str;
    fn from_wire(name: &str) -> Option<Self>
    where
        Self: Sized;
}

/// A request. `id` is assigned by the host and echoed on the matching response (spec §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub method: String,
    /// Omitted, not `null`, when the method takes no parameters (spec §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
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

/// The only JSON-RPC revision this mechanism speaks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum JsonRpcVersion {
    #[default]
    #[serde(rename = "2.0")]
    V2,
}

/// A successful response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Success {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub result: serde_json::Value,
}

/// An error response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A failure response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub error: ErrorObject,
}

/// A response, discriminated by the presence of `result` or `error` (spec §3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Response {
    Success(Success),
    Failure(Failure),
}

impl Response {
    pub fn ok(id: u64, result: impl Serialize) -> Self {
        Response::Success(Success {
            jsonrpc: JsonRpcVersion::V2,
            id,
            result: serde_json::to_value(result).expect("result is serializable"),
        })
    }

    pub fn err(id: u64, code: i64, message: impl Into<String>) -> Self {
        Response::Failure(Failure {
            jsonrpc: JsonRpcVersion::V2,
            id,
            error: ErrorObject {
                code,
                message: message.into(),
                data: None,
            },
        })
    }

    pub fn id(&self) -> u64 {
        match self {
            Response::Success(s) => s.id,
            Response::Failure(f) => f.id,
        }
    }
}

/// The four core fields of a handshake response (spec §5). The object is open on purpose:
/// a profile adds top-level fields, serde ignores what it does not name, and the committed
/// envelope schema allows them, so a profile evolves its handshake without a mechanism
/// revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Handshake {
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    /// Opaque capability strings from the profile's vocabulary (spec §6).
    pub capabilities: Vec<String>,
}

/// Anything that can go wrong on the pipe. A lost plugin is `Eof`, not a failure to report:
/// callers map it onto the meaning their profile gives to loss (spec §7).
#[derive(Debug, Error)]
pub enum FrameError {
    #[error("io on the plugin pipe: {0}")]
    Io(#[from] std::io::Error),
    #[error("message is not serializable: {0}")]
    Encode(#[from] serde_json::Error),
    #[error("malformed json-rpc line: {0}")]
    Malformed(String),
    #[error("plugin closed the pipe")]
    Eof,
}

/// Write one message as a single line (spec §2).
pub fn write_line(writer: &mut impl Write, msg: &impl Serialize) -> Result<(), FrameError> {
    let mut line = serde_json::to_string(msg)?;
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()?;
    Ok(())
}

/// Read one message. `Ok(None)` means the peer closed the pipe or sent a blank line, which is
/// loss, not an error (spec §2).
pub fn read_line(
    reader: &mut impl BufRead,
    buf: &mut String,
) -> Result<Option<String>, FrameError> {
    buf.clear();
    let n = reader.read_line(buf)?;
    if n == 0 {
        return Ok(None);
    }
    let trimmed = buf.trim_end_matches(['\n', '\r']);
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

/// Parse one response line.
pub fn parse_response(line: &str) -> Result<Response, FrameError> {
    serde_json::from_str(line).map_err(|e| FrameError::Malformed(format!("{e}: {line}")))
}

/// Parse one request line, as a plugin server does.
pub fn parse_request(line: &str) -> Result<Request, FrameError> {
    serde_json::from_str(line).map_err(|e| FrameError::Malformed(format!("{e}: {line}")))
}

/// A line reader over a byte stream, sized so a large payload does not thrash.
pub fn buffered<R: Read>(read: R) -> BufReader<R> {
    BufReader::with_capacity(1 << 16, read)
}

/// A line writer over a byte stream.
pub fn buffered_writer<W: Write>(write: W) -> BufWriter<W> {
    BufWriter::with_capacity(1 << 16, write)
}
