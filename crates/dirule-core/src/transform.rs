use crate::{AcronymLexicon, Error, Result};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transform {
    SnakeCase,
    KebabCase,
    CamelCase,
    PascalCase,
}

impl Transform {
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "snake-case" => Ok(Self::SnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "camel-case" => Ok(Self::CamelCase),
            "pascal-case" => Ok(Self::PascalCase),
            other => Err(Error::Config(format!("unknown transform `{other}`"))),
        }
    }
}

pub fn transform(value: &str, operation: Transform, acronyms: &AcronymLexicon) -> String {
    render_tokens(tokenize(value, acronyms), operation)
}

pub fn transform_filename(value: &str, operation: Transform, acronyms: &AcronymLexicon) -> String {
    let (hidden, value) = value
        .strip_prefix('.')
        .map_or((false, value), |v| (true, v));
    let (stem, extension) = split_extension(value);
    let mut output = String::new();
    if hidden {
        output.push('.');
    }
    output.push_str(&render_tokens(tokenize(stem, acronyms), operation));
    if let Some(extension) = extension {
        output.push('.');
        output.push_str(extension);
    }
    output
}

fn render_tokens(tokens: Vec<String>, operation: Transform) -> String {
    match operation {
        Transform::SnakeCase => tokens
            .iter()
            .map(|word| word.to_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
        Transform::KebabCase => tokens
            .iter()
            .map(|word| word.to_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
        Transform::CamelCase => tokens
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == 0 {
                    word.to_lowercase()
                } else {
                    title_case(word)
                }
            })
            .collect::<String>(),
        Transform::PascalCase => tokens.iter().map(|word| title_case(word)).collect(),
    }
}

fn split_extension(value: &str) -> (&str, Option<&str>) {
    match value.rfind('.') {
        Some(index) if index > 0 => (&value[..index], Some(&value[index + 1..])),
        _ => (value, None),
    }
}

fn tokenize(value: &str, acronyms: &AcronymLexicon) -> Vec<String> {
    let graphemes: Vec<(usize, &str)> = value.grapheme_indices(true).collect();
    if acronyms.is_empty() {
        return tokenize_generic(&graphemes);
    }

    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < graphemes.len() {
        if let Some((acronym, length)) = acronym_at(value, &graphemes, index, acronyms) {
            result.extend(tokenize_generic(&graphemes[start..index]));
            result.push(acronym.to_owned());
            index += length;
            start = index;
        } else {
            index += 1;
        }
    }
    result.extend(tokenize_generic(&graphemes[start..]));
    result
}

fn acronym_at<'a>(
    value: &str,
    graphemes: &[(usize, &str)],
    index: usize,
    acronyms: &'a AcronymLexicon,
) -> Option<(&'a str, usize)> {
    if !is_token_boundary(graphemes, index) {
        return None;
    }

    let start = graphemes[index].0;

    acronyms
        .entries_starting_with(first_char(graphemes[index].1))
        .find_map(|acronym| {
            let end = start + acronym.len();
            if value.get(start..end) != Some(acronym) {
                return None;
            }

            let end_index = if end == value.len() {
                graphemes.len()
            } else {
                graphemes
                    .binary_search_by_key(&end, |(offset, _)| *offset)
                    .ok()?
            };
            let next = graphemes
                .get(end_index)
                .map(|(_, cluster)| first_char(cluster));
            if next.is_some_and(|ch| ch.is_alphabetic() && !ch.is_uppercase()) {
                return None;
            }

            let length = end_index - index;
            Some((acronym, length))
        })
}

fn is_token_boundary(graphemes: &[(usize, &str)], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = first_char(graphemes[index - 1].1);
    let current = first_char(graphemes[index].1);
    let next = graphemes
        .get(index + 1)
        .map(|(_, cluster)| first_char(cluster));

    !previous.is_alphanumeric()
        || (previous.is_lowercase() && current.is_uppercase())
        || (previous.is_alphabetic() && current.is_numeric())
        || (previous.is_numeric() && current.is_alphabetic())
        || (previous.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(char::is_lowercase))
}

fn tokenize_generic(graphemes: &[(usize, &str)]) -> Vec<String> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for index in 1..graphemes.len() {
        let previous = first_char(graphemes[index - 1].1);
        let current = first_char(graphemes[index].1);
        let next = graphemes
            .get(index + 1)
            .map(|(_, cluster)| first_char(cluster));
        let boundary = (!previous.is_alphanumeric() || !current.is_alphanumeric())
            || (previous.is_lowercase() && current.is_uppercase())
            || (previous.is_alphabetic() && current.is_numeric())
            || (previous.is_numeric() && current.is_alphabetic())
            || (previous.is_uppercase()
                && current.is_uppercase()
                && next.is_some_and(char::is_lowercase));
        if boundary {
            let word = &graphemes[start..index];
            if word
                .iter()
                .any(|(_, cluster)| first_char(cluster).is_alphanumeric())
            {
                ranges.push(word.iter().map(|(_, cluster)| *cluster).collect());
            }

            start = index;
        }
    }
    let word = &graphemes[start..];
    if word
        .iter()
        .any(|(_, cluster)| first_char(cluster).is_alphanumeric())
    {
        ranges.push(word.iter().map(|(_, cluster)| *cluster).collect());
    }

    ranges
}

fn first_char(grapheme: &str) -> char {
    grapheme
        .chars()
        .next()
        .expect("grapheme clusters are never empty")
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect()
    })
}
