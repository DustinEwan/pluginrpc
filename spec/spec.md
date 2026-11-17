# pluginrpc: the plugin wire mechanism, revision 1 (`prpc/1`)

This document gives the rules for a host program that delegates work to an out-of-process
plugin. It defines how messages travel, how a connection starts and ends, and how a plugin
identifies the profile it implements. A profile names the methods and the payloads; the
mechanism transports them and assigns them no meaning. Most rules state, in the same sentence,
the failure they prevent.

## Status

`prpc/1` is a preview. Every rule written below is normative as stated: a plugin that breaks one
of them is wrong, and a host may reject it. What is not settled is the short list in §14, and the
mechanism states nothing about those items in either direction, so a profile must not depend on
an answer. A profile written against `prpc/1` should expect to change before the revision is
declared stable. Declaring the revision stable removes this paragraph and §14; it changes no
rule.

## 1. Terms

The mechanism uses a closed vocabulary. Each term below has one meaning in this document, and
no synonym is intended.

| Term | Definition |
| --- | --- |
| **mechanism** | This specification: framing, ids, error codes, the handshake envelope, capability strings, deadlines, loss, and the conformance procedure. It assigns no meaning to any method or payload. |
| **profile** | The domain layer on top of the mechanism. A profile supplies its method set, its payload schemas, its capability vocabulary, its loss reasons, and its conformance rules. |
| **host** | The program that starts the plugin and sends requests. A host owns process management, deadlines, and what happens to work that was waiting when a plugin was lost. |
| **plugin** | The program the host starts. A plugin reads requests from its stdin and writes responses to its stdout. |
| **message** | One JSON value on one line. A message is either a request or a response. *Retired:* "frame" — older text and the word "frame" in code names mean message. |
| **framing** | The rule that separates one message from the next: a `\n` terminator, with a trailing `\r` stripped. Framing says nothing about the content of a message. |
| **envelope** | The fields every message carries regardless of the profile: `jsonrpc`, `id`, and `method` and `params` on a request, or `jsonrpc`, `id`, and exactly one of `result` or `error` on a response. The envelope is the mechanism's; the payload inside it is the profile's. |
| **payload** | The JSON value inside `params` or inside `result`. The profile defines its schema and its strictness. |
| **refusal** | A response whose envelope carries `error`. A refusal is an answer: the plugin was reachable, understood the request, and reported that it could not do the work. A refusal is not a loss. |
| **loss** | The host received no answer. The five reasons are in §8. A refusal is not a loss. |
| **connection** | One started plugin process and the pipes the host opens on it. A connection ends at loss (§8). |
| **handshake** | The one method the mechanism names, sent first on every connection, whose response declares the profile and the capabilities (§6, §7). |
| **capability** | An opaque string from the profile's vocabulary that the handshake declares the plugin can answer (§7). |
| **revision** | A versioned layer, written `<namespace>/<number>`. `prpc/1` is a mechanism revision; `demo/1` is a profile revision (§13). |
| **malformed message** | A message that fails the envelope schemas: not JSON, not JSON-RPC, an `id` outside the allowed range, or a response that does not carry exactly one of `result` or `error`. |
| **manifest** | The JSON file in a plugin package that names its install steps, its build steps, and its argv (§11). |
| **emitter** | The program a plugin package supplies so the conformance suite can build its own fixture (§12). |
| **rule**, **violation**, **not applicable** | The three outcomes a conformance rule can report (§12). |

Two words need care because they carry a second use in this document. `message` means the wire
object above; `error.message` is a field name inside a refusal and is fixed by the envelope
schemas, so the text calls its contents the refusal's *text* rather than its message.
`payload` and `params` are not synonyms: `params` is the envelope field, and a payload is
whatever value sits inside that field or inside `result`.

## 2. Layering

The mechanism defines framing, ids, error codes, the handshake envelope, capability strings,
deadlines, loss, and the conformance procedure. A profile defines its method set, the payload
types behind those methods, the JSON Schemas that describe them, its capability vocabulary, and
its conformance rules. The mechanism uses one method name, `handshake`. From the handshake
response, the host reads only the four fields listed in §6.

A profile is identified by its `protocolVersion` string (§6). The string selects the methods
and payload types the plugin answers: two plugins with different `protocolVersion` values can
answer entirely different methods while using identical framing. The host rejects a version it
does not support, so a profile can change its payloads under a new version string without this
document changing.

The envelope is open at the top level: a profile MAY declare fields beside the envelope fields of
any message, and the mechanism ignores every field it does not name. Field strictness is set in
schemas and nowhere else, so a profile that wants unknown fields rejected sets
`additionalProperties: false` in the schemas it owns, which are its payload schemas and any
top-level field it declares. The mechanism forbids nothing at the envelope level, because a
mechanism that forbids unknown envelope fields would block the profile extension it exists to
carry.

## 3. Transport and framing

The host starts the plugin from an argv array with pipes on stdin and stdout. The plugin reads
requests from stdin and writes responses to stdout.

- Each message is one UTF-8 encoded JSON value on one line, terminated by `\n`. The host and
  the plugin strip a trailing `\r` before the `\n`.
