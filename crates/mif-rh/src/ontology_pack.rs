//! Full ontology-pack YAML parsing: `entity_types`, `discovery`, and the
//! `extends` chain, beyond what `mif_ontology::OntologyMetadata` covers.
//!
//! `mif-ontology`'s [`mif_ontology::OntologyMetadata`] deliberately parses
//! only `ontology.id`/`version`/`description`/`extends` — this module reads
//! the same files a second time for the richer `entity_types[]`/`discovery`
//! content `resolve()`/`suggest_type` need. The entity-type shape itself is
//! [`mif_ontology::EntityType`], the shared MIF-level model (including the
//! v1.1 `aliases`/`exemplars`/`negative_examples` classification fields and
//! its `embedding_doc()` composition rule), not a pack-local duplicate.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

pub use mif_ontology::EntityType;

use crate::error::MifRhError;

/// One discovery pattern: a regex tested against a finding's content, and
/// what it suggests if it matches.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryPattern {
    /// The content regex to test, case-insensitively. A pattern entry may
    /// instead (or additionally) carry `file_pattern` — a glob matched
    /// against a finding's file path rather than its content — which this
    /// struct still parses (so real ontology packs that mix both, like
    /// rht's own `software-engineering` pack, load without error) but
    /// `resolve()` never acts on: classification only ever inspects a
    /// finding's content, never a file path.
    #[serde(default)]
    pub content_pattern: Option<String>,
    /// A glob matched against a finding's file path, parsed but
    /// deliberately unused (see `content_pattern`'s doc comment).
    #[serde(default)]
    pub file_pattern: Option<String>,
    /// The entity type this pattern suggests, if it matches.
    #[serde(default)]
    pub suggest_entity: Option<String>,
}

/// An ontology's discovery-fallback configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscoveryConfig {
    /// Whether discovery classification is enabled for this ontology.
    #[serde(default)]
    pub enabled: bool,
    /// The patterns to test, in declaration order.
    #[serde(default)]
    pub patterns: Vec<DiscoveryPattern>,
}

#[derive(Debug, Deserialize)]
struct OntologyBlock {
    id: String,
    version: String,
    #[serde(default)]
    extends: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OntologyPackFile {
    ontology: OntologyBlock,
    #[serde(default)]
    entity_types: Vec<EntityType>,
    #[serde(default)]
    discovery: DiscoveryConfig,
}

/// A fully-parsed ontology pack: identity, `extends` chain, entity types,
/// and discovery configuration.
#[derive(Debug, Clone)]
pub struct OntologyPack {
    /// The ontology's id.
    pub id: String,
    /// The ontology's version.
    pub version: String,
    /// Ontology ids this ontology directly extends.
    pub extends: Vec<String>,
    /// Entity types this ontology declares.
    pub entity_types: Vec<EntityType>,
    /// Discovery-fallback configuration.
    pub discovery: DiscoveryConfig,
}

/// Parses one ontology pack YAML document.
///
/// # Errors
///
/// Returns [`MifRhError::OntologyPackYaml`] if `yaml` is not valid YAML or
/// does not match the expected ontology pack shape.
pub fn parse_pack(yaml: &str, path: &str) -> Result<OntologyPack, MifRhError> {
    let file: OntologyPackFile =
        serde_norway::from_str(yaml).map_err(|source| MifRhError::OntologyPackYaml {
            path: path.to_string(),
            source,
        })?;
    Ok(OntologyPack {
        id: file.ontology.id,
        version: file.ontology.version,
        extends: file.ontology.extends,
        entity_types: file.entity_types,
        discovery: file.discovery,
    })
}

/// Loads every `*.yaml`/`*.yml` ontology pack directly under `dir`
/// (non-recursive), keyed by ontology id.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `dir` cannot be read, or
/// [`MifRhError::OntologyPackYaml`] for a malformed pack within it.
pub fn load_packs_from_dir(dir: &Path) -> Result<HashMap<String, OntologyPack>, MifRhError> {
    let entries = fs::read_dir(dir).map_err(|source| MifRhError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let mut packs = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| MifRhError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let is_yaml = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext == "yaml" || ext == "yml");
        if !is_yaml {
            continue;
        }
        let path_display = path.display().to_string();
        let contents = fs::read_to_string(&path).map_err(|source| MifRhError::Io {
            path: path_display.clone(),
            source,
        })?;
        let pack = parse_pack(&contents, &path_display)?;
        packs.insert(pack.id.clone(), pack);
    }
    Ok(packs)
}

