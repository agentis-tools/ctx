#!/bin/sh
# rework-rate.sh -- N-day rework rate for a git repository.
#
# Purpose
#   Longitudinal-study proxy for "the change worked but wasn't right": for
#   each qualifying commit C, measure what fraction of the lines C added
#   were modified or deleted again within a fixed window (default 30 days).
#
# Definition
#   added(C)     = sum of added-line counts from `git diff --numstat C^ C`
#                  (binary entries, where the count is `-`, are skipped).
#                  For a root commit (no parent) the diff base is the git
#                  empty-tree object.
#   boundary(C)  = the last first-parent commit whose committer date is
#                  <= date(C) + window. Window arithmetic is done in pure
#                  awk/shell on `%ct` epoch seconds, so no `date` binary is
#                  needed and there is no GNU vs BSD `date` portability
#                  issue (BSD date uses `-v+30d`, GNU date uses `-d`; we
#                  avoid both).
#   surviving(C) = for each file F touched by C, if F still exists at
#                  boundary(C): the number of lines that
#                  `git blame -w --line-porcelain boundary(C) -- F`
#                  attributes to C. Files deleted by boundary(C)
#                  contribute 0.
#   rework(C)    = (added(C) - surviving(C)) / added(C)
#
# Commit set
#   First-parent, non-merge commits of HEAD in [--since, --until].
#   Excluded from the set:
#     * commits whose committer date is younger than window-days before
#       the repository's NEWEST first-parent commit date -- their window
#       is incomplete. The reference point is the newest commit's date,
#       not the wall clock, so results are reproducible on a static clone.
#     * formatting-only commits: commits for which
#       `git diff -w --ignore-blank-lines C^ C` produces no output.
#     * commits with added(C) == 0 (no denominator).
#
# Limitations
#   * Renames are not followed: a file renamed inside the window is seen
#     as deleted, so its lines count as reworked.
#   * Only the first-parent chain is measured; work merged in via side
#     branches is attributed to nothing (merge commits are skipped).
#   * Lines whose only later change is whitespace are still counted as
#     surviving (blame runs with -w), but a mixed commit's own
#     whitespace-only line edits are counted in added(C) (plain
#     --numstat) while blame -w attributes those lines to an earlier
#     commit, slightly overstating rework for such commits.
#   * Paths that git quotes in --numstat output (embedded tabs, quotes,
#     control characters) may fail the existence check and be treated as
#     deleted. `core.quotePath` is disabled so plain non-ASCII names work.
#
# Output
#   TSV on stdout: a header line
#       commit<TAB>added<TAB>surviving<TAB>rework_fraction
#   then one row per qualifying commit (fraction to 4 decimals), then a
#   final aggregate line:
#       # aggregate<TAB><total_added><TAB><total_surviving><TAB><fraction>
#   An empty commit set (e.g. an empty repository, or every commit
#   excluded) still prints the header plus a zero aggregate and exits 0.
#   Errors and usage go to stderr; exit 1 on bad usage, 0 otherwise.
#
# Usage
#   rework-rate.sh [--window-days N] [--since DATE] [--until DATE] [REPO_DIR]
#
#   REPO_DIR defaults to `.`. The script never writes inside the target
#   repository; every git command runs via `git -C "$REPO_DIR"`.

set -eu

usage() {
    echo "usage: rework-rate.sh [--window-days N] [--since DATE] [--until DATE] [REPO_DIR]" >&2
}

WINDOW_DAYS=30
SINCE=""
UNTIL=""
REPO_DIR=""

while [ $# -gt 0 ]; do
    case "$1" in
        --window-days)
            if [ $# -lt 2 ]; then usage; exit 1; fi
            WINDOW_DAYS=$2
            shift 2
            ;;
        --since)
            if [ $# -lt 2 ]; then usage; exit 1; fi
            SINCE=$2
            shift 2
            ;;
        --until)
            if [ $# -lt 2 ]; then usage; exit 1; fi
            UNTIL=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            echo "rework-rate.sh: unknown option: $1" >&2
            usage
            exit 1
            ;;
        *)
            if [ -n "$REPO_DIR" ]; then
                echo "rework-rate.sh: too many arguments" >&2
                usage
                exit 1
            fi
            REPO_DIR=$1
            shift
            ;;
    esac
done

if [ -z "$REPO_DIR" ]; then
    REPO_DIR=.
