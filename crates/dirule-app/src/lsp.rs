use dirule_core::{Config, Entry, EntryKind, Plan, ReferenceUpdatePolicy};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const INITIALIZATION_QUIET_PERIOD: Duration = Duration::from_millis(500);
const INITIALIZATION_WITHOUT_PROGRESS: Duration = Duration::from_secs(2);

/// A source document changed by a language server's file-rename response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferenceChange {
    /// Path relative to the operation target.
    pub path: PathBuf,
    /// UTF-8 document contents before applying the server's edits.
    pub before: String,
    /// UTF-8 document contents after applying the server's edits.
    pub after: String,
}

/// The language-server result for one read-only filesystem plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReferenceUpdatePreview {
    /// Documents whose contents will change when the plan is applied.
    pub changes: Vec<ReferenceChange>,
    /// Non-fatal failures in `best-effort` mode.
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct LspError(String);

impl fmt::Display for LspError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for LspError {}

impl From<io::Error> for LspError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

/// Ask configured language servers for `workspace/willRenameFiles` edits.
///
/// This performs no filesystem writes. Returned edits are validated and
/// materialized as complete before/after document text for later apply.
pub fn preview_reference_updates(
    config: &Config,
    config_path: &Path,
    target: &Path,
    scanned_entries: &[Entry],
    plan: &Plan,
) -> Result<ReferenceUpdatePreview, LspError> {
    let mut preview = ReferenceUpdatePreview::default();
    if config.lsp.reference_updates == ReferenceUpdatePolicy::Off {
        return Ok(preview);
    }

    let target = target.canonicalize().map_err(|error| {
        LspError(format!(
            "cannot resolve LSP target {}: {error}",
            target.display()
        ))
    })?;
    let canonical_config = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let config_directory = canonical_config.parent().unwrap_or(&target);
    // A language server may return edits for any document in its workspace.
    // Restrict those edits to regular files Dirule actually scanned, so VCS,
    // .diruleignore, and config exclusions remain authoritative.
    let scanned_files = scanned_entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| entry.path.clone())
        .collect::<HashSet<_>>();
    let destination_sources = planned_file_destinations(plan, &scanned_files);
    let mut combined = BTreeMap::<PathBuf, (String, String)>::new();

    for server in &config.lsp.servers {
        let relevant = rename_files_for_server(&target, plan, &scanned_files, &server.languages)?;
        for (root, files) in rename_files_by_root(&target, &server.root_markers, &relevant)? {
            let result = run_will_rename(
                server,
                config_directory,
                &root,
                &target,
                &scanned_files,
                &files,
            )
            .and_then(|workspace_edit| {
                collect_workspace_edits(
                    &workspace_edit,
                    &target,
                    &scanned_files,
                    &destination_sources,
                )
            })
            .and_then(|edits| {
                let mut changes = BTreeMap::new();
                for (path, edits) in edits {
                    let before = read_regular_text_file(&path)?;
                    let after = apply_text_edits(&before, &edits)?;
                    if before == after {
                        continue;
                    }
                    let relative = path.strip_prefix(&target).map_err(|_| {
                        LspError(format!(
                            "language server returned an edit outside target {}: {}",
                            target.display(),
                            path.display()
                        ))
                    })?;
                    changes.insert(relative.to_path_buf(), (before, after));
                }
                Ok(changes)
            });
            match result {
                Ok(changes) => {
                    let overlapping = changes.iter().find_map(|(path, (before, after))| {
                        combined
                            .get(path)
                            .and_then(|(previous_before, previous_after)| {
                                (previous_before != before || previous_after != after)
                                    .then_some(path.clone())
                            })
                    });
                    if let Some(path) = overlapping {
                        let message = format!(
                            "multiple language servers returned incompatible edits for {}",
                            path.display()
                        );
                        if config.lsp.reference_updates == ReferenceUpdatePolicy::Required {
                            return Err(LspError(message));
                        }
                        preview.warnings.push(message);
                        continue;
                    }
                    combined.extend(changes);
                }
                Err(error) => {
                    let message = format!("{}: {error}", server.command.display());
                    if config.lsp.reference_updates == ReferenceUpdatePolicy::Required {
                        return Err(LspError(format!(
                            "required language-server reference update failed ({message}); install/configure the server or set lsp.reference_updates = \"best-effort\" or \"off\""
                        )));
                    }
                    preview.warnings.push(message);
                }
            }
        }
    }

    preview.changes = combined
        .into_iter()
        .map(|(path, (before, after))| ReferenceChange {
            path,
            before,
            after,
        })
        .collect();
    Ok(preview)
}

fn planned_file_destinations(
    plan: &Plan,
    scanned_files: &HashSet<PathBuf>,
) -> HashMap<PathBuf, PathBuf> {
    let mut counts = HashMap::<PathBuf, usize>::new();
    for entry in &plan.entries {
        if entry.kind == EntryKind::File && scanned_files.contains(&entry.source) {
            *counts.entry(entry.destination.clone()).or_default() += 1;
        }
    }

    plan.entries
        .iter()
        .filter(|entry| {
            entry.kind == EntryKind::File
                && scanned_files.contains(&entry.source)
                && counts.get(&entry.destination) == Some(&1)
        })
        .map(|entry| (entry.destination.clone(), entry.source.clone()))
        .collect()
}

