#!/bin/sh
# PostToolUse hook (matcher: Edit|Write): reindex incrementally, then check
# architecture rules against HEAD. Claude Code shows a PostToolUse hook's plain
# stdout only in its debug log, never to the model, so findings are emitted as
# hookSpecificOutput.additionalContext JSON (shown to the model next to the
# tool result). A clean check prints nothing. Human-facing notes go to stderr.
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

# Architecture rules, scoped to the current change set. Findings (exit 1) are
# expected feedback for the model, not a hook failure.
result=$(ctx check --against HEAD --json)
check_status=$?
if [ "$check_status" -eq 1 ]; then
    # JSON-escape portably (POSIX tr/sed/awk; no GNU-only escapes): tabs and
    # CRs become spaces, other control characters are dropped, backslashes and
    # quotes are escaped, and lines are joined with a literal \n.
    escaped=$(printf '%s' "$result" \
        | tr '\t\r' '  ' \
        | tr -d '\000-\010\013\014\016-\037' \
        | sed 's/\\/\\\\/g; s/"/\\"/g' \
        | awk 'BEGIN { ORS = "\\n" } { print }')
    printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"ctx check found architecture rule violations in the current change set (ctx check --against HEAD --json):\\n%s"}}\n' "$escaped"
elif [ "$check_status" -ne 0 ]; then
    echo "ctx hooks: 'ctx check' failed (status $check_status); no architecture feedback for this edit." >&2
fi
exit 0
