use crate::{CURRENT_CONFIG_VERSION, ConfigVersion, Error, Result};

struct Migration {
    from: ConfigVersion,
    to: ConfigVersion,
    apply: fn(toml::Value) -> Result<toml::Value>,
}

const MIGRATIONS: &[Migration] = &[];

pub(super) fn upgrade(document: toml::Value, version: ConfigVersion) -> Result<toml::Value> {
    upgrade_through(document, version, CURRENT_CONFIG_VERSION, MIGRATIONS)
}

fn upgrade_through(
    mut document: toml::Value,
    mut version: ConfigVersion,
    target: ConfigVersion,
    migrations: &[Migration],
) -> Result<toml::Value> {
    if version > target {
        return Err(Error::Config(format!(
            "configuration version {version} is newer than the supported version {target}"
        )));
    }

    while version < target {
        let migration = migrations
            .iter()
            .find(|migration| migration.from == version)
            .ok_or_else(|| {
                Error::Config(format!(
                    "no configuration migration path from {version} to {target}"
                ))
            })?;
        if migration.to <= migration.from || migration.to > target {
            return Err(Error::Config(format!(
                "invalid migration step from {} to {}",
                migration.from, migration.to
            )));
        }
        document = (migration.apply)(document)?;
        version = migration.to;
        let document_version = super::version::read(&document)?;
        if document_version != version {
            return Err(Error::Config(format!(
                "migration produced version {document_version}, expected {version}"
            )));
        }
    }

    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::{Migration, upgrade_through};
    use crate::ConfigVersion;

    fn advance_version(mut document: toml::Value) -> crate::Result<toml::Value> {
        document["version"] = toml::Value::String("1.1".into());
        Ok(document)
    }

    fn advance_major(mut document: toml::Value) -> crate::Result<toml::Value> {
        document["version"] = toml::Value::String("2.0".into());
        Ok(document)
    }

    #[test]
    fn applies_each_registered_migration_in_order() {
        let migrations = [
            Migration {
                from: ConfigVersion::new(1, 0),
                to: ConfigVersion::new(1, 1),
                apply: advance_version,
            },
            Migration {
                from: ConfigVersion::new(1, 1),
                to: ConfigVersion::new(2, 0),
                apply: advance_major,
            },
        ];
        let document = toml::from_str("version = '1.0'").expect("valid document");
        let upgraded = upgrade_through(
            document,
            ConfigVersion::new(1, 0),
            ConfigVersion::new(2, 0),
            &migrations,
        )
        .expect("complete migration path");

        assert_eq!(upgraded["version"].as_str(), Some("2.0"));
    }

    #[test]
    fn rejects_a_missing_migration_step() {
        let document = toml::from_str("version = '1.0'").expect("valid document");
        let error = upgrade_through(
            document,
            ConfigVersion::new(1, 0),
            ConfigVersion::new(2, 0),
            &[],
        )
        .expect_err("missing migration path");

        assert!(
            error
                .to_string()
                .contains("no configuration migration path")
        );
    }
}
