#!/bin/sh
# The demo profile's suite program (spec §12 item 6): one verdict per rule the profile declares.
#
# It reads only what the mechanism put in the directory it was given, which is what that directory
# is for: a schema path the profile spec names has to be reachable there, or a suite that validates
# an answer against it would be reading a checkout it was never handed (§10 item 2, §12 item 6).
#
# The plugin it drives is the same reference plugin the core rules run, with the same one selector,
# so running this suite with `DEMO_DEFECT=ping` is how the profile's own scripted defect fires.

set -eu

out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out)
      out="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

[ -n "$out" ] || {
  echo "suite.sh: no --out" >&2
  exit 2
}
for committed in profile.json manifest.json schema/ping.json; do
  [ -f "$out/$committed" ] || {
    echo "suite.sh: $out has no $committed, which the profile spec names" >&2
    exit 2
  }
done

answer=$(
  printf '{"jsonrpc":"2.0","id":1,"method":"handshake"}\n{"jsonrpc":"2.0","id":2,"method":"ping"}\n' |
    /bin/sh tests/fixtures/demo.sh | sed -n 's/.*"id":2,"result":\(.*\)/\1/p'
)
[ -n "$answer" ] || {
  echo "suite.sh: the reference plugin answered no ping" >&2
  exit 2
}

# One assertion of the committed schema is checked here, in the shape a shell can check it: the
# result's `pong` is the value the schema states. The suite is a program, so a profile whose schema
# states more checks more (§12 item 6).
want=$(sed -n 's/.*"const":[[:space:]]*\([a-z]*\).*/\1/p' "$out/schema/ping.json")
case "$answer" in
  *\"pong\":$want*)
    ping='{"rule":"ping-schema","outcome":"pass","note":"pong is true, as schema/ping.json states"}'
    ;;
  *)
    # The answer goes into the note as written, apart from its quotes, which the note's own JSON
    # cannot carry.
    safe=$(printf '%s' "$answer" | tr -d '"')
    note="pong is not $want, which schema/ping.json requires; the answer was $safe"
    ping="{\"rule\":\"ping-schema\",\"outcome\":\"violation\",\"note\":\"$note\"}"
    ;;
esac

# `prepare-after-ping` is not attempted, for the reason the profile spec states in its `unverified`
# sentence, and not for a reason the suite invented (spec §12 item 3).
prepare='{"rule":"prepare-after-ping","outcome":"not-applicable","reason":"the reference plugin cannot answer prepare with a foreign id"}'

printf '[%s,\n %s]\n' "$ping" "$prepare" >"$out/verdicts.json"
