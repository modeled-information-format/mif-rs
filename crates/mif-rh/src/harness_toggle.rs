//! Harness manifest toggles (rht Category B, Story #302).
//!
//! Ports rht's `scripts/site-toggle.sh` and `scripts/pack-toggle.sh`: small,
//! atomic `harness.config.json` mutations. `pack-toggle.sh`'s
//! `sync-packs.sh` re-materialization step stays a separate call the bash
//! wrapper chains afterward.

use std::path::Path;

use serde_json::Value;

use crate::error::MifRhError;
use crate::harness_project::read_json;

fn write_json_pretty(path: &Path, value: &Value) -> Result<(), MifRhError> {
    let text = serde_json::to_string_pretty(value).map_err(|source| MifRhError::JsonSerialize {
        path: path.display().to_string(),
        source,
    })?;
    std::fs::write(path, format!("{text}\n")).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Sets `.site.primarySurface` in `config_path` to `value` (`"reports"`,
/// `"docs"`, or `"auto"`).
///
/// # Errors
///
/// Returns [`MifRhError::Io`]/[`MifRhError::Json`] if `config_path` cannot
/// be read, [`MifRhError::InvalidToggleValue`] if `value` is not one of the
/// three allowed values, and [`MifRhError::Io`]/[`MifRhError::JsonSerialize`]
/// if the write fails.
pub fn site_toggle_primary(config_path: &Path, value: &str) -> Result<(), MifRhError> {
    if !matches!(value, "reports" | "docs" | "auto") {
        return Err(MifRhError::InvalidToggleValue {
            field: "primarySurface".to_string(),
            value: value.to_string(),
            allowed: "reports|docs|auto".to_string(),
        });
    }
    let mut config = read_json(config_path)?;
    let site = config
        .as_object_mut()
        .map(|object| {
            object
                .entry("site")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
        })
        .and_then(Value::as_object_mut)
        .ok_or_else(|| MifRhError::ConfigMalformed {
            path: config_path.display().to_string(),
            detail: ".site is not an object".to_string(),
        })?;
    site.insert(
        "primarySurface".to_string(),
        Value::String(value.to_string()),
    );
    write_json_pretty(config_path, &config)
}

/// The site plugins `site_toggle_plugin` recognizes.
pub const SITE_PLUGINS: [&str; 4] = ["llmsTxt", "mermaid", "imageZoom", "linksValidator"];

/// Sets `.site.plugins.<name>` in `config_path` to `enabled`.
///
/// # Errors
///
/// Returns [`MifRhError::InvalidToggleValue`] if `name` is not one of
/// [`SITE_PLUGINS`], and [`MifRhError::Io`]/[`MifRhError::Json`]/
/// [`MifRhError::JsonSerialize`] for read/write failures.
pub fn site_toggle_plugin(config_path: &Path, name: &str, enabled: bool) -> Result<(), MifRhError> {
    if !SITE_PLUGINS.contains(&name) {
        return Err(MifRhError::InvalidToggleValue {
            field: "plugin".to_string(),
            value: name.to_string(),
            allowed: SITE_PLUGINS.join("|"),
        });
    }
    let mut config = read_json(config_path)?;
    let site = config
        .as_object_mut()
        .map(|object| {
            object
                .entry("site")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
        })
        .and_then(Value::as_object_mut)
        .ok_or_else(|| MifRhError::ConfigMalformed {
            path: config_path.display().to_string(),
            detail: ".site is not an object".to_string(),
        })?;
    let plugins = site
        .entry("plugins")
        .or_insert_with(|| Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| MifRhError::ConfigMalformed {
            path: config_path.display().to_string(),
            detail: ".site.plugins is not an object".to_string(),
        })?;
    plugins.insert(name.to_string(), Value::Bool(enabled));
    write_json_pretty(config_path, &config)
}

/// Sets `.packs[] | select(.name==pack).enabled` in `config_path`.
///
/// The pack must already be declared in `packs[]`; this only flips
/// `enabled`. Callers must run `sync-packs.sh` afterward to
/// re-materialize the enablement set (the original script's own next
/// step, kept as a separate call rather than folded in here).
///
/// # Errors
///
/// Returns [`MifRhError::PackNotDeclared`] if `pack` is not declared in
/// `config_path`'s `packs[]`, and [`MifRhError::Io`]/[`MifRhError::Json`]/
/// [`MifRhError::JsonSerialize`] for read/write failures.
pub fn pack_toggle(config_path: &Path, pack: &str, enabled: bool) -> Result<(), MifRhError> {
    let mut config = read_json(config_path)?;
    let packs = config
        .get_mut("packs")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| MifRhError::ConfigMalformed {
            path: config_path.display().to_string(),
            detail: ".packs is not an array".to_string(),
        })?;
    let entry = packs
        .iter_mut()
        .find(|pack_entry| pack_entry.get("name").and_then(Value::as_str) == Some(pack))
        .ok_or_else(|| MifRhError::PackNotDeclared {
            name: pack.to_string(),
            path: config_path.display().to_string(),
        })?;
    entry["enabled"] = Value::Bool(enabled);
    write_json_pretty(config_path, &config)
}

#[cfg(test)]
mod tests {
    use super::{pack_toggle, site_toggle_plugin, site_toggle_primary};
    use std::fs;

    fn write_config(dir: &std::path::Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("harness.config.json");
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn site_toggle_primary_sets_the_value() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_config(dir.path(), r#"{"version": "1.0.0"}"#);
        site_toggle_primary(&cfg, "docs").unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(after["site"]["primarySurface"], "docs");
    }

    #[test]
    fn site_toggle_primary_rejects_an_invalid_value() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_config(dir.path(), r#"{"version": "1.0.0"}"#);
        let error = site_toggle_primary(&cfg, "bogus").unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::InvalidToggleValue { .. }
        ));
    }

    #[test]
    fn site_toggle_plugin_sets_the_flag_without_disturbing_other_plugins() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_config(
            dir.path(),
            r#"{"version": "1.0.0", "site": {"plugins": {"mermaid": true}}}"#,
        );
        site_toggle_plugin(&cfg, "llmsTxt", true).unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(after["site"]["plugins"]["llmsTxt"], true);
        assert_eq!(after["site"]["plugins"]["mermaid"], true);
    }

    #[test]
    fn site_toggle_plugin_rejects_an_unknown_plugin_name() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_config(dir.path(), r#"{"version": "1.0.0"}"#);
        let error = site_toggle_plugin(&cfg, "bogus", true).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::InvalidToggleValue { .. }
        ));
    }

    #[test]
    fn pack_toggle_flips_the_named_packs_enabled_flag() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_config(
            dir.path(),
            r#"{"version": "1.0.0", "packs": [{"name": "pdf", "enabled": false}, {"name": "book", "enabled": true}]}"#,
        );
        pack_toggle(&cfg, "pdf", true).unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(after["packs"][0]["enabled"], true);
        assert_eq!(
            after["packs"][1]["enabled"], true,
            "unrelated pack must be untouched"
        );
    }

    #[test]
    fn pack_toggle_rejects_an_undeclared_pack() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = write_config(dir.path(), r#"{"version": "1.0.0", "packs": []}"#);
        let error = pack_toggle(&cfg, "nonexistent", true).unwrap_err();
        assert!(matches!(error, super::MifRhError::PackNotDeclared { .. }));
    }
}
