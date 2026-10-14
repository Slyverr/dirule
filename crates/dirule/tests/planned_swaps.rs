use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_PROJECT: AtomicU64 = AtomicU64::new(0);

struct TempProject {
    root: PathBuf,
    state: PathBuf,
}

impl TempProject {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEMP_PROJECT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "dirule-swap-{}-{stamp}-{sequence}",
            std::process::id()
        ));
        let state = root.with_extension("state");
        fs::create_dir(&root).expect("create test project");

        Self { root, state }
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
        #[cfg(target_os = "windows")]
        command.env("LOCALAPPDATA", &self.state);
        #[cfg(target_os = "macos")]
        command.env("HOME", &self.state);
        #[cfg(all(unix, not(target_os = "macos")))]
        command.env("XDG_STATE_HOME", &self.state);

        command
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("run dirule")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.state);
    }
}

#[test]
fn configured_file_swap_plans_applies_and_undoes() {
    let project = TempProject::new();
    fs::write(project.root.join("first.txt"), "first").expect("write first source");
    fs::write(project.root.join("second.txt"), "second").expect("write second source");
    fs::write(
        project.root.join("dirule.toml"),
        r#"
version = "1.0"

[[rules]]
name = "Move first into second's path"
match = "first.txt"
to = "template:second.txt"

[[rules]]
name = "Move second into first's path"
match = "second.txt"
to = "template:first.txt"
"#,
    )
    .expect("write swap config");

    let plan = project.run(&["plan", "--format", "json"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    assert!(plan["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(
        plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["source"] != entry["destination"])
            .count(),
        2
    );

    let apply = project.run(&["apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    let run_id = apply["execution"]["run_id"].as_str().expect("run ID");
    assert_eq!(
        fs::read_to_string(project.root.join("first.txt")).unwrap(),
        "second"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("second.txt")).unwrap(),
        "first"
    );

    let undo = project.run(&["undo", run_id, "--yes", "--format", "json"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(
        fs::read_to_string(project.root.join("first.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("second.txt")).unwrap(),
        "second"
    );
}

#[test]
fn configured_file_cycle_plans_applies_and_undoes() {
    let project = TempProject::new();
    fs::write(project.root.join("first.txt"), "first").expect("write first source");
    fs::write(project.root.join("second.txt"), "second").expect("write second source");
    fs::write(project.root.join("third.txt"), "third").expect("write third source");
    fs::write(
        project.root.join("dirule.toml"),
        r#"
version = "1.0"

[[rules]]
name = "Move first into second's path"
match = "first.txt"
to = "template:second.txt"

[[rules]]
name = "Move second into third's path"
match = "second.txt"
to = "template:third.txt"

[[rules]]
name = "Move third into first's path"
match = "third.txt"
to = "template:first.txt"
"#,
    )
    .expect("write cycle config");

    let plan = project.run(&["plan", "--format", "json"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    assert!(plan["conflicts"].as_array().unwrap().is_empty());
    assert_eq!(
        plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["source"] != entry["destination"])
            .count(),
        3
    );

    let apply = project.run(&["apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    let run_id = apply["execution"]["run_id"].as_str().expect("run ID");
    assert_eq!(
        fs::read_to_string(project.root.join("first.txt")).unwrap(),
        "third"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("second.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("third.txt")).unwrap(),
        "second"
    );

    let undo = project.run(&["undo", run_id, "--yes", "--format", "json"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(
        fs::read_to_string(project.root.join("first.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("second.txt")).unwrap(),
        "second"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("third.txt")).unwrap(),
        "third"
    );
}

#[test]
fn configured_directory_swap_preserves_descendants_and_undoes() {
    let project = TempProject::new();
    fs::create_dir(project.root.join("first")).expect("create first directory");
    fs::create_dir(project.root.join("second")).expect("create second directory");
    fs::write(project.root.join("first/first.txt"), "first").expect("write first descendant");
    fs::write(project.root.join("second/second.txt"), "second").expect("write second descendant");
    fs::write(
        project.root.join("dirule.toml"),
        r#"
version = "1.0"

[[rules]]
name = "Move first into second's path"
match = "first"
kind = "directory"
to = "template:second"

[[rules]]
name = "Move second into first's path"
match = "second"
kind = "directory"
to = "template:first"
"#,
    )
    .expect("write directory swap config");

    let plan = project.run(&["plan", "--format", "json"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).expect("valid plan JSON");
    assert!(plan["conflicts"].as_array().unwrap().is_empty());
    assert!(
        plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["source"] == "first" && entry["destination"] == "second" })
    );
    assert!(
        plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["source"] == "second" && entry["destination"] == "first" })
    );

    let apply = project.run(&["apply", "--yes", "--format", "json"]);
    assert!(
        apply.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    let apply: serde_json::Value = serde_json::from_slice(&apply.stdout).expect("valid apply JSON");
    let run_id = apply["execution"]["run_id"].as_str().expect("run ID");
    assert_eq!(
        fs::read_to_string(project.root.join("first/second.txt")).unwrap(),
        "second"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("second/first.txt")).unwrap(),
        "first"
    );

    let undo = project.run(&["undo", run_id, "--yes", "--format", "json"]);
    assert!(
        undo.status.success(),
        "{}",
        String::from_utf8_lossy(&undo.stderr)
    );
    assert_eq!(
        fs::read_to_string(project.root.join("first/first.txt")).unwrap(),
        "first"
    );
    assert_eq!(
        fs::read_to_string(project.root.join("second/second.txt")).unwrap(),
        "second"
    );
}
