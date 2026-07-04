//! rht's harness configuration (`harness.config.json`): topic-to-ontology
//! bindings.

use std::path::Path;

use serde::Deserialize;

use crate::error::MifRhError;

/// One topic's configuration: its id and the ontologies it directly binds.
///
/// Each binding is either a bare id (`"edu-fixture"`) or a version-pinned
/// id (`"edu-fixture@0.1.0"`); [`HarnessConfig::topic_bindings`] parses the
/// pin out of the string, it is not a separate field here.
#[derive(Debug, Clone, Deserialize)]
pub struct TopicConfig {
    /// The topic's id.
    pub id: String,
    /// Directly bound ontology ids, optionally version-pinned.
    #[serde(default)]
    pub ontologies: Vec<String>,
}

/// rht's harness configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct HarnessConfig {
    /// Every configured topic.
    #[serde(default)]
    pub topics: Vec<TopicConfig>,
}

/// A parsed topic ontology binding: a bare ontology id, and the version it
/// pins, if any (`"edu-fixture@0.1.0"` -> `("edu-fixture", Some("0.1.0"))`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicBinding {
    /// The bound ontology's id.
    pub id: String,
    /// The pinned version, if the binding string included one.
    pub pinned_version: Option<String>,
}

impl HarnessConfig {
    /// Reads and parses the config file at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::ConfigMissing`] if `path` does not exist, or
    /// [`MifRhError::Json`] if it exists but is not valid JSON.
    pub fn load(path: &Path) -> Result<Self, MifRhError> {
        if !path.exists() {
            return Err(MifRhError::ConfigMissing {
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

    /// The parsed direct ontology bindings for `topic`. Empty (not an
    /// error) if the topic is not configured — matching a `bare`-style
    /// topic that only ever resolves core ontologies.
    #[must_use]
    pub fn topic_bindings(&self, topic: &str) -> Vec<TopicBinding> {
        self.topics
            .iter()
            .find(|t| t.id == topic)
            .map(|t| t.ontologies.iter().map(|s| parse_binding(s)).collect())
            .unwrap_or_default()
    }
}

fn parse_binding(binding: &str) -> TopicBinding {
    binding.split_once('@').map_or_else(
        || TopicBinding {
            id: binding.to_string(),
            pinned_version: None,
        },
        |(id, version)| TopicBinding {
            id: id.to_string(),
            pinned_version: Some(version.to_string()),
        },
    )
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::{HarnessConfig, TopicBinding};

    #[test]
    fn parses_bare_and_version_pinned_bindings() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(
            br#"{"topics":[
                {"id":"edu","ontologies":["edu-fixture"]},
                {"id":"eng","ontologies":["software-engineering@0.5.0"]},
                {"id":"bare","ontologies":[]}
            ]}"#,
        )
        .unwrap();

        let config = HarnessConfig::load(file.path()).unwrap();
        assert_eq!(
            config.topic_bindings("edu"),
            [TopicBinding {
                id: "edu-fixture".to_string(),
                pinned_version: None
            }]
        );
        assert_eq!(
            config.topic_bindings("eng"),
            [TopicBinding {
                id: "software-engineering".to_string(),
                pinned_version: Some("0.5.0".to_string())
            }]
        );
        assert_eq!(config.topic_bindings("bare"), []);
    }

    #[test]
    fn unconfigured_topic_binds_nothing_rather_than_erroring() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(br#"{"topics":[]}"#).unwrap();
        let config = HarnessConfig::load(file.path()).unwrap();
        assert_eq!(config.topic_bindings("unknown"), []);
    }

    #[test]
    fn reports_missing_config() {
        let error = HarnessConfig::load(std::path::Path::new("/nonexistent/harness.config.json"))
            .unwrap_err();
        assert!(matches!(error, super::MifRhError::ConfigMissing { .. }));
    }
}
