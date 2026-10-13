use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry, parse_config, plan};
use dirule_fs::{ApplyError, ConflictPolicy, apply, apply_with_conflict_policy, scan};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("dirule-fs-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scan_respects_ignore_files_and_includes_hidden_paths() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join(".ignore"), "ignored/\n").expect("write ignore file");
    std::fs::create_dir(temp.0.join("ignored")).expect("create ignored directory");
    std::fs::write(temp.0.join("ignored/file.txt"), "ignored").expect("write ignored file");
    std::fs::write(temp.0.join(".hidden"), "visible to rules").expect("write hidden file");

    let result = scan(&temp.0).expect("scan target");
    let paths = result
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    assert!(paths.contains(&PathBuf::from(".hidden")));
    assert!(!paths.iter().any(|path| path.starts_with("ignored")));
}

#[test]
fn scan_respects_configured_global_gitignore() {
    let temp = TempDir::new();
    let target = temp.0.join("target");
    std::fs::create_dir(&target).expect("create scan target");
    std::fs::write(target.join("globally-ignored.txt"), "ignored")
        .expect("write globally ignored file");

    let global_ignore = temp.0.join("global-ignore");
    std::fs::write(&global_ignore, "globally-ignored.txt\n").expect("write global ignore rules");
    let git_config = temp.0.join("gitconfig");
    let global_ignore = global_ignore.to_string_lossy().replace('\\', "/");
    std::fs::write(
        &git_config,
        format!("[core]\n\texcludesFile = {global_ignore}\n"),
    )
    .expect("write isolated Git config");

    let output = Command::new(std::env::current_exe().expect("test executable path"))
        .args(["--exact", "scan_global_gitignore_child", "--nocapture"])
        .env("GIT_CONFIG_GLOBAL", git_config)
        .env("DIRULE_GLOBAL_IGNORE_TARGET", target)
        .output()
        .expect("run isolated global-ignore test");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn scan_global_gitignore_child() {
    let Some(target) = std::env::var_os("DIRULE_GLOBAL_IGNORE_TARGET") else {
        return;
    };
    let result = scan(Path::new(&target)).expect("scan using isolated global Git config");

    assert!(result.entries.is_empty());
}

#[test]
fn scan_includes_git_tracked_paths_even_when_ignored() {
    let temp = TempDir::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&temp.0)
        .output();
    let Ok(init) = init else {
        return;
    };
    assert!(init.status.success(), "initialize Git fixture");

    std::fs::write(temp.0.join(".gitignore"), "ignored*.txt\n").expect("write ignore rules");
    std::fs::create_dir(temp.0.join("nested")).expect("create nested directory");
    std::fs::write(temp.0.join("nested/.gitignore"), "nested-ignore.txt\n")
        .expect("write nested ignore rules");
    std::fs::write(
        temp.0.join("nested/nested-ignore.txt"),
        "ignored by nested rules",
    )
    .expect("write nested ignored file");
    std::fs::write(temp.0.join(".git/info/exclude"), "local-excluded.txt\n")
        .expect("write repository excludes");
    std::fs::write(temp.0.join("local-excluded.txt"), "excluded locally")
        .expect("write locally excluded file");
    std::fs::write(temp.0.join("ignored.txt"), "tracked and ignored")
        .expect("write tracked ignored file");
    let add_rules = Command::new("git")
        .args(["add", ".gitignore"])
        .current_dir(&temp.0)
        .status()
        .expect("stage Git fixture");
    assert!(add_rules.success(), "stage Git ignore rules");
    let add_ignored = Command::new("git")
        .args(["add", "-f", "ignored.txt"])
        .current_dir(&temp.0)
        .status()
        .expect("force-stage ignored Git fixture");
    assert!(add_ignored.success(), "force-stage ignored Git fixture");
    std::fs::write(
        temp.0.join("ignored-untracked.txt"),
        "ignored and untracked",
    )
    .expect("write untracked ignored file");

    let result = scan(&temp.0).expect("scan Git target");
    let paths = result
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    assert!(paths.contains(&PathBuf::from("ignored.txt")));
    assert!(!paths.contains(&PathBuf::from("ignored-untracked.txt")));
    assert!(!paths.contains(&PathBuf::from("nested/nested-ignore.txt")));
    assert!(!paths.contains(&PathBuf::from("local-excluded.txt")));
}

