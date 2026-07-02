use serde::{Deserialize, Serialize};

/// A reference to the ontology a MIF memory conforms to.
///
/// Corresponds to `$defs.OntologyReference` in `mif.schema.json`. `id` must
/// match the `ontology.id` declared in the referenced ontology definition
/// (see the three-tier resolution chain: `mif-base` -> `shared-traits` ->
/// domain ontologies, driven by each ontology's own `extends` list).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OntologyReference {
    /// JSON-LD type marker. Always `"OntologyReference"` when present;
    /// preserved verbatim across round-trips rather than re-derived, since
    /// the schema declares it optional.
    #[serde(rename = "@type", skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Ontology identifier (`^[a-z][a-z0-9-]*$`).
    pub id: String,
    /// Ontology version (`^\d+\.\d+\.\d+.*$`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// URI to the ontology definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

impl OntologyReference {
    /// Creates a new reference to the ontology identified by `id`.
    #[must_use]
    pub const fn new(id: String) -> Self {
        Self {
            r#type: None,
            id,
            version: None,
            uri: None,
        }
    }

    /// Sets the ontology version.
    #[must_use]
    pub fn with_version(mut self, version: String) -> Self {
        self.version = Some(version);
        self
    }

    /// Sets the URI to the ontology definition.
    #[must_use]
    pub fn with_uri(mut self, uri: String) -> Self {
        self.uri = Some(uri);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::OntologyReference;

    #[test]
    fn round_trips_through_json() {
        let reference = OntologyReference::new("grazing-plan".to_string())
            .with_version("1.0.0".to_string())
            .with_uri("https://mif-spec.dev/ontologies/grazing-plan".to_string());
        let json = serde_json::to_string(&reference).unwrap();
        let parsed: OntologyReference = serde_json::from_str(&json).unwrap();
        assert_eq!(reference, parsed);
    }

    #[test]
    fn omits_absent_optional_fields() {
        let reference = OntologyReference::new("mif-base".to_string());
        let json = serde_json::to_value(&reference).unwrap();
        assert!(json.get("version").is_none());
        assert!(json.get("uri").is_none());
        assert!(json.get("@type").is_none());
    }
}
