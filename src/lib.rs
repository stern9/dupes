//! Find duplicate files by content.
//!
//! The pipeline is deliberately cheap before it is thorough:
//!
//! 1. [`scan`] walks the given roots and records every regular file with its size.
//! 2. [`group_by_size`] buckets those files by size. A file with a unique size
//!    cannot have a duplicate, so it is never opened.
//! 3. [`group_duplicates`] hashes the remaining candidates in parallel and
//!    groups them by content hash.
//!
//! [`find_duplicates`] runs all three and returns a [`Report`]. Nothing in this
//! crate deletes or modifies files.

use std::collections::{HashMap, HashSet};
use std::fs::{File, Metadata};
use std::hash::Hash;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::{Serialize, Serializer};
use walkdir::WalkDir;

/// Settings that control which files are considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Files smaller than this many bytes are ignored.
    ///
    /// The default of 1 skips empty files; set it to 0 to include them.
    pub min_size: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { min_size: 1 }
    }
}

/// A regular file found during the scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
}

/// A set of two or more files with identical content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateGroup {
    /// Size of each file in the group, in bytes.
    pub size: u64,
    /// Bytes that would be freed by keeping only one file of the group.
    pub reclaimable_bytes: u64,
    /// Hex-encoded BLAKE3 hash of the shared content.
    pub hash: String,
    /// Paths of the identical files, sorted.
    #[serde(serialize_with = "serialize_paths")]
    pub paths: Vec<PathBuf>,
}

/// The outcome of a duplicate search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Number of files that passed the size filter.
    pub files_scanned: usize,
    /// Number of files that could be removed while keeping one copy of each.
    pub duplicate_files: usize,
    /// Total bytes that could be freed by keeping one copy of each.
    pub reclaimable_bytes: u64,
    /// Duplicate groups, the most wasteful first.
    pub groups: Vec<DuplicateGroup>,
    /// Problems that caused a file or directory to be skipped.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// Walks `roots`, hashes files that share a size, and reports the duplicates.
///
/// Unreadable files and directories are skipped and recorded in
/// [`Report::warnings`]; they never abort the search.
pub fn find_duplicates(roots: &[PathBuf], options: &Options) -> Report {
    let mut warnings = Vec::new();
    let files = scan(roots, options, &mut warnings);
    let files_scanned = files.len();
    let (groups, hash_warnings) = group_duplicates(files, hash_file);
    warnings.extend(hash_warnings);

    Report {
        files_scanned,
        duplicate_files: groups.iter().map(|g| g.paths.len() - 1).sum(),
        reclaimable_bytes: groups.iter().map(|g| g.reclaimable_bytes).sum(),
        groups,
        warnings,
    }
}

/// Collects every regular file under `roots` that is at least
/// `options.min_size` bytes.
///
/// Symlinks are not followed and are not reported themselves. A file reached
/// more than once (overlapping roots, or hard links to the same inode on Unix)
/// is listed only once, because removing the extra name would free no space.
pub fn scan(roots: &[PathBuf], options: &Options, warnings: &mut Vec<String>) -> Vec<FileEntry> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for root in roots {
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    warnings.push(walk_error_message(root, &err));
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(err) => {
                    warnings.push(walk_error_message(entry.path(), &err));
                    continue;
                }
            };
            if metadata.len() < options.min_size {
                continue;
            }
            if !seen.insert(file_identity(entry.path(), &metadata)) {
                continue;
            }
            files.push(FileEntry {
                size: metadata.len(),
                path: entry.into_path(),
            });
        }
    }

    files
}

/// Buckets files by size. Only buckets with more than one file can contain
/// duplicates.
pub fn group_by_size(files: Vec<FileEntry>) -> HashMap<u64, Vec<PathBuf>> {
    let mut by_size: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for file in files {
        by_size.entry(file.size).or_default().push(file.path);
    }
    by_size
}

/// Groups `files` into sets with identical content.
///
/// `hasher` is only called for files that share their size with at least one
/// other file, and is called from multiple threads. Files it fails on are
/// skipped and described in the returned warnings.
///
/// Groups are ordered by reclaimable space (largest first), then by path, so
/// the result is deterministic regardless of thread scheduling.
pub fn group_duplicates<H>(files: Vec<FileEntry>, hasher: H) -> (Vec<DuplicateGroup>, Vec<String>)
where
    H: Fn(&Path) -> io::Result<String> + Sync,
{
    let candidates: Vec<(u64, PathBuf)> = group_by_size(files)
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .flat_map(|(size, paths)| paths.into_iter().map(move |path| (size, path)))
        .collect();

    let hashed: Vec<(u64, PathBuf, io::Result<String>)> = candidates
        .into_par_iter()
        .map(|(size, path)| {
            let hash = hasher(&path);
            (size, path, hash)
        })
        .collect();

    let mut warnings = Vec::new();
    let mut by_content: HashMap<(u64, String), Vec<PathBuf>> = HashMap::new();
    for (size, path, hash) in hashed {
        match hash {
            Ok(hash) => by_content.entry((size, hash)).or_default().push(path),
            Err(err) => warnings.push(format!("{}: {err}", path.display())),
        }
    }
    warnings.sort();

    let mut groups: Vec<DuplicateGroup> = by_content
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|((size, hash), mut paths)| {
            paths.sort();
            DuplicateGroup {
                size,
                reclaimable_bytes: size.saturating_mul(paths.len() as u64 - 1),
                hash,
                paths,
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.reclaimable_bytes
            .cmp(&a.reclaimable_bytes)
            .then_with(|| a.paths.cmp(&b.paths))
    });

    (groups, warnings)
}

/// Returns the hex-encoded BLAKE3 hash of a file's content.
pub fn hash_file(path: &Path) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok(hasher.finalize().to_hex().to_string())
}

