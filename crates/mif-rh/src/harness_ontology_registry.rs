//! Whole-registry ontology reference integrity (rht Category B, Story #287).
//!
//! Ports the registry-wide scans embedded directly in `scripts/verify.sh`'s
//! `gate_m20` ("cross-pack relationship reference integrity") and
//! `gate_m22` ("entity-type subsumption"): every relationship's `from`/`to`
//! endpoint and every `subtype_of` parent, across EVERY ontology in the
//! registry (not just one topic's bound catalog — `@id`/entity-type names
//! are globally unique, so a cross-pack reference like software-security's
//! `governs` naming engineering-base's `component` must resolve against the
//! whole registry, which `validate_concordance`'s per-topic bound set
//! cannot see).

use std::path::{Path, PathBuf};

use crate::ontology_pack::{OntologyPack, parse_pack};

/// A relationship endpoint or `subtype_of` parent naming no declared entity
/// type anywhere in the registry.
#[derive(Debug, Clone)]
pub struct RegistryValidation {
    /// Relationship `from`/`to` endpoint type names with no declared type
    /// in the registry (excluding the `"*"` wildcard, which means "any
    /// declared type").
    pub relationship_endpoint_orphans: Vec<String>,
    /// `subtype_of` parent type names with no declared type in the
    /// registry.
    pub subtype_of_orphans: Vec<String>,
    /// The number of distinct declared entity type names across the whole
    /// registry.
    pub registry_type_count: usize,
}

impl RegistryValidation {
    /// Whether every relationship endpoint and `subtype_of` parent
    /// resolves to a declared registry type.
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.relationship_endpoint_orphans.is_empty() && self.subtype_of_orphans.is_empty()
    }
}

