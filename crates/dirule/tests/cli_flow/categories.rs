use super::common::*;

#[test]
fn category_listing_reports_effective_user_and_builtin_categories() {
    let project = TempProject::new();
    let catalog = preset_config_base(&project).join("dirule/categories.toml");
    fs::create_dir_all(catalog.parent().expect("catalog parent")).expect("create config dir");
    fs::write(
        &catalog,
        format!(
            "version = \"{}\"\n\
             [[categories]]\nname = 'images'\nextensions = ['raw']\ndestination = 'photos'\n\
             [[categories]]\nname = 'ebooks'\nextensions = ['epub', 'mobi']\ndestination = 'reading/books'\n",
            dirule_app::CURRENT_CATEGORY_CATALOG_VERSION
        ),
    )
    .expect("write category catalog");

    let output = project.run(&["categories", "list", "--format", "json"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    assert_eq!(value["schema_version"], "1.4");
    assert_eq!(value["categories"][0]["name"], "audio");
    assert_eq!(value["categories"][1]["name"], "documents");
    assert_eq!(value["categories"][2]["name"], "ebooks");
    assert_eq!(
        value["categories"][2]["extensions"],
        serde_json::json!(["epub", "mobi"])
    );
    assert_eq!(value["categories"][2]["destination"], "reading/books");
    assert_eq!(
        value["categories"][3]["extensions"],
        serde_json::json!(["raw"])
    );
    assert_eq!(value["categories"][3]["destination"], "photos");
    assert_eq!(value["categories"][4]["name"], "videos");

    let human = project.run(&["categories", "list", "--color", "never"]);
    assert!(human.status.success());
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("Categories"));
    assert!(
        human.lines().any(
            |line| line.trim_start().starts_with("ebooks") && line.contains("-> reading/books")
        )
    );
    assert!(human.contains("Extensions: epub, mobi"));
}

#[test]
fn category_listing_reports_invalid_user_catalog_as_json_error() {
    let project = TempProject::new();
    let catalog = preset_config_base(&project).join("dirule/categories.toml");
    fs::create_dir_all(catalog.parent().expect("catalog parent")).expect("create config dir");
    fs::write(
        &catalog,
        format!(
            "version = \"{}\"\n\
             [[categories]]\nname = 'ebooks'\nextensions = ['..epub']\ndestination = 'books'\n",
            dirule_app::CURRENT_CATEGORY_CATALOG_VERSION
        ),
    )
    .expect("write invalid category catalog");

    let output = project.run(&["categories", "list", "--format", "json"]);

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).expect("JSON error");
    assert_eq!(error["error"]["kind"], "configuration");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("categories.toml")
    );
}
