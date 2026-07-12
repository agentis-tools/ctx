#!/bin/sh
set -u
ctx harness compat --require "{{CTX_VERSION}}" >/dev/null 2>&1 || exit 0
cat >/dev/null 2>&1 || true
ctx map --budget 2000 || {
  echo "ctx hooks: ctx map failed; run ctx index to rebuild the index." >&2
  exit 0
}
