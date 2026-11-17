# pluginrpc: the plugin wire mechanism, revision 1 (`prpc/1`)

This document gives the rules for a host program that delegates work to an out-of-process
plugin. It defines how messages travel, how a connection starts and ends, and how a plugin
identifies the profile it implements. A profile names the methods and the payloads; the
mechanism transports them and assigns them no meaning. Most rules state, in the same sentence,
the failure they prevent.

## 1. Layering

The mechanism defines: framing, ids, error codes, the handshake envelope, capability strings,
deadlines, loss, and the conformance procedure. A profile defines: its method set, the payload
types behind those methods, the JSON Schemas that describe them, its capability vocabulary, and
its conformance rules. The mechanism uses one method name, `handshake`. From the handshake
response, the host reads only the four fields listed in §5.

A profile is identified by its `protocolVersion` string (§5). The string selects the methods
and payload types the plugin answers: two plugins with different `protocolVersion` values can
answer entirely different methods while using identical framing. The host rejects a version it
does not support, so a profile can change its payloads under a new version string without this
document changing.

## 2. Transport and framing

The host starts the plugin from an argv array with pipes on stdin and stdout. The plugin reads
requests from stdin and writes responses to stdout. The plugin may write anything to stderr;
the mechanism assigns stderr no role.

- Each message is one UTF-8 encoded JSON value on one line, terminated by `\n`. The host and
  the plugin strip a trailing `\r` before the `\n`.
- A blank line or an end-of-file on either pipe ends the connection (§7): the plugin process
  has exited or its stdout has closed, and the host records the event as the end of the
  connection.
- Every request carries an `id` and receives exactly one response, because a host that cannot
  match an answer to a question cannot distinguish a plugin that has stopped from a plugin that
  is still working.

## 3. Requests and responses

The normative envelope schemas are in `spec/schema/prpc-1/`.

A request is `{"jsonrpc":"2.0","id":<id>,"method":<string>,"params":<value>}`. The host assigns
`id` as an integer with `0 <= id <= 2^53 - 1`; the upper bound keeps the id exact for a plugin
written in JavaScript, whose numbers cannot represent a larger integer. The host echoes `id`
unchanged on the matching response. If a method takes no parameters, `params` is omitted, not
sent as `null`: a plugin that defaults `null` and a plugin that rejects `null` would otherwise
disagree about the meaning of a request that carries no parameters.

A response is `{"jsonrpc":"2.0","id":<id>,"result":<value>}` or
`{"jsonrpc":"2.0","id":<id>,"error":{"code":<int>,"message":<string>,"data":<value>}}`, with
exactly one of `result` or `error` present. The host discards a response whose `id` matches no
request it is still waiting on: a late answer to a request the host already timed out is a
normal consequence of deadlines, and the connection stays open.

The profile defines the shape of `params` and `result`. The mechanism requires only that both
are JSON values.

## 4. Errors

| Code | Name | Meaning |
| --- | --- | --- |
| -32700 | parse error | The line received was not valid JSON-RPC. The connection continues. |
| -32600 | invalid request | The frame parsed but was not a well-formed request. |
| -32601 | method not found | The method is not in the profile the plugin announced, or the plugin will not answer it. |
| -32000 | plugin error | The plugin failed while answering. |

Codes -32099 through -32001 are reserved for profiles. A profile that needs more error detail
than `PLUGIN_ERROR` puts a typed reason in `error.data` or, better, in the `result` of the
method: a domain failure (the symbol does not exist, the input cannot be built) is returned in
`result`, so an error frame always means a transport or plugin fault and the host never has to
decode a domain failure out of one.

## 5. Lifecycle

The host starts the plugin process, sends `handshake` as the first request with no `params`,
and rejects the plugin unless `protocolVersion` in the response names a profile the host
supports. The check comes before any other method call, because until the profile is known the
host cannot interpret any other response.

The handshake response carries four core fields:

```json
{
  "protocolVersion": "demo/1",
  "name": "mylang",
  "version": "0.3.0",
  "capabilities": ["alpha", "beta"]
}
```

The handshake object is open: a profile adds its own top-level fields, and the committed schema
(`handshake.json`) allows them. The host reads only the four core fields. An open envelope lets
a profile change its handshake within its own revision, and each profile sets field strictness
in its own schema.

