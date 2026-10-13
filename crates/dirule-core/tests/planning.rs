use dirule_core::{AcronymOptions, Config, Entry, EntryKind, Rule, RuleKind, parse_config, plan};
use std::path::PathBuf;

#[test]
fn built_in_transform_rules_can_use_named_match_captures() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize invoice names"
match = "Invoices/<name>.pdf"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid config");
    let entries = [Entry {
        path: PathBuf::from("Invoices/HTTPServer.pdf"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("Invoices/http-server.pdf")
    );
}

#[test]
fn rule_kind_filters_entries_and_mismatches_continue_to_later_rules() {
    let config = parse_config(
        r#"
version = "1.2"

[[rules]]
name = "Normalize directories"
match = "**/*"
kind = "directory"
to = "template:folders/${name}"

[[rules]]
name = "Normalize files"
match = "**/*"
kind = "file"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid typed rules");
    let entries = [
        Entry {
            path: PathBuf::from("HTTPServer"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("MyXMLParser.pdf"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("folders/HTTPServer")
    );
    assert_eq!(
        planned.entries[0].rule.as_deref(),
        Some("Normalize directories")
    );
    assert_eq!(
        planned.entries[1].destination,
        PathBuf::from("my-xml-parser.pdf")
    );
    assert_eq!(planned.entries[1].rule.as_deref(), Some("Normalize files"));
}

#[test]
fn rule_exclusions_fall_through_without_claiming_entries() {
    let config = parse_config(
        r#"
version = "1.2"

[[rules]]
name = "Normalize files"
match = "**/*"
kind = "file"
exclude = ["**/*.md"]
to = "builtin:kebab-case"

[[rules]]
name = "Move the README"
match = "README.md"
kind = "file"
to = "template:docs/${name}"
"#,
    )
    .expect("valid exclusion rules");
    let entries = [
        Entry {
            path: PathBuf::from("MyXMLParser.pdf"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("README.md"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("guide.md"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("my-xml-parser.pdf")
    );
    assert_eq!(planned.entries[0].rule.as_deref(), Some("Normalize files"));
    assert_eq!(
        planned.entries[1].destination,
        PathBuf::from("docs/README.md")
    );
    assert_eq!(planned.entries[1].rule.as_deref(), Some("Move the README"));
    assert_eq!(planned.entries[2].destination, PathBuf::from("guide.md"));
    assert_eq!(planned.entries[2].rule, None);
}

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

    let mut pascal_case = config.clone();
    pascal_case.rules[0].to = "builtin:pascal-case".into();
    let planned = plan(&pascal_case, &entries).expect("valid PascalCase plan");
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("IosGuide.txt")
    );

    pascal_case.rules[0].to = "template:${stem|pascal-case}.${extension}".into();
    let planned = plan(&pascal_case, &entries).expect("valid PascalCase template plan");
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("IosGuide.txt")
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
fn built_in_transforms_match_equivalent_template_filters() {
    let config = parse_config(
        r#"
version = "1.0"

[acronyms]
entries = ["XML", "iOS"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid configuration");
    let entries = [
        Entry {
            path: PathBuf::from("MyXMLParser.tar.gz"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("iOSGuide.txt"),
            kind: EntryKind::File,
        },
    ];

    for (builtin, filter) in [
        ("snake-case", "snake-case"),
        ("kebab-case", "kebab-case"),
        ("camel-case", "camel-case"),
        ("pascal-case", "pascal-case"),
    ] {
        let mut builtin_config = config.clone();
        builtin_config.rules[0].to = format!("builtin:{builtin}");
        let builtin_plan = plan(&builtin_config, &entries).expect("built-in plan");

        let mut template_config = config.clone();
        template_config.rules[0].to = format!("template:${{stem|{filter}}}.${{extension}}");
        let template_plan = plan(&template_config, &entries).expect("template plan");

        let builtin_destinations = builtin_plan
            .entries
            .iter()
            .map(|entry| &entry.destination)
            .collect::<Vec<_>>();
        let template_destinations = template_plan
            .entries
            .iter()
            .map(|entry| &entry.destination)
            .collect::<Vec<_>>();
        assert_eq!(builtin_destinations, template_destinations, "{builtin}");
    }
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
fn acronym_matching_tries_shorter_prefixes_when_longer_entries_do_not_match() {
    let config = parse_config(
        r#"
version = "1.0"

[acronyms]
entries = ["API", "APIs"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid acronym config");
    let entries = [Entry {
        path: PathBuf::from("APIResponse.txt"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("api-response.txt")
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
                kind: RuleKind::Any,
                exclude: Vec::new(),
                to: "builtin:kebab-case".into(),
            },
            Rule {
                name: "Normalize remaining paths".into(),
                pattern: "**/*".into(),
                kind: RuleKind::Any,
                exclude: Vec::new(),
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

#[test]
fn first_matching_rule_owns_the_original_path() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Archive report PDFs"
match = "Reports/*.pdf"
to = "template:archive/${name}"

[[rules]]
name = "Rewrite archived paths"
match = "archive/**"
to = "template:changed/${path}"

[[rules]]
name = "Normalize remaining PDFs"
match = "**/*.pdf"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid configuration");
    let entries = [Entry {
        path: PathBuf::from("Reports/MyReport.pdf"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned.entries[0].rule.as_deref(),
        Some("Archive report PDFs")
    );
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("archive/MyReport.pdf")
    );
}

#[test]
fn collisions_are_found_across_distinct_rule_owners() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Archive A reports"
match = "A/<part>.txt"
to = "template:archive/${part}.txt"

[[rules]]
name = "Archive B reports"
match = "B/<part>.txt"
to = "template:archive/${part}.txt"
"#,
    )
    .expect("valid configuration");
    let entries = [
        Entry {
            path: PathBuf::from("A/report.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("B/report.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan with a conflict");

    assert_eq!(
        planned
            .entries
            .iter()
            .map(|entry| entry.rule.as_deref())
            .collect::<Vec<_>>(),
        [Some("Archive A reports"), Some("Archive B reports")]
    );
    assert_eq!(planned.conflicts.len(), 1, "{planned:#?}");
    assert_eq!(
        planned.conflicts[0].path,
        PathBuf::from("archive/report.txt")
    );
}

#[test]
fn conflicts_detect_files_that_would_become_parent_paths() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Move report"
match = "report.txt"
to = "template:archive"

[[rules]]
name = "Move details"
match = "details.txt"
to = "template:archive/details.txt"
"#,
    )
    .expect("valid configuration");
    let entries = [
        Entry {
            path: PathBuf::from("report.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("details.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(planned.conflicts.len(), 1, "{planned:#?}");
    assert_eq!(planned.conflicts[0].path, PathBuf::from("archive"));
    assert_eq!(
        planned.conflicts[0].reason,
        "a file destination is also a parent of another destination"
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
