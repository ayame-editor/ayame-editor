# CLI Reference

`ayame` can open the native editor, run the local web editor, and process huge
text files from a terminal. Commands are designed to stream or use bounded
memory so they remain useful on files that are too large for ordinary editors.

<div class="doc-jump-grid">
  <a class="doc-jump" href="#commands">Inspect and read files</a>
  <a class="doc-jump" href="#transform-options">Search and transform</a>
  <a class="doc-jump" href="#sort-and-group-options">Sort and aggregate</a>
  <a class="doc-jump" href="#serve-options">Open the web editor</a>
  <a class="doc-jump" href="#update-and-remove">Update or remove</a>
  <a class="doc-jump" href="#examples">Copy an example</a>
</div>

## Usage

```sh
ayame <COMMAND> [OPTIONS]
```

No arguments open the native desktop window in GUI builds. Plain CLI builds
print help.

## Commands { #commands }

| Command | Purpose |
| --- | --- |
| `stat <FILE>` | Show file size, line count, encoding, EOL style, and index stats. |
| `head <FILE> [-n N]` | Print the first N lines. Default: 10. |
| `tail <FILE> [-n N]` | Print the last N lines. Default: 10. |
| `line <FILE> <N>` | Print one 1-based line. |
| `lines <FILE> <START> <COUNT>` | Print COUNT lines from START, both 1-based. |
| `search <FILE> <PATTERN>` | Search with literal, regex, ignore-case, whole-word, and max-result options. |
| `sort <FILE>` | External merge sort with memory-bounded spill files. |
| `replace <FILE> <FIND> <REPL>` | Streaming replace to a new output file. |
| `case <FILE> <MODE>` | Streaming case conversion (`upper`, `lower`, `camel`, `pascal`, `snake`, `kebab`, `constant`) to a new output file. |
| `grep-lines <FILE> <PATTERN>` | Extract only the matching lines to a new output file. |
| `split <FILE> --lines N` | Split a file into N-line parts. |
| `group <FILE> -k COL` | Group by a key column and count or aggregate values. |
| `top <FILE> -k COL -n N` | Keep the top N rows by key with bounded memory. |
| `distinct <FILE> -k COL` | Estimate distinct key count with HyperLogLog. |
| `gen <FILE> --lines N` | Generate synthetic test data. |
| `serve [FILE]` | Launch the local web editor. |
| `gui [FILE]` | Open the native desktop window when the GUI feature is built. |
| `cache [path|info|gc|clear]` | Inspect or clean the on-disk index cache. |
| `update` | Download, verify, and install the selected GitHub release artifact. |
| `remove` | Remove the installed Ayame binary or app bundle. |
| `version` | Print the Ayame version. |

The former `diff`, `sortdiff`, and `sort-diff` commands were removed in v0.7.0.
For one release they return an error naming the corresponding ayame-diff
command. See [Migrating comparison workflows to ayame-diff](MIGRATING_TO_AYAME_DIFF.md).

## Common Options

| Option | Applies to | Notes |
| --- | --- | --- |
| `--encoding <ENC>` | file-opening commands | Force `utf8`, `utf-16le`, `utf-16be`, `shift_jis`, `euc-jp`, `iso-2022-jp`, or `ascii`. |
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
| `-k`, `--key <COL[S]>` | 1-based key column. `sort` accepts columns in priority order, for example `3,1,2`; other commands accept one column. Omit to use the whole line. |
| `-t`, `--delim <C>` | Field delimiter. Default: comma. Use `\\t` or `tab` for TSV. |
| `--csv` | Use RFC 4180 CSV parsing, including quoted fields. |
| `--quote <C>` | CSV quote character. Default: `"`. |
| `--numeric` | Treat keys as numbers for `sort` and `top`. |

## Sort and Group Options { #sort-and-group-options }

| Option | Notes |
| --- | --- |
| `-r`, `--reverse` | Reverse the sort order (`sort`). |
| `--budget <SIZE>` | In-memory budget before spilling to disk for `sort` / `group`. Default: 256MiB. Accepts sizes like `512MiB` or `2GiB`. |
| `--spill-dir <DIR>` | Directory for external-merge spill files (`sort` / `group`). |

## Transform Options { #transform-options }

