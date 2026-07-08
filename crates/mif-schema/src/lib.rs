//! JSON Schema validation for the MIF (Modeled Information Format) ecosystem.
//!
//! Validates MIF documents and citation objects against the canonical MIF
//! JSON Schema (draft 2020-12; see <https://mif-spec.dev/schema/>). Schemas
//! are vendored at compile time (`src/schemas/`, synced from the canonical
//! `MIF` repo) and resolved entirely offline — no network access happens at
//! validation time.

use std::sync::OnceLock;

use jsonschema::{Registry, Validator};
use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};
use serde_json::Value;

const MIF_SCHEMA: &str = include_str!("schemas/mif.schema.json");
const CITATION_SCHEMA: &str = include_str!("schemas/citation.schema.json");
const ONTOLOGY_SCHEMA: &str = include_str!("schemas/ontology.schema.json");
const ENTITY_REFERENCE_SCHEMA: &str =
    include_str!("schemas/definitions/entity-reference.schema.json");
const ENTITY_REFERENCE_SCHEMA_ID: &str =
    "https://mif-spec.dev/schema/definitions/entity-reference.schema.json";

/// The original `serde_json`/`jsonschema` error message behind a
/// [`MifSchemaError::SchemaCompilation`] failure.
///
/// `serde_json::Error` and `jsonschema`'s build/registry errors are
/// stringified before being cached (see `build_registry`/`build_validator`),
/// so this wrapper is what `#[source]` actually points at: it preserves the
/// original error's message so `std::error::Error::source()` on
/// `SchemaCompilation` yields a real hop in the chain instead of `None`.
/// This wrapper's own `source()` returns `None` — the original typed error
/// itself could not be preserved through the cache.
#[derive(Debug)]
pub struct SchemaCompilationSource(String);

impl std::fmt::Display for SchemaCompilationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SchemaCompilationSource {}

/// A MIF level floor (L1/L2/L3 conformance tier).
///
/// Level floors are additive: L2 requires everything L1 requires plus its
/// own fields, and L3 requires everything L2 requires plus its own. L1's
/// fields (`id`/`@id`, `type`/`conceptType`, `created`) are already enforced
/// by the canonical core schema's `required` list, so [`validate_level`]
/// with [`Level::L1`] is equivalent to [`validate_document`]; L2 and L3 add
/// genuinely new checks beyond the core schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// `id`, `type`, `created` — already enforced by the core schema.
    L1,
    /// Adds `namespace`, `modified`, `temporal`.
    L2,
    /// Adds `provenance` and a non-null `temporal.validFrom`.
    L3,
}

impl Level {
    const fn as_u8(self) -> u8 {
        match self {
            Self::L1 => 1,
            Self::L2 => 2,
            Self::L3 => 3,
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "L{}", self.as_u8())
    }
}

impl TryFrom<u8> for Level {
    type Error = MifSchemaError;

    /// Converts a raw level number into a [`Level`].
    ///
    /// # Errors
    ///
    /// Returns [`MifSchemaError::UnsupportedLevel`] if `value` is not 1, 2, or 3.
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::L1),
            2 => Ok(Self::L2),
            3 => Ok(Self::L3),
            other => Err(MifSchemaError::UnsupportedLevel(other)),
        }
    }
}

/// Error validating a MIF document or citation against the canonical schema.
#[derive(Debug, thiserror::Error)]
pub enum MifSchemaError {
    /// The vendored schema itself failed to compile. Indicates a bug in
    /// this crate's vendored schema files, not a problem with the instance
    /// being validated.
    #[error("internal error: vendored MIF schema failed to compile: {0}")]
    SchemaCompilation(#[source] SchemaCompilationSource),
    /// The instance failed schema validation.
    #[error("MIF document failed schema validation: {}", .0.join("; "))]
    Invalid(Vec<String>),
    /// The instance passed core schema validation but is missing fields the
    /// requested [`Level`] floor requires.
    #[error(
        "document does not satisfy the {level} level floor: missing {}",
        .missing.join(", ")
    )]
    LevelFloorViolation {
        /// The level floor that was requested.
        level: Level,
        /// The missing field names (dotted paths for nested fields, e.g.
        /// `temporal.validFrom`).
        missing: Vec<String>,
    },
    /// The requested level number is not a valid MIF level floor.
    #[error("unsupported MIF level: {0} (must be 1, 2, or 3)")]
    UnsupportedLevel(u8),
}

