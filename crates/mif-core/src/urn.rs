//! Concept URNs (`urn:mif:<uuid>`, MIF spec §6.1).
//!
//! MIF 1.4.0 requires a concept `@id` to be `urn:mif:<uuid>`; the
//! `mif.schema.json` pattern rejects slug or structured suffixes such as
//! `urn:mif:my-note` or `urn:mif:source:<ns>:<slug>`. Producers that key
//! concepts by a stable name derive the UUID with [`concept_uuid`], a version-5 UUID
//! in the same namespace the MIF repo's `scripts/migrate_0_1_to_1_0.py`
//! uses, so the same name always yields the same URN across tools.

use uuid::Uuid;

/// The URN prefix of a concept `@id`.
pub const CONCEPT_URN_PREFIX: &str = "urn:mif:";

/// The version-5 UUID namespace for derived concept ids:
/// `uuid5(NAMESPACE_URL, "https://mif-spec.dev")`.
#[must_use]
pub fn mif_namespace() -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, b"https://mif-spec.dev")
}

/// The deterministic concept UUID for `name` (version 5, in [`mif_namespace`]).
#[must_use]
pub fn concept_uuid(name: &str) -> Uuid {
    Uuid::new_v5(&mif_namespace(), name.as_bytes())
}

/// The concept URN `urn:mif:<uuid>` for `name`.
///
/// ```
/// let urn = mif_core::concept_urn("source:topic:paper");
/// assert!(mif_core::is_concept_urn(&urn));
/// assert_eq!(urn, mif_core::concept_urn("source:topic:paper"));
/// ```
#[must_use]
pub fn concept_urn(name: &str) -> String {
    format!("{CONCEPT_URN_PREFIX}{}", concept_uuid(name).hyphenated())
}

/// Whether `id` is a valid concept URN: `urn:mif:` followed by a hyphenated
/// UUID, the form `mif.schema.json`'s `@id` pattern accepts.
#[must_use]
pub fn is_concept_urn(id: &str) -> bool {
    id.strip_prefix(CONCEPT_URN_PREFIX)
        .is_some_and(|rest| rest.len() == 36 && Uuid::try_parse(rest).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_matches_mif_migration_script() {
        // python3 -c 'import uuid;print(uuid.uuid5(uuid.NAMESPACE_URL,"https://mif-spec.dev"))'
        assert_eq!(
            mif_namespace().to_string(),
            "2d637db4-b33e-5a8f-9663-b8dff8cf358b"
        );
    }

    #[test]
    fn concept_urn_matches_python_uuid5() {
        // python3 -c 'import uuid;n=uuid.uuid5(uuid.NAMESPACE_URL,"https://mif-spec.dev");print(uuid.uuid5(n,"memory:test-001"))'
        assert_eq!(
            concept_urn("memory:test-001"),
            "urn:mif:89886348-4773-5732-88b5-84366da76468"
        );
    }

    #[test]
    fn is_concept_urn_accepts_uuid_and_rejects_slugs() {
        assert!(is_concept_urn(
            "urn:mif:550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_concept_urn("urn:mif:my-note"));
        assert!(!is_concept_urn("urn:mif:550e8400"));
        assert!(!is_concept_urn(
            "urn:mif:entity:550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(!is_concept_urn("550e8400-e29b-41d4-a716-446655440000"));
        // Simple (unhyphenated) form: valid UUID, but not the schema's pattern.
        assert!(!is_concept_urn("urn:mif:550e8400e29b41d4a716446655440000"));
    }
}
