use dirule_core::{AcronymLexicon, Transform, transform, transform_filename};

#[test]
fn handles_case_boundaries_dotfiles_and_extensions() {
    let acronyms = AcronymLexicon::default();

    assert_eq!(
        transform_filename("MyXMLParser.rs", Transform::KebabCase, &acronyms),
        "my-xml-parser.rs"
    );
    assert_eq!(
        transform_filename("HTTPServer.pdf", Transform::SnakeCase, &acronyms),
        "http_server.pdf"
    );
    assert_eq!(
        transform("APIResponse", Transform::KebabCase, &acronyms),
        "api-response"
    );
    assert_eq!(
        transform("XMLHttpRequest", Transform::KebabCase, &acronyms),
        "xml-http-request"
    );
    assert_eq!(
        transform_filename(".MyConfig", Transform::KebabCase, &acronyms),
        ".my-config"
    );
    assert_eq!(
        transform("OAuthToken", Transform::CamelCase, &acronyms),
        "oAuthToken"
    );
}

#[test]
fn handles_digits_punctuation_unicode_and_multiple_extensions() {
    let acronyms = AcronymLexicon::default();

    assert_eq!(
        transform("v2HTTPServer", Transform::SnakeCase, &acronyms),
        "v_2_http_server"
    );
    assert_eq!(
        transform("...My__XMLParser--", Transform::KebabCase, &acronyms),
        "my-xml-parser"
    );
    assert_eq!(
        transform("ÜberHTTPServer", Transform::KebabCase, &acronyms),
        "über-http-server"
    );
    assert_eq!(
        transform("東京/MyFile", Transform::KebabCase, &acronyms),
        "東京-my-file"
    );
    assert_eq!(
        transform_filename("archive.tar.GZ", Transform::KebabCase, &acronyms),
        "archive-tar.GZ"
    );
    assert_eq!(
        transform_filename(".JSONData", Transform::KebabCase, &acronyms),
        ".json-data"
    );
}

#[test]
fn preserves_combining_marks_in_decomposed_unicode_names() {
    let acronyms = AcronymLexicon::default();

    assert_eq!(
        transform("Cafe\u{301}Guide", Transform::KebabCase, &acronyms),
        "cafe\u{301}-guide"
    );
    assert_eq!(
        transform_filename("Cafe\u{301}Guide.pdf", Transform::KebabCase, &acronyms),
        "cafe\u{301}-guide.pdf"
    );
}
