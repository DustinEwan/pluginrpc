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

`prpc/1` is the current revision. Every rule in the specification is normative as written. The
mechanism describes one connection between one host and one plugin process: a host that runs
several plugins decides pooling, restarts, and how each process's output reaches its operator as
its own policy, and the specification sets no maximum message size, leaving a profile whose
payloads are large to state the size both peers must accept.

## Layout

| Path | What it is |
| --- | --- |
| `spec/spec.md` | The normative mechanism specification, revision `prpc/1`, opening with the glossary |
| `spec/schema/prpc-1/` | The normative envelope schemas, and `profile.json`, the meta-schema for the one file a profile publishes (JSON Schema draft 2020-12) |
| `spec/core-rules.json` | The conformance rules every profile inherits |
| `crates/plugin-rpc/` | The Rust reference implementation of the whole mechanism: the messages, the loss reasons, and the answers; `src/process.rs` the transport for a plugin started as a child process; `src/conformance/` the profile-spec reader, the six core rules, the report shape, and the `prpc-core` command |

## Reference implementation

The `plugin-rpc` crate implements the envelopes, the framing, and the vocabulary of `prpc/1`, and
its `conformance` module implements §11 and §12. Its 52 tests assert the wire contract exactly as
a plugin written in any other language must
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

`Call<T>` is what one call ended as: a payload, a `Refusal`, or a `Lost`. A refusal is an answer,
so only `Call::Lost` says anything about the process, and `Call::answered()` is the single test a
host makes before restarting. A result the profile's own type cannot read is a loss carrying the
reason `UNUSABLE_RESULT`, not a refusal. `Loss` holds the five reasons a host receives no answer,
plus `Loss::Profile` for a case the mechanism cannot name. `Loss::resolve` keeps the first cause
when two arrive together and lets a later reason replace only `eof` or `no answer`, which is the
ordering both a restart and a report depend on.

`plugin_rpc::process` is the one part of the crate that is not medium-neutral. `Plugin::start`
takes a `PluginConfig` (the command, the environment, the directory, and the two `Timeouts`),
starts the process, and sends the handshake, because §6 makes the handshake the first request and
a host that forgets that rule can call a plugin whose profile it never read. `Plugin::start` hands
back the whole handshake answer in `Handshake::result`, so the fields a profile adds beside the
four core ones reach the host without a second `handshake` request.
`Plugin::request` and `Plugin::call` return `Call<T>`. The handle owns the process: it drains
stderr, which §3 requires, because a pipe nobody reads stops the plugin, it records
`Loss::Killed` before it asks for a kill so an in-flight call is attributed to the host rather
than to the exit status that kill produced, and it ends the plugin on drop unless `detach` said
otherwise. The 16 tests in `tests/process.rs` start real child processes, so they cover a plugin
that refuses, stays silent, writes to stderr, answers with an `id` 0 refusal, carries profile
fields in the handshake, and dies or is killed while a call is in flight. A call waiting on a dead
plugin is woken when the process goes, so a restart policy that counts consecutive losses does not
fall one full timeout behind per restart.

The two timings the transport itself needs are mechanism constants rather than configuration: how
long a call waits for the process's exit status before it stops reporting a closed pipe as the
cause, and how long `kill` blocks for the process to be gone. §9's deadline pair comes from
`Timeouts::plugin_deadline` when the host's configuration names its own wait, and from
`Timeouts::host_wait` when it names the plugin's budget, so the rule is written once and neither
direction silently changes what the host's own option means.

## Conformance

A profile publishes one file, the **profile spec**, naming the mechanism revision it is written
for and the six things §10 requires of it: its `protocolVersion`, its manifest filename, its method
set with the path to each request and response schema, its capability vocabulary with what the host
does when each is absent, its loss mapping, and its own rules with the scripted defect that proves
each one or a sentence saying none exists. Its shape is `spec/schema/prpc-1/profile.json`, and
`plugin_rpc::conformance` reads it.

The six rules in `spec/core-rules.json` name no method of any profile, so the mechanism runs them
for every profile from that file and the plugin's manifest alone, with no profile code:

```
cargo run -p plugin-rpc --bin prpc-core -- <profile.json> <manifest.json> [--suite]
```

It reports `pass`, `violation`, or `not-applicable` per rule, and a rule the run could not attempt
is counted not applicable with the reason that stopped it, never as a pass. Its own tests carry one
scripted defect per rule, so each of the six is shown to fire on the failure it targets (§12 item
4) — the discipline the audit found missing from the two profiles this mechanism came out of.

A profile's own rules are a **suite program**, run as `<argv> --out <dir>` and writing
`verdicts.json`, for the same reason the fixture emitter already is a program: those rules compare
one response against another, fold across ten invocations, and derive inputs by hashing, and the
mechanism is not a programming language. A suite in any language produces the same report because
§12 fixes the order, the three results, the not-applicable accounting, and the rule that a missing
verdict is a suite error rather than a pass.

`prpc-core --suite` is the one command that runs both halves: it writes the spec, the manifest,
and the schemas the spec names into a directory, runs the profile's suite program there, reads
`verdicts.json` back, and prints the profile's verdicts after the six core ones. A suite program
that fails to write the file is reported as a suite error, never as a plugin violation (§12 item
5).

Before the suite program starts, the mechanism writes `profile.json`, `manifest.json`, and every
schema file the profile spec names into `<dir>`, each at the path the spec gives. The spec's paths
are resolved beside the committed spec (§10 item 2), and the loader refuses a spec that names a
schema it cannot open, so a suite validates an answer against the file its author read rather than
against a path in a checkout the suite was never handed.

`tests/suite.rs` drives that contract against the demo profile: the suite program opens
`schema/ping.json` out of the directory it was given, the scripted defect the spec names is run by
the mechanism and makes `ping-schema` report a violation and no other rule, the same command with
the defect removed leaves that rule unverified, and a suite that reports one of the profile's two
rules fails as a suite error rather than passing the rule it skipped.

The profile states, for each of its own rules, either the command that starts the plugin with the
defect for that rule in place and the exact set of rules it must make fail, or a sentence saying no
such defect exists. `--suite` runs the first kind instead of trusting it, so the field is a claim
anybody can check rather than a sentence about a run nobody did.

The defect run is what makes §12 item 4 mean anything to a reader of a report: a rule whose defect
left the rules it names still passing keeps the `unverified` mark a profile gets for admitting it
has no defect, and the run prints which rules those were.

A defect names the plugin it was written against. When the plugin under test is a different
program — a harness checkout has no copy of the engine's test plugin — the runner says so once and
leaves the rule's mark as the profile states it, because that proof belongs to the profile's own
run and is not this run's to take back.
