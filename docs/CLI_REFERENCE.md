# CLI Reference

*日本語版: [ja/CLI_REFERENCE.md](ja/CLI_REFERENCE.md)*

`ayame` can open the native editor, run the local web editor, and process huge
text files from a terminal. Commands are designed to stream or use bounded
memory so they remain useful on files that are too large for ordinary editors.

## Usage

```sh
ayame <COMMAND> [OPTIONS]
```

No arguments open the native desktop window in GUI builds. Plain CLI builds
print help.

### Output streams and exit codes

Ayame follows a **stdout = data / stderr = diagnostics** policy: pipeable
results (matches, sorted lines, group rows, JSON, `cache path`) go to stdout,
while summaries and progress reports go to stderr. `--json` always writes its
structured payload to stdout.

Exit codes follow grep convention:

| Code | Meaning |
| --- | --- |
| `0` | Success; for `search` / `grep-lines`, at least one match. |
| `1` | A matcher (`search` / `grep-lines`) completed with zero matches, or a runtime error occurred. |
| `2` | Usage or argument error (unknown option, missing value, unknown command). |

## Commands

| Command | Purpose |
| --- | --- |
| `stat <FILE>` | Show file size, line count, encoding, EOL style, and index stats. |
| `head <FILE> [-n N]` | Print the first N lines. Default: 10. |
| `tail <FILE> [-n N]` | Print the last N lines. Default: 10. |
| `line <FILE> <N>` | Print one 1-based line. |
| `lines <FILE> <START> <COUNT>` | Print COUNT lines from START, both 1-based. Each line is prefixed with its 1-based number and a tab (`head`/`tail`/`line` print bare text). |
| `search <FILE> <PATTERN>` | Search with literal, regex, ignore-case, whole-word, and max-result options. |
| `diff <OLD> <NEW>` | Compare two files with line hunks or side-by-side output. |
| `sort <FILE>` | External merge sort with memory-bounded spill files. |
| `sortdiff <OLD> <NEW>` | Sort both files, then diff the sorted outputs. `sort-diff` is also accepted. |
| `replace <FILE> <FIND> <REPL>` | Streaming replace to a new output file. |
| `case <FILE> <MODE>` | Streaming case conversion to a new output file. `MODE` is `upper`, `lower`, `camel`, `pascal`, `snake`, `kebab`, or `constant`. |
| `grep-lines <FILE> <PATTERN>` | Extract only the matching lines to a new output file. |
| `split <FILE> --lines N` | Split a file into N-line parts. |
| `group <FILE> -k COL` | Group by a key column and count or aggregate values. |
| `top <FILE> -k COL -n N` | Keep the top N rows by key with bounded memory. |
| `distinct <FILE> -k COL` | Estimate distinct key count with HyperLogLog. |
| `gen <FILE> --lines N` | Generate synthetic test data. |
| `serve [FILE]` | Launch the local web editor. |
| `gui [FILE]` | Open the native desktop window when the GUI feature is built. |
| `cache [path|info|gc|clear]` | Inspect or clean the on-disk index cache. |
| `version` | Print the Ayame version. |

## Common Options

| Option | Applies to | Notes |
| --- | --- | --- |
| `--encoding <ENC>` | file-opening commands | Force `utf8`, `utf-16le`, `utf-16be`, `shift_jis`, `euc-jp`, or `ascii`. |
| `--stride <N>` | file-opening commands | Lines per sparse-index checkpoint. Default: 4096. |
| `--no-cache` | file-opening commands | Disable persistent index-cache reads and writes. |
| `--cache-dir <DIR>` | file-opening commands | Override the index-cache directory. |
| `--json` | `stat`, `search`, `split`, `group`, `top`, `distinct`, `cache` | Emit machine-readable output on stdout. |
| `-V`, `--version` | global | Print the version. |
| `-h`, `--help` | global | Print command help. |

## Field Options

`sort`, `group`, `top`, and `distinct` can parse a key column:

| Option | Notes |
| --- | --- |
| `-k`, `--key <COL>` | 1-based key column. Omit to use the whole line. |
| `-t`, `--delim <C>` | Field delimiter. Default: comma. |
| `--csv` | Use RFC 4180 CSV parsing, including quoted fields. |
| `--quote <C>` | CSV quote character. Default: `"`. |
| `--numeric` | Treat keys as numbers for `sort` and `top`. (`sort` also accepts `-n` as a hidden compatibility alias; prefer `--numeric` since `-n` means a count elsewhere.) |

## Sort Options

