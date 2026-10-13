use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("invalid glob pattern: {0}")]
    Pattern(String),
    #[error("invalid destination template: {0}")]
    Template(String),
    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),
}
