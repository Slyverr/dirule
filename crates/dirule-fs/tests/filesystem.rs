use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry, parse_config, plan};
use dirule_fs::validate_destinations;
use dirule_fs::{ApplyError, ConflictPolicy, ScanError, apply, apply_with_conflict_policy, scan};
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
    std::fs::create_dir(temp.0.join("nested")).expect("create nested directory");
    std::fs::write(temp.0.join("nested/.ignore"), "ignored/\n").expect("write nested ignore file");
    std::fs::create_dir(temp.0.join("nested/ignored")).expect("create nested ignored directory");
    std::fs::write(temp.0.join("nested/ignored/file.txt"), "ignored")
        .expect("write nested ignored file");

    let result = scan(&temp.0).expect("scan target");
    let paths = result
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(paths.contains(&PathBuf::from(".hidden")));
    assert!(!paths.iter().any(|path| path.starts_with("ignored")));
    assert!(!paths.iter().any(|path| path.starts_with("nested/ignored")));
}

#[cfg(unix)]
#[test]
fn scan_handles_git_repository_roots_with_newlines() {
    let temp = TempDir::new();
    let target = temp.0.join("repo\nwith-newline\r");
    std::fs::create_dir(&target).expect("create repository root");
    std::fs::write(target.join(".gitignore"), "tracked.txt\n").expect("write ignore rule");
    std::fs::write(target.join("tracked.txt"), "content").expect("write tracked fixture");

    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target)
        .status()
        .expect("initialize Git fixture");
    assert!(init.success(), "initialize Git fixture");
    let add_ignore = Command::new("git")
        .args(["add", ".gitignore"])
        .current_dir(&target)
        .status()
        .expect("stage ignore rule");
    assert!(add_ignore.success(), "stage ignore rule");
    let add_tracked = Command::new("git")
        .args(["add", "-f", "tracked.txt"])
        .current_dir(&target)
        .status()
        .expect("stage ignored file");
    assert!(add_tracked.success(), "stage ignored file");

    let result = scan(&target).expect("scan repository root containing newline");

    assert!(
        result
            .entries
            .iter()
            .any(|entry| entry.path == Path::new("tracked.txt"))
    );
}

#[test]
fn scan_respects_configured_global_gitignore() {
    run_global_gitignore_child("globally-ignored.txt\n", "scan_global_gitignore_child");
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
fn scan_fails_when_configured_global_gitignore_is_malformed() {
    run_global_gitignore_child("[z-a]\n", "scan_malformed_global_gitignore_child");
}

fn run_global_gitignore_child(ignore_rules: &str, test_name: &str) {
    let temp = TempDir::new();
    let target = temp.0.join("target");
    std::fs::create_dir(&target).expect("create scan target");
    std::fs::write(target.join("globally-ignored.txt"), "ignored")
        .expect("write globally ignored file");

    let home = temp.0.join("home");
    std::fs::create_dir(&home).expect("create isolated home directory");
    let xdg_config = temp.0.join("xdg-config");
    let git_config_dir = xdg_config.join("git");
    std::fs::create_dir_all(&git_config_dir).expect("create isolated Git config directory");
    std::fs::write(git_config_dir.join("ignore"), ignore_rules).expect("write global ignore rules");

    let output = Command::new(std::env::current_exe().expect("test executable path"))
        .args(["--exact", test_name, "--nocapture"])
        .env("GIT_CONFIG_GLOBAL", "")
        .env("GIT_CONFIG_SYSTEM", "")
        .env("HOME", &home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", xdg_config)
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
fn scan_malformed_global_gitignore_child() {
    let Some(target) = std::env::var_os("DIRULE_GLOBAL_IGNORE_TARGET") else {
        return;
    };
    let error = scan(Path::new(&target)).expect_err("malformed global ignore rules must fail");

    assert!(matches!(error, ScanError::Walk { .. }));
}

#[test]
fn scan_limits_tracked_git_paths_to_targets_with_literal_characters() {
    let temp = TempDir::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&temp.0)
        .output();
    let Ok(init) = init else {
        return;
    };
    assert!(init.status.success(), "initialize Git fixture");

    let target = temp.0.join("dir[1]");
    std::fs::create_dir_all(&target).expect("create target directory");
    std::fs::write(target.join(".gitignore"), "kept.txt\n").expect("write target ignore rule");
    std::fs::write(target.join("kept.txt"), "tracked and ignored").expect("write target file");
    std::fs::create_dir(temp.0.join("dir1")).expect("create glob-like sibling");
    std::fs::write(temp.0.join("dir1/unrelated.txt"), "outside target")
        .expect("write unrelated file");

    let add_rules = Command::new("git")
        .args(["add", "--", "dir[1]/.gitignore"])
        .current_dir(&temp.0)
        .status()
        .expect("stage target ignore rules");
    assert!(add_rules.success(), "stage target ignore rules");
    let add_ignored = Command::new("git")
        .args(["add", "-f", "--", "dir[1]/kept.txt"])
        .current_dir(&temp.0)
        .status()
        .expect("force-stage ignored target file");
    assert!(add_ignored.success(), "force-stage ignored target file");

    let result = scan(&target).expect("scan nested target with pathspec characters");
    let paths = result
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();

    assert!(paths.contains(&PathBuf::from("kept.txt")));
    assert!(!paths.contains(&PathBuf::from("unrelated.txt")));
}

#[cfg(unix)]
#[test]
fn scan_fails_when_repository_excludes_are_unreadable() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&temp.0)
        .output();
    let Ok(init) = init else {
        return;
    };
    assert!(init.status.success(), "initialize Git fixture");

    let exclude = temp.0.join(".git/info/exclude");
    std::fs::write(&exclude, "ignored/\n").expect("write repository excludes");
    std::fs::set_permissions(&exclude, std::fs::Permissions::from_mode(0o000))
        .expect("remove repository exclude read permissions");
    if std::fs::read(&exclude).is_ok() {
        return;
    }

    let error = scan(&temp.0).expect_err("unreadable repository excludes must fail scanning");

    assert!(error.to_string().contains("exclude"));
}

