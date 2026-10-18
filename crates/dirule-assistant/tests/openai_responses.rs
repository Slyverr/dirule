use dirule_assistant::{
    ApprovalRequirement, Message, ModelProvider, ModelRequest, OpenAiResponsesConfig,
    OpenAiResponsesProvider, ToolDefinition,
};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

#[test]
fn preserves_response_items_across_approved_tool_turns() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut first_stream, _) = listener.accept().expect("accept initial request");
        let first_request = read_request(&mut first_stream);
        let (first_headers, first_body) = first_request
            .split_once("\r\n\r\n")
            .expect("initial request headers");
        assert!(first_headers.starts_with("POST /v1/responses "));
        assert!(has_test_authorization(first_headers));

        let first_request: Value = serde_json::from_str(first_body).expect("initial JSON");
        assert_eq!(first_request["model"], "demo-model");
        assert_eq!(first_request["store"], false);
        assert_eq!(first_request["tool_choice"], "auto");
        assert_eq!(first_request["tools"][0]["type"], "function");
        assert_eq!(first_request["tools"][0]["name"], "preview_sort");
        assert_eq!(first_request["tools"][0]["strict"], false);
        assert_eq!(first_request["input"][0]["role"], "system");
        assert_eq!(first_request["input"][1]["role"], "user");
        write_json(
            &mut first_stream,
            &json!({
                "status": "completed",
                "output": [
                    {"id": "rs_1", "type": "reasoning", "summary": []},
                    {
                        "id": "fc_1",
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "preview_sort",
                        "arguments": "{\"selectors\":[\"images\"]}"
                    }
                ]
            }),
        );

        let (mut second_stream, _) = listener.accept().expect("accept follow-up request");
        let second_request = read_request(&mut second_stream);
        let (second_headers, second_body) = second_request
            .split_once("\r\n\r\n")
            .expect("follow-up request headers");
        assert!(has_test_authorization(second_headers));

        let second_request: Value = serde_json::from_str(second_body).expect("follow-up JSON");
        assert_eq!(second_request["store"], false);
        assert_eq!(second_request["input"][0]["role"], "system");
        assert_eq!(second_request["input"][1]["role"], "user");
        assert_eq!(second_request["input"][2]["id"], "rs_1");
        assert_eq!(second_request["input"][3]["call_id"], "call_1");
        assert_eq!(second_request["input"][4]["type"], "function_call_output");
        assert_eq!(second_request["input"][4]["call_id"], "call_1");
        assert_eq!(second_request["input"][4]["output"], "preview result");
        write_json(
            &mut second_stream,
            &json!({
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "The preview is ready."}]
                }]
            }),
        );
    });

    let provider = OpenAiResponsesProvider::with_api_key(
        OpenAiResponsesConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
        "test-secret",
    )
    .expect("valid mock endpoint");
    let request = model_request(vec![
        Message::System {
            content: "Use Dirule tools.".into(),
        },
        Message::User {
            content: "Preview image sorting.".into(),
        },
    ]);

    let first_response = provider
        .complete_with_timeout(&request, Duration::from_secs(1))
        .expect("function-call response");
    assert_eq!(first_response.content, None);
    assert_eq!(first_response.tool_calls.len(), 1);
    assert_eq!(first_response.tool_calls[0].id, "call_1");
    assert_eq!(
        first_response.tool_calls[0].arguments,
        json!({"selectors": ["images"]})
    );

    let mut follow_up = request;
    follow_up.messages.push(Message::Assistant {
        content: None,
        tool_calls: vec![first_response.tool_calls[0].clone()],
    });
    follow_up.messages.push(Message::Tool {
        tool_call_id: "call_1".into(),
        name: "preview_sort".into(),
        content: "preview result".into(),
    });
    let final_response = provider
        .complete_with_timeout(&follow_up, Duration::from_secs(1))
        .expect("final response");
    server.join().expect("mock provider thread");

    assert_eq!(
        final_response.content.as_deref(),
        Some("The preview is ready.")
    );
    assert!(final_response.tool_calls.is_empty());
    assert!(!format!("{provider:?}").contains("test-secret"));
}

