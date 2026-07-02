//! Shared foundational types for the MIF (Modeled Information Format) ecosystem.
//!
//! `mif-core` provides the types shared across the ecosystem's other crates:
//! [`OntologyReference`], [`EntityReference`], [`EntityData`], and
//! [`ConceptType`]. Field definitions are taken directly from the canonical
//! MIF JSON Schema (`mif.schema.json`, `entity-reference.schema.json`,
//! draft 2020-12; see <https://mif-spec.dev/schema/>).
//!
//! Validation of MIF documents against the schema itself lives in the
//! `mif-schema` crate; `mif-core` only defines the shared data shapes.

mod concept;
mod entity;
mod ontology;

pub use concept::ConceptType;
pub use entity::{EntityData, EntityId, EntityReference, EntityType, KnownEntityType};
pub use ontology::OntologyReference;
