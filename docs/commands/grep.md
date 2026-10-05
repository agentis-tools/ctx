# ctx grep

Search file contents like ripgrep, with every match tagged by what the index knows about it.

## Synopsis

```bash
ctx grep <PATTERN> [PATHS]... [OPTIONS]
```

## Description

`ctx grep` runs ripgrep's search engine and file walker (the same libraries
`rg` is built from), so it finds exactly the lines `rg -n` finds. It never
drops a match. What it adds is a label on each matching line saying what the
code-intelligence index knows about the identifier the match hit.

Text search and the call graph fail in opposite directions. Text search finds
every occurrence, including every unrelated symbol that shares the name. The
graph binds names to the right symbol, but sites it has no edge for, such as
interface members, dynamic dispatch, macros, config, and scripts, never
appear in its answers, and nothing tells you they are missing. `ctx grep`
keeps the text search's complete result and uses the graph only to label it,
so you can tell the matches you can trust from the ones you need to read.

Files are selected the way `rg` selects them: hidden files and anything in
`.gitignore`/`.ignore` are skipped. The index's own include rules are *not*
applied, so files outside the index still appear, tagged `unindexed`.

Positional paths and globs after the pattern restrict the search. Literal
files, directories, and globs are ORed together; with none, the whole
repository is searched.

## Tags

| Tag | Meaning |
|-----|---------|
| `def` | The identifier is the name of a symbol defined on this line. Shows the symbol. |
| `ref` | An index edge on this line references the identifier and resolves to a symbol. Shows the edge kind (`calls`, `uses`, `implements`, …) and the target. |
| `unresolved` | An index edge references the identifier, but its target could not be determined. Shows how many indexed symbols share the name. Running `ctx judge edges` resolves many of these. |
| `unlinked` | The file is indexed and current, but the index has no fact for this identifier on this line. Typical causes: comments, strings, local variables, macro bodies, and multi-line call chains. It does **not** mean the site is irrelevant. |
| `stale` | The file changed since it was last indexed. Line-level facts are withheld rather than guessed; run `ctx index`. |
| `unindexed` | The file is not in the index (unsupported language, excluded, or no index at all). |

`def` and `ref` tags also show `family <Type>` when the target is a member of a
trait, interface, or base class that the index can see. Renaming one member of
a family usually means renaming all of them.

On a line where any identifier is linked, other identifiers in the match are
not reported as `unlinked`. When nothing on the line is linked, the
unlinked identifiers are listed together.

The tag names are versioned (`tag_vocabulary_version` in JSON output) and
will not change meaning within a version.

## Output

```text
ctx grep: 6 matches in 3 files · def 2 · unresolved 1 · unlinked 2 · unindexed 1 · targets: Conn::close ×1, File::close ×1
NOTES.md:1:Remember to close everything.  [unindexed]
src/conn.rs:2:    fn close(&self);  [unlinked close]
src/conn.rs:8:    fn close(&self) {}  [def Conn::close · family Closeable]
src/conn.rs:14:    // close the connection before exit  [unlinked close]
src/conn.rs:16:    conn.close();  [unresolved close · 2 candidates]
src/file.rs:4:    pub fn close(&self) {}  [def File::close]
```

The first line counts matching lines per tag and lists the symbols the
pattern resolved to, most frequent first. Each following line is
`path:line:text` followed by its tags in brackets.

Read the example the way an agent would before renaming `Conn::close`. The
definition carries `family Closeable`, so the trait declaration on line 2
must change too, even though the index has no symbol for a bodyless trait
method (`unlinked`). The call on line 16 is `unresolved` between two
candidates: it does call `Conn::close`, but only reading the line shows that.
The graph alone would have returned neither site. `File::close` is a
different symbol and should be left alone.

With `--no-annotate` the index is not opened and the output is exactly
`path:line:text`, the same as `rg -n --no-heading`. This is useful as the
plain arm when comparing annotated and plain search.

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `-F, --fixed-strings` | Treat the pattern as a literal string | false |
| `--ignore-case` | Case-insensitive search (`-i` is ctx's global `--ignore`, see below) | false |
| `-S, --smart-case` | Case-insensitive unless the pattern contains an uppercase letter | false |
| `-w, --word-regexp` | Only match whole words | false |
| `-g, --glob <GLOB>` | Include files matching a glob; prefix with `!` to exclude. Repeatable | — |
| `-t, --type <TYPE>` | Only search files of a ripgrep file type (`rust`, `py`, `ts`, …). Repeatable | — |
| `--hidden` | Search hidden files and directories | false |
| `-i, --ignore <GLOB>` | Exclude files matching a glob (global flag, same meaning as elsewhere in ctx). Repeatable | — |
| `--no-gitignore` | Do not honor `.gitignore`/`.ignore` (global flag) | false |
| `--max-results <N>` | Stop after N matching lines; the summary and JSON report truncation | unlimited |
| `--no-annotate` | Do not consult the index; plain `rg -n` output | false |
| `--json` | Machine-readable output (global flag) | false |

The pattern uses Rust regular expression syntax, as `rg` does by default.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success, with or without matches |
| 2 | Invalid pattern, glob, or file type, or another operational error |

Unlike `rg`, finding no matches is not exit code 1: in ctx, exit code 1 means
"findings", and a search result is not a finding.

## Examples

```bash
# Every occurrence of a name, with what each one is
ctx grep -w close

# Before renaming: which sites bind to the method, which to other
# same-named symbols, and which the index cannot vouch for
ctx grep -w close src/

# Literal search in Rust files only
ctx grep -F 'conn.close(' -t rust

# Exclude tests
ctx grep -w handler -g '!tests/**'

# Plain output for scripting or comparison
ctx grep --no-annotate -w close

# Machine-readable (standard envelope: ctx_version, command, generated_at, data)
ctx grep --json -w close
```
