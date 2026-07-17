//! Topic README metadata rollup (rht Category B, Story #282).
//!
//! Ports the jq-dependent data computation from
//! `scripts/build-topic-readme.sh`: the topic's registered title/status, a
//! findings rollup (counts, verdicts, sources, dimensions, tags,
//! earliest-created date, per-dimension counts, a draft "Key Findings"
//! excerpt), a dimension bullet list, and a purpose line. Everything else
//! in that script (file scanning, title/genre/version extraction from
//! rendered deliverables, markdown table assembly, the structural check
//! gate, the atomic write) stays pure bash — it never used jq.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use crate::error::MifRhError;
use crate::harness_project::read_json;

/// The README title budget: an emitted [`TopicMetadata::title`] never
/// exceeds this many characters (including the truncation marker).
const TITLE_MAX_CHARS: usize = 80;

/// Everything `build-topic-readme.sh` previously computed via `jq`, ready
/// to be emitted as shell variable assignments.
#[derive(Debug, Clone)]
pub struct TopicMetadata {
    /// The topic's registered title (falls back to the topic id), trimmed
    /// of surrounding whitespace and truncated on a word boundary to
    /// [`TITLE_MAX_CHARS`] characters with a `…` marker when longer. It
    /// never starts or ends with whitespace, so the README `# <TITLE>`
    /// heading built from it cannot fail markdownlint MD009.
    pub title: String,
    /// The topic's registered status (falls back to `"active"`).
    pub status: String,
    /// Total finding count.
    pub count: usize,
    /// Unique non-null citation URLs across every finding.
    pub sources: usize,
    /// The earliest non-null `created` value across findings, or empty.
    pub created: String,
    /// Count of findings with verdict `survived`.
    pub survived: usize,
    /// Count of findings with verdict `weakened`.
    pub weakened: usize,
    /// Count of findings with verdict `inconclusive`.
    pub inconclusive: usize,
    /// Count of findings with verdict `falsified`.
    pub falsified: usize,
    /// Pre-rendered `- **dim** — desc` bullet list (or `"—"` if empty).
    pub dim_bullets: String,
    /// Pre-rendered backtick-quoted tag list (or `"—"` if empty).
    pub tags: String,
    /// The goal statement, or a generic fallback naming the title.
    pub purpose: String,
    /// Pre-rendered `- <summary>` draft bullets (up to 8), survived-first.
    pub key_draft: String,
    /// Pre-rendered `| dim | count |` markdown table rows.
    pub by_dim_table: String,
}

fn escape_shell_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

impl TopicMetadata {
    /// Renders every field as a `NAME='value'` shell assignment, one per
    /// line, suitable for `source <(...)` from bash.
    #[must_use]
    pub fn to_shell_script(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("TITLE={}", escape_shell_single_quoted(&self.title)));
        lines.push(format!(
            "STATUS={}",
            escape_shell_single_quoted(&self.status)
        ));
        lines.push(format!("COUNT={}", self.count));
        lines.push(format!("SOURCES={}", self.sources));
        lines.push(format!(
            "CREATED={}",
            escape_shell_single_quoted(&self.created)
        ));
        lines.push(format!("SURV={}", self.survived));
        lines.push(format!("WEAK={}", self.weakened));
        lines.push(format!("INC={}", self.inconclusive));
        lines.push(format!("FALS={}", self.falsified));
        lines.push(format!(
            "DIM_BULLETS={}",
            escape_shell_single_quoted(&self.dim_bullets)
        ));
        lines.push(format!("TAGS={}", escape_shell_single_quoted(&self.tags)));
        lines.push(format!(
            "PURPOSE={}",
            escape_shell_single_quoted(&self.purpose)
        ));
        lines.push(format!(
            "KEY_DRAFT={}",
            escape_shell_single_quoted(&self.key_draft)
        ));
        lines.push(format!(
            "BY_DIM_TABLE={}",
            escape_shell_single_quoted(&self.by_dim_table)
        ));
        lines.join("\n")
    }
}

