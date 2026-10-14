use super::common::*;

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
