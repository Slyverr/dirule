pub(crate) use std::fs;
pub(crate) use std::io::Read;
pub(crate) use std::path::{Path, PathBuf};
pub(crate) use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_PROJECT_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TempProject(pub(crate) PathBuf);

impl TempProject {
    pub(crate) fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
            .as_nanos();
        let id = NEXT_TEMP_PROJECT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dirule-cli-{}-{stamp}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("create temp project");
        Self(path)
    }

    pub(crate) fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("run dirule")
    }

    pub(crate) fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dirule"));
        #[cfg(target_os = "windows")]
        command.env("LOCALAPPDATA", self.0.with_extension("local-app-data"));
        #[cfg(target_os = "macos")]
        command.env("HOME", self.0.with_extension("home"));
        #[cfg(all(unix, not(target_os = "macos")))]
        command.env("XDG_STATE_HOME", self.0.with_extension("state"));
        configure_preset_environment(&mut command, self);
        command.args(args).current_dir(&self.0);

        command
    }

    pub(crate) fn journal_directory(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        let base = self.0.with_extension("local-app-data");
        #[cfg(target_os = "macos")]
        let base = self
            .0
            .with_extension("home")
            .join("Library/Application Support");
        #[cfg(all(unix, not(target_os = "macos")))]
        let base = self.0.with_extension("state");

        base.join("dirule/runs")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        let _ = fs::remove_dir_all(self.0.with_extension("state"));
        let _ = fs::remove_dir_all(self.0.with_extension("local-app-data"));
        let _ = fs::remove_dir_all(self.0.with_extension("home"));
    }
}

pub(crate) fn text(path: &Path) -> String {
    fs::read_to_string(path).expect("read fixture file")
}

pub(crate) fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied fixture directory");
    for entry in fs::read_dir(source).expect("read fixture directory") {
        let entry = entry.expect("read fixture entry");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().expect("inspect fixture entry").is_dir() {
            copy_tree(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).expect("copy fixture file");
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn preset_config_base(project: &TempProject) -> PathBuf {
    project.0.join("appdata")
}

#[cfg(target_os = "macos")]
pub(crate) fn preset_config_base(project: &TempProject) -> PathBuf {
    project
        .0
        .with_extension("home")
        .join("Library/Application Support")
}

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn preset_config_base(project: &TempProject) -> PathBuf {
    project.0.join("xdg-config")
}

pub(crate) fn configure_preset_environment(command: &mut Command, project: &TempProject) {
    #[cfg(target_os = "windows")]
    command.env("APPDATA", preset_config_base(project));
    #[cfg(target_os = "macos")]
    command.env("HOME", project.0.with_extension("home"));
    #[cfg(all(unix, not(target_os = "macos")))]
    command.env("XDG_CONFIG_HOME", preset_config_base(project));
}

pub(crate) fn configure_home(command: &mut Command, home: &Path) {
    #[cfg(target_os = "windows")]
    command.env("USERPROFILE", home);
    #[cfg(not(target_os = "windows"))]
    command.env("HOME", home);
}

pub(crate) fn config_for_target(target: Option<&Path>, transform: &str) -> String {
    let target = target
        .map(|path| format!("target = {:?}\n", path.to_string_lossy()))
        .unwrap_or_default();

    format!(
        "version = \"{}\"\n{target}\n[[rules]]\nname = \"Normalize names\"\nmatch = \"**/*\"\nto = \"builtin:{transform}\"\n",
        dirule_core::CURRENT_CONFIG_VERSION
    )
}