fn unique_sorted_strings(values: impl Iterator<Item = Option<String>>) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for value in values.flatten() {
        set.insert(value);
    }
    set.into_iter().collect()
}

struct Roll {
    count: usize,
    sources: usize,
    dimensions: Vec<String>,
    tags: Vec<String>,
    created: Option<String>,
    verdicts: BTreeMap<String, usize>,
    by_dim: Vec<(String, usize)>,
    key: Vec<String>,
}

fn compute_roll(findings: &[Value]) -> Roll {
    if findings.is_empty() {
        return Roll {
            count: 0,
            sources: 0,
            dimensions: Vec::new(),
            tags: Vec::new(),
            created: None,
            verdicts: BTreeMap::new(),
            by_dim: Vec::new(),
            key: Vec::new(),
        };
    }

    let sources = unique_sorted_strings(
        findings
            .iter()
            .flat_map(|f| {
                f.get("citations")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .map(|c| c.get("url").and_then(Value::as_str).map(str::to_string)),
    )
    .len();

    let dimensions = unique_sorted_strings(findings.iter().map(|f| {
        f.pointer("/extensions/harness/dimension")
            .and_then(Value::as_str)
            .map(str::to_string)
    }));

    let tags = unique_sorted_strings(
        findings
            .iter()
            .flat_map(|f| {
                f.get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .map(|t| t.as_str().map(str::to_string)),
    );

    let mut created_values: Vec<String> = findings
        .iter()
        .filter_map(|f| f.get("created").and_then(Value::as_str).map(str::to_string))
        .collect();
    created_values.sort();
    let created = created_values.into_iter().next();

    let verdicts = count_verdicts(findings);
    let by_dim = count_by_dimension(findings);
    let key = draft_key_findings(findings);

    Roll {
        count: findings.len(),
        sources,
        dimensions,
        tags,
        created,
        verdicts,
        by_dim,
        key,
    }
}

fn verdict_of(finding: &Value) -> &str {
    finding
        .pointer("/extensions/harness/verification/verdict")
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn count_verdicts(findings: &[Value]) -> BTreeMap<String, usize> {
    let mut verdicts: BTreeMap<String, usize> = BTreeMap::new();
    for finding in findings {
        let verdict = match verdict_of(finding) {
            "" => "none",
            v => v,
        };
        *verdicts.entry(verdict.to_string()).or_insert(0) += 1;
    }
    verdicts
}

fn count_by_dimension(findings: &[Value]) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for finding in findings {
        let dim = finding
            .pointer("/extensions/harness/dimension")
            .and_then(Value::as_str)
            .unwrap_or("unspecified")
            .to_string();
        *counts.entry(dim).or_insert(0) += 1;
    }
    let mut by_dim: Vec<(String, usize)> = counts.into_iter().collect();
    by_dim.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    by_dim
}

/// The up-to-8 survived/weakened finding summaries seeding the "Key
/// Findings" draft, survived-first (stable — ties preserve the findings'
/// own sorted-filename order).
fn draft_key_findings(findings: &[Value]) -> Vec<String> {
    let mut candidates: Vec<&Value> = findings
        .iter()
        .filter(|f| matches!(verdict_of(f), "survived" | "weakened"))
        .collect();
    candidates.sort_by_key(|f| verdict_of(f) != "survived");
    candidates
        .into_iter()
        .filter_map(|f| {
            f.get("summary")
                .and_then(Value::as_str)
                .or_else(|| f.get("title").and_then(Value::as_str))
        })
        .map(str::to_string)
        .take(8)
        .collect()
}

fn render_dim_bullets(roll_dimensions: &[String], goal: &Value, config: &Value) -> String {
    let goal_dimensions = goal
        .get("dimensions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string));
    let dims = unique_sorted_strings(
        goal_dimensions
            .chain(roll_dimensions.iter().cloned())
            .map(Some),
    );
    if dims.is_empty() {
        return "—".to_string();
    }
    let config_dims: &[Value] = config
        .get("dimensions")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice);
    dims.iter()
        .map(|dim| {
            let description = config_dims
                .iter()
                .find(|entry| {
                    entry.get("id").and_then(Value::as_str) == Some(dim.as_str())
                        || entry.get("name").and_then(Value::as_str) == Some(dim.as_str())
                })
                .and_then(|entry| entry.get("description"))
                .and_then(Value::as_str);
            match description {
                Some(desc) if !desc.is_empty() => format!("- **{dim}** — {desc}"),
                _ => format!("- **{dim}**"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        "—".to_string()
    } else {
        tags.iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Sanitizes a topic title for README emission.
///
/// Trims surrounding whitespace, and when the result still exceeds
/// [`TITLE_MAX_CHARS`] characters, cuts it back to the last word boundary
/// inside the budget (falling back to a hard character cut when a single
/// unbroken token overruns it), trims again, and appends `…` so a
/// truncated title stays distinguishable from a short one. The result
/// never starts or ends with whitespace, no matter where in the input the
/// cut lands — a registered title carrying an upstream 80-character
/// cutoff's trailing space comes out clean (issue #86).
fn sanitize_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().count() <= TITLE_MAX_CHARS {
        return trimmed.to_string();
    }
    // Reserve one character of the budget for the truncation marker.
    let budget = TITLE_MAX_CHARS - 1;
    let head: String = trimmed.chars().take(budget).collect();
    let cut = if trimmed.chars().nth(budget).is_some_and(char::is_whitespace) {
        // The cut already lands on a word boundary: keep the whole head.
        head.as_str()
    } else {
        // Mid-word cut: back up to the last word boundary, or keep the
        // hard cut when the head is one unbroken token.
        head.rfind(char::is_whitespace)
            .map_or(head.as_str(), |idx| &head[..idx])
    };
    format!("{}…", cut.trim_end())
}

fn render_purpose(goal: &Value, title: &str) -> String {
    for key in ["goal_statement", "research_question", "goal", "question"] {
        if let Some(value) = goal.get(key).and_then(Value::as_str)
            && !value.is_empty()
        {
            return value.to_string();
        }
    }
    format!("Research session for {title}.")
}

/// Computes a topic's README metadata rollup.
///
/// # Errors
///
/// Returns [`MifRhError::TopicNotRegistered`] if `topic` has no entry in
/// `config_path`'s `topics[]`, or [`MifRhError::Io`]/[`MifRhError::Json`] if
/// a finding file under `findings_dir` cannot be read or parsed.
pub fn topic_metadata(
    topic: &str,
    config_path: &Path,
    findings_dir: &Path,
    goal_path: &Path,
) -> Result<TopicMetadata, MifRhError> {
    let config = read_json(config_path)?;
    let topic_entry = config
        .get("topics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(topic))
        .cloned()
        .ok_or_else(|| MifRhError::TopicNotRegistered {
            topic: topic.to_string(),
            config_path: config_path.display().to_string(),
        })?;
    let title = sanitize_title(
        topic_entry
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(topic),
    );
    let status = topic_entry
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("active")
        .to_string();

    let mut paths: Vec<_> = std::fs::read_dir(findings_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let findings: Vec<Value> = paths
        .iter()
        .map(|path| read_json(path))
        .collect::<Result<Vec<_>, _>>()?;

    let roll = compute_roll(&findings);
    let goal = read_json(goal_path).unwrap_or_else(|_| Value::Object(serde_json::Map::new()));

    let dim_bullets = render_dim_bullets(&roll.dimensions, &goal, &config);
    let tags = render_tags(&roll.tags);
    let purpose = render_purpose(&goal, &title);
    let key_draft = roll
        .key
        .iter()
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let by_dim_table = roll
        .by_dim
        .iter()
        .map(|(dim, count)| format!("| {dim} | {count} |"))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(TopicMetadata {
        title,
        status,
        count: roll.count,
        sources: roll.sources,
        created: roll.created.unwrap_or_default(),
        survived: roll.verdicts.get("survived").copied().unwrap_or(0),
        weakened: roll.verdicts.get("weakened").copied().unwrap_or(0),
        inconclusive: roll.verdicts.get("inconclusive").copied().unwrap_or(0),
        falsified: roll.verdicts.get("falsified").copied().unwrap_or(0),
        dim_bullets,
        tags,
        purpose,
        key_draft,
        by_dim_table,
    })
}

#[cfg(test)]
mod tests {
    use super::topic_metadata;
    use std::fs;

    fn setup(
        dir: &std::path::Path,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let config_path = dir.join("harness.config.json");
        fs::write(
            &config_path,
            r#"{"topics": [{"id": "t1", "title": "Topic One", "status": "active"}],
                "dimensions": [{"id": "landscape", "description": "Market landscape"}]}"#,
        )
        .unwrap();
        let findings_dir = dir.join("findings");
        fs::create_dir_all(&findings_dir).unwrap();
        let goal_path = dir.join("goal.json");
        (config_path, findings_dir, goal_path)
    }

    #[test]
    fn errors_when_the_topic_is_not_registered() {
        let dir = tempfile::tempdir().unwrap();
        let (config_path, findings_dir, goal_path) = setup(dir.path());

        let error = topic_metadata("nope", &config_path, &findings_dir, &goal_path).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::TopicNotRegistered { .. }
        ));
    }

    #[test]
    fn computes_counts_and_falls_back_title_purpose_with_no_findings() {
        let dir = tempfile::tempdir().unwrap();
        let (config_path, findings_dir, goal_path) = setup(dir.path());

        let meta = topic_metadata("t1", &config_path, &findings_dir, &goal_path).unwrap();
        assert_eq!(meta.title, "Topic One");
        assert_eq!(meta.status, "active");
        assert_eq!(meta.count, 0);
        assert_eq!(meta.sources, 0);
        assert_eq!(meta.dim_bullets, "—");
        assert_eq!(meta.tags, "—");
        assert_eq!(meta.purpose, "Research session for Topic One.");
        assert_eq!(meta.key_draft, "");
    }

    #[test]
    fn rolls_up_verdicts_sources_and_key_findings() {
        let dir = tempfile::tempdir().unwrap();
        let (config_path, findings_dir, goal_path) = setup(dir.path());
        fs::write(
            findings_dir.join("f1.json"),
            r#"{"summary": "Finding one survives", "created": "2026-01-02",
                "citations": [{"url": "https://a"}, {"url": "https://a"}],
                "tags": ["alpha", "beta"],
                "extensions": {"harness": {"dimension": "landscape",
                    "verification": {"verdict": "survived"}}}}"#,
        )
        .unwrap();
        fs::write(
            findings_dir.join("f2.json"),
            r#"{"summary": "Finding two is falsified", "created": "2026-01-01",
                "extensions": {"harness": {"dimension": "market",
                    "verification": {"verdict": "falsified"}}}}"#,
        )
        .unwrap();

        let meta = topic_metadata("t1", &config_path, &findings_dir, &goal_path).unwrap();
        assert_eq!(meta.count, 2);
        assert_eq!(meta.sources, 1);
        assert_eq!(meta.survived, 1);
        assert_eq!(meta.falsified, 1);
        assert_eq!(meta.created, "2026-01-01");
        assert_eq!(meta.tags, "`alpha` `beta`");
        assert_eq!(
            meta.dim_bullets,
            "- **landscape** — Market landscape\n- **market**"
        );
        assert_eq!(meta.key_draft, "- Finding one survives");
    }

    #[test]
    fn purpose_prefers_the_goal_statement_over_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (config_path, findings_dir, goal_path) = setup(dir.path());
        fs::write(
            &goal_path,
            r#"{"goal_statement": "Understand the market."}"#,
        )
        .unwrap();

        let meta = topic_metadata("t1", &config_path, &findings_dir, &goal_path).unwrap();
        assert_eq!(meta.purpose, "Understand the market.");
    }

    /// Regression for issue #86: an upstream registrar truncates a
    /// `goal_statement` to 80 characters for the registered title; when
    /// that cutoff lands right after a space the config carries a title
    /// with a trailing space, which used to reach the emitted `TITLE=`
    /// verbatim and fail markdownlint MD009 in the generated README.
    #[test]
    fn title_from_an_80_char_cutoff_landing_after_a_space_has_no_trailing_whitespace() {
        // Engineered so the 80th character of the goal statement lands
        // right after a space: the first 80 chars end with "to the ".
        let goal_statement = "Enable the decision of whether and how to add a set of Claude \
                              Code hooks to the github-sdlc-planning plugin";
        let registered_title: String = goal_statement.chars().take(80).collect();
        assert_eq!(registered_title.chars().count(), 80);
        assert!(
            registered_title.ends_with(' '),
            "fixture must reproduce the cut-after-a-space case"
        );

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(
            &config_path,
            serde_json::json!({"topics": [{"id": "t1", "title": registered_title}]}).to_string(),
        )
        .unwrap();
        let findings_dir = dir.path().join("findings");
        fs::create_dir_all(&findings_dir).unwrap();

        let meta = topic_metadata(
            "t1",
            &config_path,
            &findings_dir,
            &dir.path().join("goal.json"),
        )
        .unwrap();
        assert_eq!(meta.title, registered_title.trim_end());
        assert_eq!(meta.title, meta.title.trim());
        let title_line = meta
            .to_shell_script()
            .lines()
            .next()
            .map(str::to_string)
            .unwrap();
        assert!(!title_line.trim_end_matches('\'').ends_with(' '));
    }

    #[test]
    fn over_budget_title_cut_landing_on_a_word_boundary_keeps_the_whole_head() {
        // 5-char groups: the 79-char head ends exactly at the 16th word.
        let long = "word ".repeat(30);
        let title = super::sanitize_title(&long);
        assert_eq!(title, format!("{}…", "word ".repeat(16).trim_end()));
        assert_eq!(title.chars().count(), 80);
    }

    #[test]
    fn over_budget_title_cut_landing_mid_word_backs_up_to_the_last_word_boundary() {
        // 6-char groups: the 79-char head ends one char into the 14th
        // word, so the cut backs up to the 13th word's end.
        let long = "alpha ".repeat(30);
        let title = super::sanitize_title(&long);
        assert_eq!(title, format!("{}…", "alpha ".repeat(13).trim_end()));
        assert!(title.chars().count() <= 80);
    }

    #[test]
    fn over_budget_unbroken_token_title_is_hard_cut_with_a_marker() {
        let long = "x".repeat(100);
        let title = super::sanitize_title(&long);
        assert_eq!(title, format!("{}…", "x".repeat(79)));
        assert_eq!(title.chars().count(), 80);
    }

    #[test]
    fn exactly_80_char_title_without_trailing_space_is_unchanged() {
        let exact = "y".repeat(80);
        assert_eq!(super::sanitize_title(&exact), exact);
    }

    #[test]
    fn shell_script_escapes_embedded_single_quotes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("harness.config.json"),
            r#"{"topics": [{"id": "t1", "title": "Bob's Topic"}]}"#,
        )
        .unwrap();
        let findings_dir = dir.path().join("findings");
        fs::create_dir_all(&findings_dir).unwrap();

        let meta = topic_metadata(
            "t1",
            &dir.path().join("harness.config.json"),
            &findings_dir,
            &dir.path().join("goal.json"),
        )
        .unwrap();
        let script = meta.to_shell_script();
        assert!(script.contains(r"TITLE='Bob'\''s Topic'"));
    }
}
