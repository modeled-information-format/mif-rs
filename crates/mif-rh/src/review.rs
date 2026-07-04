//! `review()`: rebuild `ontology-map.json` for one or more topics and
//! aggregate coverage, matching rht's `ontology-review.sh` exactly.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::catalog::Catalog;
use crate::config::HarnessConfig;
use crate::error::MifRhError;
use crate::finding::Finding;
use crate::ontology_pack::OntologyPack;
use crate::resolve::{Basis, MapRecord, ResolveContext, resolve_finding};

/// Options for one `review()` run.
pub struct ReviewOptions<'a> {
    /// Topics to review. `None` reviews every topic in `config`.
    pub topics: Option<&'a [String]>,
    /// Root `reports/` directory (`reports/<topic>/findings/`,
    /// `reports/<topic>/ontology-map.json`).
    pub reports_dir: &'a Path,
    /// Every loaded ontology pack, keyed by id. Loading strategy (a flat
    /// directory scan via [`crate::ontology_pack::load_packs_from_dir`], or
    /// rht's own catalog-`source`-driven layout via
    /// [`crate::ontology_pack::load_packs_via_catalog`]) is the caller's
    /// choice, not `review()`'s — it only classifies against whatever
    /// corpus it is given.
    pub ontology_packs: &'a HashMap<String, OntologyPack>,
    /// The enabled-ontologies catalog.
    pub catalog: &'a Catalog,
    /// The harness's topic-to-ontology bindings.
    pub config: &'a HarnessConfig,
    /// Optional path to rht's own `scripts/check-relationship-targets.sh`,
    /// run exactly once across the whole review (not per topic). `None`
    /// skips the check entirely — used when reviewing a corpus that has no
    /// rht scripts available (e.g. isolated parity tests).
    pub check_relationship_targets_script: Option<&'a Path>,
}

/// Per-topic coverage counts, matching `ontology-review.sh`'s own bucket
/// definitions exactly.
#[derive(Debug, Clone)]
pub struct TopicSummary {
    /// The topic's id.
    pub topic: String,
    /// The topic's directly bound ontology ids, for display.
    pub bound: Vec<String>,
    /// Total findings seen (including unprocessable ones).
    pub total: usize,
    /// `basis in {declared, resolved} && valid`.
    pub stamped: usize,
    /// `basis == discovery && valid`.
    pub discovery: usize,
    /// `basis == untyped` (valid is always true for untyped).
    pub untyped: usize,
    /// `!valid`, plus any finding that could not even be processed (a
    /// reconciliation "gap" — never silently dropped).
    pub bad: usize,
}

/// The result of one `review()` run.
#[derive(Debug, Clone)]
pub struct ReviewReport {
    /// Per-topic summaries, in review order.
    pub topics: Vec<TopicSummary>,
    /// Whether any topic had `bad > 0`, or the relationship-target check
    /// found orphans.
    pub any_bad: bool,
}

impl ReviewReport {
    /// Total findings across every reviewed topic.
    #[must_use]
    pub fn total_findings(&self) -> usize {
        self.topics.iter().map(|t| t.total).sum()
    }

    /// Total stamped findings across every reviewed topic.
    #[must_use]
    pub fn total_stamped(&self) -> usize {
        self.topics.iter().map(|t| t.stamped).sum()
    }

    /// Total discovery-only findings across every reviewed topic.
    #[must_use]
    pub fn total_discovery(&self) -> usize {
        self.topics.iter().map(|t| t.discovery).sum()
    }

    /// Total untyped findings across every reviewed topic.
    #[must_use]
    pub fn total_untyped(&self) -> usize {
        self.topics.iter().map(|t| t.untyped).sum()
    }

    /// Total invalid/unresolved findings across every reviewed topic.
    #[must_use]
    pub fn total_bad(&self) -> usize {
        self.topics.iter().map(|t| t.bad).sum()
    }

