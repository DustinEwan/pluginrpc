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
| `crates/plugin-rpc/` | Rust reference implementation of the messages, the loss reasons, and the answers; `src/process.rs` adds the transport for a plugin started as a child process |

## Reference implementation

The `plugin-rpc` crate implements the envelopes, the framing, and the vocabulary of `prpc/1`.
Its 33 tests assert the wire contract exactly as a plugin written in any other language must
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
otherwise. The 15 tests in `tests/process.rs` start real child processes, so they cover a plugin
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
