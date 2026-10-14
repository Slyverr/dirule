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
    assert_eq!(value["schema_version"], "1.0");
    assert_eq!(value["error"]["code"], "command_error");
    assert_eq!(value["error"]["exit_code"], 2);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("rules.toml")
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
    assert!(error.contains(&child.join("dirule.toml").display().to_string()));
    assert!(error.contains("run `dirule init`"));
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
    fs::write(
        project.0.join("dirule.toml"),
        "version = \"1.0\"\ntarget = \"missing-target\"\n\n[[rules]]\nname = \"Normalize\"\nmatch = \"**/*\"\nto = \"builtin:kebab-case\"\n",
    )
    .expect("write config with missing target");

    let output = project.run(&["validate", "--format", "json"]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid validation JSON");
    assert_eq!(result["schema_version"], "1.0");
    assert_eq!(result["valid"], true);
    assert_eq!(result["source"]["kind"], "config");
    assert_eq!(result["source"]["name"], "dirule.toml");
    assert_eq!(result["rule_count"], 1);
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
        .args(["plan", "--target", "override"])
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
    assert_eq!(value["schema_version"], "1.0");
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
    assert_eq!(error["schema_version"], "1.0");
    assert_eq!(error["error"]["code"], "check_failed");
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