fn rename_files_for_server(
    target: &Path,
    plan: &Plan,
    scanned_files: &HashSet<PathBuf>,
    languages: &BTreeMap<String, String>,
) -> Result<Vec<Value>, LspError> {
    let moved_directories = plan
        .entries
        .iter()
        .filter(|entry| {
            entry.kind == EntryKind::Directory
                && entry.source != entry.destination
                && scanned_files.iter().any(|file| {
                    file.starts_with(&entry.source)
                        && extension(file).is_some_and(|ext| languages.contains_key(ext))
                })
        })
        .collect::<Vec<_>>();

    // LSP file-operation requests describe a directory move once, without
    // listing its descendants. Omit inherited nested-directory moves too.
    let directory_operations = moved_directories
        .iter()
        .filter(|directory| {
            !moved_directories.iter().any(|ancestor| {
                ancestor.source != directory.source
                    && is_inherited_move(
                        &directory.source,
                        &directory.destination,
                        &ancestor.source,
                        &ancestor.destination,
                    )
            })
        })
        .map(|entry| (entry.source.clone(), entry.destination.clone()))
        .collect::<Vec<_>>();

    let file_operations = plan
        .entries
        .iter()
        .filter(|entry| {
            entry.kind == EntryKind::File
                && entry.source != entry.destination
                && (extension(&entry.source).is_some_and(|ext| languages.contains_key(ext))
                    || extension(&entry.destination).is_some_and(|ext| languages.contains_key(ext)))
                && !directory_operations.iter().any(|(source, destination)| {
                    is_inherited_move(&entry.source, &entry.destination, source, destination)
                })
        })
        .map(|entry| (entry.source.clone(), entry.destination.clone()))
        .collect::<Vec<_>>();

    directory_operations
        .into_iter()
        .chain(file_operations)
        .map(|(source, destination)| {
            let old_path = target.join(source);
            let new_path = target.join(destination);
            Ok(json!({
                "oldUri": file_uri(&old_path)?,
                "newUri": file_uri(&new_path)?,
            }))
        })
        .collect()
}

fn is_inherited_move(
    source: &Path,
    destination: &Path,
    parent_source: &Path,
    parent_destination: &Path,
) -> bool {
    source != parent_source
        && source
            .strip_prefix(parent_source)
            .ok()
            .zip(destination.strip_prefix(parent_destination).ok())
            .is_some_and(|(source_suffix, destination_suffix)| source_suffix == destination_suffix)
}

fn extension(path: &Path) -> Option<&str> {
    path.extension()?.to_str()
}

fn server_root(target: &Path, markers: &[String]) -> PathBuf {
    let root = if markers.is_empty() {
        target.to_path_buf()
    } else {
        target
            .ancestors()
            .find(|candidate| markers.iter().any(|marker| candidate.join(marker).exists()))
            .map(Path::to_path_buf)
            .unwrap_or_else(|| target.to_path_buf())
    };
    root.canonicalize().unwrap_or(root)
}

fn rename_files_by_root(
    target: &Path,
    markers: &[String],
    files: &[Value],
) -> Result<BTreeMap<PathBuf, Vec<Value>>, LspError> {
    let mut groups = BTreeMap::<PathBuf, Vec<Value>>::new();
    for file in files {
        let uri = file
            .get("oldUri")
            .and_then(Value::as_str)
            .ok_or_else(|| LspError("rename operation is missing oldUri".into()))?;
        let path = Url::parse(uri)
            .map_err(|error| LspError(format!("invalid rename file URI: {error}")))?
            .to_file_path()
            .map_err(|()| LspError(format!("rename operation is not a local file URI: {uri}")))?;
        let start = if path.is_dir() {
            path.as_path()
        } else {
            path.parent().unwrap_or(target)
        };
        let root = start
            .ancestors()
            .find(|candidate| markers.iter().any(|marker| candidate.join(marker).exists()))
            .map(Path::to_path_buf)
            .unwrap_or_else(|| server_root(target, markers));
        let root = root.canonicalize().unwrap_or(root);
        groups.entry(root).or_default().push(file.clone());
    }
    Ok(groups)
}

fn file_uri(path: &Path) -> Result<String, LspError> {
    Url::from_file_path(path)
        .map(|uri| uri.to_string())
        .map_err(|()| LspError(format!("cannot create file URI for {}", path.display())))
}

fn run_will_rename(
    server: &dirule_core::LspServerConfig,
    config_directory: &Path,
    root: &Path,
    target: &Path,
    scanned_files: &HashSet<PathBuf>,
    files: &[Value],
) -> Result<Value, LspError> {
    let open_documents = documents_to_open(target, scanned_files, &server.languages, files)?;
    let executable = resolve_command(&server.command, config_directory);
    let mut child = Command::new(&executable)
        .args(&server.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            LspError(format!(
                "cannot start {} at {}: {error}",
                server.command.display(),
                executable.display()
            ))
        })?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| LspError("language server stdin is unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LspError("language server stdout is unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LspError("language server stderr is unavailable".into()))?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || read_messages(stdout, sender));
    let stderr_reader = thread::spawn(move || read_stderr(stderr));
    let result = request_rename(&mut child, stdin, &receiver, root, files, &open_documents);
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    let stderr = stderr_reader
        .join()
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    match (result, stderr.is_empty()) {
        (Ok(result), _) => Ok(result),
        (Err(error), true) => Err(error),
        (Err(error), false) => Err(LspError(format!("{error}; server stderr: {stderr}"))),
    }
}

fn read_stderr(mut reader: impl Read) -> io::Result<String> {
    const CAPTURE_LIMIT: usize = 64 * 1024;
    let mut captured = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = CAPTURE_LIMIT.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(String::from_utf8_lossy(&captured).trim().to_owned())
}

fn request_rename(
    child: &mut Child,
    mut stdin: std::process::ChildStdin,
    messages: &Receiver<Result<Value, String>>,
    root: &Path,
    files: &[Value],
    open_documents: &[Value],
) -> Result<Value, LspError> {
    let root_uri = file_uri(root)?;
    let mut registrations = HashMap::new();
    send_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": root_uri,
                "workspaceFolders": [{"uri": root_uri, "name": root.file_name().unwrap_or_default().to_string_lossy()}],
                "capabilities": {
                    "window": {"workDoneProgress": true},
                    "workspace": {
                        "configuration": true,
                        "workspaceFolders": true,
                        "workspaceEdit": {"documentChanges": true},
                        "fileOperations": {
                            "dynamicRegistration": true,
                            "willRename": true
                        }
                    },
                    "textDocument": {"publishDiagnostics": {"relatedInformation": true}}
                }
            }
        }),
    )?;
    let initialized = response_for(&mut stdin, messages, root, 1, &mut registrations)?;
    let supports_static_rename = initialized
        .get("result")
        .and_then(|value| value.get("capabilities"))
        .and_then(|value| value.get("workspace"))
        .and_then(|value| value.get("fileOperations"))
        .and_then(|value| value.get("willRename"))
        .is_some_and(|capability| capability.as_bool() == Some(true) || capability.is_object());
    send_message(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
    )?;
    wait_for_workspace_ready(&mut stdin, messages, root, &mut registrations)?;
    let supports_dynamic_rename = registrations
        .values()
        .any(|method| method == "workspace/willRenameFiles");
    if !supports_static_rename && !supports_dynamic_rename {
        return Err(LspError(
            "server did not advertise or register workspace file-rename support".into(),
        ));
    }
    for document in open_documents {
        send_message(&mut stdin, document)?;
    }
    send_message(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "workspace/willRenameFiles",
            "params": {"files": files}
        }),
    )?;
    let result = response_for(&mut stdin, messages, root, 2, &mut registrations)?
        .get("result")
        .cloned()
        .unwrap_or(Value::Null);

    let _ = send_message(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":3,"method":"shutdown","params":null}),
    );
    let _ = response_for(&mut stdin, messages, root, 3, &mut registrations);
    let _ = send_message(
        &mut stdin,
        &json!({"jsonrpc":"2.0","method":"exit","params":null}),
    );
    drop(stdin);
    let _ = child.try_wait();
    Ok(result)
}