#[test]
fn scan_git_missing_child() {
    let Some(target) = std::env::var_os("DIRULE_GIT_MISSING_TARGET") else {
        return;
    };
    let result = scan(Path::new(&target));

    assert!(matches!(
        result,
        Err(ScanError::Git(message)) if message.contains("Git is required to include tracked paths")
    ));
}

#[test]
fn scan_fails_when_git_is_unavailable_inside_a_repository() {
    let temp = TempDir::new();
    let target = temp.0.join("target");
    std::fs::create_dir(&target).expect("create scan target");
    std::fs::create_dir(temp.0.join(".git")).expect("create repository marker");

    let empty_bin = temp.0.join("empty-bin");
    std::fs::create_dir(&empty_bin).expect("create empty PATH directory");
    let output = Command::new(std::env::current_exe().expect("test executable path"))
        .args(["--exact", "scan_git_missing_child", "--nocapture"])
        .env("DIRULE_GIT_MISSING_TARGET", &target)
        .env("PATH", empty_bin)
        .output()
        .expect("run scanner without Git");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
fn scan_applies_repository_ignore_rules_to_nested_targets() {
    let temp = TempDir::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&temp.0)
        .status()
        .expect("initialize Git fixture");
    assert!(init.success(), "initialize Git fixture");

    let target = temp.0.join("target");
    std::fs::create_dir(&target).expect("create nested target");
    std::fs::write(
        temp.0.join(".gitignore"),
        "/target/ignored-tracked.txt\n/target/ignored-untracked.txt\n",
    )
    .expect("write repository ignore rule");
    std::fs::write(target.join("ignored-tracked.txt"), "tracked despite ignore")
        .expect("write tracked ignored file");
    std::fs::write(
        target.join("ignored-untracked.txt"),
        "excluded by parent rule",
    )
    .expect("write untracked ignored file");
    std::fs::write(target.join("visible.txt"), "included").expect("write visible file");

    let add_rules = Command::new("git")
        .args(["add", ".gitignore"])
        .current_dir(&temp.0)
        .status()
        .expect("stage repository ignore rules");
    assert!(add_rules.success(), "stage repository ignore rules");
    let add_ignored = Command::new("git")
        .args(["add", "-f", "--", "target/ignored-tracked.txt"])
        .current_dir(&temp.0)
        .status()
        .expect("force-stage ignored file");
    assert!(add_ignored.success(), "force-stage ignored file");

    let result = scan(&target).expect("scan nested target");
    let paths = result
        .entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();

    assert!(paths.contains(&PathBuf::from("ignored-tracked.txt")));
    assert!(!paths.contains(&PathBuf::from("ignored-untracked.txt")));
    assert!(paths.contains(&PathBuf::from("visible.txt")));
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
to = "template:renamed-parent"

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
    assert!(temp.0.join("renamed-parent").is_dir());
    assert_eq!(
        std::fs::read_to_string(temp.0.join("Parent/child.txt")).unwrap(),
        "content"
    );
    assert!(!temp.0.join("renamed-parent/child.txt").exists());
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
fn apply_skips_moves_that_depend_on_a_conflicting_move() {
    let temp = TempDir::new();
    for (path, contents) in [
        ("first", "first"),
        ("second", "second"),
        ("final", "keep"),
        ("independent", "independent"),
    ] {
        std::fs::write(temp.0.join(path), contents).expect("write test file");
    }
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: [
            ("first", "second"),
            ("second", "final"),
            ("independent", "renamed"),
        ]
        .into_iter()
        .map(|(source, destination)| PlannedEntry {
            source: PathBuf::from(source),
            destination: PathBuf::from(destination),
            kind: EntryKind::File,
            rule: None,
        })
        .collect(),
        ..Plan::default()
    };
    let mut validated_plan = plan.clone();

    validate_destinations(&temp.0, &mut validated_plan).expect("validate dependent moves");
    assert_eq!(
        validated_plan
            .conflicts
            .iter()
            .map(|conflict| conflict.path.as_path())
            .collect::<Vec<_>>(),
        [Path::new("final"), Path::new("second")]
    );

    let report = apply_with_conflict_policy(&temp.0, &plan, &snapshot, ConflictPolicy::Skip)
        .expect("apply unrelated move");

    assert_eq!(report.moved.len(), 1);
    assert_eq!(report.skipped.len(), 2);
    assert_eq!(
        std::fs::read_to_string(temp.0.join("first")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("second")).unwrap(),
        "second"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("final")).unwrap(),
        "keep"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("renamed")).unwrap(),
        "independent"
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
fn apply_rotates_files_in_a_rename_cycle() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("first"), "first").expect("write first source");
    std::fs::write(temp.0.join("second"), "second").expect("write second source");
    std::fs::write(temp.0.join("third"), "third").expect("write third source");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("second"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
            PlannedEntry {
                source: PathBuf::from("second"),
                destination: PathBuf::from("third"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
            PlannedEntry {
                source: PathBuf::from("third"),
                destination: PathBuf::from("first"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
        ],
        ..Plan::default()
    };

    let report = apply(&temp.0, &plan, &snapshot).expect("apply file rotation");

    assert_eq!(report.moved.len(), 3);
    assert_eq!(
        std::fs::read_to_string(temp.0.join("first")).unwrap(),
        "third"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("second")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("third")).unwrap(),
        "second"
    );
}

#[test]
fn apply_moves_sources_along_a_dependency_chain() {
    let temp = TempDir::new();
    for (path, contents) in [("first", "first"), ("second", "second"), ("third", "third")] {
        std::fs::write(temp.0.join(path), contents).expect("write chain source");
    }
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: [("first", "second"), ("second", "third"), ("third", "final")]
            .into_iter()
            .map(|(source, destination)| PlannedEntry {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
                kind: EntryKind::File,
                rule: Some("chain".into()),
            })
            .collect(),
        ..Plan::default()
    };

    apply(&temp.0, &plan, &snapshot).expect("apply move chain");

    assert_eq!(
        std::fs::read_to_string(temp.0.join("second")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("third")).unwrap(),
        "second"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("final")).unwrap(),
        "third"
    );
    assert!(!temp.0.join("first").exists());
}

