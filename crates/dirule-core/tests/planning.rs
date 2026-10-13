use dirule_core::{AcronymOptions, Config, Entry, EntryKind, Rule, parse_config, plan};
use std::path::PathBuf;

#[test]
fn project_acronyms_drive_builtin_and_template_transformations() {
    let config = parse_config(
        r#"
version = "1.0"

[acronyms]
entries = ["iOS"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid acronym configuration");
    let entries = [Entry {
        path: PathBuf::from("iOSGuide.txt"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("ios-guide.txt")
    );

    let mut templated = config.clone();
    templated.rules[0].to = "template:${stem|kebab-case}.${extension}".into();
    let planned = plan(&templated, &entries).expect("valid template plan");
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("ios-guide.txt")
    );

    let mut without_entries = config;
    without_entries.acronyms.entries.clear();
    let planned = plan(&without_entries, &entries).expect("valid plan without acronym entries");
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("i-os-guide.txt")
    );
}

#[test]
fn longer_project_acronyms_take_precedence_at_token_boundaries() {
    let config = parse_config(
        r#"
version = "1.0"

[acronyms]
entries = ["API", "APIClient"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid acronym config");
    let entries = [Entry {
        path: PathBuf::from("APIClientGuide.txt"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("apiclient-guide.txt")
    );
}

#[test]
fn project_acronyms_do_not_match_inside_a_larger_token() {
    let config = parse_config(
        r#"
version = "1.0"

[acronyms]
entries = ["API"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid acronym config");
    let entries = [Entry {
        path: PathBuf::from("XAPIClient.txt"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("xapi-client.txt")
    );
}

#[cfg(unix)]
#[test]
fn planning_skips_and_reports_non_unicode_paths() {
    use std::os::unix::ffi::OsStringExt;

    let config =
        parse_config("version='1.0'\n[[rules]]\nname='x'\nmatch='**/*'\nto='builtin:kebab-case'")
            .expect("valid config");
    let entries = [Entry {
        path: PathBuf::from(std::ffi::OsString::from_vec(vec![b'f', 0xff])),
        kind: EntryKind::File,
    }];

    let plan = plan(&config, &entries).expect("unrepresentable paths are skipped");

    assert!(plan.entries.is_empty());
    assert_eq!(plan.findings.len(), 1);
    assert!(plan.findings[0].contains("cannot be represented as Unicode"));
}

#[test]
fn planning_propagates_directory_names_and_keeps_first_match_on_original_paths() {
    let config = Config {
        name: None,
        target: None,
        acronyms: AcronymOptions::default(),
        rules: vec![
            Rule {
                name: "Normalize project folder".into(),
                pattern: "*".into(),
                to: "builtin:kebab-case".into(),
            },
            Rule {
                name: "Normalize remaining paths".into(),
                pattern: "**/*".into(),
                to: "builtin:snake-case".into(),
            },
        ],
    };
    let entries = [
        Entry {
            path: PathBuf::from("MyXMLParser"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("MyXMLParser/ReadMe.txt"),
            kind: EntryKind::File,
        },
    ];
    let result = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        result.entries[0].destination,
        PathBuf::from("my-xml-parser")
    );
    assert_eq!(
        result.entries[1].destination,
        PathBuf::from("my-xml-parser/read_me.txt")
    );
    assert_eq!(
        result.entries[0].rule.as_deref(),
        Some("Normalize project folder")
    );
    assert_eq!(
        result.entries[1].rule.as_deref(),
        Some("Normalize remaining paths")
    );
}

#[cfg(unix)]
#[test]
fn path_separator_matching_does_not_treat_a_backslash_as_a_directory() {
    let config = parse_config(
        "version='1.0'\n[[rules]]\nname='nested path'\nmatch='nested/name'\nto='builtin:kebab-case'",
    )
    .expect("valid config");
    let entries = [Entry {
        path: PathBuf::from("nested\\name"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(planned.entries[0].source, PathBuf::from("nested\\name"));
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("nested\\name")
    );
    assert_eq!(planned.entries[0].rule, None);
}

#[cfg(unix)]
#[test]
fn templates_reject_backslash_paths_from_unix_filenames() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Preserve source path"
match = "nested*"
to = "template:${path}"
"#,
    )
    .expect("valid config");
    let entries = [Entry {
        path: PathBuf::from("nested\\name"),
        kind: EntryKind::File,
    }];

    assert!(plan(&config, &entries).is_err());
}
