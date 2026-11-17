# pluginrpc: the plugin wire mechanism, revision 1 (`prpc/1`)

This document gives the normative rules for a host program that delegates work to an
out-of-process plugin. It defines how messages travel, how a connection starts and ends, and how
a plugin proves it speaks its profile. It does not define what any plugin does: the two profiles
in use today, `afz/1` (autofuzz harnesses) and `vd/1` (veridoc plugins), answer different methods
with different payloads, and the mechanism never reads either. Where a rule exists to stop a
specific failure, the failure sits next to it.

## 1. Layering

The mechanism owns the pipe: framing, ids, error codes, the handshake envelope, capability
strings, deadlines, loss, and the conformance procedure. A profile owns the content: its method
set, the payload types behind those methods, the JSON Schemas that describe them, its capability
vocabulary, and its conformance rules. Nothing in this document names a method other than
`handshake`, and `handshake` carries no payload a host interprets beyond the four envelope fields
in §5.

A profile is identified by its `protocolVersion` string (§5), which plays the role a port number
plays for TCP: it says which upper-layer protocol the bytes belong to. The host refuses a version
it does not speak, so a profile can change its payloads freely under a new version string without
this document changing.

## 2. Transport and framing

The host starts the plugin from an argv array with pipes on stdin and stdout. The plugin reads
requests from stdin and writes responses to stdout. stderr is the plugin's for logging; the
mechanism gives it no meaning.

- Each message is one UTF-8 encoded JSON value on one line, terminated by `\n`. A trailing `\r`
  before the `\n` is stripped.
- A blank line or an end-of-file on either pipe means the connection is lost (§7). It is never a
  protocol error: a plugin that closes its stdout has died or left, and inventing an error frame
  for it would blame the peer for the pipe.
- Every request carries an `id`. Notifications and batching are not part of the mechanism,
  because a host that cannot match an answer to a question cannot tell a lost plugin from a
  silent one.

## 3. Requests and responses

The envelopes are normative in `spec/schema/prpc-1/`.

A request is `{"jsonrpc":"2.0","id":<id>,"method":<string>,"params":<value>}`. The host assigns
`id` as an integer with `0 <= id <= 2^53 - 1`; the upper bound keeps the id exact for a plugin
written in JavaScript, whose numbers cannot represent a larger integer. The host echoes `id`
unchanged on the matching response. When a method takes no parameters, `params` is omitted, not
sent as `null`: a plugin that defaults `null` and a plugin that rejects `null` would otherwise
disagree about a request nobody described.

A response is `{"jsonrpc":"2.0","id":<id>,"result":<value>}` or
`{"jsonrpc":"2.0","id":<id>,"error":{"code":<int>,"message":<string>,"data":<value>}}`, with
exactly one of `result` or `error` present. A response whose `id` matches no in-flight request is
discarded, not an error: a late answer to a request the host already timed out is a normal
consequence of deadlines, and killing the connection over it would turn a slow plugin into a
failed run.

The shape of `params` and `result` belongs to the profile. The mechanism requires only that both
are JSON values.

## 4. Errors

| Code | Name | Meaning |
| --- | --- | --- |
| -32700 | parse error | The line received was not valid JSON-RPC. The connection continues. |
| -32600 | invalid request | The frame parsed but was not a well-formed request. |
| -32601 | method not found | The method is not in the plugin's profile, or the plugin declined to answer it. |
| -32000 | plugin error | The plugin failed while answering. |

Codes -32099 through -32001 are reserved for profiles. A profile that needs more error detail
than `PLUGIN_ERROR` carries puts a typed reason in `error.data` or, better, in the `result` of the
method: a domain failure (the symbol does not exist, the input cannot be built) is an answer, not
a transport error, and treating it as one would make every domain failure look like a broken pipe.

## 5. Lifecycle

The host starts the plugin process, sends `handshake` as the first request with no `params`, and refuses
the plugin unless the response's `protocolVersion` is one the host speaks. Refusal comes before
any other method call, because a host that calls first and checks later reads unknown payloads as
known ones.

The handshake response carries four core fields:

```json
{
  "protocolVersion": "afz/1",
  "name": "js-ts",
  "version": "0.3.0",
  "capabilities": ["coverage", "minimize"]
}
```

The handshake object is open: a profile adds its own top-level fields (autofuzz adds
`targetClass`; veridoc adds none), and the committed schema (`handshake.json`) allows them. The
host reads only the four core fields. An open envelope is what lets a profile evolve its handshake
without a mechanism revision, and it replaces the per-field strictness the two source
implementations applied inconsistently.

