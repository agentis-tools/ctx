#!/bin/sh
# SessionStart hook: give the model a token-budgeted map of the codebase.
# Model-bound content goes to stdout; human-facing notes go to stderr.
set -u

# Compat guard: if this binary is older than the templates that generated
# this script, warn (stderr) and fail open -- do nothing, exit 0.
ctx harness compat --require "{{CTX_VERSION}}"
compat_status=$?
if [ "$compat_status" -eq 3 ]; then
    echo "ctx hooks: installed ctx is older than these hook templates (need {{CTX_VERSION}}); skipping session-start action. Update ctx, then rerun 'ctx harness init'." >&2
    exit 0
elif [ "$compat_status" -ne 0 ]; then
    echo "ctx hooks: 'ctx harness compat' failed (status $compat_status); is ctx on PATH? Skipping session-start action." >&2
    exit 0
fi

# Consume the hook's JSON payload on stdin.
cat > /dev/null 2>&1 || true

# Codebase overview for the model (stdout).
ctx map --budget 2000 || {
    echo "ctx hooks: 'ctx map' failed; run 'ctx index' to (re)build the index." >&2
}
exit 0
