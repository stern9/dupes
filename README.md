# dupes

[![CI](https://github.com/stern9/dupes/actions/workflows/ci.yml/badge.svg)](https://github.com/stern9/dupes/actions/workflows/ci.yml)

A command-line tool that finds files with identical content and shows how much
space the extra copies waste. It only reports; it never deletes or changes
anything.

```console
$ dupes
3 x 3.3 MiB (6.7 MiB reclaimable)  blake3:1e603129c34b
  ./backup/beach.jpg
  ./backup/old/IMG_0042.jpg
  ./photos/2024/beach.jpg

2 x 46.9 KiB (46.9 KiB reclaimable)  blake3:1649ee813ebd
  ./backup/report-final.pdf
  ./docs/report.pdf

2 x 5 B (5 B reclaimable)  blake3:19a004cec74d
  ./backup/notes.txt
  ./docs/notes.txt

4 duplicate files in 3 groups, 6.7 MiB reclaimable (8 files scanned)
```

Each group shows the number of copies, the size of one copy, and the space
freed by keeping just one. The most wasteful groups come first.

## Install

With a [Rust toolchain](https://rustup.rs) installed:

```sh
git clone https://github.com/stern9/dupes
cd dupes
cargo install --path .
```

## Usage

```text
Usage: dupes [OPTIONS] [PATH]...

Arguments:
  [PATH]...  Directories (or files) to search [default: .]

Options:
      --min-size <BYTES>  Ignore files smaller than this many bytes (0 includes empty files) [default: 1]
      --json              Print the report as JSON instead of text
  -h, --help              Print help (see more with '--help')
  -V, --version           Print version
```

`--json` prints the same report with exact byte counts and full hashes, for
scripting. For example, to list every redundant copy with
[jq](https://jqlang.org):

```sh
dupes --json | jq -r '.groups[].paths[1:][]'
```

## Good to know

- Files are grouped by size first, so a file with a unique size is never
  opened. The rest are hashed with [BLAKE3](https://github.com/BLAKE3-team/BLAKE3)
  in parallel.
- Symlinks are not followed, and empty files are skipped by default.
- Hard links to the same file count once, since removing one frees no space
  (Unix only).
- Unreadable entries are skipped with a warning on stderr.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT),
at your option.
