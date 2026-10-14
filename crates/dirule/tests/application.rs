use dirule_core::parse_config;
use dirule_fs::{ConflictPolicy, JournalStore};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn preview_and_authorized_apply_share_the_operation_api() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after Unix epoch")
        .as_nanos();
    let target =
        std::env::temp_dir().join(format!("dirule-application-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&target).expect("create target");
    let source = target.join("MyFile.txt");
    fs::write(&source, "content").expect("write source file");
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("parse config");

    let preview = dirule::application::preview(&config, &target).expect("preview target");

    assert_eq!(preview.target, target.canonicalize().unwrap());
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("MyFile.txt") && entry.destination == Path::new("my-file.txt")
    }));
    assert!(source.is_file());
    assert!(!target.join("my-file.txt").exists());

    let journal = JournalStore::new(target.with_extension("journal"));
    let applied = dirule::application::apply_authorized(preview, ConflictPolicy::Abort, &journal)
        .expect("apply authorized plan");
    let run_id = applied.run_id.as_deref().expect("journaled run ID");
    assert_eq!(applied.report.moved.len(), 1);
    assert!(!source.exists());
    assert!(target.join("my-file.txt").is_file());

    journal.undo(run_id).expect("undo applied plan");
    assert!(source.is_file());
    assert!(!target.join("my-file.txt").exists());

    fs::remove_dir_all(&target).expect("remove target");
    fs::remove_dir_all(target.with_extension("journal")).expect("remove test journal");
}
