//! Services for selecting, loading, creating, and editing rule configurations.

mod create;
mod editor;
pub(crate) mod file;
mod loader;
mod selection;
mod source;

pub use create::{ConfigCreationError, create_config, create_starter};
pub use editor::{ConfigEditor, ConfigEditorError};
pub use loader::{ConfigurationLoadError, LoadedConfiguration, load, load_source};
pub use selection::{ConfigurationSelectionError, discover_source};
pub use source::{ConfigurationSource, ConfigurationSourceKind};