    /// The exact summary line format `ontology-review.sh` prints, e.g.
    /// `"2 topic(s); 4 findings — 1 stamped, 1 discovery-only, 1 untyped, 1 invalid/unresolved"`.
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "{} topic(s); {} findings — {} stamped, {} discovery-only, {} untyped, {} invalid/unresolved",
            self.topics.len(),
            self.total_findings(),
            self.total_stamped(),
            self.total_discovery(),
            self.total_untyped(),
            self.total_bad(),
        )
    }

    /// Whether `--strict` should fail this review: `any_bad` alone (never
    /// discovery-only/untyped findings by themselves).
    #[must_use]
    pub const fn strict_should_fail(&self) -> bool {
        self.any_bad
    }
}

/// One entry in a `--followup` backlog: a finding that is not durably
/// stamped, or could not be processed at all (`basis: "gap"`).
#[derive(Debug, Clone, Serialize)]
pub struct FollowupEntry {
    /// The finding's id (best-effort, from the file stem, for a `"gap"`
    /// entry whose file could not even be parsed).
    pub finding_id: String,
    /// The finding's file path, repo-relative if derivable.
    pub file: Option<String>,
    /// `"discovery"`, `"untyped"`, or `"gap"`.
    pub basis: String,
    /// The finding's entity type, if identified.
    pub entity_type: Option<String>,
    /// The resolved ontology, if any.
    pub resolved_ontology: Option<String>,
    /// Whether the finding validated (always `false` for discovery/untyped/
    /// gap entries by construction, included for shape parity).
    pub valid: bool,
}

/// A `--followup` backlog: every finding across the reviewed topics that
/// still needs human/agent attention.
#[derive(Debug, Clone, Serialize)]
pub struct FollowupBacklog {
    /// Entries, keyed by topic id.
    pub topics: HashMap<String, Vec<FollowupEntry>>,
    /// Total entries across every topic.
    pub total_needs_followup: usize,
}

fn topic_findings_dir(reports_dir: &Path, topic: &str) -> PathBuf {
    reports_dir.join(topic).join("findings")
}

pub(crate) fn list_finding_files(dir: &Path) -> Result<Vec<PathBuf>, MifRhError> {
    let entries = fs::read_dir(dir).map_err(|source| MifRhError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let is_json = path.extension().and_then(|ext| ext.to_str()) == Some("json");
            let is_dotfile = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'));
            is_json && !is_dotfile
        })
        .collect();
    files.sort();
    Ok(files)
}

/// Atomically (over)writes `path` with `records`, sorted by `finding_id` —
/// `ontology-map.json` is always rebuilt from scratch, never incrementally
/// patched, matching `ontology-review.sh` exactly.
fn write_map(path: &Path, records: &[MapRecord]) -> Result<(), MifRhError> {
    let mut sorted = records.to_vec();
    sorted.sort_by(|a, b| a.finding_id.cmp(&b.finding_id));
    crate::write_json_atomic(path, &sorted)
}

