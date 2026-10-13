use dirule_core::{EntryKind, Plan, PlannedEntry};
use dirule_fs::{ConflictPolicy, JournalStatus, JournalStore, apply_journaled, scan};
use fs4::FileExt;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dirule-journal-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn one_file_plan(source: &str, destination: &str) -> Plan {
    Plan {
        entries: vec![PlannedEntry {
            source: PathBuf::from(source),
            destination: PathBuf::from(destination),
            kind: EntryKind::File,
            rule: Some("test".into()),
        }],
        ..Plan::default()
    }
}

fn store(temp: &TempDir) -> JournalStore {
    JournalStore::new(temp.0.join("state/runs"))
}

fn journaled_file_run() -> (TempDir, PathBuf, JournalStore, String, PathBuf) {
    journaled_run(&[("before.txt", "after.txt")])
}

#[test]
fn run_details_include_target_relative_rename_operations() {
    let (_temp, _target, store, run_id, _journal) = journaled_file_run();

    let details = store.get(&run_id).expect("read run details");

    assert_eq!(details.run.id, run_id);
    assert_eq!(details.run.operations, 1);
    assert_eq!(details.operations.len(), 1);
    assert_eq!(details.operations[0].source, PathBuf::from("before.txt"));
    assert_eq!(
        details.operations[0].destination,
        PathBuf::from("after.txt")
    );
    assert_eq!(details.operations[0].kind, dirule_core::EntryKind::File);
}

#[cfg(unix)]
#[test]
fn journal_state_uses_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let (temp, _target, _store, _run_id, journal) = journaled_file_run();
    let lock = journal.with_extension("lock");
    let state_directory = temp.0.join("state");
    let runs_directory = state_directory.join("runs");

    assert_eq!(
        std::fs::metadata(&state_directory)
            .expect("read state directory permissions")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&runs_directory)
            .expect("read runs directory permissions")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    for path in [&journal, &lock] {
        assert_eq!(
            std::fs::metadata(path)
                .expect("read journal state file permissions")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "{} should be accessible only by its owner",
            path.display()
        );
    }
}

#[cfg(unix)]
#[test]
fn journal_state_rejects_symlinked_private_directories_before_mutation() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new();
    let target = temp.0.join("project");
    let state = temp.0.join("state");
    let outside = temp.0.join("outside");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    std::fs::create_dir(&state).expect("create state base");
    std::fs::create_dir(&outside).expect("create external directory");
    symlink(&outside, state.join("dirule")).expect("link private state directory");

    let snapshot = scan(&target).expect("scan project");
    let store = JournalStore::new(state.join("dirule/runs"));
    let result = apply_journaled(
        &target,
        &one_file_plan("before.txt", "after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    );

    assert!(result.is_err());
    assert_eq!(
        std::fs::read(target.join("before.txt")).expect("read original file"),
        b"contents"
    );
    assert!(!target.join("after.txt").exists());
    assert!(!outside.join("runs").exists());
}

fn journaled_run(files: &[(&str, &str)]) -> (TempDir, PathBuf, JournalStore, String, PathBuf) {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    for (source, _) in files {
        std::fs::write(target.join(source), "contents").expect("write source file");
    }
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let plan = Plan {
        entries: files
            .iter()
            .map(|(source, destination)| PlannedEntry {
                source: PathBuf::from(source),
                destination: PathBuf::from(destination),
                kind: EntryKind::File,
                rule: Some("test".into()),
            })
            .collect(),
        ..Plan::default()
    };
    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));

    (temp, target, store, run_id, journal)
}

