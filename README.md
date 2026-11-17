# pluginrpc

pluginrpc is a wire mechanism for a host program that delegates work to an out-of-process
plugin. The host starts the plugin as a child process, the two exchange one JSON message per
line over the pipes, and the plugin answers the methods that its profile declares in the
handshake.

The specification defines the mechanism, and profiles define the content, in the same relation
as TCP to the protocols on top of it. The mechanism covers framing, request ids, error codes,
the handshake envelope, capability strings, deadlines, loss, and the conformance procedure. A
profile supplies its method set, its payload schemas, its capability vocabulary, and its own
conformance rules. The core schemas are open objects, so a profile adds handshake fields
without touching this repository.

## Layout

| Path | What it is |
| --- | --- |
| `spec/spec.md` | The normative mechanism specification, revision `prpc/1` |
| `spec/schema/prpc-1/` | The normative envelope schemas (JSON Schema draft 2020-12) |
| `spec/core-rules.json` | The conformance rules every profile inherits |
| `crates/plugin-rpc/` | Rust reference implementation of framing and the envelopes; a profile implements `MethodName` |

## Reference implementation

The `plugin-rpc` crate implements the framing and the envelopes of `prpc/1`. Its test suite
asserts the wire contract exactly as a plugin written in any other language must meet it:
envelope shapes, the omitted-`params` rule, blank-line loss, and the open handshake.
