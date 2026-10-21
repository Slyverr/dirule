use super::common::*;

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
fn json_command_errors_emit_a_versioned_error_document_to_stderr() {
    let project = TempProject::new();
    fs::write(project.0.join("rules.toml"), "version = 12\n").expect("write malformed config");

    let output = project.run(&["plan", "--config", "rules.toml", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("command error is a JSON document");
    assert_eq!(value["schema_version"], "1.5");
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
    assert_eq!(value["schema_version"], "1.5");
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
    assert_eq!(value["schema_version"], "1.5");
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
    assert_eq!(value["schema_version"], "1.5");
    assert_eq!(value["error"]["kind"], "configuration_selection");
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
    assert_eq!(value["schema_version"], "1.5");
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
    assert_eq!(error["schema_version"], "1.5");
    assert_eq!(error["error"]["code"], "check_failed");
    assert_eq!(error["error"]["kind"], "check_failure");
    assert_eq!(error["error"]["exit_code"], 1);
    assert!(project.0.join("MyFile.txt").is_file());
    assert!(!project.0.join("my-file.txt").exists());
    assert!(!project.journal_directory().exists());
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
fn check_passes_when_only_skipped_symlinks_are_found() {
    use std::os::unix::fs::symlink;

    let project = TempProject::new();
    fs::write(project.0.join("target.txt"), "target").expect("write target file");
    symlink("target.txt", project.0.join("link")).expect("create symlink");
    assert!(project.run(&["init"]).status.success());

    let output = project.run(&["check", "--format", "json"]);
    assert!(output.status.success());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid check JSON");

    assert_eq!(value["diagnostics"][0]["kind"], "skipped_symlink");
    assert_eq!(value["diagnostics"][0]["path"], "link");
    assert!(
        project
            .0
            .join("link")
            .symlink_metadata()
            .expect("inspect symlink")
            .file_type()
            .is_symlink()
    );
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

    assert_eq!(value["schema_version"], "1.5");
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

#[cfg(unix)]
#[test]
fn plan_json_reports_non_regular_entry_diagnostics() {
    use std::os::unix::net::UnixListener;

    let project = TempProject::new();
    let _listener =
        UnixListener::bind(project.0.join("service.sock")).expect("create socket fixture");
    assert!(project.run(&["init"]).status.success());

    let output = project.run(&["plan", "--format", "json"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");

    assert!(
        value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["kind"] == "skipped_non_regular_entry"
                    && diagnostic["path"] == "service.sock"
                    && diagnostic["suggestion"].is_string()
            })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn plan_json_reports_non_unicode_path_diagnostics() {
    use std::os::unix::ffi::OsStringExt;

    let project = TempProject::new();
    let name = std::ffi::OsString::from_vec(vec![b'f', 0xff]);
    fs::write(project.0.join(name), "non-Unicode path").expect("write non-Unicode file");
    assert!(project.run(&["init"]).status.success());

    let output = project.run(&["plan", "--format", "json"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");

    assert!(
        value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["kind"] == "skipped_non_unicode_path" && diagnostic["path"].is_null()
            })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn plan_json_reports_git_tracked_non_unicode_path_diagnostics() {
    use std::os::unix::ffi::OsStringExt;

    let project = TempProject::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project.0)
        .status()
        .expect("initialize Git fixture");
    assert!(init.success(), "initialize Git fixture");

    let name = std::ffi::OsString::from_vec(vec![b'f', 0xff]);
    fs::write(project.0.join(name), "non-Unicode path").expect("write non-Unicode file");
    let add = Command::new("git")
        .args(["add", "--all"])
        .current_dir(&project.0)
        .status()
        .expect("stage non-Unicode fixture");
    assert!(add.success(), "stage non-Unicode fixture");
    assert!(project.run(&["init"]).status.success());

    let output = project.run(&["plan", "--format", "json"]);
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");

    assert!(
        value["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["kind"] == "skipped_git_non_unicode_path" && diagnostic["path"].is_null()
            })
    );
}

#[test]
fn cli_plan_shows_language_server_reference_diffs() {
    let project = lsp_fixture_project();

    let output = project.run(&["plan"]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Reference updates:"), "{stdout}");
    assert!(
        stdout.contains(&Path::new("src/api.rs").display().to_string()),
        "{stdout}"
    );
    assert!(
        stdout.contains("- use crate::{HTTPClient::Client as ApiClient, model::Record};"),
        "{stdout}"
    );
    assert!(
        stdout.contains("+ use crate::{http_client::Client as ApiClient, model::Record};"),
        "{stdout}"
    );
    assert!(stdout.contains("- pub mod HTTPClient;"), "{stdout}");
    assert!(stdout.contains("+ pub mod http_client;"), "{stdout}");
    assert!(project.0.join("src/HTTPClient.rs").is_file());
    assert!(!project.0.join("src/http_client.rs").exists());
}

#[test]
fn cli_plan_json_includes_language_server_reference_edits() {
    let project = lsp_fixture_project();

    let output = project.run(&["plan", "--format", "json"]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan JSON");
    assert_eq!(plan["reference_updates"]["warnings"], serde_json::json!([]));
    let files = plan["reference_updates"]["files"]
        .as_array()
        .expect("reference update files");
    let lib = files
        .iter()
        .find(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| Path::new(path).ends_with(Path::new("src/lib.rs")))
        })
        .expect("lib.rs reference edit");
    assert_eq!(
        lib["before"],
        "pub mod HTTPClient;\npub use HTTPClient::Client as SharedClient;\n"
    );
    assert_eq!(
        lib["after"],
        "pub mod http_client;\npub use http_client::Client as SharedClient;\n"
    );
    let api = files
        .iter()
        .find(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| Path::new(path).ends_with(Path::new("src/api.rs")))
        })
        .expect("api.rs reference edit");
    assert_eq!(
        api["after"],
        "use crate::{http_client::Client as ApiClient, model::Record};\n"
    );
    assert!(project.0.join("src/HTTPClient.rs").is_file());
    assert!(!project.0.join("src/http_client.rs").exists());
}

#[test]
fn cli_plan_fails_when_required_language_server_is_missing() {
    let project = TempProject::new();
    fs::write(project.0.join("HTTPClient.rs"), "pub struct Client;\n")
        .expect("write Rust source fixture");
    fs::write(
        project.0.join("dirule.toml"),
        format!(
            r#"version = "{}"
target = "."

[lsp]
reference_updates = "required"

[[rules]]
name = "Normalize Rust module filename"
match = "HTTPClient.rs"
kind = "file"
to = "builtin:snake-case"

[[lsp.servers]]
command = "dirule-test-missing-lsp-server-8f0a66d2"

[lsp.servers.languages]
rs = "rust"
"#,
            dirule_core::CURRENT_CONFIG_VERSION
        ),
    )
    .expect("write required LSP config");

    let output = project.run(&["plan"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("required language-server reference update failed"),
        "{stderr}"
    );
    assert!(stderr.contains("dirule-test-missing-lsp-server-8f0a66d2"));
    assert!(project.0.join("HTTPClient.rs").is_file());
    assert!(!project.0.join("http_client.rs").exists());
}

fn lsp_fixture_project() -> TempProject {
    let project = TempProject::new();
    fs::create_dir(project.0.join("src")).expect("create Rust source directory");
    fs::write(
        project.0.join("Cargo.toml"),
        "[package]\nname = \"dirule-cli-lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo workspace marker");
    fs::write(project.0.join("src/HTTPClient.rs"), "pub struct Client;\n")
        .expect("write module source");
    fs::write(
        project.0.join("src/lib.rs"),
        "pub mod HTTPClient;\npub use HTTPClient::Client as SharedClient;\n",
    )
    .expect("write module references");
    fs::write(
        project.0.join("src/api.rs"),
        "use crate::{HTTPClient::Client as ApiClient, model::Record};\n",
    )
    .expect("write API reference");
    fs::write(project.0.join("src/model.rs"), "pub struct Record;\n").expect("write model source");

    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cli_flow/lsp_server.rs");
    let executable = project.0.join(if cfg!(windows) {
        "dirule-test-lsp-server.exe"
    } else {
        "dirule-test-lsp-server"
    });
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compiled = Command::new(rustc)
        .arg("--edition=2024")
        .arg(fixture)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("compile CLI LSP fixture server");
    assert!(
        compiled.status.success(),
        "failed to compile LSP fixture server: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let executable = format!("{:?}", executable.to_string_lossy().to_string());
    fs::write(
        project.0.join("dirule.toml"),
        format!(
            r#"version = "{}"
target = "."

[lsp]
reference_updates = "required"

[[rules]]
name = "Normalize Rust module filename"
match = "src/HTTPClient.rs"
kind = "file"
to = "builtin:snake-case"

[[lsp.servers]]
command = {executable}
root_markers = ["Cargo.toml"]

[lsp.servers.languages]
rs = "rust"
"#,
            dirule_core::CURRENT_CONFIG_VERSION
        ),
    )
    .expect("write project LSP config");
    project
}
