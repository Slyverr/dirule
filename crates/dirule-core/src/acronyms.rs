use crate::{Error, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::ops::Range;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcronymOptions {
    #[serde(default)]
    pub entries: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct AcronymLexicon {
    entries: Vec<String>,
    groups: Vec<AcronymGroup>,
}

#[derive(Clone, Debug)]
struct AcronymGroup {
    initial: char,
    range: Range<usize>,
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
            let left_initial = left.chars().next().expect("validated non-empty acronym");
            let right_initial = right.chars().next().expect("validated non-empty acronym");
            left_initial
                .cmp(&right_initial)
                .then_with(|| right.chars().count().cmp(&left.chars().count()))
                .then_with(|| left.cmp(right))
        });

        let mut groups = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let initial = entry.chars().next().expect("validated non-empty acronym");
            match groups.last_mut() {
                Some(AcronymGroup {
                    initial: previous,
                    range,
                }) if *previous == initial => range.end = index + 1,
                _ => groups.push(AcronymGroup {
                    initial,
                    range: index..index + 1,
                }),
            }
        }

        Ok(AcronymLexicon { entries, groups })
    }
}

impl AcronymLexicon {
    pub(crate) fn entries_starting_with(&self, initial: char) -> impl Iterator<Item = &str> {
        let range = self
            .groups
            .binary_search_by_key(&initial, |group| group.initial)
            .ok()
            .map(|index| self.groups[index].range.clone())
            .unwrap_or_default();

        self.entries[range].iter().map(String::as_str)
    }
}
