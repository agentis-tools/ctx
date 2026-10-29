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
# The compact plain report, cut at line boundaries to stay under Claude Code's
# 10,000-character additionalContext limit (beyond it the model only sees a
# short preview).
result=$(ctx check --against HEAD --format plain)
check_status=$?
if [ "$check_status" -eq 1 ]; then
    # Byte-wise locale: every transform below is ASCII-only, and BSD tools
    # reject invalid UTF-8 under a UTF-8 locale.
    LC_ALL=C
    export LC_ALL
    # JSON-escape portably (POSIX tr/sed/awk; no GNU-only escapes): tabs and
    # CRs become spaces, other control characters are dropped, backslashes and
    # quotes are escaped. awk then joins lines with a literal \n, keeping whole
    # lines while the escaped text stays under 9,000 characters.
    escaped=$(printf '%s\n' "$result" \
        | tr '\t\r' '  ' \
        | tr -d '\000-\010\013\014\016-\037' \
        | sed 's/\\/\\\\/g; s/"/\\"/g' \
        | awk -v max=9000 'BEGIN { ORS = "\\n" }
            { if (n + length($0) + 2 <= max) { print; n += length($0) + 2 } else dropped++ }
            END { if (dropped) print "... " dropped " more line(s) not shown; run: ctx check --against HEAD" }')
    printf '{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"ctx check found architecture rule violations in the current change set (ctx check --against HEAD):\\n%s"}}\n' "$escaped"
elif [ "$check_status" -ne 0 ]; then
    echo "ctx hooks: 'ctx check' failed (status $check_status); no architecture feedback for this edit." >&2
fi
exit 0
