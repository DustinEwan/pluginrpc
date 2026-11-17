#!/bin/sh
# The demo profile's reference plugin: a correct plugin, plus one scripted defect at a time.
#
# `DEMO_DEFECT` names the defect to run, and unset means be correct. One program with a selector,
# rather than seven programs, because a defect that drifts from the correct plugin fires a rule by
# accident and the report then blames the plugin for the suite's mistake (spec §12 item 4).

id_of() { printf '%s' "$1" | sed 's/.*"id":[[:space:]]*\([0-9][0-9]*\).*/\1/'; }
answer() { printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$1" "$2"; }
refuse() { printf '{"jsonrpc":"2.0","id":%s,"error":{"code":%s,"message":"%s"}}\n' "$1" "$2" "$3"; }

handshake_result() {
  case "$DEMO_DEFECT" in
    # A different profile: the plugin answers, and the version is not the one under test.
    version) printf '{"protocolVersion":"demo/2","name":"fake","version":"0.1.0","capabilities":[]}' ;;
    # Not an instance of handshake.json: `capabilities` is absent.
    schema) printf '{"protocolVersion":"demo/1","name":"fake","version":"0.1.0"}' ;;
    *) printf '{"protocolVersion":"demo/1","name":"fake","version":"0.1.0","capabilities":["alpha"]}' ;;
  esac
}

# `spawn`: the command runs and answers nothing, so no connection comes up.
if [ "$DEMO_DEFECT" = spawn ]; then
  exit 3
fi

while IFS= read -r line; do
  case "$line" in
    *'"method":"handshake"'*)
      answer "$(id_of "$line")" "$(handshake_result)"
      # `process-alive`: the plugin is a fine plugin right up until it is asked again.
      if [ "$DEMO_DEFECT" = alive ]; then
        exit 0
      fi
      ;;
    *'"method":"ping"'*)
      # `ping-schema`: the answer is not an instance of schema/ping.json.
      if [ "$DEMO_DEFECT" = ping ]; then
        answer "$(id_of "$line")" '{"pong":"yes"}'
      else
        answer "$(id_of "$line")" '{"pong":true}'
      fi
      ;;
    *'"method":'*)
      # `unknown-method`: a method outside the profile is answered instead of refused.
      if [ "$DEMO_DEFECT" = unknown ]; then
        answer "$(id_of "$line")" '{"note":"answered anyway"}'
      else
        refuse "$(id_of "$line")" -32601 "no such method"
      fi
      ;;
    *)
      # `framing-intact`: a line that is not JSON is swallowed instead of refused.
      if [ "$DEMO_DEFECT" != framing ]; then
        refuse 0 -32700 "the line is not JSON"
      fi
      ;;
  esac
done