#[test]
fn preserves_reasoning_and_parallel_function_calls_across_tool_turns() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut first_stream, _) = listener.accept().expect("accept initial request");
        let first_request = read_request(&mut first_stream);
        let (_, first_body) = first_request
            .split_once("\r\n\r\n")
            .expect("initial request headers");
        let first_request: Value = serde_json::from_str(first_body).expect("initial JSON");
        assert_eq!(first_request["input"][0]["role"], "system");
        assert_eq!(first_request["input"][1]["role"], "user");
        write_json(
            &mut first_stream,
            &json!({
                "status": "completed",
                "output": [
                    {"id": "rs_2", "type": "reasoning", "summary": []},
                    {
                        "type": "function_call",
                        "call_id": "call_images",
                        "name": "preview_sort",
                        "arguments": "{\"selectors\":[\"images\"]}"
                    },
                    {
                        "type": "function_call",
                        "call_id": "call_videos",
                        "name": "preview_sort",
                        "arguments": "{\"selectors\":[\"videos\"]}"
                    }
                ]
            }),
        );

        let (mut second_stream, _) = listener.accept().expect("accept follow-up request");
        let second_request = read_request(&mut second_stream);
        let (_, second_body) = second_request
            .split_once("\r\n\r\n")
            .expect("follow-up request headers");
        let second_request: Value = serde_json::from_str(second_body).expect("follow-up JSON");
        assert_eq!(second_request["input"][2]["id"], "rs_2");
        assert_eq!(second_request["input"][3]["call_id"], "call_images");
        assert_eq!(second_request["input"][4]["call_id"], "call_videos");
        assert_eq!(second_request["input"][5]["call_id"], "call_images");
        assert_eq!(second_request["input"][5]["output"], "image preview");
        assert_eq!(second_request["input"][6]["call_id"], "call_videos");
        assert_eq!(second_request["input"][6]["output"], "video preview");
        write_json(
            &mut second_stream,
            &json!({
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Both previews are ready."}]
                }]
            }),
        );
    });

    let provider = OpenAiResponsesProvider::with_api_key(
        OpenAiResponsesConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
        "test-secret",
    )
    .expect("valid mock endpoint");
    let mut request = model_request(vec![
        Message::System {
            content: "Use Dirule tools.".into(),
        },
        Message::User {
            content: "Preview images and videos.".into(),
        },
    ]);

    let first_response = provider
        .complete_with_timeout(&request, Duration::from_secs(1))
        .expect("parallel function-call response");
    assert_eq!(first_response.tool_calls.len(), 2);
    assert_eq!(first_response.tool_calls[0].id, "call_images");
    assert_eq!(first_response.tool_calls[1].id, "call_videos");
    request.messages.push(Message::Assistant {
        content: None,
        tool_calls: first_response.tool_calls,
    });
    request.messages.push(Message::Tool {
        tool_call_id: "call_images".into(),
        name: "preview_sort".into(),
        content: "image preview".into(),
    });
    request.messages.push(Message::Tool {
        tool_call_id: "call_videos".into(),
        name: "preview_sort".into(),
        content: "video preview".into(),
    });

    let final_response = provider
        .complete_with_timeout(&request, Duration::from_secs(1))
        .expect("final response");
    server.join().expect("mock provider thread");

    assert_eq!(
        final_response.content.as_deref(),
        Some("Both previews are ready.")
    );
    assert!(final_response.tool_calls.is_empty());
}

#[test]
fn rejects_incomplete_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let _request = read_request(&mut stream);
        write_json(&mut stream, &json!({"status": "incomplete", "output": []}));
    });
    let provider = OpenAiResponsesProvider::with_api_key(
        OpenAiResponsesConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
        "test-secret",
    )
    .expect("valid mock endpoint");

    let error = provider
        .complete_with_timeout(
            &model_request(vec![Message::User {
                content: "hello".into(),
            }]),
            Duration::from_secs(1),
        )
        .expect_err("incomplete response rejected");
    server.join().expect("mock provider thread");

    assert!(error.to_string().contains("incomplete response"));
}

#[test]
fn rejects_failed_responses_and_malformed_function_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock provider");
    let endpoint = format!("http://{}/v1/responses", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        for response in [
            json!({"status": "failed", "output": []}),
            json!({
                "status": "completed",
                "output": [{
                    "type": "function_call",
                    "name": "preview_sort",
                    "arguments": "{}"
                }]
            }),
        ] {
            let (mut stream, _) = listener.accept().expect("accept provider request");
            let _request = read_request(&mut stream);
            write_json(&mut stream, &response);
        }
    });
    let provider = OpenAiResponsesProvider::with_api_key(
        OpenAiResponsesConfig::new(endpoint, "demo-model", "DIRULE_TEST_KEY"),
        "test-secret",
    )
    .expect("valid mock endpoint");
    let request = model_request(vec![Message::User {
        content: "Preview images.".into(),
    }]);

    let failed = provider
        .complete_with_timeout(&request, Duration::from_secs(1))
        .expect_err("failed response rejected");
    assert!(failed.to_string().contains("failed response"));

    let malformed = provider
        .complete_with_timeout(&request, Duration::from_secs(1))
        .expect_err("malformed function call rejected");
    server.join().expect("mock provider thread");

    assert!(malformed.to_string().contains("missing its call ID"));
}

fn model_request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: String::new(),
        messages,
        tools: vec![ToolDefinition {
            name: "preview_sort".into(),
            description: "Preview a Dirule sort.".into(),
            parameters: json!({
                "type": "object",
                "properties": {"selectors": {"type": "array", "items": {"type": "string"}}},
                "required": ["selectors"],
                "additionalProperties": false
            }),
            approval: ApprovalRequirement::None,
        }],
    }
}

fn has_test_authorization(headers: &str) -> bool {
    headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("authorization: Bearer test-secret"))
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    let mut request_length = None;

    loop {
        let read = stream.read(&mut buffer).expect("read request");
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

        if request_length.is_some_and(|length| bytes.len() >= length) {
            bytes.truncate(request_length.expect("request length"));
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
