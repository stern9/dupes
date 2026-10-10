# dupes

[![CI](https://github.com/stern9/dupes/actions/workflows/ci.yml/badge.svg)](https://github.com/stern9/dupes/actions/workflows/ci.yml)

A small command-line tool that finds files with identical content and tells you
how much disk space the extra copies take up.

`dupes` only reports. It never deletes, moves, or modifies anything.

## How it works

1. Walk the given directories and record the size of every regular file.
2. Group files by size. A file whose size is unique cannot have a duplicate, so
   it is never opened.
3. Hash the remaining candidates with [BLAKE3](https://github.com/BLAKE3-team/BLAKE3),
   in parallel across all cores with [rayon](https://github.com/rayon-rs/rayon).
4. Files with the same size and hash form a duplicate group.

Details worth knowing:

- Symlinks are not followed and are not reported.
- Empty files are skipped by default (`--min-size 0` includes them).
- Hard links to the same file, and files reached twice through overlapping
  paths, count once: removing the extra name would not free any space.
  (Hard-link detection is Unix only.)
- Files and directories that cannot be read produce a warning on stderr and are
  skipped. The exit status is still 0.

## Install

Requires a recent stable Rust toolchain (edition 2024).

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

### Example

Run in a directory with a few copied files:

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
freed by keeping a single copy. Groups are sorted with the most wasteful first.

Several paths and a size threshold:

```console
$ dupes --min-size 1000 photos backup docs
3 x 3.3 MiB (6.7 MiB reclaimable)  blake3:1e603129c34b
  backup/beach.jpg
  backup/old/IMG_0042.jpg
  photos/2024/beach.jpg

2 x 46.9 KiB (46.9 KiB reclaimable)  blake3:1649ee813ebd
  backup/report-final.pdf
  docs/report.pdf

3 duplicate files in 2 groups, 6.7 MiB reclaimable (6 files scanned)
```

### JSON

`--json` prints the same report in a stable, machine-readable form, with exact
byte counts and full hashes:

```console
$ dupes --json --min-size 1000000
{
  "files_scanned": 3,
  "duplicate_files": 2,
  "reclaimable_bytes": 7000000,
  "groups": [
    {
      "size": 3500000,
      "reclaimable_bytes": 7000000,
      "hash": "1e603129c34b9217ff2a14eb69af7018a01e400fb3a6182770708da77c99d79a",
      "paths": [
        "./backup/beach.jpg",
        "./backup/old/IMG_0042.jpg",
        "./photos/2024/beach.jpg"
      ]
    }
  ]
}
```

For example, to list every redundant copy with [jq](https://jqlang.org):

```sh
dupes --json | jq -r '.groups[].paths[1:][]'
```

Paths that are not valid UTF-8 are written lossily in JSON output.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The logic lives in the library crate (`src/lib.rs`); `src/main.rs` only parses
arguments and prints. Integration tests in `tests/` run the real binary against
temporary directories.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