- Whoever opens a pipe reads it continuously for as long as the connection lives. A pipe holds
  a limited amount of data, so a writer stops once the buffer is full and stays stopped until a
  reader takes the data out. This rule covers every pipe the host opens on the plugin, not only
  stdin and stdout: a host that captures the plugin's stderr without reading it stops the plugin
  partway through a write, and then records `no answer` (§8) for a cause that sits in the host.
- The plugin's stderr carries the plugin's own logging. The mechanism assigns it no meaning and
  defines no protocol over it, so a host may show it, store it, or discard it. A plugin puts no
  message on stderr.
- A blank line or an end-of-file on either pipe ends the connection (§8): the plugin process
  has exited or its stdout has closed, and the host records the event as the end of the
  connection.
- Every request carries an `id` and receives exactly one response, because a host that cannot
  match an answer to a question cannot distinguish a plugin that has stopped from a plugin that
  is still working.
- A receiver that cannot decode an incoming message drops it and keeps the connection open. A
  plugin additionally answers a dropped request with a refusal (§5). A host does not answer a
  dropped response, so a request whose response was dropped ends at its deadline and the host
  records `no answer` (§8). Dropping rather than tearing down keeps one buggy message from
  ending a connection that is still working.

## 4. Requests and responses

The normative envelope schemas are in `spec/schema/prpc-1/`. Each file names itself with a `$id`
under this repository's URL; that `$id` is a namespace, not an address, because every `$ref` in
the three files is local and a validator needs no network access to check a message.

A request is `{"jsonrpc":"2.0","id":<id>,"method":<string>,"params":<value>}`. The host assigns
`id` as an integer with `1 <= id <= 2^53 - 1`; the upper bound keeps the id exact for a plugin
written in JavaScript, whose numbers cannot represent a larger integer. The host echoes `id`
unchanged on the matching response. `id` 0 is reserved: it is the id a plugin uses for a refusal
whose request id it could not determine (§5). The host never assigns 0, so 0 cannot collide with
a real request. If a method takes no parameters, `params` is omitted, not sent as `null`: a
plugin that defaults `null` and a plugin that rejects `null` would otherwise disagree about the
meaning of a request that carries no parameters.

A response is `{"jsonrpc":"2.0","id":<id>,"result":<value>}` or
`{"jsonrpc":"2.0","id":<id>,"error":{"code":<int>,"message":<string>,"data":<value>}}`, with
exactly one of `result` or `error` present. Exactly one is enforced, not advised: a response
carrying both is malformed, because the plugin that built it assigned a result and then failed,
and a host that read the result would report success while discarding the failure. A response
carrying neither is malformed for the same reason in the other direction.

The host discards a response whose `id` matches no request it is still waiting on: a late answer
to a request the host already timed out is a normal consequence of deadlines, and the connection
stays open. The host never discards a refusal whose `id` is 0, because that refusal reports a
fault on the connection rather than an answer to a request.

The profile defines the shape of `params` and `result`. The mechanism requires only that both
are JSON values.

## 5. Errors

| Code | Name | Meaning |
| --- | --- | --- |
| -32700 | parse error | The line received was malformed. The connection continues. |
| -32600 | invalid request | The line decoded as JSON but was not a well-formed request. |
| -32601 | method not found | The method is not in the profile the plugin announced, or the plugin will not answer it. |
| -32000 | plugin error | The plugin failed while answering. |

Codes -32099 through -32001 are reserved for profiles. A profile that needs more error detail
than `PLUGIN_ERROR` puts a typed reason in `error.data` or, better, in the `result` of the
method: a domain failure (the symbol does not exist, the input cannot be built) is returned in
`result`, so a refusal always means a transport or plugin fault and the host never has to decode
a domain failure out of one.

A refusal reports a fault in a request the plugin received. When the plugin cannot determine
which request, because the line was not valid JSON-RPC or carried no usable `id`, the refusal
carries `id` 0. A plugin that can determine the id always echoes it, because a refusal with `id`
0 tells the host that something arrived badly but not what.

## 6. Lifecycle

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
(`handshake.json`) allows them. The host reads only the four core fields, and the mechanism hands
it the whole response object as well, because the fields beside the four belong to the host's own
profile: `handshake` is one request, and a host forced to send it a second time to learn its own
fields pays a process round trip to read what the first answer already carried. An open envelope
lets a profile change its handshake within its own revision, and each profile sets field strictness
in its own schema.

The connection stays open while work remains, and the host may keep several requests waiting
for responses. The host decides between one connection and a pool of plugin processes as host
policy.

## 7. Capabilities

`capabilities` is an array of strings, drawn from the profile's vocabulary, that the plugin
declares it can answer. The array carries no duplicates, because a duplicate reports no
additional ability and a host that counts entries would count one ability twice. The mechanism
requires one property: a declared capability must be answerable — for every capability the
handshake declares, the methods it names must respond to well-formed requests during the
connection. If a capability is absent, the host follows the behavior the profile fixes for that
absence and reports what it skipped. The mechanism does not interpret capability strings; only
the profile defines their meaning.

## 8. Loss

