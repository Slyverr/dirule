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
        Command::new(env!("CARGO_BIN_EXE_dirule"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("run dirule")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
    assert!(!String::from_utf8_lossy(&plain.stdout).contains('\u{1b}'));

    let check = project.run(&["check"]);
    assert_eq!(check.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&check.stdout).contains("Check failed"));
    assert!(String::from_utf8_lossy(&check.stderr).contains("target does not satisfy"));

    let json = project.run(&["plan", "--format", "json"]);
    assert!(json.status.success());
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("valid JSON plan");
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
    assert_eq!(text(&project.0.join("my-xml-parser.rs")), "parser");
    assert_eq!(
        text(&project.0.join("http-server/o-auth-token.txt")),
        "token"
    );

    let check = project.run(&["check"]);
    assert!(check.status.success());
    assert!(String::from_utf8_lossy(&check.stdout).contains("Check passed"));
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
    assert!(output.contains("MyXMLParser.txt -> my-xml-parser.txt"));
    assert!(!output.contains("OtherFile.txt"));
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
