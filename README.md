# pluginrpc

pluginrpc is a wire mechanism for a host program that delegates work to a program in another
process. The host starts the plugin as a child process, the two exchange one JSON message per
line over the pipes, and the plugin answers the methods that its profile declares in the
handshake. Nothing in this repository knows what your plugin does. A plugin that finds crashes
in a library and a plugin that checks claims in a document use the same rules on the wire and
share no method name.

The specification defines the mechanism, and profiles define the content, in the same relation
as TCP to the protocols on top of it. The mechanism covers framing, request ids, error codes,
the handshake envelope, capability strings, deadlines, loss, and the conformance procedure. A
profile supplies its method set, its payload schemas, its capability vocabulary, its own loss
reasons, and its own conformance rules. Every envelope is open at the top level, so a profile
adds a field beside `params` or a field to the handshake without touching this repository.

`spec/spec.md` §1 gives the vocabulary. The words carry one meaning each, and the code uses the
same words: a **message** is one line (the older word "frame" is retired), the **envelope** is
the set of fields every message carries regardless of profile, a **refusal** is a response that
carries `error`, and a **loss** is the host receiving no answer.

## Status

`prpc/1` is a preview. Every rule in the specification is normative as written, and three items
are open: whether a host must keep one plugin process's stderr separate from another's, whether
a host must report the stderr it captures, and what message size a profile must promise. §14 of
`spec/spec.md` gives each one. Declaring the revision stable removes that section and changes no
rule.

## Layout

| Path | What it is |
| --- | --- |
| `spec/spec.md` | The normative mechanism specification, revision `prpc/1`, opening with the glossary |
| `spec/schema/prpc-1/` | The normative envelope schemas (JSON Schema draft 2020-12) |
| `spec/core-rules.json` | The conformance rules every profile inherits |
| `crates/plugin-rpc/` | Rust reference implementation of the messages, the loss reasons, and the answers; a profile implements `MethodName` |

## Reference implementation

The `plugin-rpc` crate implements the envelopes, the framing, and the vocabulary of `prpc/1`.
Its 13 tests assert the wire contract exactly as a plugin written in any other language must
meet it, so a Rust plugin author who trusts the types cannot pass locally and then fail a host
that validates against the committed schemas. The tests cover the omitted-`params` rule,
blank-line loss, the open envelope, the reserved id 0, and the two rules the types enforce
rather than advise:

- A response carries exactly one of `result` or `error`. A response carrying both is a plugin
  that assigned a result and then failed, and reading the result would report success while
  discarding the failure, so `parse_response` rejects it. The types read the two arms from a
  field map, which is why a payload of `null` still counts as a result.
- `id` 0 is reserved. A host assigns ids from 1, and a plugin answers with `id` 0 when it cannot
  tell which request a refusal belongs to. `MIN_HOST_ID`, `UNCORRELATED_ID`, and `MAX_ID` state
  the bounds, and `check_id` enforces them on both directions.

`Answer` separates what the plugin said from whether the plugin said anything. A call returns an
`Answer`, which is a payload or a `Refusal`, and a refusal is an answer: the plugin was reachable
and reported that the work did not happen. `Loss` holds the five reasons a host receives no
answer, plus `Loss::Profile` for a case the mechanism cannot name, such as a result that fails
the profile's own schema. `Loss::resolve` keeps the more specific reason when two arrive together,
which is the ordering both a restart and a report depend on.
