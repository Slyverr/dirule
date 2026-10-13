use crate::{Error, Result};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcronymOptions {
    #[serde(default)]
    pub entries: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AcronymLexicon {
    entries: Vec<String>,
}

impl AcronymOptions {
    pub(crate) fn compile(&self) -> Result<AcronymLexicon> {
        let mut seen = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            if entry.is_empty() || !entry.chars().all(char::is_alphabetic) {
                return Err(Error::Config(format!(
                    "acronym {entry:?} must contain only letters"
                )));
            }

            if !seen.insert(entry.as_str()) {
                return Err(Error::Config(format!(
                    "acronym {entry:?} is listed more than once"
                )));
            }
        }

        let mut entries = self.entries.clone();
        entries.sort_by(|left, right| {
            right
                .chars()
                .count()
                .cmp(&left.chars().count())
                .then_with(|| left.cmp(right))
        });

        Ok(AcronymLexicon { entries })
    }
}

impl AcronymLexicon {
    pub(crate) fn entries(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(String::as_str)
    }
}
