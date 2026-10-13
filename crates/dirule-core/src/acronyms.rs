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
    lengths: Vec<AcronymLengthGroup>,
}

#[derive(Clone, Debug)]
struct AcronymLengthGroup {
    characters: usize,
    bytes: usize,
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
                .then_with(|| right.len().cmp(&left.len()))
                .then_with(|| left.cmp(right))
        });

        let mut groups = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let initial = entry.chars().next().expect("validated non-empty acronym");
            let characters = entry.chars().count();
            let bytes = entry.len();
            match groups.last_mut() {
                Some(AcronymGroup {
                    initial: previous,
                    lengths,
                }) if *previous == initial => match lengths.last_mut() {
                    Some(AcronymLengthGroup {
                        characters: previous_characters,
                        bytes: previous_bytes,
                        range,
                    }) if *previous_characters == characters && *previous_bytes == bytes => {
                        range.end = index + 1;
                    }
                    _ => lengths.push(AcronymLengthGroup {
                        characters,
                        bytes,
                        range: index..index + 1,
                    }),
                },
                _ => groups.push(AcronymGroup {
                    initial,
                    lengths: vec![AcronymLengthGroup {
                        characters,
                        bytes,
                        range: index..index + 1,
                    }],
                }),
            }
        }

        Ok(AcronymLexicon { entries, groups })
    }
}

impl AcronymLexicon {
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn candidates_starting_at<'a>(
        &'a self,
        initial: char,
        value: &'a str,
        start: usize,
    ) -> impl Iterator<Item = &'a str> + 'a {
        let lengths = self
            .groups
            .binary_search_by_key(&initial, |group| group.initial)
            .ok()
            .map(|index| self.groups[index].lengths.as_slice())
            .unwrap_or(&[]);

        lengths.iter().filter_map(move |length| {
            let end = start.checked_add(length.bytes)?;
            let candidate = value.get(start..end)?;
            let entries = &self.entries[length.range.clone()];
            entries
                .binary_search_by(|entry| entry.as_str().cmp(candidate))
                .ok()
                .map(|index| entries[index].as_str())
        })
    }
}