/// Formats a byte count with binary units, e.g. `1.5 KiB`.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    // Compare against the rounded-up boundary so we never print "1024.0 KiB".
    while value >= 1023.95 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

impl Report {
    /// Writes a human-readable listing of the duplicate groups.
    pub fn write_text(&self, out: &mut impl Write) -> io::Result<()> {
        if self.groups.is_empty() {
            return writeln!(
                out,
                "No duplicates found ({} scanned).",
                count(self.files_scanned, "file")
            );
        }

        for group in &self.groups {
            writeln!(
                out,
                "{} x {} ({} reclaimable)  blake3:{}",
                group.paths.len(),
                format_size(group.size),
                format_size(group.reclaimable_bytes),
                group.hash.get(..12).unwrap_or(&group.hash),
            )?;
            for path in &group.paths {
                writeln!(out, "  {}", path.display())?;
            }
            writeln!(out)?;
        }

        writeln!(
            out,
            "{} in {}, {} reclaimable ({} scanned)",
            count(self.duplicate_files, "duplicate file"),
            count(self.groups.len(), "group"),
            format_size(self.reclaimable_bytes),
            count(self.files_scanned, "file"),
        )
    }

    /// Writes the report as pretty-printed JSON followed by a newline.
    pub fn write_json(&self, out: &mut impl Write) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut *out, self).map_err(io::Error::from)?;
        writeln!(out)
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

fn walk_error_message(fallback: &Path, err: &walkdir::Error) -> String {
    let path = err.path().unwrap_or(fallback);
    match err.io_error() {
        Some(io_err) => format!("{}: {io_err}", path.display()),
        None => format!("{}: {err}", path.display()),
    }
}

/// A key that is equal for two directory entries naming the same file.
#[cfg(unix)]
fn file_identity(_path: &Path, metadata: &Metadata) -> impl Hash + Eq + use<> {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

/// A key that is equal for two directory entries naming the same file.
#[cfg(not(unix))]
fn file_identity(path: &Path, _metadata: &Metadata) -> impl Hash + Eq + use<> {
    path.to_path_buf()
}

fn serialize_paths<S: Serializer>(paths: &[PathBuf], serializer: S) -> Result<S::Ok, S::Error> {
    // Paths are not guaranteed to be UTF-8; degrade lossily rather than fail.
    serializer.collect_seq(paths.iter().map(|path| path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            size,
        }
    }

    /// Pretends each file's content is the first character of its file name.
    fn fake_hash(path: &Path) -> io::Result<String> {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('!') {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        }
        Ok(name.chars().take(1).collect())
    }

    #[test]
    fn group_by_size_buckets_files() {
        let by_size = group_by_size(vec![entry("a", 1), entry("b", 2), entry("c", 1)]);
        assert_eq!(by_size.len(), 2);
        assert_eq!(by_size[&1], vec![PathBuf::from("a"), PathBuf::from("c")]);
        assert_eq!(by_size[&2], vec![PathBuf::from("b")]);
    }

    #[test]
    fn same_size_and_hash_form_a_group() {
        let files = vec![entry("d/a2", 10), entry("a1", 10), entry("b1", 10)];
        let (groups, warnings) = group_duplicates(files, fake_hash);
        assert!(warnings.is_empty());
        assert_eq!(
            groups,
            vec![DuplicateGroup {
                size: 10,
                reclaimable_bytes: 10,
                hash: "a".into(),
                paths: vec![PathBuf::from("a1"), PathBuf::from("d/a2")],
            }]
        );
    }

    #[test]
    fn same_hash_but_different_size_is_not_a_group() {
        let files = vec![entry("a1", 10), entry("a2", 11)];
        let (groups, _) = group_duplicates(files, fake_hash);
        assert!(groups.is_empty());
    }

    #[test]
    fn files_with_a_unique_size_are_never_hashed() {
        let files = vec![entry("a1", 1), entry("a2", 2), entry("a3", 3)];
        let (groups, warnings) = group_duplicates(files, |path| {
            panic!("hashed {} although its size is unique", path.display())
        });
        assert!(groups.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn groups_are_sorted_by_reclaimable_space_then_path() {
        let files = vec![
            entry("s1", 5),
            entry("s2", 5),
            entry("b1", 100),
            entry("b2", 100),
            // Three small copies reclaim 2 * 60 = 120, more than one big copy.
            entry("m1", 60),
            entry("m2", 60),
            entry("m3", 60),
            entry("a1", 100),
            entry("a2", 100),
        ];
        let (groups, _) = group_duplicates(files, fake_hash);
        let order: Vec<(&str, u64)> = groups
            .iter()
            .map(|g| (g.hash.as_str(), g.reclaimable_bytes))
            .collect();
        assert_eq!(order, vec![("m", 120), ("a", 100), ("b", 100), ("s", 5)]);
    }

    #[test]
    fn hash_failures_become_warnings_and_drop_only_that_file() {
        let files = vec![entry("a1", 10), entry("a2", 10), entry("!locked", 10)];
        let (groups, warnings) = group_duplicates(files, fake_hash);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);
        assert_eq!(warnings, vec!["!locked: denied".to_string()]);
    }

    #[test]
    fn format_size_uses_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
        assert_eq!(format_size(1024 * 1024 - 1), "1.0 MiB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GiB");
        assert_eq!(format_size(u64::MAX), "16.0 EiB");
    }

    #[test]
    fn text_report_without_duplicates() {
        let report = Report {
            files_scanned: 1,
            duplicate_files: 0,
            reclaimable_bytes: 0,
            groups: Vec::new(),
            warnings: Vec::new(),
        };
        let mut out = Vec::new();
        report.write_text(&mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "No duplicates found (1 file scanned).\n"
        );
    }
}
