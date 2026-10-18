use dirule_assistant::{
    Message, ModelProvider, ModelRequest, OpenAiCompatibleConfig, OpenAiCompatibleProvider,
    OpenAiResponsesConfig, OpenAiResponsesProvider,
};
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const CHILD_CASE: &str = "DIRULE_CREDENTIALS_CHILD_CASE";
const CHILD_ENDPOINT: &str = "DIRULE_CREDENTIALS_CHILD_ENDPOINT";
const CHILD_EXPECTED_ERROR: &str = "DIRULE_CREDENTIALS_CHILD_EXPECTED_ERROR";
const API_KEY_ENV: &str = "DIRULE_CREDENTIALS_TEST_KEY";
const MAX_CREDENTIALS_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug)]
enum Adapter {
    ChatCompletions,
    Responses,
}

impl Adapter {
    fn case(self) -> &'static str {
        match self {
            Self::ChatCompletions => "chat",
            Self::Responses => "responses",
        }
    }

    fn endpoint(self, address: std::net::SocketAddr) -> String {
        let path = match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
        };

        format!("http://{address}{path}")
    }

    fn response(self) -> serde_json::Value {
        match self {
            Self::ChatCompletions => json!({
                "choices": [{"message": {"content": "ok"}}]
            }),
            Self::Responses => json!({
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "ok"}]
                }]
            }),
        }
    }
}

const ADAPTERS: [Adapter; 2] = [Adapter::ChatCompletions, Adapter::Responses];

#[test]
fn credentials_resolution_child() {
    let Ok(case) = std::env::var(CHILD_CASE) else {
        return;
    };

    let endpoint = std::env::var(CHILD_ENDPOINT).expect("mock endpoint");
    let adapter = if case.ends_with("responses") {
        Adapter::Responses
    } else {
        Adapter::ChatCompletions
    };
    let provider = match adapter {
        Adapter::ChatCompletions => OpenAiCompatibleProvider::from_config(
            OpenAiCompatibleConfig::new(endpoint, "test-model", API_KEY_ENV),
        )
        .map(|provider| Box::new(provider) as Box<dyn ModelProvider>),
        Adapter::Responses => OpenAiResponsesProvider::from_config(OpenAiResponsesConfig::new(
            endpoint,
            "test-model",
            API_KEY_ENV,
        ))
        .map(|provider| Box::new(provider) as Box<dyn ModelProvider>),
    };

    let expected_error = std::env::var(CHILD_EXPECTED_ERROR)
        .ok()
        .or_else(|| (case == "insecure-file").then(|| "must be private to its owner".into()));
    let provider = match (expected_error, provider) {
        (Some(expected), Err(error)) => {
            assert!(error.to_string().contains(&expected));

            return;
        }
        (Some(expected), Ok(_)) => panic!("provider should reject credentials: {expected}"),
        (None, Ok(provider)) => provider,
        (None, Err(error)) => panic!("provider loads credentials: {error}"),
    };
    let response = provider
        .complete_with_timeout(
            &ModelRequest {
                model: String::new(),
                messages: vec![Message::User {
                    content: "hello".into(),
                }],
                tools: Vec::new(),
            },
            Duration::from_secs(2),
        )
        .expect("mock provider response");

    assert_eq!(response.content.as_deref(), Some("ok"));
}

#[test]
fn environment_credentials_override_the_file_for_both_adapters() {
    for adapter in ADAPTERS {
        let (endpoint, server) = start_mock_provider(adapter);
        let home = tempfile::tempdir().expect("temporary config home");
        write_credentials(
            &credentials_path(home.path()),
            "DIRULE_CREDENTIALS_TEST_KEY=file-secret\n",
        );

        let output = run_child(
            &format!("environment-{}", adapter.case()),
            &endpoint,
            home.path(),
            Some("  environment-secret  "),
            None,
        );

        assert_success(&output);
        let request = server.join().expect("mock server thread");
        assert_authorization(&request, "Bearer environment-secret");
    }
}

#[test]
fn credentials_file_is_used_when_the_environment_value_is_blank() {
    for adapter in ADAPTERS {
        let (endpoint, server) = start_mock_provider(adapter);
        let home = tempfile::tempdir().expect("temporary config home");
        write_credentials(
            &credentials_path(home.path()),
            "# local test credential\nDIRULE_CREDENTIALS_TEST_KEY = \"file-secret\"\n",
        );

        let output = run_child(
            &format!("file-{}", adapter.case()),
            &endpoint,
            home.path(),
            Some("  "),
            None,
        );

        assert_success(&output);
        let request = server.join().expect("mock server thread");
        assert_authorization(&request, "Bearer file-secret");
    }
}