fn review_one_topic(
    topic: &str,
    opts: &ReviewOptions<'_>,
) -> Result<Option<(TopicSummary, Vec<FollowupEntry>)>, MifRhError> {
    let findings_dir = topic_findings_dir(opts.reports_dir, topic);
    if !findings_dir.is_dir() {
        // Not even counted — matches `ontology-review.sh` skipping a topic
        // whose findings directory doesn't exist.
        return Ok(None);
    }

    let files = list_finding_files(&findings_dir)?;
    let ctx = ResolveContext {
        topic,
        catalog: opts.catalog,
        config: opts.config,
        ontology_packs: opts.ontology_packs,
    };

    // (file, Some(record)) on success, (file, None) for a "gap" — a file
    // that could not be processed at all.
    let mut items: Vec<(&PathBuf, Option<MapRecord>)> = Vec::with_capacity(files.len());
    for file in &files {
        let record = Finding::load(file).and_then(|finding| resolve_finding(&finding, &ctx));
        items.push((file, record.ok()));
    }

    let records: Vec<&MapRecord> = items.iter().filter_map(|(_, r)| r.as_ref()).collect();
    let gap = items.iter().filter(|(_, r)| r.is_none()).count();

    let stamped = records
        .iter()
        .filter(|r| matches!(r.basis, Basis::Declared | Basis::Resolved) && r.valid)
        .count();
    let discovery = records
        .iter()
        .filter(|r| r.basis == Basis::Discovery && r.valid)
        .count();
    let untyped = records.iter().filter(|r| r.basis == Basis::Untyped).count();
    let bad = records.iter().filter(|r| !r.valid).count() + gap;

    write_map(
        &opts.reports_dir.join(topic).join("ontology-map.json"),
        &records.iter().map(|r| (*r).clone()).collect::<Vec<_>>(),
    )?;

    let mut followup: Vec<FollowupEntry> = Vec::new();
    for (file, record) in &items {
        match record {
            Some(r) if matches!(r.basis, Basis::Discovery | Basis::Untyped) || !r.valid => {
                followup.push(FollowupEntry {
                    finding_id: r.finding_id.clone(),
                    file: Some(file.display().to_string()),
                    basis: r.basis.label().to_string(),
                    entity_type: r.entity_type.clone(),
                    resolved_ontology: r.resolved_ontology.clone(),
                    valid: r.valid,
                });
            },
            None => {
                let finding_id = file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                followup.push(FollowupEntry {
                    finding_id,
                    file: Some(file.display().to_string()),
                    basis: "gap".to_string(),
                    entity_type: None,
                    resolved_ontology: None,
                    valid: false,
                });
            },
            Some(_) => {},
        }
    }
    followup.sort_by(|a, b| a.finding_id.cmp(&b.finding_id));

    let bound = opts
        .config
        .topic_bindings(topic)
        .into_iter()
        .map(|b| b.id)
        .collect();

    Ok(Some((
        TopicSummary {
            topic: topic.to_string(),
            bound,
            total: files.len(),
            stamped,
            discovery,
            untyped,
            bad,
        },
        followup,
    )))
}

/// Whether `check-relationship-targets.sh` (run once, corpus-wide) found
/// orphaned relationship targets.
///
/// Runs it as an external process rather than reimplementing it —
/// deliberately out of `mif-rh`'s scope (see this crate's top-level
/// design notes): it runs once per review, not once per finding, so it
/// never touches the per-finding subprocess cost this engine exists to
/// eliminate.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if the script cannot be executed at all.
fn relationship_targets_clean(script: &Path, reports_dir: &Path) -> Result<bool, MifRhError> {
    let status = Command::new(script)
        .arg("--reports-dir")
        .arg(reports_dir)
        .status()
        .map_err(|source| MifRhError::Io {
            path: script.display().to_string(),
            source,
        })?;
    Ok(status.success())
}

/// Reviews every requested topic, rebuilding each one's `ontology-map.json`
/// from scratch and aggregating coverage.
///
/// # Errors
///
/// Returns [`MifRhError`] if a topic's findings directory cannot be read,
/// or if the relationship-targets check script (when configured) cannot be
/// executed.
pub fn review(opts: &ReviewOptions<'_>) -> Result<(ReviewReport, FollowupBacklog), MifRhError> {
    let topic_ids: Vec<String> = opts.topics.map_or_else(
        || opts.config.topics.iter().map(|t| t.id.clone()).collect(),
        <[String]>::to_vec,
    );

    let mut summaries = Vec::new();
    let mut followup_topics: HashMap<String, Vec<FollowupEntry>> = HashMap::new();
    let mut any_bad = false;

    for topic in &topic_ids {
        if let Some((summary, followup)) = review_one_topic(topic, opts)? {
            if summary.bad > 0 {
                any_bad = true;
            }
            if !followup.is_empty() {
                followup_topics.insert(topic.clone(), followup);
            }
            summaries.push(summary);
        }
    }

    if let Some(script) = opts.check_relationship_targets_script
        && !relationship_targets_clean(script, opts.reports_dir)?
    {
        any_bad = true;
    }

    let total_needs_followup = followup_topics.values().map(Vec::len).sum();

    Ok((
        ReviewReport {
            topics: summaries,
            any_bad,
        },
        FollowupBacklog {
            topics: followup_topics,
            total_needs_followup,
        },
    ))
}

