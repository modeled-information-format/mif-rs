//! rht's ontology catalog (`.claude/enabled-packs.json`).

use std::path::Path;

use serde::Deserialize;

use crate::error::MifRhError;

/// One cataloged ontology: its id, cataloged version, source path, and
/// whether it is a core ontology (implicitly allowed for every topic).
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    /// The ontology's id.
    pub id: String,
    /// The cataloged version.
    pub version: String,
    /// Repo-relative path to the ontology's YAML definition.
    #[serde(default)]
    pub source: Option<String>,
    /// Whether this ontology is implicitly allowed for every topic.
    #[serde(default)]
    pub core: bool,
}

/// rht's ontology catalog: which ontologies are enabled, and which are core.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    /// The cataloged ontologies.
    pub ontologies: Vec<CatalogEntry>,
}

impl Catalog {
    /// Reads and parses the catalog file at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::CatalogMissing`] if `path` does not exist, or
    /// [`MifRhError::Json`] if it exists but is not valid JSON.
    pub fn load(path: &Path) -> Result<Self, MifRhError> {
        if !path.exists() {
            return Err(MifRhError::CatalogMissing {
                path: path.display().to_string(),
            });
        }
        let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
            path: path.display().to_string(),
            source,
        })
    }

    /// Looks up a cataloged entry by id.
    #[must_use]
    pub fn find(&self, id: &str) -> Option<&CatalogEntry> {
        self.ontologies.iter().find(|entry| entry.id == id)
    }

    /// Every core ontology's id.
    pub fn core_ids(&self) -> impl Iterator<Item = &str> {
        self.ontologies
            .iter()
            .filter(|entry| entry.core)
            .map(|entry| entry.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::Catalog;

    #[test]
    fn loads_and_finds_entries() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"ontologies":[
                {"id":"mif-generic","version":"1.0.0","core":true},
                {"id":"edu-fixture","version":"0.1.0","source":"x.yaml"}
            ]}"#,
        )
        .unwrap();

        let catalog = Catalog::load(file.path()).unwrap();
        assert_eq!(catalog.core_ids().collect::<Vec<_>>(), ["mif-generic"]);
        assert_eq!(catalog.find("edu-fixture").unwrap().version, "0.1.0");
        assert!(catalog.find("missing").is_none());
    }

    #[test]
    fn reports_missing_catalog() {
        let error =
            Catalog::load(std::path::Path::new("/nonexistent/enabled-packs.json")).unwrap_err();
        assert!(matches!(error, super::MifRhError::CatalogMissing { .. }));
    }
}