impl MifSchemaError {
    /// The individual validation error messages, if this is an
    /// [`MifSchemaError::Invalid`] or [`MifSchemaError::LevelFloorViolation`].
    /// Empty for [`MifSchemaError::SchemaCompilation`] and
    /// [`MifSchemaError::UnsupportedLevel`].
    #[must_use]
    pub fn messages(&self) -> &[String] {
        match self {
            Self::Invalid(errors)
            | Self::LevelFloorViolation {
                missing: errors, ..
            } => errors,
            Self::SchemaCompilation(_) | Self::UnsupportedLevel(_) => &[],
        }
    }

    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::SchemaCompilation(_) => ProblemMeta {
                slug: "schema-compilation",
                version: "v1",
                title: "Internal schema compilation error",
                status: 500,
                exit_code: 1,
            },
            Self::Invalid(_) => ProblemMeta {
                slug: "invalid-document",
                version: "v1",
                title: "Document failed schema validation",
                status: 422,
                exit_code: 2,
            },
            Self::LevelFloorViolation { .. } => ProblemMeta {
                slug: "level-floor-violation",
                version: "v1",
                title: "Document does not satisfy the requested level floor",
                status: 422,
                exit_code: 5,
            },
            Self::UnsupportedLevel(_) => ProblemMeta {
                slug: "unsupported-level",
                version: "v1",
                title: "Unsupported MIF level",
                status: 400,
                exit_code: 2,
            },
        }
    }
}