fn simulate_interrupted_move(journal: &Path) {
    let retained = std::fs::read_to_string(journal)
        .expect("read journal")
        .lines()
        .filter(|line| {
            !line.contains("\"record\":\"move_completed\"")
                && !line.contains("\"record\":\"status\"")
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(journal, format!("{retained}\n")).expect("simulate a crash after move intent");
}

fn remove_final_status_record(journal: &Path) {
    let mut records = std::fs::read_to_string(journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let final_status = records
        .iter()
        .rposition(|line| line.contains("\"record\":\"status\""))
        .expect("final status record");
    records.remove(final_status);

    std::fs::write(journal, format!("{}\n", records.join("\n")))
        .expect("simulate a crash before final status");
}

fn retain_first_completed_move(journal: &Path) -> PathBuf {
    let records = std::fs::read_to_string(journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut retained = Vec::with_capacity(3);
    let mut sequence = None;
    let mut destination = None;

    for record in records {
        let value: serde_json::Value = serde_json::from_str(&record).expect("parse journal record");
        match value["record"].as_str() {
            Some("header") => retained.push(record),
            Some("move_intent") if sequence.is_none() => {
                sequence = value["sequence"].as_u64();
                destination = Some(
                    serde_json::from_value(value["destination"].clone())
                        .expect("decode first move destination"),
                );
                retained.push(record);
            }
            Some("move_completed") if value["sequence"].as_u64() == sequence => {
                retained.push(record);
                break;
            }
            _ => break,
        }
    }

    assert!(sequence.is_some(), "journal contains a move intent");
    assert!(
        retained
            .last()
            .is_some_and(|record| record.contains("\"record\":\"move_completed\"")),
        "first move has a completion record"
    );
    std::fs::write(journal, format!("{}\n", retained.join("\n")))
        .expect("simulate a crash after first completed move");

    destination.expect("first move destination")
}

fn simulate_interrupted_undo(journal: &Path) {
    let mut records = std::fs::read_to_string(journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let completion = records
        .iter()
        .rposition(|line| line.contains("\"record\":\"move_completed\""))
        .expect("undo move completion record");
    records.remove(completion);
    let final_status = records
        .iter()
        .rposition(|line| line.contains("\"record\":\"status\""))
        .expect("final status record");
    records.remove(final_status);

    std::fs::write(journal, format!("{}\n", records.join("\n")))
        .expect("simulate a crash after reverse rename");
}

#[test]
fn journaled_apply_can_be_listed_and_undone() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let plan = one_file_plan("before.txt", "after.txt");
    let store = store(&temp);

    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    assert!(!target.join("before.txt").exists());
    assert_eq!(
        std::fs::read(target.join("after.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(store.list().unwrap()[0].id, run_id);

    store.undo(&run_id).expect("undo journaled apply");

    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"contents"
    );
    assert!(!target.join("after.txt").exists());
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn journaled_file_swap_can_be_undone() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("first"), "first").expect("write first source");
    std::fs::write(target.join("second"), "second").expect("write second source");
    let snapshot = scan(&target).expect("scan project");
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
                destination: PathBuf::from("first"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
        ],
        ..Plan::default()
    };
    let store = store(&temp);

    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply journaled swap");
    let run_id = applied.run_id.expect("mutation creates a journal run");

    assert_eq!(std::fs::read(target.join("first")).unwrap(), b"second");
    assert_eq!(std::fs::read(target.join("second")).unwrap(), b"first");

    remove_final_status_record(&temp.0.join(format!("state/runs/{run_id}.jsonl")));
    store.undo(&run_id).expect("undo journaled swap");

    assert_eq!(std::fs::read(target.join("first")).unwrap(), b"first");
    assert_eq!(std::fs::read(target.join("second")).unwrap(), b"second");
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn interrupted_cycle_staging_can_be_undone() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("first"), "first").expect("write first source");
    std::fs::write(target.join("second"), "second").expect("write second source");
    let snapshot = scan(&target).expect("scan project");
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
                destination: PathBuf::from("first"),
                kind: EntryKind::File,
                rule: Some("swap".into()),
            },
        ],
        ..Plan::default()
    };
    let store = store(&temp);

    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply journaled swap");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    let staged_path = target.join(retain_first_completed_move(&journal));

    std::fs::rename(target.join("second"), &staged_path).expect("restore staged first source");
    std::fs::rename(target.join("first"), target.join("second"))
        .expect("restore second source path");

    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Interrupted);
    store
        .undo(&run_id)
        .expect("undo the completed staging move");

    assert_eq!(std::fs::read(target.join("first")).unwrap(), b"first");
    assert_eq!(std::fs::read(target.join("second")).unwrap(), b"second");
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn journaled_directory_swap_can_be_undone() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir_all(target.join("first")).expect("create first directory");
    std::fs::create_dir_all(target.join("second")).expect("create second directory");
    std::fs::write(target.join("first/file"), "first").expect("write first contents");
    std::fs::write(target.join("second/file"), "second").expect("write second contents");
    let snapshot = scan(&target).expect("scan project");
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
    let store = store(&temp);

    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply journaled directory swap");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    assert_eq!(std::fs::read(target.join("first/file")).unwrap(), b"second");
    assert_eq!(std::fs::read(target.join("second/file")).unwrap(), b"first");

    store.undo(&run_id).expect("undo journaled directory swap");

    assert_eq!(std::fs::read(target.join("first/file")).unwrap(), b"first");
    assert_eq!(
        std::fs::read(target.join("second/file")).unwrap(),
        b"second"
    );
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn partially_recovered_run_is_listed_and_can_be_undone() {
    let (_temp, target, store, run_id, journal) = journaled_file_run();
    let contents = std::fs::read_to_string(&journal).expect("read applied journal");
    let partially_recovered = contents.replace(
        r#"{"record":"status","status":"applied"}"#,
        r#"{"record":"status","status":"partially_recovered"}"#,
    );
    assert_ne!(contents, partially_recovered, "replace final run status");
    std::fs::write(&journal, partially_recovered).expect("mark run partially recovered");

    let runs = store.list().expect("list partially recovered run");
    assert_eq!(runs[0].id, run_id);
    assert_eq!(runs[0].status, JournalStatus::PartiallyRecovered);
    store.undo(&run_id).expect("undo partially recovered run");

    assert_eq!(
        std::fs::read(target.join("before.txt")).expect("read restored file"),
        b"contents"
    );
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn active_run_lock_blocks_reading_and_undo_without_mutation() {
    let (_temp, target, store, run_id, journal) = journaled_file_run();
    let lock_path = journal.with_extension("lock");
    let mut child = Command::new(std::env::current_exe().expect("test executable path"))
        .args(["--exact", "journal_lock_helper_process", "--nocapture"])
        .env("DIRULE_JOURNAL_LOCK_PATH", lock_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn lock holder process");
    let mut output = BufReader::new(child.stdout.take().expect("capture helper output"));
    let mut line = String::new();
    let mut lock_acquired = false;
    loop {
        line.clear();
        if output.read_line(&mut line).expect("read helper output") == 0 {
            break;
        }
        if line.trim() == "LOCKED" {
            lock_acquired = true;
            break;
        }
    }

    let list = store.list();
    let details = store.get(&run_id);
    let undo = store.undo(&run_id);
    let _ = child
        .stdin
        .take()
        .expect("open helper input")
        .write_all(b"release\n");
    let status = child.wait().expect("wait for lock holder process");

    assert!(lock_acquired, "helper process did not acquire its lock");
    assert!(status.success(), "lock holder process failed");
    assert!(matches!(list, Err(dirule_fs::JournalError::RunBusy(id)) if id == run_id));
    assert!(matches!(details, Err(dirule_fs::JournalError::RunBusy(id)) if id == run_id));
    assert!(matches!(undo, Err(dirule_fs::JournalError::RunBusy(id)) if id == run_id));
    assert!(target.join("after.txt").exists());
    assert!(!target.join("before.txt").exists());
}

#[test]
fn journal_lock_helper_process() {
    let Some(path) = std::env::var_os("DIRULE_JOURNAL_LOCK_PATH") else {
        return;
    };
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open run lock");
    FileExt::try_lock(&lock).expect("lock run exclusively");
    writeln!(std::io::stdout().lock(), "LOCKED").expect("signal lock acquired");
    std::io::stdout().flush().expect("flush lock signal");

    let mut release = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut release)
        .expect("wait for release signal");
}

#[test]
fn missing_run_reads_do_not_create_lock_files() {
    let temp = TempDir::new();
    let store = store(&temp);
    let lock = temp.0.join("state/runs/missing.lock");

    assert!(store.get("missing").is_err());
    assert!(!lock.exists());
    assert!(store.undo("missing").is_err());
    assert!(!lock.exists());
}

#[test]
fn incomplete_temporary_headers_do_not_block_run_listing() {
    let temp = TempDir::new();
    let directory = temp.0.join("state/runs");
    std::fs::create_dir_all(&directory).expect("create journal directory");
    std::fs::write(directory.join("interrupted.tmp"), "partial header")
        .expect("write interrupted temporary header");

    assert!(store(&temp).list().unwrap().is_empty());
}

#[test]
fn journaled_case_only_rename_can_be_undone() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("Name.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("Name.txt", "name.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply case-only rename");
    let run_id = applied.run_id.expect("mutation creates a journal run");

    store.undo(&run_id).expect("undo case-only rename");

    assert_eq!(std::fs::read(target.join("Name.txt")).unwrap(), b"contents");
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
}

#[test]
fn interrupted_apply_after_rename_is_reconciled_and_undone() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("before.txt", "after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    simulate_interrupted_move(&journal);

    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Interrupted);
    store
        .undo(&run_id)
        .expect("reconcile and undo the completed filesystem move");

    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"contents"
    );
    assert!(!target.join("after.txt").exists());
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn interrupted_apply_before_rename_leaves_the_source_untouched() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("before.txt", "after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    std::fs::rename(target.join("after.txt"), target.join("before.txt"))
        .expect("restore source to simulate crash before rename");
    simulate_interrupted_move(&journal);

    store.undo(&run_id).expect("reconcile an unapplied move");

    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"contents"
    );
    assert!(!target.join("after.txt").exists());
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn interrupted_apply_with_ambiguous_destination_is_left_untouched() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("before.txt", "after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    std::fs::write(target.join("after.txt"), "changed contents")
        .expect("replace destination contents");
    simulate_interrupted_move(&journal);

    let error = store
        .undo(&run_id)
        .expect_err("ambiguous identity must block recovery");

    assert!(
        error
            .to_string()
            .contains("unresolved filesystem operation")
    );
    assert_eq!(
        std::fs::read(target.join("after.txt")).unwrap(),
        b"changed contents"
    );
    assert!(!target.join("before.txt").exists());
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Interrupted);
}

