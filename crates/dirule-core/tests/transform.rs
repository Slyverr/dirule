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