impl ToProblem for MifSchemaError {
    fn to_problem(&self) -> ProblemDetails {
        let (fix, action) = match self {
            Self::SchemaCompilation(_) => (
                SuggestedFix::new(
                    "This indicates a bug in mif-schema's vendored schema files, not the \
                     instance being validated. Report it upstream.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "File a bug against mif-schema's vendored schemas",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            ),
            Self::Invalid(_) => (
                SuggestedFix::new(
                    "Correct the document so it conforms to the canonical MIF JSON Schema, \
                     then retry.",
                    Applicability::MaybeIncorrect,
                ),
                CodeAction::new(
                    "Fix the reported schema violations",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            ),
            Self::LevelFloorViolation { .. } => (
                SuggestedFix::new(
                    "Add the missing fields the requested level floor requires, then retry.",
                    Applicability::MaybeIncorrect,
                ),
                CodeAction::new(
                    "Add the missing level-floor fields",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            ),
            Self::UnsupportedLevel(_) => (
                SuggestedFix::new(
                    "Request level 1, 2, or 3.",
                    Applicability::MachineApplicable,
                ),
                CodeAction::new(
                    "Use a supported level (1, 2, or 3)",
                    "quickfix",
                    Applicability::MachineApplicable,
                ),
            ),
        };
        self.meta()
            .into_details(env!("CARGO_PKG_NAME"), self.to_string())
            .with_suggested_fix(fix)
            .with_code_action(action)
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
        .map_err(|e| MifSchemaError::SchemaCompilation(SchemaCompilationSource(e.clone())))
}

fn citation_validator() -> Result<&'static Validator, MifSchemaError> {
    static VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| build_validator(CITATION_SCHEMA))
        .as_ref()
        .map_err(|e| MifSchemaError::SchemaCompilation(SchemaCompilationSource(e.clone())))
}

fn ontology_validator() -> Result<&'static Validator, MifSchemaError> {
    static VALIDATOR: OnceLock<Result<Validator, String>> = OnceLock::new();
    VALIDATOR
        .get_or_init(|| build_validator(ONTOLOGY_SCHEMA))
        .as_ref()
        .map_err(|e| MifSchemaError::SchemaCompilation(SchemaCompilationSource(e.clone())))
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

/// Returns `true` if `instance` has a present, non-null value at `key`.
fn has_non_null_field(instance: &Value, key: &str) -> bool {
    instance.get(key).is_some_and(|value| !value.is_null())
}

/// The field names (dotted paths for nested fields) that `instance` is
/// missing relative to `level`'s floor, beyond what the core schema already
/// requires. Empty if `instance` already satisfies the floor.
fn missing_level_fields(instance: &Value, level: Level) -> Vec<String> {
    let mut missing = Vec::new();
    if level.as_u8() >= Level::L2.as_u8() {
        for field in ["namespace", "modified", "temporal"] {
            if !has_non_null_field(instance, field) {
                missing.push(field.to_string());
            }
        }
    }
    if level.as_u8() >= Level::L3.as_u8() {
        if !has_non_null_field(instance, "provenance") {
            missing.push("provenance".to_string());
        }
        let has_valid_from = instance
            .get("temporal")
            .and_then(|temporal| temporal.get("validFrom"))
            .is_some_and(|value| !value.is_null());
        if !has_valid_from {
            missing.push("temporal.validFrom".to_string());
        }
    }
    missing
}

/// Validates a MIF document (a JSON-LD-projected memory) against the
/// canonical `mif.schema.json`, then against the additional fields the
/// requested [`Level`] floor requires.
///
/// # Errors
///
/// Returns [`MifSchemaError::Invalid`] or [`MifSchemaError::SchemaCompilation`]
/// per [`validate_document`], or [`MifSchemaError::LevelFloorViolation`] if
/// `instance` passes core schema validation but is missing fields the
/// requested level floor requires.
pub fn validate_level(instance: &Value, level: Level) -> Result<(), MifSchemaError> {
    validate_document(instance)?;
    let missing = missing_level_fields(instance, level);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(MifSchemaError::LevelFloorViolation { level, missing })
    }
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
    use mif_problem::ToProblem;
    use serde_json::json;

    use super::{
        Level, MifSchemaError, SchemaCompilationSource, validate_citation, validate_document,
        validate_level, validate_ontology_definition,
    };

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
    fn messages_reports_the_invalid_variant_and_is_empty_for_schema_compilation() {
        let mut instance = minimal_valid_document();
        instance.as_object_mut().unwrap().remove("conceptType");
        let error = validate_document(&instance).unwrap_err();
        assert!(matches!(error, MifSchemaError::Invalid(_)));
        assert!(!error.messages().is_empty());

        let compilation_error = MifSchemaError::SchemaCompilation(SchemaCompilationSource(
            "synthetic failure for coverage".to_string(),
        ));
        assert!(compilation_error.messages().is_empty());
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
    fn entity_type_with_classification_fields_passes() {
        // The v1.1 additive fields: aliases, exemplars, negative_examples.
        let ontology = json!({
            "ontology": {
                "id": "sec-fixture",
                "version": "1.1.0",
            },
            "entity_types": [{
                "name": "control",
                "base": "semantic",
                "aliases": ["safeguard", "countermeasure"],
                "exemplars": ["Enforce MFA for all administrative access"],
                "negative_examples": ["An incident report describing a control failure"],
            }]
        });
        assert!(validate_ontology_definition(&ontology).is_ok());
    }

    #[test]
    fn entity_type_classification_fields_reject_non_string_and_empty_items() {
        let non_string_alias = json!({
            "ontology": { "id": "sec-fixture", "version": "1.1.0" },
            "entity_types": [{
                "name": "control",
                "base": "semantic",
                "aliases": [123],
            }]
        });
        assert!(validate_ontology_definition(&non_string_alias).is_err());

        let empty_exemplar = json!({
            "ontology": { "id": "sec-fixture", "version": "1.1.0" },
            "entity_types": [{
                "name": "control",
                "base": "semantic",
                "exemplars": [""],
            }]
        });
        assert!(validate_ontology_definition(&empty_exemplar).is_err());
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

    #[test]
    fn invalid_document_maps_to_versioned_problem_details() {
        let mut instance = minimal_valid_document();
        instance.as_object_mut().unwrap().remove("conceptType");
        let error = validate_document(&instance).unwrap_err();
        let problem = error.to_problem();

        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-document/v1"
        );
        assert_eq!(problem.status, 422);
        assert_eq!(problem.exit_code, Some(2));
        assert!(problem.suggested_fix.is_some());
        assert_eq!(problem.code_actions.len(), 1);
        assert!(problem.detail.contains("schema validation"));
    }

    #[test]
    fn schema_compilation_error_maps_to_distinct_problem_type() {
        let error = MifSchemaError::SchemaCompilation(SchemaCompilationSource("boom".to_string()));
        let problem = error.to_problem();

        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/schema-compilation/v1"
        );
        assert_eq!(problem.status, 500);
        assert_eq!(problem.exit_code, Some(1));
    }

    #[test]
    fn schema_compilation_error_source_preserves_the_underlying_cause() {
        let error = MifSchemaError::SchemaCompilation(SchemaCompilationSource("boom".to_string()));

        let source = std::error::Error::source(&error).expect("source should not be None");
        assert_eq!(source.to_string(), "boom");
    }

    fn l2_document() -> serde_json::Value {
        let mut instance = minimal_valid_document();
        instance["namespace"] = json!("test");
        instance["modified"] = json!("2026-07-02T00:00:00Z");
        instance["temporal"] = json!({});
        instance
    }

    fn l3_document() -> serde_json::Value {
        let mut instance = l2_document();
        instance["provenance"] = json!({});
        instance["temporal"]["validFrom"] = json!("2026-07-02T00:00:00Z");
        instance
    }

    #[test]
    fn level_display_formats_as_l_and_number() {
        assert_eq!(Level::L1.to_string(), "L1");
        assert_eq!(Level::L2.to_string(), "L2");
        assert_eq!(Level::L3.to_string(), "L3");
    }

    #[test]
    fn level_try_from_accepts_one_two_three() {
        assert_eq!(Level::try_from(1).unwrap(), Level::L1);
        assert_eq!(Level::try_from(2).unwrap(), Level::L2);
        assert_eq!(Level::try_from(3).unwrap(), Level::L3);
    }

    #[test]
    fn level_try_from_rejects_out_of_range_numbers() {
        let error = Level::try_from(0).unwrap_err();
        assert!(matches!(error, MifSchemaError::UnsupportedLevel(0)));
        assert!(error.messages().is_empty());

        let error = Level::try_from(4).unwrap_err();
        assert!(matches!(error, MifSchemaError::UnsupportedLevel(4)));
    }

    #[test]
    fn unsupported_level_error_maps_to_versioned_problem_details() {
        let error = MifSchemaError::UnsupportedLevel(9);
        let problem = error.to_problem();

        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/unsupported-level/v1"
        );
        assert_eq!(problem.status, 400);
        assert_eq!(problem.exit_code, Some(2));
    }

    #[test]
    fn validate_level_l1_is_satisfied_by_the_minimal_valid_document() {
        assert!(validate_level(&minimal_valid_document(), Level::L1).is_ok());
    }

    #[test]
    fn validate_level_l2_fails_on_a_document_missing_namespace_modified_temporal() {
        let error = validate_level(&minimal_valid_document(), Level::L2).unwrap_err();
        let MifSchemaError::LevelFloorViolation { level, missing } = error else {
            unreachable!("expected LevelFloorViolation")
        };
        assert_eq!(level, Level::L2);
        assert_eq!(missing, vec!["namespace", "modified", "temporal"]);
    }

    #[test]
    fn validate_level_l2_passes_once_namespace_modified_temporal_are_present() {
        assert!(validate_level(&l2_document(), Level::L2).is_ok());
    }

    #[test]
    fn validate_level_l3_fails_on_a_document_missing_provenance_and_valid_from() {
        let error = validate_level(&l2_document(), Level::L3).unwrap_err();
        let MifSchemaError::LevelFloorViolation { level, missing } = error else {
            unreachable!("expected LevelFloorViolation")
        };
        assert_eq!(level, Level::L3);
        assert_eq!(missing, vec!["provenance", "temporal.validFrom"]);
    }

    #[test]
    fn validate_level_l3_treats_a_null_valid_from_as_missing() {
        let mut instance = l2_document();
        instance["provenance"] = json!({});
        instance["temporal"]["validFrom"] = serde_json::Value::Null;
        let error = validate_level(&instance, Level::L3).unwrap_err();
        let MifSchemaError::LevelFloorViolation { missing, .. } = error else {
            unreachable!("expected LevelFloorViolation")
        };
        assert_eq!(missing, vec!["temporal.validFrom"]);
    }

    #[test]
    fn validate_level_l3_passes_once_provenance_and_valid_from_are_present() {
        assert!(validate_level(&l3_document(), Level::L3).is_ok());
    }

    #[test]
    fn validate_level_reports_core_schema_failures_before_level_floor_checks() {
        let mut instance = minimal_valid_document();
        instance.as_object_mut().unwrap().remove("conceptType");
        let error = validate_level(&instance, Level::L3).unwrap_err();
        assert!(matches!(error, MifSchemaError::Invalid(_)));
    }

    #[test]
    fn level_floor_violation_maps_to_versioned_problem_details() {
        let error = validate_level(&minimal_valid_document(), Level::L2).unwrap_err();
        let problem = error.to_problem();

        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/level-floor-violation/v1"
        );
        assert_eq!(problem.status, 422);
        assert_eq!(problem.exit_code, Some(5));
        assert!(problem.suggested_fix.is_some());
        assert!(!error.messages().is_empty());
    }
}
