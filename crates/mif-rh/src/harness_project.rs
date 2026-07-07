//! MIF Level-3 report projection/validation (rht Category B, Story #298).
//!
//! Ports rht's `scripts/mif-project.sh`: splits a report markdown file's
//! YAML frontmatter from its body, folds the body in as `content` (unless
//! the frontmatter already sets it), and validates the resulting JSON
//! against an arbitrary caller-supplied schema (with `$ref` dependencies)
//! at runtime. Citation-integrity (`scripts/check-citation-integrity.sh`,
//! [`crate::harness_citation_integrity`], Story #287) is a separate
//! concern — this function covers only the projection + schema-validation
//! half of the original script; the bash wrapper chains the
//! citation-integrity check afterward.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::MifRhError;

/// Projects `md_path`'s frontmatter + body into a flat JSON document and
/// validates it against `schema_path` (resolving `$ref`s against
/// `ref_paths`, each of which must declare its own `$id`).
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if any file cannot be read,
/// [`MifRhError::Frontmatter`] if the frontmatter cannot be split/parsed,
/// [`MifRhError::Json`] if a schema file is not valid JSON,
/// [`MifRhError::RefSchemaMissingId`] if a `$ref` dependency has no `$id`,
/// [`MifRhError::SchemaCompilation`] if the validator cannot be built, and
/// [`MifRhError::SchemaValidationFailed`] if the projection does not
/// conform.
pub fn project_report(
    md_path: &Path,
    schema_path: &Path,
    ref_paths: &[PathBuf],
) -> Result<Value, MifRhError> {
    let contents = read_text(md_path)?;
    let (frontmatter, body) = mif_frontmatter::parse_markdown(&contents)?;
    let mut doc: Value =
        serde_json::to_value(&frontmatter).map_err(|source| MifRhError::JsonSerialize {
            path: md_path.display().to_string(),
            source,
        })?;

    let content_is_empty = doc
        .get("content")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty);
    if content_is_empty {
        let body_trimmed = body.trim();
        let fallback = if body_trimmed.is_empty() {
            doc.get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        } else {
            body_trimmed.to_string()
        };
        doc["content"] = Value::String(fallback);
    }

    validate_against_schema(&doc, md_path, schema_path, ref_paths)?;
    Ok(doc)
}

pub(crate) fn read_text(path: &Path) -> Result<String, MifRhError> {
    std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

pub(crate) fn read_json(path: &Path) -> Result<Value, MifRhError> {
    let contents = read_text(path)?;
    serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
        path: path.display().to_string(),
        source,
    })
}

