# The framing helpers

<doc-section name="parse_request">

## What a line the mechanism cannot use becomes

`parse_request` refuses a line that is not JSON <doc-claim name="refuses-a-malformed-line"/>, and the
reason it returns quotes the line, so the `-32700` frame a plugin sends names what actually arrived
instead of only that something was wrong. A line that carries the envelope fields is accepted
<doc-claim name="accepts-a-valid-request"/>, so the plugin server and the host dispatch on the same
decode rather than on two similar ones.

<doc-section name="read_line">

## Where a connection ends

`read_line` stops at a blank line and reports loss <doc-claim name="treats-a-blank-line-as-loss"/>. A
peer that sends a blank line therefore ends the connection, which the host turns into a loss it can
report, rather than shifting every later message by one and pairing answers with the wrong requests.

<doc-section name="framing">

## What a refusal names

Every refusal the framing helpers return names the line it refused
<doc-claim name="refusals-name-their-line"/>, and `evidence/refusals.txt` is the artefact that records
the ones the scenario harness produced.

The harness answers these observables, and a scenario may assert on any of them:

<doc-each source="observables">
- `<doc-value field="name"/>` of type `<doc-value field="type"/>`
