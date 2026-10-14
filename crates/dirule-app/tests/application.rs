use dirule_app::configuration::{ConfigEditor, ConfigEditorError};
use dirule_app::journal::JournalService;
use dirule_app::presets::{PresetError, PresetStore};
use dirule_core::{Rule, RuleKind, parse_config};
use dirule_fs::{ConflictPolicy, JournalStore};
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_TARGET: AtomicU64 = AtomicU64::new(0);

struct TempTarget(PathBuf);

impl TempTarget {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_TARGET.fetch_add(1, Ordering::Relaxed);
        let target = std::env::temp_dir().join(format!(
            "dirule-application-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&target).expect("create target");

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

fn config_with_target(target: Option<&Path>) -> String {
    let target = target
        .map(|path| format!("target = {:?}\n", path.to_string_lossy()))
        .unwrap_or_default();

    format!(
        "version = \"{}\"\n{target}\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    )
}

#[test]
fn starter_configuration_is_created_and_loads_through_the_public_api() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");

    dirule_app::configuration::create_starter(&path).expect("create starter config");

    let loaded = dirule_app::configuration::load(&path).expect("load starter config");
    assert_eq!(loaded.config().name.as_deref(), Some("Local rules"));
    assert_eq!(loaded.config().target.as_deref(), Some(Path::new(".")));
    assert_eq!(loaded.config().rules.len(), 1);
    assert_eq!(loaded.config().rules[0].name, "Normalize file names");
}

#[test]
fn starter_configuration_does_not_replace_an_existing_file() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    fs::write(&path, "existing configuration").expect("write existing config");

    let error = dirule_app::configuration::create_starter(&path)
        .expect_err("starter creation must not overwrite an existing file");

    assert!(error.to_string().contains("cannot create"));
    assert_eq!(fs::read_to_string(path).unwrap(), "existing configuration");
}

#[test]
fn starter_configuration_requires_an_absolute_path() {
    let error = dirule_app::configuration::create_starter("dirule.toml")
        .expect_err("starter creation must not depend on the process current directory");

    assert!(matches!(
        error,
        dirule_app::configuration::ConfigCreationError::RelativePath(_)
    ));
}

#[cfg(unix)]
#[test]
fn starter_configuration_uses_standard_create_permissions() {
    use std::fs::OpenOptions;
    use std::os::unix::fs::PermissionsExt;

    let target = TempTarget::new();
    let reference_path = target.join("reference");
    let config_path = target.join("dirule.toml");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(reference_path)
        .expect("create reference file");

    dirule_app::configuration::create_starter(&config_path).expect("create starter config");

    let reference_mode = fs::metadata(target.join("reference"))
        .expect("read reference metadata")
        .permissions()
        .mode()
        & 0o777;
    let config_mode = fs::metadata(config_path)
        .expect("read config metadata")
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(config_mode, reference_mode);
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

    let preview = dirule_app::preview(&config, &target.0).expect("preview target");

    assert_eq!(preview.target, target.0.canonicalize().unwrap());
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("MyFile.txt") && entry.destination == Path::new("my-file.txt")
    }));
    assert!(source.is_file());
    assert!(!target.join("my-file.txt").exists());

    let journal = JournalService::new(JournalStore::new(target.join(".journal")));
    let applied = dirule_app::apply_authorized(preview, ConflictPolicy::Abort, &journal)
        .expect("apply authorized plan");
    let run_id = applied.run_id.as_deref().expect("journaled run ID");
    assert_eq!(applied.report.moved.len(), 1);
    assert!(!source.exists());
    assert!(target.join("my-file.txt").is_file());

    let runs = journal.list_runs().expect("list apply runs");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, run_id);

    let details = journal.run_details(run_id).expect("inspect apply run");
    assert_eq!(details.run.id, run_id);
    assert_eq!(details.operations.len(), 1);

    journal
        .undo_authorized(run_id)
        .expect("undo authorized run");
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
fn application_path_resolution_keeps_absolute_paths_and_anchors_relative_paths() {
    let base = TempTarget::new();
    let relative = PathBuf::from("nested/project");
    let absolute = base.join("other-project");

    assert_eq!(
        dirule_app::paths::resolve_from(&base, relative.clone()),
        base.join(relative)
    );
    assert_eq!(
        dirule_app::paths::resolve_from(&base, absolute.clone()),
        absolute
    );
    assert_eq!(
        dirule_app::paths::resolve_target(&base, "nested/project".into())
            .expect("resolve relative target"),
        base.join("nested/project")
    );
}

