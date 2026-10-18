use super::Colorizer;
use dirule_assistant::{AssistantEvent, AssistantObserver};
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct ProgressState {
    message: Mutex<Option<String>>,
    output: Mutex<()>,
    stopped: AtomicBool,
    terminal: bool,
    styles: Colorizer,
}

#[derive(Clone)]
pub(in crate::cli) struct ProgressHandle(Arc<ProgressState>);

pub(in crate::cli) struct AssistantProgress {
    handle: ProgressHandle,
    worker: Option<JoinHandle<()>>,
}

impl AssistantProgress {
    pub(in crate::cli) fn new(color: super::super::args::ColorChoice) -> Self {
        let terminal = io::stderr().is_terminal();
        let state = Arc::new(ProgressState {
            message: Mutex::new(None),
            output: Mutex::new(()),
            stopped: AtomicBool::new(false),
            terminal,
            styles: Colorizer::stderr(color),
        });
        let worker = terminal.then(|| run_spinner(Arc::clone(&state)));

        Self {
            handle: ProgressHandle(state),
            worker,
        }
    }

    pub(in crate::cli) fn handle(&self) -> ProgressHandle {
        self.handle.clone()
    }
}

impl AssistantObserver for AssistantProgress {
    fn on_event(&mut self, event: AssistantEvent) {
        match event {
            AssistantEvent::ModelRequestStarted { turn, turn_limit } => self
                .handle
                .begin(format!("Waiting for model · turn {turn}/{turn_limit}")),
            AssistantEvent::ModelRequestFinished { turn, succeeded } => {
                let state = if succeeded {
                    "Model reply received"
                } else {
                    "Model request failed"
                };
                self.handle
                    .finish(succeeded, format!("{state} · turn {turn}"));
            }
            AssistantEvent::ToolCallStarted { name } => {
                self.handle
                    .begin(format!("Running tool · {}", safe_tool_name(&name)));
            }
            AssistantEvent::ApprovalRequired { .. } => self.handle.pause(),
            AssistantEvent::ToolCallFinished { name, succeeded } => {
                let state = if succeeded {
                    "Tool returned"
                } else {
                    "Tool failed"
                };
                self.handle
                    .finish(succeeded, format!("{state} · {}", safe_tool_name(&name)));
            }
        }
    }
}

fn safe_tool_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '?'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "unknown".to_owned()
    } else {
        sanitized
    }
}

impl ProgressHandle {
    fn begin(&self, message: String) {
        if self.0.terminal {
            let _output = lock(&self.0.output);
            *lock(&self.0.message) = Some(message.clone());
            let mut stderr = io::stderr().lock();
            let spinner = self.0.styles.style("|", "36");
            let message = self.0.styles.style(message, "90");
            let _ = write!(stderr, "\r{spinner} {message}");
            let _ = stderr.flush();
        } else {
            let styles = self.0.styles;
            let mut stderr = io::stderr().lock();
            let _ = writeln!(stderr, "{} {message}", styles.style("Ask", "36"));
        }
    }

    fn finish(&self, succeeded: bool, label: String) {
        if self.0.terminal {
            self.complete_terminal(Some((succeeded, label)));
        }
    }

    pub(in crate::cli) fn pause(&self) {
        if self.0.terminal {
            self.complete_terminal(None);
        }
    }

    fn complete_terminal(&self, result: Option<(bool, String)>) {
        let _output = lock(&self.0.output);
        lock(&self.0.message).take();
        let mut stderr = io::stderr().lock();
        let _ = write!(stderr, "\r\x1b[2K");
        if let Some((succeeded, label)) = result {
            let (symbol, color) = if succeeded {
                ("✓", "32")
            } else {
                ("!", "31")
            };
            let _ = writeln!(
                stderr,
                "{} {}",
                self.0.styles.style(symbol, color),
                self.0.styles.style(label, "90")
            );
        }
        let _ = stderr.flush();
    }
}

impl Drop for AssistantProgress {
    fn drop(&mut self) {
        self.handle.pause();
        self.handle.0.stopped.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_spinner(state: Arc<ProgressState>) -> JoinHandle<()> {
    thread::spawn(move || {
        let frames = ["|", "/", "-", "\\"];
        let mut frame = 0;

        while !state.stopped.load(Ordering::Acquire) {
            {
                let _output = lock(&state.output);
                if let Some(message) = lock(&state.message).as_ref() {
                    let mut stderr = io::stderr().lock();
                    let spinner = state.styles.style(frames[frame], "36");
                    let message = state.styles.style(message, "90");
                    let _ = write!(stderr, "\r{spinner} {message}");
                    let _ = stderr.flush();
                    frame = (frame + 1) % frames.len();
                }
            }

            thread::sleep(Duration::from_millis(90));
        }
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
