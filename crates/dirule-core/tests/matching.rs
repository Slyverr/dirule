use dirule_core::{Matcher, Rule, parse_config};

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
        to: "builtin:kebab-case".into(),
    };
    let matcher = Matcher::new(&rule.pattern).expect("compiled matcher");

    assert!(matcher.captures("invoice.pdf").is_some());
    assert!(matcher.captures("archive/invoice.pdf").is_some());
    assert!(matcher.captures("archive/INVOICE.PDF").is_none());
}

#[test]
fn rejects_capture_names_inside_alternations() {
    let config = parse_config(
        "version='1.0'\n[[rules]]\nname='x'\nmatch='{<name>,other}'\nto='template:${name}'",
    );
    assert!(config.is_err());
}
