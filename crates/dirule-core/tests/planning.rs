use dirule_core::{
    AcronymOptions, Config, ConflictKind, Entry, EntryKind, FindingKind, Rule, RuleKind, RuleMode,
    parse_config, plan,
};
use std::path::{Path, PathBuf};

#[test]
fn planning_skips_symlinks_before_matching_rules() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid config");
    let entries = [Entry {
        path: PathBuf::from("MyShortcut"),
        kind: EntryKind::Symlink,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert!(planned.entries.is_empty());
    assert!(!planned.has_changes());
    assert_eq!(planned.findings.len(), 1);
    assert_eq!(planned.findings[0].kind, FindingKind::SkippedSymlink);
    assert_eq!(planned.findings[0].path.as_deref(), Some("MyShortcut"));
}

#[test]
fn planning_rejects_paths_outside_the_target() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid config");
    let absolute = std::env::temp_dir().join("outside");
    let entries = [
        Entry {
            path: PathBuf::new(),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("."),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("../outside"),
            kind: EntryKind::File,
        },
        Entry {
            path: absolute,
            kind: EntryKind::File,
        },
    ];

    for entry in entries {
        let error = plan(&config, &[entry]).expect_err("entry path must stay target-relative");

        assert!(error.to_string().contains("entry path"), "{error}");
    }
}

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
fn check_mode_reports_expected_names_without_planning_moves() {
    let config = parse_config(
        r#"
version = "1.4"

[[rules]]
name = "Require kebab case"
match = "*.txt"
mode = "check"
to = "builtin:kebab-case"

[[rules]]
name = "Fallback rename"
match = "**/*.txt"
to = "builtin:snake-case"
"#,
    )
    .expect("valid check-only config");
    let entries = [Entry {
        path: PathBuf::from("QuarterlyReport.txt"),
        kind: EntryKind::File,
    }];

    let planned = plan(&config, &entries).expect("valid plan");

    assert!(!planned.has_changes());
    assert!(planned.has_check_violations());
    assert!(planned.conflicts.is_empty());
    assert_eq!(
        planned.entries[0].source,
        PathBuf::from("QuarterlyReport.txt")
    );
    assert_eq!(
        planned.entries[0].destination,
        PathBuf::from("QuarterlyReport.txt")
    );
    assert_eq!(
        planned.entries[0].rule.as_deref(),
        Some("Require kebab case")
    );
    assert_eq!(planned.findings.len(), 1);
    assert_eq!(planned.findings[0].kind, FindingKind::CheckOnlyViolation);
    assert!(planned.findings[0].message.contains("quarterly-report.txt"));
}