#[cfg(unix)]
#[test]
fn credentials_file_with_group_or_world_permissions_is_rejected() {
    let home = tempfile::tempdir().expect("temporary config home");
    let credentials = credentials_path(home.path());
    write_credentials(&credentials, "DIRULE_CREDENTIALS_TEST_KEY=file-secret\n");
    std::fs::set_permissions(
        &credentials,
        std::os::unix::fs::PermissionsExt::from_mode(0o644),
    )
    .expect("make credentials file readable by others");

    let output = run_child(
        "insecure-file",
        "http://127.0.0.1:1/v1/chat/completions",
        home.path(),
        None,
        None,
    );

    assert_success(&output);
}

#[test]
fn malformed_duplicate_invalid_utf8_and_oversized_files_are_rejected() {
    let malformed = b"this is not a NAME=value line\n";
    let duplicate = b"DIRULE_CREDENTIALS_TEST_KEY=first\nDIRULE_CREDENTIALS_TEST_KEY=second\n";
    let invalid_utf8 = b"DIRULE_CREDENTIALS_TEST_KEY=\xff\n";
    let oversized = vec![b'x'; MAX_CREDENTIALS_FILE_BYTES + 1];

    for (contents, expected_error) in [
        (malformed.as_slice(), "invalid Dirule credentials file"),
        (duplicate.as_slice(), "invalid Dirule credentials file"),
        (invalid_utf8.as_slice(), "not valid UTF-8"),
        (oversized.as_slice(), "exceeds the 65536-byte limit"),
    ] {
        let home = tempfile::tempdir().expect("temporary config home");
        write_credentials_bytes(&credentials_path(home.path()), contents);

        let output = run_child(
            "reject-chat",
            "http://127.0.0.1:1/v1/chat/completions",
            home.path(),
            None,
            Some(expected_error),
        );

        assert_success(&output);
    }
}

fn start_mock_provider(adapter: Adapter) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = adapter.endpoint(listener.local_addr().expect("listener address"));
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept provider request");
        let request = read_request(&mut stream);
        write_response(&mut stream, &adapter.response());

        request
    });

    (endpoint, server)
}

fn run_child(
    case: &str,
    endpoint: &str,
    home: &Path,
    api_key: Option<&str>,
    expected_error: Option<&str>,
) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args(["--exact", "credentials_resolution_child", "--nocapture"])
        .env(CHILD_CASE, case)
        .env(CHILD_ENDPOINT, endpoint)
        .env_remove(API_KEY_ENV)
        .env_remove(CHILD_EXPECTED_ERROR);
    set_config_directory(&mut command, home);

    if let Some(api_key) = api_key {
        command.env(API_KEY_ENV, api_key);
    }
    if let Some(expected_error) = expected_error {
        command.env(CHILD_EXPECTED_ERROR, expected_error);
    }

    command.output().expect("run isolated credential test")
}

fn set_config_directory(command: &mut Command, home: &Path) {
    #[cfg(target_os = "windows")]
    command.env("APPDATA", home);

    #[cfg(target_os = "macos")]
    command.env("HOME", home);

    #[cfg(all(unix, not(target_os = "macos")))]
    command.env("XDG_CONFIG_HOME", home);
}

fn credentials_path(home: &Path) -> PathBuf {
    #[cfg(target_os = "windows")]
    let config_directory = home.to_path_buf();

    #[cfg(target_os = "macos")]
    let config_directory = home.join("Library/Application Support");

    #[cfg(all(unix, not(target_os = "macos")))]
    let config_directory = home.to_path_buf();

    config_directory.join("dirule/credentials.env")
}

fn write_credentials(path: &Path, contents: &str) {
    write_credentials_bytes(path, contents.as_bytes());
}

fn write_credentials_bytes(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("credentials parent"))
        .expect("create config directory");
    std::fs::write(path, contents).expect("write test credentials");

    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
        .expect("make test credentials private");
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "credential child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_authorization(request: &str, expected: &str) {
    let headers = request.split_once("\r\n\r\n").expect("request headers").0;
    let authorization = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("authorization")
                .then(|| value.trim())
        })
        .expect("authorization header");

    assert_eq!(authorization, expected);
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    let mut request_length = None;

    loop {
        let read = stream.read(&mut buffer).expect("read provider request");
        assert_ne!(read, 0, "connection closed before request completed");
        bytes.extend_from_slice(&buffer[..read]);

        if request_length.is_none()
            && let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .expect("content length header");
            request_length = Some(end + 4 + length);
        }

        if let Some(length) = request_length
            && bytes.len() >= length
        {
            bytes.truncate(length);
            return String::from_utf8(bytes).expect("provider request UTF-8");
        }
    }
}

fn write_response(stream: &mut TcpStream, response: &serde_json::Value) {
    let body = serde_json::to_vec(response).expect("mock response JSON");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write response headers");
    stream.write_all(&body).expect("write response body");
}
