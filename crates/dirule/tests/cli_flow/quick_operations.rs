use super::common::*;

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
fn quick_sort_accepts_multiple_extensions_and_builtin_categories() {
    let project = TempProject::new();
    fs::write(project.0.join("Portrait.JPG"), "image").expect("write image");
    fs::write(project.0.join("Clip.MP4"), "video").expect("write video");
    fs::write(project.0.join("Guide.PDF"), "document").expect("write document");
    fs::write(project.0.join("Notes.TXT"), "text").expect("write text");

    let preview = project.run(&["sort", "images", "videos", "pdf", "txt", "--format", "json"]);

    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    let preview: serde_json::Value =
        serde_json::from_slice(&preview.stdout).expect("valid categorized sort preview");
    let entries = preview["entries"].as_array().expect("plan entries");
    for (source, destination) in [
        ("Portrait.JPG", "media/images/Portrait.JPG"),
        ("Clip.MP4", "media/videos/Clip.MP4"),
        ("Guide.PDF", "pdfs/Guide.PDF"),
        ("Notes.TXT", "txts/Notes.TXT"),
    ] {
        assert!(
            entries
                .iter()
                .any(|entry| { entry["source"] == source && entry["destination"] == destination })
        );
    }
    assert!(project.0.join("Portrait.JPG").is_file());
    assert!(!project.0.join("media/images/Portrait.JPG").exists());

    let apply = project.run(&[
        "sort", "images", "videos", "pdf", "txt", "--apply", "--yes", "--format", "json",
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
    assert!(project.0.join("media/images/Portrait.JPG").is_file());
    assert!(project.0.join("media/videos/Clip.MP4").is_file());
    assert!(project.0.join("pdfs/Guide.PDF").is_file());
    assert!(project.0.join("txts/Notes.TXT").is_file());

    let undo = project.run(&["undo", run_id, "--yes"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert!(project.0.join("Portrait.JPG").is_file());
}

#[test]
fn quick_sort_loads_user_category_catalog_and_overrides_built_ins() {
    let project = TempProject::new();
    fs::write(project.0.join("Camera.RAW"), "raw").expect("write raw image");
    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create Dirule config directory");
    fs::write(
        config_directory.join("categories.toml"),
        format!(
            "version = \"{}\"\n\
             [[categories]]\nname = 'images'\nextensions = ['raw']\ndestination = 'photos'\n\
             [[categories]]\nname = 'ebooks'\nextensions = ['epub']\ndestination = 'reading/books'\n",
            dirule_app::CURRENT_CATEGORY_CATALOG_VERSION
        ),
    )
    .expect("write user category catalog");

    let mut command = project.command(&["sort", "images", "--format", "json"]);
    configure_preset_environment(&mut command, &project);
    let output = command.output().expect("run category sort");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid plan");
    assert!(plan["entries"].as_array().unwrap().iter().any(|entry| {
        entry["source"] == "Camera.RAW" && entry["destination"] == "photos/Camera.RAW"
    }));
}

#[test]
fn quick_sort_rejects_selected_categories_with_overlapping_extensions() {
    let project = TempProject::new();
    fs::write(project.0.join("Photo.PNG"), "image").expect("write image");
    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create Dirule config directory");
    fs::write(
        config_directory.join("categories.toml"),
        format!(
            "version = \"{}\"\n\
             [[categories]]\nname = 'pictures'\nextensions = ['png']\ndestination = 'pictures'\n",
            dirule_app::CURRENT_CATEGORY_CATALOG_VERSION
        ),
    )
    .expect("write overlapping category");

    let mut command = project.command(&["sort", "images", "pictures", "--format", "json"]);
    configure_preset_environment(&mut command, &project);
    let output = command.output().expect("run overlapping category sort");

    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("valid error document");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overlap")
    );
    assert!(project.0.join("Photo.PNG").is_file());
}

#[test]
fn quick_sort_reports_invalid_category_catalog_as_configuration_error() {
    let project = TempProject::new();
    fs::write(project.0.join("Photo.PNG"), "image").expect("write image");
    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create Dirule config directory");
    fs::write(
        config_directory.join("categories.toml"),
        "version = '2.0'\n[[categories]]\nname = 'books'\nextensions = ['epub']\ndestination = 'books'\n",
    )
    .expect("write unsupported category catalog");

    let output = project.run(&["sort", "images", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("valid error document");
    assert_eq!(error["error"]["kind"], "configuration");
    assert!(project.0.join("Photo.PNG").is_file());
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
fn quick_sort_rejects_diruleignore_negations() {
    let project = TempProject::new();
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project.0)
        .status()
        .expect("initialize Git fixture");
    assert!(init.success(), "initialize Git fixture");
    fs::write(project.0.join(".gitignore"), "Invoice.PDF\n").expect("write Git ignore rule");
    fs::write(project.0.join(".diruleignore"), "!Invoice.PDF\n")
        .expect("write invalid Dirule ignore negation");
    fs::write(project.0.join("Invoice.PDF"), "invoice").expect("write ignored PDF");

    let preview = project.run(&["sort", "pdf", "--format", "json"]);

    assert!(!preview.status.success());
    let error: serde_json::Value =
        serde_json::from_slice(&preview.stderr).expect("valid scan error");
    assert_eq!(error["error"]["kind"], "scan");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid ignore file")
    );
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains(".diruleignore only excludes paths")
    );
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