#[test]
fn application_target_resolution_prefers_explicit_then_configured_then_base() {
    let base = TempTarget::new();
    let configured = PathBuf::from("configured-target");
    let explicit = PathBuf::from("explicit-target");

    assert_eq!(
        dirule_app::paths::resolve_operation_target(
            &base,
            Some(explicit.clone()),
            Some(configured.clone())
        )
        .expect("resolve explicit target"),
        base.join(explicit)
    );
    assert_eq!(
        dirule_app::paths::resolve_operation_target(&base, None, Some(configured.clone()))
            .expect("resolve configured target"),
        base.join(configured)
    );
    assert_eq!(
        dirule_app::paths::resolve_operation_target(&base, None, None)
            .expect("resolve base target"),
        base.0
    );
}

#[test]
fn configuration_loader_exposes_validated_config_and_source_path() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "# project rules\nversion = \"{}\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, &contents).expect("write project config");

    let loaded = dirule_app::configuration::load(&path).expect("load project config");

    assert_eq!(loaded.path(), path);
    assert_eq!(loaded.config().rules.len(), 1);
    assert_eq!(loaded.config().rules[0].name, "Normalize names");
}

#[test]
fn configuration_loader_errors_include_the_source_path() {
    let target = TempTarget::new();
    let path = target.join("broken.toml");
    fs::write(&path, "version = [").expect("write invalid config");

    let error = dirule_app::configuration::load(&path).unwrap_err();

    assert!(matches!(
        error,
        dirule_app::configuration::ConfigurationLoadError::Invalid { .. }
    ));
    assert!(error.to_string().contains(&path.display().to_string()));
}

#[test]
fn configuration_sources_resolve_project_files_and_named_presets() {
    let base = TempTarget::new();
    let default = dirule_app::configuration::ConfigurationSource::project_default(&base);

    assert_eq!(default.path(), base.join("dirule.toml"));
    assert_eq!(default.name(), "dirule.toml");
    assert_eq!(
        default.kind(),
        dirule_app::configuration::ConfigurationSourceKind::ProjectDefault
    );

    let explicit = dirule_app::configuration::ConfigurationSource::explicit_file(
        &base,
        PathBuf::from("config/rules.toml"),
    );
    assert_eq!(explicit.path(), base.join("config/rules.toml"));
    assert_eq!(explicit.name(), "config/rules.toml");
    assert_eq!(
        explicit.kind(),
        dirule_app::configuration::ConfigurationSourceKind::ExplicitFile
    );

    let store = PresetStore::new(base.join("presets")).expect("create preset store");
    let preset = dirule_app::configuration::ConfigurationSource::preset(&store, "downloads")
        .expect("resolve preset");

    assert_eq!(preset.path(), store.directory().join("downloads.toml"));
    assert_eq!(preset.name(), "downloads");
    assert_eq!(
        preset.kind(),
        dirule_app::configuration::ConfigurationSourceKind::Preset
    );
}

#[test]
fn implicit_config_discovery_prefers_a_target_local_config() {
    let base = TempTarget::new();
    let target = base.join("nested-project");
    fs::create_dir(&target).expect("create target directory");
    fs::write(
        target.join("dirule.toml"),
        config_with_target(Some(Path::new("."))),
    )
    .expect("write target-local config");
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("matching", &config_with_target(Some(&target)))
        .expect("save matching preset");

    let source = dirule_app::configuration::discover_source(
        &base,
        Some(PathBuf::from("nested-project")),
        Some(&presets),
    )
    .expect("discover source");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::ProjectDefault
    );
    assert_eq!(source.path(), target.join("dirule.toml"));
}

#[test]
fn invalid_target_local_config_does_not_fall_back_to_a_matching_preset() {
    let base = TempTarget::new();
    let target = base.join("nested-project");
    fs::create_dir(&target).expect("create target directory");
    fs::write(target.join("dirule.toml"), "version = [").expect("write invalid local config");
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("matching", &config_with_target(Some(&target)))
        .expect("save matching preset");

    let source = dirule_app::configuration::discover_source(
        &base,
        Some(PathBuf::from("nested-project")),
        Some(&presets),
    )
    .expect("discover local source");
    let error = dirule_app::configuration::load_source(&source)
        .expect_err("invalid local config should be reported");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::ProjectDefault
    );
    assert!(error.to_string().contains("dirule.toml"));
}

