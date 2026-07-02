//! JSON Schema validation for the MIF (Modeled Information Format) ecosystem.
//!
//! Validates MIF documents and citation objects against the canonical MIF
//! JSON Schema (draft 2020-12; see <https://mif-spec.dev/schema/>). Schemas
//! are vendored at compile time (`src/schemas/`, synced from the canonical
//! `MIF` repo) and resolved entirely offline — no network access happens at
//! validation time.

use std::sync::OnceLock;

use jsonschema::{Registry, Validator};
use serde_json::Value;

const MIF_SCHEMA: &str = include_str!("schemas/mif.schema.json");
const CITATION_SCHEMA: &str = include_str!("schemas/citation.schema.json");
const ONTOLOGY_SCHEMA: &str = include_str!("schemas/ontology.schema.json");
const ENTITY_REFERENCE_SCHEMA: &str =
    include_str!("schemas/definitions/entity-reference.schema.json");
const ENTITY_REFERENCE_SCHEMA_ID: &str =
    "https://mif-spec.dev/schema/definitions/entity-reference.schema.json";

/// Error validating a MIF document or citation against the canonical schema.
#[derive(Debug, Clone, thiserror::Error)]
pub enum MifSchemaError {
    /// The vendored schema itself failed to compile. Indicates a bug in
    /// this crate's vendored schema files, not a problem with the instance
    /// being validated.
    #[error("internal error: vendored MIF schema failed to compile: {0}")]
    SchemaCompilation(String),
    /// The instance failed schema validation.
    #[error("MIF document failed schema validation ({} error(s))", .0.len())]
    Invalid(Vec<String>),
}

impl MifSchemaError {
    /// The individual validation error messages, if this is an
    /// [`MifSchemaError::Invalid`]. Empty for [`MifSchemaError::SchemaCompilation`].
    #[must_use]
    pub fn messages(&self) -> &[String] {
        match self {
            Self::Invalid(errors) => errors,
            Self::SchemaCompilation(_) => &[],
        }
    }
}

fn build_registry() -> Result<Registry<'static>, String> {
    let entity_reference: Value =
        serde_json::from_str(ENTITY_REFERENCE_SCHEMA).map_err(|e| e.to_string())?;
    Registry::new()
        .add(ENTITY_REFERENCE_SCHEMA_ID, entity_reference)
        .map_err(|e| e.to_string())?
        .prepare()
        .map_err(|e| e.to_string())
}

fn build_validator(schema_json: &str) -> Result<Validator, String> {
    let schema: Value = serde_json::from_str(schema_json).map_err(|e| e.to_string())?;
    let registry = build_registry()?;
    jsonschema::options()
        .with_registry(&registry)
        .build(&schema)
        .map_err(|e| e.to_string())
}

fn document_validator() -> Result<&'static Validator, MifSchemaError> {
    static VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| build_validator(MIF_SCHEMA))
        .as_ref()
        .map_err(|e| MifSchemaError::SchemaCompilation(e.clone()))
}

fn citation_validator() -> Result<&'static Validator, MifSchemaError> {
    static VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| build_validator(CITATION_SCHEMA))
        .as_ref()
        .map_err(|e| MifSchemaError::SchemaCompilation(e.clone()))
}

fn ontology_validator() -> Result<&'static Validator, MifSchemaError> {
    static VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| build_validator(ONTOLOGY_SCHEMA))
        .as_ref()
        .map_err(|e| MifSchemaError::SchemaCompilation(e.clone()))
}

fn validate(validator: &Validator, instance: &Value) -> Result<(), MifSchemaError> {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(MifSchemaError::Invalid(errors))
    }
}

/// Validates a MIF document (a JSON-LD-projected memory) against the
/// canonical `mif.schema.json`.
///
/// # Errors
///
/// Returns [`MifSchemaError::Invalid`] with every validation error message
/// if `instance` does not conform to the schema, or
/// [`MifSchemaError::SchemaCompilation`] if the vendored schema itself
/// fails to compile (indicates a bug in this crate).
pub fn validate_document(instance: &Value) -> Result<(), MifSchemaError> {
    validate(document_validator()?, instance)
}

/// Validates a standalone MIF citation object against `citation.schema.json`.
///
/// # Errors
///
/// See [`validate_document`].
pub fn validate_citation(instance: &Value) -> Result<(), MifSchemaError> {
    validate(citation_validator()?, instance)
}

/// Validates an ontology definition object against `ontology.schema.json`.
///
/// # Errors
///
/// See [`validate_document`].
pub fn validate_ontology_definition(instance: &Value) -> Result<(), MifSchemaError> {
    validate(ontology_validator()?, instance)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{validate_citation, validate_document, validate_ontology_definition};

    fn minimal_valid_document() -> serde_json::Value {
        json!({
            "@context": "https://mif-spec.dev/schema/context.jsonld",
            "@type": "Concept",
            "@id": "urn:mif:memory:test-001",
            "conceptType": "semantic",
            "content": "Test content.",
            "created": "2026-07-02T00:00:00Z",
        })
    }

    #[test]
    fn valid_document_passes() {
        assert!(validate_document(&minimal_valid_document()).is_ok());
    }

    #[test]
    fn document_missing_required_field_fails() {
        let mut instance = minimal_valid_document();
        instance.as_object_mut().unwrap().remove("conceptType");
        let result = validate_document(&instance);
        assert!(result.is_err());
    }

    #[test]
    fn document_with_bad_id_pattern_fails() {
        let mut instance = minimal_valid_document();
        instance["@id"] = json!("not-a-urn");
        assert!(validate_document(&instance).is_err());
    }

    #[test]
    fn document_with_entity_reference_resolves_ref_chain() {
        let mut instance = minimal_valid_document();
        instance["entities"] = json!([{
            "@type": "EntityReference",
            "entity": { "@id": "urn:mif:entity:person:jane-smith" },
            "entityType": "Person",
        }]);
        assert!(validate_document(&instance).is_ok());
    }

    #[test]
    fn document_with_invalid_entity_reference_fails() {
        let mut instance = minimal_valid_document();
        instance["entities"] = json!([{
            "@type": "EntityReference",
            "entity": { "@id": "not-a-urn" },
        }]);
        assert!(validate_document(&instance).is_err());
    }

    #[test]
    fn valid_citation_passes() {
        let citation = json!({
            "@type": "Citation",
            "citationType": "documentation",
            "citationRole": "source",
            "title": "MIF Specification",
            "url": "https://mif-spec.dev",
        });
        assert!(validate_citation(&citation).is_ok());
    }

    #[test]
    fn valid_ontology_definition_passes() {
        let ontology = json!({
            "ontology": {
                "id": "mif-base",
                "version": "1.0.0",
            }
        });
        assert!(validate_ontology_definition(&ontology).is_ok());
    }

    #[test]
    fn ontology_definition_with_bad_id_pattern_fails() {
        let ontology = json!({
            "ontology": {
                "id": "Not_Valid",
                "version": "1.0.0",
            }
        });
        assert!(validate_ontology_definition(&ontology).is_err());
    }

    #[test]
    fn citation_missing_required_field_fails() {
        let citation = json!({
            "@type": "Citation",
            "citationType": "documentation",
            "citationRole": "source",
            "title": "MIF Specification",
        });
        assert!(validate_citation(&citation).is_err());
    }
}
