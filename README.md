# pluginrpc

pluginrpc is the wire mechanism behind two working plugin systems: the autofuzz harness
protocol (`afz/1`) and the veridoc plugin protocol (`vd/1`). A host program starts a plugin as a
child process, the two exchange one JSON message per line over the pipes, and the plugin answers
the methods that its profile declares in the handshake. Both projects built this transport
independently and shipped it twice; this repository holds the extracted mechanism, so a third
consumer writes payloads instead of plumbing.

The specification defines the mechanism, and profiles define the content, in the same relation as
TCP to the protocols on top of it. The mechanism covers framing, request ids, error codes, the
handshake envelope, capability strings, deadlines, loss, and the conformance procedure. A profile
supplies its method set, its payload schemas, its capability vocabulary, and its own conformance
rules. The core schemas are open objects, so a profile adds handshake fields without touching this
repository.

## Layout

| Path | What it is |
| --- | --- |
| `spec/spec.md` | The normative mechanism specification, revision `prpc/1` |
| `spec/schema/prpc-1/` | The normative envelope schemas (JSON Schema draft 2020-12) |
| `spec/core-rules.json` | The conformance rules every profile inherits |
| `crates/plugin-rpc/` | Rust reference implementation of framing; a profile implements `MethodName` |

## Status

The specification and the framing crate are extracted from code that ships in autofuzz and
veridoc today, so the mechanism is proven rather than designed on paper. The host client, the
plugin-side server loop, and the conformance harness from both repositories will follow as
reference-implementation crates. Profiles are not part of this repository: `afz/1` lives in
autofuzz, `vd/1` lives in veridoc, and each keeps its own payload schemas and rules.