#[test]
fn implicit_config_discovery_selects_one_exact_global_preset_target() {
    let base = TempTarget::new();
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("downloads", &config_with_target(Some(&base)))
        .expect("save target-bound preset");

    let source = dirule_app::configuration::discover_source(&base, None, Some(&presets))
        .expect("discover source");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::Preset
    );
    assert_eq!(source.name(), "downloads");
    assert_eq!(source.path(), presets.directory().join("downloads.toml"));
}

#[test]
fn invalid_unrelated_presets_do_not_block_target_discovery() {
    let base = TempTarget::new();
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("downloads", &config_with_target(Some(&base)))
        .expect("save target-bound preset");
    fs::write(presets.directory().join("broken.toml"), "version = [")
        .expect("write malformed unrelated preset");

    let source = dirule_app::configuration::discover_source(&base, None, Some(&presets))
        .expect("discover target-bound preset despite unrelated malformed preset");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::Preset
    );
    assert_eq!(source.name(), "downloads");
}

#[test]
fn implicit_discovery_normalizes_nonexistent_target_paths() {
    let base = TempTarget::new();
    let target = base.join("project");
    let target_with_parent = base.join("missing/../project");
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("project", &config_with_target(Some(&target)))
        .expect("save target-bound preset");

    let source =
        dirule_app::configuration::discover_source(&base, Some(target_with_parent), Some(&presets))
            .expect("discover preset for normalized path");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::Preset
    );
    assert_eq!(source.name(), "project");
}

#[test]
fn implicit_config_discovery_rejects_ambiguous_preset_targets() {
    let base = TempTarget::new();
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    for name in ["downloads", "archive"] {
        presets
            .save_contents(name, &config_with_target(Some(&base)))
            .expect("save target-bound preset");
    }

    let error = dirule_app::configuration::discover_source(&base, None, Some(&presets))
        .expect_err("ambiguous target bindings");

    assert!(error.to_string().contains("multiple presets target"));
    assert!(error.to_string().contains("archive, downloads"));
}

#[test]
fn relative_preset_targets_do_not_bind_to_the_invocation_directory() {
    let base = TempTarget::new();
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("relative", &config_with_target(Some(Path::new("."))))
        .expect("save relative-target preset");

    let source = dirule_app::configuration::discover_source(&base, None, Some(&presets))
        .expect("discover source");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::ProjectDefault
    );
    assert_eq!(source.path(), base.join("dirule.toml"));
}

#[cfg(unix)]
#[test]
fn implicit_config_discovery_matches_symlinked_target_paths() {
    use std::os::unix::fs::symlink;

    let base = TempTarget::new();
    let target = base.join("project");
    let target_alias = base.join("project-alias");
    fs::create_dir(&target).expect("create target directory");
    symlink(&target, &target_alias).expect("create target symlink");
    let presets = PresetStore::new(base.join("presets")).expect("create preset store");
    presets
        .save_contents("project", &config_with_target(Some(&target)))
        .expect("save target-bound preset");

    let source = dirule_app::configuration::discover_source(
        &base,
        Some(PathBuf::from("project-alias")),
        Some(&presets),
    )
    .expect("discover source through symlink");

    assert_eq!(
        source.kind(),
        dirule_app::configuration::ConfigurationSourceKind::Preset
    );
    assert_eq!(source.name(), "project");
}

#[test]
fn configuration_loading_reports_missing_selected_sources() {
    let target = TempTarget::new();
    let source = dirule_app::configuration::ConfigurationSource::project_default(&target);

    let error = dirule_app::configuration::load_source(&source).unwrap_err();

    assert!(matches!(
        error,
        dirule_app::configuration::ConfigurationLoadError::Missing { .. }
    ));
    assert!(
        error
            .to_string()
            .contains(&source.path().display().to_string())
    );
}

