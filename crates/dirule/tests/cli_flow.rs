use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_PROJECT_ID: AtomicU64 = AtomicU64::new(0);

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let id = NEXT_TEMP_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dirule-cli-{}-{stamp}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp project");
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("run dirule")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
        #[cfg(target_os = "windows")]
        command.env("LOCALAPPDATA", self.0.with_extension("local-app-data"));
        #[cfg(target_os = "macos")]
        command.env("HOME", self.0.with_extension("home"));
        #[cfg(all(unix, not(target_os = "macos")))]
        command.env("XDG_STATE_HOME", self.0.with_extension("state"));
        command.args(args).current_dir(&self.0);

        command
    }

    fn journal_directory(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        let base = self.0.with_extension("local-app-data");
        #[cfg(target_os = "macos")]
        let base = self
            .0
            .with_extension("home")
            .join("Library/Application Support");
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = self.0.with_extension("state");

        base.join("dirule/runs")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        let _ = fs::remove_dir_all(self.0.with_extension("state"));
        let _ = fs::remove_dir_all(self.0.with_extension("local-app-data"));
        let _ = fs::remove_dir_all(self.0.with_extension("home"));
    }
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).expect("read fixture file")
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().expect("inspect fixture entry").is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("copy fixture file");
        }
    }
}

#[cfg(target_os = "windows")]
fn preset_config_base(project: &TempProject) -> PathBuf {
    project.0.join("appdata")
}