#[test]
fn apply_swaps_directories_with_their_contents() {
    let temp = TempDir::new();
    std::fs::create_dir(temp.0.join("first")).expect("create first directory");
    std::fs::create_dir(temp.0.join("second")).expect("create second directory");
    std::fs::write(temp.0.join("first/file"), "first").expect("write first contents");
    std::fs::write(temp.0.join("second/file"), "second").expect("write second contents");
    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("second"),
                kind: EntryKind::Directory,
                rule: Some("swap".into()),
            },
            PlannedEntry {
                source: PathBuf::from("second"),
                destination: PathBuf::from("first"),
                kind: EntryKind::Directory,
                rule: Some("swap".into()),
            },
        ],
        ..Plan::default()
    };

    apply(&temp.0, &plan, &snapshot).expect("apply directory swap");

    assert_eq!(
        std::fs::read_to_string(temp.0.join("first/file")).unwrap(),
        "second"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("second/file")).unwrap(),
        "first"
    );
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

#[cfg(unix)]
#[test]
fn apply_rejects_a_replaced_target_when_source_identity_is_preserved() {
    let temp = TempDir::new();
    let target = temp.0.join("target");
    let displaced = temp.0.join("displaced");
    std::fs::create_dir(&target).expect("create target");
    std::fs::write(target.join("source"), "content").expect("write source");
    let snapshot = scan(&target).expect("capture apply snapshot");
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

    std::fs::rename(&target, &displaced).expect("displace original target");
    std::fs::create_dir(&target).expect("create replacement target");
    std::fs::hard_link(displaced.join("source"), target.join("source"))
        .expect("preserve source identity in replacement target");

    let result = apply(&target, &plan, &snapshot);

    assert!(matches!(result, Err(ApplyError::ChangedTarget(_))));
    assert!(target.join("source").is_file());
    assert!(!target.join("renamed").exists());
    assert!(displaced.join("source").is_file());
}

