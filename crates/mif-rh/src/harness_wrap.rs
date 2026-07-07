//! MIF source-envelope wrapping (rht Category B, Story #302).
//!
//! Ports rht's `scripts/wrap-source.sh`: normalizes a raw ingested source
//! into a MIF source-envelope at the ingestion boundary and validates it
//! at MIF Level 3 before any analyst consumes it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::MifRhError;
use crate::harness_project::{read_text, validate_against_schema};

/// Inputs for [`wrap_source`].
pub struct WrapSourceInputs<'a> {
    /// The source URL.
    pub url: &'a str,
    /// The source's MIME content type.
    pub content_type: &'a str,
    /// The namespace the source belongs to.
    pub namespace: &'a str,
    /// The source's slug (combined with `namespace` to form its `@id`).
    pub slug: &'a str,
    /// The source's title. Defaults to `slug` if empty.
    pub title: &'a str,
    /// The source content.
    pub content: &'a str,
    /// The provenance `sourceType`. Defaults to `"agent_inferred"` if
    /// empty.
    pub source_type: &'a str,
    /// The `created`/`fetchedAt` timestamp, RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`).
    pub created: &'a str,
}

/// Composes `inputs` into a MIF source-envelope and validates it against
/// `schema_path` (with `$ref` dependencies in `ref_paths`).
///
/// # Errors
///
/// Returns [`MifRhError::Io`]/[`MifRhError::Json`] if a schema file cannot
/// be read/parsed, and [`MifRhError::SchemaValidationFailed`] if the
/// envelope does not conform.
pub fn wrap_source(
    inputs: &WrapSourceInputs<'_>,
    schema_path: &Path,
    ref_paths: &[PathBuf],
) -> Result<Value, MifRhError> {
    let title = if inputs.title.is_empty() {
        inputs.slug
    } else {
        inputs.title
    };
    let source_type = if inputs.source_type.is_empty() {
        "agent_inferred"
    } else {
        inputs.source_type
    };
    let envelope = json!({
        "@context": "https://mif-spec.dev/schema/context.jsonld",
        "@type": "Concept",
        "@id": format!("urn:mif:source:{}:{}", inputs.namespace, inputs.slug),
        "conceptType": "episodic",
        "namespace": format!("{}/sources", inputs.namespace),
        "title": title,
        "content": inputs.content,
        "created": inputs.created,
        "provenance": {
            "@type": "Provenance",
            "sourceType": source_type,
            "confidence": 0.8,
            "trustLevel": "moderate_confidence",
        },
        "extensions": {
            "harness": {
                "source": {
                    "url": inputs.url,
                    "fetchedAt": inputs.created,
                    "contentType": inputs.content_type,
                }
            }
        },
    });

    let envelope_path = PathBuf::from(format!(
        "urn:mif:source:{}:{}",
        inputs.namespace, inputs.slug
    ));
    validate_against_schema(&envelope, &envelope_path, schema_path, ref_paths)?;
    Ok(envelope)
}

/// Reads source content from `content_file` if given, else `content` if
/// non-empty, else `stdin` if it is not a TTY.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `content_file` cannot be read, or
/// [`MifRhError::EmptySourceContent`] if no content is available from any
/// of the three sources.
pub fn read_source_content(
    content_file: Option<&Path>,
    content: &str,
) -> Result<String, MifRhError> {
    use std::io::{IsTerminal, Read};

    let text = if let Some(path) = content_file {
        read_text(path)?
    } else if !content.is_empty() {
        content.to_string()
    } else if std::io::stdin().is_terminal() {
        String::new()
    } else {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .map_err(|source| MifRhError::Io {
                path: "<stdin>".to_string(),
                source,
            })?;
        buffer
    };
    if text.trim().is_empty() {
        return Err(MifRhError::EmptySourceContent);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::{WrapSourceInputs, wrap_source};
    use std::fs;

    const SOURCE_ENVELOPE_SCHEMA: &str = r#"{
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["@id", "content", "title"],
        "properties": {
            "@id": {"type": "string"},
            "content": {"type": "string", "minLength": 1},
            "title": {"type": "string"}
        }
    }"#;

    fn inputs() -> WrapSourceInputs<'static> {
        WrapSourceInputs {
            url: "https://example.com/paper",
            content_type: "text/html",
            namespace: "physics",
            slug: "example-paper",
            title: "",
            content: "the paper's full text",
            source_type: "",
            created: "2026-01-01T00:00:00Z",
        }
    }

    #[test]
    fn composes_a_valid_envelope_with_the_expected_id() {
        let dir = tempfile::tempdir().unwrap();
        let schema_path = dir.path().join("source-envelope.schema.json");
        fs::write(&schema_path, SOURCE_ENVELOPE_SCHEMA).unwrap();

        let envelope = wrap_source(&inputs(), &schema_path, &[]).unwrap();
        assert_eq!(envelope["@id"], "urn:mif:source:physics:example-paper");
        assert_eq!(envelope["content"], "the paper's full text");
    }

    #[test]
    fn falls_back_to_slug_as_title_when_title_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let schema_path = dir.path().join("source-envelope.schema.json");
        fs::write(&schema_path, SOURCE_ENVELOPE_SCHEMA).unwrap();

        let envelope = wrap_source(&inputs(), &schema_path, &[]).unwrap();
        assert_eq!(envelope["title"], "example-paper");
    }

    #[test]
    fn defaults_source_type_to_agent_inferred() {
        let dir = tempfile::tempdir().unwrap();
        let schema_path = dir.path().join("source-envelope.schema.json");
        fs::write(&schema_path, SOURCE_ENVELOPE_SCHEMA).unwrap();

        let envelope = wrap_source(&inputs(), &schema_path, &[]).unwrap();
        assert_eq!(envelope["provenance"]["sourceType"], "agent_inferred");
    }

    #[test]
    fn rejects_an_envelope_that_fails_schema_validation() {
        let dir = tempfile::tempdir().unwrap();
        let schema_path = dir.path().join("source-envelope.schema.json");
        fs::write(&schema_path, SOURCE_ENVELOPE_SCHEMA).unwrap();
        let mut broken = inputs();
        broken.content = "";

        let error = wrap_source(&broken, &schema_path, &[]).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::SchemaValidationFailed { .. }
        ));
    }
}