#[test]
fn preview_source_uses_configured_target_and_explicit_override() {
    let base = TempTarget::new();
    let configured_target = base.join("configured");
    let explicit_target = base.join("explicit");
    fs::create_dir(&configured_target).expect("create configured target");
    fs::create_dir(&explicit_target).expect("create explicit target");
    fs::write(configured_target.join("MyFile.txt"), "configured").expect("write source file");
    fs::write(explicit_target.join("OtherFile.txt"), "explicit").expect("write override file");
    fs::write(
        base.join("dirule.toml"),
        format!(
            "version = \"{}\"\ntarget = \"configured\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
            dirule_core::CURRENT_CONFIG_VERSION
        ),
    )
    .expect("write project config");
    let source = dirule_app::configuration::ConfigurationSource::project_default(&base);

    let configured =
        dirule_app::preview_source(&source, &base, None).expect("preview configured target");
    let explicit = dirule_app::preview_source(&source, &base, Some(PathBuf::from("explicit")))
        .expect("preview explicit target");

    assert_eq!(configured.target, configured_target.canonicalize().unwrap());
    assert!(configured.plan.entries.iter().any(|entry| {
        entry.source == Path::new("MyFile.txt") && entry.destination == Path::new("my-file.txt")
    }));
    assert_eq!(explicit.target, explicit_target.canonicalize().unwrap());
    assert!(explicit.plan.entries.iter().any(|entry| {
        entry.source == Path::new("OtherFile.txt")
            && entry.destination == Path::new("other-file.txt")
    }));
}

#[test]
fn config_editor_preserves_comments_and_file_permissions() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "# project settings\nversion = \"{}\" # schema\nname = 'Local rules' # keep this note\n\n[[rules]] # retain rule note\nname = 'Normalize names'\nmatch = '**/*'\nto = 'builtin:kebab-case'\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, &contents).expect("write project config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("set project config permissions");
    }

    let mut editor = ConfigEditor::open(&path).expect("open project config");
    editor.document_mut()["rules"][0]["to"] = toml_edit::value("builtin:snake-case");
    let config = editor.save().expect("save edited project config");
    let saved = fs::read_to_string(&path).expect("read edited project config");

    assert_eq!(config.rules[0].to, "builtin:snake-case");
    assert!(saved.contains("# project settings"));
    assert!(saved.contains("# schema"));
    assert!(saved.contains("# keep this note"));
    assert!(saved.contains("# retain rule note"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}

#[test]
fn config_editor_sets_hard_ignore_patterns_and_migrates_the_document() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    fs::write(
        &path,
        "version = \"1.2\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write legacy config");

    let mut editor = ConfigEditor::open(&path).expect("open legacy config");
    editor
        .set_ignore_patterns(&["generated/**".into(), "vendor".into()])
        .expect("set target-wide exclusions");
    let config = editor.save().expect("save migrated config");
    let saved = fs::read_to_string(path).expect("read saved config");

    assert_eq!(config.ignore, ["generated/**", "vendor"]);
    assert!(saved.contains(&format!(
        "version = \"{}\"",
        dirule_core::CURRENT_CONFIG_VERSION
    )));
    assert!(saved.contains("ignore = [\"generated/**\", \"vendor\"]"));
}

#[test]
fn config_editor_updates_metadata_through_typed_operations() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "version = \"{}\"\nname = 'Old rules' # name note\ntarget = '.' # target note\n\n[acronyms] # acronym table note\nentries = ['iOS']\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, contents).expect("write project config");

    let mut editor = ConfigEditor::open(&path).expect("open project config");
    editor
        .set_name(Some("Project rules"))
        .expect("update config name");
    editor
        .set_target(Some(Path::new("archive")))
        .expect("update config target");
    editor
        .set_acronym_entries(&["iOS".into(), "eBay".into()])
        .expect("update acronym entries");

    let config = editor.config().expect("validate edited config");
    let rendered = editor.contents();

    assert_eq!(config.name.as_deref(), Some("Project rules"));
    assert_eq!(config.target.as_deref(), Some(Path::new("archive")));
    assert_eq!(config.acronyms.entries, ["iOS", "eBay"]);
    assert!(rendered.contains("# name note"));
    assert!(rendered.contains("# target note"));
    assert!(rendered.contains("# acronym table note"));

    editor.save().expect("save edited config");
    let saved = fs::read_to_string(&path).expect("read edited config");
    assert_eq!(
        parse_config(&saved).unwrap().acronyms.entries,
        ["iOS", "eBay"]
    );

    editor.set_name(None).expect("remove config name");
    editor.set_target(None).expect("remove config target");
    editor
        .set_acronym_entries(&[])
        .expect("clear acronym entries");
    let cleared = editor.config().expect("validate cleared metadata");

    assert_eq!(cleared.name, None);
    assert_eq!(cleared.target, None);
    assert!(cleared.acronyms.entries.is_empty());
}