#[test]
fn interrupted_undo_after_reverse_rename_can_be_retried() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("before.txt", "after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    store.undo(&run_id).expect("undo the apply");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    simulate_interrupted_undo(&journal);

    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undoing);
    store
        .undo(&run_id)
        .expect("reconcile and finish the interrupted undo");

    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"contents"
    );
    assert!(!target.join("after.txt").exists());
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn interrupted_undo_after_directory_removal_can_be_retried() {
    let (_temp, target, store, run_id, journal) =
        journaled_run(&[("before.txt", "nested/after.txt")]);
    store.undo(&run_id).expect("undo journaled apply");

    let records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .filter(|line| {
            !line.contains("\"record\":\"directory_changed\"")
                || !line.contains("\"action\":\"remove\"")
        })
        .filter(|line| !line.contains("\"status\":\"undone\""))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("simulate a crash after directory removal");

    store.undo(&run_id).expect("retry interrupted undo");

    assert!(target.join("before.txt").is_file());
    assert!(!target.join("nested").exists());
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn undo_refuses_to_move_a_modified_destination() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let plan = one_file_plan("before.txt", "after.txt");
    let store = store(&temp);

    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    std::fs::write(target.join("after.txt"), "changed contents").expect("modify destination file");

    assert!(store.undo(&run_id).is_err());
    assert_eq!(
        std::fs::read(target.join("after.txt")).unwrap(),
        b"changed contents"
    );
    assert!(!target.join("before.txt").exists());
}

