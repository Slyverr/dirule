use super::common::*;

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
    assert_eq!(value["schema_version"], "1.5");
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
