use std::fmt;

use serde_json::Value;

use super::CURRENT_JOURNAL_VERSION;

struct Migration {
    from: JournalVersion,
    to: JournalVersion,
    apply: fn(Vec<Value>) -> Result<Vec<Value>, String>,
}

const MIGRATIONS: &[Migration] = &[];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct JournalVersion {
    major: u32,
    minor: u32,
}

impl JournalVersion {
    #[cfg(test)]
    const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    fn parse(value: &str) -> Result<Self, String> {
        let Some((major, minor)) = value.split_once('.') else {
            return Err("expected a version string in `major.minor` form".into());
        };
        if minor.contains('.') {
            return Err("expected exactly two numeric version components".into());
        }

        Ok(Self {
            major: parse_component(major, "major")?,
            minor: parse_component(minor, "minor")?,
        })
    }

    fn immediately_follows(self, previous: Self) -> bool {
        previous
            .minor
            .checked_add(1)
            .is_some_and(|minor| self.major == previous.major && self.minor == minor)
            || previous
                .major
                .checked_add(1)
                .is_some_and(|major| self.major == major && self.minor == 0)
    }
}

impl fmt::Display for JournalVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

fn parse_component(value: &str, name: &str) -> Result<u32, String> {
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

pub(super) fn upgrade(
    path: &std::path::Path,
    records: Vec<Value>,
    version: &str,
) -> Result<Vec<Value>, String> {
    let from = JournalVersion::parse(version)
        .map_err(|error| format!("journal {} has invalid version: {error}", path.display()))?;
    let target = JournalVersion::parse(CURRENT_JOURNAL_VERSION)
        .map_err(|error| format!("invalid current journal version: {error}"))?;

    if from > target {
        return Err(format!(
            "journal {} uses newer version {from}; this app supports up to {target}",
            path.display()
        ));
    }

    upgrade_through(records, from, target, path, MIGRATIONS)
}

fn upgrade_through(
    mut records: Vec<Value>,
    mut version: JournalVersion,
    target: JournalVersion,
    path: &std::path::Path,
    migrations: &[Migration],
) -> Result<Vec<Value>, String> {
    while version < target {
        let mut candidates = migrations
            .iter()
            .filter(|migration| migration.from == version);
        let migration = candidates.next().ok_or_else(|| {
            format!(
                "journal {} has no journal migration path from {version} to {target}",
                path.display()
            )
        })?;
        if candidates.next().is_some() {
            return Err(format!(
                "journal {} has multiple journal migrations from version {version}",
                path.display()
            ));
        }
        if !migration.to.immediately_follows(migration.from) {
            return Err(format!(
                "journal migration from {} to {} must advance exactly one version",
                migration.from, migration.to
            ));
        }
        if migration.to > target {
            return Err(format!(
                "journal migration to {} exceeds supported version {target}",
                migration.to
            ));
        }

        records = (migration.apply)(records)?;
        version = migration.to;
        let header_version = read_version(&records)?;
        if header_version != version.to_string() {
            return Err(format!(
                "journal migration produced version {header_version}, expected {version}"
            ));
        }
    }

    Ok(records)
}

pub(super) fn read_version(records: &[Value]) -> Result<String, String> {
    records
        .first()
        .and_then(|record| record.get("version"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "journal header must contain a string `version`".into())
}

#[cfg(test)]
mod tests {
    use super::{JournalVersion, Migration, read_version, upgrade_through};
    use serde_json::{Value, json};

    fn advance_minor(mut records: Vec<Value>) -> Result<Vec<Value>, String> {
        records[0]["version"] = json!("1.1");
        Ok(records)
    }

    fn advance_major(mut records: Vec<Value>) -> Result<Vec<Value>, String> {
        records[0]["version"] = json!("2.0");
        Ok(records)
    }

    fn jump_minor(mut records: Vec<Value>) -> Result<Vec<Value>, String> {
        records[0]["version"] = json!("1.2");
        Ok(records)
    }

    fn keep_version(records: Vec<Value>) -> Result<Vec<Value>, String> {
        Ok(records)
    }

    fn header(version: &str) -> Vec<Value> {
        vec![json!({"record": "header", "version": version, "id": "run"})]
    }

    #[test]
    fn applies_each_registered_migration_in_order() {
        let migrations = [
            Migration {
                from: JournalVersion::new(1, 0),
                to: JournalVersion::new(1, 1),
                apply: advance_minor,
            },
            Migration {
                from: JournalVersion::new(1, 1),
                to: JournalVersion::new(2, 0),
                apply: advance_major,
            },
        ];
        let records = upgrade_through(
            header("1.0"),
            JournalVersion::new(1, 0),
            JournalVersion::new(2, 0),
            std::path::Path::new("journal.jsonl"),
            &migrations,
        )
        .expect("complete migration path");

        assert_eq!(read_version(&records).as_deref(), Ok("2.0"));
    }

    #[test]
    fn rejects_a_missing_migration_step() {
        let error = upgrade_through(
            header("1.0"),
            JournalVersion::new(1, 0),
            JournalVersion::new(2, 0),
            std::path::Path::new("journal.jsonl"),
            &[],
        )
        .expect_err("missing migration path");

        assert!(error.contains("no journal migration path"));
    }

    #[test]
    fn rejects_migrations_that_skip_a_version() {
        let migrations = [Migration {
            from: JournalVersion::new(1, 0),
            to: JournalVersion::new(1, 2),
            apply: jump_minor,
        }];
        let error = upgrade_through(
            header("1.0"),
            JournalVersion::new(1, 0),
            JournalVersion::new(1, 2),
            std::path::Path::new("journal.jsonl"),
            &migrations,
        )
        .expect_err("reject skipped version");

        assert!(error.contains("must advance exactly one version"));
    }

    #[test]
    fn rejects_ambiguous_migrations() {
        let migrations = [
            Migration {
                from: JournalVersion::new(1, 0),
                to: JournalVersion::new(1, 1),
                apply: advance_minor,
            },
            Migration {
                from: JournalVersion::new(1, 0),
                to: JournalVersion::new(1, 1),
                apply: advance_minor,
            },
        ];
        let error = upgrade_through(
            header("1.0"),
            JournalVersion::new(1, 0),
            JournalVersion::new(1, 1),
            std::path::Path::new("journal.jsonl"),
            &migrations,
        )
        .expect_err("reject ambiguous migrations");

        assert!(error.contains("multiple journal migrations"));
    }

    #[test]
    fn rejects_a_migration_that_does_not_update_the_header_version() {
        let migrations = [Migration {
            from: JournalVersion::new(1, 0),
            to: JournalVersion::new(1, 1),
            apply: keep_version,
        }];
        let error = upgrade_through(
            header("1.0"),
            JournalVersion::new(1, 0),
            JournalVersion::new(1, 1),
            std::path::Path::new("journal.jsonl"),
            &migrations,
        )
        .expect_err("migration must update the header version");

        assert!(error.contains("produced version 1.0, expected 1.1"));
    }
}
