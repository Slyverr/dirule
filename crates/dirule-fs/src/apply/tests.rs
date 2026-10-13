use super::{ApplyError, EntryKind, NoopRecorder, rollback_after_failure};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dirule-rollback-test-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn rollback_failure_preserves_the_operation_error_and_occupied_paths() {
    let temp = TempDir::new();
    std::fs::write(temp.0.join("original.txt"), "external file")
        .expect("write new file at original path");
    std::fs::write(temp.0.join("renamed.txt"), "moved file")
        .expect("write moved file at destination");
    let moved = [(
        0,
        PathBuf::from("original.txt"),
        PathBuf::from("renamed.txt"),
        EntryKind::File,
    )];
    let operation = ApplyError::Io {
        path: PathBuf::from("later.txt"),
        source: std::io::Error::other("simulated later operation failure"),
    };

    let error = rollback_after_failure(operation, &temp.0, &moved, &[], &mut NoopRecorder);

    match error {
        ApplyError::RollbackFailed {
            operation,
            failures,
        } => {
            assert!(
                operation
                    .to_string()
                    .contains("simulated later operation failure")
            );
            assert!(failures.contains("renamed.txt -> original.txt"));
        }
        other => panic!("expected a rollback failure, got {other}"),
    }
    assert_eq!(
        std::fs::read_to_string(temp.0.join("original.txt")).expect("read external file"),
        "external file"
    );
    assert_eq!(
        std::fs::read_to_string(temp.0.join("renamed.txt")).expect("read moved file"),
        "moved file"
    );
}
