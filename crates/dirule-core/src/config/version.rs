use crate::{Error, Result};
use serde::Deserialize;
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConfigVersion {
    major: u32,
    minor: u32,
}

pub const CURRENT_CONFIG_VERSION: ConfigVersion = ConfigVersion::new(1, 3);

impl ConfigVersion {
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    pub(super) const fn immediately_follows(self, previous: Self) -> bool {
        let follows_minor = match previous.minor.checked_add(1) {
            Some(minor) => self.major == previous.major && self.minor == minor,
            None => false,
        };
        let follows_major = match previous.major.checked_add(1) {
            Some(major) => self.major == major && self.minor == 0,
            None => false,
        };
        follows_minor || follows_major
    }

    fn parse(value: &str) -> std::result::Result<Self, String> {
        let Some((major, minor)) = value.split_once('.') else {
            return Err("expected a version string in `major.minor` form".into());
        };
        if minor.contains('.') {
            return Err("expected exactly two numeric version components".into());
        }

        let major = parse_component(major, "major")?;
        let minor = parse_component(minor, "minor")?;
        Ok(Self { major, minor })
    }
}

impl FromStr for ConfigVersion {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Self::parse(value)
    }
}

fn parse_component(value: &str, name: &str) -> std::result::Result<u32, String> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "{name} version must be a canonical non-negative integer"
        ));
    }

    value
        .parse()
        .map_err(|_| format!("{name} version exceeds the supported range"))
}

impl fmt::Display for ConfigVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl<'de> Deserialize<'de> for ConfigVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

pub(super) fn read(document: &toml::Value) -> Result<ConfigVersion> {
    let value = document
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| Error::Config("`version` must be a string in `major.minor` form".into()))?;
    ConfigVersion::parse(value).map_err(Error::Config)
}
