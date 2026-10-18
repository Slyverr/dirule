use super::common::*;
use serde_json::{Value, json};
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::thread;

#[test]
fn ask_uses_a_profile_tool_loop_and_sends_no_file_contents() {
    let project = TempProject::new();
    fs::write(project.0.join("Photo.JPG"), "private image bytes").expect("write photo");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind model endpoint");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept model request");
            let (headers, body) = read_request(&mut stream);
            assert!(
                headers
                    .lines()
                    .any(|line| line
                        .eq_ignore_ascii_case("authorization: Bearer dirule-test-secret"))
            );
            let body: Value = serde_json::from_str(&body).expect("request JSON");
            requests.push(body);

            if index == 0 {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": null,
                                "tool_calls": [{
                                    "id": "call-sort",
                                    "type": "function",
                                    "function": {
                                        "name": "preview_sort",
                                        "arguments": "{\"selectors\":[\".jpg\"]}"
                                    }
                                }]
                            }
                        }]
                    }),
                );
            } else {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": "**Preview:**\n\n- `Photo.JPG` → `jpgs/Photo.JPG`\n\nNo files were changed.\u{1b}[2J\u{1b}]0;attacker title\u{7}"
                            }
                        }]
                    }),
                );
            }
        }

        requests
    });

    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create assistant config directory");
    fs::write(
        config_directory.join("assistant.toml"),
        format!(
            "version = \"1.0\"\ndefault_profile = \"local\"\n\n[profiles.local]\nadapter = \"openai-compatible\"\nendpoint = {endpoint:?}\nmodel = \"test-model\"\napi_key_env = \"DIRULE_TEST_KEY\"\n"
        ),
    )
    .expect("write assistant profile");
    let mut command = project.command(&[
        "--color",
        "always",
        "ask",
        "--target",
        ".",
        "Sort JPG files.",
    ]);
    let output = command
        .env("DIRULE_TEST_KEY", "dirule-test-secret")
        .output()
        .expect("run ask");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(stdout.contains("Preview:"));
    assert!(stdout.contains("Photo.JPG"));
    assert!(stdout.contains("No files were changed"));
    assert!(!stdout.contains("**"));
    assert!(!stdout.contains('`'));
    assert!(!stdout.contains("[2J"));
    assert!(!stdout.contains("attacker title"));
    assert!(stdout.contains("\u{1b}["));
    assert!(stderr.contains("profile \"local\" · model \"test-model\""));
    assert!(stderr.contains("Target:"));
    assert!(stderr.contains("May send:"));
    assert!(stderr.contains("File contents are not read"));
    assert!(stderr.contains("Waiting for model · turn 1/8"));
    assert!(stderr.contains("Running tool · preview_sort"));
    assert!(stderr.contains("Waiting for model · turn 2/8"));
    assert!(!stderr.contains(&endpoint));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("dirule-test-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("dirule-test-secret"));
    assert_eq!(text(&project.0.join("Photo.JPG")), "private image bytes");
    assert!(!project.0.join("jpgs/Photo.JPG").exists());

    let requests = server.join().expect("model test server");
    let encoded = serde_json::to_string(&requests).expect("encode captured requests");
    assert!(encoded.contains("Sort JPG files."));
    assert!(encoded.contains("Photo.JPG"));
    assert!(encoded.contains("jpgs/Photo.JPG"));
    assert!(!encoded.contains("private image bytes"));
}

