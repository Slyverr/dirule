use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempProject(PathBuf);

impl TempProject {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("dirule-cli-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp project");
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
        #[cfg(target_os = "windows")]
        command.env("LOCALAPPDATA", self.0.with_extension("local-app-data"));
        #[cfg(target_os = "macos")]
        command.env("HOME", self.0.with_extension("home"));
        #[cfg(all(unix, not(target_os = "macos")))]
        command.env("XDG_STATE_HOME", self.0.with_extension("state"));
        command
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("run dirule")
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
    project.0.join("Library/Application Support")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn preset_config_base(project: &TempProject) -> PathBuf {
    project.0.join("xdg-config")
}

fn configure_preset_environment(command: &mut Command, project: &TempProject) {
    #[cfg(target_os = "windows")]
    command.env("APPDATA", preset_config_base(project));
    #[cfg(target_os = "macos")]
    command.env("HOME", &project.0);
    #[cfg(all(unix, not(target_os = "macos")))]
    command.env("XDG_CONFIG_HOME", preset_config_base(project));
}

fn configure_home(command: &mut Command, home: &Path) {
    #[cfg(target_os = "windows")]
    command.env("USERPROFILE", home);
    #[cfg(not(target_os = "windows"))]
    command.env("HOME", home);
}

#[test]
fn cli_initializes_checks_plans_applies_and_emits_json() {
    let project = TempProject::new();
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
    assert!(plain_output.contains(&format!("Target: {}", project.0.display())));
    assert!(!plain_output.contains('\u{1b}'));

    let check = project.run(&["check"]);
    assert_eq!(check.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&check.stdout).contains("Check failed"));
    assert!(String::from_utf8_lossy(&check.stderr).contains("target does not satisfy"));

    let json = project.run(&["plan", "--format", "json"]);
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("valid JSON plan");
    assert_eq!(value["target"], project.0.to_string_lossy().as_ref());
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
    assert!(script.contains("presets"));
    assert!(script.contains("--target"));
    assert!(script.contains("--format"));
}

#[test]
fn apply_and_undo_emit_single_json_documents() {
    let project = TempProject::new();
    assert!(project.run(&["init"]).status.success());
    fs::write(project.0.join("NeedsWork.txt"), "contents").expect("write fixture file");

    let apply = project.run(&["apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value =
        serde_json::from_slice(&apply.stdout).expect("one apply JSON document");
    assert_eq!(apply["execution"]["status"], "applied");
    assert_eq!(
        apply["plan"]["target"],
        project.0.to_string_lossy().as_ref()
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
    assert!(error.contains(&child.join("dirule.toml").display().to_string()));
    assert!(error.contains("run `dirule init`"));
}

#[test]
fn preset_listing_uses_the_platform_config_directory() {
    let project = TempProject::new();
    #[cfg(target_os = "windows")]
    let config_directory = project.0.join("appdata");
    #[cfg(target_os = "macos")]
    let config_directory = project.0.join("Library/Application Support");
    #[cfg(all(unix, not(target_os = "macos")))]
    let config_directory = project.0.join(".config");

    let presets = config_directory.join("dirule/presets");
    fs::create_dir_all(&presets).expect("create user preset directory");
    fs::write(presets.join("downloads.toml"), "preset").expect("write preset fixture");

    let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
    #[cfg(target_os = "windows")]
    command.env("APPDATA", &config_directory);
    #[cfg(target_os = "macos")]
    command.env("HOME", &project.0);
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("downloads"));
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
    assert_eq!(value["schema_version"], "1.0");
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

    let plan = Command::new(env!("CARGO_BIN_EXE_dirule"))
        .args(["plan", "--target", "override"])
        .current_dir(&project.0)
        .output()
        .expect("run dirule");

    assert!(plan.status.success());
    let output = String::from_utf8_lossy(&plan.stdout);
    assert!(output.contains(&format!("Target: {}", override_target.display())));
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
}

#[test]
fn cli_checks_a_real_project_config_without_mutating_it() {
    let project = TempProject::new();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invoice-project");
    copy_tree(&fixture, &project.0);

    let check = project.run(&["check"]);
    assert_eq!(check.status.code(), Some(2));
    let output = String::from_utf8_lossy(&check.stdout).replace('\\', "/");
    assert!(
        output.contains("Invoices/2026-Acme-Quarter 1.pdf -> Archive/2026/acme/quarter-1.pdf"),
        "unexpected check output: {output}"
    );
    assert!(String::from_utf8_lossy(&check.stderr).contains("target does not satisfy"));
    assert_eq!(
        text(&project.0.join("Invoices/2026-Acme-Quarter 1.pdf")),
        "invoice contents\n"
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
