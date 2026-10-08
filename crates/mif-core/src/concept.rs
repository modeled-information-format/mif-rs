use serde::{Deserialize, Serialize};

/// MIF's three-way knowledge taxonomy.
///
/// Shared by the `conceptType` field (current) and the `memoryType` field
/// (deprecated v0.1 alias, retained for backward compatibility). A MIF
/// document must carry one of the two; since MIF 1.4.0 a document with only
/// `memoryType` satisfies the schema's type requirement. See the MIF schema
/// (`mif.schema.json`) for the authoritative definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConceptType {
    /// Declarative knowledge.
    Semantic,
    /// Time-bound records.
    Episodic,
    /// How-to knowledge.
    Procedural,
}

#[cfg(test)]
mod tests {
    use super::ConceptType;

    #[test]
    fn round_trips_through_json() {
        for value in [
            ConceptType::Semantic,
            ConceptType::Episodic,
            ConceptType::Procedural,
        ] {
            let json = serde_json::to_string(&value).unwrap();
            let parsed: ConceptType = serde_json::from_str(&json).unwrap();
            assert_eq!(value, parsed);
        }
    }

    #[test]
    fn serializes_lowercase() {
        let json = serde_json::to_string(&ConceptType::Episodic).unwrap();
        assert_eq!(json, "\"episodic\"");
    }
}
