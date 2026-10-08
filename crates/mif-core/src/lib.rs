//! Shared foundational types for the MIF (Modeled Information Format) ecosystem.
//!
//! `mif-core` provides the types shared across the ecosystem's other crates:
//! [`OntologyReference`], [`EntityReference`], [`EntityData`], and
//! [`ConceptType`], plus the concept-URN helpers ([`concept_urn`],
//! [`is_concept_urn`]) for the `urn:mif:<uuid>` id form MIF 1.4.0 requires. Field definitions are taken directly from the canonical
//! MIF JSON Schema (`mif.schema.json`, `entity-reference.schema.json`,
//! draft 2020-12; see <https://mif-spec.dev/schema/>).
//!
//! Validation of MIF documents against the schema itself lives in the
//! `mif-schema` crate; `mif-core` only defines the shared data shapes.

mod concept;
mod entity;
mod ontology;
mod urn;

pub use concept::ConceptType;
pub use entity::{EntityData, EntityId, EntityReference, EntityType, KnownEntityType};
pub use ontology::OntologyReference;
pub use urn::{CONCEPT_URN_PREFIX, concept_urn, concept_uuid, is_concept_urn, mif_namespace};
