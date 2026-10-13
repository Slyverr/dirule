use crate::{Error, Result};
use std::collections::BTreeMap;
use wax::{CandidatePath, Glob, Program};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Captures(BTreeMap<String, String>);

impl Captures {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

#[derive(Clone, Debug)]
pub struct Matcher {
    pattern: String,
    glob: Glob<'static>,
    capture_indexes: Vec<(String, usize)>,
}

impl Matcher {
    pub fn new(pattern: &str) -> Result<Self> {
        let (expression, capture_indexes) = compile_named_captures(pattern)?;
        let expression = case_sensitive_expression(&expression);
        let glob = Glob::new(&expression)
            .map_err(|error| Error::Pattern(format!("{pattern}: {error}")))?
            .into_owned();
        Ok(Self {
            pattern: pattern.to_owned(),
            glob,
            capture_indexes,
        })
    }

    pub fn captures(&self, path: &str) -> Option<Captures> {
        let candidate = CandidatePath::from(path);
        let matched = self.glob.matched(&candidate)?;
        let mut captures = BTreeMap::new();
        for (name, index) in &self.capture_indexes {
            let value = matched.get(*index)?;
            if value.is_empty() {
                return None;
            }
            captures.insert(name.clone(), value.to_owned());
        }
        Some(Captures(captures))
    }

    pub fn capture_names(&self) -> impl Iterator<Item = &str> {
        self.capture_indexes.iter().map(|(name, _)| name.as_str())
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }
}

fn compile_named_captures(pattern: &str) -> Result<(String, Vec<(String, usize)>)> {
    if pattern.starts_with('/') || pattern.split('/').any(|part| part == ".." || part == ".") {
        return Err(Error::Pattern("patterns must be target-relative".into()));
    }

    let chars: Vec<char> = pattern.chars().collect();
    let mut output = String::with_capacity(pattern.len());
    let mut captures = Vec::new();
    let mut capture_count = 0usize;
    let mut cursor = 0;
    while cursor < chars.len() {
        if chars[cursor] == '<' {
            let close = capture_close(&chars, cursor)?;
            let name: String = chars[cursor + 1..close].iter().collect();
            if !valid_identifier(&name) {
                return Err(Error::Pattern(format!("invalid capture name `{name}`")));
            }

            if captures.iter().any(|(existing, _)| existing == &name) {
                return Err(Error::Pattern(format!("duplicate capture `{name}`")));
            }
            if (cursor > 0 && matches!(chars[cursor - 1], '*' | '?' | '>'))
                || chars
                    .get(close + 1)
                    .is_some_and(|ch| matches!(ch, '*' | '?' | '<'))
            {
                return Err(Error::Pattern(format!(
                    "capture `{name}` has an ambiguous boundary"
                )));
            }
            output.push_str("{?$}");
            captures.push((name, capture_count + 1));
            capture_count += 1;
            cursor = close + 1;
            continue;
        }
        match chars[cursor] {
            '(' if chars.get(cursor + 1) == Some(&'?') => {
                return Err(Error::Pattern("matching flags are not supported".into()));
            }
            '\\' => {
                output.push(chars[cursor]);
                cursor += 1;
                let literal = chars
                    .get(cursor)
                    .ok_or_else(|| Error::Pattern("trailing escape".into()))?;
                output.push(*literal);
            }
            '*' if chars.get(cursor + 1) == Some(&'*') => {
                capture_count += 1;
                output.push_str("**");
                cursor += 1;
            }
            '*' | '?' => {
                capture_count += 1;
                output.push(chars[cursor]);
            }
            '[' => {
                let close = class_close(&chars, cursor)?;
                output.extend(chars[cursor..=close].iter().copied());
                cursor = close;
            }
            '{' => {
                let close = matching_brace(&chars, cursor)?;
                validate_alternation(&chars, cursor, close)?;
                capture_count += 1;
                output.extend(chars[cursor..=close].iter().copied());
                cursor = close;
            }
            '$' => output.push_str("\\$"),
            ch => output.push(ch),
        }
        cursor += 1;
    }
    Ok((output, captures))
}

fn case_sensitive_expression(expression: &str) -> String {
    if let Some(suffix) = expression.strip_prefix("**/") {
        format!("**/(?-i){suffix}")
    } else if expression == "**" {
        expression.to_owned()
    } else {
        format!("(?-i){expression}")
    }
}

fn validate_alternation(chars: &[char], start: usize, close: usize) -> Result<()> {
    let mut escaped = false;
    let mut in_class = false;
    let mut has_separator = false;
    for ch in &chars[start + 1..close] {
        if escaped {
            escaped = false;
        } else if *ch == '\\' {
            escaped = true;
        } else if *ch == '[' {
            in_class = true;
        } else if *ch == ']' {
            in_class = false;
        } else if !in_class && *ch == '{' {
            return Err(Error::Pattern(
                "nested alternatives are not supported".into(),
            ));
        } else if !in_class && *ch == '<' {
            return Err(Error::Pattern(
                "named captures inside alternatives are not supported".into(),
            ));
        } else if !in_class && *ch == ',' {
            has_separator = true;
        }
    }
    if !has_separator {
        return Err(Error::Pattern(
            "alternations need at least two comma-separated branches".into(),
        ));
    }
    Ok(())
}

fn matching_brace(chars: &[char], start: usize) -> Result<usize> {
    let mut depth = 0usize;
    let mut escaped = false;
    let mut in_class = false;
    for (offset, ch) in chars[start..].iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '{' if !in_class => depth += 1,
            '}' if !in_class => {
                depth -= 1;
                if depth == 0 {
                    return Ok(start + offset);
                }
            }
            _ => {}
        }
    }
    Err(Error::Pattern("unclosed alternation".into()))
}

fn class_close(chars: &[char], start: usize) -> Result<usize> {
    chars[start + 1..]
        .iter()
        .position(|ch| *ch == ']')
        .map(|offset| start + 1 + offset)
        .ok_or_else(|| Error::Pattern("unclosed character class".into()))
}

fn capture_close(chars: &[char], start: usize) -> Result<usize> {
    chars[start + 1..]
        .iter()
        .position(|ch| *ch == '>')
        .map(|offset| start + 1 + offset)
        .ok_or_else(|| Error::Pattern("unclosed named capture".into()))
}

fn valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