The connection stays open while work remains, and the host may keep several requests waiting
for responses. The host decides between one connection and a pool of plugin processes as host
policy.

## 6. Capabilities

`capabilities` is an array of strings, drawn from the profile's vocabulary, that the plugin
declares it can answer. The mechanism requires one property: a declared capability must be
answerable — for every capability the handshake declares, the methods it names must respond to
well-formed requests during the connection. If a capability is absent, the host follows the
behavior the profile fixes for that absence and reports what it skipped. The mechanism does
not interpret capability strings; only the profile defines their meaning.

## 7. Loss

A plugin that closes its stdout, exits, or stops answering within the deadline the host set
has produced a *loss*. The host records the reason (no answer, closed pipe, process exit, host
kill) and stops using the connection. The profile defines what the host does with the work
that was waiting on that connection — skip it, fail it, or retry it — because only the profile
knows what was pending.

## 8. Deadlines

A request that triggers work MAY carry `deadlineMs` in its params. The plugin SHOULD finish or
return before the deadline; the host MAY kill the plugin or close the connection after the
deadline passes. The kill mechanism and any grace period are host policy.

## 9. What a profile must define

A profile is complete if it names:

1. its `protocolVersion` string, as `<namespace>/<revision>`;
2. its method set, with a normative JSON Schema (draft 2020-12) for every request's params and
   every response's result;
3. its capability vocabulary, and the host's behavior if each capability is absent;
4. the mapping from loss (§7) to domain meaning;
5. its manifest filename (§10);
6. its conformance rules on top of the core rules (§11).

A profile MAY set `additionalProperties: false` on its own payload schemas if it wants strict
field checking. The schemas are the only place field strictness is set: conformance validates
against the schemas, and deserializers enforce exactly what the schemas state.

## 10. Plugin manifest

A plugin package carries a JSON manifest whose filename the profile fixes. The shape is the
mechanism's:

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

`install` holds slow, once-per-checkout steps, and only installation runs them. `build`
compiles the plugin and the emitter, and both installation and validation run it, so
validation always tests current source. `command` is the plugin's argv, which the host runs
verbatim.
`conformance` is described in §11.

## 11. Conformance

A suite runs each rule against the plugin. Each rule ends in one of three results: pass,
violation (the plugin's answer failed the rule's check, recorded with a message naming the
rule), or not applicable (the rule could not be run). The plugin conforms when every
applicable rule passes. The mechanism fixes the procedure:

1. The plugin's manifest names a conformance **emitter**, a program the suite runs as
   `<argv> --out <dir>`. The emitter writes a fixture project to that directory and writes a
   manifest naming every subject the suite must use. The emitter produces inputs only; the
   suite produces results. Every subject is either named in the fixture manifest or answered
   by the plugin itself, so an empty or incomplete fixture produces a report that shows the
   missing subjects instead of a pass.
2. The suite validates every response against the profile's committed schema **before**
   deserializing it. A plugin that answers with a shape the schema rejects receives a message
   written in schema terms — field name, expected type — rather than an internal error from
   the suite's decoder, so the author sees the rule from the schema they implemented against
   named in the failure.
3. Rules the suite could not run — the plugin did not declare the capability, an earlier rule
   ended the run — are counted as not applicable, kept apart from the passes in the report.
4. Every rule must be verified: the profile's test suite includes one scripted defect that
   makes exactly that rule report a violation. A rule with no scripted defect is unverified,
   because nothing has shown that the rule can detect the failure it targets.
5. A suite that cannot start its own fixture reports a suite error, not a plugin violation,
   because a report that attributes the suite's own failure to the plugin stops authors from
   trusting reports.

The core rules, in the order a suite runs them, are `spec/core-rules.json`. A profile extends
that list with its own rules.

## 12. Versioning

This document is revision `prpc/1`. A mechanism revision changes only framing, envelopes,
lifecycle, or the conformance procedure, and it is published together with the envelope schemas
under a new `prpc-<n>/` directory. A profile revision is independent: `demo/2` may keep using
`prpc/1`. A profile that adds a handshake field or a method needs no mechanism revision,
because the envelope is open (§5) and the method vocabulary is the profile's (§9).