#[test]
fn undo_preflights_every_move_before_mutating_any_file() {
    let (_temp, target, store, run_id, journal) = journaled_run(&[
        ("before-a.txt", "after-a.txt"),
        ("before-b.txt", "after-b.txt"),
    ]);
    let journal_before_undo = std::fs::read(&journal).expect("read applied journal");
    std::fs::write(target.join("after-a.txt"), "changed contents")
        .expect("modify later undo destination");

    let error = store
        .undo(&run_id)
        .expect_err("undo must reject a modified object before changing other files");
    let expected_path = target
        .join("after-a.txt")
        .canonicalize()
        .expect("canonicalize changed destination");

    assert!(matches!(
        error,
        dirule_fs::JournalError::ChangedObject { path, .. }
            if path == expected_path
    ));
    assert_eq!(
        std::fs::read(target.join("after-b.txt")).unwrap(),
        b"contents"
    );
    assert!(!target.join("before-b.txt").exists());
    assert_eq!(
        std::fs::read(target.join("after-a.txt")).unwrap(),
        b"changed contents"
    );
    assert!(!target.join("before-a.txt").exists());
    assert_eq!(std::fs::read(&journal).unwrap(), journal_before_undo);
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Applied);
}

#[test]
fn undo_refuses_to_overwrite_a_new_file_at_the_original_path() {
    let (_temp, target, store, run_id, journal) = journaled_file_run();
    let journal_before_undo = std::fs::read(&journal).expect("read applied journal");
    std::fs::write(target.join("before.txt"), "new file").expect("create new original path");

    let error = store
        .undo(&run_id)
        .expect_err("undo must preserve the new file");

    assert!(matches!(
        error,
        dirule_fs::JournalError::ExistingDestination(_)
    ));
    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"new file"
    );
    assert_eq!(
        std::fs::read(target.join("after.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(std::fs::read(&journal).unwrap(), journal_before_undo);
}

#[test]
fn undo_refuses_to_mutate_a_replaced_target_directory() {
    let (_temp, target, store, run_id, journal) = journaled_file_run();
    let journal_before_undo = std::fs::read(&journal).expect("read applied journal");
    let original_target = target.with_file_name("original-project");
    std::fs::rename(&target, &original_target).expect("move original target");
    std::fs::create_dir(&target).expect("create replacement target");
    std::fs::write(target.join("after.txt"), "replacement target")
        .expect("write replacement destination");

    let error = store
        .undo(&run_id)
        .expect_err("undo must reject a replaced target");

    assert!(matches!(error, dirule_fs::JournalError::ChangedTarget(id) if id == run_id));
    assert_eq!(
        std::fs::read(target.join("after.txt")).unwrap(),
        b"replacement target"
    );
    assert_eq!(
        std::fs::read(original_target.join("after.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(std::fs::read(&journal).unwrap(), journal_before_undo);
}

#[test]
fn undo_removes_only_parent_directories_created_by_the_apply() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let plan = one_file_plan("before.txt", "archive/after.txt");
    let store = store(&temp);

    let applied = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    assert!(target.join("archive/after.txt").is_file());

    store.undo(&run_id).expect("undo and clean created parent");

    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"contents"
    );
    assert!(!target.join("archive").exists());
}

#[test]
fn undo_records_multiple_directory_removals() {
    let (_temp, target, store, run_id, _) = journaled_run(&[
        ("before-one.txt", "one/after-one.txt"),
        ("before-two.txt", "two/after-two.txt"),
    ]);

    store.undo(&run_id).expect("undo journaled apply");

    assert!(target.join("before-one.txt").is_file());
    assert!(target.join("before-two.txt").is_file());
    assert!(!target.join("one").exists());
    assert!(!target.join("two").exists());
}

#[test]
fn undo_keeps_and_reports_a_created_directory_that_contains_new_files() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("before.txt", "archive/after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    std::fs::write(target.join("archive/keep.txt"), "user data")
        .expect("write new file in created directory");

    let undone = store.undo(&run_id).expect("undo file move");

    assert_eq!(undone.retained_directories, [PathBuf::from("archive")]);
    assert_eq!(
        std::fs::read(target.join("before.txt")).unwrap(),
        b"contents"
    );
    assert_eq!(
        std::fs::read(target.join("archive/keep.txt")).unwrap(),
        b"user data"
    );

    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    let mut records = std::fs::read_to_string(&journal)
        .expect("read completed journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    records.pop();
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("simulate interruption before final status");

    let recovered = store.undo(&run_id).expect("retry interrupted undo");

    assert_eq!(recovered.retained_directories, [PathBuf::from("archive")]);
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
}

#[test]
fn no_op_apply_does_not_create_a_journal() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("same.txt"), "contents").expect("write file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let plan = Plan::default();

    let result = apply_journaled(&target, &plan, &snapshot, ConflictPolicy::Abort, &store)
        .expect("no-op apply");

    assert!(result.run_id.is_none());
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn unsupported_journal_versions_are_rejected_without_rewriting_the_source() {
    let temp = TempDir::new();
    let journal = temp.0.join("state/runs/old-run.jsonl");
    std::fs::create_dir_all(journal.parent().expect("journal parent"))
        .expect("create journal directory");
    let original = b"{\"record\":\"header\",\"version\":\"0.9\"}\n";
    std::fs::write(&journal, original).expect("write an unsupported journal");

    let error = store(&temp)
        .list()
        .expect_err("no migration is registered for this old format");

    assert!(error.to_string().contains("no journal migration path"));
    assert_eq!(
        std::fs::read(journal).expect("read unchanged journal"),
        original
    );
}

#[test]
fn future_journal_versions_are_rejected_without_rewriting_the_source() {
    let temp = TempDir::new();
    let journal = temp.0.join("state/runs/future-run.jsonl");
    std::fs::create_dir_all(journal.parent().expect("journal parent"))
        .expect("create journal directory");
    let original = b"{\"record\":\"header\",\"version\":\"99.0\"}\n";
    std::fs::write(&journal, original).expect("write a future journal");

    let error = store(&temp)
        .list()
        .expect_err("future format is not supported");

    assert!(error.to_string().contains("uses newer version 99.0"));
    assert_eq!(
        std::fs::read(journal).expect("read unchanged journal"),
        original
    );
}

#[test]
fn duplicate_journal_operation_sequences_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let mut records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let intent = records
        .iter()
        .find(|line| line.contains("\"record\":\"move_intent\""))
        .expect("move intent")
        .clone();
    records.insert(2, intent);
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("duplicate move intent sequence");

    let error = store.list().expect_err("duplicate sequences are invalid");

    assert!(error.to_string().contains("duplicate journal sequence"));
}

#[test]
fn out_of_order_journal_operation_sequences_are_rejected() {
    let (_temp, _target, store, _run_id, journal) =
        journaled_run(&[("first.txt", "one.txt"), ("second.txt", "two.txt")]);
    let mut records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let second_intent = records
        .iter_mut()
        .find(|line| line.contains("\"record\":\"move_intent\"") && line.contains("second.txt"))
        .expect("second move intent");
    *second_intent = second_intent.replace("\"sequence\":1", "\"sequence\":3");
    let first_intent = records
        .iter_mut()
        .find(|line| line.contains("\"record\":\"move_intent\"") && line.contains("first.txt"))
        .expect("first move intent");
    *first_intent = first_intent.replace("\"sequence\":0", "\"sequence\":4");
    let first_completion = records
        .iter_mut()
        .find(|line| {
            line.contains("\"record\":\"move_completed\"") && line.contains("\"sequence\":0")
        })
        .expect("first move completion");
    *first_completion = first_completion.replace("\"sequence\":0", "\"sequence\":4");
    let second_completion = records
        .iter_mut()
        .find(|line| {
            line.contains("\"record\":\"move_completed\"") && line.contains("\"sequence\":1")
        })
        .expect("second move completion");
    *second_completion = second_completion.replace("\"sequence\":1", "\"sequence\":3");
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("reorder journal sequences");

    let error = store
        .list()
        .expect_err("out-of-order sequences are invalid");

    assert!(error.to_string().contains("is out of order"));
}

#[test]
fn orphaned_journal_completions_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .filter(|line| !line.contains("\"record\":\"move_intent\""))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("remove intent and preserve its completion");

    let error = store.list().expect_err("completion must have an intent");

    assert!(error.to_string().contains("has no matching intent"));
}

#[test]
fn duplicate_journal_completions_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let mut records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let completion = records
        .iter()
        .find(|line| line.contains("\"record\":\"move_completed\""))
        .expect("move completion")
        .clone();
    let final_status = records
        .iter()
        .rposition(|line| line.contains("\"record\":\"status\""))
        .expect("final status record");
    records.insert(final_status, completion);
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("duplicate move completion");

    let error = store.list().expect_err("duplicate completions are invalid");

    assert!(
        error
            .to_string()
            .contains("duplicate or mismatched completion")
    );
}

#[test]
fn mismatched_journal_completion_kinds_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(|line| {
            line.replace("move_completed", "directory_changed")
                .replace("\"identity\":", "\"action\":\"create\",\"identity\":")
        })
        .collect::<Vec<_>>();
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("replace move completion with a directory completion");

    let error = store.list().expect_err("completion kinds must match");

    assert!(
        error
            .to_string()
            .contains("mismatched directory completion")
    );
}