/// Loads every cataloged ontology's pack via its own catalog `source` path
/// (resolved relative to `base_dir`), keyed by id.
///
/// This is the faithful equivalent of rht's own mechanism: `resolve-ontology.sh`
/// and `ontology-review.sh` never scan one flat directory — each cataloged
/// ontology names its own YAML file's repo-relative path in
/// `.claude/enabled-packs.json`'s `source` field (core ontologies typically
/// under `schemas/ontologies/`, domain packs vendored under
/// `packs/ontologies/`, per rht's ADR-0012, "On-demand ontology vendoring
/// from a canonical registry" — not this workspace's ADR-0012, which is
/// unrelated). A catalog entry with no `source` is skipped, not an error —
/// matching how a hand-authored catalog fixture may omit it for a pack
/// resolved another way.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if a named source file cannot be read, or
/// [`MifRhError::OntologyPackYaml`] if it is malformed.
pub fn load_packs_via_catalog(
    catalog: &crate::catalog::Catalog,
    base_dir: &Path,
) -> Result<HashMap<String, OntologyPack>, MifRhError> {
    let mut packs = HashMap::new();
    for entry in &catalog.ontologies {
        let Some(source) = &entry.source else {
            continue;
        };
        let path = base_dir.join(source);
        let path_display = path.display().to_string();
        let contents = fs::read_to_string(&path).map_err(|source| MifRhError::Io {
            path: path_display.clone(),
            source,
        })?;
        let pack = parse_pack(&contents, &path_display)?;
        packs.insert(pack.id.clone(), pack);
    }
    Ok(packs)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{load_packs_from_dir, load_packs_via_catalog, parse_pack};
    use crate::catalog::{Catalog, CatalogEntry};

    const EDU_FIXTURE_YAML: &str = "
ontology:
  id: edu-fixture
  version: \"0.1.0\"
  extends: [mif-base]
entity_types:
  - name: title
    description: A published educational title
    schema:
      required: [name, isbn]
      properties:
        name: {type: string}
        isbn: {type: string}
discovery:
  enabled: true
  patterns:
    - content_pattern: \"\\\\b(ISBN|textbook)\\\\b\"
      suggest_entity: title
";

    #[test]
    fn parses_entity_types_and_discovery() {
        let pack = parse_pack(EDU_FIXTURE_YAML, "edu-fixture.yaml").unwrap();
        assert_eq!(pack.id, "edu-fixture");
        assert_eq!(pack.extends, ["mif-base"]);
        assert_eq!(pack.entity_types.len(), 1);
        assert_eq!(pack.entity_types[0].name, "title");
        assert!(pack.discovery.enabled);
        assert_eq!(pack.discovery.patterns.len(), 1);
        assert_eq!(
            pack.discovery.patterns[0].suggest_entity.as_deref(),
            Some("title")
        );
    }

    #[test]
    fn load_packs_via_catalog_resolves_each_entries_source_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("packs")).unwrap();
        fs::write(
            dir.path().join("packs").join("edu-fixture.yaml"),
            EDU_FIXTURE_YAML,
        )
        .unwrap();

        let catalog = Catalog {
            ontologies: vec![
                CatalogEntry {
                    id: "edu-fixture".to_string(),
                    version: "0.1.0".to_string(),
                    source: Some("packs/edu-fixture.yaml".to_string()),
                    core: false,
                },
                CatalogEntry {
                    id: "no-source".to_string(),
                    version: "1.0.0".to_string(),
                    source: None,
                    core: false,
                },
            ],
        };

        let packs = load_packs_via_catalog(&catalog, dir.path()).unwrap();
        assert_eq!(packs.len(), 1);
        assert!(packs.contains_key("edu-fixture"));
        assert!(!packs.contains_key("no-source"));
    }

    #[test]
    fn load_packs_from_dir_skips_non_yaml_and_keys_by_id() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), b"not a pack").unwrap();
        fs::write(dir.path().join("edu-fixture.yaml"), EDU_FIXTURE_YAML).unwrap();

        let packs = load_packs_from_dir(dir.path()).unwrap();
        assert_eq!(packs.len(), 1);
        assert!(packs.contains_key("edu-fixture"));
    }

    #[test]
    fn reports_malformed_yaml() {
        let error = parse_pack("{", "broken.yaml").unwrap_err();
        assert!(matches!(error, super::MifRhError::OntologyPackYaml { .. }));
    }
}