#[cfg(target_os = "macos")]
fn preset_config_base(project: &TempProject) -> PathBuf {
    project
        .0
        .with_extension("home")
        .join("Library/Application Support")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn preset_config_base(project: &TempProject) -> PathBuf {
    project.0.join("xdg-config")
}

fn configure_preset_environment(command: &mut Command, project: &TempProject) {
    #[cfg(target_os = "windows")]
    command.env("APPDATA", preset_config_base(project));
    #[cfg(target_os = "macos")]
    command.env("HOME", project.0.with_extension("home"));
    #[cfg(all(unix, not(target_os = "macos")))]
    command.env("XDG_CONFIG_HOME", preset_config_base(project));
}

fn configure_home(command: &mut Command, home: &Path) {
    #[cfg(target_os = "windows")]
    command.env("USERPROFILE", home);
    #[cfg(not(target_os = "windows"))]
    command.env("HOME", home);
}

fn config_for_target(target: Option<&Path>, transform: &str) -> String {
    let target = target
        .map(|path| format!("target = {:?}\n", path.to_string_lossy()))
        .unwrap_or_default();

    format!(
        "version = \"{}\"\n{target}\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:{transform}\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    )
}

#[test]
fn cli_accepts_human_output_format_and_rejects_text_alias() {
    let project = TempProject::new();
    let init = project.run(&["init"]);
    assert!(init.status.success());

    let human = project.run(&["plan", "--format", "human"]);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(String::from_utf8_lossy(&human.stdout).contains("Target:"));

    let text = project.run(&["plan", "--format", "text"]);
    assert_eq!(text.status.code(), Some(2));
}

#[test]
fn cli_initializes_checks_plans_applies_and_emits_json() {
    let project = TempProject::new();
    let canonical_target = project
        .0
        .canonicalize()
        .expect("canonicalize project target");
    let directory = project.0.join("HTTPServer");
    fs::create_dir_all(&directory).expect("create fixture directory");
    fs::write(project.0.join("MyXMLParser.rs"), "parser").expect("write root file");
    fs::write(directory.join("OAuthToken.txt"), "token").expect("write nested file");

    let init = project.run(&["init", "--color", "always"]);
    assert!(init.status.success());
    assert!(String::from_utf8_lossy(&init.stdout).contains("\u{1b}[32mCreated"));
    assert!(project.0.join("dirule.toml").is_file());

    let plan = project.run(&["plan", "--color", "always"]);
    assert!(plan.status.success());
    let output = String::from_utf8_lossy(&plan.stdout);
    assert!(output.contains("3 rename(s), 0 conflict(s)"));
    assert!(output.contains("\u{1b}[31mMyXMLParser.rs\u{1b}[0m"));
    assert!(output.contains("\u{1b}[32mmy-xml-parser.rs\u{1b}[0m"));
    assert!(output.contains("OAuthToken.txt"));

    let plain = project.run(&["plan", "--color", "never"]);
    assert!(plain.status.success());
    let plain_output = String::from_utf8_lossy(&plain.stdout);
    assert!(plain_output.contains(&format!("Target: {}", canonical_target.display())));
    assert!(!plain_output.contains('\u{1b}'));

    let check = project.run(&["check"]);
    assert_eq!(check.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&check.stdout).contains("Check failed"));
    assert!(String::from_utf8_lossy(&check.stderr).contains("target does not satisfy"));
    assert!(!project.journal_directory().exists());

    let json = project.run(&["plan", "--format", "json"]);
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("valid JSON plan");
    assert_eq!(value["target"], canonical_target.to_string_lossy().as_ref());
    let entries = value["entries"].as_array().expect("plan entries");
    let changes = entries
        .iter()
        .filter(|entry| entry["source"] != entry["destination"])
        .count();
    assert_eq!(changes, 3);
    assert!(entries.iter().any(|entry| {
        entry["source"] == "HTTPServer/OAuthToken.txt"
            && entry["destination"] == "http-server/o-auth-token.txt"
    }));
    assert!(entries.iter().all(|entry| {
        !entry["source"]
            .as_str()
            .expect("source path")
            .contains('\\')
            && !entry["destination"]
                .as_str()
                .expect("destination path")
                .contains('\\')
    }));
    assert!(!String::from_utf8_lossy(&json.stdout).contains('\u{1b}'));

    let apply = project.run(&["apply", "--yes", "--color", "always"]);
    assert!(apply.status.success());
    assert!(String::from_utf8_lossy(&apply.stdout).contains("Applied 3 rename(s)."));
    assert!(String::from_utf8_lossy(&apply.stdout).contains("Undo with: dirule undo "));
    assert_eq!(text(&project.0.join("my-xml-parser.rs")), "parser");
    assert_eq!(
        text(&project.0.join("http-server/o-auth-token.txt")),
        "token"
    );

    let check = project.run(&["check"]);
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("Check passed"));

    let runs = project.run(&["runs", "list", "--format", "json"]);
    assert!(runs.status.success());
    let runs: serde_json::Value = serde_json::from_slice(&runs.stdout).expect("valid runs JSON");
    let run_id = runs["runs"][0]["id"].as_str().expect("journal run ID");
    assert_eq!(runs["runs"][0]["status"], "applied");

    let details = project.run(&["runs", "show", run_id, "--format", "json"]);
    assert!(
        details.status.success(),
        "{}",
        String::from_utf8_lossy(&details.stderr)
    );
    let details: serde_json::Value =
        serde_json::from_slice(&details.stdout).expect("valid run details JSON");
    assert_eq!(details["run"]["id"], run_id);
    assert_eq!(details["changes"].as_array().unwrap().len(), 3);
    assert!(details["changes"].as_array().unwrap().iter().any(|change| {
        change["source"] == "MyXMLParser.rs" && change["destination"] == "my-xml-parser.rs"
    }));

    let details = project.run(&["runs", "show", run_id]);
    assert!(details.status.success());
    let details = String::from_utf8_lossy(&details.stdout);
    assert!(details.contains("Apply run"));
    assert!(details.contains("MyXMLParser.rs -> my-xml-parser.rs (file)"));

    let undo = project.run(&["undo", run_id, "--yes"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert!(project.0.join("MyXMLParser.rs").is_file());
    assert!(project.0.join("HTTPServer/OAuthToken.txt").is_file());
}

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
fn quick_rename_previews_applies_and_undoes_without_a_config() {
    let project = TempProject::new();
    fs::write(project.0.join("MyXMLParser.pdf"), "document").expect("write source file");

    let preview = project.run(&["rename", "kebab-case", "--format", "json"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid rename preview");
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "MyXMLParser.pdf" && entry["destination"] == "my-xml-parser.pdf"
    }));
    assert!(project.0.join("MyXMLParser.pdf").is_file());
    assert!(!project.0.join("my-xml-parser.pdf").exists());

    let apply = project.run(&[
        "rename",
        "kebab-case",
        "--apply",
        "--yes",
        "--format",
        "json",
    ]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value =
        serde_json::from_slice(&apply.stdout).expect("valid apply result");
    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    assert!(project.0.join("my-xml-parser.pdf").is_file());

    let undo = project.run(&["undo", run_id, "--yes"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert!(project.0.join("MyXMLParser.pdf").is_file());
}

#[test]
fn quick_rename_applies_and_undoes_case_only_names() {
    let project = TempProject::new();
    fs::write(project.0.join("LICENSE"), "license text").expect("write source file");

    let apply = project.run(&[
        "rename",
        "kebab-case",
        "--apply",
        "--yes",
        "--format",
        "json",
    ]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value =
        serde_json::from_slice(&apply.stdout).expect("valid apply result");
    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    let names = fs::read_dir(&project.0)
        .expect("read renamed target")
        .map(|entry| entry.expect("read renamed entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, [std::ffi::OsString::from("license")]);

    let undo = project.run(&["undo", run_id, "--yes"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    let names = fs::read_dir(&project.0)
        .expect("read restored target")
        .map(|entry| entry.expect("read restored entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, [std::ffi::OsString::from("LICENSE")]);
    assert_eq!(
        fs::read(project.0.join("LICENSE")).unwrap(),
        b"license text"
    );
}

#[test]
fn quick_recursive_rename_transforms_directories_and_undoes() {
    let project = TempProject::new();
    fs::create_dir_all(project.0.join("HTTPServer/NestedFolder"))
        .expect("create nested source directories");
    fs::write(project.0.join("HTTPServer/ReadMeFile.txt"), "notes")
        .expect("write nested source file");
    fs::write(
        project.0.join("HTTPServer/NestedFolder/MyXMLParser.pdf"),
        "document",
    )
    .expect("write deeply nested source file");

    let preview = project.run(&["rename", "kebab-case", "--recursive", "--format", "json"]);
    assert!(preview.status.success());
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid recursive rename preview");
    assert!(
        preview["entries"].as_array().unwrap().iter().any(|entry| {
            entry["source"] == "HTTPServer" && entry["destination"] == "http-server"
        })
    );
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "HTTPServer/NestedFolder/MyXMLParser.pdf"
            && entry["destination"] == "http-server/nested-folder/my-xml-parser.pdf"
    }));
    assert!(
        project
            .0
            .join("HTTPServer/NestedFolder/MyXMLParser.pdf")
            .is_file()
    );

    let apply = project.run(&[
        "rename",
        "kebab-case",
        "--recursive",
        "--apply",
        "--yes",
        "--format",
        "json",
    ]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value =
        serde_json::from_slice(&apply.stdout).expect("valid recursive rename apply result");
    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    assert!(
        project
            .0
            .join("http-server/nested-folder/my-xml-parser.pdf")
            .is_file()
    );

    let undo = project.run(&["undo", run_id, "--yes"]);
    assert!(undo.status.success());
    assert!(
        project
            .0
            .join("HTTPServer/NestedFolder/MyXMLParser.pdf")
            .is_file()
    );
}

#[test]
fn quick_sort_matches_extension_case_insensitively_and_defaults_to_preview() {
    let project = TempProject::new();
    fs::write(project.0.join("Invoice.PDF"), "invoice").expect("write uppercase extension");
    fs::write(project.0.join("notes.txt"), "notes").expect("write unrelated file");
    fs::create_dir(project.0.join("docs")).expect("create nested directory");
    fs::write(project.0.join("docs/Guide.PDF"), "guide").expect("write nested PDF");

    let preview = project.run(&["sort", "pdf", "--format", "json"]);
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid sort preview");
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "Invoice.PDF" && entry["destination"] == "pdfs/Invoice.PDF"
    }));
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "docs/Guide.PDF" && entry["destination"] == "docs/Guide.PDF"
    }));
    assert!(project.0.join("Invoice.PDF").is_file());
    assert!(!project.0.join("pdfs/Invoice.PDF").exists());

    let apply = project.run(&["sort", "pdf", "--apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value =
        serde_json::from_slice(&apply.stdout).expect("valid apply result");
    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("journal run ID");
    assert!(project.0.join("pdfs/Invoice.PDF").is_file());
    assert!(project.0.join("docs/Guide.PDF").is_file());

    let undo = project.run(&["undo", run_id, "--yes"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert!(project.0.join("Invoice.PDF").is_file());
}

#[test]
fn quick_sort_respects_project_ignore_files() {
    let project = TempProject::new();
    fs::write(project.0.join(".ignore"), "Invoice.PDF\n").expect("write ignore rule");
    fs::write(project.0.join("Invoice.PDF"), "invoice").expect("write ignored PDF");

    let preview = project.run(&["sort", "pdf", "--format", "json"]);

    assert!(preview.status.success());
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid ignored-file preview");
    assert!(preview["entries"].as_array().unwrap().iter().all(|entry| {
        entry["source"] != "Invoice.PDF" && entry["destination"] != "pdfs/Invoice.PDF"
    }));
    assert!(project.0.join("Invoice.PDF").is_file());
    assert!(!project.0.join("pdfs").exists());
}

#[test]
fn quick_sort_diruleignore_can_include_a_vcs_ignored_path() {
    let project = TempProject::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project.0)
        .status()
        .expect("initialize Git fixture");
    assert!(init.success(), "initialize Git fixture");
    fs::write(project.0.join(".gitignore"), "Invoice.PDF\n").expect("write Git ignore rule");
    fs::write(project.0.join(".diruleignore"), "!Invoice.PDF\n")
        .expect("write Dirule ignore exception");
    fs::write(project.0.join("Invoice.PDF"), "invoice").expect("write ignored PDF");

    let preview = project.run(&["sort", "pdf", "--format", "json"]);

    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid re-included-file preview");
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "Invoice.PDF" && entry["destination"] == "pdfs/Invoice.PDF"
    }));
    assert!(project.0.join("Invoice.PDF").is_file());
    assert!(!project.0.join("pdfs/Invoice.PDF").exists());
}

#[test]
fn quick_sort_apply_requires_confirmation_without_yes() {
    let project = TempProject::new();
    fs::write(project.0.join("Invoice.pdf"), "invoice").expect("write source invoice");

    let output = project
        .command(&["sort", "pdf", "--apply", "--format", "json"])
        .stdin(Stdio::null())
        .output()
        .expect("run quick sort without an interactive terminal");

    assert_eq!(output.status.code(), Some(2));
    assert!(project.0.join("Invoice.pdf").is_file());
    assert!(!project.0.join("pdfs/Invoice.pdf").exists());
}

#[test]
fn quick_recursive_sort_collects_files_in_target_level_buckets() {
    let project = TempProject::new();
    fs::write(project.0.join("ReadMe.MD"), "readme").expect("write root markdown file");
    fs::create_dir_all(project.0.join("docs")).expect("create docs directory");
    fs::write(project.0.join("docs/Guide.PDF"), "guide").expect("write nested PDF");

    let preview = project.run(&[
        "sort",
        "--by",
        "extension",
        "--recursive",
        "--format",
        "json",
    ]);

    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid extension sort preview");
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "ReadMe.MD" && entry["destination"] == "mds/ReadMe.MD"
    }));
    assert!(preview["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "docs/Guide.PDF" && entry["destination"] == "pdfs/Guide.PDF"
    }));
    assert!(project.0.join("ReadMe.MD").is_file());
    assert!(project.0.join("docs/Guide.PDF").is_file());

    let apply = project.run(&[
        "sort",
        "--by",
        "extension",
        "--recursive",
        "--apply",
        "--yes",
        "--format",
        "json",
    ]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    assert!(project.0.join("mds/ReadMe.MD").is_file());
    assert!(project.0.join("pdfs/Guide.PDF").is_file());

    let repeated = project.run(&[
        "sort",
        "--by",
        "extension",
        "--recursive",
        "--format",
        "json",
    ]);
    assert!(
        repeated.status.success(),
        "{}",
        String::from_utf8_lossy(&repeated.stderr)
    );
    let repeated: serde_json::Value =
        serde_json::from_slice(&repeated.stdout).expect("valid repeated sort plan");
    assert!(repeated["conflicts"].as_array().unwrap().is_empty());
    assert!(
        repeated["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| { entry["source"] == entry["destination"] })
    );
}

#[test]
fn quick_sort_reports_duplicate_recursive_destinations_without_mutating() {
    let project = TempProject::new();
    fs::create_dir_all(project.0.join("invoices-a")).expect("create first invoice directory");
    fs::create_dir_all(project.0.join("invoices-b")).expect("create second invoice directory");
    fs::write(project.0.join("invoices-a/April.PDF"), "first").expect("write first invoice");
    fs::write(project.0.join("invoices-b/April.PDF"), "second").expect("write second invoice");

    let output = project.run(&["sort", "pdf", "--recursive", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan");
    assert!(!plan["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(text(&project.0.join("invoices-a/April.PDF")), "first");
    assert_eq!(text(&project.0.join("invoices-b/April.PDF")), "second");
    assert!(!project.0.join("pdfs").exists());
}

#[test]
fn quick_sort_reports_existing_destinations_without_overwriting() {
    let project = TempProject::new();
    fs::write(project.0.join("Invoice.pdf"), "source").expect("write source invoice");
    fs::create_dir_all(project.0.join("pdfs")).expect("create destination bucket");
    fs::write(project.0.join("pdfs/Invoice.pdf"), "existing").expect("write existing destination");

    let output = project.run(&["sort", "pdf", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan");
    assert!(!plan["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(text(&project.0.join("Invoice.pdf")), "source");
    assert_eq!(text(&project.0.join("pdfs/Invoice.pdf")), "existing");
}

#[test]
fn cli_plans_pascal_case_names_in_text_and_json() {
    let project = TempProject::new();
    fs::write(project.0.join("my-xml-parser.rs"), "parser").expect("write fixture file");
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\n\n[[rules]]\nname = \"PascalCase source files\"\nmatch = \"*.rs\"\nto = \"builtin:pascal-case\"\n",
    )
    .expect("write PascalCase config");

    let text_plan = project.run(&["plan"]);

    assert!(text_plan.status.success());
    assert!(
        String::from_utf8_lossy(&text_plan.stdout).contains("my-xml-parser.rs -> MyXmlParser.rs")
    );

    let json_plan = project.run(&["plan", "--format", "json"]);

    assert!(json_plan.status.success());
    let json_plan: serde_json::Value =
        serde_json::from_slice(&json_plan.stdout).expect("valid PascalCase plan JSON");
    let entry = json_plan["entries"]
        .as_array()
        .expect("plan entries")
        .iter()
        .find(|entry| entry["source"] == "my-xml-parser.rs")
        .expect("PascalCase entry");
    assert_eq!(entry["destination"], "MyXmlParser.rs");
    assert!(project.0.join("my-xml-parser.rs").is_file());
    assert!(!project.0.join("MyXmlParser.rs").exists());
}

#[test]
fn common_short_options_work_through_the_public_cli() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());

    let target = project.0.join("target");
    fs::create_dir(&target).expect("create target directory");
    fs::write(target.join("MyXMLParser.rs"), "parser").expect("write target file");
    let canonical_target = target.canonicalize().expect("canonicalize target");

    let validate = project.run(&["validate", "-c", "dirule.toml", "-f", "json"]);
    assert!(validate.status.success());
    let validate: serde_json::Value =
        serde_json::from_slice(&validate.stdout).expect("valid validation JSON");
    assert_eq!(validate["valid"], true);

    let plan = project.run(&["plan", "-t", "target", "-c", "dirule.toml", "-f", "json"]);
    assert!(plan.status.success());
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    assert_eq!(plan["target"], canonical_target.to_string_lossy().as_ref());
    assert!(plan["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "MyXMLParser.rs" && entry["destination"] == "my-xml-parser.rs"
    }));

    let mut save = project.command(&["presets", "save", "compact", "-c", "dirule.toml"]);
    configure_preset_environment(&mut save, &project);
    let save = save.output().expect("save preset");
    assert!(
        save.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&save.stdout),
        String::from_utf8_lossy(&save.stderr)
    );

    let mut list = project.command(&["presets", "list", "-f", "json"]);
    configure_preset_environment(&mut list, &project);
    let list = list.output().expect("list presets");
    assert!(list.status.success());
    let list: serde_json::Value = serde_json::from_slice(&list.stdout).expect("valid preset JSON");
    assert!(
        list["presets"]
            .as_array()
            .unwrap()
            .contains(&"compact".into())
    );

    let mut apply =
        project.command(&["apply", "-t", "target", "-p", "compact", "-y", "-f", "json"]);
    configure_preset_environment(&mut apply, &project);
    let apply = apply.output().expect("apply preset");
    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    let run_id = apply["execution"]["run_id"]
        .as_str()
        .expect("applied run ID");

    let runs = project.run(&["runs", "list", "-f", "json"]);
    assert!(runs.status.success());
    let runs: serde_json::Value = serde_json::from_slice(&runs.stdout).expect("valid runs JSON");
    assert_eq!(runs["runs"][0]["id"], run_id);

    let details = project.run(&["runs", "show", run_id, "-f", "json"]);
    assert!(details.status.success());
    let details: serde_json::Value =
        serde_json::from_slice(&details.stdout).expect("valid run details JSON");
    assert_eq!(details["run"]["id"], run_id);

    let undo = project.run(&["undo", run_id, "-y", "-f", "json"]);
    assert!(undo.status.success());
    let undo: serde_json::Value = serde_json::from_slice(&undo.stdout).expect("valid undo JSON");
    assert_eq!(undo["status"], "undone");
    assert!(target.join("MyXMLParser.rs").is_file());

    let mut remove = project.command(&["presets", "remove", "compact", "-y"]);
    configure_preset_environment(&mut remove, &project);
    assert!(remove.output().expect("remove preset").status.success());
}

#[test]
fn plan_json_output_is_stable_across_repeated_scans() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    for name in ["ZebraFile.txt", "AppleFile.txt", "MiddleFile.txt"] {
        fs::write(project.0.join(name), name).expect("write source file");
    }

    let first = project.run(&["plan", "--format", "json"]);
    let second = project.run(&["plan", "--format", "json"]);

    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    for name in ["ZebraFile.txt", "AppleFile.txt", "MiddleFile.txt"] {
        assert!(project.0.join(name).is_file());
    }
}

#[test]
fn plan_exits_cleanly_when_a_piped_reader_closes_early() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    for index in 0..2_000 {
        fs::write(project.0.join(format!("FileNumber{index:04}.txt")), "x")
            .expect("write fixture file");
    }

    for args in [&["plan"][..], &["plan", "--format", "json"][..]] {
        let mut child = project
            .command(args)
            .stdout(Stdio::piped())
            .spawn()
            .expect("start dirule");
        let mut stdout = child.stdout.take().expect("capture stdout");
        let mut first_byte = [0];
        stdout
            .read_exact(&mut first_byte)
            .expect("read initial output");
        drop(stdout);

        let output = child.wait_with_output().expect("wait for dirule");
        assert!(
            output.status.success(),
            "dirule did not handle the closed pipe cleanly: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn completions_generate_commands_and_global_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_dirule"))
        .args(["completions", "bash"])
        .output()
        .expect("generate Bash completions");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let script = String::from_utf8_lossy(&output.stdout);
    assert!(script.contains("completions"));
    assert!(script.contains("validate"));
    assert!(script.contains("presets"));
    assert!(script.contains("--target"));
    assert!(script.contains("--format"));
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
fn init_preserves_an_existing_config() {
    let project = TempProject::new();
    let config = project.0.join("dirule.toml");
    fs::write(&config, "user-owned config\n").expect("write existing config");

    let init = project.run(&["init"]);

    assert_eq!(init.status.code(), Some(2));
    assert_eq!(text(&config), "user-owned config\n");
    assert!(String::from_utf8_lossy(&init.stderr).contains("cannot create"));
}

#[test]
fn malformed_config_errors_identify_the_source_file() {
    let project = TempProject::new();
    let config_path = project.0.join("rules.toml");
    fs::write(&config_path, "version = 12\n").expect("write malformed config");

    let plan = project.run(&["plan", "--config", "rules.toml"]);
    let plan_error = String::from_utf8_lossy(&plan.stderr);
    assert_eq!(plan.status.code(), Some(2));
    assert!(plan_error.contains("rules.toml"));
    assert!(plan_error.contains("configuration"));

    let check = project.run(&["check", "--config", "rules.toml"]);
    assert_eq!(check.status.code(), Some(2));

    let mut save = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut save, &project);
    let save = save
        .args(["presets", "save", "invalid", "--config", "rules.toml"])
        .current_dir(&project.0)
        .output()
        .expect("reject malformed preset source");
    let save_error = String::from_utf8_lossy(&save.stderr);

    assert_eq!(save.status.code(), Some(2));
    assert!(save_error.contains("rules.toml"));
    assert!(save_error.contains("configuration"));
    assert!(
        !preset_config_base(&project)
            .join("dirule/presets/invalid.toml")
            .exists()
    );
}

#[test]
fn json_command_errors_emit_a_versioned_error_document_to_stderr() {
    let project = TempProject::new();
    fs::write(project.0.join("rules.toml"), "version = 12\n").expect("write malformed config");

    let output = project.run(&["plan", "--config", "rules.toml", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("command error is a JSON document");
    assert_eq!(value["schema_version"], "1.3");
    assert_eq!(value["error"]["code"], "command_error");
    assert_eq!(value["error"]["kind"], "configuration");
    assert_eq!(value["error"]["exit_code"], 2);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rules.toml")
    );
}

#[test]
fn json_command_errors_classify_unavailable_targets() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());

    let output = project.run(&[
        "plan",
        "--config",
        "dirule.toml",
        "--target",
        "missing-target",
        "--format",
        "json",
    ]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("target error is a JSON document");
    assert_eq!(value["schema_version"], "1.3");
    assert_eq!(value["error"]["code"], "command_error");
    assert_eq!(value["error"]["kind"], "target");
    assert_eq!(value["error"]["exit_code"], 2);
}

#[test]
fn json_journal_errors_are_classified() {
    let project = TempProject::new();

    let output = project.run(&["undo", "invalid/id", "--yes", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("journal error is a JSON document");
    assert_eq!(value["schema_version"], "1.3");
    assert_eq!(value["error"]["kind"], "journal");
}

#[test]
fn json_config_selection_errors_are_classified() {
    let project = TempProject::new();
    let target = project.0.join("target");
    fs::create_dir_all(&target).expect("create target");
    let preset_directory = preset_config_base(&project).join("dirule/presets");
    fs::create_dir_all(&preset_directory).expect("create preset directory");
    let config = config_for_target(Some(&target), "snake-case");
    fs::write(preset_directory.join("first.toml"), &config).expect("write first preset");
    fs::write(preset_directory.join("second.toml"), config).expect("write second preset");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let output = command
        .args(["plan", "--target", "target", "--format", "json"])
        .current_dir(&project.0)
        .output()
        .expect("run ambiguous target lookup");

    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("selection error is a JSON document");
    assert_eq!(value["schema_version"], "1.3");
    assert_eq!(value["error"]["kind"], "configuration_selection");
}

#[test]
fn commands_do_not_search_parent_directories_for_a_config() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    let child = project.0.join("nested");
    fs::create_dir(&child).expect("create nested working directory");

    let output = Command::new(env!("CARGO_BIN_EXE_dirule"))
        .args(["plan"])
        .current_dir(&child)
        .output()
        .expect("run plan from nested directory");

    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains(&child.join("dirule.toml").display().to_string()),
        "unexpected error: {error}"
    );
    assert!(error.contains("run `dirule init`"));
}

#[test]
fn target_local_config_precedes_exact_preset_and_explicit_sources_override_both() {
    let project = TempProject::new();
    let target = project.0.join("nested-project");
    fs::create_dir(&target).expect("create target directory");
    fs::write(target.join("MyXMLParser.txt"), "parser").expect("write target file");
    fs::write(
        target.join("dirule.toml"),
        config_for_target(Some(Path::new(".")), "snake-case"),
    )
    .expect("write target-local config");
    let preset_directory = preset_config_base(&project).join("dirule/presets");
    fs::create_dir_all(&preset_directory).expect("create global preset directory");
    fs::write(
        preset_directory.join("matching.toml"),
        config_for_target(Some(&target), "kebab-case"),
    )
    .expect("write target-bound preset");
    fs::write(
        preset_directory.join("base.toml"),
        config_for_target(Some(&project.0), "kebab-case"),
    )
    .expect("write invocation-directory preset");
    fs::write(
        project.0.join("custom.toml"),
        config_for_target(Some(Path::new(".")), "camel-case"),
    )
    .expect("write explicit config");
    let target_argument = target.to_string_lossy().into_owned();

    let mut local_command =
        project.command(&["plan", "--target", &target_argument, "--format", "json"]);
    configure_preset_environment(&mut local_command, &project);
    let local = local_command
        .output()
        .expect("run with target-local config");
    assert!(
        local.status.success(),
        "{}",
        String::from_utf8_lossy(&local.stderr)
    );
    let local: serde_json::Value = serde_json::from_slice(&local.stdout).expect("valid local plan");
    assert!(local["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "MyXMLParser.txt" && entry["destination"] == "my_xml_parser.txt"
    }));

    let preset_only_target = project.0.join("preset-only-project");
    fs::create_dir(&preset_only_target).expect("create preset-only target");
    fs::write(preset_only_target.join("MyXMLParser.txt"), "parser")
        .expect("write preset-only target file");
    fs::write(
        preset_directory.join("preset-only.toml"),
        config_for_target(Some(&preset_only_target), "pascal-case"),
    )
    .expect("write preset-only target binding");
    let preset_only_argument = preset_only_target.to_string_lossy().into_owned();
    let mut implicit_preset_command = project.command(&[
        "plan",
        "--target",
        &preset_only_argument,
        "--format",
        "json",
    ]);
    configure_preset_environment(&mut implicit_preset_command, &project);
    let implicit_preset = implicit_preset_command
        .output()
        .expect("run with implicitly selected preset");
    assert!(
        implicit_preset.status.success(),
        "{}",
        String::from_utf8_lossy(&implicit_preset.stderr)
    );
    let implicit_preset: serde_json::Value =
        serde_json::from_slice(&implicit_preset.stdout).expect("valid implicit preset plan");
    assert!(
        implicit_preset["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["source"] == "MyXMLParser.txt" && entry["destination"] == "MyXmlParser.txt"
            })
    );

    let mut preset_command = project.command(&[
        "plan",
        "--target",
        &target_argument,
        "--preset",
        "matching",
        "--format",
        "json",
    ]);
    configure_preset_environment(&mut preset_command, &project);
    let preset = preset_command.output().expect("run with explicit preset");
    assert!(
        preset.status.success(),
        "{}",
        String::from_utf8_lossy(&preset.stderr)
    );
    let preset: serde_json::Value =
        serde_json::from_slice(&preset.stdout).expect("valid preset plan");
    assert!(preset["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "MyXMLParser.txt" && entry["destination"] == "my-xml-parser.txt"
    }));

    let mut config_command = project.command(&[
        "plan",
        "--target",
        &target_argument,
        "--config",
        "custom.toml",
        "--format",
        "json",
    ]);
    configure_preset_environment(&mut config_command, &project);
    let config = config_command.output().expect("run with explicit config");
    assert!(
        config.status.success(),
        "{}",
        String::from_utf8_lossy(&config.stderr)
    );
    let config: serde_json::Value =
        serde_json::from_slice(&config.stdout).expect("valid config plan");
    assert!(config["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "MyXMLParser.txt" && entry["destination"] == "myXmlParser.txt"
    }));

    let mut validate_command = project.command(&["validate", "--format", "json"]);
    configure_preset_environment(&mut validate_command, &project);
    let validation = validate_command
        .output()
        .expect("validate automatically selected preset");
    assert!(
        validation.status.success(),
        "{}",
        String::from_utf8_lossy(&validation.stderr)
    );
    let validation: serde_json::Value =
        serde_json::from_slice(&validation.stdout).expect("valid validation result");
    assert_eq!(validation["source"]["kind"], "preset");
    assert_eq!(validation["source"]["name"], "base");
}