#[test]
fn mismatched_journal_object_identities_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(|line| {
            let mut record: serde_json::Value =
                serde_json::from_str(line).expect("valid journal record");
            if record["record"] == "move_completed" {
                record["identity"]["length"] = serde_json::Value::from(999_u64);
            }
            serde_json::to_string(&record).expect("serialize journal record")
        })
        .collect::<Vec<_>>();
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("change completion identity");

    let error = store.list().expect_err("completion identity must match");

    assert!(error.to_string().contains("mismatched move completion"));
}

#[test]
fn invalid_journal_status_transitions_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let mut records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    records.push(r#"{"record":"status","status":"undone"}"#.into());
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("append invalid status transition");

    let error = store.list().expect_err("invalid status transition");

    assert!(error.to_string().contains("invalid status transition"));
}

#[test]
fn duplicate_journal_headers_are_rejected() {
    let (_temp, _target, store, _run_id, journal) = journaled_file_run();
    let mut records = std::fs::read_to_string(&journal)
        .expect("read journal")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    records.insert(1, records[0].clone());
    std::fs::write(&journal, format!("{}\n", records.join("\n")))
        .expect("duplicate journal header");

    let error = store.list().expect_err("journal must have one header");

    assert!(error.to_string().contains("unexpected header record"));
}

#[test]
fn undo_truncates_an_incomplete_final_journal_record_before_appending() {
    let temp = TempDir::new();
    let target = temp.0.join("project");
    std::fs::create_dir(&target).expect("create project");
    std::fs::write(target.join("before.txt"), "contents").expect("write source file");
    let snapshot = scan(&target).expect("scan project");
    let store = store(&temp);
    let applied = apply_journaled(
        &target,
        &one_file_plan("before.txt", "after.txt"),
        &snapshot,
        ConflictPolicy::Abort,
        &store,
    )
    .expect("apply with journal");
    let run_id = applied.run_id.expect("mutation creates a journal run");
    let journal = temp.0.join(format!("state/runs/{run_id}.jsonl"));
    use std::io::Write as _;
    let mut journal_file = std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .expect("open journal");
    journal_file
        .write_all(b"{\"record\":\"partial")
        .expect("append incomplete journal tail");
    drop(journal_file);

    store.undo(&run_id).expect("truncate tail and undo");

    let contents = std::fs::read_to_string(journal).expect("read recovered journal");
    assert!(!contents.contains("partial"));
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
    assert!(target.join("before.txt").is_file());
}
