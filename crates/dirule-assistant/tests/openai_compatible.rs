use dirule_assistant::{
    ApprovalRequirement, Message, ModelProvider, ModelRequest, OpenAiCompatibleConfig,
    OpenAiCompatibleProvider, ToolDefinition,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn sends_chat_completion_tool_schema_and_parses_tool_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept provider request");
        let request = read_request(&mut stream);
        let (headers, body) = request.split_once("\r\n\r\n").expect("request headers");
        assert!(
            headers
                .lines()
                .any(|line| line.eq_ignore_ascii_case("authorization: Bearer test-secret"))
        );

        let request: Value = serde_json::from_str(body).expect("request JSON");
        assert_eq!(request["model"], "demo-model");
        assert_eq!(request["tools"][0]["function"]["name"], "list_categories");
        assert_eq!(request["tool_choice"], "auto");

        write_json(
            &mut stream,
            &json!({
                "choices": [{
                    "message": {
                        "content": null,
                        "tool_calls": [{
                            "id": "call-1",
                            "type": "function",
                            "function": {
                                "name": "list_categories",
                                "arguments": "{}"
                            }
                        }]
                    }
                }]
            }),
        );
    });

    let provider = OpenAiCompatibleProvider::with_api_key(
        OpenAiCompatibleConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
        "test-secret",
    )
    .expect("valid mock endpoint");
    let request = ModelRequest {
        model: String::new(),
        messages: vec![
            Message::System {
                content: "Use Dirule tools.".into(),
            },
            Message::User {
                content: "List the categories.".into(),
            },
        ],
        tools: vec![ToolDefinition {
            name: "list_categories".into(),
            description: "List available categories.".into(),
            parameters: json!({"type": "object", "properties": {}}),
            approval: ApprovalRequirement::None,
        }],
    };

    let response = provider
        .complete_with_timeout(&request, Duration::from_secs(1))
        .expect("provider response");
    server.join().expect("mock provider thread");

    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "list_categories");
    assert_eq!(response.tool_calls[0].arguments, json!({}));
    assert!(!format!("{provider:?}").contains("test-secret"));
}

#[test]
fn applies_the_assistant_deadline_to_the_http_request() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().expect("accept provider request");
        thread::sleep(Duration::from_millis(500));
    });
    let provider = OpenAiCompatibleProvider::with_api_key(
        OpenAiCompatibleConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
        "test-secret",
    )
    .expect("valid mock endpoint");
    let request = ModelRequest {
        model: "demo-model".into(),
        messages: vec![Message::User {
            content: "hello".into(),
        }],
        tools: Vec::new(),
    };
    let started = Instant::now();
    let error = provider
        .complete_with_timeout(&request, Duration::from_millis(100))
        .expect_err("provider should time out before the server responds");
    let elapsed = started.elapsed();

    server.join().expect("mock provider thread");

    assert!(elapsed < Duration::from_millis(400));
    assert!(!error.to_string().contains("test-secret"));
}

#[test]
fn rejects_non_https_remote_endpoints() {
    let error = OpenAiCompatibleProvider::with_api_key(
        OpenAiCompatibleConfig::new("http://example.com/v1/chat/completions", "model", "KEY"),
        "secret",
    )
    .expect_err("remote HTTP endpoint should be rejected");

    assert!(error.to_string().contains("HTTPS"));
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    let mut header_end = None;
    let mut request_length = None;

    loop {
        let read = stream.read(&mut buffer).expect("read request");
        assert_ne!(read, 0, "connection closed before request completed");
        bytes.extend_from_slice(&buffer[..read]);

        if header_end.is_none()
            && let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
        {
            header_end = Some(end + 4);
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

        if request_length.is_some_and(|length| bytes.len() >= length) {
            bytes.truncate(request_length.unwrap());
            return String::from_utf8(bytes).expect("request UTF-8");
        }
    }
}

fn write_json(stream: &mut TcpStream, value: &Value) {
    let body = serde_json::to_vec(value).expect("response JSON");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write response headers");
    stream.write_all(&body).expect("write response body");
}