#[cfg(windows)]
#[test]
fn destinations_reject_names_that_win32_reinterprets_or_disallows() {
    let temp = TempDir::new();
    for name in [
        "bad:name.txt",
        "bad?.txt",
        "trailing.",
        "trailing ",
        "CON",
        "nul.txt",
        "COM9.log",
        "LPT³",
        "control\u{0001}name",
    ] {
        let mut plan = Plan {
            entries: vec![PlannedEntry {
                source: PathBuf::from("source.txt"),
                destination: PathBuf::from(name),
                kind: EntryKind::File,
                rule: None,
            }],
            ..Plan::default()
        };

        assert!(
            matches!(
                validate_destinations(&temp.0, &mut plan),
                Err(ApplyError::UnsafePath(_))
            ),
            "expected Windows to reject {name:?}"
        );
    }

    for name in [".hidden", "COM10.log", "console.txt"] {
        let mut plan = Plan {
            entries: vec![PlannedEntry {
                source: PathBuf::from("source.txt"),
                destination: PathBuf::from(name),
                kind: EntryKind::File,
                rule: None,
            }],
            ..Plan::default()
        };

        assert!(
            validate_destinations(&temp.0, &mut plan).is_ok(),
            "expected Windows to accept {name:?}"
        );
    }
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
    std::fs::write(&replacement, "Replaced").expect("write replacement");
    std::fs::OpenOptions::new()
        .write(true)
        .open(&replacement)
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
fn apply_stages_case_insensitive_move_dependencies() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("first"), "first").expect("write first source");
    std::fs::write(temp.0.join("Second"), "second").expect("write second source");
    if !temp.0.join("second").exists() {
        return;
    }

    let snapshot = scan(&temp.0).expect("capture apply snapshot");
    let plan = Plan {
        entries: vec![
            PlannedEntry {
                source: PathBuf::from("first"),
                destination: PathBuf::from("second"),
                kind: EntryKind::File,
                rule: None,
            },
            PlannedEntry {
                source: PathBuf::from("Second"),
                destination: PathBuf::from("final"),
                kind: EntryKind::File,
                rule: None,
            },
        ],
        ..Plan::default()
    };

    apply(&temp.0, &plan, &snapshot).expect("apply case-insensitive dependency");

    assert_eq!(
        std::fs::read_to_string(temp.0.join("second")).unwrap(),
        "first"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("final")).unwrap(),
        "second"
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
fn scan_sorts_findings_for_skipped_special_files() {
    use std::os::unix::net::UnixListener;

    let temp = TempDir::new();
    let _listeners = ["z.sock", "m.sock", "a.sock"]
        .map(|name| UnixListener::bind(temp.0.join(name)).expect("bind Unix socket fixture"));

    let result = scan(&temp.0).expect("scan target");

    assert!(result.entries.iter().all(|entry| {
        entry
            .path
            .extension()
            .is_none_or(|extension| extension != "sock")
    }));
    assert_eq!(result.findings.len(), 3);
    assert!(result.findings.is_sorted());
}

#[cfg(target_os = "linux")]
#[test]
fn scan_skips_and_reports_non_unicode_paths() {
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new();
    let name = std::ffi::OsString::from_vec(vec![b'f', 0xff]);
    std::fs::write(temp.0.join(name), "not representable in config paths")
        .expect("write non-Unicode filename");

    let result = scan(&temp.0).expect("scan target");

    assert!(result.entries.is_empty());
    assert_eq!(result.findings.len(), 1);
    assert!(result.findings[0].contains("cannot be represented as Unicode"));
}

#[test]
fn scan_fails_when_an_ignore_file_is_malformed() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join(".ignore"), "[z-a]\n").expect("write malformed ignore file");

    let error = scan(&temp.0).expect_err("malformed ignore rules must fail scanning");

    assert!(error.to_string().contains("cannot scan target"));
}

#[cfg(unix)]
#[test]
fn scan_fails_when_an_ignore_file_is_unreadable() {
    use std::os::unix::fs::PermissionsExt;

    for relative in [Path::new(".ignore"), Path::new("nested/.gitignore")] {
        let temp = TempDir::new();
        let ignore_file = temp.0.join(relative);
        let ignore_directory = relative.parent().unwrap_or(Path::new(""));
        std::fs::create_dir_all(temp.0.join(ignore_directory))
            .expect("create ignore file directory");
        std::fs::write(&ignore_file, "ignored/\n").expect("write ignore file");
        std::fs::create_dir_all(temp.0.join(ignore_directory).join("ignored"))
            .expect("create ignored directory");
        std::fs::set_permissions(&ignore_file, std::fs::Permissions::from_mode(0o000))
            .expect("remove ignore file read permissions");

        if std::fs::read(&ignore_file).is_ok() {
            continue;
        }

        let error = scan(&temp.0).expect_err("unreadable ignore rules must fail scanning");

        assert!(error.to_string().contains("ignore"));
    }
}
