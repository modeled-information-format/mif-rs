//! Ontology resolution for the MIF (Modeled Information Format) ecosystem.
//!
//! Resolves the three-tier ontology inheritance chain (`mif-base` ->
//! `shared-traits` -> domain ontologies), driven by each ontology
//! definition's own `extends` list (see `schema/ontology/ontology.schema.json`
//! and ADR-004, "Three-Tier Trait Inheritance"). Ontology content itself is
//! not vendored here — it is loaded from a corpus of ontology definition
//! YAML files supplied by the caller (e.g. a local checkout of the
//! `ontologies` repository), matching the convention already used by the
//! `MIF` repository's own `validate-ontologies.py --path`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::BuildHasher;
use std::path::Path;

use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};
use serde::Deserialize;

/// Metadata for one ontology definition: identifier, version, and the
/// `extends` chain used for three-tier resolution.
///
/// This deliberately covers only `ontology.schema.json`'s `ontology` block
/// (id/version/description/extends) — not the richer namespace/entity-type/
/// trait/relationship/discovery content an ontology definition may also
/// carry, which is out of scope for resolution.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OntologyMetadata {
    /// Unique identifier for the ontology (`^[a-z][a-z0-9-]*$`).
    pub id: String,
    /// Semantic version string.
    pub version: String,
    /// Human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// List of ontology IDs this ontology extends (inherits traits and
    /// relationships from). Empty for a tier-1 base ontology.
    #[serde(default)]
    pub extends: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OntologyDefinitionFile {
    ontology: OntologyMetadata,
}

/// Errors from loading or resolving ontology definitions.
#[derive(Debug, thiserror::Error)]
pub enum OntologyError {
    /// Failed to read an ontology definition file.
    #[error("failed to read {path}: {source}")]
    Io {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The ontology definition file was not valid YAML.
    #[error("failed to parse {path} as YAML: {source}")]
    Yaml {
        /// The path that failed to parse.
        path: String,
        /// The underlying parse error.
        #[source]
        source: serde_norway::Error,
    },
    /// The ontology definition failed schema validation.
    #[error("{path} failed ontology schema validation: {source}")]
    Invalid {
        /// The path that failed validation.
        path: String,
        /// The underlying schema validation error.
        #[source]
        source: mif_schema::MifSchemaError,
    },
    /// The ontology definition passed schema validation but did not match
    /// the expected metadata shape. Indicates a bug in this crate or a gap
    /// between `ontology.schema.json` and [`OntologyMetadata`].
    #[error("failed to extract ontology metadata from {path}: {source}")]
    Deserialize {
        /// The path that failed extraction.
        path: String,
        /// The underlying deserialization error.
        #[source]
        source: serde_json::Error,
    },
    /// The requested ontology ID was not found in the supplied corpus.
    #[error("ontology '{0}' not found in the supplied corpus")]
    NotFound(String),
    /// The `extends` graph contains a cycle.
    #[error("ontology '{0}' is part of an extends cycle")]
    Cycle(String),
}

impl OntologyError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::Io { .. } => ProblemMeta {
                slug: "io",
                version: "v1",
                title: "Failed to read an ontology definition file",
                status: 500,
                exit_code: 1,
            },
            Self::Yaml { .. } => ProblemMeta {
                slug: "invalid-yaml",
                version: "v1",
                title: "Malformed ontology definition YAML",
                status: 422,
                exit_code: 2,
            },
            Self::Invalid { .. } => ProblemMeta {
                slug: "invalid-ontology-definition",
                version: "v1",
                title: "Ontology definition failed schema validation",
                status: 422,
                exit_code: 2,
            },
            Self::Deserialize { .. } => ProblemMeta {
                slug: "ontology-metadata-mismatch",
                version: "v1",
                title: "Internal error extracting ontology metadata",
                status: 500,
                exit_code: 1,
            },
            Self::NotFound(_) => ProblemMeta {
                slug: "ontology-not-found",
                version: "v1",
                title: "Ontology not found in the supplied corpus",
                status: 404,
                exit_code: 3,
            },
            Self::Cycle(_) => ProblemMeta {
                slug: "ontology-extends-cycle",
                version: "v1",
                title: "Ontology extends graph contains a cycle",
                status: 422,
                exit_code: 4,
            },
        }
    }
}