Loss means the host received no answer. The mechanism names five reasons, and a host records
exactly one per lost connection:

| Reason | The host observed |
| --- | --- |
| `spawn` | The process could not be started. |
| `eof` | The plugin closed its stdout. |
| `exited` | The plugin process ended, with its exit status and, on a unix system, its signal. |
| `killed` | The host ended the plugin. |
| `no answer` | The deadline the host set passed with the request still waiting. |

Several reasons can be observed within milliseconds of each other, so the mechanism fixes which
one the host records: the first cause stands. The observations that follow a cause are usually that
cause's consequences, which is why a host records its own `killed` before it sends it and the
`exited` that kill produced never replaces it. The ordering matters in the other direction as well:
a `killed` recorded after an `exited` must not replace it, because a process that had already gone
is not the host's doing. Two reasons name what an observer noticed rather than what happened to the
process — `eof` is what the reader saw on the way down, `no answer` is what the deadline saw — and a
later reason replaces either of those, so `exited` replaces `eof` and `eof` replaces `no answer`.

A refusal is not a loss. A refusal means the connection delivered the request and brought back
an answer that says the work did not happen, and the host handles that per the profile rather
than by restarting or giving up on the process.

The mechanism names these three endings as the one value a host's call returns: a payload, a
refusal, or a loss. A host that invents its own error enum instead has nowhere to put a refusal,
and ends up reporting an answer as a lost plugin. Only the loss says anything about the process, so
the host's restart decision is a single test rather than a reading of the error text.

A profile MAY add loss reasons for the cases where the mechanism delivered an answer the profile
cannot use — a result that fails the profile's own schema, or a request that names a session
object the plugin never announced in this connection. A profile names each reason it adds and
says what the host does with the work that was waiting, because only the profile knows what was
pending.

## 9. Deadlines

A request that triggers work MAY carry `deadlineMs` in its params. The plugin SHOULD finish or
return before the deadline; the host MAY kill the plugin or close the connection after the
deadline passes. The kill mechanism and any grace period are host policy. A request that
carries `deadlineMs` also carries a host-side wait longer than `deadlineMs`, so a plugin that
cuts itself off in time still delivers its answer before the host kills it.

## 10. What a profile must define

A profile is complete if it names:

1. its `protocolVersion` string, as `<namespace>/<revision>`;
2. its method set, with a normative JSON Schema (draft 2020-12) for every request's params and
   every response's result;
3. its capability vocabulary, and the host's behavior if each capability is absent;
4. the mapping from loss (§8) to domain meaning, and any loss reason it adds (§8);
5. its manifest filename (§11);
6. its conformance rules on top of the core rules (§12).

A profile MAY set `additionalProperties: false` on its own payload schemas if it wants strict
field checking. The schemas are the only place field strictness is set: conformance validates
against the schemas, and deserializers enforce exactly what the schemas state.

## 11. Plugin manifest

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
`conformance` is described in §12.

## 12. Conformance

A suite runs each rule against the plugin. Each rule ends in one of three results: pass,
violation (the plugin's answer failed the rule's check, recorded with a note naming the rule),
or not applicable (the rule could not be run). The plugin conforms when every applicable rule
passes. The mechanism fixes the procedure:

1. The plugin's manifest names a conformance **emitter**, a program the suite runs as
   `<argv> --out <dir>`. The emitter writes a fixture project to that directory and writes a
   manifest naming every subject the suite must use. The emitter produces inputs only; the
   suite produces results. Every subject is either named in the fixture manifest or answered
   by the plugin itself, so an empty or incomplete fixture produces a report that shows the
   missing subjects instead of a pass.
2. The suite validates every response against the profile's committed schema **before**
   deserializing it. A plugin that answers with a shape the schema rejects receives a note
   written in schema terms — field name, expected type — rather than an internal error from the
   suite's decoder, so the author sees the rule from the schema they implemented against named
   in the failure.
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

## 13. Versioning

This document is revision `prpc/1`. A mechanism revision changes only framing, envelopes,
lifecycle, or the conformance procedure, and it is published together with the envelope schemas
under a new `prpc-<n>/` directory. A profile revision is independent: `demo/2` may keep using
`prpc/1`. A profile that adds a handshake field or a method needs no mechanism revision,
because the envelope is open (§2) and the method vocabulary is the profile's (§10).

## 14. Open items

The mechanism states nothing about the three items below, and a host may choose its own behavior
today.

1. **Attribution of captured stderr.** A host that runs several plugin processes and captures
   their stderr can merge the lines into one stream, which leaves the operator unable to tell
   which process wrote which line. The mechanism does not require a host to keep them apart.
2. **Reporting captured stderr.** A host may show, store, or discard the stderr it captures (§3).
   A host that captures it and shows nothing pays the cost of reading it and gives the operator
   nothing, so a profile that wants plugin diagnostics in its own reports should say so there.
3. **Message size.** The mechanism needs no maximum message size to frame a message, because a
   message ends at the newline (§3). A profile whose payloads are large should state the size
   that both peers must accept, because a plugin written in another language may cap its line
   reader below that size, pass every rule against small fixtures, and then fail on a real
   payload.
