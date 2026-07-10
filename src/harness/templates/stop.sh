#!/bin/sh
# Stop hook: print a quality scorecard for the session's changes.
# Non-blocking in v1: the scorecard goes to stdout, findings go to stderr,
# and the hook always exits 0.
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
# condition fired; report it (stderr) but never block the session.
ctx score --against {{DEFAULT_BRANCH}} --fail-on "check_violations>0,new_duplication>0" || {
    echo "ctx hooks: quality gates reported findings (non-blocking); see the scorecard above." >&2
}
exit 0
