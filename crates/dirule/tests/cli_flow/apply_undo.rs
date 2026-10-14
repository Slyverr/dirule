use super::common::*;

#[test]
fn cli_applies_and_undoes_a_configured_file_rename_cycle() {
    let project = TempProject::new();
    for (name, contents) in [("a.txt", "A"), ("b.txt", "B"), ("c.txt", "C")] {
        fs::write(project.0.join(name), contents).expect("write cycle source");
    }
    fs::write(
        project.0.join("dirule.toml"),
        r#"
version = "1.0"

[[rules]]
name = "Move A to B"
match = "a.txt"
to = "template:b.txt"

[[rules]]
name = "Move B to C"
match = "b.txt"
to = "template:c.txt"

[[rules]]
name = "Move C to A"
match = "c.txt"
to = "template:a.txt"
"#,
    )
    .expect("write cycle rules");

    let plan = project.run(&["plan", "--format", "json"]);

    assert!(
        plan.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&plan.stdout),
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    let changes = plan["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["source"] != entry["destination"])
        .count();
    assert_eq!(changes, 3);
    assert!(plan["conflicts"].as_array().unwrap().is_empty());

    let apply = project.run(&["apply", "--yes", "--format", "json"]);

    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    assert_eq!(apply["execution"]["status"], "applied");
    assert_eq!(text(&project.0.join("a.txt")), "C");
    assert_eq!(text(&project.0.join("b.txt")), "A");
    assert_eq!(text(&project.0.join("c.txt")), "B");

    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    let undo = project.run(&["undo", run_id, "--yes"]);

    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(text(&project.0.join("a.txt")), "A");
    assert_eq!(text(&project.0.join("b.txt")), "B");
    assert_eq!(text(&project.0.join("c.txt")), "C");
}

#[test]
fn cli_applies_and_undoes_a_configured_directory_swap() {
    let project = TempProject::new();
    fs::create_dir_all(project.0.join("first")).expect("create first directory");
    fs::create_dir_all(project.0.join("second")).expect("create second directory");
    fs::write(project.0.join("first/marker.txt"), "first").expect("write first marker");
    fs::write(project.0.join("second/marker.txt"), "second").expect("write second marker");
    fs::write(
        project.0.join("dirule.toml"),
        r#"
version = "1.0"

[[rules]]
name = "Swap first directory"
match = "first"
kind = "directory"
to = "template:second"

[[rules]]
name = "Swap second directory"
match = "second"
kind = "directory"
to = "template:first"
"#,
    )
    .expect("write directory swap rules");

    let plan = project.run(&["plan", "--format", "json"]);

    assert!(
        plan.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&plan.stdout),
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    assert!(plan["conflicts"].as_array().unwrap().is_empty());

    let apply = project.run(&["apply", "--yes", "--format", "json"]);

    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    assert_eq!(apply["execution"]["status"], "applied");
    assert_eq!(text(&project.0.join("first/marker.txt")), "second");
    assert_eq!(text(&project.0.join("second/marker.txt")), "first");

    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    let undo = project.run(&["undo", run_id, "--yes"]);

    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(text(&project.0.join("first/marker.txt")), "first");
    assert_eq!(text(&project.0.join("second/marker.txt")), "second");
}

#[test]
fn cli_skips_an_occupied_directory_and_its_descendants_but_applies_independent_moves() {
    let project = TempProject::new();
    fs::create_dir_all(project.0.join("Source")).expect("create source directory");
    fs::create_dir_all(project.0.join("Occupied")).expect("create occupied destination");
    fs::write(project.0.join("Source/child.txt"), "dependent").expect("write dependent child");
    fs::write(project.0.join("Occupied/existing.txt"), "keep")
        .expect("write existing destination content");
    fs::write(project.0.join("Independent.txt"), "independent").expect("write independent source");
    fs::write(
        project.0.join("dirule.toml"),
        r#"
version = "1.0"

[[rules]]
name = "Move source directory"
match = "Source"
kind = "directory"
to = "template:Occupied"

[[rules]]
name = "Normalize independent file"
match = "Independent.txt"
kind = "file"
to = "template:independent.txt"
"#,
    )
    .expect("write mixed conflict rules");

    let apply = project.run(&[
        "apply",
        "--yes",
        "--on-conflict",
        "skip",
        "--format",
        "json",
    ]);

    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    assert_eq!(apply["execution"]["status"], "applied");
    assert!(project.0.join("Source/child.txt").is_file());
    assert_eq!(text(&project.0.join("Occupied/existing.txt")), "keep");
    assert_eq!(text(&project.0.join("independent.txt")), "independent");
    let root_names = || {
        fs::read_dir(&project.0)
            .expect("read project entries")
            .map(|entry| {
                entry
                    .expect("read project entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>()
    };
    assert!(root_names().iter().any(|name| name == "independent.txt"));
    assert!(!root_names().iter().any(|name| name == "Independent.txt"));

    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    let undo = project.run(&["undo", run_id, "--yes"]);

    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(text(&project.0.join("Independent.txt")), "independent");
    assert!(root_names().iter().any(|name| name == "Independent.txt"));
    assert!(!root_names().iter().any(|name| name == "independent.txt"));
    assert!(project.0.join("Source/child.txt").is_file());
    assert_eq!(text(&project.0.join("Occupied/existing.txt")), "keep");
}

#[test]
fn configured_directory_rename_preserves_hidden_paths_and_undoes() {
    let project = TempProject::new();
    fs::create_dir_all(project.0.join(".github")).expect("create GitHub config directory");
    fs::create_dir_all(project.0.join(".config")).expect("create config directory");
    fs::create_dir_all(project.0.join(".HiddenProject")).expect("create hidden project directory");
    fs::create_dir_all(project.0.join("nested/.MyConfig")).expect("create nested hidden directory");
    fs::write(project.0.join(".github/README.md"), "workflow").expect("write GitHub config file");
    fs::write(project.0.join(".config/settings.toml"), "setting = true")
        .expect("write config file");
    fs::write(project.0.join(".HiddenProject/data.txt"), "hidden")
        .expect("write hidden project file");
    fs::write(project.0.join("nested/.MyConfig/data.txt"), "nested")
        .expect("write nested config file");
    fs::write(
        project.0.join("dirule.toml"),
        r#"
version = "1.2"

[[rules]]
name = "Normalize directories"
match = "**/*"
kind = "directory"
to = "builtin:kebab-case"
"#,
    )
    .expect("write config");

    let plan = project.run(&["plan", "--format", "json"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    let entries = plan["entries"].as_array().expect("plan entries");
    for (source, destination) in [
        (".HiddenProject", ".hidden-project"),
        ("nested/.MyConfig", "nested/.my-config"),
    ] {
        assert!(
            entries
                .iter()
                .any(|entry| { entry["source"] == source && entry["destination"] == destination })
        );
    }
    for path in [".github", ".config"] {
        assert!(
            entries
                .iter()
                .any(|entry| { entry["source"] == path && entry["destination"] == path })
        );
    }

    let apply = project.run(&["apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    let run_id = apply["execution"]["run_id"].as_str().expect("run ID");
    assert!(project.0.join(".github/README.md").is_file());
    assert!(project.0.join(".config/settings.toml").is_file());
    assert_eq!(text(&project.0.join(".hidden-project/data.txt")), "hidden");
    assert_eq!(
        text(&project.0.join("nested/.my-config/data.txt")),
        "nested"
    );

    let undo = project.run(&["undo", run_id, "--yes", "--format", "json"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert!(project.0.join(".HiddenProject/data.txt").is_file());
    assert!(project.0.join("nested/.MyConfig/data.txt").is_file());
}

#[test]
fn apply_and_undo_emit_single_json_documents() {
    let project = TempProject::new();
    let canonical_target = project
        .0
        .canonicalize()
        .expect("canonicalize project target");
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("NeedsWork.txt"), "contents").expect("write fixture file");

    let apply = project.run(&["apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert!(!String::from_utf8_lossy(&apply.stderr).contains("Apply preview"));
    let apply: serde_json::Value =
        serde_json::from_slice(&apply.stdout).expect("one apply JSON document");
    assert_eq!(apply["execution"]["status"], "applied");
    assert_eq!(
        apply["plan"]["target"],
        canonical_target.to_string_lossy().as_ref()
    );
    assert_eq!(apply["execution"]["moved"], 1);
    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("run identifier");

    let undo = project.run(&["undo", run_id, "--yes", "--format", "json"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    let undo: serde_json::Value =
        serde_json::from_slice(&undo.stdout).expect("one undo JSON document");
    assert_eq!(undo["status"], "undone");
    assert!(project.0.join("NeedsWork.txt").is_file());
}

#[test]
fn undo_reports_created_directories_that_cannot_be_removed() {
    let project = TempProject::new();
    fs::write(project.0.join("before.txt"), "contents").expect("write source file");
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\ntarget = \".\"\n\n[[rules]]\nname = \"Archive\"\nmatch = \"before.txt\"\nto = \"template:archive/after.txt\"\n",
    )
    .expect("write project config");

    let apply = project.run(&["apply", "--yes"]);
    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let runs = project.run(&["runs", "list", "--format", "json"]);
    let runs: serde_json::Value = serde_json::from_slice(&runs.stdout).expect("valid runs JSON");
    let run_id = runs["runs"][0]["id"].as_str().expect("journal run ID");
    fs::write(project.0.join("archive/keep.txt"), "user data")
        .expect("write new file in created directory");

    let undo = project.run(&["undo", run_id, "--yes"]);

    assert!(
        undo.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&undo.stdout),
        String::from_utf8_lossy(&undo.stderr)
    );
    assert!(
        String::from_utf8_lossy(&undo.stdout)
            .contains("Left non-empty directory archive in place.")
    );
    assert!(project.0.join("before.txt").is_file());
    assert!(project.0.join("archive/keep.txt").is_file());
}

#[test]
fn journal_storage_rejects_relative_platform_directories() {
    let project = TempProject::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    #[cfg(target_os = "windows")]
    command.env("LOCALAPPDATA", "relative-local-app-data");
    #[cfg(target_os = "macos")]
    command.env("HOME", "relative-home");
    #[cfg(all(unix, not(target_os = "macos")))]
    command
        .env("HOME", "relative-home")
        .env("XDG_STATE_HOME", "");

    let output = command
        .args(["runs", "list"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("user state directory"));
}

#[test]
fn apply_requires_confirmation_when_stdin_is_not_a_terminal() {
    let project = TempProject::new();
    fs::write(project.0.join("MyFile.txt"), "content").expect("write source file");
    assert!(project.run(&["init"]).status.success());

    let apply = project.run(&["apply"]);

    assert_eq!(apply.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&apply.stderr).contains("use --yes"));
    assert!(project.0.join("MyFile.txt").is_file());
    assert!(!project.0.join("my-file.txt").exists());
    assert!(!project.journal_directory().exists());

    let apply_json = project.run(&["apply", "--format", "json"]);

    assert_eq!(apply_json.status.code(), Some(2));
    assert!(apply_json.stdout.is_empty());
    let apply_json_error = String::from_utf8_lossy(&apply_json.stderr);
    assert!(apply_json_error.contains("Apply preview"));
    assert!(apply_json_error.contains("MyFile.txt -> my-file.txt"));
    assert!(apply_json_error.contains("use --yes"));
    let error_document_start = apply_json_error
        .rfind("\n{")
        .expect("JSON command error follows the preview");
    let error: serde_json::Value =
        serde_json::from_str(&apply_json_error[error_document_start + 1..])
            .expect("confirmation error is a JSON document");
    assert_eq!(error["schema_version"], "1.3");
    assert_eq!(error["error"]["code"], "command_error");
    assert_eq!(error["error"]["exit_code"], 2);
    assert!(project.0.join("MyFile.txt").is_file());
    assert!(!project.0.join("my-file.txt").exists());
    assert!(!project.journal_directory().exists());
}

#[test]
fn no_op_apply_does_not_create_journal_state() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("already-normal.txt"), "content").expect("write normalized file");

    let apply = project.run(&["apply", "--yes"]);

    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    assert!(String::from_utf8_lossy(&apply.stdout).contains("No changes needed"));
    assert!(!project.journal_directory().exists());
}

#[test]
fn rejected_conflicting_apply_does_not_create_journal_state() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("A__B.txt"), "first").expect("write first conflict source");
    fs::write(project.0.join("A--B.txt"), "second").expect("write second conflict source");

    let apply = project.run(&["apply", "--yes"]);

    assert_eq!(apply.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&apply.stderr).contains("conflict"));
    assert_eq!(text(&project.0.join("A__B.txt")), "first");
    assert_eq!(text(&project.0.join("A--B.txt")), "second");
    assert!(!project.journal_directory().exists());
}

#[test]
fn cli_can_skip_conflicts_and_apply_unrelated_moves() {
    let project = TempProject::new();
    fs::write(project.0.join("First.txt"), "first source").expect("write first source");
    fs::write(project.0.join("Second.txt"), "second source").expect("write second source");
    fs::write(project.0.join("Independent.txt"), "independent move")
        .expect("write independent source");
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\ntarget = \".\"\n\n[[rules]]\nname = \"first\"\nmatch = \"First.txt\"\nto = \"template:collision.txt\"\n\n[[rules]]\nname = \"second\"\nmatch = \"Second.txt\"\nto = \"template:collision.txt\"\n\n[[rules]]\nname = \"independent\"\nmatch = \"Independent.txt\"\nto = \"template:renamed.txt\"\n",
    )
    .expect("write fixture config");

    let abort = project.run(&["apply", "--yes"]);
    assert_eq!(abort.status.code(), Some(2));
    assert!(project.0.join("Independent.txt").is_file());

    let skip = project.run(&["apply", "--yes", "--on-conflict", "skip"]);
    assert!(skip.status.success());
    assert!(
        String::from_utf8_lossy(&skip.stdout)
            .contains("Applied 1 rename(s); skipped 1 conflict(s).")
    );
    assert!(project.0.join("First.txt").is_file());
    assert!(project.0.join("Second.txt").is_file());
    assert_eq!(text(&project.0.join("renamed.txt")), "independent move");
}
