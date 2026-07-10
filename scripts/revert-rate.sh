#!/bin/sh
# revert-rate.sh -- revert and fix-commit rates for a git repository.
#
# Purpose
#   Longitudinal-study signal for "the change shipped but had to be undone
#   or patched": counts revert commits and fix commits over the first-parent,
#   non-merge history and reports them per 100 commits.
#
# Classification (a commit counts once; revert takes precedence over fix)
#   revert: subject matches           ^Revert "
#           OR body contains          This reverts commit<space>
#   fix:    subject matches one of these conservative anchored patterns,
#           case-insensitively (this is the exact, complete list):
#               ^fix[(:! ]
#               ^fix$
#               ^hotfix[(:! ]
#               ^bugfix[(:! ]
#
# Commit set
#   First-parent, non-merge commits of HEAD in [--since, --until].
#
# Limitations
#   * Only the first-parent chain is inspected; reverts that live on merged
#     side branches are not seen.
#   * Message-based classification only: reverts done by hand without the
#     conventional message, or fixes with other subject styles, are missed.
#
# Output
#   TSV on stdout: a header line
#       total_commits<TAB>reverts<TAB>fixes<TAB>reverts_per_100<TAB>fixes_per_100
#   followed by one data row (rates to 2 decimals; 0.00 when the commit set
#   is empty). Errors and usage go to stderr; exit 1 on bad usage,
#   0 otherwise.
#
# Usage
#   revert-rate.sh [--since DATE] [--until DATE] [REPO_DIR]
#
#   REPO_DIR defaults to `.`. The script never writes inside the target
#   repository; every git command runs via `git -C "$REPO_DIR"`.

set -eu

usage() {
    echo "usage: revert-rate.sh [--since DATE] [--until DATE] [REPO_DIR]" >&2
}

SINCE=""
UNTIL=""
REPO_DIR=""

while [ $# -gt 0 ]; do
    case "$1" in
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
            echo "revert-rate.sh: unknown option: $1" >&2
            usage
            exit 1
            ;;
        *)
            if [ -n "$REPO_DIR" ]; then
                echo "revert-rate.sh: too many arguments" >&2
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

if ! git -C "$REPO_DIR" rev-parse --git-dir >/dev/null 2>&1; then
    echo "revert-rate.sh: not a git repository: $REPO_DIR" >&2
    exit 1
fi

g() {
    git -C "$REPO_DIR" "$@"
}

TOTAL=0
REVERTS=0
FIXES=0

COMMITS=""
if g rev-parse -q --verify 'HEAD^{commit}' >/dev/null 2>&1; then
    set --
    if [ -n "$SINCE" ]; then
        set -- "$@" --since="$SINCE"
    fi
    if [ -n "$UNTIL" ]; then
        set -- "$@" --until="$UNTIL"
    fi
    # Capture into a variable so a rev-list failure aborts under set -e.
    COMMITS=$(g rev-list --first-parent --no-merges "$@" HEAD)
fi

for C in $COMMITS; do
    TOTAL=$((TOTAL + 1))
    SUBJECT=$(g show -s --format=%s "$C")
    BODY=$(g show -s --format=%b "$C")

    IS_REVERT=0
    case "$SUBJECT" in
        'Revert "'*) IS_REVERT=1 ;;
    esac
    if [ "$IS_REVERT" -eq 0 ]; then
        case "$BODY" in
            *'This reverts commit '*) IS_REVERT=1 ;;
        esac
    fi

    if [ "$IS_REVERT" -eq 1 ]; then
        REVERTS=$((REVERTS + 1))
        continue
    fi

    # Case-insensitive anchored fix patterns (see header for the list).
    LSUBJECT=$(printf '%s' "$SUBJECT" | tr '[:upper:]' '[:lower:]')
    case "$LSUBJECT" in
        fix|'fix('*|'fix:'*|'fix!'*|'fix '*) FIXES=$((FIXES + 1)) ;;
        'hotfix('*|'hotfix:'*|'hotfix!'*|'hotfix '*) FIXES=$((FIXES + 1)) ;;
        'bugfix('*|'bugfix:'*|'bugfix!'*|'bugfix '*) FIXES=$((FIXES + 1)) ;;
    esac
done

RATES=$(awk -v t="$TOTAL" -v r="$REVERTS" -v f="$FIXES" 'BEGIN {
    if (t > 0) printf "%.2f\t%.2f", r * 100 / t, f * 100 / t
    else printf "0.00\t0.00"
}')

printf 'total_commits\treverts\tfixes\treverts_per_100\tfixes_per_100\n'
printf '%s\t%s\t%s\t%s\n' "$TOTAL" "$REVERTS" "$FIXES" "$RATES"
exit 0