The connection stays open for the life of the work, and the host may keep several requests in
flight. Whether the host runs one connection or a pool of plugin processes is host policy, not
mechanism.

## 6. Capabilities

`capabilities` is an array of strings the plugin declares it can answer, drawn from the profile's
vocabulary. The mechanism requires one property: a declared capability must be answerable — for
every capability the handshake declares, the methods it names must respond to well-formed
requests during the connection. A missing capability is never a failure and never silently a
pass: the profile fixes what the host does without it, and the host reports what it skipped.
The mechanism attaches no meaning to any capability string.

## 7. Loss

A plugin that closes its stdout, exits, or stops answering within the deadline it was given has
produced a *loss*, not an error frame. The host records the reason (no answer, closed pipe,
process exit, engine kill) and stops using the connection. What a loss means for the work — a
skipped test case, a failed check, a retried call — belongs to the profile, because only the
profile knows what was in flight. The two source implementations agree on this rule and differ
only in that mapping, which is the point of the split.

## 8. Deadlines

A request that triggers work MAY carry `deadlineMs` in its params. The plugin SHOULD finish or
return before the deadline; the host MAY kill the plugin or retire the connection when the
deadline passes. The kill mechanism and any grace period are host policy. A plugin that needs
more time than a deadline allows is a profile-level mismatch, not something the plugin negotiates
on the wire.

## 9. What a profile must define

A profile is complete when it names:

1. its `protocolVersion` string, as `<namespace>/<revision>`;
2. its method set, with a normative JSON Schema (draft 2020-12) for every request's params and
   every response's result;
3. its capability vocabulary, and the host's behavior when each capability is absent;
4. the mapping from loss (§7) to domain meaning;
5. its manifest filename (§10);
6. its conformance rules on top of the core rules (§11).

A profile MAY set `additionalProperties: false` on its own payload schemas when it wants strict
field checking. Strictness lives in the schemas, because the schema is the contract that
conformance validates; host and plugin deserializers add no policy of their own on top of it.

## 10. Plugin manifest

A plugin package carries a JSON manifest whose filename the profile fixes (`afz-harness.json`,
`veridoc-plugin.json`). The shape is the mechanism's:

```json
{
  "name": "mylang",
  "install": [["npm", "ci"]],
  "build": [["npm", "run", "build"]],
  "command": ["node", "dist/server.js"],
  "conformance": {
    "emitter": ["node", "conformance/emit.mjs"],
    "deadlineMs": 1500
  }
}
```

`install` holds slow, once-per-checkout steps, and only installation runs them. `build` compiles
the plugin and the emitter, and both installation and validation run it, so a validation never
tests a stale build. `command` is the plugin's argv; the host appends nothing. `conformance` is
described in §11.

## 11. Conformance

A plugin conforms when a suite drives it over the real transport and no rule fires. The
mechanism fixes the procedure, and it is the part both source implementations proved out:

1. The plugin's manifest names a conformance **emitter**, a program the suite runs as
   `<argv> --out <dir>`. The emitter renders a fixture project and writes a manifest naming every
   subject the suite must drive. The emitter never decides pass or fail, and the suite reads no
   source the target language owns: every subject is named in the fixture manifest or came from
   the plugin's own answers. This is what stops an empty fixture from passing silently.
2. The suite validates every response against the profile's committed schema **before**
   deserializing it. A plugin that drifts from the published contract then hears the contract's
   own vocabulary, not a decoder's internal message, and a non-Rust plugin author is validated by
   the same artifact they wrote code against.
3. Rules the suite could not evaluate — the plugin did not declare the capability, an earlier
   rule ended the run — are counted as not applicable and never shown as passes.
4. A rule must be falsifiable: the profile's test suite scripts the one defect that makes each
   rule fire, and a rule with no scripted defect is unproven. autofuzz ships 17 scripted
   defects of this kind; veridoc's suite proves only that its fake plugin passes, which is why
   this is a mechanism rule and not a suggestion.
5. A suite that cannot start its own fixture is reported as a suite error, not as a plugin
   violation, because blaming the plugin for a broken emitter teaches authors to ignore reports.

The core rules, in the order a suite drives them, are `spec/core-rules.json`. Profiles add rules
beside them; profiles never remove a core rule.

## 12. Versioning

This document is revision `prpc/1`. A mechanism revision changes only framing, envelopes,
lifecycle, or the conformance procedure, and it is published together with the envelope schemas
under a new `prpc-<n>/` directory. A profile revision is independent: `afz/2` may ride `prpc/1`
forever. A profile that adds a handshake field or a method needs no mechanism revision, because
the envelope is open (§5) and the method vocabulary is the profile's (§9).
