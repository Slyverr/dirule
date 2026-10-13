use crate::{AcronymLexicon, Captures, Error, Result, Transform, transform};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DestinationValues<'a> {
    pub path: &'a str,
    pub parent: &'a str,
    pub name: &'a str,
    pub stem: &'a str,
    pub extension: &'a str,
    pub captures: &'a Captures,
}

#[derive(Clone, Debug)]
pub struct CompiledTemplate {
    parts: Vec<TemplatePart>,
}

#[derive(Clone, Debug)]
enum TemplatePart {
    Literal(String),
    Value {
        key: TemplateValue,
        transform: Option<Transform>,
    },
}

#[derive(Clone, Debug)]
enum TemplateValue {
    Path,
    Parent,
    Name,
    Stem,
    Extension,
    Capture(String),
}

impl CompiledTemplate {
    pub fn parse(template: &str, capture_names: &[String]) -> Result<Self> {
        if template.is_empty() {
            return Err(Error::Template(
                "destination template cannot be empty".into(),
            ));
        }

        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch != '$' {
                literal.push(ch);
                continue;
            }

            match chars.next() {
                Some('$') => literal.push('$'),
                Some('{') => {
                    if !literal.is_empty() {
                        parts.push(TemplatePart::Literal(std::mem::take(&mut literal)));
                    }
                    let mut expression = String::new();
                    let mut closed = false;
                    for next in chars.by_ref() {
                        if next == '}' {
                            closed = true;
                            break;
                        }
                        expression.push(next);
                    }
                    if !closed {
                        return Err(Error::Template("unclosed placeholder".into()));
                    }
                    let part = parse_placeholder(&expression, capture_names)?;
                    parts.push(part);
                }
                _ => {
                    return Err(Error::Template(
                        "`$` must be escaped as `$$` or start a placeholder".into(),
                    ));
                }
            }
        }
        if !literal.is_empty() {
            parts.push(TemplatePart::Literal(literal));
        }

        validate_destination_path(&static_path_shape(&parts))?;
        Ok(Self { parts })
    }

    pub fn expand(
        &self,
        values: &DestinationValues<'_>,
        acronyms: &AcronymLexicon,
    ) -> Result<PathBuf> {
        let mut expanded = String::new();
        for part in &self.parts {
            match part {
                TemplatePart::Literal(value) => expanded.push_str(value),
                TemplatePart::Value { key, transform: op } => {
                    let (name, value, path_value) = resolve_value(key, values)?;
                    if !path_value && (value.contains('/') || value.contains('\\')) {
                        return Err(Error::Template(format!(
                            "single-component value `${name}` contains a path separator"
                        )));
                    }
                    if let Some(operation) = op {
                        expanded.push_str(&transform(value, *operation, acronyms));
                    } else {
                        expanded.push_str(value);
                    }
                }
            }
        }

        validate_destination_path(&expanded)
    }
}

pub fn validate_template(template: &str, capture_names: &[String]) -> Result<()> {
    CompiledTemplate::parse(template, capture_names).map(|_| ())
}

pub fn expand_template(
    template: &str,
    values: &DestinationValues<'_>,
    acronyms: &AcronymLexicon,
) -> Result<PathBuf> {
    let captures = values
        .captures
        .iter()
        .map(|(name, _)| name.to_owned())
        .collect::<Vec<_>>();
    CompiledTemplate::parse(template, &captures)?.expand(values, acronyms)
}

fn parse_placeholder(expression: &str, capture_names: &[String]) -> Result<TemplatePart> {
    if expression.is_empty() {
        return Err(Error::Template("placeholder cannot be empty".into()));
    }

    let mut parts = expression.split('|');
    let name = parts.next().unwrap_or_default();
    let operation = parts.next();
    if parts.next().is_some() {
        return Err(Error::Template(
            "a placeholder may have only one filter".into(),
        ));
    }

    let transform = operation
        .map(Transform::parse)
        .transpose()
        .map_err(|error| Error::Template(error.to_string()))?;
    let key = match name {
        "path" => TemplateValue::Path,
        "parent" => TemplateValue::Parent,
        "name" => TemplateValue::Name,
        "stem" => TemplateValue::Stem,
        "extension" => TemplateValue::Extension,
        capture if capture_names.iter().any(|known| known == capture) => {
            TemplateValue::Capture(capture.to_owned())
        }
        _ => {
            return Err(Error::Template(format!(
                "unknown value or missing capture `${name}`"
            )));
        }
    };
    Ok(TemplatePart::Value { key, transform })
}

fn resolve_value<'key, 'value>(
    key: &'key TemplateValue,
    values: &DestinationValues<'value>,
) -> Result<(&'key str, &'value str, bool)> {
    match key {
        TemplateValue::Path => Ok(("path", values.path, true)),
        TemplateValue::Parent => Ok(("parent", values.parent, true)),
        TemplateValue::Name => Ok(("name", values.name, false)),
        TemplateValue::Stem => Ok(("stem", values.stem, false)),
        TemplateValue::Extension => Ok(("extension", values.extension, false)),
        TemplateValue::Capture(name) => values
            .captures
            .get(name)
            .map(|value| (name.as_str(), value, false))
            .ok_or_else(|| Error::Template(format!("missing capture `${name}`"))),
    }
}

fn static_path_shape(parts: &[TemplatePart]) -> String {
    let mut shape = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(value) => shape.push_str(value),
            TemplatePart::Value { key, .. } => {
                if matches!(key, TemplateValue::Path | TemplateValue::Parent) {
                    shape.push_str("component/component");
                } else {
                    shape.push_str("component");
                }
            }
        }
    }
    shape
}

fn validate_destination_path(value: &str) -> Result<PathBuf> {
    let first = value.split('/').next().unwrap_or_default();
    let has_windows_drive_prefix =
        first.as_bytes().get(1) == Some(&b':') && first.as_bytes()[0].is_ascii_alphabetic();
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || has_windows_drive_prefix
        || path.is_absolute()
        || value.split('/').any(str::is_empty)
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::CurDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        return Err(Error::Template(
            "destination must be a safe target-relative path".into(),
        ));
    }
    Ok(path.to_path_buf())
}
