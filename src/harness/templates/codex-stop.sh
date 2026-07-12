#!/bin/sh
set -u
ctx harness compat --require "{{CTX_VERSION}}" >/dev/null 2>&1 || { cat >/dev/null 2>&1 || true; exit 0; }
cat >/dev/null 2>&1 || true
result=$(ctx score --against {{DEFAULT_BRANCH}} --fail-on "check_violations>0,new_duplication>0" 2>&1)
status=$?
escaped=$(printf '%s' "$result" | sed 's/\\/\\\\/g; s/"/\\"/g' | awk 'BEGIN{ORS="\\n"}{print}')
if [ "$status" -eq 1 ]; then
  if [ "${CTX_GATE_BLOCKING:-0}" = "1" ]; then
    printf '%s\n' "$result" >&2
    echo "ctx quality gates failed; fix the scorecard findings before stopping." >&2
    exit 2
  fi
  printf '{"systemMessage":"ctx quality gates reported non-blocking findings.","hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"%s"}}\n' "$escaped"
elif [ "$status" -eq 0 ]; then
  printf '{"continue":true,"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"%s"}}\n' "$escaped"
else
  printf '{"continue":true,"systemMessage":"ctx score failed operationally; run ctx score manually."}\n'
fi
exit 0
