use crate::{AcronymLexicon, Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transform {
    SnakeCase,
    KebabCase,
    CamelCase,
}

impl Transform {
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "snake-case" => Ok(Self::SnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "camel-case" => Ok(Self::CamelCase),
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
    }
}

fn split_extension(value: &str) -> (&str, Option<&str>) {
    match value.rfind('.') {
        Some(index) if index > 0 => (&value[..index], Some(&value[index + 1..])),
        _ => (value, None),
    }
}

fn tokenize(value: &str, acronyms: &AcronymLexicon) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < chars.len() {
        if let Some((acronym, length)) = acronym_at(&chars, index, acronyms) {
            result.extend(tokenize_generic(&chars[start..index]));
            result.push(acronym.to_owned());
            index += length;
            start = index;
        } else {
            index += 1;
        }
    }
    result.extend(tokenize_generic(&chars[start..]));
    result
}

fn acronym_at<'a>(
    chars: &[char],
    index: usize,
    acronyms: &'a AcronymLexicon,
) -> Option<(&'a str, usize)> {
    if !is_token_boundary(chars, index) {
        return None;
    }

    acronyms.entries().find_map(|acronym| {
        let mut end = index;
        let mut length = 0;
        for expected in acronym.chars() {
            if chars.get(end).copied() != Some(expected) {
                return None;
            }
            end += 1;
            length += 1;
        }
        let next = chars.get(end).copied();
        if next.is_some_and(|ch| ch.is_alphabetic() && !ch.is_uppercase()) {
            return None;
        }
        Some((acronym, length))
    })
}

fn is_token_boundary(chars: &[char], index: usize) -> bool {
    if index == 0 {
        return true;
    }
    let previous = chars[index - 1];
    let current = chars[index];
    let next = chars.get(index + 1).copied();
    !previous.is_alphanumeric()
        || (previous.is_lowercase() && current.is_uppercase())
        || (previous.is_alphabetic() && current.is_numeric())
        || (previous.is_numeric() && current.is_alphabetic())
        || (previous.is_uppercase()
            && current.is_uppercase()
            && next.is_some_and(char::is_lowercase))
}

fn tokenize_generic(chars: &[char]) -> Vec<String> {
    let mut ranges = Vec::new();
    let mut start = 0;
    for index in 1..chars.len() {
        let previous = chars[index - 1];
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        let boundary = (!previous.is_alphanumeric() || !current.is_alphanumeric())
            || (previous.is_lowercase() && current.is_uppercase())
            || (previous.is_alphabetic() && current.is_numeric())
            || (previous.is_numeric() && current.is_alphabetic())
            || (previous.is_uppercase()
                && current.is_uppercase()
                && next.is_some_and(char::is_lowercase));
        if boundary {
            if start < index && chars[start..index].iter().any(|ch| ch.is_alphanumeric()) {
                ranges.push(chars[start..index].iter().collect::<String>());
            }
            start = index;
        }
    }
    if start < chars.len() && chars[start..].iter().any(|ch| ch.is_alphanumeric()) {
        ranges.push(chars[start..].iter().collect());
    }
    ranges
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