#[test]
fn apply_moves_directory_once_and_preserves_its_descendants() {
    let temp = TempDir::new();
    std::fs::create_dir(temp.0.join("old")).expect("create source directory");
    std::fs::write(temp.0.join("old/file.txt"), "content").expect("write source file");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("old"),
                destination: PathBuf::from("new"),
                kind: EntryKind::Directory,
                rule: Some("directory".into()),
            },
            PlannedEntry {
                source: PathBuf::from("old/file.txt"),
                destination: PathBuf::from("new/file.txt"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let result = apply(&temp.0, &plan, &snapshot).expect("apply plan");
    assert_eq!(result.moved.len(), 1);
    assert_eq!(
        std::fs::read_to_string(temp.0.join("new/file.txt")).unwrap(),
        "content"
    );
    assert!(!temp.0.join("old").exists());
}

#[test]
fn explicit_descendant_template_survives_a_parent_rename() {
    let temp = TempDir::new();
    std::fs::create_dir(temp.0.join("Parent")).expect("create source directory");
    std::fs::write(temp.0.join("Parent/child.txt"), "content").expect("write source child");
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Rename parent"
match = "Parent"
to = "builtin:kebab-case"

[[rules]]
name = "Keep child at configured path"
match = "Parent/child.txt"
to = "template:Parent/child.txt"
"#,
    )
    .expect("valid config");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = plan(&config, &snapshot.entries).expect("build plan");
    assert!(plan.is_conflict_free());

    let report = apply(&temp.0, &plan, &snapshot).expect("apply plan");

    assert_eq!(report.moved.len(), 2);
    assert!(temp.0.join("parent").is_dir());
    assert_eq!(
        std::fs::read_to_string(temp.0.join("Parent/child.txt")).unwrap(),
        "content"
    );
    assert!(!temp.0.join("parent/child.txt").exists());
}

#[test]
fn apply_revalidates_destinations_before_renaming() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("source"), "source").expect("write source");
    std::fs::write(temp.0.join("destination"), "keep").expect("write destination");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("source"),
            destination: PathBuf::from("destination"),
            kind: EntryKind::File,
            rule: None,
        }],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    assert!(apply(&temp.0, &plan, &snapshot).is_err());
    assert_eq!(
        std::fs::read_to_string(temp.0.join("destination")).unwrap(),
        "keep"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("source")).unwrap(),
        "source"
    );
}

#[test]
fn apply_can_skip_conflicting_moves_and_apply_independent_moves() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("conflicting"), "keep source").expect("write conflicting source");
    std::fs::write(temp.0.join("occupied"), "keep destination")
        .expect("write occupied destination");
    std::fs::write(temp.0.join("independent"), "move me").expect("write independent source");
    let conflict = Conflict {
        path: PathBuf::from("occupied"),
        reason: "destination already exists".into(),
    };
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("conflicting"),
                destination: PathBuf::from("occupied"),
                kind: EntryKind::File,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("independent"),
                destination: PathBuf::from("renamed"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: vec![conflict.clone()],
        findings: Vec::new(),
    };

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let report = apply_with_conflict_policy(&temp.0, &plan, &snapshot, ConflictPolicy::Skip)
        .expect("apply independent move");

    assert_eq!(report.moved.len(), 1);
    assert_eq!(report.skipped, vec![conflict]);
    assert!(temp.0.join("conflicting").is_file());
    assert_eq!(
        std::fs::read_to_string(temp.0.join("occupied")).unwrap(),
        "keep destination"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("renamed")).unwrap(),
        "move me"
    );
}