fn documents_to_open(
    target: &Path,
    scanned_files: &HashSet<PathBuf>,
    languages: &BTreeMap<String, String>,
    files: &[Value],
) -> Result<Vec<Value>, LspError> {
    let mut paths = HashSet::new();
    for operation in files {
        let uri = operation
            .get("oldUri")
            .and_then(Value::as_str)
            .ok_or_else(|| LspError("rename operation is missing oldUri".into()))?;
        let path = Url::parse(uri)
            .map_err(|error| LspError(format!("invalid rename file URI: {error}")))?
            .to_file_path()
            .map_err(|()| LspError(format!("rename operation is not a local file URI: {uri}")))?;
        // File URIs may preserve a lexical alias such as macOS `/var` or a
        // Windows non-extended path while the operation target is canonical.
        // Resolve the existing source before enforcing the target boundary.
        let path = canonicalize_with_missing_tail(&path)?;
        let target = canonicalize_with_missing_tail(target)?;
        let relative = path.strip_prefix(&target).map_err(|_| {
            LspError(format!(
                "rename operation is outside target {}: {}",
                target.display(),
                path.display()
            ))
        })?;

        if scanned_files.contains(relative) {
            paths.insert(relative.to_path_buf());
        } else {
            paths.extend(
                scanned_files
                    .iter()
                    .filter(|candidate| candidate.starts_with(relative))
                    .cloned(),
            );
        }
    }

    paths
        .into_iter()
        .filter_map(|relative| {
            let language_id = languages.get(extension(&relative)?)?;
            Some((relative, language_id))
        })
        .map(|(relative, language_id)| {
            let path = target.join(&relative);
            let text = read_regular_text_file(&path)?;
            Ok(json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": file_uri(&path)?,
                        "languageId": language_id,
                        "version": 1,
                        "text": text
                    }
                }
            }))
        })
        .collect()
}

fn wait_for_workspace_ready(
    stdin: &mut std::process::ChildStdin,
    messages: &Receiver<Result<Value, String>>,
    root: &Path,
    registrations: &mut HashMap<String, String>,
) -> Result<(), LspError> {
    let started = Instant::now();
    let mut last_progress = started;
    let mut saw_progress = false;
    let mut active_tokens = HashSet::new();

    loop {
        if started.elapsed() >= INITIALIZATION_TIMEOUT {
            return Err(LspError(format!(
                "language server did not finish workspace loading within {} seconds",
                INITIALIZATION_TIMEOUT.as_secs()
            )));
        }

        let message = match messages.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => message.map_err(LspError)?,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(LspError(
                    "language server exited while loading its workspace".into(),
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if saw_progress
                    && active_tokens.is_empty()
                    && last_progress.elapsed() >= INITIALIZATION_QUIET_PERIOD
                {
                    return Ok(());
                }
                if !saw_progress && started.elapsed() >= INITIALIZATION_WITHOUT_PROGRESS {
                    return Ok(());
                }
                continue;
            }
        };

        if respond_to_server_request(stdin, &message, root, registrations)? {
            continue;
        }

        if message.get("method").and_then(Value::as_str) == Some("$/progress") {
            let params = message.get("params");
            let token = params
                .and_then(|params| params.get("token"))
                .map(Value::to_string);
            let kind = params
                .and_then(|params| params.get("value"))
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str);
            if let (Some(token), Some(kind)) = (token, kind) {
                match kind {
                    "begin" => {
                        saw_progress = true;
                        active_tokens.insert(token);
                        last_progress = Instant::now();
                    }
                    "end" => {
                        saw_progress = true;
                        active_tokens.remove(&token);
                        last_progress = Instant::now();
                    }
                    "report" => {
                        last_progress = Instant::now();
                    }
                    _ => {}
                }
            }
        }
    }
}

fn send_message(writer: &mut impl Write, message: &Value) -> Result<(), LspError> {
    let body = serde_json::to_vec(message).map_err(|error| LspError(error.to_string()))?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

fn response_for(
    stdin: &mut std::process::ChildStdin,
    messages: &Receiver<Result<Value, String>>,
    root: &Path,
    id: u64,
    registrations: &mut HashMap<String, String>,
) -> Result<Value, LspError> {
    loop {
        let message = messages
            .recv_timeout(RESPONSE_TIMEOUT)
            .map_err(|error| {
                LspError(format!(
                    "timed out waiting for language-server response: {error}"
                ))
            })?
            .map_err(LspError)?;
        if respond_to_server_request(stdin, &message, root, registrations)? {
            continue;
        }
        if message.get("id").and_then(Value::as_u64) != Some(id) {
            continue;
        }
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(LspError(
                "language-server response has an invalid JSON-RPC version".into(),
            ));
        }
        if let Some(error) = message.get("error") {
            if !error.is_object() || message.get("result").is_some() {
                return Err(LspError(
                    "language-server response has an invalid error result".into(),
                ));
            }
            return Err(LspError(format!("language-server request failed: {error}")));
        }
        if message.get("result").is_none() {
            return Err(LspError(
                "language-server response is missing result".into(),
            ));
        }
        return Ok(message);
    }
}

