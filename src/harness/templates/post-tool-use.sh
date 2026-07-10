#!/bin/sh
# PostToolUse hook (matcher: Edit|Write): reindex incrementally, then check
# architecture rules against HEAD. Model-bound content goes to stdout;
# human-facing notes go to stderr.
set -u

# Compat guard: if this binary is older than the templates that generated
# this script, warn (stderr) and fail open -- do nothing, exit 0.
ctx harness compat --require "{{CTX_VERSION}}"
compat_status=$?
if [ "$compat_status" -eq 3 ]; then
    echo "ctx hooks: installed ctx is older than these hook templates (need {{CTX_VERSION}}); skipping post-tool-use action. Update ctx, then rerun 'ctx harness init'." >&2
    exit 0
elif [ "$compat_status" -ne 0 ]; then
    echo "ctx hooks: 'ctx harness compat' failed (status $compat_status); is ctx on PATH? Skipping post-tool-use action." >&2
    exit 0
fi

# Consume the hook's JSON payload on stdin.
cat > /dev/null 2>&1 || true

# Keep the index fresh (incremental; quiet).
ctx index > /dev/null 2>&1 || {
    echo "ctx hooks: 'ctx index' failed; skipping architecture check." >&2
    exit 0
}

# Architecture rules, scoped to the current change set. Findings (exit 1)
# are expected output for the model, not a hook failure.
ctx check --against HEAD --json
exit 0