#[test]
fn ask_sanitizes_model_supplied_tool_names_in_progress_output() {
    let project = TempProject::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind model endpoint");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let malicious_name = "unsafe\u{1b}[2J\nFORGED".to_owned();
    let provider_name = malicious_name.clone();
    let server = thread::spawn(move || {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept model request");
            let (_, body) = read_request(&mut stream);
            let _: Value = serde_json::from_str(&body).expect("request JSON");

            if index == 0 {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": null,
                                "tool_calls": [{
                                    "id": "call-unknown",
                                    "type": "function",
                                    "function": {
                                        "name": provider_name,
                                        "arguments": "{}"
                                    }
                                }]
                            }
                        }]
                    }),
                );
            } else {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{"message": {"content": "The requested tool is unavailable."}}]
                    }),
                );
            }
        }
    });

    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create assistant config directory");
    fs::write(
        config_directory.join("assistant.toml"),
        format!(
            "version = \"1.0\"\ndefault_profile = \"local\"\n\n[profiles.local]\nadapter = \"openai-compatible\"\nendpoint = {endpoint:?}\nmodel = \"test-model\"\napi_key_env = \"DIRULE_TEST_KEY\"\n"
        ),
    )
    .expect("write assistant profile");
    let output = project
        .command(&["ask", "--target", ".", "Inspect this folder."])
        .env("DIRULE_TEST_KEY", "dirule-test-secret")
        .output()
        .expect("run ask");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Running tool · unsafe?"));
    assert!(!stderr.contains('\u{1b}'));
    assert!(!stderr.contains("\nFORGED"));
    assert!(!stderr.contains("dirule-test-secret"));
    server.join().expect("model test server");
}

#[test]
fn ask_reports_provider_failure_without_echoing_response_body_or_credentials() {
    let project = TempProject::new();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind model endpoint");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept model request");
        let (_, body) = read_request(&mut stream);
        let _: Value = serde_json::from_str(&body).expect("request JSON");
        let response_body = "provider-private-response";
        write!(
            stream,
            "HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        )
        .expect("write provider error");
    });

    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create assistant config directory");
    fs::write(
        config_directory.join("assistant.toml"),
        format!(
            "version = \"1.0\"\ndefault_profile = \"local\"\n\n[profiles.local]\nadapter = \"openai-compatible\"\nendpoint = {endpoint:?}\nmodel = \"test-model\"\napi_key_env = \"DIRULE_TEST_KEY\"\n"
        ),
    )
    .expect("write assistant profile");
    let output = project
        .command(&["ask", "--target", ".", "Inspect this folder."])
        .env("DIRULE_TEST_KEY", "dirule-test-secret")
        .output()
        .expect("run ask");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Waiting for model · turn 1/8"));
    assert!(stderr.contains("HTTP status 503"));
    assert!(!stderr.contains("provider-private-response"));
    assert!(!stderr.contains("dirule-test-secret"));
    server.join().expect("model test server");
}

#[test]
fn ask_refuses_model_requested_changes_without_an_interactive_terminal() {
    let project = TempProject::new();
    fs::write(project.0.join("Portrait.JPG"), "private image bytes").expect("write photo");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind model endpoint");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept model request");
            let (_, body) = read_request(&mut stream);
            let body: Value = serde_json::from_str(&body).expect("request JSON");
            requests.push(body.clone());

            if index == 0 {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": null,
                                "tool_calls": [{
                                    "id": "call-apply",
                                    "type": "function",
                                    "function": {
                                        "name": "apply_sort",
                                        "arguments": "{\"selectors\":[\"images\"]}"
                                    }
                                }]
                            }
                        }]
                    }),
                );
            } else {
                let encoded = serde_json::to_string(&body).expect("encode second request");
                assert!(encoded.contains("interactive terminal"));
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": "No changes were applied because approval needs an interactive terminal."
                            }
                        }]
                    }),
                );
            }
        }

        requests
    });

    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create assistant config directory");
    fs::write(
        config_directory.join("assistant.toml"),
        format!(
            "version = \"1.0\"\ndefault_profile = \"local\"\n\n[profiles.local]\nadapter = \"openai-compatible\"\nendpoint = {endpoint:?}\nmodel = \"test-model\"\napi_key_env = \"DIRULE_TEST_KEY\"\n"
        ),
    )
    .expect("write assistant profile");
    let mut command = project.command(&["ask", "--target", ".", "Sort these images."]);
    let output = command
        .env("DIRULE_TEST_KEY", "dirule-test-secret")
        .output()
        .expect("run ask");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains('\u{1b}'));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Apply sorting"));
    assert!(stderr.contains("Portrait.JPG"));
    assert!(stderr.contains("media/images/Portrait.JPG"));
    assert_eq!(text(&project.0.join("Portrait.JPG")), "private image bytes");
    assert!(!project.0.join("media/images/Portrait.JPG").exists());
    assert!(project.journal_directory().read_dir().is_err());

    let requests = server.join().expect("model test server");
    let encoded = serde_json::to_string(&requests).expect("encode captured requests");
    assert!(encoded.contains("interactive terminal"));
    assert!(!encoded.contains("private image bytes"));
}

