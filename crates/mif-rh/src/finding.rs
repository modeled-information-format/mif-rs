//! A research-harness-template (rht) finding file: `reports/<topic>/findings/<id>.json`.

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::error::MifRhError;

/// One finding, as written by rht's dimension-analyst agents.
#[derive(Debug, Clone, Deserialize)]
pub struct Finding {
    /// The finding's unique identifier.
    #[serde(rename = "@id")]
    pub id: String,
    /// The typed entity payload, if this finding declares one. Validated
    /// against its resolved entity type's schema.
    #[serde(default)]
    pub entity: Option<Value>,
    /// An explicit ontology reference, disambiguating which ontology's
    /// entity type this finding intends when more than one bound ontology
    /// declares the same type name.
    #[serde(default)]
    pub ontology: Option<FindingOntologyRef>,
    /// Every other top-level field (e.g. `content`), scanned for discovery
    /// pattern matches when no explicit typing is present.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

/// A finding's explicit `ontology.id` reference.
#[derive(Debug, Clone, Deserialize)]
pub struct FindingOntologyRef {
    /// The referenced ontology's id.
    pub id: String,
}

impl Finding {
    /// Reads and parses a finding file.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::FindingIo`] if `path` cannot be read, or
    /// [`MifRhError::FindingJson`] if it is not valid JSON.
    pub fn load(path: &Path) -> Result<Self, MifRhError> {
        let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::FindingIo {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&contents).map_err(|source| MifRhError::FindingJson {
            path: path.display().to_string(),
            source,
        })
    }

    /// The finding's declared `entity.entity_type`, if any, treating an
    /// empty string the same as absent.
    #[must_use]
    pub fn entity_type(&self) -> Option<&str> {
        self.entity
            .as_ref()?
            .get("entity_type")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Whether this finding carries typing intent: an `entity` block or an
    /// explicit `ontology.id` reference. When false, classification falls
    /// back to discovery-pattern matching.
    #[must_use]
    pub const fn has_typing_intent(&self) -> bool {
        self.entity.is_some() || self.ontology.is_some()
    }

    /// The concatenation of every top-level, non-`@`-prefixed string field
    /// (typically just `content`), for discovery-pattern matching. Matches
    /// rht's own `resolve-ontology.sh`, which joins the finding's top-level
    /// string fields with spaces before testing each discovery pattern.
    #[must_use]
    pub fn discovery_text(&self) -> String {
        self.extra
            .iter()
            .filter(|(key, _)| !key.starts_with('@'))
            .filter_map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::Finding;

    fn write_temp(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file
    }

    #[test]
    fn loads_a_typed_finding() {
        let file =
            write_temp(r#"{"@id":"f-good","entity":{"name":"Algebra I","entity_type":"title"}}"#);
        let finding = Finding::load(file.path()).unwrap();
        assert_eq!(finding.id, "f-good");
        assert_eq!(finding.entity_type(), Some("title"));
        assert!(finding.has_typing_intent());
    }

    #[test]
    fn untyped_finding_has_no_typing_intent() {
        let file = write_temp(r#"{"@id":"f-untyped","content":"x"}"#);
        let finding = Finding::load(file.path()).unwrap();
        assert!(!finding.has_typing_intent());
        assert_eq!(finding.discovery_text(), "x");
    }

    #[test]
    fn discovery_text_excludes_at_prefixed_top_level_fields() {
        let file = write_temp(
            r#"{"@id":"f-context","@context":"https://mif-spec.dev/context.jsonld","content":"has an ISBN"}"#,
        );
        let finding = Finding::load(file.path()).unwrap();
        assert_eq!(finding.discovery_text(), "has an ISBN");
    }

    #[test]
    fn empty_entity_type_string_counts_as_absent() {
        let file = write_temp(r#"{"@id":"f-empty","entity":{"entity_type":""}}"#);
        let finding = Finding::load(file.path()).unwrap();
        assert_eq!(finding.entity_type(), None);
        // Still has typing intent: the `entity` block itself is present.
        assert!(finding.has_typing_intent());
    }

    #[test]
    fn reports_invalid_json() {
        let file = write_temp("not json");
        let error = Finding::load(file.path()).unwrap_err();
        assert!(matches!(error, super::MifRhError::FindingJson { .. }));
    }

    #[test]
    fn reports_missing_file() {
        let error = Finding::load(std::path::Path::new("/nonexistent/finding.json")).unwrap_err();
        assert!(matches!(error, super::MifRhError::FindingIo { .. }));
    }
}