#[test]
fn check_mode_does_not_inherit_a_hypothetical_directory_rename() {
    let config = parse_config(
        r#"
version = "1.4"

[[rules]]
name = "Check directory names"
match = "*"
kind = "directory"
mode = "check"
to = "builtin:kebab-case"

[[rules]]
name = "Check file names"
match = "**/*"
kind = "file"
mode = "check"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid check-only config");
    let entries = [
        Entry {
            path: PathBuf::from("HTTPServer"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("HTTPServer/ReadMe.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert!(!planned.has_changes());
    assert_eq!(planned.findings.len(), 2);
    assert!(planned.findings[0].message.contains("http-server"));
    assert!(
        planned.findings[1]
            .message
            .contains("HTTPServer/read-me.txt")
    );
}

#[test]
fn check_mode_child_tracks_only_an_applied_parent_rename() {
    let config = parse_config(
        r#"
version = "1.4"

[[rules]]
name = "Normalize directories"
match = "*"
kind = "directory"
to = "builtin:kebab-case"

[[rules]]
name = "Check file names"
match = "**/*"
kind = "file"
mode = "check"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid mixed-mode config");
    let entries = [
        Entry {
            path: PathBuf::from("HTTPServer"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("HTTPServer/ReadMe.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert!(planned.has_changes());
    assert!(planned.has_check_violations());
    assert_eq!(planned.entries[0].destination, PathBuf::from("http-server"));
    assert_eq!(
        planned.entries[1].destination,
        PathBuf::from("http-server/ReadMe.txt")
    );
    assert!(
        planned.findings[0]
            .message
            .contains("http-server/read-me.txt")
    );
}

#[test]
fn planning_allows_destinations_vacated_by_other_moves() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Swap first"
match = "first"
to = "template:second"

[[rules]]
name = "Swap second"
match = "second"
to = "template:first"
"#,
    )
    .expect("valid config");
    let entries = [
        Entry {
            path: PathBuf::from("first"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("second"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert!(planned.conflicts.is_empty(), "{planned:#?}");
    assert_eq!(planned.entries[0].destination, PathBuf::from("second"));
    assert_eq!(planned.entries[1].destination, PathBuf::from("first"));
}

#[test]
fn planning_preserves_hidden_directory_prefixes() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize directories"
match = "**/*"
kind = "directory"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid config");
    let entries = [
        Entry {
            path: PathBuf::from(".GitHub"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("nested/.Config"),
            kind: EntryKind::Directory,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(planned.entries[0].destination, PathBuf::from(".git-hub"));
    assert_eq!(
        planned.entries[1].destination,
        PathBuf::from("nested/.config")
    );
}

#[test]
fn planning_hard_excludes_configured_paths_and_directory_descendants() {
    let config = parse_config(
        r#"
version = "1.3"
ignore = ["generated", "docs/api/*.json"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid config");
    let entries = [
        Entry {
            path: PathBuf::from("generated"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("generated/client/Client.ts"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("docs/api/schema.json"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("src/KeepThis.ts"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(planned.entries.len(), 1);
    assert_eq!(planned.entries[0].source, Path::new("src/KeepThis.ts"));
    assert_eq!(
        planned.entries[0].destination,
        Path::new("src/keep-this.ts")
    );
}

#[test]
fn template_parent_uses_the_planned_directory_name() {
    let config = parse_config(
        r#"
version = "1.2"

[[rules]]
name = "Normalize directories"
match = "**/*"
kind = "directory"
to = "builtin:kebab-case"

[[rules]]
name = "Normalize files under their planned parent"
match = "HTTPServer/<stem>.txt"
kind = "file"
to = "template:${parent}/${stem|kebab-case}.${extension}"
"#,
    )
    .expect("valid config");
    let entries = [
        Entry {
            path: PathBuf::from("HTTPServer/ReadMe.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("HTTPServer"),
            kind: EntryKind::Directory,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");
    let file = planned
        .entries
        .iter()
        .find(|entry| entry.source == Path::new("HTTPServer/ReadMe.txt"))
        .expect("planned file entry");

    assert_eq!(file.destination, PathBuf::from("http-server/read-me.txt"));
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
fn excluding_a_directory_path_does_not_exclude_its_children() {
    let config = parse_config(
        r#"
version = "1.2"

[[rules]]
name = "Normalize names"
match = "**/*"
exclude = ["Archive"]
to = "builtin:kebab-case"
"#,
    )
    .expect("valid exclusion rule");
    let entries = [
        Entry {
            path: PathBuf::from("Archive"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("Archive/ReadMe.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(planned.entries[0].destination, PathBuf::from("Archive"));
    assert_eq!(planned.entries[0].rule, None);
    assert_eq!(
        planned.entries[1].destination,
        PathBuf::from("Archive/read-me.txt")
    );
    assert_eq!(planned.entries[1].rule.as_deref(), Some("Normalize names"));
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
fn acronym_lookup_preserves_longest_unicode_prefixes() {
    let config = parse_config(
        r#"
version = "1.0"

[acronyms]
entries = ["ÄB", "ÄBC", "Äé"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid Unicode acronym config");
    let entries = [
        Entry {
            path: PathBuf::from("ÄéGuide.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("ÄBCGuide.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    for (source, destination) in [
        ("ÄBCGuide.txt", "äbc-guide.txt"),
        ("ÄéGuide.txt", "äé-guide.txt"),
    ] {
        let entry = planned
            .entries
            .iter()
            .find(|entry| entry.source == std::path::Path::new(source))
            .expect("planned Unicode filename");
        assert_eq!(entry.destination, PathBuf::from(destination));
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
    assert_eq!(
        plan.findings[0].kind,
        dirule_core::FindingKind::SkippedNonUnicodePath
    );
    assert_eq!(plan.findings[0].path, None);
    assert!(
        plan.findings[0]
            .message
            .contains("cannot be represented as Unicode")
    );
}

#[test]
fn planning_propagates_directory_names_from_unsorted_entries() {
    let config = Config {
        name: None,
        target: None,
        ignore: Vec::new(),
        acronyms: AcronymOptions::default(),
        rules: vec![
            Rule {
                name: "Normalize project folder".into(),
                pattern: "*".into(),
                kind: RuleKind::Any,
                mode: RuleMode::Apply,
                exclude: Vec::new(),
                to: "builtin:kebab-case".into(),
            },
            Rule {
                name: "Normalize remaining paths".into(),
                pattern: "**/*".into(),
                kind: RuleKind::Any,
                mode: RuleMode::Apply,
                exclude: Vec::new(),
                to: "builtin:snake-case".into(),
            },
        ],
    };
    let entries = [
        Entry {
            path: PathBuf::from("MyXMLParser/ReadMe.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("MyXMLParser"),
            kind: EntryKind::Directory,
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
fn conflicts_are_reported_in_destination_order() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid configuration");
    let entries = [
        Entry {
            path: PathBuf::from("C__D.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("A--B.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("A B.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("C--D.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("A__B.txt"),
            kind: EntryKind::File,
        },
    ];

    let planned = plan(&config, &entries).expect("valid plan");

    assert_eq!(
        planned
            .conflicts
            .iter()
            .map(|conflict| conflict.path.clone())
            .collect::<Vec<_>>(),
        [PathBuf::from("a-b.txt"), PathBuf::from("c-d.txt")]
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
        planned.conflicts[0].kind,
        ConflictKind::FileDestinationParent
    );
    assert_eq!(
        planned.conflicts[0].related_paths,
        [
            PathBuf::from("archive"),
            PathBuf::from("archive/details.txt"),
            PathBuf::from("details.txt"),
            PathBuf::from("report.txt"),
        ]
    );
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

#[test]
fn planning_handles_generated_rename_cycles_without_conflicts() {
    for cycle_len in 2..=32 {
        let mut source = String::from("version = '1.0'\n");
        let mut entries = Vec::with_capacity(cycle_len);

        for index in 0..cycle_len {
            let next = (index + 1) % cycle_len;
            source.push_str(&format!(
                "\n[[rules]]\nname = 'Move {index}'\nmatch = 'item-{index}.txt'\nto = 'template:item-{next}.txt'\n"
            ));
            entries.push(Entry {
                path: PathBuf::from(format!("item-{index}.txt")),
                kind: EntryKind::File,
            });
        }

        let config = parse_config(&source).expect("valid generated configuration");
        let planned = plan(&config, &entries).expect("valid generated plan");

        assert!(
            planned.conflicts.is_empty(),
            "cycle length {cycle_len}: {planned:#?}"
        );
        assert!(planned.has_changes(), "cycle length {cycle_len}");
        assert_eq!(planned.entries.len(), cycle_len);

        for entry in &planned.entries {
            let index = entry
                .source
                .file_stem()
                .and_then(|stem| stem.to_str())
                .and_then(|stem| stem.strip_prefix("item-"))
                .and_then(|index| index.parse::<usize>().ok())
                .expect("generated source index");
            let next = (index + 1) % cycle_len;

            assert_eq!(entry.destination, PathBuf::from(format!("item-{next}.txt")));
        }
    }
}

#[test]
fn normalized_plans_are_idempotent_for_generated_nested_paths() {
    let config = parse_config(
        r#"
version = "1.0"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid configuration");
    let entries = [
        Entry {
            path: PathBuf::from("HTTPServer"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("HTTPServer/MyXMLParser.pdf"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("APIClient/JSONSchema/ReadMe.txt"),
            kind: EntryKind::File,
        },
        Entry {
            path: PathBuf::from("APIClient"),
            kind: EntryKind::Directory,
        },
        Entry {
            path: PathBuf::from("APIClient/JSONSchema"),
            kind: EntryKind::Directory,
        },
    ];

    let first_plan = plan(&config, &entries).expect("valid first plan");
    let normalized_entries = first_plan
        .entries
        .iter()
        .map(|entry| Entry {
            path: entry.destination.clone(),
            kind: entry.kind,
        })
        .collect::<Vec<_>>();
    let second_plan = plan(&config, &normalized_entries).expect("valid second plan");

    assert!(first_plan.conflicts.is_empty(), "{first_plan:#?}");
    assert!(first_plan.has_changes());
    assert!(second_plan.conflicts.is_empty(), "{second_plan:#?}");
    assert!(!second_plan.has_changes(), "{second_plan:#?}");
    assert!(
        second_plan
            .entries
            .iter()
            .all(|entry| entry.source == entry.destination)
    );
}
