---
id: untrusted-ci
title: Run ctx safely in CI on untrusted code
sidebar_position: 16
---

# Run ctx safely in CI on untrusted code

A pull request from a fork is untrusted code. It may change build scripts, configuration, checked-in
executables, or any script that a workflow invokes. The job that checks out and analyzes that code
must not also hold permission to comment, label, merge, or access repository secrets.

This recipe separates unprivileged analysis from privileged publication. It was verified against
ctx's own pinned `pull_request` and `workflow_run` workflows on 2026-07-14. The pattern is specific
to GitHub Actions, but the trust boundary applies to every CI system.

## Quickest version

1. Run ctx in a `pull_request` workflow with `contents: read`, no secrets, and checkout credentials
   disabled.
2. Upload bounded JSON plus the forge-provided PR number, base SHA, head SHA, repository, and run ID.
3. Publish from a separate `workflow_run` workflow that checks out only the trusted default branch.
4. Validate artifact size, schema, command, version, identity metadata, and the current PR head
   immediately before writing.

Do not replace this split with `pull_request_target` plus a checkout of the pull-request head. That
combination gives untrusted code a privileged execution context.

## 1. Draw the trust boundary

| Stage | May execute PR code? | May hold write permission? |
|---|---:|---:|
| Analyze | yes | no |
| Transfer artifact | carries untrusted data | no |
| Validate and render | no | yes, narrowly scoped |

Treat uploaded JSON as hostile input. Separating workflows prevents direct credential theft, but it
does not make an artifact trustworthy.

## 2. Analyze with read-only permissions

The analysis workflow should declare its permissions at workflow scope:

```yaml
on:
  pull_request:
    types: [opened, reopened, synchronize, ready_for_review]

permissions:
  contents: read

jobs:
  analyze:
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@<full-commit-sha>
        with:
          ref: ${{ github.event.pull_request.head.sha }}
          fetch-depth: 0
          persist-credentials: false
```

Pin third-party actions by full commit SHA. Install a fixed ctx release outside the checkout and
verify its archive against a separately published checksum before executing it. Do not run an
installer or a repository script supplied by the pull request.

Use base and head SHAs from the event, not files or environment output controlled by the checkout:

```bash
git fetch --no-tags origin "${BASE_SHA}"
ctx index
ctx score --against "${BASE_SHA}" --json > analysis/score.json

status=0
ctx check --against "${BASE_SHA}" --json > analysis/check.json || status=$?
if test "${status}" -gt 1; then
  exit "${status}"
fi
```

Code `1` is valid analysis with findings; code `2` is an operational failure. The shared
[exit-code model](concepts#exit-codes-are-part-of-the-integration-api) must survive the shell
wrapper.

## 3. Bind the artifact to one run

Add metadata that the trusted workflow can compare with its own event:

```json
{
  "schema_version": 1,
  "pr_number": 123,
  "head_sha": "...",
  "base_sha": "...",
  "repository": "owner/repository",
  "run_id": 456
}
```

Give the artifact a run-specific name, set a short retention period, and fail when expected files
are missing. Never let the publisher choose “the latest artifact” by an attacker-controlled name.

## 4. Publish only trusted code

The publisher runs after the named analysis workflow completes:

```yaml
on:
  workflow_run:
    workflows: ["ctx PR analysis"]
    types: [completed]

permissions:
  actions: read
  contents: read
  pull-requests: write
```

Check out the repository's default branch, not the analyzed head. The renderer and validation code
must come from that trusted checkout. Download only the artifact belonging to
`github.event.workflow_run.id`.

Before parsing and rendering:

- reject symlinks, unexpected files, and files over a small size limit;
- require the expected ctx version and command name in every JSON envelope;
- validate required object and array shapes before using values;
- compare repository, run ID, PR number, base SHA, and head SHA with the trusted event;
- render text as data rather than evaluating it as HTML, shell, JavaScript, or expressions.

## 5. Reject stale results twice

A new commit can arrive while the trusted workflow is rendering. Fetch the current pull request and
require both `state == open` and `current head SHA == analyzed head SHA`:

1. before building the comment;
2. immediately before the API write.

When updating a sticky comment, include the analysis run ID and refuse to overwrite a comment from
a newer run. Use a concurrency group per pull request and cancel superseded analysis runs to reduce
races; the identity checks remain necessary even with concurrency configured.

## 6. Report failures without executing the artifact

If analysis fails, the trusted workflow can publish a short diagnostic using only trusted
`workflow_run` metadata and a link to the run. It should still verify that the pull request is open
and its head SHA matches. Do not download or render a partial artifact from a failed run.

## What worked, and what did not

| Technique | Verified use in ctx | Boundary or limitation |
|---|---|---|
| Read-only `pull_request` analysis | Executes ctx against fork and same-repository heads | Untrusted output remains untrusted data |
| `persist-credentials: false` | Prevents checkout credentials remaining in git config | It does not remove every possible ambient capability |
| Pinned, checksum-verified ctx | Keeps the analyzer independent of PR scripts | Release and checksum provenance still require governance |
| Run-bound metadata and artifact name | Binds results to one PR head and workflow run | Every field must be checked by the trusted side |
| Trusted default-branch renderer | Keeps publication code outside attacker control | Renderer bugs can still mishandle hostile strings |
| Double head-SHA validation | Rejects results superseded during rendering | It cannot make an old result current; it only refuses the write |

This pattern protects the write boundary. It does not prove the analyzed code is safe, the ctx
result is semantically correct, or the repository's branch rules require the workflow.

## Give the workflow to an agent

```text
Design a fork-safe ctx pull-request integration. Put all execution of pull-request code in a
read-only pull_request workflow with no secrets, disabled checkout credentials, pinned actions, and
a pinned checksum-verified ctx binary. Upload bounded JSON plus trusted event identity metadata.
Publish only from a separate workflow_run job that checks out the default branch, treats every
artifact byte as hostile, validates schema, command, version, run, repository, PR, base, and head,
and confirms the current PR head immediately before writing. Never combine pull_request_target
privileges with execution of the pull-request checkout.
```

## Next steps

- Use [pull-request governance](pr-governance) to decide what the analysis should report or block.
- Use [gate recovery](gate-recovery) when the analysis produces a disputed finding or an
  operational failure.
- Review the general [CI integration guide](../integrations/ci-cd).