/// Every `*.yaml` file directly under one level of `dir` (`dir/*/*.yaml`),
/// sorted for determinism.
fn glob_one_level(dir: &Path, suffix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.is_dir())
        .filter_map(|subdir| {
            std::fs::read_dir(&subdir).ok().map(|inner| {
                inner
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|p| {
                        p.is_file()
                            && p.file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n.ends_with(suffix))
                    })
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect();
    paths.sort();
    paths
}

fn load_packs(paths: &[PathBuf]) -> Vec<OntologyPack> {
    paths
        .iter()
        .filter_map(|path| {
            let contents = std::fs::read_to_string(path).ok()?;
            parse_pack(&contents, &path.display().to_string()).ok()
        })
        .collect()
}

/// Validates the whole ontology registry under `root`.
///
/// Scans `schemas/ontologies/*/*.yaml` plus `packs/ontologies/*/*.ontology.yaml`:
/// every relationship's `from`/`to` endpoint (from `packs/ontologies/*/*.ontology.yaml`
/// packs only, matching the original scan) and every `subtype_of` parent
/// (across the whole registry) must name a type declared somewhere in the
/// registry.
///
/// A pack file that fails to parse is silently skipped, matching the
/// original script's `yq ... 2>/dev/null` — this is a whole-registry
/// consistency scan over committed files, not a per-corpus fail-closed
/// gate.
#[must_use]
pub fn validate_ontology_registry(root: &Path) -> RegistryValidation {
    let core_layers = load_packs(&glob_one_level(&root.join("schemas/ontologies"), ".yaml"));
    let domain_packs = load_packs(&glob_one_level(
        &root.join("packs/ontologies"),
        ".ontology.yaml",
    ));

    let mut registry_types: Vec<String> = core_layers
        .iter()
        .chain(&domain_packs)
        .flat_map(|pack| pack.entity_types.iter().map(|et| et.name.clone()))
        .collect();
    registry_types.sort();
    registry_types.dedup();

    let mut subtype_of_orphans: Vec<String> = core_layers
        .iter()
        .chain(&domain_packs)
        .flat_map(|pack| {
            pack.entity_types
                .iter()
                .flat_map(|et| et.subtype_of.iter().cloned())
        })
        .filter(|parent| !registry_types.contains(parent))
        .collect();
    subtype_of_orphans.sort();
    subtype_of_orphans.dedup();

    // Endpoint scan is scoped to packs/ontologies/*.ontology.yaml only,
    // matching the original script (schemas/ontologies core layers declare
    // no relationships of their own in practice, but the scope is
    // deliberate, not incidental — keep it exact).
    let mut relationship_endpoint_orphans: Vec<String> = domain_packs
        .iter()
        .flat_map(|pack| pack.relationships.values())
        .flat_map(|rel| rel.from.iter().chain(rel.to.iter()))
        .filter(|endpoint| endpoint.as_str() != "*")
        .filter(|endpoint| !registry_types.contains(*endpoint))
        .cloned()
        .collect();
    relationship_endpoint_orphans.sort();
    relationship_endpoint_orphans.dedup();

    RegistryValidation {
        relationship_endpoint_orphans,
        subtype_of_orphans,
        registry_type_count: registry_types.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_ontology_registry;
    use std::fs;

    fn write(dir: &std::path::Path, rel: &str, contents: &str) {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn passes_a_fully_resolved_registry() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "schemas/ontologies/engineering-base/0.1.0.yaml",
            "
ontology:
  id: engineering-base
  version: \"0.1.0\"
entity_types:
  - name: control
  - name: component
relationships:
  governs:
    from: [control]
    to: [component]
",
        );
        write(
            dir.path(),
            "packs/ontologies/software-security/software-security.ontology.yaml",
            "
ontology:
  id: software-security
  version: \"0.1.0\"
entity_types:
  - name: security-control
    subtype_of: [control]
",
        );

        let result = validate_ontology_registry(dir.path());
        assert!(result.ok(), "{result:?}");
        assert_eq!(result.registry_type_count, 3);
    }

    #[test]
    fn flags_a_relationship_endpoint_naming_no_registry_type() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packs/ontologies/x/x.ontology.yaml",
            "
ontology:
  id: x
  version: \"0.1.0\"
entity_types:
  - name: a
relationships:
  rel:
    from: [a]
    to: [does-not-exist]
",
        );

        let result = validate_ontology_registry(dir.path());
        assert!(!result.ok());
        assert_eq!(result.relationship_endpoint_orphans, vec!["does-not-exist"]);
        assert!(result.subtype_of_orphans.is_empty());
    }

    #[test]
    fn a_wildcard_relationship_endpoint_is_never_an_orphan() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packs/ontologies/x/x.ontology.yaml",
            "
ontology:
  id: x
  version: \"0.1.0\"
entity_types:
  - name: a
relationships:
  relates-to:
    from: [a]
    to: [\"*\"]
",
        );

        let result = validate_ontology_registry(dir.path());
        assert!(result.ok(), "{result:?}");
    }

    #[test]
    fn flags_a_subtype_of_parent_naming_no_registry_type() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "packs/ontologies/x/x.ontology.yaml",
            "
ontology:
  id: x
  version: \"0.1.0\"
entity_types:
  - name: a
    subtype_of: [ghost-parent]
",
        );

        let result = validate_ontology_registry(dir.path());
        assert!(!result.ok());
        assert_eq!(result.subtype_of_orphans, vec!["ghost-parent"]);
        assert!(result.relationship_endpoint_orphans.is_empty());
    }

    #[test]
    fn a_relationship_endpoint_may_resolve_against_a_different_pack_cross_pack() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "schemas/ontologies/engineering-base/0.1.0.yaml",
            "
ontology:
  id: engineering-base
  version: \"0.1.0\"
entity_types:
  - name: component
",
        );
        write(
            dir.path(),
            "packs/ontologies/software-security/software-security.ontology.yaml",
            "
ontology:
  id: software-security
  version: \"0.1.0\"
entity_types:
  - name: security-control
relationships:
  governs:
    from: [security-control]
    to: [component]
",
        );

        let result = validate_ontology_registry(dir.path());
        assert!(result.ok(), "{result:?}");
    }
}
