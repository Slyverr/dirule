use dirule_core::{Matcher, MatcherSet, Rule, RuleKind, RuleMode, parse_config};

#[test]
fn extracts_named_capture_values() {
    let config = parse_config(
        "version='1.0'\n[[rules]]\nname='invoice'\nmatch='Invoices/<year>-<vendor>-<title>.pdf'\nto='template:${year}/${vendor}/${title}.pdf'",
    )
    .expect("valid config");
    let matcher = Matcher::new(&config.rules[0].pattern).expect("compiled matcher");
    let captures = matcher
        .captures("Invoices/2026-Acme-Q4-report.pdf")
        .expect("path matches");

    assert_eq!(captures.get("year"), Some("2026"));
    assert_eq!(captures.get("vendor"), Some("Acme"));
    assert_eq!(captures.get("title"), Some("Q4-report"));
    assert!(matcher.captures("Invoices/2026-Acme/report.pdf").is_none());
}

#[test]
fn matching_without_capture_values_preserves_named_capture_rules() {
    for (pattern, path) in [
        ("**/*.pdf", "invoice.pdf"),
        ("Invoices/<year>-<vendor>.pdf", "Invoices/2026-Acme.pdf"),
        ("Invoices/<year>-<vendor>.pdf", "Invoices/-Acme.pdf"),
        ("Invoices/<year>-<vendor>.pdf", "Invoices/2026-.pdf"),
        ("Invoices/<year>-<vendor>.pdf", "Invoices/2026-Acme.txt"),
    ] {
        let matcher = Matcher::new(pattern).expect("valid matcher");

        assert_eq!(
            matcher.is_match(path),
            matcher.captures(path).is_some(),
            "match behavior differs for {pattern:?} and {path:?}"
        );
    }
}

#[test]
fn captures_after_globs_keep_their_named_values() {
    let matcher = Matcher::new("*/<id>.txt").expect("valid matcher");
    let captures = matcher.captures("archive/42.txt").expect("path matches");

    assert_eq!(captures.get("id"), Some("42"));
}

#[test]
fn tree_globs_match_root_and_nested_paths_case_sensitively() {
    let rule = Rule {
        name: "PDF files".into(),
        pattern: "**/*.pdf".into(),
        kind: RuleKind::Any,
        mode: RuleMode::Apply,
        exclude: Vec::new(),
        to: "builtin:kebab-case".into(),
    };
    let matcher = Matcher::new(&rule.pattern).expect("compiled matcher");

    assert!(matcher.captures("invoice.pdf").is_some());
    assert!(matcher.captures("archive/invoice.pdf").is_some());
    assert!(matcher.captures("archive/INVOICE.PDF").is_none());
}

#[test]
fn path_exclusion_matching_includes_matching_ancestor_directories() {
    let matcher = Matcher::new("generated").expect("valid exclusion pattern");

    assert!(matcher.matches_path_or_parent("generated"));
    assert!(matcher.matches_path_or_parent("generated/api/client.ts"));
    assert!(!matcher.matches_path_or_parent("src/generated/client.ts"));
    assert!(!matcher.matches_path_or_parent("generated-files/client.ts"));
}

#[test]
fn matcher_sets_preserve_multiple_pattern_and_capture_behavior() {
    let matchers = MatcherSet::new(
        ["generated", "**/vendor/**", "Invoices/<year>.pdf"]
            .into_iter()
            .map(|pattern| Matcher::new(pattern).expect("valid matcher")),
    );

    for path in [
        "generated",
        "generated/api/client.ts",
        "src/vendor/package/file.rs",
        "Invoices/2026.pdf",
    ] {
        assert!(matchers.matches_path_or_parent(path), "{path}");
    }

    for path in [
        "generated-files/client.ts",
        "src/generated/client.ts",
        "src/file.rs",
    ] {
        assert!(!matchers.matches_path_or_parent(path), "{path}");
    }
}

#[test]
fn matcher_sets_match_the_same_paths_as_individual_globs() {
    let patterns = [
        "**/*.(?i)pdf",
        "archive/[!a-c]?.{pdf,png}",
        r"literal/\*",
        "ReadMe",
    ];
    let matchers = patterns
        .iter()
        .map(|pattern| Matcher::new(pattern).expect("valid matcher"))
        .collect::<Vec<_>>();
    let set = MatcherSet::new(matchers.clone());

    for path in [
        "invoice.PDF",
        "src/invoice.PdF",
        "archive/z7.pdf",
        "archive/z7.png",
        "archive/b7.txt",
        "literal/*",
        "literal/anything",
        "ReadMe",
        "readme",
    ] {
        assert_eq!(
            set.is_match(path),
            matchers.iter().any(|matcher| matcher.is_match(path)),
            "{path}"
        );
    }
}

#[test]
fn matching_flags_can_select_case_insensitive_components() {
    let matcher = Matcher::new("**/*.(?i)pdf").expect("valid glob pattern");

    assert!(matcher.is_match("invoice.PDF"));
    assert!(matcher.is_match("archive/Invoice.Pdf"));
    assert!(!matcher.is_match("archive/PDF.invoice"));
}

#[test]
fn supports_character_classes_question_marks_and_alternatives() {
    let matcher = Matcher::new("archive/[!a-c]?.{pdf,png}").expect("valid glob pattern");

    assert!(matcher.captures("archive/z7.pdf").is_some());
    assert!(matcher.captures("archive/z7.png").is_some());
    assert!(matcher.captures("archive/b7.pdf").is_none());
    assert!(matcher.captures("archive/z7.txt").is_none());
}

#[test]
fn rejects_ambiguous_capture_boundaries() {
    for pattern in [
        "<first><last>.txt",
        "*<name>.txt",
        "<name>?.txt",
        r"\\*<name>.txt",
    ] {
        assert!(Matcher::new(pattern).is_err(), "accepted {pattern:?}");
    }
}

#[test]
fn escaped_wildcards_are_literal_before_named_captures() {
    for (pattern, path) in [
        (r"\*<name>.txt", "*report.txt"),
        (r"\?<name>.txt", "?report.txt"),
    ] {
        let matcher = Matcher::new(pattern).expect("escaped wildcard has a clear boundary");
        let captures = matcher
            .captures(path)
            .expect("escaped wildcard and capture match");

        assert_eq!(captures.get("name"), Some("report"));
    }
}

#[test]
fn rejects_capture_names_inside_alternations() {
    let config = parse_config(
        "version='1.0'\n[[rules]]\nname='x'\nmatch='{<name>,other}'\nto='template:${name}'",
    );
    assert!(config.is_err());
}
