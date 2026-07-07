//! Corpus import with schema/provenance validation (rht Category B, Story
//! #282).
//!
//! Ports rht's `scripts/import-corpus.sh`: imports an existing corpus'
//! findings into a freshly instantiated harness, refusing anything that
//! fails MIF-backed schema validation or is missing a provenance block
//! (SPEC §10, §8a — provenance must survive the import).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::MifRhError;
use crate::harness_project::{read_json, validate_against_schema};

fn has_provenance(finding: &Value) -> bool {
    finding
        .pointer("/provenance/sourceType")
        .is_some_and(|v| !v.is_null())
}

/// The result of an [`import_corpus`] call.
#[derive(Debug, Clone)]
pub struct ImportReport {
    /// How many findings were imported.
    pub imported: usize,
    /// The topic's namespace (from the first imported finding, or
    /// `harness/<topic>` if none carry one).
    pub namespace: String,
    /// Whether the topic was newly registered in `harness.config.json`
    /// (`false` if it was already present, or no config path was given).
    pub topic_registered: bool,
}

/// Validates and imports every `*.json` finding directly under
/// `src_findings_dir` into `dest_dir`, then registers `topic` in
/// `config_path` (if given and not already present).
///
/// All findings are validated (schema + provenance) BEFORE any file is
/// copied — a rejection leaves `dest_dir` untouched, unlike a partial-copy
/// ordering that would otherwise leave invalid state behind an error.
///
/// # Errors
///
/// Returns [`MifRhError::NoFindingsFound`] if `src_findings_dir` has no
/// `*.json` files, [`MifRhError::SchemaValidationFailed`] if any finding
/// fails schema validation, and [`MifRhError::MissingProvenance`] if any
/// finding has no `provenance.sourceType`.
pub fn import_corpus(
    src_findings_dir: &Path,
    dest_dir: &Path,
    topic: &str,
    config_path: Option<&Path>,
    schema_path: &Path,
    ref_paths: &[PathBuf],
) -> Result<ImportReport, MifRhError> {
    let mut paths: Vec<_> = std::fs::read_dir(src_findings_dir)
        .map_err(|source| MifRhError::Io {
            path: src_findings_dir.display().to_string(),
            source,
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(MifRhError::NoFindingsFound {
            path: src_findings_dir.display().to_string(),
        });
    }

    let mut findings = Vec::with_capacity(paths.len());
    let mut missing_provenance = Vec::new();
    for path in &paths {
        let finding = read_json(path)?;
        validate_against_schema(&finding, path, schema_path, ref_paths)?;
        if !has_provenance(&finding) {
            missing_provenance.push(path.display().to_string());
        }
        findings.push(finding);
    }
    if !missing_provenance.is_empty() {
        return Err(MifRhError::MissingProvenance {
            count: missing_provenance.len(),
            paths: missing_provenance,
        });
    }

    std::fs::create_dir_all(dest_dir).map_err(|source| MifRhError::Io {
        path: dest_dir.display().to_string(),
        source,
    })?;
    for path in &paths {
        let dest = dest_dir.join(path.file_name().unwrap_or_default());
        std::fs::copy(path, &dest).map_err(|source| MifRhError::Io {
            path: dest.display().to_string(),
            source,
        })?;
    }

    let namespace = findings[0]
        .get("namespace")
        .and_then(Value::as_str)
        .map_or_else(|| format!("harness/{topic}"), str::to_string);

    let topic_registered = if let Some(config_path) = config_path {
        register_topic(config_path, topic, &namespace)?
    } else {
        false
    };

    Ok(ImportReport {
        imported: findings.len(),
        namespace,
        topic_registered,
    })
}

/// Registers `topic` in `config_path`'s `topics[]` array if not already
/// present. Returns `true` if newly registered, `false` if it was already
/// there or `config_path` does not exist.
fn register_topic(config_path: &Path, topic: &str, namespace: &str) -> Result<bool, MifRhError> {
    if !config_path.is_file() {
        return Ok(false);
    }
    let mut config = read_json(config_path)?;
    let already_registered = config
        .get("topics")
        .and_then(Value::as_array)
        .is_some_and(|topics| {
            topics
                .iter()
                .any(|t| t.get("id").and_then(Value::as_str) == Some(topic))
        });
    if already_registered {
        return Ok(false);
    }
    let topics = config
        .as_object_mut()
        .map(|object| {
            object
                .entry("topics")
                .or_insert_with(|| serde_json::json!([]))
        })
        .and_then(Value::as_array_mut)
        .ok_or_else(|| MifRhError::ConfigMalformed {
            path: config_path.display().to_string(),
            detail: ".topics is not an array".to_string(),
        })?;
    topics.push(serde_json::json!({
        "id": topic,
        "title": topic,
        "namespace": namespace,
        "status": "active",
    }));
    let text =
        serde_json::to_string_pretty(&config).map_err(|source| MifRhError::JsonSerialize {
            path: config_path.display().to_string(),
            source,
        })?;
    std::fs::write(config_path, format!("{text}\n")).map_err(|source| MifRhError::Io {
        path: config_path.display().to_string(),
        source,
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::import_corpus;
    use std::fs;

    const FINDINGS_SCHEMA: &str = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["@id", "provenance"],
        "properties": {
            "@id": {"type": "string"},
            "provenance": {"type": "object"}
        }
    }"#;

    fn write_finding(dir: &std::path::Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn imports_valid_findings_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/findings");
        fs::create_dir_all(&src).unwrap();
        write_finding(
            &src,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "namespace": "harness/x", "provenance": {"sourceType": "agent_inferred"}}"#,
        );
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let dest = dir.path().join("reports/topic/findings");

        let report = import_corpus(&src, &dest, "topic", None, &schema_path, &[]).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(report.namespace, "harness/x");
        assert!(dest.join("f1.json").is_file());
    }

    #[test]
    fn rejects_and_copies_nothing_when_a_finding_lacks_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/findings");
        fs::create_dir_all(&src).unwrap();
        write_finding(
            &src,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "provenance": {"sourceType": "x"}}"#,
        );
        write_finding(
            &src,
            "f2.json",
            r#"{"@id": "urn:mif:f2", "provenance": {}}"#,
        );
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let dest = dir.path().join("reports/topic/findings");

        let error = import_corpus(&src, &dest, "topic", None, &schema_path, &[]).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::MissingProvenance { count: 1, .. }
        ));
        // Transactional: nothing copied when any finding is rejected.
        assert!(!dest.exists() || fs::read_dir(&dest).unwrap().next().is_none());
    }

    #[test]
    fn rejects_a_finding_that_fails_schema_validation() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/findings");
        fs::create_dir_all(&src).unwrap();
        write_finding(&src, "f1.json", r#"{"provenance": {"sourceType": "x"}}"#);
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let dest = dir.path().join("reports/topic/findings");

        let error = import_corpus(&src, &dest, "topic", None, &schema_path, &[]).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::SchemaValidationFailed { .. }
        ));
    }

    #[test]
    fn registers_a_new_topic_in_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/findings");
        fs::create_dir_all(&src).unwrap();
        write_finding(
            &src,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "namespace": "harness/x", "provenance": {"sourceType": "y"}}"#,
        );
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(&config_path, r#"{"version": "1.0.0", "topics": []}"#).unwrap();
        let dest = dir.path().join("reports/topic/findings");

        let report = import_corpus(
            &src,
            &dest,
            "new-topic",
            Some(&config_path),
            &schema_path,
            &[],
        )
        .unwrap();
        assert!(report.topic_registered);
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(config["topics"][0]["id"], "new-topic");
        assert_eq!(config["topics"][0]["namespace"], "harness/x");
    }

    #[test]
    fn does_not_re_register_an_already_registered_topic() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src/findings");
        fs::create_dir_all(&src).unwrap();
        write_finding(
            &src,
            "f1.json",
            r#"{"@id": "urn:mif:f1", "provenance": {"sourceType": "y"}}"#,
        );
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(
            &config_path,
            r#"{"version": "1.0.0", "topics": [{"id": "existing", "title": "Existing", "namespace": "harness/existing", "status": "active"}]}"#,
        )
        .unwrap();
        let dest = dir.path().join("reports/topic/findings");

        let report = import_corpus(
            &src,
            &dest,
            "existing",
            Some(&config_path),
            &schema_path,
            &[],
        )
        .unwrap();
        assert!(!report.topic_registered);
        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(config["topics"].as_array().unwrap().len(), 1);
    }
}
