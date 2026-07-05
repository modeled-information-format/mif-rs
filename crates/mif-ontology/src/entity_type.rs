//! The full `entity_type` model from `ontology.schema.json`.
//!
//! Models `$defs/entityType` including the v1.1 classification fields
//! (`aliases`/`exemplars`/`negative_examples`) and the positive
//! embedding-document composition rule they exist for.
//!
//! Parsing is deliberately tolerant (only `name` is required, unknown keys
//! are ignored): consumers like `mif-rh` load real-world ontology pack YAML
//! without schema-validating first, and a lenient model keeps that path
//! working. Schema-strict validation stays where it already lives —
//! [`crate::parse_definition`] validates a whole definition document against
//! the vendored schema before extracting metadata.

use serde::Deserialize;
use serde_json::Value;

/// One entity type an ontology declares (`$defs/entityType` in
/// `ontology.schema.json`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EntityType {
    /// The entity type's name (`^[a-z][a-z0-9-]*$`).
    pub name: String,
    /// Human-readable description — the primary positive embedding signal.
    #[serde(default)]
    pub description: Option<String>,
    /// Base memory type (`semantic`/`episodic`/`procedural`). Optional here
    /// (tolerant parse); the schema requires it for a conformant document.
    #[serde(default)]
    pub base: Option<String>,
    /// Traits this entity type includes.
    #[serde(default)]
    pub traits: Vec<String>,
    /// Parent entity types this type specializes (subsumption).
    #[serde(default)]
    pub subtype_of: Vec<String>,
    /// Synonyms and label variations (`skos:altLabel` analog), distinct
    /// from `description`. Part of the positive embedding document.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Curated canonical example phrases or instances (`skos:example`
    /// analog; 2-5 recommended). Part of the positive embedding document.
    #[serde(default)]
    pub exemplars: Vec<String>,
    /// Curated near-misses from the ontology's most confusable type pairs:
    /// texts that resemble this type but do NOT denote it. Never part of
    /// the positive embedding document (see [`EntityType::embedding_doc`]);
    /// consumed by the negative-demotion-v1 gate
    /// ([`crate::confidence::negative_demotes`]): a candidate whose query
    /// sits at least as close to one of these as to the type's positive
    /// document is barred from auto-classify eligibility. Types carrying
    /// none score exactly as before the gate existed.
    #[serde(default)]
    pub negative_examples: Vec<String>,
    /// The `{required, properties}` shape a finding's `entity` payload is
    /// validated against.
    #[serde(default)]
    pub schema: Value,
}

impl EntityType {
    /// Composes the positive embedding document for this entity type:
    /// `description`, then each alias, then each exemplar, newline-joined
    /// in that deterministic order, skipping empty strings.
    ///
    /// Returns `None` when no non-empty signal exists — callers skip such
    /// types rather than embedding an empty document (generalizing the
    /// previous skip-when-no-description behavior).
    ///
    /// `negative_examples` is deliberately excluded: it asserts what this
    /// type is *not*, and concatenating it into the positive document would
    /// poison the type's own embedding.
    #[must_use]
    pub fn embedding_doc(&self) -> Option<String> {
        let mut lines: Vec<&str> = Vec::new();
        if let Some(description) = &self.description
            && !description.is_empty()
        {
            lines.push(description);
        }
        lines.extend(
            self.aliases
                .iter()
                .map(String::as_str)
                .filter(|a| !a.is_empty()),
        );
        lines.extend(
            self.exemplars
                .iter()
                .map(String::as_str)
                .filter(|e| !e.is_empty()),
        );
        if lines.is_empty() {
            return None;
        }
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::EntityType;

    fn parse(yaml: &str) -> EntityType {
        serde_norway::from_str(yaml).unwrap()
    }

    #[test]
    fn legacy_yaml_without_new_fields_parses_with_empty_defaults() {
        let entity_type = parse(
            "
name: title
description: A published educational title
schema:
  required: [name]
",
        );
        assert_eq!(entity_type.name, "title");
        assert!(entity_type.aliases.is_empty());
        assert!(entity_type.exemplars.is_empty());
        assert!(entity_type.negative_examples.is_empty());
        assert_eq!(
            entity_type.embedding_doc().as_deref(),
            Some("A published educational title")
        );
    }

    #[test]
    fn enriched_yaml_parses_all_three_fields() {
        let entity_type = parse(
            "
name: control
base: semantic
description: A security safeguard
aliases: [safeguard, countermeasure]
exemplars:
  - Enforce MFA for all administrative access
negative_examples:
  - An incident report describing a control failure
",
        );
        assert_eq!(entity_type.aliases, ["safeguard", "countermeasure"]);
        assert_eq!(entity_type.exemplars.len(), 1);
        assert_eq!(entity_type.negative_examples.len(), 1);
    }

    #[test]
    fn embedding_doc_orders_description_then_aliases_then_exemplars() {
        let entity_type = parse(
            "
name: control
description: A security safeguard
aliases: [safeguard]
exemplars: [Enforce MFA]
negative_examples: [An incident report]
",
        );
        assert_eq!(
            entity_type.embedding_doc().as_deref(),
            Some("A security safeguard\nsafeguard\nEnforce MFA")
        );
    }

    #[test]
    fn embedding_doc_excludes_negative_examples() {
        let entity_type = parse(
            "
name: control
description: A security safeguard
negative_examples: [An incident report]
",
        );
        let doc = entity_type.embedding_doc().unwrap();
        assert!(!doc.contains("incident report"));
    }

    #[test]
    fn embedding_doc_is_none_when_no_positive_signal_exists() {
        let bare = parse("name: control");
        assert_eq!(bare.embedding_doc(), None);

        let only_negative = parse(
            "
name: control
negative_examples: [An incident report]
",
        );
        assert_eq!(only_negative.embedding_doc(), None);
    }

    #[test]
    fn embedding_doc_skips_empty_strings_but_keeps_the_rest() {
        let entity_type = parse(
            "
name: control
aliases: ['', safeguard]
",
        );
        assert_eq!(entity_type.embedding_doc().as_deref(), Some("safeguard"));
    }
}