/// Validates `instance` against `schema_path`, resolving `$ref`s against
/// `ref_paths` (each of which must declare its own `$id`). Shared by
/// [`crate::harness_project::project_report`] and
/// `crate::harness_wrap::wrap_source`, which both validate an
/// assembled JSON document against a MIF schema at runtime.
pub(crate) fn validate_against_schema(
    instance: &Value,
    instance_path: &Path,
    schema_path: &Path,
    ref_paths: &[PathBuf],
) -> Result<(), MifRhError> {
    let schema = read_json(schema_path)?;

    let mut registry_builder = jsonschema::Registry::new();
    for ref_path in ref_paths {
        let ref_schema = read_json(ref_path)?;
        let id = ref_schema
            .get("$id")
            .and_then(Value::as_str)
            .ok_or_else(|| MifRhError::RefSchemaMissingId {
                path: ref_path.display().to_string(),
            })?
            .to_string();
        registry_builder = registry_builder.add(id, ref_schema).map_err(|source| {
            MifRhError::SchemaCompilation {
                path: ref_path.display().to_string(),
                detail: source.to_string(),
            }
        })?;
    }
    let registry = registry_builder
        .prepare()
        .map_err(|source| MifRhError::SchemaCompilation {
            path: schema_path.display().to_string(),
            detail: source.to_string(),
        })?;
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .map_err(|source| MifRhError::SchemaCompilation {
            path: schema_path.display().to_string(),
            detail: source.to_string(),
        })?;

    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(MifRhError::SchemaValidationFailed {
            path: instance_path.display().to_string(),
            schema_path: schema_path.display().to_string(),
            detail: errors.join("; "),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::project_report;
    use std::fs;

    const FINDINGS_SCHEMA: &str = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["id", "content"],
        "properties": {
            "id": {"type": "string"},
            "content": {"type": "string", "minLength": 1},
            "title": {"type": "string"}
        }
    }"#;

    #[test]
    fn folds_the_body_into_content_when_frontmatter_omits_it() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        fs::write(
            &md_path,
            "---\nid: r-1\ntitle: A report\n---\n\nThe body text.\n",
        )
        .unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();

        let projected = project_report(&md_path, &schema_path, &[]).unwrap();
        assert_eq!(projected["content"], "The body text.");
    }

    #[test]
    fn keeps_an_explicit_content_field_over_the_body() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        fs::write(
            &md_path,
            "---\nid: r-1\ncontent: explicit content\n---\n\nignored body.\n",
        )
        .unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();

        let projected = project_report(&md_path, &schema_path, &[]).unwrap();
        assert_eq!(projected["content"], "explicit content");
    }

    #[test]
    fn falls_back_to_title_when_both_content_and_body_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        fs::write(
            &md_path,
            "---\nid: r-1\ntitle: Fallback title\n---\n\n   \n",
        )
        .unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();

        let projected = project_report(&md_path, &schema_path, &[]).unwrap();
        assert_eq!(projected["content"], "Fallback title");
    }

    #[test]
    fn rejects_a_projection_that_fails_schema_validation() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        // No `id` field: violates the schema's `required`.
        fs::write(&md_path, "---\ntitle: Missing id\n---\n\nbody\n").unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();

        let error = project_report(&md_path, &schema_path, &[]).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::SchemaValidationFailed { .. }
        ));
    }

    #[test]
    fn rejects_a_report_with_no_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        fs::write(&md_path, "just a body, no frontmatter\n").unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();

        let error = project_report(&md_path, &schema_path, &[]).unwrap_err();
        assert!(matches!(error, super::MifRhError::Frontmatter(_)));
    }

    #[test]
    fn resolves_a_ref_dependency_schema_by_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        fs::write(
            &md_path,
            "---\nid: r-1\ncontent: has entity\nentity: {name: widget}\n---\n\nbody\n",
        )
        .unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(
            &schema_path,
            r#"{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "required": ["id", "content"],
                "properties": {
                    "id": {"type": "string"},
                    "content": {"type": "string"},
                    "entity": {"$ref": "urn:test:entity"}
                }
            }"#,
        )
        .unwrap();
        let ref_path = dir.path().join("entity.schema.json");
        fs::write(
            &ref_path,
            r#"{
                "$id": "urn:test:entity",
                "type": "object",
                "required": ["name"]
            }"#,
        )
        .unwrap();

        let projected = project_report(&md_path, &schema_path, &[ref_path]).unwrap();
        assert_eq!(projected["entity"]["name"], "widget");
    }

    #[test]
    fn rejects_a_ref_dependency_schema_with_no_id() {
        let dir = tempfile::tempdir().unwrap();
        let md_path = dir.path().join("report.md");
        fs::write(&md_path, "---\nid: r-1\ncontent: x\n---\n\nbody\n").unwrap();
        let schema_path = dir.path().join("findings.schema.json");
        fs::write(&schema_path, FINDINGS_SCHEMA).unwrap();
        let ref_path = dir.path().join("no-id.schema.json");
        fs::write(&ref_path, r#"{"type": "object"}"#).unwrap();

        let error = project_report(&md_path, &schema_path, &[ref_path]).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::RefSchemaMissingId { .. }
        ));
    }
}