#[test]
fn apply_skips_descendants_of_conflicted_directories() {
    let temp = TempDir::new();
    std::fs::create_dir(temp.0.join("old")).expect("create source directory");
    std::fs::write(temp.0.join("old/file"), "keep child").expect("write source child");
    std::fs::create_dir(temp.0.join("occupied")).expect("create occupied directory");
    std::fs::create_dir(temp.0.join("redirected")).expect("create redirect directory");
    std::fs::write(temp.0.join("independent"), "move me").expect("write independent source");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("old"),
                destination: PathBuf::from("occupied"),
                kind: EntryKind::Directory,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("old/file"),
                destination: PathBuf::from("redirected/file"),
                kind: EntryKind::File,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("independent"),
                destination: PathBuf::from("occupied/independent"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: vec![Conflict {
            path: PathBuf::from("occupied"),
            reason: "destination already exists".into(),
        }],
        findings: Vec::new(),
    };

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let report = apply_with_conflict_policy(&temp.0, &plan, &snapshot, ConflictPolicy::Skip)
        .expect("apply independent move");

    assert_eq!(report.moved.len(), 1);
    assert_eq!(report.skipped.len(), 1);
    assert!(temp.0.join("old/file").is_file());
    assert!(!temp.0.join("occupied/file").exists());
    assert!(!temp.0.join("redirected/file").exists());
    assert!(temp.0.join("occupied/independent").is_file());
}

#[test]
fn apply_preflights_all_destinations_before_moving_anything() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("first"), "first").expect("write first source");
    std::fs::write(temp.0.join("second"), "second").expect("write second source");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("first-renamed"),
                kind: EntryKind::File,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("second"),
                destination: PathBuf::from("second-renamed"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };
    std::fs::write(temp.0.join("second-renamed"), "occupied")
        .expect("create destination after planning");

    assert!(apply(&temp.0, &plan, &snapshot).is_err());
    assert!(temp.0.join("first").is_file());
    assert!(!temp.0.join("first-renamed").exists());
    assert!(temp.0.join("second").is_file());
}

#[test]
fn apply_rejects_a_source_changed_after_planning() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("source"), "old").expect("write source");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    std::fs::write(temp.0.join("source"), "replacement").expect("replace source contents");
    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("source"),
            destination: PathBuf::from("renamed"),
            kind: EntryKind::File,
            rule: None,
        }],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };

    assert!(apply(&temp.0, &plan, &snapshot).is_err());
    assert!(temp.0.join("source").is_file());
    assert!(!temp.0.join("renamed").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn apply_rejects_a_replaced_source_with_same_size_and_timestamp() {
    let temp = TempDir::new();
    let source = temp.0.join("source");
    let replacement = temp.0.join("replacement");
    std::fs::write(&source, "Original").expect("write source");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let modified = std::fs::metadata(&source)
        .expect("inspect source")
        .modified()
        .expect("read source timestamp");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let metadata = std::fs::metadata(&source).expect("inspect Windows source identity");
        if metadata.volume_serial_number().is_none() || metadata.file_index().is_none() {
            return;
        }
    }

    std::fs::write(&replacement, "Replaced").expect("write replacement");
    std::fs::File::open(&replacement)
        .expect("open replacement")
        .set_modified(modified)
        .expect("preserve source timestamp");
    std::fs::remove_file(&source).expect("remove original file");
    std::fs::rename(&replacement, &source).expect("install replacement at source path");

    let plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("source"),
            destination: PathBuf::from("renamed"),
            kind: EntryKind::File,
            rule: None,
        }],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };
    let error = apply(&temp.0, &plan, &snapshot).expect_err("reject replaced source");

    assert!(matches!(error, ApplyError::ChangedSource(_)));
    assert_eq!(
        std::fs::read_to_string(&source).expect("read replacement"),
        "Replaced"
    );
    assert!(!temp.0.join("renamed").exists());
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn apply_supports_case_only_renames_on_case_insensitive_filesystems() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("README"), "project notes").expect("write source file");
    if !temp.0.join("readme").exists() {
        return;
    }

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let mut plan = Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from("README"),
            destination: PathBuf::from("readme"),
            kind: EntryKind::File,
            rule: None,
        }],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };
    validate_destinations(&temp.0, &mut plan).expect("validate case-only rename");
    assert!(plan.is_conflict_free());

    let report = apply(&temp.0, &plan, &snapshot).expect("apply case-only rename");

    assert_eq!(report.moved.len(), 1);
    assert_eq!(
        std::fs::read_dir(&temp.0)
            .expect("read target directory")
            .map(|entry| entry.expect("read target entry").file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from("readme")]
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("readme")).expect("read renamed file"),
        "project notes"
    );
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[test]
fn apply_supports_case_only_directory_renames_with_changed_children() {
    let temp = TempDir::new();
    std::fs::create_dir(temp.0.join("HTTPServer")).expect("create source directory");
    std::fs::write(temp.0.join("HTTPServer/README"), "project notes").expect("write source file");
    if !temp.0.join("httpserver/readme").exists() {
        return;
    }

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let mut plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("HTTPServer"),
                destination: PathBuf::from("httpserver"),
                kind: EntryKind::Directory,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("HTTPServer/README"),
                destination: PathBuf::from("httpserver/readme"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };
    validate_destinations(&temp.0, &mut plan).expect("validate case-only directory rename");
    assert!(plan.is_conflict_free());

    let report = apply(&temp.0, &plan, &snapshot).expect("apply case-only directory rename");

    assert_eq!(report.moved.len(), 2);
    assert_eq!(
        std::fs::read_dir(&temp.0)
            .expect("read target directory")
            .map(|entry| entry.expect("read target entry").file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from("httpserver")]
    );
    assert_eq!(
        std::fs::read_dir(temp.0.join("httpserver"))
            .expect("read renamed directory")
            .map(|entry| entry.expect("read renamed entry").file_name())
            .collect::<Vec<_>>(),
        vec![std::ffi::OsString::from("readme")]
    );
}