#[test]
fn preset_listing_uses_the_platform_config_directory() {
    let project = TempProject::new();
    #[cfg(target_os = "windows")]
    let config_directory = project.0.join("appdata");
    #[cfg(target_os = "macos")]
    let config_directory = preset_config_base(&project);
    #[cfg(all(unix, not(target_os = "macos")))]
    let config_directory = project.0.join(".config");

    let presets = config_directory.join("dirule/presets");
    fs::create_dir_all(&presets).expect("create user preset directory");
    fs::write(presets.join("downloads.toml"), "preset").expect("write preset fixture");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    #[cfg(target_os = "windows")]
    command.env("APPDATA", &config_directory);
    #[cfg(target_os = "macos")]
    command.env("HOME", project.0.with_extension("home"));
    #[cfg(all(unix, not(target_os = "macos")))]
    command.env("HOME", &project.0).env("XDG_CONFIG_HOME", "");

    let output = command
        .args(["presets", "list"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("downloads"),
        "expected the preset listing to include downloads; stdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn validate_checks_config_without_scanning_its_target() {
    let project = TempProject::new();
    let config = "version = \"1.0\"\ntarget = \"missing-target\"\n\n[[rules]]\nname = \"Normalize\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n";
    fs::write(project.0.join("dirule.toml"), config).expect("write config with missing target");

    let output = project.run(&["validate", "--format", "json"]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid validation JSON");
    assert_eq!(result["schema_version"], "1.3");
    assert_eq!(result["valid"], true);
    assert_eq!(result["source"]["kind"], "config");
    assert_eq!(result["source"]["name"], "dirule.toml");
    assert_eq!(result["rule_count"], 1);
    assert_eq!(
        fs::read_to_string(project.0.join("dirule.toml")).expect("read config after validation"),
        config
    );
    assert!(!project.0.join("missing-target").exists());
    assert!(!project.journal_directory().exists());
}

#[test]
fn validate_loads_a_preset_without_a_project_config() {
    let project = TempProject::new();
    let presets = preset_config_base(&project).join("dirule/presets");
    fs::create_dir_all(&presets).expect("create user preset directory");
    fs::write(
        presets.join("offline.toml"),
        "version = \"1.0\"\ntarget = \"missing-target\"\n\n[[rules]]\nname = \"Normalize\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write user preset");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let output = command
        .args(["validate", "--preset", "offline"])
        .current_dir(&project.0)
        .output()
        .expect("validate user preset");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Preset is valid: offline (1 rule)"));
    assert!(!project.0.join("missing-target").exists());
}

#[test]
fn validate_reports_invalid_configuration_without_scanning() {
    let project = TempProject::new();
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\ntarget = \"missing-target\"\n\n[[rules]]\nname = \"Invalid pattern\"\nmatch = \"<first><last>.txt\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write invalid config");

    let output = project.run(&["validate"]);

    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("invalid configuration"));
    assert!(error.contains("dirule.toml"));
    assert!(!project.0.join("missing-target").exists());
}

#[test]
fn preset_listing_emits_versioned_json() {
    let project = TempProject::new();
    let presets = preset_config_base(&project).join("dirule/presets");
    fs::create_dir_all(&presets).expect("create user preset directory");
    fs::write(presets.join("zeta.toml"), "preset").expect("write later preset");
    fs::write(presets.join("downloads.toml"), "preset").expect("write earlier preset");
    fs::write(presets.join("not a valid name.toml"), "preset").expect("write invalid preset name");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let output = command
        .args(["presets", "list", "--format", "json"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid preset listing JSON");
    assert_eq!(value["schema_version"], "1.3");
    assert_eq!(value["presets"], serde_json::json!(["downloads", "zeta"]));
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn preset_storage_ignores_a_relative_xdg_config_home() {
    let project = TempProject::new();
    let fallback_presets = project.0.join(".config/dirule/presets");
    fs::create_dir_all(&fallback_presets).expect("create fallback preset directory");
    fs::write(fallback_presets.join("downloads.toml"), "preset").expect("write fallback preset");

    let relative_presets = project.0.join("relative-config/dirule/presets");
    fs::create_dir_all(&relative_presets).expect("create relative preset directory");
    fs::write(relative_presets.join("unexpected.toml"), "preset").expect("write relative preset");

    let output = Command::new(env!("CARGO_BIN_EXE_dirule"))
        .env("HOME", &project.0)
        .env("XDG_CONFIG_HOME", "relative-config")
        .args(["presets", "list"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("downloads"));
    assert!(!output.contains("unexpected"));
}

#[test]
fn preset_storage_rejects_relative_platform_directories() {
    let project = TempProject::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    #[cfg(target_os = "windows")]
    command.env("APPDATA", "relative-app-data");
    #[cfg(target_os = "macos")]
    command.env("HOME", "relative-home");
    #[cfg(all(unix, not(target_os = "macos")))]
    command
        .env("HOME", "relative-home")
        .env("XDG_CONFIG_HOME", "");

    let output = command
        .args(["presets", "list"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("user configuration directory"));
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
fn preset_save_copies_a_valid_config_without_overwriting_existing_presets() {
    let project = TempProject::new();
    let init = project.run(&["init"]);
    assert!(init.status.success());
    let config = text(&project.0.join("dirule.toml"));
    let preset = preset_config_base(&project).join("dirule/presets/downloads.toml");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let save = command
        .args(["presets", "save", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("run preset save");

    assert!(
        save.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&save.stdout),
        String::from_utf8_lossy(&save.stderr)
    );
    assert_eq!(text(&preset), config);

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let duplicate = command
        .args(["presets", "save", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("run duplicate preset save");

    assert_eq!(duplicate.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already exists"));
    assert_eq!(text(&preset), config);

    let alternate_config = project.0.join("custom-rules.toml");
    fs::write(&alternate_config, &config).expect("write alternate config");
    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let save_alternate = command
        .args(["presets", "save", "custom", "--config", "custom-rules.toml"])
        .current_dir(&project.0)
        .output()
        .expect("save alternate config");

    assert!(save_alternate.status.success());
    assert_eq!(
        text(&preset_config_base(&project).join("dirule/presets/custom.toml")),
        config
    );

    fs::write(&alternate_config, "version = \"future\"\n").expect("write invalid config");
    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let save_invalid = command
        .args([
            "presets",
            "save",
            "invalid",
            "--config",
            "custom-rules.toml",
        ])
        .current_dir(&project.0)
        .output()
        .expect("reject invalid config");

    assert_eq!(save_invalid.status.code(), Some(2));
    assert!(
        !preset_config_base(&project)
            .join("dirule/presets/invalid.toml")
            .exists()
    );
}

#[test]
fn preset_export_preserves_config_for_transfer_and_save() {
    let project = TempProject::new();
    let initialized = project.run(&["init"]);
    assert!(initialized.status.success());
    let config = text(&project.0.join("dirule.toml"));

    let mut save = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut save, &project);
    let saved = save
        .args(["presets", "save", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("save preset");
    assert!(saved.status.success());

    let mut export = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut export, &project);
    let exported = export
        .args(["--color", "always", "presets", "export", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("export preset");

    assert!(
        exported.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&exported.stdout),
        String::from_utf8_lossy(&exported.stderr)
    );
    assert_eq!(exported.stdout, config.as_bytes());
    assert!(exported.stderr.is_empty());

    let portable_config = project.0.join("portable.toml");
    fs::write(&portable_config, exported.stdout).expect("write exported config");

    let mut import = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut import, &project);
    let imported = import
        .args(["presets", "save", "imported", "--config", "portable.toml"])
        .current_dir(&project.0)
        .output()
        .expect("save exported config as another preset");

    assert!(imported.status.success());
    assert_eq!(
        text(&preset_config_base(&project).join("dirule/presets/imported.toml")),
        config
    );
}

#[test]
fn preset_export_rejects_invalid_config_without_writing_stdout() {
    let project = TempProject::new();
    let presets = preset_config_base(&project).join("dirule/presets");
    fs::create_dir_all(&presets).expect("create user preset directory");
    fs::write(presets.join("invalid.toml"), "version = \"future\"\n")
        .expect("write invalid preset");

    let mut export = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut export, &project);
    let output = export
        .args(["presets", "export", "invalid"])
        .current_dir(&project.0)
        .output()
        .expect("export invalid preset");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid configuration"));
}

#[test]
fn plan_loads_a_saved_preset_without_a_project_config() {
    let project = TempProject::new();
    fs::write(project.0.join("MyXMLParser.pdf"), "document").expect("write fixture file");
    assert!(project.run(&["init"]).status.success());

    let mut save = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut save, &project);
    let saved = save
        .args(["presets", "save", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("save preset");
    assert!(
        saved.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&saved.stdout),
        String::from_utf8_lossy(&saved.stderr)
    );
    fs::remove_file(project.0.join("dirule.toml")).expect("remove project config");

    let mut plan = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut plan, &project);
    let output = plan
        .args(["plan", "--preset", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("plan with saved preset");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("MyXMLParser.pdf -> my-xml-parser.pdf")
    );
}

#[test]
fn preset_remove_requires_confirmation_without_an_interactive_terminal() {
    let project = TempProject::new();
    let preset = preset_config_base(&project).join("dirule/presets/downloads.toml");
    fs::create_dir_all(preset.parent().expect("preset parent"))
        .expect("create user preset directory");
    fs::write(&preset, "preset contents").expect("write preset");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let rejected = command
        .args(["presets", "remove", "downloads"])
        .current_dir(&project.0)
        .output()
        .expect("require remove confirmation");

    assert_eq!(rejected.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("use --yes to remove"));
    assert!(preset.is_file());

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_preset_environment(&mut command, &project);
    let removed = command
        .args(["presets", "remove", "downloads", "--yes"])
        .current_dir(&project.0)
        .output()
        .expect("remove preset");

    assert!(removed.status.success());
    assert!(String::from_utf8_lossy(&removed.stdout).contains("Removed preset downloads"));
    assert!(!preset.exists());
}

#[test]
fn explicit_target_overrides_the_configured_target() {
    let project = TempProject::new();
    let configured = project.0.join("configured");
    let override_target = project.0.join("override");
    fs::create_dir_all(&configured).expect("create configured target");
    fs::create_dir_all(&override_target).expect("create override target");
    fs::write(configured.join("OtherFile.txt"), "configured").expect("write configured file");
    fs::write(override_target.join("MyXMLParser.txt"), "override").expect("write target file");
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\ntarget = \"configured\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write configuration");

    let canonical_override_target = override_target
        .canonicalize()
        .expect("canonicalize explicit target");

    let plan = Command::new(env!("CARGO_BIN_EXE_dirule"))
        .args(["plan", "--target", "override", "--config", "dirule.toml"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert!(plan.status.success());
    let output = String::from_utf8_lossy(&plan.stdout);
    assert!(output.contains(&format!("Target: {}", canonical_override_target.display())));
    assert!(output.contains("MyXMLParser.txt -> my-xml-parser.txt"));
    assert!(!output.contains("OtherFile.txt"));
}

#[test]
fn target_expands_home_for_configured_defaults_and_cli_overrides() {
    let project = TempProject::new();
    let home = project.0.join("home");
    let downloads = home.join("Downloads");
    let archive = home.join("Archive");
    #[cfg(target_os = "windows")]
    let config_base = project.0.join("appdata");
    #[cfg(target_os = "macos")]
    let config_base = home.join("Library/Application Support");
    #[cfg(all(unix, not(target_os = "macos")))]
    let config_base = project.0.join("xdg-config");

    fs::create_dir_all(&downloads).expect("create configured target");
    fs::create_dir_all(&archive).expect("create override target");
    fs::write(downloads.join("MyFile.txt"), "download").expect("write configured target file");
    fs::write(archive.join("OtherFile.txt"), "archive").expect("write override target file");
    let presets = config_base.join("dirule/presets");
    fs::create_dir_all(&presets).expect("create global preset directory");
    fs::write(
        presets.join("downloads.toml"),
        "version = \"1.0\"\ntarget = \"~/Downloads\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write global preset");

    let mut configured_command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_home(&mut configured_command, &home);
    #[cfg(target_os = "windows")]
    configured_command.env("APPDATA", &config_base);
    #[cfg(target_os = "macos")]
    configured_command.env("HOME", &home);
    #[cfg(all(unix, not(target_os = "macos")))]
    configured_command.env("XDG_CONFIG_HOME", &config_base);
    let configured = configured_command
        .args(["plan", "--preset", "downloads", "--format", "json"])
        .current_dir(&project.0)
        .output()
        .expect("plan configured home target");
    assert!(
        configured.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&configured.stdout),
        String::from_utf8_lossy(&configured.stderr)
    );
    let configured: serde_json::Value =
        serde_json::from_slice(&configured.stdout).expect("valid configured target plan");
    assert_eq!(
        configured["target"],
        downloads.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert!(
        configured["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["source"] == "MyFile.txt" && entry["destination"] == "my-file.txt"
            })
    );

    let mut override_command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_home(&mut override_command, &home);
    #[cfg(target_os = "windows")]
    override_command.env("APPDATA", &config_base);
    #[cfg(target_os = "macos")]
    override_command.env("HOME", &home);
    #[cfg(all(unix, not(target_os = "macos")))]
    override_command.env("XDG_CONFIG_HOME", &config_base);
    let override_plan = override_command
        .args([
            "plan",
            "--target",
            "~/Archive",
            "--preset",
            "downloads",
            "--format",
            "json",
        ])
        .current_dir(&project.0)
        .output()
        .expect("plan explicit home target");
    assert!(
        override_plan.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&override_plan.stdout),
        String::from_utf8_lossy(&override_plan.stderr)
    );
    let override_plan: serde_json::Value =
        serde_json::from_slice(&override_plan.stdout).expect("valid explicit target plan");
    assert_eq!(
        override_plan["target"],
        archive.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert!(
        override_plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| {
                entry["source"] == "OtherFile.txt" && entry["destination"] == "other-file.txt"
            })
    );
}

#[cfg(unix)]
#[test]
fn target_preserves_backslashes_in_unix_home_relative_paths() {
    let project = TempProject::new();
    let home = project.0.join("home");
    let target_name = format!("{}Archive", '\\');
    let target = home.join(&target_name);
    fs::create_dir_all(&target).expect("create target with a leading backslash");
    fs::write(target.join("MyFile.txt"), "content").expect("write target file");
    fs::write(
        project.0.join("rules.toml"),
        "version = \"1.0\"\n\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write rules");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    configure_home(&mut command, &home);
    let output = command
        .args([
            "plan",
            "--target",
            "~/\\Archive",
            "--config",
            "rules.toml",
            "--format",
            "json",
        ])
        .current_dir(&project.0)
        .output()
        .expect("plan target containing a backslash");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan");
    assert_eq!(
        plan["target"],
        target.canonicalize().unwrap().to_string_lossy().as_ref()
    );
}

#[test]
fn plan_json_reports_conflicts_without_mutating_the_target() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("A__B.txt"), "first").expect("write first conflict source");
    fs::write(project.0.join("A--B.txt"), "second").expect("write second conflict source");

    let output = project.run(&["plan", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");
    let conflicts = value["conflicts"].as_array().expect("conflicts array");
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["path"], "a-b.txt");
    assert!(conflicts[0]["reason"].is_string());
    assert!(project.0.join("A__B.txt").is_file());
    assert!(project.0.join("A--B.txt").is_file());
    assert!(!project.journal_directory().exists());
}

#[test]
fn plan_json_conflicts_identify_the_kind_and_related_paths() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("A__B.txt"), "first").expect("write first conflict source");
    fs::write(project.0.join("A--B.txt"), "second").expect("write second conflict source");

    let output = project.run(&["plan", "--format", "json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");
    let conflict = &value["conflicts"][0];

    assert_eq!(conflict["kind"], "duplicate_destination");
    let related = conflict["related_paths"]
        .as_array()
        .expect("related paths array")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(related, ["A--B.txt", "A__B.txt", "a-b.txt"]);
}

#[test]
fn plan_json_occupied_destination_identifies_the_move_and_existing_path() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("MyFile.txt"), "source").expect("write source");
    fs::write(project.0.join("my-file.txt"), "destination").expect("write destination");

    let output = project.run(&["plan", "--format", "json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");
    let conflict = &value["conflicts"][0];

    assert_eq!(conflict["kind"], "destination_occupied");
    assert_eq!(conflict["path"], "my-file.txt");
    assert_eq!(
        conflict["related_paths"],
        serde_json::json!(["MyFile.txt", "my-file.txt"])
    );
    assert!(conflict["suggestion"].is_string());
    assert_eq!(text(&project.0.join("MyFile.txt")), "source");
    assert_eq!(text(&project.0.join("my-file.txt")), "destination");
}

#[test]
fn plan_text_conflicts_show_related_paths_and_a_suggestion() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("A__B.txt"), "first").expect("write first conflict source");
    fs::write(project.0.join("A--B.txt"), "second").expect("write second conflict source");

    let output = project.run(&["plan"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(output.status.code(), Some(2));
    assert!(stdout.contains("Related paths: A--B.txt, A__B.txt, a-b.txt"));
    assert!(
        stdout.contains("Suggestion: Adjust the rules so each entry maps to a unique destination.")
    );
}

#[test]
fn check_reports_conflicts_without_mutating_the_target() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("A__B.txt"), "first").expect("write first conflict source");
    fs::write(project.0.join("A--B.txt"), "second").expect("write second conflict source");

    let check = project.run(&["check"]);

    assert_eq!(check.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&check.stdout).contains("1 conflict(s)"));
    assert!(String::from_utf8_lossy(&check.stderr).contains("target does not satisfy"));
    assert_eq!(text(&project.0.join("A__B.txt")), "first");
    assert_eq!(text(&project.0.join("A--B.txt")), "second");
    assert!(!project.0.join("a-b.txt").exists());
    assert!(!project.journal_directory().exists());
}

#[test]
fn check_json_reports_violations_with_failure_status_without_mutation() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("MyFile.txt"), "content").expect("write nonconforming file");

    let check = project.run(&["check", "--format", "json"]);

    assert_eq!(check.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&check.stdout).expect("valid check JSON");
    assert_eq!(value["schema_version"], "1.3");
    let entries = value["entries"].as_array().expect("entries array");
    assert!(
        entries.iter().any(|entry| {
            entry["source"] == "MyFile.txt" && entry["destination"] == "my-file.txt"
        })
    );
    assert!(
        value["conflicts"]
            .as_array()
            .expect("conflicts array")
            .is_empty()
    );
    let error: serde_json::Value =
        serde_json::from_slice(&check.stderr).expect("check failure is a JSON document");
    assert_eq!(error["schema_version"], "1.3");
    assert_eq!(error["error"]["code"], "check_failed");
    assert_eq!(error["error"]["kind"], "check_failure");
    assert_eq!(error["error"]["exit_code"], 1);
    assert!(project.0.join("MyFile.txt").is_file());
    assert!(!project.0.join("my-file.txt").exists());
    assert!(!project.journal_directory().exists());
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
fn cli_checks_a_real_project_config_without_mutating_it() {
    let project = TempProject::new();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invoice-project");
    copy_tree(&fixture, &project.0);
    let source_path = project.0.join("Invoices/2026-Acme-Quarter 1.pdf");
    let original_contents = fs::read(&source_path).expect("read source fixture");

    let check = project.run(&["check"]);
    assert_eq!(check.status.code(), Some(1));
    let output = String::from_utf8_lossy(&check.stdout).replace('\\', "/");
    assert!(
        output.contains("Invoices/2026-Acme-Quarter 1.pdf -> Archive/2026/acme/quarter-1.pdf"),
        "unexpected check output: {output}"
    );
    assert!(String::from_utf8_lossy(&check.stderr).contains("target does not satisfy"));
    assert_eq!(
        fs::read(&source_path).expect("read source fixture after check"),
        original_contents
    );
    assert!(!project.0.join("Archive/2026/acme/quarter-1.pdf").exists());
}

#[test]
fn verbose_plan_explains_rule_ownership_and_inherited_directory_paths() {
    let project = TempProject::new();
    fs::create_dir(project.0.join("HTTPServer")).expect("create source directory");
    fs::write(project.0.join("HTTPServer/ReadMe.txt"), "notes").expect("write nested source file");
    fs::write(project.0.join("MyXMLParser.pdf"), "document").expect("write root source file");
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize directory\"\nmatch = \"HTTPServer\"\nto = \"builtin:kebab-case\"\n\n[[rules]]\nname = \"Normalize parser file\"\nmatch = \"MyXMLParser.pdf\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write fixture config");

    let plan = project.run(&["plan"]);
    assert!(plan.status.success());
    assert!(!String::from_utf8_lossy(&plan.stdout).contains("[rule:"));

    let verbose = project.run(&["plan", "--verbose"]);
    assert!(verbose.status.success());
    let output = String::from_utf8_lossy(&verbose.stdout).replace('\\', "/");
    assert!(output.contains("HTTPServer -> http-server [rule: Normalize directory]"));
    assert!(output.contains(
        "HTTPServer/ReadMe.txt -> http-server/ReadMe.txt [inherited from directory rename]"
    ));
    assert!(output.contains("MyXMLParser.pdf -> my-xml-parser.pdf [rule: Normalize parser file]"));
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

#[cfg(unix)]
#[test]
fn cli_reports_a_skipped_symlink_once() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new();
    fs::write(project.0.join("target.txt"), "target").expect("write target file");
    symlink("target.txt", project.0.join("MyLink")).expect("create symlink");
    assert!(project.run(&["init"]).status.success());

    let plan = project.run(&["plan"]);
    assert!(plan.status.success());
    let output = String::from_utf8_lossy(&plan.stdout);
    assert!(output.contains("1 finding(s)"));
    assert_eq!(output.matches("skipped symlink MyLink").count(), 1);
}

#[cfg(unix)]
#[test]
fn plan_json_keeps_finding_messages_and_exposes_structured_diagnostics() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new();
    fs::write(project.0.join("target.txt"), "target").expect("write target file");
    symlink("target.txt", project.0.join("MyLink")).expect("create symlink");
    assert!(project.run(&["init"]).status.success());

    let output = project.run(&["plan", "--format", "json"]);
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");

    assert_eq!(value["schema_version"], "1.3");
    assert_eq!(
        value["findings"],
        serde_json::json!(["skipped symlink MyLink"])
    );
    assert_eq!(
        value["diagnostics"],
        serde_json::json!([{
            "kind": "skipped_symlink",
            "path": "MyLink",
            "message": "skipped symlink MyLink",
            "suggestion": "Dirule skips symlinks; manage their targets separately."
        }])
    );
}