#[test]
fn config_editor_updates_inline_acronym_tables() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "version = \"{}\"\nacronyms = {{ entries = [\"iOS\"] }}\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, contents).expect("write project config");
    let mut editor = ConfigEditor::open(&path).expect("open project config");

    editor
        .set_acronym_entries(&["eBay".into()])
        .expect("update inline acronym table");

    assert_eq!(
        editor
            .config()
            .expect("validate edited config")
            .acronyms
            .entries,
        ["eBay"]
    );
}

#[test]
fn config_editor_rejects_invalid_acronym_edits_without_losing_valid_state() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    fs::write(&path, config_with_target(None)).expect("write project config");
    let mut editor = ConfigEditor::open(&path).expect("open project config");
    let before = editor.contents();

    assert!(matches!(
        editor.set_acronym_entries(&["iOS".into(), "iOS".into()]),
        Err(ConfigEditorError::Configuration { .. })
    ));
    assert_eq!(editor.contents(), before);
}

#[cfg(unix)]
#[test]
fn config_editor_rejects_non_unicode_target_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    fs::write(&path, config_with_target(None)).expect("write project config");
    let mut editor = ConfigEditor::open(&path).expect("open project config");
    let target = PathBuf::from(OsString::from_vec(vec![0xff]));

    assert!(matches!(
        editor.set_target(Some(&target)),
        Err(ConfigEditorError::NonUnicodeTarget(_))
    ));
}

#[test]
fn config_editor_supports_adding_and_reordering_rules() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "version = \"{}\"\n\n# first rule note\n[[rules]]\nname = \"First rule\"\nmatch = \"**/*.md\"\nto = \"builtin:kebab-case\"\n\n# second rule note\n[[rules]]\nname = \"Second rule\"\nmatch = \"**/*\"\nto = \"builtin:snake-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, contents).expect("write project config");

    let mut editor = ConfigEditor::open(&path).expect("open project config");
    editor
        .add_rule(Rule {
            name: "Third rule".into(),
            pattern: "**/*.txt".into(),
            kind: RuleKind::File,
            exclude: vec!["**/drafts/**".into()],
            to: "builtin:pascal-case".into(),
        })
        .expect("add a rule");
    editor.move_rule(2, 0).expect("reorder rules");

    let mut updated_rule = editor.config().expect("read configured rules").rules[0].clone();
    updated_rule.to = "builtin:camel-case".into();
    editor.update_rule(0, updated_rule).expect("update a rule");
    assert_eq!(
        editor.remove_rule(1).expect("remove a rule").name,
        "First rule"
    );

    let edited = editor.config().expect("validate edited config");
    assert_eq!(
        edited
            .rules
            .iter()
            .map(|rule| rule.name.as_str())
            .collect::<Vec<_>>(),
        ["Third rule", "Second rule"]
    );
    assert_eq!(edited.rules[0].to, "builtin:camel-case");
    assert_eq!(edited.rules[0].kind, RuleKind::File);
    assert_eq!(edited.rules[0].exclude, ["**/drafts/**"]);

    editor.save().expect("save edited project config");
    let saved = fs::read_to_string(&path).expect("read edited project config");
    assert!(!saved.contains("# first rule note"));
    assert!(saved.contains("# second rule note"));
    let reloaded = parse_config(&saved).expect("parse saved project config");
    assert_eq!(
        reloaded
            .rules
            .iter()
            .map(|rule| rule.name.as_str())
            .collect::<Vec<_>>(),
        ["Third rule", "Second rule"]
    );
}

#[test]
fn config_editor_rejects_invalid_rule_edits_without_losing_valid_state() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    fs::write(&path, config_with_target(None)).expect("write project config");

    let mut editor = ConfigEditor::open(&path).expect("open project config");
    let mut invalid_rule = editor.config().expect("read configuration").rules[0].clone();
    invalid_rule.to = "unknown:transform".into();

    assert!(matches!(
        editor.update_rule(0, invalid_rule),
        Err(ConfigEditorError::Configuration { .. })
    ));
    assert_eq!(editor.config().unwrap().rules[0].to, "builtin:kebab-case");
    assert!(matches!(
        editor.move_rule(0, 1),
        Err(ConfigEditorError::InvalidRuleIndex { .. })
    ));
}