impl ToProblem for OntologyError {
    fn to_problem(&self) -> ProblemDetails {
        let mut problem = self
            .meta()
            .into_details(env!("CARGO_PKG_NAME"), self.to_string());
        let (fix, action) = match self {
            Self::Io { source, .. } => {
                let (status, fix, action) = mif_problem::classify_io_error(source);
                problem.status = status;
                (fix, action)
            },
            Self::Yaml { .. } => (
                SuggestedFix::new(
                    "Fix the YAML syntax error in the ontology definition file, then retry.",
                    Applicability::MaybeIncorrect,
                ),
                CodeAction::new(
                    "Fix the malformed YAML",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            ),
            Self::Invalid { .. } => (
                SuggestedFix::new(
                    "Correct the ontology definition so it conforms to ontology.schema.json, then retry.",
                    Applicability::MaybeIncorrect,
                ),
                CodeAction::new(
                    "Fix the reported schema violations",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            ),
            Self::Deserialize { .. } => (
                SuggestedFix::new(
                    "This indicates a bug in mif-ontology (a gap between ontology.schema.json \
                     and OntologyMetadata). Report it upstream.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "File a bug against mif-ontology",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            ),
            Self::NotFound(_) => (
                SuggestedFix::new(
                    "Add the missing ontology definition to the supplied corpus directory, or \
                     correct the requested ontology ID, then retry.",
                    Applicability::MaybeIncorrect,
                ),
                CodeAction::new(
                    "Supply the missing ontology definition",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            ),
            Self::Cycle(_) => (
                SuggestedFix::new(
                    "Break the extends cycle by removing or redirecting one of the offending \
                     ontology definitions' extends entries.",
                    Applicability::MaybeIncorrect,
                ),
                CodeAction::new(
                    "Remove the cyclic extends reference",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            ),
        };
        problem.with_suggested_fix(fix).with_code_action(action)
    }
}

/// Parses and validates one ontology definition document (already-loaded
/// YAML text) against `ontology.schema.json`, returning its metadata.
///
/// # Errors
///
/// Returns [`OntologyError::Yaml`] if `yaml` is not valid YAML, or
/// [`OntologyError::Invalid`] if it does not conform to the ontology schema.
pub fn parse_definition(yaml: &str, path: &str) -> Result<OntologyMetadata, OntologyError> {
    let value: serde_json::Value =
        serde_norway::from_str(yaml).map_err(|source| OntologyError::Yaml {
            path: path.to_string(),
            source,
        })?;
    mif_schema::validate_ontology_definition(&value).map_err(|source| OntologyError::Invalid {
        path: path.to_string(),
        source,
    })?;
    let definition: OntologyDefinitionFile =
        serde_json::from_value(value).map_err(|source| OntologyError::Deserialize {
            path: path.to_string(),
            source,
        })?;
    Ok(definition.ontology)
}

/// Loads every `*.yaml`/`*.yml` ontology definition file directly under
/// `dir` (non-recursive) into a corpus keyed by ontology ID, suitable for
/// passing to [`resolve_chain`].
///
/// # Errors
///
/// Returns [`OntologyError::Io`] if `dir` cannot be read, or any error from
/// [`parse_definition`] for a malformed file within it.
pub fn load_corpus_from_dir(
    dir: &Path,
) -> Result<HashMap<String, OntologyMetadata>, OntologyError> {
    let entries = fs::read_dir(dir).map_err(|source| OntologyError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let mut corpus = HashMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| OntologyError::Io {
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
        let contents = fs::read_to_string(&path).map_err(|source| OntologyError::Io {
            path: path_display.clone(),
            source,
        })?;
        let metadata = parse_definition(&contents, &path_display)?;
        corpus.insert(metadata.id.clone(), metadata);
    }
    Ok(corpus)
}

/// Resolves the three-tier `extends` chain for `id` against `corpus`.
///
/// Returns the ontologies in base-to-specific order: a tier-1 ontology with
/// no `extends` appears first, and `id`'s own metadata appears last.
///
/// # Errors
///
/// Returns [`OntologyError::NotFound`] if `id` or any ontology it
/// (transitively) extends is missing from `corpus`, or
/// [`OntologyError::Cycle`] if the `extends` graph is cyclic.
pub fn resolve_chain<S: BuildHasher>(
    id: &str,
    corpus: &HashMap<String, OntologyMetadata, S>,
) -> Result<Vec<OntologyMetadata>, OntologyError> {
    let mut chain = Vec::new();
    let mut visiting = HashSet::new();
    resolve_into(id, corpus, &mut visiting, &mut chain)?;
    Ok(chain)
}

fn resolve_into<S: BuildHasher>(
    id: &str,
    corpus: &HashMap<String, OntologyMetadata, S>,
    visiting: &mut HashSet<String>,
    chain: &mut Vec<OntologyMetadata>,
) -> Result<(), OntologyError> {
    if chain.iter().any(|resolved| resolved.id == id) {
        return Ok(());
    }
    if !visiting.insert(id.to_string()) {
        return Err(OntologyError::Cycle(id.to_string()));
    }
    let metadata = corpus
        .get(id)
        .cloned()
        .ok_or_else(|| OntologyError::NotFound(id.to_string()))?;
    for parent in &metadata.extends {
        resolve_into(parent, corpus, visiting, chain)?;
    }
    visiting.remove(id);
    chain.push(metadata);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use mif_problem::ToProblem;

    use super::{
        OntologyError, OntologyMetadata, load_corpus_from_dir, parse_definition, resolve_chain,
    };

    /// Builds a fresh, non-colliding scratch directory path under the OS temp
    /// dir for a single test's filesystem fixtures. Does not create it.
    fn unique_temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "mif-ontology-test-{}-{label}-{id}",
            std::process::id()
        ))
    }

    fn metadata(id: &str, extends: &[&str]) -> OntologyMetadata {
        OntologyMetadata {
            id: id.to_string(),
            version: "1.0.0".to_string(),
            description: None,
            extends: extends.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn parses_valid_definition() {
        let yaml = "ontology:\n  id: mif-base\n  version: 1.0.0\n";
        let parsed = parse_definition(yaml, "mif-base.yaml").unwrap();
        assert_eq!(parsed.id, "mif-base");
        assert!(parsed.extends.is_empty());
    }

    #[test]
    fn rejects_invalid_id_pattern() {
        let yaml = "ontology:\n  id: Not_Valid\n  version: 1.0.0\n";
        assert!(matches!(
            parse_definition(yaml, "bad.yaml"),
            Err(OntologyError::Invalid { .. })
        ));
    }

    #[test]
    fn resolves_three_tier_chain_base_to_specific() {
        let mut corpus = HashMap::new();
        corpus.insert("mif-base".to_string(), metadata("mif-base", &[]));
        corpus.insert(
            "shared-traits".to_string(),
            metadata("shared-traits", &["mif-base"]),
        );
        corpus.insert(
            "grazing-plan".to_string(),
            metadata("grazing-plan", &["shared-traits"]),
        );

        let chain = resolve_chain("grazing-plan", &corpus).unwrap();
        let ids: Vec<&str> = chain.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["mif-base", "shared-traits", "grazing-plan"]);
    }

    #[test]
    fn deduplicates_diamond_extends() {
        let mut corpus = HashMap::new();
        corpus.insert("mif-base".to_string(), metadata("mif-base", &[]));
        corpus.insert("a".to_string(), metadata("a", &["mif-base"]));
        corpus.insert("b".to_string(), metadata("b", &["mif-base"]));
        corpus.insert("domain".to_string(), metadata("domain", &["a", "b"]));

        let chain = resolve_chain("domain", &corpus).unwrap();
        let ids: Vec<&str> = chain.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["mif-base", "a", "b", "domain"]);
    }

    #[test]
    fn reports_missing_ontology() {
        let corpus = HashMap::new();
        assert!(matches!(
            resolve_chain("missing", &corpus),
            Err(OntologyError::NotFound(id)) if id == "missing"
        ));
    }

    #[test]
    fn detects_extends_cycle() {
        let mut corpus = HashMap::new();
        corpus.insert("a".to_string(), metadata("a", &["b"]));
        corpus.insert("b".to_string(), metadata("b", &["a"]));

        assert!(matches!(
            resolve_chain("a", &corpus),
            Err(OntologyError::Cycle(_))
        ));
    }

    #[test]
    fn not_found_and_cycle_map_to_distinct_problem_types() {
        let not_found = OntologyError::NotFound("missing".to_string()).to_problem();
        let cycle = OntologyError::Cycle("a".to_string()).to_problem();

        assert_eq!(
            not_found.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/ontology-not-found/v1"
        );
        assert_eq!(not_found.status, 404);
        assert_eq!(not_found.exit_code, Some(3));

        assert_eq!(
            cycle.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/ontology-extends-cycle/v1"
        );
        assert_eq!(cycle.status, 422);
        assert_eq!(cycle.exit_code, Some(4));
        assert_ne!(not_found.problem_type, cycle.problem_type);
        assert!(not_found.suggested_fix.is_some());
        assert!(cycle.suggested_fix.is_some());
    }

    #[test]
    fn io_error_status_is_classified_by_the_underlying_error_kind() {
        let not_found = OntologyError::Io {
            path: "/nonexistent/ontologies".to_string(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }
        .to_problem();
        assert_eq!(not_found.status, 404);
        assert_eq!(
            not_found.suggested_fix.unwrap().applicability,
            mif_problem::Applicability::MaybeIncorrect
        );

        let generic_fault = OntologyError::Io {
            path: "/ontologies".to_string(),
            source: std::io::Error::from(std::io::ErrorKind::Other),
        }
        .to_problem();
        assert_eq!(generic_fault.status, 500);
        assert_eq!(
            generic_fault.suggested_fix.unwrap().applicability,
            mif_problem::Applicability::Unspecified
        );
    }

    #[test]
    fn malformed_yaml_syntax_returns_yaml_error_that_maps_to_422_problem() {
        let error = parse_definition("{", "broken.yaml").unwrap_err();
        assert!(matches!(error, OntologyError::Yaml { .. }));

        let problem = error.to_problem();
        assert_eq!(problem.status, 422);
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-yaml/v1"
        );
        assert_eq!(
            problem.suggested_fix.unwrap().applicability,
            mif_problem::Applicability::MaybeIncorrect
        );
    }

    #[test]
    fn invalid_definition_error_to_problem_reports_422_and_maybe_incorrect_fix() {
        let yaml = "ontology:\n  id: Not_Valid\n  version: 1.0.0\n";
        let error = parse_definition(yaml, "bad.yaml").unwrap_err();
        assert!(matches!(error, OntologyError::Invalid { .. }));

        let problem = error.to_problem();
        assert_eq!(problem.status, 422);
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-ontology-definition/v1"
        );
        assert_eq!(
            problem.suggested_fix.unwrap().applicability,
            mif_problem::Applicability::MaybeIncorrect
        );
    }

    #[test]
    fn definition_missing_ontology_key_returns_deserialize_error_that_maps_to_500_problem() {
        // The ontology schema does not mark the top-level `ontology` key as
        // required, so this document passes schema validation but then fails
        // to deserialize into `OntologyDefinitionFile`, exercising the
        // "schema/struct gap" defensive branch.
        let error = parse_definition("{}\n", "empty.yaml").unwrap_err();
        assert!(matches!(error, OntologyError::Deserialize { .. }));

        let problem = error.to_problem();
        assert_eq!(problem.status, 500);
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/ontology-metadata-mismatch/v1"
        );
        assert_eq!(
            problem.suggested_fix.unwrap().applicability,
            mif_problem::Applicability::Unspecified
        );
    }

    #[test]
    fn load_corpus_from_dir_reports_io_error_for_missing_directory() {
        let missing = unique_temp_dir("missing-dir");
        assert!(matches!(
            load_corpus_from_dir(&missing),
            Err(OntologyError::Io { .. })
        ));
    }

    #[test]
    fn load_corpus_from_dir_skips_non_yaml_files_and_loads_yaml_files() {
        let dir = unique_temp_dir("skip-non-yaml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("readme.txt"), b"not an ontology file").unwrap();
        fs::write(
            dir.join("mif-base.yaml"),
            b"ontology:\n  id: mif-base\n  version: 1.0.0\n",
        )
        .unwrap();

        let corpus = load_corpus_from_dir(&dir).unwrap();
        assert_eq!(corpus.len(), 1);
        assert!(corpus.contains_key("mif-base"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_corpus_from_dir_reports_io_error_when_yaml_entry_is_unreadable() {
        let dir = unique_temp_dir("unreadable-yaml");
        fs::create_dir_all(&dir).unwrap();
        // A directory whose name ends in .yaml passes the extension filter
        // but cannot be read as file contents, exercising the
        // fs::read_to_string IO-error path (distinct from the read_dir
        // IO-error path already covered above).
        fs::create_dir_all(dir.join("looks-like-a-file.yaml")).unwrap();

        assert!(matches!(
            load_corpus_from_dir(&dir),
            Err(OntologyError::Io { .. })
        ));

        fs::remove_dir_all(&dir).unwrap();
    }
}
