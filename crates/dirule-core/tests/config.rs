use dirule_core::parse_config;

#[test]
fn parses_and_validates_rule_configuration() {
    let config = parse_config(
        r#"
version = "1.0"
target = "."

[[rules]]
name = "Archive invoices"
match = "Invoices/<year>-<vendor>-<title>.pdf"
to = "template:Archive/${year}/${vendor|kebab-case}/${title}.pdf"
"#,
    )
    .expect("valid configuration");

    assert_eq!(config.rules.len(), 1);
    assert!(config.acronyms.entries.is_empty());
    assert_eq!(
        config.rules[0].pattern,
        "Invoices/<year>-<vendor>-<title>.pdf"
    );
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