fn respond_to_server_request(
    stdin: &mut std::process::ChildStdin,
    message: &Value,
    root: &Path,
    registrations: &mut HashMap<String, String>,
) -> Result<bool, LspError> {
    let (Some(id), Some(method)) = (
        message.get("id"),
        message.get("method").and_then(Value::as_str),
    ) else {
        return Ok(false);
    };

    let result = match method {
        "window/workDoneProgress/create"
        | "workspace/semanticTokens/refresh"
        | "workspace/codeLens/refresh"
        | "workspace/inlayHint/refresh"
        | "workspace/diagnostic/refresh"
        | "workspace/inlineValue/refresh" => Value::Null,
        "client/registerCapability" => {
            let values = message
                .get("params")
                .and_then(|params| params.get("registrations"))
                .and_then(Value::as_array)
                .ok_or_else(|| LspError("language-server registrations must be an array".into()))?;
            for registration in values {
                let id = registration
                    .get("id")
                    .ok_or_else(|| LspError("language-server registration is missing id".into()))?;
                let method = registration
                    .get("method")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        LspError("language-server registration is missing method".into())
                    })?;
                registrations.insert(id.to_string(), method.to_owned());
            }
            Value::Null
        }
        "client/unregisterCapability" => {
            let params = message.get("params").ok_or_else(|| {
                LspError("language-server unregistration is missing params".into())
            })?;
            let values = params
                .get("unregisterations")
                .or_else(|| params.get("unregistrations"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    LspError("language-server unregisterations must be an array".into())
                })?;
            for unregistration in values {
                let id = unregistration.get("id").ok_or_else(|| {
                    LspError("language-server unregistration is missing id".into())
                })?;
                registrations.remove(&id.to_string());
            }
            Value::Null
        }
        "workspace/configuration" => {
            let items = message
                .get("params")
                .and_then(|params| params.get("items"))
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        }
        "workspace/workspaceFolders" => {
            let uri = file_uri(root)?;
            let name = root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            json!([{"uri": uri, "name": name}])
        }
        "workspace/applyEdit" => json!({
            "applied": false,
            "failureReason": "Dirule only accepts edits returned by the rename preview"
        }),
        _ => {
            send_message(
                stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("unsupported language-server request: {method}")}
                }),
            )?;
            return Ok(true);
        }
    };
    send_message(
        stdin,
        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )?;
    Ok(true)
}

fn read_messages(stdout: ChildStdout, sender: mpsc::Sender<Result<Value, String>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_message(&mut reader) {
            Ok(Some(message)) => {
                if sender.send(Ok(message)).is_err() {
                    return;
                }
            }
            Ok(None) => return,
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return;
            }
        }
    }
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
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
    let length = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    if length > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "language-server message exceeds the 16 MiB limit",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn resolve_command(command: &Path, config_directory: &Path) -> PathBuf {
    if command.is_absolute() || command.components().count() > 1 {
        if command.is_absolute() {
            command.to_path_buf()
        } else {
            config_directory.join(command)
        }
    } else {
        std::env::var_os("PATH")
            .and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join(command))
                    .find(|candidate| candidate.is_file())
            })
            .unwrap_or_else(|| command.to_path_buf())
    }
}

fn collect_workspace_edits(
    workspace_edit: &Value,
    target: &Path,
    scanned_files: &HashSet<PathBuf>,
    destination_sources: &HashMap<PathBuf, PathBuf>,
) -> Result<BTreeMap<PathBuf, Vec<PositionedEdit>>, LspError> {
    let mut edits = BTreeMap::<PathBuf, Vec<PositionedEdit>>::new();
    if workspace_edit.is_null() {
        return Ok(edits);
    }
    let workspace_edit = workspace_edit
        .as_object()
        .ok_or_else(|| LspError("language server returned an invalid WorkspaceEdit".into()))?;
    if workspace_edit.contains_key("changes") && workspace_edit.contains_key("documentChanges") {
        return Err(LspError(
            "language server returned both changes and documentChanges".into(),
        ));
    }
    if let Some(changes) = workspace_edit.get("changes") {
        let changes = changes.as_object().ok_or_else(|| {
            LspError("language-server WorkspaceEdit.changes must be an object".into())
        })?;
        for (uri, values) in changes {
            let path = edit_path(uri, target, scanned_files, destination_sources)?;
            parse_text_edits(values, &path, &mut edits)?;
        }
    }
    if let Some(changes) = workspace_edit.get("documentChanges") {
        let changes = changes.as_array().ok_or_else(|| {
            LspError("language-server WorkspaceEdit.documentChanges must be an array".into())
        })?;
        for change in changes {
            let Some(document) = change.get("textDocument") else {
                return Err(LspError(
                    "language server returned a resource operation; only text edits are supported"
                        .into(),
                ));
            };
            let uri = document.get("uri").and_then(Value::as_str).ok_or_else(|| {
                LspError("language-server edit is missing textDocument.uri".into())
            })?;
            let path = edit_path(uri, target, scanned_files, destination_sources)?;
            let values = change
                .get("edits")
                .ok_or_else(|| LspError("language-server edit is missing edits".into()))?;
            parse_text_edits(values, &path, &mut edits)?;
        }
    }
    Ok(edits)
}

fn parse_text_edits(
    values: &Value,
    path: &Path,
    edits: &mut BTreeMap<PathBuf, Vec<PositionedEdit>>,
) -> Result<(), LspError> {
    let values = values
        .as_array()
        .ok_or_else(|| LspError("language-server text edits must be an array".into()))?;
    let target = edits.entry(path.to_path_buf()).or_default();
    for value in values {
        let range = value
            .get("range")
            .ok_or_else(|| LspError("language-server text edit is missing a range".into()))?;
        let start = parse_position(range.get("start"))?;
        let end = parse_position(range.get("end"))?;
        let new_text = value
            .get("newText")
            .and_then(Value::as_str)
            .ok_or_else(|| LspError("language-server text edit is missing newText".into()))?;
        target.push(PositionedEdit {
            start,
            end,
            new_text: new_text.to_owned(),
        });
    }
    Ok(())
}

