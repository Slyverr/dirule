use crate::{AcronymLexicon, Error, Result};
use std::ops::Range;
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
    let tokens = tokenize(value, acronyms);
    let mut output = String::with_capacity(value.len());
    render_tokens(value, &tokens, operation, &mut output);

    output
}

pub fn transform_filename(value: &str, operation: Transform, acronyms: &AcronymLexicon) -> String {
    let (hidden, value) = value
        .strip_prefix('.')
        .map_or((false, value), |v| (true, v));
    let (stem, extension) = split_extension(value);
    let tokens = tokenize(stem, acronyms);
    let mut output = String::with_capacity(value.len());
    if hidden {
        output.push('.');
    }
    render_tokens(stem, &tokens, operation, &mut output);
    if let Some(extension) = extension {
        output.push('.');
        output.push_str(extension);
    }
    output
}

pub fn transform_directory(value: &str, operation: Transform, acronyms: &AcronymLexicon) -> String {
    let (hidden, value) = value
        .strip_prefix('.')
        .map_or((false, value), |value| (true, value));
    let tokens = tokenize(value, acronyms);
    let mut output = String::with_capacity(value.len());
    if hidden {
        output.push('.');
    }
    render_tokens(value, &tokens, operation, &mut output);

    output
}

fn render_tokens(value: &str, tokens: &[Range<usize>], operation: Transform, output: &mut String) {
    let separator = match operation {
        Transform::SnakeCase => Some('_'),
        Transform::KebabCase => Some('-'),
        Transform::CamelCase | Transform::PascalCase => None,
    };

    for (index, range) in tokens.iter().enumerate() {
        if index > 0
            && let Some(separator) = separator
        {
            output.push(separator);
        }

        let token = &value[range.clone()];
        match operation {
            Transform::SnakeCase | Transform::KebabCase => lowercase_into(token, output),
            Transform::CamelCase if index == 0 => lowercase_into(token, output),
            Transform::CamelCase | Transform::PascalCase => title_case_into(token, output),
        }
    }
}

fn split_extension(value: &str) -> (&str, Option<&str>) {
    match value.rfind('.') {
        Some(index) if index > 0 => (&value[..index], Some(&value[index + 1..])),
        _ => (value, None),
    }
}

fn tokenize(value: &str, acronyms: &AcronymLexicon) -> Vec<Range<usize>> {
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
            let byte_start = graphemes[index].0;
            result.push(byte_start..byte_start + acronym.len());
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
    value: &'a str,
    graphemes: &[(usize, &str)],
    index: usize,
    acronyms: &'a AcronymLexicon,
) -> Option<(&'a str, usize)> {
    if !is_token_boundary(graphemes, index) {
        return None;
    }

    let start = graphemes[index].0;

    acronyms
        .candidates_starting_at(first_char(graphemes[index].1), value, start)
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

fn tokenize_generic(graphemes: &[(usize, &str)]) -> Vec<Range<usize>> {
    let mut tokens = Vec::new();
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
            push_token_range(graphemes, start, index, &mut tokens);
            start = index;
        }
    }
    push_token_range(graphemes, start, graphemes.len(), &mut tokens);

    tokens
}

fn push_token_range(
    graphemes: &[(usize, &str)],
    start: usize,
    end: usize,
    tokens: &mut Vec<Range<usize>>,
) {
    if start >= end {
        return;
    }

    let word = &graphemes[start..end];
    if word
        .iter()
        .any(|(_, cluster)| first_char(cluster).is_alphanumeric())
    {
        let (last_offset, last_cluster) = word.last().expect("token range is not empty");
        tokens.push(graphemes[start].0..last_offset + last_cluster.len());
    }
}

fn first_char(grapheme: &str) -> char {
    grapheme
        .chars()
        .next()
        .expect("grapheme clusters are never empty")
}

fn lowercase_into(value: &str, output: &mut String) {
    for character in value.chars() {
        output.extend(character.to_lowercase());
    }
}

fn title_case_into(value: &str, output: &mut String) {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return;
    };

    output.extend(first.to_uppercase());
    for character in chars {
        output.extend(character.to_lowercase());
    }
}
