use dirule_core::{EntryKind, Plan, PlannedEntry};
use dirule_fs::{ConflictPolicy, JournalStatus, JournalStore, apply_journaled, scan};
use std::path::{Path, PathBuf};
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
    records.push(completion);
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
    std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .expect("open journal")
        .write_all(b"{\"record\":\"partial")
        .expect("append incomplete journal tail");

    store.undo(&run_id).expect("truncate tail and undo");

    let contents = std::fs::read_to_string(journal).expect("read recovered journal");
    assert!(!contents.contains("partial"));
    assert_eq!(store.list().unwrap()[0].status, JournalStatus::Undone);
    assert!(target.join("before.txt").is_file());
}