fn parse_position(value: Option<&Value>) -> Result<(usize, usize), LspError> {
    let value =
        value.ok_or_else(|| LspError("language-server edit is missing a position".into()))?;
    let line = value
        .get("line")
        .and_then(Value::as_u64)
        .ok_or_else(|| LspError("language-server edit has an invalid line".into()))?
        as usize;
    let character = value
        .get("character")
        .and_then(Value::as_u64)
        .ok_or_else(|| LspError("language-server edit has an invalid character".into()))?
        as usize;
    Ok((line, character))
}

fn edit_path(
    uri: &str,
    target: &Path,
    scanned_files: &HashSet<PathBuf>,
    destination_sources: &HashMap<PathBuf, PathBuf>,
) -> Result<PathBuf, LspError> {
    let requested = Url::parse(uri)
        .map_err(|error| LspError(format!("language-server returned an invalid URI: {error}")))?
        .to_file_path()
        .map_err(|()| {
            LspError(format!(
                "language-server edit is not a local file URI: {uri}"
            ))
        })?;
    // Resolve existing ancestors as well as OS-specific path aliases. The
    // final component may not exist yet when the server returns a post-rename
    // URI, so canonicalize the longest existing prefix and append its tail.
    let normalized = canonicalize_with_missing_tail(&requested)?;
    let normalized_target = canonicalize_with_missing_tail(target)?;
    let requested_relative = normalized.strip_prefix(&normalized_target).map_err(|_| {
        LspError(format!(
            "language-server edit is outside target {}: {}",
            normalized_target.display(),
            normalized.display()
        ))
    })?;

    if scanned_files.contains(requested_relative) {
        return Ok(normalized);
    }

    // A server may key an edit by the post-rename URI, which does not exist
    // until apply. Translate that URI back to its scanned source document.
    if let Some(source) = destination_sources.get(requested_relative) {
        let canonical = canonicalize_with_missing_tail(&target.join(source)).map_err(|error| {
            LspError(format!(
                "cannot resolve scanned language-server document {}: {error}",
                source.display()
            ))
        })?;
        if canonical.starts_with(&normalized_target) && scanned_files.contains(source) {
            return Ok(canonical);
        }
    }

    // If the URI existed but was ignored or excluded, do not permit the
    // server to broaden Dirule's scan boundary.
    Err(LspError(format!(
        "language server returned an edit for a document Dirule did not scan: {}",
        requested_relative.display()
    )))
}

fn canonicalize_with_missing_tail(path: &Path) -> Result<PathBuf, LspError> {
    let normalized = normalize_absolute_path(path)?;
    let mut existing = normalized.as_path();
    let mut missing = Vec::new();

    loop {
        match existing.canonicalize() {
            Ok(mut canonical) => {
                for component in missing.into_iter().rev() {
                    canonical.push(component);
                }
                return normalize_absolute_path(&canonical);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Do not treat a broken symlink as an ordinary missing path
                // component. That could otherwise hide an escaping symlink.
                match fs::symlink_metadata(existing) {
                    Ok(_) => {
                        return Err(LspError(format!(
                            "cannot resolve language-server path {}: {error}",
                            path.display()
                        )));
                    }
                    Err(metadata_error) if metadata_error.kind() == io::ErrorKind::NotFound => {}
                    Err(metadata_error) => {
                        return Err(LspError(format!(
                            "cannot inspect language-server path {}: {metadata_error}",
                            path.display()
                        )));
                    }
                }

                let Some(name) = existing.file_name() else {
                    return Err(LspError(format!(
                        "cannot resolve language-server path {}: {error}",
                        path.display()
                    )));
                };
                missing.push(name.to_os_string());
                existing = existing.parent().ok_or_else(|| {
                    LspError(format!(
                        "cannot resolve language-server path {}: {error}",
                        path.display()
                    ))
                })?;
            }
            Err(error) => {
                return Err(LspError(format!(
                    "cannot resolve language-server path {}: {error}",
                    path.display()
                )));
            }
        }
    }
}

fn normalize_absolute_path(path: &Path) -> Result<PathBuf, LspError> {
    if !path.is_absolute() {
        return Err(LspError(format!(
            "language-server file URI is not an absolute path: {}",
            path.display()
        )));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized.file_name().is_some() {
                    normalized.pop();
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    Ok(normalized)
}

#[derive(Clone)]
struct PositionedEdit {
    start: (usize, usize),
    end: (usize, usize),
    new_text: String,
}

fn read_regular_text_file(path: &Path) -> Result<String, LspError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        LspError(format!(
            "cannot inspect edited file {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(LspError(format!(
            "language server returned an edit for a non-regular file: {}",
            path.display()
        )));
    }
    fs::read_to_string(path).map_err(|error| {
        LspError(format!(
            "cannot read edited file {}: {error}",
            path.display()
        ))
    })
}

fn apply_text_edits(source: &str, edits: &[PositionedEdit]) -> Result<String, LspError> {
    let mut byte_edits = edits
        .iter()
        .map(|edit| {
            let start = byte_offset(source, edit.start)?;
            let end = byte_offset(source, edit.end)?;
            if start > end {
                return Err(LspError("language-server edit range is reversed".into()));
            }
            Ok((start, end, edit.new_text.as_str()))
        })
        .collect::<Result<Vec<_>, LspError>>()?;
    byte_edits.sort_by_key(|edit| std::cmp::Reverse(edit.0));

    let mut result = source.to_owned();
    let mut previous_start = None;
    for (start, end, replacement) in byte_edits {
        if previous_start.is_some_and(|previous| end > previous) {
            return Err(LspError(
                "language-server returned overlapping text edits".into(),
            ));
        }
        result.replace_range(start..end, replacement);
        previous_start = Some(start);
    }
    Ok(result)
}

