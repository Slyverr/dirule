use crate::paths;
use dirule_core::ConfigVersion;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use toml_edit::{DocumentMut, Item};

/// Current version of the user category-catalog format.
pub const CURRENT_CATEGORY_CATALOG_VERSION: ConfigVersion = ConfigVersion::new(1, 0);

#[derive(Clone, Debug, Eq, PartialEq)]
/// One file category and its extension-to-destination mapping.
pub struct SortCategory {
    /// Name accepted as a sort selector.
    pub name: String,
    /// Lowercase extensions assigned to this category.
    pub extensions: Vec<String>,
    /// Target-relative directory for matching files.
    pub destination: String,
}

#[derive(Clone, Debug)]
/// Built-in and user-defined categories available to sort operations.
pub struct CategoryCatalog {
    categories: BTreeMap<String, SortCategory>,
}

#[derive(Debug)]
/// A category catalog could not be read or did not match its schema.
pub enum CategoryCatalogError {
    Io { path: PathBuf, source: io::Error },
    Invalid { path: PathBuf, message: String },
}

impl CategoryCatalog {
    /// Return the built-in categories without reading user configuration.
    pub fn built_in() -> Self {
        Self {
            categories: built_in_categories()
                .into_iter()
                .map(|category| (category.name.clone(), category))
                .collect(),
        }
    }

    /// Load the user's category catalog when available, otherwise use built-ins.
    ///
    /// User-defined entries with a built-in name replace that built-in category.
    pub fn user_if_available() -> Result<Self, CategoryCatalogError> {
        let Some(base) = paths::user_config_directory() else {
            return Ok(Self::built_in());
        };

        Self::load_optional(&base.join("dirule").join("categories.toml"))
    }

    /// Load a catalog from `path`; a missing file selects built-in categories.
    pub fn load_optional(path: &Path) -> Result<Self, CategoryCatalogError> {
        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::built_in()),
            Err(source) => {
                return Err(CategoryCatalogError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        Self::parse(path, &contents)
    }

    /// Parse and validate a category catalog from TOML contents.
    pub fn parse(path: &Path, contents: &str) -> Result<Self, CategoryCatalogError> {
        let document = contents
            .parse::<DocumentMut>()
            .map_err(|error| invalid(path, format!("cannot parse category catalog: {error}")))?;

        validate_fields(path, &document, &["version", "categories"])?;

        let version = document
            .get("version")
            .and_then(Item::as_str)
            .ok_or_else(|| invalid(path, "`version` must be a string in `major.minor` form"))?;
        let version = ConfigVersion::from_str(version)
            .map_err(|message| invalid(path, format!("invalid `version`: {message}")))?;
        if version != CURRENT_CATEGORY_CATALOG_VERSION {
            return Err(invalid(
                path,
                format!(
                    "category catalog version {version} is unsupported; expected {CURRENT_CATEGORY_CATALOG_VERSION}"
                ),
            ));
        }

        let tables = document
            .get("categories")
            .and_then(Item::as_array_of_tables)
            .ok_or_else(|| invalid(path, "`categories` must be an array of tables"))?;
        let mut catalog = Self::built_in();
        let mut user_names = Vec::new();

        for (index, table) in tables.iter().enumerate() {
            validate_fields(path, table, &["name", "extensions", "destination"])?;

            let category = parse_category(path, index, table)?;
            if user_names.contains(&category.name) {
                return Err(invalid(
                    path,
                    format!("category {:?} is defined more than once", category.name),
                ));
            }

            user_names.push(category.name.clone());
            catalog.categories.insert(category.name.clone(), category);
        }

        Ok(catalog)
    }

    /// Find a category by its normalized lowercase name.
    pub fn get(&self, name: &str) -> Option<&SortCategory> {
        self.categories.get(name)
    }

    /// Iterate over categories in stable name order.
    pub fn iter(&self) -> impl Iterator<Item = &SortCategory> {
        self.categories.values()
    }
}

impl fmt::Display for CategoryCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(
                    formatter,
                    "cannot read category catalog {}: {source}",
                    path.display()
                )
            }
            Self::Invalid { path, message } => {
                write!(
                    formatter,
                    "invalid category catalog {}: {message}",
                    path.display()
                )
            }
        }
    }
}

