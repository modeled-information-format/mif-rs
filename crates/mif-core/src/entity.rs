use serde::{Deserialize, Serialize};

/// Closed pointer to an entity mentioned by a MIF memory.
///
/// Corresponds to `schema/definitions/entity-reference.schema.json`. See
/// [`EntityData`] for the open-payload counterpart used when a memory *is*
/// an entity, rather than merely mentioning one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityReference {
    /// JSON-LD type marker. Always `"EntityReference"`.
    #[serde(rename = "@type")]
    pub r#type: String,
    /// The entity's identifier.
    pub entity: EntityId,
    /// Entity type classification.
    #[serde(rename = "entityType", skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<EntityType>,
    /// Display name for the entity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Role of the entity in the memory context (e.g. author, subject, topic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl EntityReference {
    /// Creates a new reference to the entity identified by `id`
    /// (e.g. `urn:mif:entity:person:jane-smith`).
    #[must_use]
    pub fn new(id: String) -> Self {
        Self {
            r#type: "EntityReference".to_string(),
            entity: EntityId { id },
            entity_type: None,
            name: None,
            role: None,
        }
    }

    /// Sets the entity type classification.
    #[must_use]
    pub fn with_entity_type(mut self, entity_type: EntityType) -> Self {
        self.entity_type = Some(entity_type);
        self
    }

    /// Sets the display name.
    #[must_use]
    pub fn with_name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    /// Sets the entity's role in the memory context.
    #[must_use]
    pub fn with_role(mut self, role: String) -> Self {
        self.role = Some(role);
        self
    }
}

/// The entity identifier object nested inside an [`EntityReference`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityId {
    /// Entity URN identifier (`^urn:mif:entity:`).
    #[serde(rename = "@id")]
    pub id: String,
}

/// Entity type classification: a closed vocabulary of well-known types, or
/// a custom ontology-defined type (`^[a-z][a-z0-9-]*$`) preserved verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EntityType {
    /// One of the schema's closed enum values.
    Known(KnownEntityType),
    /// A custom entity type from an ontology (e.g. `grazing-plan`, `soil-profile`).
    Custom(String),
}

/// The closed set of well-known entity type values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KnownEntityType {
    /// A person.
    Person,
    /// An organization.
    Organization,
    /// A technology.
    Technology,
    /// A concept.
    Concept,
    /// A file.
    File,
}

/// Open, ontology-typed entity payload for a memory that *is* an entity.
///
/// Corresponds to `$defs.EntityData` in `mif.schema.json`, an open schema
/// (`additionalProperties: true`) — see [`EntityReference`] for the closed
/// pointer counterpart used when a memory merely mentions an entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityData {
    /// The entity's name.
    pub name: String,
    /// The ontology-defined entity type (`^[a-z][a-z0-9-]*$`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// The entity's identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    /// Additional ontology-defined fields, preserved losslessly across
    /// round-trips since this schema is open (`additionalProperties: true`).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl EntityData {
    /// Creates a new entity payload with the given name and no extra fields.
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            name,
            entity_type: None,
            entity_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EntityData, EntityReference, EntityType, KnownEntityType};

    fn reference_with_type(entity_type: &str) -> String {
        format!(
            r#"{{"@type":"EntityReference","entity":{{"@id":"urn:mif:entity:person:jane-smith"}},"entityType":"{entity_type}"}}"#
        )
    }

    #[test]
    fn round_trips_known_entity_type() {
        let json = reference_with_type("Person");
        let parsed: EntityReference = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.entity_type,
            Some(EntityType::Known(KnownEntityType::Person))
        );
        let reserialized = serde_json::to_string(&parsed).unwrap();
        let reparsed: EntityReference = serde_json::from_str(&reserialized).unwrap();
        assert_eq!(parsed, reparsed);
    }

    #[test]
    fn preserves_custom_entity_type_string() {
        let json = reference_with_type("grazing-plan");
        let parsed: EntityReference = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.entity_type,
            Some(EntityType::Custom("grazing-plan".to_string()))
        );
    }

    #[test]
    fn entity_data_flattens_unknown_fields_losslessly() {
        let json = r#"{"name":"Jane Smith","entity_type":"person","herd_size":42}"#;
        let parsed: EntityData = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "Jane Smith");
        assert_eq!(parsed.extra.get("herd_size"), Some(&serde_json::json!(42)));
        let reserialized = serde_json::to_value(&parsed).unwrap();
        assert_eq!(reserialized["herd_size"], serde_json::json!(42));
    }

    #[test]
    fn entity_reference_builder_sets_all_optional_fields() {
        let reference = EntityReference::new("urn:mif:entity:person:jane-smith".to_string())
            .with_entity_type(EntityType::Known(KnownEntityType::Person))
            .with_name("Jane Smith".to_string())
            .with_role("author".to_string());

        assert_eq!(reference.r#type, "EntityReference");
        assert_eq!(reference.entity.id, "urn:mif:entity:person:jane-smith");
        assert_eq!(
            reference.entity_type,
            Some(EntityType::Known(KnownEntityType::Person))
        );
        assert_eq!(reference.name, Some("Jane Smith".to_string()));
        assert_eq!(reference.role, Some("author".to_string()));
    }

    #[test]
    fn entity_reference_new_has_no_optional_fields_set() {
        let reference = EntityReference::new("urn:mif:entity:file:readme".to_string());
        assert!(reference.entity_type.is_none());
        assert!(reference.name.is_none());
        assert!(reference.role.is_none());
    }

    #[test]
    fn entity_data_new_has_no_optional_fields_or_extras() {
        let data = EntityData::new("Jane Smith".to_string());
        assert_eq!(data.name, "Jane Smith");
        assert!(data.entity_type.is_none());
        assert!(data.entity_id.is_none());
        assert!(data.extra.is_empty());
    }

    #[test]
    fn known_entity_type_covers_every_closed_variant() {
        for (variant, label) in [
            (KnownEntityType::Person, "Person"),
            (KnownEntityType::Organization, "Organization"),
            (KnownEntityType::Technology, "Technology"),
            (KnownEntityType::Concept, "Concept"),
            (KnownEntityType::File, "File"),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{label}\""));
        }
    }
}
