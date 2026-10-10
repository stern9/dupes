//! End-to-end tests that run the real `dupes` binary against a temp directory.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::Value;
use tempfile::TempDir;

fn dupes() -> Command {
    Command::new(env!("CARGO_BIN_EXE_dupes"))
}

fn write(dir: &Path, name: &str, content: &[u8]) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

/// Runs `dupes --json` with the given extra arguments and parses stdout.
fn json_report(dir: &Path, args: &[&str]) -> Value {
    let output = dupes()
        .current_dir(dir)
        .arg("--json")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).unwrap()
}

fn paths_of(group: &Value) -> Vec<&str> {
    group["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect()
}

#[test]
fn reports_duplicates_across_nested_directories() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "a.txt", b"hello world");
    write(tmp.path(), "sub/b.txt", b"hello world");
    write(tmp.path(), "sub/deep/c.txt", b"hello world");
    write(tmp.path(), "unique.txt", b"something else entirely");

    dupes()
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout(contains("3 x 11 B (22 B reclaimable)"))
        .stdout(contains("  ./a.txt\n  ./sub/b.txt\n  ./sub/deep/c.txt\n"))
        .stdout(contains(
            "2 duplicate files in 1 group, 22 B reclaimable (4 files scanned)",
        ))
        .stdout(contains("unique.txt").not())
        .stderr("");
}

#[test]
fn same_size_but_different_content_is_not_a_duplicate() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "a.txt", b"aaaa");
    write(tmp.path(), "b.txt", b"bbbb");

    dupes()
        .current_dir(tmp.path())
        .assert()
        .success()
        .stdout("No duplicates found (2 files scanned).\n");
}

#[test]
fn json_output_is_machine_readable() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "big1", &[7u8; 1000]);
    write(tmp.path(), "big2", &[7u8; 1000]);
    write(tmp.path(), "small1", b"xy");
    write(tmp.path(), "small2", b"xy");
    write(tmp.path(), "small3", b"xy");
    write(tmp.path(), "other", b"zzz");

    let report = json_report(tmp.path(), &[]);
    assert_eq!(report["files_scanned"], 6);
    assert_eq!(report["duplicate_files"], 3);
    assert_eq!(report["reclaimable_bytes"], 1004);

    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
    // Most reclaimable space first.
    assert_eq!(groups[0]["size"], 1000);
    assert_eq!(groups[0]["reclaimable_bytes"], 1000);
    assert_eq!(paths_of(&groups[0]), ["./big1", "./big2"]);
    assert_eq!(
        groups[0]["hash"],
        blake3::hash(&[7u8; 1000]).to_hex().as_str()
    );
    assert_eq!(groups[1]["size"], 2);
    assert_eq!(groups[1]["reclaimable_bytes"], 4);
    assert_eq!(paths_of(&groups[1]), ["./small1", "./small2", "./small3"]);
}

#[test]
fn empty_files_are_skipped_unless_min_size_is_zero() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "empty1", b"");
    write(tmp.path(), "empty2", b"");

    let report = json_report(tmp.path(), &[]);
    assert_eq!(report["files_scanned"], 0);
    assert_eq!(report["groups"].as_array().unwrap().len(), 0);

    let report = json_report(tmp.path(), &["--min-size", "0"]);
    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(paths_of(&groups[0]), ["./empty1", "./empty2"]);
    assert_eq!(report["reclaimable_bytes"], 0);
}

#[test]
fn min_size_filters_small_files() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "small1", b"12345");
    write(tmp.path(), "small2", b"12345");
    write(tmp.path(), "large1", b"1234567890");
    write(tmp.path(), "large2", b"1234567890");

    let report = json_report(tmp.path(), &["--min-size", "10"]);
    let groups = report["groups"].as_array().unwrap();
    assert_eq!(report["files_scanned"], 2);
    assert_eq!(groups.len(), 1);
    assert_eq!(paths_of(&groups[0]), ["./large1", "./large2"]);
}

#[test]
fn accepts_multiple_roots_and_finds_duplicates_between_them() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "one/photo.jpg", b"not really a jpeg");
    write(tmp.path(), "two/copy.jpg", b"not really a jpeg");

    let report = json_report(tmp.path(), &["one", "two"]);
    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(paths_of(&groups[0]), ["one/photo.jpg", "two/copy.jpg"]);
}

#[test]
fn overlapping_roots_do_not_report_a_file_as_its_own_duplicate() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "dir/only.txt", b"just one copy");

    let report = json_report(tmp.path(), &[".", "dir", "dir"]);
    assert_eq!(report["files_scanned"], 1);
    assert_eq!(report["groups"].as_array().unwrap().len(), 0);
}

#[test]
fn missing_path_is_a_warning_not_a_failure() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "a", b"same");
    write(tmp.path(), "b", b"same");

    dupes()
        .current_dir(tmp.path())
        .args([".", "does-not-exist"])
        .assert()
        .success()
        .stdout(contains("1 duplicate file in 1 group"))
        .stderr(contains("dupes: warning: does-not-exist: "));
}

#[test]
fn does_not_modify_the_files_it_scans() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "a", b"same");
    write(tmp.path(), "b", b"same");

    dupes().current_dir(tmp.path()).assert().success();

    let mut names: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(fs::read(tmp.path().join("a")).unwrap(), b"same");
    assert_eq!(fs::read(tmp.path().join("b")).unwrap(), b"same");
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn symlinks_are_not_followed_or_reported() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "real/a.txt", b"content");
        symlink(tmp.path().join("real/a.txt"), tmp.path().join("link.txt")).unwrap();
        symlink(tmp.path().join("real"), tmp.path().join("linked-dir")).unwrap();

        let report = json_report(tmp.path(), &[]);
        assert_eq!(report["files_scanned"], 1);
        assert_eq!(report["groups"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn hard_links_to_the_same_file_are_not_duplicates() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "a.txt", b"content");
        fs::hard_link(tmp.path().join("a.txt"), tmp.path().join("b.txt")).unwrap();

        let report = json_report(tmp.path(), &[]);
        assert_eq!(report["files_scanned"], 1);
        assert_eq!(report["groups"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn unreadable_entries_are_warned_about_and_skipped() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "a", b"same");
        write(tmp.path(), "b", b"same");
        write(tmp.path(), "locked-file", b"same");
        write(tmp.path(), "locked-dir/c", b"same");
        let locked_file = tmp.path().join("locked-file");
        let locked_dir = tmp.path().join("locked-dir");
        fs::set_permissions(&locked_file, fs::Permissions::from_mode(0o000)).unwrap();
        fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o000)).unwrap();

        // Permission bits do not stop root, so there is nothing to observe.
        let running_as_root = fs::read(&locked_file).is_ok();

        let assert = dupes().current_dir(tmp.path()).assert();

        // Restore access so the temp directory can be cleaned up.
        fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&locked_file, fs::Permissions::from_mode(0o644)).unwrap();

        if running_as_root {
            return;
        }
        assert
            .success()
            .stdout(contains("2 x 4 B (4 B reclaimable)"))
            .stdout(contains("  ./a\n  ./b\n"))
            .stderr(contains("dupes: warning: ./locked-file: "))
            .stderr(contains("dupes: warning: ./locked-dir: "));
    }
}