#[test]
fn ask_discovers_project_rules_and_previews_them_without_mutation() {
    let project = TempProject::new();
    let config = format!(
        "version = {:?}\nname = \"Photo cleanup\"\ntarget = \".\"\n\n[[rules]]\nname = \"Normalize photos\"\nmatch = \"<title>.JPG\"\nto = \"template:${{title|kebab-case}}.jpg\"\n",
        dirule_core::CURRENT_CONFIG_VERSION.to_string()
    );
    fs::write(project.0.join("dirule.toml"), &config).expect("write project config");
    fs::write(project.0.join("Summer Photo.JPG"), "private image bytes").expect("write photo");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind model endpoint");
    let endpoint = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept model request");
            let (_, body) = read_request(&mut stream);
            let body: Value = serde_json::from_str(&body).expect("request JSON");
            requests.push(body);

            if index == 0 {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": null,
                                "tool_calls": [
                                    {
                                        "id": "inspect-rules",
                                        "type": "function",
                                        "function": {
                                            "name": "inspect_rules",
                                            "arguments": "{}"
                                        }
                                    },
                                    {
                                        "id": "preview-rules",
                                        "type": "function",
                                        "function": {
                                            "name": "preview_configured_rules",
                                            "arguments": "{}"
                                        }
                                    }
                                ]
                            }
                        }]
                    }),
                );
            } else {
                write_json(
                    &mut stream,
                    &json!({
                        "choices": [{
                            "message": {
                                "content": "The rule would rename Summer Photo.JPG to summer-photo.jpg."
                            }
                        }]
                    }),
                );
            }
        }

        requests
    });

    let config_directory = preset_config_base(&project).join("dirule");
    fs::create_dir_all(&config_directory).expect("create assistant config directory");
    fs::write(
        config_directory.join("assistant.toml"),
        format!(
            "version = \"1.0\"\ndefault_profile = \"local\"\n\n[profiles.local]\nadapter = \"openai-compatible\"\nendpoint = {endpoint:?}\nmodel = \"test-model\"\napi_key_env = \"DIRULE_TEST_KEY\"\n"
        ),
    )
    .expect("write assistant profile");
    let output = project
        .command(&["ask", "--target", ".", "Explain the configured rule."])
        .env("DIRULE_TEST_KEY", "dirule-test-secret")
        .output()
        .expect("run ask");

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("summer-photo.jpg"));
    assert_eq!(text(&project.0.join("dirule.toml")), config);
    assert_eq!(
        text(&project.0.join("Summer Photo.JPG")),
        "private image bytes"
    );
    assert!(!project.0.join("summer-photo.jpg").exists());

    let requests = server.join().expect("model test server");
    let encoded = serde_json::to_string(&requests).expect("encode captured requests");
    assert!(encoded.contains("Normalize photos"));
    assert!(encoded.contains("Summer Photo.JPG"));
    assert!(encoded.contains("summer-photo.jpg"));
    assert!(!encoded.contains("private image bytes"));
}

fn read_request(stream: &mut TcpStream) -> (String, String) {
    let mut bytes = Vec::new();
    let mut buffer = [0; 1024];
    let mut request_length = None;

    loop {
        let read = stream.read(&mut buffer).expect("read request");
        assert_ne!(read, 0, "connection closed before request completed");
        bytes.extend_from_slice(&buffer[..read]);

        if request_length.is_none()
            && let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_length = header_end + 4;
            let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .expect("content-length header");
            request_length = Some((header_length, header_length + content_length, headers));
        }

        if let Some((header_length, total_length, headers)) = &request_length
            && bytes.len() >= *total_length
        {
            let body = String::from_utf8(bytes[*header_length..*total_length].to_vec())
                .expect("request body UTF-8");
            return (headers.clone(), body);
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
