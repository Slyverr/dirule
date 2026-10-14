use dirule::application::presets::{PresetError, PresetStore};
use dirule_core::parse_config;
use dirule_fs::{ConflictPolicy, JournalStore};
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempTarget(PathBuf);

impl TempTarget {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let target =
            std::env::temp_dir().join(format!("dirule-application-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&target).expect("create target");

        Self(target)
    }
}

impl Deref for TempTarget {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TempTarget {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn preview_and_authorized_apply_share_the_operation_api() {
    let target = TempTarget::new();
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

    let preview = dirule::application::preview(&config, &target.0).expect("preview target");

    assert_eq!(preview.target, target.0.canonicalize().unwrap());
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("MyFile.txt") && entry.destination == Path::new("my-file.txt")
    }));
    assert!(source.is_file());
    assert!(!target.join("my-file.txt").exists());

    let journal = JournalStore::new(target.join(".journal"));
    let applied = dirule::application::apply_authorized(preview, ConflictPolicy::Abort, &journal)
        .expect("apply authorized plan");
    let run_id = applied.run_id.as_deref().expect("journaled run ID");
    assert_eq!(applied.report.moved.len(), 1);
    assert!(!source.exists());
    assert!(target.join("my-file.txt").is_file());

    journal.undo(run_id).expect("undo applied plan");
    assert!(source.is_file());
    assert!(!target.join("my-file.txt").exists());
}

#[test]
fn preset_store_validates_roundtrips_and_removes_user_presets() {
    let target = TempTarget::new();
    let source = target.join("dirule.toml");
    let contents = r#"
version = "1.0"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#;
    fs::write(&source, contents).expect("write source config");
    let store = PresetStore::new(target.join("presets")).expect("create preset store");

    assert!(store.list().expect("list empty store").is_empty());
    assert!(matches!(
        store.path("../outside"),
        Err(PresetError::InvalidName(_))
    ));

    let path = store.save("downloads", &source).expect("save valid preset");
    assert_eq!(store.list().expect("list saved preset"), ["downloads"]);
    assert!(matches!(
        store.save("downloads", &source),
        Err(PresetError::AlreadyExists(_))
    ));

    let edited_path = store
        .save_contents("edited", contents)
        .expect("save preset contents");
    assert_eq!(
        store.list().expect("list saved presets"),
        ["downloads", "edited"]
    );
    assert_eq!(
        store.load("edited").expect("load edited preset").path,
        edited_path
    );

    let loaded = store.load("downloads").expect("load saved preset");
    assert_eq!(loaded.name, "downloads");
    assert_eq!(loaded.path, path);
    assert_eq!(loaded.contents, contents);
    assert_eq!(loaded.config.rules.len(), 1);

    store.remove("downloads").expect("remove saved preset");
    store.remove("edited").expect("remove edited preset");
    assert!(store.list().expect("list empty store").is_empty());
    assert!(matches!(
        store.load("downloads"),
        Err(PresetError::NotFound(_))
    ));
}

#[test]
fn sort_by_extension_normalizes_suffixes_and_leaves_dotfiles_alone() {
    let target = TempTarget::new();
    fs::write(target.join("archive.tar.GZ"), "archive").expect("write multi-extension file");
    fs::write(target.join(".profile"), "profile").expect("write dotfile");
    fs::create_dir_all(target.join("nested")).expect("create nested directory");
    fs::write(target.join("nested/Report.PDF"), "report").expect("write nested PDF");

    let preview = dirule::application::sort_by_extension_preview(&target.0, true)
        .expect("preview extension sort");

    assert!(preview.plan.conflicts.is_empty(), "{:#?}", preview.plan);
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("archive.tar.GZ")
            && entry.destination == Path::new("gzs/archive.tar.GZ")
    }));
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("nested/Report.PDF")
            && entry.destination == Path::new("pdfs/Report.PDF")
    }));
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new(".profile") && entry.destination == Path::new(".profile")
    }));
    assert!(target.join("archive.tar.GZ").is_file());
    assert!(!target.join("gzs/archive.tar.GZ").exists());
}

#[test]
fn sort_extension_matches_case_insensitively_without_recursing_by_default() {
    let target = TempTarget::new();
    fs::write(target.join("Invoice.PDF"), "invoice").expect("write PDF");
    fs::create_dir_all(target.join("nested")).expect("create nested directory");
    fs::write(target.join("nested/Report.pdf"), "report").expect("write nested PDF");

    let preview = dirule::application::sort_extension_preview(&target.0, ".PDF", false)
        .expect("preview case-insensitive extension sort");

    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("Invoice.PDF")
            && entry.destination == Path::new("pdfs/Invoice.PDF")
    }));
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("nested/Report.pdf")
            && entry.destination == Path::new("nested/Report.pdf")
    }));
    assert!(target.join("Invoice.PDF").is_file());
    assert!(!target.join("pdfs/Invoice.PDF").exists());
}

#[test]
fn rename_preview_uses_shared_transforms_and_inherits_directory_moves() {
    let target = TempTarget::new();
    fs::create_dir_all(target.join("HTTPServer")).expect("create source directory");
    fs::write(target.join("HTTPServer/ReadMe.txt"), "notes").expect("write nested file");
    fs::write(target.join("MyXMLParser.pdf"), "document").expect("write source file");

    let preview =
        dirule::application::rename_preview(&target.0, dirule_core::Transform::KebabCase, false)
            .expect("preview rename operation");

    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("MyXMLParser.pdf")
            && entry.destination == Path::new("my-xml-parser.pdf")
    }));
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("HTTPServer") && entry.destination == Path::new("http-server")
    }));
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("HTTPServer/ReadMe.txt")
            && entry.destination == Path::new("http-server/ReadMe.txt")
    }));
    assert!(target.join("MyXMLParser.pdf").is_file());
    assert!(target.join("HTTPServer/ReadMe.txt").is_file());
}
