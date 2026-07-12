#!/bin/sh
set -u
ctx harness compat --require "{{CTX_VERSION}}" >/dev/null 2>&1 || exit 0
cat >/dev/null 2>&1 || true
ctx index >/dev/null 2>&1 || exit 0
result=$(ctx check --against HEAD --json 2>&1)
status=$?
escaped=$(printf '%s' "$result" | sed 's/\\/\\\\/g; s/"/\\"/g' | awk 'BEGIN{ORS="\\n"}{print}')
if [ "$status" -eq 1 ]; then
  printf '{"decision":"block","reason":"ctx architecture checks reported findings.","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"%s"}}\n' "$escaped"
elif [ "$status" -ne 0 ]; then
  printf '{"systemMessage":"ctx architecture check failed operationally; run ctx check manually."}\n'
fi
exit 0