#[test]
fn config_editor_refuses_to_overwrite_external_changes() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "version = \"{}\"\nname = \"Local rules\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, &contents).expect("write project config");
    let mut editor = ConfigEditor::open(&path).expect("open project config");
    editor.document_mut()["name"] = toml_edit::value("Edited rules");
    let external = format!("{contents}\n# changed by another editor\n");
    fs::write(&path, &external).expect("write external change");

    assert!(matches!(
        editor.save(),
        Err(ConfigEditorError::ConcurrentModification(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), external);
}

#[test]
fn config_editor_rejects_invalid_edits_without_writing() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "version = \"{}\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, &contents).expect("write project config");
    let mut editor = ConfigEditor::open(&path).expect("open project config");
    editor.document_mut()["rules"][0]["to"] = toml_edit::value("unknown:transform");

    assert!(matches!(
        editor.save(),
        Err(ConfigEditorError::Configuration { .. })
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), contents);
}

#[test]
fn config_editor_leaves_unchanged_documents_byte_identical() {
    let target = TempTarget::new();
    let path = target.join("dirule.toml");
    let contents = format!(
        "version = \"{}\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"",
        dirule_core::CURRENT_CONFIG_VERSION
    );
    fs::write(&path, &contents).expect("write project config without trailing newline");
    let mut editor = ConfigEditor::open(&path).expect("open project config");

    editor.save().expect("save unchanged project config");

    assert_eq!(fs::read_to_string(&path).unwrap(), contents);
}

#[cfg(unix)]
#[test]
fn config_editor_rejects_symlinked_config_files() {
    use std::os::unix::fs::symlink;

    let target = TempTarget::new();
    let source = target.join("source.toml");
    let link = target.join("dirule.toml");
    fs::write(
        &source,
        format!(
            "version = \"{}\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
            dirule_core::CURRENT_CONFIG_VERSION
        ),
    )
    .expect("write source config");
    symlink(&source, &link).expect("create config symlink");

    assert!(matches!(
        ConfigEditor::open(link),
        Err(ConfigEditorError::NotRegularFile(_))
    ));
}

#[test]
fn sort_by_extension_normalizes_suffixes_and_leaves_dotfiles_alone() {
    let target = TempTarget::new();
    fs::write(target.join("archive.tar.GZ"), "archive").expect("write multi-extension file");
    fs::write(target.join(".profile"), "profile").expect("write dotfile");
    fs::create_dir_all(target.join("nested")).expect("create nested directory");
    fs::write(target.join("nested/Report.PDF"), "report").expect("write nested PDF");

    let preview =
        dirule_app::sort_by_extension_preview(&target.0, true).expect("preview extension sort");

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

    let preview = dirule_app::sort_extension_preview(&target.0, ".PDF", false)
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

    let preview = dirule_app::rename_preview(&target.0, dirule_core::Transform::KebabCase, false)
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

#[test]
fn preview_hard_excludes_configured_paths_independent_of_vcs_ignores() {
    let target = TempTarget::new();
    fs::create_dir_all(target.join("generated/api")).expect("create generated directory");
    fs::create_dir_all(target.join("build")).expect("create build directory");
    fs::write(target.join(".gitignore"), "build/**\n").expect("write Git ignore rules");
    fs::write(target.join("generated/api/Client.ts"), "generated").expect("write generated source");
    fs::write(target.join("build/Tracked.ts"), "tracked").expect("write tracked ignored source");
    fs::write(target.join("VisibleFile.ts"), "visible").expect("write visible source");

    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&target.0)
        .status()
        .expect("initialize Git fixture");
    assert!(init.success(), "initialize Git fixture");
    let add = Command::new("git")
        .args(["add", ".gitignore", "generated/api/Client.ts"])
        .current_dir(&target.0)
        .status()
        .expect("stage generated source");
    assert!(add.success(), "stage generated source");
    let add_ignored = Command::new("git")
        .args(["add", "-f", "build/Tracked.ts"])
        .current_dir(&target.0)
        .status()
        .expect("stage VCS-ignored source");
    assert!(add_ignored.success(), "stage VCS-ignored source");

    let config = parse_config(
        r#"
version = "1.3"
ignore = ["generated/**", "build/**"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("parse config");

    let preview = dirule_app::preview(&config, &target.0).expect("preview target");

    assert!(
        preview.snapshot.entries.iter().all(|entry| {
            !entry.path.starts_with("generated") && !entry.path.starts_with("build")
        })
    );
    assert!(preview.plan.entries.iter().any(|entry| {
        entry.source == Path::new("VisibleFile.ts")
            && entry.destination == Path::new("visible-file.ts")
    }));
    assert!(!preview
        .plan
        .entries
        .iter()
        .any(|entry| entry.source.starts_with("generated") || entry.source.starts_with("build")));
}
