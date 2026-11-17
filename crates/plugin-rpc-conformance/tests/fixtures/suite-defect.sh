#!/bin/sh
# Runs the demo profile's suite program with its declared scripted defect in place (§12 item 4).
# The selector goes to the reference plugin through the environment, and this wrapper sets it for
# one run rather than for whatever else the mechanism happens to be doing in the same process.

DEMO_DEFECT=ping exec /bin/sh tests/fixtures/suite.sh "$@"