#[test]
fn apply_checks_every_source_before_moving_anything() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("first"), "first").expect("write first source");
    std::fs::write(temp.0.join("second"), "second").expect("write second source");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("first-renamed"),
                kind: EntryKind::File,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("second"),
                destination: PathBuf::from("second-renamed"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };
    std::fs::write(temp.0.join("second"), "changed after scan").expect("change second source");

    assert!(apply(&temp.0, &plan, &snapshot).is_err());
    assert!(temp.0.join("first").is_file());
    assert!(!temp.0.join("first-renamed").exists());
    assert!(temp.0.join("second").is_file());
    assert!(!temp.0.join("second-renamed").exists());
}

#[cfg(unix)]
#[test]
fn apply_rolls_back_prior_moves_when_a_later_move_fails() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new();
    std::fs::write(temp.0.join("first"), "first").expect("write first source");
    std::fs::write(temp.0.join("second"), "second").expect("write second source");
    std::fs::create_dir(temp.0.join("blocked")).expect("create blocked destination parent");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("created/first-renamed"),
                kind: EntryKind::File,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("second"),
                destination: PathBuf::from("blocked/second-renamed"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        conflicts: Vec::new(),
        findings: Vec::new(),
    };
    std::fs::set_permissions(
        temp.0.join("blocked"),
        std::fs::Permissions::from_mode(0o555),
    )
    .expect("make destination parent read-only");

    let result = apply(&temp.0, &plan, &snapshot);

    std::fs::set_permissions(
        temp.0.join("blocked"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("restore destination parent permissions");
    assert!(result.is_err(), "later move should fail");
    assert!(temp.0.join("first").is_file());
    assert!(!temp.0.join("created").exists());
    assert!(temp.0.join("second").is_file());
    assert!(!temp.0.join("blocked/second-renamed").exists());
}

#[test]
fn scan_requires_directory_target() {
    assert!(scan(Path::new("Cargo.toml")).is_err());
}

#[cfg(unix)]
#[test]
fn scan_skips_special_files() {
    use std::os::unix::net::UnixListener;

    let temp = TempDir::new();
    let socket_path = temp.0.join("service.sock");
    let _listener = UnixListener::bind(&socket_path).expect("bind Unix socket");

    let result = scan(&temp.0).expect("scan target");

    assert!(
        result
            .entries
            .iter()
            .all(|entry| entry.path.as_path() != Path::new("service.sock"))
    );
    assert!(
        result
            .findings
            .iter()
            .any(|finding| finding.contains("non-regular filesystem entry"))
    );
}

#[test]
fn scan_fails_when_an_ignore_file_is_malformed() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join(".ignore"), "[z-a]\n").expect("write malformed ignore file");

    let error = scan(&temp.0).expect_err("malformed ignore rules must fail scanning");

    assert!(error.to_string().contains("cannot scan target"));
}