| Option | Notes |
| --- | --- |
| `--out <FILE>` | Write sorted text to a file instead of stdout. |
| `--out-order <FILE>` | Write the sorted order as little-endian `u64` line numbers. |
| `-r`, `--reverse` | Sort in descending order. |
| `--budget <SIZE>` | In-memory budget before spilling to disk. Default: 256MiB. Accepts `K`/`M`/`G` suffixes. |
| `--spill-dir <DIR>` | Directory for spill runs. Default: a per-process temp directory. |

## Transform Options

| Option | Notes |
| --- | --- |
| `--out <FILE>` | Output file for `sort`, `replace`, `case`, and `grep-lines`. Required for `replace`, `case`, and `grep-lines`. |
| `-i`, `--ignore-case` | Case-insensitive `replace` / `grep-lines`. |
| `-e`, `--regex` | Regex `replace` / `grep-lines` pattern. |
| `-w`, `--word`, `--whole-word` | Whole-word `grep-lines` matches (same alias set as `search`). |
| `--overwrite` | Let `grep-lines --out` replace an existing file. |
| `--jobs <N>` | Parallel workers for `replace`, `case`, and `grep-lines`. `0` uses the Rayon default. |
| `--chunk-lines <N>` | Lines per parallel chunk. Default: 4000000. |

Output commands refuse to overwrite existing files (for `grep-lines`,
`--overwrite` opts in). Choose a new output path or remove the target
intentionally before rerunning.

## Split Options

| Option | Notes |
| --- | --- |
| `--lines <N>` | Lines per output part. Required and must be at least 1. |
| `--out-dir <DIR>` | Output directory. Default: the source file's directory. |
| `--name <NAME>` | Base file name for parts. Default: the input file name. |
| `--json` | Emit the split result (part paths, part count, total lines) as JSON on stdout. |

Default split files use `<stem>.partNNNN<.ext>` names.

## Search Options

| Option | Notes |
| --- | --- |
| `-e`, `--regex` | Interpret the pattern as a regular expression. |
| `-i`, `--ignore-case` | Case-insensitive matching. |
| `-w`, `--word`, `--whole-word` | Match whole words only. |
| `--max <N>` | Limit printed matches. |
| `--start-byte <N>` | Begin at a byte offset for worker/API resume. |

## Serve Options

`ayame serve` binds to `127.0.0.1:8777` by default.

| Option | Notes |
| --- | --- |
| `--host <ADDR>` | Bind address. Default: `127.0.0.1`. |
| `--port <N>` | Port. Default: `8777`. |
| `--allow-remote` | Required for non-loopback hosts. This exposes unauthenticated file access to the network. |

## Group, Top, Distinct

| Command | Options |
| --- | --- |
| `group` | `--value <COL>` enables numeric `sum`, `min`, `max`, and `avg`; `--out-groups <FILE>` writes TSV rows; `--json` emits the rows as JSON. |
| `top` | `-n <N>` sets the count; `--min` returns the smallest keys; `--out-order <FILE>` writes row order as little-endian `u64`; `--json` emits ranked rows (line number + text) as JSON. |
| `distinct` | Uses the selected key column and reports an approximate distinct count. `-p`, `--precision <N>` sets HyperLogLog precision (4–18, default 14); `--json` emits the estimate and register stats as JSON. |

For all three the count/summary line is a diagnostic on stderr; the row data (or
JSON) is on stdout.

## Cache Commands

| Command | Notes |
| --- | --- |
| `cache path` | Print the cache directory (on stdout — it is pipeable data). |
| `cache info` | Show cache size and entry summary (on stderr). |
| `cache gc` | Remove old cache entries. Supports `--max-size`, `--max-age-days`, and `--dry-run`. The report prints on stderr. |
| `cache clear` | Remove cache entries. |

Every `cache` subcommand accepts `--json`, which writes its structured result to
stdout instead of the human-readable report.

## Examples

```sh
ayame stat huge.csv
ayame head huge.log -n 20
ayame tail huge.log -n 200
ayame line huge.log 500000
ayame lines huge.log 500000 50
ayame search huge.log 'ERROR' -i --max 50
ayame diff old.csv new.csv --side-by-side --width 180
ayame sort huge.csv -k 1 --csv --out sorted.csv
ayame sortdiff old.csv new.csv -k 1 --summary
ayame replace huge.log ERROR WARN --out fixed.log --jobs 0
ayame case huge.csv lower --out lower.csv
ayame grep-lines huge.log 'ERROR' -i --out errors.log
ayame split huge.csv --lines 1000000
ayame group huge.csv -k 3 --value 5
ayame top huge.csv -k 2 -n 100 --numeric
ayame distinct huge.csv -k 4
ayame gen sample.csv --lines 100000
ayame cache info
ayame serve huge.csv --port 8777
```
