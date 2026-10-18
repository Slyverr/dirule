use crate::credentials::valid_environment_name;
use dirule_core::ConfigVersion;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const CURRENT_ASSISTANT_CONFIG_VERSION: ConfigVersion = ConfigVersion::new(1, 0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub enum ProviderAdapter {
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AssistantProfile {
    pub adapter: ProviderAdapter,
    pub endpoint: String,
    pub model: String,
    pub api_key_env: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantConfig {
    default_profile: String,
    profiles: BTreeMap<String, AssistantProfile>,
}

#[derive(Debug)]
pub struct AssistantConfigError {
    path: Option<PathBuf>,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssistantConfigDocument {
    version: ConfigVersion,
    default_profile: String,
    profiles: BTreeMap<String, AssistantProfile>,
}

impl AssistantConfig {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, AssistantConfigError> {
        let path = path.into();
        let contents = fs::read_to_string(&path).map_err(|error| AssistantConfigError {
            path: Some(path.clone()),
            message: format!("cannot read assistant config: {error}"),
        })?;

        Self::parse(&path, &contents)
    }

    pub fn load_user() -> Result<Self, AssistantConfigError> {
        let base = dirule_app::paths::user_config_directory().ok_or_else(|| {
            AssistantConfigError::new("cannot determine the user config directory")
        })?;

        Self::load(base.join("dirule").join("assistant.toml"))
    }

    pub fn parse(path: &Path, contents: &str) -> Result<Self, AssistantConfigError> {
        let document: AssistantConfigDocument =
            toml::from_str(contents).map_err(|error| AssistantConfigError {
                path: Some(path.to_path_buf()),
                message: format!("invalid assistant config: {error}"),
            })?;
        if document.version != CURRENT_ASSISTANT_CONFIG_VERSION {
            return Err(AssistantConfigError {
                path: Some(path.to_path_buf()),
                message: format!(
                    "assistant config version {} is unsupported; expected {}",
                    document.version, CURRENT_ASSISTANT_CONFIG_VERSION
                ),
            });
        }
        if document.default_profile.trim().is_empty() {
            return Err(AssistantConfigError {
                path: Some(path.to_path_buf()),
                message: "`default_profile` must not be empty".into(),
            });
        }
        if document.profiles.is_empty() {
            return Err(AssistantConfigError {
                path: Some(path.to_path_buf()),
                message: "at least one assistant profile is required".into(),
            });
        }
        if !document.profiles.contains_key(&document.default_profile) {
            return Err(AssistantConfigError {
                path: Some(path.to_path_buf()),
                message: format!(
                    "default profile {:?} is not defined",
                    document.default_profile
                ),
            });
        }

        for (name, profile) in &document.profiles {
            if name.trim().is_empty() {
                return Err(AssistantConfigError {
                    path: Some(path.to_path_buf()),
                    message: "profile names must not be empty".into(),
                });
            }
            if profile.endpoint.trim().is_empty() || profile.model.trim().is_empty() {
                return Err(AssistantConfigError {
                    path: Some(path.to_path_buf()),
                    message: format!("profile {name:?} needs a non-empty endpoint and model"),
                });
            }

            if !valid_environment_name(&profile.api_key_env) {
                return Err(AssistantConfigError {
                    path: Some(path.to_path_buf()),
                    message: format!("profile {name:?} has an invalid `api_key_env` variable name"),
                });
            }
        }

        Ok(Self {
            default_profile: document.default_profile,
            profiles: document.profiles,
        })
    }

    pub fn default_profile_name(&self) -> &str {
        &self.default_profile
    }

    pub fn profile(
        &self,
        name: Option<&str>,
    ) -> Result<(&str, &AssistantProfile), AssistantConfigError> {
        let name = name.unwrap_or(&self.default_profile);
        self.profiles
            .get_key_value(name)
            .map(|(name, profile)| (name.as_str(), profile))
            .ok_or_else(|| AssistantConfigError::new(format!("profile {name:?} is not defined")))
    }

    pub fn profiles(&self) -> &BTreeMap<String, AssistantProfile> {
        &self.profiles
    }
}

impl AssistantConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            path: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for AssistantConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(formatter, "{}: {}", path.display(), self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl Error for AssistantConfigError {}

impl FromStr for ProviderAdapter {
    type Err = AssistantConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "openai-compatible" => Ok(Self::OpenAiCompatible),
            _ => Err(AssistantConfigError::new(format!(
                "unsupported provider adapter {value:?}"
            ))),
        }
    }
}
