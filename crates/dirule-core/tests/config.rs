use dirule_core::{RuleKind, parse_config};

#[test]
fn parses_and_validates_rule_configuration() {
    let config = parse_config(
        r#"
version = "1.2"
target = "."

[[rules]]
name = "Archive invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
to = "template:Archive/${year}/${vendor|kebab-case}/${title}.pdf"
"#,
    )
    .expect("valid configuration");

    assert_eq!(config.rules.len(), 1);
    assert_eq!(config.rules[0].kind, RuleKind::Any);
    assert!(config.acronyms.entries.is_empty());
    assert_eq!(
        config.rules[0].pattern,
        "Invoices/<year>-<vendor>-<title>.pdf"
    );
}

#[test]
fn parses_and_validates_hard_ignore_patterns() {
    let config = parse_config(
        r#"
version = "1.3"
ignore = ["generated/**", "vendor"]

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("valid target-wide exclusions");

    assert_eq!(config.ignore, ["generated/**", "vendor"]);

    for pattern in ["", "["] {
        let source = format!(
            "version = '1.3'\nignore = ['{pattern}']\n[[rules]]\nname = 'Normalize names'\nmatch = '**/*'\nto = 'builtin:kebab-case'"
        );
        let error = parse_config(&source).expect_err("invalid ignore pattern is rejected");

        assert!(error.to_string().contains("ignore"));
    }
}

#[test]
fn version_1_2_migrates_with_no_hard_ignore_patterns() {
    let config = parse_config(
        r#"
version = "1.2"

[[rules]]
name = "Normalize names"
match = "**/*"
to = "builtin:kebab-case"
"#,
    )
    .expect("legacy configuration migrates");

    assert!(config.ignore.is_empty());
}

#[test]
fn version_1_0_migrates_to_the_default_rule_kind() {
    let config = parse_config(
        r#"
version = "1.0"
name = "Legacy downloads"
target = "~/Downloads"

[acronyms]
entries = ["iOS"]

[[rules]]
name = "Normalize names"
match = "**/*.pdf"
to = "builtin:kebab-case"
"#,
    )
    .expect("legacy configuration migrates");

    assert_eq!(config.name.as_deref(), Some("Legacy downloads"));
    assert_eq!(
        config.target.as_deref(),
        Some(std::path::Path::new("~/Downloads"))
    );
    assert_eq!(config.acronyms.entries, ["iOS"]);
    assert_eq!(config.rules.len(), 1);
    assert_eq!(config.rules[0].name, "Normalize names");
    assert_eq!(config.rules[0].pattern, "**/*.pdf");
    assert_eq!(config.rules[0].to, "builtin:kebab-case");
    assert_eq!(config.rules[0].kind, RuleKind::Any);
    assert!(config.rules[0].exclude.is_empty());
}

#[test]
fn version_1_1_migrates_to_empty_rule_exclusions() {
    let config = parse_config(
        r#"
version = "1.1"

[[rules]]
name = "Normalize names"
match = "**/*"
kind = "file"
to = "builtin:kebab-case"
"#,
    )
    .expect("configuration migrates");

    assert_eq!(config.rules[0].kind, RuleKind::File);
    assert!(config.rules[0].exclude.is_empty());
}

#[test]
fn parses_and_validates_rule_exclusion_patterns() {
    let config = parse_config(
        r#"
version = "1.2"

[[rules]]
name = "Normalize files"
match = "**/*"
kind = "file"
exclude = ["**/*.md", "build/**"]
to = "builtin:kebab-case"
"#,
    )
    .expect("valid exclusion patterns");

    assert_eq!(config.rules[0].exclude, ["**/*.md", "build/**"]);
}

#[test]
fn rejects_empty_and_malformed_rule_exclusion_patterns() {
    for pattern in ["", "["] {
        let source = format!(
            "version = '1.2'\n[[rules]]\nname = 'Normalize files'\nmatch = '**/*'\nexclude = ['{pattern}']\nto = 'builtin:kebab-case'"
        );

        let error = parse_config(&source).expect_err("invalid exclusion is rejected");
        assert!(error.to_string().contains("Normalize files"));
        assert!(error.to_string().contains("exclusion"));
    }
}

#[test]
fn rejects_unknown_rule_kinds() {
    let error = parse_config(
        r#"
version = "1.1"

[[rules]]
name = "Normalize files"
match = "**/*"
kind = "symlink"
to = "builtin:kebab-case"
"#,
    )
    .expect_err("symlinks are not an eligible rule kind");

    assert!(error.to_string().contains("unknown variant `symlink`"));
}

#[test]
fn rejects_unsupported_versions_and_unknown_template_values() {
    let error =
        parse_config("version = \"2.0\"\n[[rules]]\nname='x'\nmatch='*'\nto='builtin:kebab-case'")
            .expect_err("version 2 is unsupported");
    assert!(
        error
            .to_string()
            .contains("newer than the supported version")
    );

    let error =
        parse_config("version = \"1.0\"\n[[rules]]\nname='x'\nmatch='*'\nto='template:${missing}'")
            .expect_err("missing template values are invalid");
    assert!(error.to_string().contains("missing capture"));
}

#[test]
fn rejects_static_unsafe_template_destinations_at_config_load() {
    for destination in ["../outside", "/absolute", "C:/absolute"] {
        let source = format!(
            "version='1.0'\n[[rules]]\nname='x'\nmatch='never'\nto='template:{destination}'"
        );
        assert!(parse_config(&source).is_err(), "accepted {destination:?}");
    }
}

#[test]
fn config_version_must_be_a_major_minor_string() {
    let error =
        parse_config("version = 1\n[[rules]]\nname='x'\nmatch='*'\nto='builtin:kebab-case'")
            .expect_err("integer versions are not accepted");

    assert!(error.to_string().contains("version` must be a string"));
}

#[test]
fn config_version_requires_canonical_components() {
    for version in ["01.0", "1.00", "1.+0", "1.0.0"] {
        let source = format!(
            "version = '{version}'\n[[rules]]\nname='x'\nmatch='*'\nto='builtin:kebab-case'"
        );
        assert!(parse_config(&source).is_err(), "accepted {version:?}");
    }
}

#[test]
fn rejects_empty_or_duplicate_custom_acronyms() {
    for entries in ["entries = ['']", "entries = ['iOS', 'iOS']"] {
        let source = format!(
            "version = '1.0'\n[acronyms]\n{entries}\n[[rules]]\nname='x'\nmatch='*'\nto='builtin:kebab-case'"
        );
        assert!(parse_config(&source).is_err(), "accepted {entries:?}");
    }
}