fn byte_offset(
    source: &str,
    (wanted_line, wanted_character): (usize, usize),
) -> Result<usize, LspError> {
    let mut offset = 0;
    for (line_number, line) in source.split('\n').enumerate() {
        if line_number == wanted_line {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut utf16_offset = 0;
            for (byte_index, character) in line.char_indices() {
                if utf16_offset == wanted_character {
                    return Ok(offset + byte_index);
                }
                utf16_offset += character.len_utf16();
                if utf16_offset > wanted_character {
                    return Err(LspError(
                        "language-server position splits a UTF-16 character".into(),
                    ));
                }
            }
            if utf16_offset == wanted_character {
                return Ok(offset + line.len());
            }
            return Err(LspError(
                "language-server character position is outside the line".into(),
            ));
        }
        offset += line.len() + 1;
    }
    Err(LspError(
        "language-server line position is outside the document".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        apply_text_edits, collect_workspace_edits, documents_to_open, edit_path, file_uri,
        planned_file_destinations, rename_files_by_root, rename_files_for_server,
    };
    use dirule_core::{Conflict, EntryKind, Plan, PlannedEntry};
    use std::collections::{BTreeMap, HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use url::Url;

    #[test]
    fn workspace_edits_reject_malformed_or_ambiguous_shapes() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path().canonicalize().expect("canonical target");
        let scanned = HashSet::new();
        let destinations = HashMap::new();

        for workspace_edit in [
            serde_json::json!("not an edit"),
            serde_json::json!({"changes": []}),
            serde_json::json!({"documentChanges": {}}),
            serde_json::json!({"changes": {}, "documentChanges": []}),
        ] {
            assert!(
                collect_workspace_edits(&workspace_edit, &target, &scanned, &destinations).is_err(),
                "expected malformed WorkspaceEdit to be rejected: {workspace_edit}"
            );
        }
        assert!(
            collect_workspace_edits(&serde_json::Value::Null, &target, &scanned, &destinations)
                .expect("null WorkspaceEdit means no edits")
                .is_empty()
        );
    }

    #[test]
    fn text_edit_positions_follow_utf16_and_crlf_line_endings() {
        let source = "// 😀\r\nmod HTTPClient;\r\n";
        let edited = apply_text_edits(
            source,
            &[super::PositionedEdit {
                start: (1, 4),
                end: (1, 14),
                new_text: "http_client".into(),
            }],
        )
        .expect("apply an edit after a non-BMP character on CRLF text");

        assert_eq!(edited, "// 😀\r\nmod http_client;\r\n");
    }

    #[test]
    fn language_server_edits_are_limited_to_scanned_files() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path().canonicalize().expect("canonical target");
        fs::write(target.join("visible.rs"), "mod old;\n").expect("write visible file");
        fs::write(target.join("ignored.rs"), "mod secret;\n").expect("write ignored file");
        let scanned = HashSet::from([PathBuf::from("visible.rs")]);
        let destinations = HashMap::new();

        let visible_uri = file_uri(&target.join("visible.rs")).expect("visible file URI");
        assert!(edit_path(&visible_uri, &target, &scanned, &destinations).is_ok());

        let ignored_uri = file_uri(&target.join("ignored.rs")).expect("ignored file URI");
        let error = edit_path(&ignored_uri, &target, &scanned, &destinations)
            .expect_err("unscanned file edits must be rejected");
        assert!(error.to_string().contains("Dirule did not scan"));
    }

    #[test]
    fn language_server_edits_at_post_rename_uri_map_back_to_scanned_source() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path().canonicalize().expect("canonical target");
        let source = PathBuf::from("src/HTTPClient.rs");
        let destination = PathBuf::from("src/http_client.rs");
        fs::create_dir_all(target.join("src")).expect("create source directory");
        fs::write(target.join(&source), "pub struct Client;\n").expect("write source file");
        let scanned = HashSet::from([source.clone()]);
        let destinations = HashMap::from([(destination.clone(), source.clone())]);
        let new_uri = file_uri(&target.join(destination)).expect("new file URI");

        assert_eq!(
            edit_path(&new_uri, &target, &scanned, &destinations).expect("map to source"),
            target
                .join(source)
                .canonicalize()
                .expect("canonical source")
        );
    }

    #[test]
    fn server_file_operations_send_a_directory_rename_once_for_its_rust_files() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path();
        let plan = Plan {
            entries: vec![
                PlannedEntry {
                    source: PathBuf::from("OldModule"),
                    destination: PathBuf::from("old-module"),
                    kind: EntryKind::Directory,
                    rule: Some("normalize directories".into()),
                },
                PlannedEntry {
                    source: PathBuf::from("OldModule/mod.rs"),
                    destination: PathBuf::from("old-module/mod.rs"),
                    kind: EntryKind::File,
                    rule: None,
                },
            ],
            conflicts: Vec::<Conflict>::new(),
            findings: Vec::new(),
        };
        let scanned_files = HashSet::from([PathBuf::from("OldModule/mod.rs")]);
        let languages = BTreeMap::from([("rs".into(), "rust".into())]);

        let events = rename_files_for_server(target, &plan, &scanned_files, &languages)
            .expect("create language-server file operations");

        assert_eq!(events.len(), 1);
        assert!(
            events[0]["oldUri"]
                .as_str()
                .unwrap()
                .ends_with("/OldModule")
        );
        assert!(
            events[0]["newUri"]
                .as_str()
                .unwrap()
                .ends_with("/old-module")
        );
    }

    #[test]
    fn rename_documents_open_only_scanned_language_files() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path().canonicalize().expect("canonical target");
        fs::create_dir_all(target.join("OldModule")).expect("create renamed directory");
        fs::write(target.join("OldModule/main.rs"), "mod child;\n")
            .expect("write scanned Rust file");
        fs::write(target.join("OldModule/ignored.rs"), "mod secret;\n")
            .expect("write excluded Rust file");
        let scanned_files = HashSet::from([PathBuf::from("OldModule/main.rs")]);
        let languages = BTreeMap::from([("rs".into(), "rust".into())]);
        let operations = vec![serde_json::json!({
            "oldUri": file_uri(&target.join("OldModule")).expect("old directory URI"),
            "newUri": file_uri(&target.join("old-module")).expect("new directory URI"),
        })];

        let documents = documents_to_open(&target, &scanned_files, &languages, &operations)
            .expect("prepare documents for LSP");

        assert_eq!(documents.len(), 1);
        assert_eq!(
            documents[0]["params"]["textDocument"]["uri"],
            file_uri(&target.join("OldModule/main.rs")).expect("source file URI")
        );
        assert_eq!(documents[0]["params"]["textDocument"]["languageId"], "rust");
        assert_eq!(
            documents[0]["params"]["textDocument"]["text"],
            "mod child;\n"
        );
    }

    #[test]
    fn server_file_operations_keep_explicit_file_renames_under_a_directory_move() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path();
        let plan = Plan {
            entries: vec![
                PlannedEntry {
                    source: PathBuf::from("OldModule"),
                    destination: PathBuf::from("old-module"),
                    kind: EntryKind::Directory,
                    rule: Some("normalize directories".into()),
                },
                PlannedEntry {
                    source: PathBuf::from("OldModule/HTTPClient.rs"),
                    destination: PathBuf::from("old-module/http_client.rs"),
                    kind: EntryKind::File,
                    rule: Some("normalize Rust files".into()),
                },
            ],
            conflicts: Vec::<Conflict>::new(),
            findings: Vec::new(),
        };
        let scanned_files = HashSet::from([PathBuf::from("OldModule/HTTPClient.rs")]);
        let languages = BTreeMap::from([("rs".into(), "rust".into())]);

        let events = rename_files_for_server(target, &plan, &scanned_files, &languages)
            .expect("create language-server file operations");

        assert_eq!(events.len(), 2);
        assert!(
            events[0]["oldUri"]
                .as_str()
                .unwrap()
                .ends_with("/OldModule")
        );
        assert!(
            events[1]["oldUri"]
                .as_str()
                .unwrap()
                .ends_with("/OldModule/HTTPClient.rs")
        );
    }

    #[test]
    fn workspace_root_markers_are_searched_from_the_operation_target_upward() {
        let directory = tempfile::tempdir().expect("create temporary workspace");
        let workspace = directory.path().join("workspace");
        let target = workspace.join("crates/example");
        fs::create_dir_all(&target).expect("create nested target");
        fs::write(workspace.join("Cargo.toml"), "[workspace]\n").expect("write marker");

        assert_eq!(
            super::server_root(&target, &["Cargo.toml".into()]),
            workspace.canonicalize().expect("canonical workspace")
        );
    }

    #[cfg(unix)]
    #[test]
    fn missing_path_tail_resolves_through_symlinked_ancestor() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let target = directory.path().join("target");
        let alias = directory.path().join("alias");
        fs::create_dir(&target).expect("create target directory");
        std::os::unix::fs::symlink(&target, &alias).expect("create target alias");

        let requested = alias.join("nested/new.rs");
        let resolved = super::canonicalize_with_missing_tail(&requested)
            .expect("resolve missing path below the symlink");
        assert_eq!(
            resolved,
            target.canonicalize().unwrap().join("nested/new.rs")
        );
    }

    #[test]
    fn renamed_files_are_grouped_by_language_server_root_below_target() {
        let directory = tempfile::tempdir().expect("create multi-project target");
        let target = directory.path().join("multilang");
        let rust_root = target.join("rust");
        let typescript_root = target.join("typescript");
        fs::create_dir_all(rust_root.join("src")).expect("create Rust source directory");
        fs::create_dir_all(typescript_root.join("src"))
            .expect("create TypeScript source directory");
        fs::write(rust_root.join("Cargo.toml"), "[package]\n").expect("write Rust marker");
        fs::write(typescript_root.join("tsconfig.json"), "{}\n").expect("write TypeScript marker");
        let rust_file = rust_root.join("src/HTTPClient.rs");
        let typescript_file = typescript_root.join("src/RecordParser.ts");
        fs::write(&rust_file, "").expect("write Rust source");
        fs::write(&typescript_file, "").expect("write TypeScript source");
        let files = vec![
            serde_json::json!({
                "oldUri": file_uri(&rust_file).expect("Rust source URI"),
                "newUri": file_uri(&rust_root.join("src/http_client.rs")).expect("Rust destination URI"),
            }),
            serde_json::json!({
                "oldUri": file_uri(&typescript_file).expect("TypeScript source URI"),
                "newUri": file_uri(&typescript_root.join("src/record-parser.ts")).expect("TypeScript destination URI"),
            }),
        ];

        let groups = rename_files_by_root(
            &target,
            &["Cargo.toml".into(), "tsconfig.json".into()],
            &files,
        )
        .expect("group rename operations by workspace marker");

        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups
                .get(&rust_root.canonicalize().expect("canonical Rust root"))
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            groups
                .get(
                    &typescript_root
                        .canonicalize()
                        .expect("canonical TypeScript root")
                )
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn reference_update_policies_control_missing_server_behavior() {
        let directory = tempfile::tempdir().expect("create temporary target");
        let target = directory.path();
        fs::write(target.join("HTTPClient.rs"), "pub struct Client;\n").expect("write source file");
        let source = crate::configuration::ConfigurationSource::project_default(target);
        let config_path = target.join("dirule.toml");
        let config = |policy: &str| {
            format!(
                r#"version = "1.5"
target = "."

[lsp]
reference_updates = "{policy}"

[[rules]]
name = "Normalize Rust module filename"
match = "HTTPClient.rs"
kind = "file"
to = "builtin:snake-case"

[[lsp.servers]]
command = "dirule-test-no-such-language-server-5a1276"

[lsp.servers.languages]
rs = "rust"
"#
            )
        };

        fs::write(&config_path, config("required")).expect("write required config");
        let error = crate::preview_source(&source, target, Some(target.to_path_buf()))
            .err()
            .expect("required policy should reject a missing server");
        assert!(
            error
                .to_string()
                .contains("required language-server reference update failed")
        );

        fs::write(&config_path, config("best-effort")).expect("write best-effort config");
        let preview = crate::preview_source(&source, target, Some(target.to_path_buf()))
            .expect("best-effort should retain the path plan");
        assert!(preview.plan().has_changes());
        assert_eq!(preview.reference_updates().changes.len(), 0);
        assert_eq!(preview.reference_updates().warnings.len(), 1);

        fs::write(&config_path, config("off")).expect("write off config");
        let preview = crate::preview_source(&source, target, Some(target.to_path_buf()))
            .expect("off should skip unavailable servers");
        assert!(preview.plan().has_changes());
        assert!(preview.reference_updates().warnings.is_empty());
    }

    #[test]
    fn duplicate_planned_destinations_are_not_eligible_uri_aliases() {
        let scanned = HashSet::from([PathBuf::from("A.rs"), PathBuf::from("B.rs")]);
        let plan = Plan {
            entries: vec![
                PlannedEntry {
                    source: PathBuf::from("A.rs"),
                    destination: PathBuf::from("same.rs"),
                    kind: EntryKind::File,
                    rule: None,
                },
                PlannedEntry {
                    source: PathBuf::from("B.rs"),
                    destination: PathBuf::from("same.rs"),
                    kind: EntryKind::File,
                    rule: None,
                },
            ],
            conflicts: Vec::new(),
            findings: Vec::new(),
        };

        assert!(planned_file_destinations(&plan, &scanned).is_empty());
    }

    #[test]
    fn file_uris_roundtrip_unicode_and_reserved_path_characters() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let path = directory.path().join("name with spaces # café.rs");
        let uri = file_uri(&path).expect("create file URI");
        assert!(uri.contains("name%20with%20spaces%20%23%20caf%C3%A9.rs"));
        let normalized = super::normalize_absolute_path(&path).expect("normalize path");
        assert_eq!(normalized, path);
        assert!(Path::new(&normalized).is_absolute());
        let decoded = Url::parse(&uri)
            .expect("parse file URI")
            .to_file_path()
            .expect("convert file URI to path");
        assert_eq!(
            super::normalize_absolute_path(&decoded).expect("normalize decoded path"),
            normalized
        );
    }

    #[test]
    fn preview_apply_and_undo_use_language_server_reference_edits() {
        let directory = tempfile::tempdir().expect("create temporary parent");
        let target = directory.path().join("project");
        fs::create_dir_all(target.join("src")).expect("create Rust source directory");
        fs::write(
            target.join("Cargo.toml"),
            "[package]\nname = \"lsp-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write marker");
        fs::write(target.join("src/HTTPClient.rs"), "pub struct Client;\n")
            .expect("write module source");
        fs::write(
            target.join("src/lib.rs"),
            "pub mod HTTPClient;\npub use HTTPClient::Client as SharedClient;\n",
        )
        .expect("write module references");
        fs::write(
            target.join("src/api.rs"),
            "use crate::{HTTPClient::Client as ApiClient};\n",
        )
        .expect("write API reference");

        let fake_server_source =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_server.rs");
        let fake_server = directory.path().join(if cfg!(windows) {
            "fake-lsp-server.exe"
        } else {
            "fake-lsp-server"
        });
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let compiled = Command::new(rustc)
            .arg("--edition=2024")
            .arg(&fake_server_source)
            .arg("-o")
            .arg(&fake_server)
            .status()
            .expect("compile fake language server");
        assert!(
            compiled.success(),
            "fake language server compilation failed"
        );
        let new_uri = file_uri(&target.join("src/http_client.rs")).expect("new module URI");
        let lib_uri = file_uri(&target.join("src/lib.rs")).expect("lib.rs URI");
        let api_uri = file_uri(&target.join("src/api.rs")).expect("api.rs URI");
        let executable = serde_json::to_string(&fake_server.to_string_lossy().to_string())
            .expect("quote test executable path");
        let new_uri = serde_json::to_string(&new_uri).expect("quote new module URI");
        let lib_uri = serde_json::to_string(&lib_uri).expect("quote lib.rs URI");
        let api_uri = serde_json::to_string(&api_uri).expect("quote api.rs URI");
        let config = format!(
            r#"version = "1.5"
target = "."

[lsp]
reference_updates = "required"

[[rules]]
name = "Normalize Rust module filename"
match = "src/HTTPClient.rs"
kind = "file"
to = "builtin:snake-case"

[[lsp.servers]]
command = {executable}
args = [{new_uri}, {lib_uri}, {api_uri}]
root_markers = ["Cargo.toml"]

[lsp.servers.languages]
rs = "rust"
"#
        );
        fs::write(target.join("dirule.toml"), config).expect("write project config");

        let source = crate::configuration::ConfigurationSource::project_default(&target);
        let preview = crate::preview_source(&source, &target, Some(target.clone()))
            .expect("preview with fake language server");
        assert_eq!(preview.reference_updates().changes.len(), 3);
        assert!(preview.reference_updates().changes.iter().any(|change| {
            change.path == Path::new("src/HTTPClient.rs")
                && change.after == "pub struct ApiClient;\n"
        }));
        assert!(preview.reference_updates().changes.iter().any(|change| {
            change.path == Path::new("src/lib.rs")
                && change.after
                    == "pub mod http_client;\npub use http_client::Client as SharedClient;\n"
        }));
        assert!(preview.reference_updates().changes.iter().any(|change| {
            change.path == Path::new("src/api.rs")
                && change.after == "use crate::{http_client::Client as ApiClient};\n"
        }));

        let journal = crate::journal::JournalService::new(dirule_fs::JournalStore::new(
            directory.path().join("journal"),
        ));
        let applied = crate::apply_authorized(preview, dirule_fs::ConflictPolicy::Abort, &journal)
            .expect("apply path and document changes");
        let run_id = applied.run_id.expect("journal run ID");
        assert_eq!(applied.report.moved.len(), 1);
        assert_eq!(applied.reference_files, 3);
        assert!(!target.join("src/HTTPClient.rs").exists());
        assert_eq!(
            fs::read_to_string(target.join("src/http_client.rs")).unwrap(),
            "pub struct ApiClient;\n"
        );
        assert!(
            fs::read_to_string(target.join("src/lib.rs"))
                .unwrap()
                .contains("http_client")
        );
        assert!(
            fs::read_to_string(target.join("src/api.rs"))
                .unwrap()
                .contains("http_client")
        );

        journal
            .undo_authorized(&run_id)
            .expect("undo path and document changes");
        assert!(target.join("src/HTTPClient.rs").is_file());
        assert!(!target.join("src/http_client.rs").exists());
        assert_eq!(
            fs::read_to_string(target.join("src/HTTPClient.rs")).unwrap(),
            "pub struct Client;\n"
        );
        assert!(
            fs::read_to_string(target.join("src/lib.rs"))
                .unwrap()
                .contains("HTTPClient")
        );
        assert!(
            fs::read_to_string(target.join("src/api.rs"))
                .unwrap()
                .contains("HTTPClient")
        );
    }
}
