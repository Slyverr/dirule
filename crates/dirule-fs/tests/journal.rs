use dirule_core::{EntryKind, Plan, PlannedEntry};
use dirule_fs::{ConflictPolicy, JournalStatus, JournalStore, apply_journaled, scan};
use std::path::PathBuf;
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
