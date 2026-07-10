#!/bin/sh
# Stop hook: print a quality scorecard for the session's changes.
# Non-blocking by default: the scorecard goes to stdout, findings go to
# stderr, and the hook exits 0. Set CTX_GATE_BLOCKING=1 to turn gate
# failures into a blocking stop (exit 2).
set -u

# Compat guard: if this binary is older than the templates that generated
# this script, warn (stderr) and fail open -- do nothing, exit 0.
ctx harness compat --require "{{CTX_VERSION}}"
compat_status=$?
if [ "$compat_status" -eq 3 ]; then
    echo "ctx hooks: installed ctx is older than these hook templates (need {{CTX_VERSION}}); skipping stop action. Update ctx, then rerun 'ctx harness init'." >&2
    exit 0
elif [ "$compat_status" -ne 0 ]; then
    echo "ctx hooks: 'ctx harness compat' failed (status $compat_status); is ctx on PATH? Skipping stop action." >&2
    exit 0
fi

# Consume the hook's JSON payload on stdin.
cat > /dev/null 2>&1 || true

# Quality scorecard vs the default branch (stdout). Exit 1 means a gate
# condition fired.
#
# Environment knobs:
#   CTX_GATE_BLOCKING=1 -- turn gate failures into exit 2, Claude Code's
#                          blocking-stop mechanism (the session keeps going
#                          until the findings are addressed).
#   CTX_GATE_LOG        -- consumed by `ctx score` itself (not this script):
#                          when set, each gate evaluation is appended to a
#                          local JSONL log (default .ctx/gate-log.jsonl).
ctx score --against {{DEFAULT_BRANCH}} --fail-on "check_violations>0,new_duplication>0"
score_status=$?
if [ "$score_status" -eq 1 ]; then
    if [ "${CTX_GATE_BLOCKING:-0}" = "1" ]; then
        echo "ctx hooks: quality gates failed (blocking mode); fix the failed conditions in the scorecard above before stopping." >&2
        exit 2
    fi
    echo "ctx hooks: quality gates reported findings (non-blocking); see the scorecard above." >&2
fi
# Any other non-zero status is an operational error: fail open.
exit 0
