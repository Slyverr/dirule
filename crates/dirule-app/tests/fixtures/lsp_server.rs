use std::env;
use std::io::{self, BufRead, Write};
use std::process;

fn main() -> io::Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected URIs for the renamed module, lib.rs, and api.rs",
        ));
    }

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut opened_rename_document = false;
    loop {
        let Some(request) = read_message(&mut reader)? else {
            return Ok(());
        };
        if request.contains("\"method\":\"initialize\"") {
            if !request.contains("\"documentChanges\":true")
                || !request.contains("\"dynamicRegistration\":true")
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "client did not advertise workspace edit and file-operation capabilities",
                ));
            }
            write_message(
                &mut writer,
                r#"{"jsonrpc":"2.0","id":40,"method":"workspace/configuration","params":{"items":[{"section":"rust-analyzer"},{"section":"editor"}]}}"#,
            )?;
            let Some(configuration) = read_message(&mut reader)? else {
                return Ok(());
            };
            if !configuration.contains("\"id\":40")
                || !configuration.contains("\"result\":[null,null]")
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "client returned an invalid workspace/configuration result",
                ));
            }
            write_message(
                &mut writer,
                r#"{"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}"#,
            )?;
        } else if request.contains("\"method\":\"initialized\"") {
            write_message(
                &mut writer,
                r#"{"jsonrpc":"2.0","id":41,"method":"client/registerCapability","params":{"registrations":[{"id":"will-rename","method":"workspace/willRenameFiles","registerOptions":{"filters":[]}}]}}"#,
            )?;
            let Some(registration) = read_message(&mut reader)? else {
                return Ok(());
            };
            if !registration.contains("\"id\":41")
                || !registration.contains("\"result\":null")
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "client did not acknowledge client/registerCapability",
                ));
            }
        } else if request.contains("\"method\":\"textDocument/didOpen\"") {
            if !request.contains("\"languageId\":\"rust\"")
                || !request.contains("pub struct Client;")
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "client opened a renamed file with unexpected language or text",
                ));
            }
            opened_rename_document = true;
        } else if request.contains("\"method\":\"workspace/willRenameFiles\"") {
            if !opened_rename_document {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "client did not open the renamed source document before requesting edits",
                ));
            }
            let response = format!(
                r#"{{"jsonrpc":"2.0","id":2,"result":{{"documentChanges":[{{"textDocument":{{"uri":"{}","version":null}},"edits":[{{"range":{{"start":{{"line":0,"character":11}},"end":{{"line":0,"character":17}}}},"newText":"ApiClient"}}]}},{{"textDocument":{{"uri":"{}","version":null}},"edits":[{{"range":{{"start":{{"line":0,"character":8}},"end":{{"line":0,"character":18}}}},"newText":"http_client"}},{{"range":{{"start":{{"line":1,"character":8}},"end":{{"line":1,"character":18}}}},"newText":"http_client"}}]}},{{"textDocument":{{"uri":"{}","version":null}},"edits":[{{"range":{{"start":{{"line":0,"character":12}},"end":{{"line":0,"character":22}}}},"newText":"http_client"}}]}}]}}}}"#,
                arguments[0], arguments[1], arguments[2]
            );
            write_message(&mut writer, &response)?;
        } else if request.contains("\"method\":\"shutdown\"") {
            write_message(
                &mut writer,
                r#"{"jsonrpc":"2.0","id":3,"result":null}"#,
            )?;
        } else if request.contains("\"method\":\"exit\"") {
            process::exit(0);
        }
    }
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut length = None;
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
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
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