impl Error for CategoryCatalogError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}

fn built_in_categories() -> Vec<SortCategory> {
    vec![
        category(
            "images",
            &[
                "avif", "bmp", "gif", "heic", "heif", "ico", "jpeg", "jpg", "png", "svg", "tif",
                "tiff", "webp",
            ],
            "media/images",
        ),
        category(
            "videos",
            &[
                "3gp", "avi", "flv", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "ts", "webm", "wmv",
            ],
            "media/videos",
        ),
        category(
            "audio",
            &[
                "aac", "aif", "aiff", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma",
            ],
            "media/audio",
        ),
        category(
            "documents",
            &[
                "csv", "doc", "docx", "md", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf",
                "tex", "txt", "xls", "xlsx",
            ],
            "documents",
        ),
    ]
}

fn category(name: &str, extensions: &[&str], destination: &str) -> SortCategory {
    SortCategory {
        name: name.into(),
        extensions: extensions
            .iter()
            .map(|extension| (*extension).into())
            .collect(),
        destination: destination.into(),
    }
}

fn parse_category(
    path: &Path,
    index: usize,
    table: &toml_edit::Table,
) -> Result<SortCategory, CategoryCatalogError> {
    let name = table.get("name").and_then(Item::as_str).ok_or_else(|| {
        invalid(
            path,
            format!("category {}: `name` must be a string", index + 1),
        )
    })?;
    if !valid_category_name(name) {
        return Err(invalid(
            path,
            format!(
                "category {}: `name` must start with a lowercase letter and contain only lowercase letters, digits, and hyphens",
                index + 1
            ),
        ));
    }

    let extension_values = table
        .get("extensions")
        .and_then(Item::as_array)
        .ok_or_else(|| {
            invalid(
                path,
                format!("category {name:?}: `extensions` must be an array of strings"),
            )
        })?;
    let mut extensions = Vec::with_capacity(extension_values.len());
    for value in extension_values.iter() {
        let extension = value.as_str().ok_or_else(|| {
            invalid(
                path,
                format!("category {name:?}: `extensions` must contain only strings"),
            )
        })?;
        let extension = extension.trim_start_matches('.').to_ascii_lowercase();
        if extension.is_empty() || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(invalid(
                path,
                format!("category {name:?}: invalid extension {extension:?}"),
            ));
        }
        if extensions.contains(&extension) {
            return Err(invalid(
                path,
                format!("category {name:?}: extension {extension:?} is repeated"),
            ));
        }

        extensions.push(extension);
    }
    if extensions.is_empty() {
        return Err(invalid(
            path,
            format!("category {name:?}: `extensions` cannot be empty"),
        ));
    }
    extensions.sort_unstable();

    let destination = table
        .get("destination")
        .and_then(Item::as_str)
        .ok_or_else(|| {
            invalid(
                path,
                format!("category {name:?}: `destination` must be a relative path"),
            )
        })?;
    validate_destination(path, name, destination)?;

    Ok(SortCategory {
        name: name.into(),
        extensions,
        destination: destination.into(),
    })
}

fn valid_category_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn validate_destination(
    path: &Path,
    name: &str,
    destination: &str,
) -> Result<(), CategoryCatalogError> {
    if destination.is_empty()
        || destination.starts_with('/')
        || destination.contains(['\\', ':', '$'])
        || destination
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(invalid(
            path,
            format!(
                "category {name:?}: `destination` must be a safe target-relative path using `/` separators"
            ),
        ));
    }

    Ok(())
}

fn validate_fields(
    path: &Path,
    table: &toml_edit::Table,
    allowed: &[&str],
) -> Result<(), CategoryCatalogError> {
    if let Some((field, _)) = table.iter().find(|(field, _)| !allowed.contains(field)) {
        return Err(invalid(path, format!("unknown field `{field}`")));
    }

    Ok(())
}

fn invalid(path: &Path, message: impl Into<String>) -> CategoryCatalogError {
    CategoryCatalogError::Invalid {
        path: path.to_path_buf(),
        message: message.into(),
    }
}