/// Writes a `--followup` backlog to `path`, atomically.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `path` cannot be written.
pub fn write_followup(path: &Path, backlog: &FollowupBacklog) -> Result<(), MifRhError> {
    crate::write_json_atomic(path, backlog)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{ReviewOptions, review};
    use crate::catalog::{Catalog, CatalogEntry};
    use crate::config::{HarnessConfig, TopicConfig};
    use crate::ontology_pack;

    fn write_finding(dir: &std::path::Path, name: &str, contents: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), contents).unwrap();
    }

    #[test]
    fn review_rebuilds_the_map_and_summarizes_coverage() {
        let root = tempfile::tempdir().unwrap();
        let reports_dir = root.path().join("reports");
        let ontologies_dir = root.path().join("ontologies");
        fs::create_dir_all(&ontologies_dir).unwrap();
        fs::write(
            ontologies_dir.join("edu-fixture.yaml"),
            "
ontology:
  id: edu-fixture
  version: \"0.1.0\"
entity_types:
  - name: title
    schema:
      required: [name]
      properties: {name: {type: string}}
discovery:
  enabled: true
  patterns:
    - content_pattern: \"ISBN\"
      suggest_entity: title
",
        )
        .unwrap();

        let findings_dir = reports_dir.join("edu").join("findings");
        write_finding(
            &findings_dir,
            "good.json",
            r#"{"@id":"f-good","entity":{"name":"Algebra I","entity_type":"title"}}"#,
        );
        write_finding(
            &findings_dir,
            "disc.json",
            r#"{"@id":"f-disc","content":"has an ISBN"}"#,
        );
        write_finding(
            &findings_dir,
            "untyped.json",
            r#"{"@id":"f-untyped","content":"nothing special"}"#,
        );
        write_finding(
            &findings_dir,
            "invalid.json",
            r#"{"@id":"f-invalid","entity":{"entity_type":"title"}}"#,
        );

        let catalog = Catalog {
            ontologies: vec![CatalogEntry {
                id: "edu-fixture".to_string(),
                version: "0.1.0".to_string(),
                source: None,
                core: false,
            }],
        };
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "edu".to_string(),
                ontologies: vec!["edu-fixture".to_string()],
            }],
        };

        let ontology_packs = ontology_pack::load_packs_from_dir(&ontologies_dir).unwrap();
        let opts = ReviewOptions {
            topics: None,
            reports_dir: &reports_dir,
            ontology_packs: &ontology_packs,
            catalog: &catalog,
            config: &config,
            check_relationship_targets_script: None,
        };

        let (report, backlog) = review(&opts).unwrap();
        assert_eq!(report.topics.len(), 1);
        assert_eq!(
            report.summary_line(),
            "1 topic(s); 4 findings — 1 stamped, 1 discovery-only, 1 untyped, 1 invalid/unresolved"
        );
        assert!(report.strict_should_fail());
        assert_eq!(backlog.total_needs_followup, 3);

        let map_path = reports_dir.join("edu").join("ontology-map.json");
        assert!(map_path.exists());
        let map: Vec<crate::resolve::MapRecord> =
            serde_json::from_str(&fs::read_to_string(map_path).unwrap()).unwrap();
        assert_eq!(map.len(), 4);
    }

    #[test]
    fn a_topic_with_no_findings_directory_is_skipped_entirely() {
        let root = tempfile::tempdir().unwrap();
        let reports_dir = root.path().join("reports");

        let catalog = Catalog { ontologies: vec![] };
        let config = HarnessConfig {
            topics: vec![TopicConfig {
                id: "ghost".to_string(),
                ontologies: vec![],
            }],
        };
        let ontology_packs = std::collections::HashMap::new();
        let opts = ReviewOptions {
            topics: None,
            reports_dir: &reports_dir,
            ontology_packs: &ontology_packs,
            catalog: &catalog,
            config: &config,
            check_relationship_targets_script: None,
        };

        let (report, _backlog) = review(&opts).unwrap();
        assert_eq!(report.topics.len(), 0);
        assert_eq!(
            report.summary_line(),
            "0 topic(s); 0 findings — 0 stamped, 0 discovery-only, 0 untyped, 0 invalid/unresolved"
        );
    }
}
