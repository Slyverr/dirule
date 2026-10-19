use dirule_assistant::{
    Message, ModelProvider, ModelRequest, OpenAiCompatibleConfig, OpenAiCompatibleProvider,
    OpenAiResponsesConfig, OpenAiResponsesProvider,
};
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

const REQUEST_LIMIT_BYTES: usize = 1024 * 1024;
const RESPONSE_LIMIT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
enum Adapter {
    ChatCompletions,
    Responses,
}

impl Adapter {
    fn endpoint(self, address: std::net::SocketAddr) -> String {
        let path = match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
        };

        format!("http://{address}{path}")
    }

    fn provider(self, endpoint: String) -> Box<dyn ModelProvider> {
        match self {
            Self::ChatCompletions => Box::new(
                OpenAiCompatibleProvider::with_api_key(
                    OpenAiCompatibleConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
                    "test-secret",
                )
                .expect("valid mock endpoint"),
            ),
            Self::Responses => Box::new(
                OpenAiResponsesProvider::with_api_key(
                    OpenAiResponsesConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
                    "test-secret",
                )
                .expect("valid mock endpoint"),
            ),
        }
    }
}

const ADAPTERS: [Adapter; 2] = [Adapter::ChatCompletions, Adapter::Responses];

#[test]
fn rejects_oversized_requests_for_both_adapters_before_sending_them() {
    for adapter in ADAPTERS {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused mock endpoint");
        listener
            .set_nonblocking(true)
            .expect("make listener nonblocking");
        let endpoint = adapter.endpoint(listener.local_addr().expect("listener address"));
        let provider = adapter.provider(endpoint);
        let request = model_request("x".repeat(REQUEST_LIMIT_BYTES));

        let error = provider
            .complete_with_timeout(&request, Duration::from_secs(1))
            .expect_err("oversized request rejected");

        assert!(
            error.to_string().contains("request exceeds"),
            "{adapter:?}: {error}"
        );
        assert!(matches!(
            listener.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
    }
}

#[test]
fn applies_the_assistant_deadline_to_both_adapters() {
    for adapter in ADAPTERS {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
        let endpoint = adapter.endpoint(listener.local_addr().expect("listener address"));
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept provider request");
            thread::sleep(Duration::from_millis(500));
        });
        let provider = adapter.provider(endpoint);
        let started = Instant::now();
        let error = provider
            .complete_with_timeout(&model_request("hello".into()), Duration::from_millis(100))
            .expect_err("provider should time out before the server responds");
        let elapsed = started.elapsed();

        server.join().expect("mock provider thread");

        assert!(
            elapsed < Duration::from_millis(400),
            "{adapter:?} took {elapsed:?} to honor the request deadline"
        );
        assert!(!error.to_string().contains("test-secret"));
    }
}

#[test]
fn rejects_oversized_responses_for_both_adapters() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let address = listener.local_addr().expect("listener address");
    let server = thread::spawn(move || {
        for _ in ADAPTERS {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let request = read_request(&mut stream);
            let request_line = request.lines().next().expect("request line");
            let body = oversized_response(request_line);
            write_response(&mut stream, "200 OK", &body);
        }
    });

    for adapter in ADAPTERS {
        let provider = adapter.provider(adapter.endpoint(address));
        let error = provider
            .complete_with_timeout(&model_request("hello".into()), Duration::from_secs(2))
            .expect_err("oversized response rejected");

        assert!(
            error
                .to_string()
                .contains("could not read provider response"),
            "{adapter:?}: {error}"
        );
        assert!(!error.to_string().contains("oversized-provider-payload"));
    }

    server.join().expect("mock provider thread");
}

#[test]
fn http_errors_do_not_expose_provider_response_bodies() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let address = listener.local_addr().expect("listener address");
    let server = thread::spawn(move || {
        for _ in ADAPTERS {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let _request = read_request(&mut stream);
            write_response(
                &mut stream,
                "401 Unauthorized",
                br#"{"error":"provider-private-detail"}"#,
            );
        }
    });

    for adapter in ADAPTERS {
        let provider = adapter.provider(adapter.endpoint(address));
        let error = provider
            .complete_with_timeout(&model_request("hello".into()), Duration::from_secs(1))
            .expect_err("provider HTTP error returned");

        assert!(error.to_string().contains("HTTP status 401"));
        assert!(!error.to_string().contains("provider-private-detail"));
        assert!(!error.to_string().contains("test-secret"));
    }

    server.join().expect("mock provider thread");
}

#[test]
fn provider_redirects_are_rejected_without_sending_data_to_the_redirect_target() {
    for adapter in ADAPTERS {
        let redirect_target = TcpListener::bind("127.0.0.1:0").expect("bind redirect target");
        redirect_target
            .set_nonblocking(true)
            .expect("make redirect target nonblocking");
        let redirect_endpoint = format!("http://{}/collect", redirect_target.local_addr().unwrap());

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
        let endpoint = adapter.endpoint(listener.local_addr().expect("listener address"));
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let _request = read_request(&mut stream);
            write_redirect_response(&mut stream, &redirect_endpoint);
        });

        let provider = adapter.provider(endpoint);
        let error = provider
            .complete_with_timeout(
                &model_request("private path data".into()),
                Duration::from_secs(1),
            )
            .expect_err("redirected provider request rejected");

        assert!(error.to_string().contains("redirected the request"));
        server.join().expect("mock provider thread");
        assert!(matches!(
            redirect_target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
    }
}

fn model_request(content: String) -> ModelRequest {
    ModelRequest {
        model: String::new(),
        messages: vec![Message::User { content }],
        tools: Vec::new(),
    }
}

fn oversized_response(request_line: &str) -> Vec<u8> {
    let content = format!(
        "oversized-provider-payload{}",
        "x".repeat(RESPONSE_LIMIT_BYTES)
    );
    let response = if request_line.contains("/v1/responses ") {
        json!({
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": content}]
            }]
        })
    } else {
        json!({
            "choices": [{"message": {"content": content}}]
        })
    };

    serde_json::to_vec(&response).expect("oversized response JSON")
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

fn write_response(stream: &mut TcpStream, status: &str, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .expect("write response headers");
    let _ = stream.write_all(body);
}

fn write_redirect_response(stream: &mut TcpStream, location: &str) {
    write!(
        stream,
        "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .expect("write redirect response");
}
