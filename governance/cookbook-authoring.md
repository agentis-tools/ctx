# Cookbook authoring contract

The public cookbook teaches both engineers and coding agents how to use ctx. A recipe is therefore
an executable evidence workflow, not a command showcase or an implicit product claim.

## Required structure

Every recipe under `docs/website/docs/cookbook/` except the index and shared-concepts page must
contain, in this order where practical:

1. **Problem and boundary** — the engineering question, when the workflow applies, and what it
   cannot establish.
2. **Quickest version** — the smallest safe command sequence, with a link into the deeper workflow.
3. **Evidence loop** — deterministic collection, source verification, interpretation, and a
   re-measurement or validation step.
4. **What worked, and what did not** — verified behavior and observed limitations. Use a table when
   several techniques are compared.
5. **Give the workflow to an agent** — a self-contained prompt that preserves the recipe's
   uncertainty and stopping conditions.
6. **Related concepts or next steps** — links rather than independent redefinitions of shared
   semantics.

## Evidence standard

- Exercise the workflow against a real repository, fixture, or isolated regression before
  publishing it. Do not infer command behavior solely from help text.
- Treat graph edges, rankings, and metrics as evidence to inspect, not conclusions.
- State false positives, missing edges, operational failures, and unsupported configurations.
- When embedding measured values, name the repository commit or release and measurement date. Say
  that the values are illustrative, not a stable product contract.
- Prefer CI that validates commands, output schemas, links, and documented assumptions. Do not
  automatically rewrite measured prose: changing evidence deserves review.
- Keep secrets, maintainer procedure, repository settings, and internal policy out of public
  recipes. Link to public integration guidance where appropriate.

## Shared concepts

Define exit codes, Report/Review/Block enforcement, comparison-base selection, and indexed totals
once in the public cookbook concepts page. A recipe may summarize the relevant rule, but must link
to the canonical explanation instead of creating a competing definition.

## Machine-readable publication

The Markdown recipe is authoritative. `docs/website/static/llms.txt`, the standalone ctx skill, and
generated plugin copies are routing views over that source. Update the canonical harness skill
template and regenerate derived plugin trees; never hand-edit generated copies.
