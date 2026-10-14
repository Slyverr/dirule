use super::common::*;

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
fn init_preserves_an_existing_config() {
    let project = TempProject::new();
    let config = project.0.join("dirule.toml");
    fs::write(&config, "user-owned config\n").expect("write existing config");

    let init = project.run(&["init"]);

    assert_eq!(init.status.code(), Some(2));
    assert_eq!(text(&config), "user-owned config\n");
    assert!(String::from_utf8_lossy(&init.stderr).contains("cannot create"));
}
