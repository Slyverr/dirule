use crate::{CURRENT_CONFIG_VERSION, ConfigVersion, Error, Result};

struct Migration {
    from: ConfigVersion,
    to: ConfigVersion,
    apply: fn(toml::Value, ConfigVersion) -> Result<toml::Value>,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        from: ConfigVersion::new(1, 0),
        to: ConfigVersion::new(1, 1),
        apply: advance_version,
    },
    Migration {
        from: ConfigVersion::new(1, 1),
        to: ConfigVersion::new(1, 2),
        apply: advance_version,
    },
    Migration {
        from: ConfigVersion::new(1, 2),
        to: ConfigVersion::new(1, 3),
        apply: add_ignore_defaults,
    },
    Migration {
        from: ConfigVersion::new(1, 3),
        to: ConfigVersion::new(1, 4),
        apply: advance_check_mode_version,
    },
];

fn advance_version(mut document: toml::Value, version: ConfigVersion) -> Result<toml::Value> {
    let document_table = document
        .as_table_mut()
        .ok_or_else(|| Error::Config("configuration document must be a table".into()))?;
    document_table.insert("version".into(), toml::Value::String(version.to_string()));

    Ok(document)
}

fn add_ignore_defaults(mut document: toml::Value, version: ConfigVersion) -> Result<toml::Value> {
    let document_table = document
        .as_table_mut()
        .ok_or_else(|| Error::Config("configuration document must be a table".into()))?;
    if document_table.contains_key("ignore") {
        return Err(Error::Config(format!(
            "`ignore` requires configuration version {version}"
        )));
    }

    document_table.insert("ignore".into(), toml::Value::Array(Vec::new()));
    document_table.insert("version".into(), toml::Value::String(version.to_string()));

    Ok(document)
}

fn advance_check_mode_version(
    document: toml::Value,
    version: ConfigVersion,
) -> Result<toml::Value> {
    let has_rule_mode = document
        .get("rules")
        .and_then(toml::Value::as_array)
        .is_some_and(|rules| {
            rules.iter().any(|rule| {
                rule.as_table()
                    .is_some_and(|rule| rule.contains_key("mode"))
            })
        });
    if has_rule_mode {
        return Err(Error::Config(format!(
            "rule `mode` requires configuration version {version}"
        )));
    }

    advance_version(document, version)
}

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
        let mut candidates = migrations
            .iter()
            .filter(|migration| migration.from == version);
        let migration = candidates.next().ok_or_else(|| {
            Error::Config(format!(
                "no configuration migration path from {version} to {target}"
            ))
        })?;
        if candidates.next().is_some() {
            return Err(Error::Config(format!(
                "multiple configuration migrations start at version {version}"
            )));
        }
        if !migration.to.immediately_follows(migration.from) {
            return Err(Error::Config(format!(
                "migration step from {} to {} must advance exactly one version",
                migration.from, migration.to
            )));
        }
        if migration.to > target {
            return Err(Error::Config(format!(
                "migration step from {} to {} exceeds target version {target}",
                migration.from, migration.to
            )));
        }
        document = (migration.apply)(document, migration.to)?;
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

    fn advance_version(
        mut document: toml::Value,
        version: ConfigVersion,
    ) -> crate::Result<toml::Value> {
        document["version"] = toml::Value::String(version.to_string());
        Ok(document)
    }

    fn advance_major(
        mut document: toml::Value,
        version: ConfigVersion,
    ) -> crate::Result<toml::Value> {
        document["version"] = toml::Value::String(version.to_string());
        Ok(document)
    }

    fn jump_minor(mut document: toml::Value, version: ConfigVersion) -> crate::Result<toml::Value> {
        document["version"] = toml::Value::String(version.to_string());
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
        let document =
            toml::from_str("version = '1.0'\nname = 'Directory rules'\n[options]\nenabled = true")
                .expect("valid document");
        let upgraded = upgrade_through(
            document,
            ConfigVersion::new(1, 0),
            ConfigVersion::new(2, 0),
            &migrations,
        )
        .expect("complete migration path");

        assert_eq!(upgraded["version"].as_str(), Some("2.0"));
        assert_eq!(upgraded["name"].as_str(), Some("Directory rules"));
        assert_eq!(upgraded["options"]["enabled"].as_bool(), Some(true));
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

    #[test]
    fn rejects_a_migration_that_skips_a_version() {
        let migrations = [Migration {
            from: ConfigVersion::new(1, 0),
            to: ConfigVersion::new(1, 2),
            apply: jump_minor,
        }];
        let document = toml::from_str("version = '1.0'").expect("valid document");
        let error = upgrade_through(
            document,
            ConfigVersion::new(1, 0),
            ConfigVersion::new(1, 2),
            &migrations,
        )
        .expect_err("migration must not skip a format version");

        assert!(
            error
                .to_string()
                .contains("must advance exactly one version")
        );
    }

    #[test]
    fn rejects_ambiguous_migration_routes() {
        let migrations = [
            Migration {
                from: ConfigVersion::new(1, 0),
                to: ConfigVersion::new(1, 1),
                apply: advance_version,
            },
            Migration {
                from: ConfigVersion::new(1, 0),
                to: ConfigVersion::new(1, 1),
                apply: advance_version,
            },
        ];
        let document = toml::from_str("version = '1.0'").expect("valid document");
        let error = upgrade_through(
            document,
            ConfigVersion::new(1, 0),
            ConfigVersion::new(1, 1),
            &migrations,
        )
        .expect_err("reject ambiguous migration registry");

        assert!(
            error
                .to_string()
                .contains("multiple configuration migrations")
        );
    }
}