fi

case "$WINDOW_DAYS" in
    ''|*[!0-9]*)
        echo "rework-rate.sh: --window-days must be a non-negative integer" >&2
        exit 1
        ;;
esac

if ! git -C "$REPO_DIR" rev-parse --git-dir >/dev/null 2>&1; then
    echo "rework-rate.sh: not a git repository: $REPO_DIR" >&2
    exit 1
fi

# Every git invocation goes through this wrapper: read-only, repo-targeted,
# and with path quoting disabled so plain UTF-8 paths round-trip.
g() {
    git -C "$REPO_DIR" -c core.quotePath=false "$@"
}

printf 'commit\tadded\tsurviving\trework_fraction\n'

TOTAL_ADDED=0
TOTAL_SURVIVING=0

emit_aggregate() {
    AGG=$(awk -v a="$TOTAL_ADDED" -v s="$TOTAL_SURVIVING" \
        'BEGIN { if (a > 0) printf "%.4f", (a - s) / a; else printf "%.4f", 0 }')
    printf '# aggregate\t%s\t%s\t%s\n' "$TOTAL_ADDED" "$TOTAL_SURVIVING" "$AGG"
}

# Empty repository: header plus zero aggregate.
if ! g rev-parse -q --verify 'HEAD^{commit}' >/dev/null 2>&1; then
    emit_aggregate
    exit 0
fi

EMPTY_TREE=$(g hash-object -t tree /dev/null)
NEWEST_CT=$(g log -1 --first-parent --format=%ct HEAD)
WINDOW_SECS=$((WINDOW_DAYS * 86400))
CUTOFF=$((NEWEST_CT - WINDOW_SECS))

# Build the rev-list argument vector (avoid `[ -n ] &&` which trips set -e).
set --
if [ -n "$SINCE" ]; then
    set -- "$@" --since="$SINCE"
fi
if [ -n "$UNTIL" ]; then
    set -- "$@" --until="$UNTIL"
fi

# Capture into a variable so a rev-list failure aborts under set -e.
COMMITS=$(g rev-list --first-parent --no-merges "$@" HEAD)

for C in $COMMITS; do
    CT=$(g show -s --format=%ct "$C")

    # Incomplete window: committer date younger than window-days before the
    # newest first-parent commit date.
    if [ "$CT" -gt "$CUTOFF" ]; then
        continue
    fi

    # Root commits have no parent; diff against the empty tree instead.
    if g rev-parse -q --verify "$C^" >/dev/null 2>&1; then
        BASE="$C^"
    else
        BASE=$EMPTY_TREE
    fi

    # Skip formatting-only commits (whitespace / blank-line changes only).
    if g diff --quiet -w --ignore-blank-lines "$BASE" "$C"; then
        continue
    fi

    ADDED=$(g diff --no-renames --numstat "$BASE" "$C" \
        | awk -F'\t' '$1 != "-" { a += $1 } END { printf "%d", a }')
    if [ "$ADDED" -eq 0 ]; then
        continue
    fi

    # Boundary commit: last first-parent commit dated <= date(C) + window.
    # Pure epoch arithmetic; git parses "@<epoch>" natively.
    END_TS=$((CT + WINDOW_SECS))
    B=$(g rev-list -1 --first-parent --before="@$END_TS" HEAD)

    SURVIVING=$(
        g diff --no-renames --numstat "$BASE" "$C" \
            | awk -F'\t' '$1 != "-" && $2 != "-"' \
            | cut -f3- \
            | while IFS= read -r F; do
                if g cat-file -e "$B:$F" 2>/dev/null; then
                    g blame -w --line-porcelain "$B" -- "$F" \
                        | awk -v c="$C" '$1 == c { n++ } END { printf "%d\n", n + 0 }'
                fi
            done \
            | awk '{ s += $1 } END { printf "%d", s + 0 }'
    )

    FRACTION=$(awk -v a="$ADDED" -v s="$SURVIVING" \
        'BEGIN { printf "%.4f", (a - s) / a }')
    printf '%s\t%s\t%s\t%s\n' "$C" "$ADDED" "$SURVIVING" "$FRACTION"

    TOTAL_ADDED=$((TOTAL_ADDED + ADDED))
    TOTAL_SURVIVING=$((TOTAL_SURVIVING + SURVIVING))
done

emit_aggregate
exit 0