| Option | Notes |
| --- | --- |
| `--out <FILE>` | Output file for `sort`, `replace`, `case`, and `grep-lines`. Required for `replace`, `case`, and `grep-lines`. |
| `-i`, `--ignore-case` | Case-insensitive `replace` / `grep-lines`. |
| `-e`, `--regex` | Regex `replace` / `grep-lines` pattern. |
| `-w`, `--whole-word` | Whole-word `grep-lines` matches. |
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
| `--json` | Print the split result (part files and counts) as JSON. |

Default split files use `<stem>.partNNNN<.ext>` names.

## Search Options

| Option | Notes |
| --- | --- |
| `-e`, `--regex` | Interpret the pattern as a regular expression. |
| `-i`, `--ignore-case` | Case-insensitive matching. |
| `-w`, `--whole-word` | Match whole words only. |
| `--max <N>` | Limit printed matches. |
| `--start-byte <N>` | Begin at a byte offset for worker/API resume. |

## Serve Options { #serve-options }

`ayame serve` binds to `127.0.0.1:8777` by default.

| Option | Notes |
| --- | --- |
| `--host <ADDR>` | Bind address. Default: `127.0.0.1`. |
| `--port <N>` | Port. Default: `8777`. |
| `--allow-remote` | Required for non-loopback hosts. This exposes unauthenticated file access to the network. |

## Group, Top, Distinct

| Command | Options |
| --- | --- |
| `group` | `--value <COL>` enables numeric `sum`, `min`, `max`, and `avg`; `--out-groups <FILE>` writes TSV rows; `--json` prints the run summary (`groups`, `runs`, `spill_bytes`). |
| `top` | `-n <N>` sets the count; `--min` returns the smallest keys; `--out-order <FILE>` writes row order as little-endian `u64`; `--json` prints the selected rows. |
| `distinct` | Uses the selected key column and reports an approximate distinct count; `--json` prints the estimate and HyperLogLog stats. |

## Cache Commands

| Command | Notes |
| --- | --- |
| `cache path` | Print the cache directory to stdout. |
| `cache info` | Show cache size and entry summary. |
| `cache gc` | Remove old cache entries. Supports `--max-size`, `--max-age-days`, and `--dry-run`. |
| `cache clear` | Remove cache entries. |

`cache path` prints the directory to stdout (it is the one pipeable datum);
`info`, `gc`, and `clear` write their human-readable report to stderr so stdout
stays free for piping. Add `--json` to any subcommand for the structured form on
stdout.

## Update and Remove { #update-and-remove }

| Command | Options |
| --- | --- |
| `update` | `--version <VERSION>` selects a release tag (`latest` by default); `--install-dir <DIR>` installs there instead of replacing the current install; `--force` allows installing an equal or older release; `--dry-run` resolves the release without changing files. |
| `remove` | `--install-dir <DIR>` removes that install target instead of the current install; `--yes` skips the confirmation prompt; `--dry-run` prints the target without changing files. |

`update` verifies the release `.sha256` file before installing. On macOS it
updates `Ayame.app` when running from an app bundle; otherwise it can replace a
standalone binary. On Windows, replacing or removing the running executable is
completed by a helper after the current process exits. Binaries running from
`/nix/store` are treated as Nix-managed and are not modified; update or remove
them through Nix, or pass `--install-dir` to install a standalone release
elsewhere.

## Exit Codes

Ayame follows the `grep` convention:

| Code | Meaning |
| --- | --- |
| `0` | Success. For `search`, at least one match was found. |
| `1` | `search` ran cleanly but found no matches. |
| `2` | A usage error, or a failure during the run. |

`search --json` always exits `0` — machine callers read match status from the
`hits` array rather than the exit code.

## Examples { #examples }

```sh
ayame stat huge.csv
ayame head huge.log -n 20
ayame tail huge.log -n 200
ayame line huge.log 500000
ayame lines huge.log 500000 50
ayame search huge.log 'ERROR' -i --max 50
ayame sort huge.csv -k 1 --csv --out sorted.csv
ayame replace huge.log ERROR WARN --out fixed.log --jobs 0
ayame case huge.csv lower --out lower.csv
ayame grep-lines huge.log 'ERROR' -i --out errors.log
ayame split huge.csv --lines 1000000
ayame group huge.csv -k 3 --value 5
ayame top huge.csv -k 2 -n 100 --numeric
ayame distinct huge.csv -k 4
ayame gen sample.csv --lines 100000
ayame cache info
ayame update --dry-run
ayame serve huge.csv --port 8777
```
