use std::io::{self, BufRead, Read, Write};
use std::process;

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut root_uri = String::new();

    loop {
        let Some(request) = read_message(&mut reader)? else {
            return Ok(());
        };
        if request.contains("\"method\":\"initialize\"") {
            root_uri = string_field(&request, "rootUri")
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing rootUri"))?;
            write_message(
                &mut writer,
                r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"workspace":{"fileOperations":{"willRename":true}}}}}"#,
            )?;
        } else if request.contains("\"method\":\"workspace/willRenameFiles\"") {
            let module_uri = format!("{root_uri}/src/http_client.rs");
            let lib_uri = format!("{root_uri}/src/lib.rs");
            let api_uri = format!("{root_uri}/src/api.rs");
            let result = format!(
                r#"{{"jsonrpc":"2.0","id":2,"result":{{"documentChanges":[{{"textDocument":{{"uri":"{module_uri}","version":null}},"edits":[{{"range":{{"start":{{"line":0,"character":11}},"end":{{"line":0,"character":17}}}},"newText":"ApiClient"}}]}},{{"textDocument":{{"uri":"{lib_uri}","version":null}},"edits":[{{"range":{{"start":{{"line":0,"character":8}},"end":{{"line":0,"character":18}}}},"newText":"http_client"}},{{"range":{{"start":{{"line":1,"character":8}},"end":{{"line":1,"character":18}}}},"newText":"http_client"}}]}},{{"textDocument":{{"uri":"{api_uri}","version":null}},"edits":[{{"range":{{"start":{{"line":0,"character":12}},"end":{{"line":0,"character":22}}}},"newText":"http_client"}}]}}]}}}}"#
            );
            write_message(&mut writer, &result)?;
        } else if request.contains("\"method\":\"shutdown\"") {
            write_message(&mut writer, r#"{"jsonrpc":"2.0","id":3,"result":null}"#)?;
        } else if request.contains("\"method\":\"exit\"") {
            process::exit(0);
        }
    }
}

fn string_field(message: &str, name: &str) -> Option<String> {
    let marker = format!("\"{name}\":\"");
    let start = message.find(&marker)? + marker.len();
    let tail = &message[start..];
    let end = tail.find('"')?;
    Some(tail[..end].to_owned())
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse::<usize>().ok();
        }
    }
    let length = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn write_message(writer: &mut impl Write, body: &str) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}
