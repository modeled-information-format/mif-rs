//! Citation-integrity gate (rht Category B, Story #287).
//!
//! Ports rht's `scripts/check-citation-integrity.sh` (SPEC §4
//! "Verifier/citation-integrity layer"): over one or more MIF-backed
//! findings files (a single finding object or an array of them), asserts
//! every finding carries at least one citation, every citation is
//! traceable (a well-formed http(s) URL, or — when the instance opts in via
//! `harness.config.json`'s `features.internalCitations` — an internal
//! citation with a note), every citation declares a `citationRole`, no
//! finding ships with an adversarial verdict of `falsified`, and no citation
//! URL is listed dead.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// One [`check_citation_integrity`] run's result: every violation found (in
/// file-then-finding-then-check order, matching the original script), and
/// how many files were checked.
#[derive(Debug, Clone)]
pub struct CitationIntegrityReport {
    /// One line per violation, in the same order the original script would
    /// have printed them.
    pub violations: Vec<String>,
    /// The number of file arguments checked (for the summary line).
    pub files_checked: usize,
}

impl CitationIntegrityReport {
    /// Whether every finding in every file passed (no violations).
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.violations.is_empty()
    }
}

fn internal_citations_enabled(config_path: Option<&Path>) -> bool {
    config_path
        .filter(|path| path.is_file())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|contents| serde_json::from_str::<Value>(&contents).ok())
        .and_then(|config| {
            config
                .pointer("/features/internalCitations")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false)
}

fn finding_id(finding: &Value, index: usize) -> String {
    finding
        .get("@id")
        .and_then(Value::as_str)
        .map_or_else(|| format!("#{index}"), str::to_string)
}

/// Renders a JSON value the way jq's `tostring` would: a string value is
/// returned verbatim (not re-quoted); anything else uses its canonical JSON
/// form.
fn jq_tostring(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `.title // "<untitled>" | tostring` — jq's `//` treats `null` and
/// `false` as absent.
fn title_or_untitled(citation: &Value) -> String {
    match citation.get("title") {
        None | Some(Value::Null | Value::Bool(false)) => "<untitled>".to_string(),
        Some(title) => jq_tostring(title),
    }
}

fn is_traceable(citation: &Value, internal_ok: bool) -> bool {
    let has_url = citation
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(|url| url.starts_with("http://") || url.starts_with("https://"));
    let has_internal_note = internal_ok
        && citation
            .get("citationType")
            .and_then(Value::as_str)
            .is_some_and(|t| t.starts_with("internal:"))
        && citation
            .get("note")
            .and_then(Value::as_str)
            .is_some_and(|note| !note.is_empty());
    has_url || has_internal_note
}

fn citation_role_missing(citation: &Value) -> bool {
    citation
        .get("citationRole")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
}

fn check_finding(
    file: &str,
    finding: &Value,
    index: usize,
    internal_ok: bool,
    violations: &mut Vec<String>,
) {
    let loc = format!("{file}:{}: ", finding_id(finding, index));
    let citations: Vec<&Value> = finding
        .get("citations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .collect();
    let verdict = finding
        .pointer("/extensions/harness/verification/verdict")
        .and_then(Value::as_str)
        .unwrap_or("inconclusive");

    if citations.is_empty() {
        violations.push(format!("{loc}no citations (MIF Level 3 requires >=1)"));
    }
    for citation in &citations {
        if !is_traceable(citation, internal_ok) {
            let title = title_or_untitled(citation);
            let reason = if internal_ok {
                "citation has neither a well-formed http(s) URL nor an internal: source with a note"
            } else {
                "citation missing well-formed http(s) URL"
            };
            violations.push(format!("{loc}{reason}: {title}"));
        }
    }
    for citation in &citations {
        if citation_role_missing(citation) {
            violations.push(format!(
                "{loc}citation missing citationRole: {}",
                title_or_untitled(citation)
            ));
        }
    }
    if verdict == "falsified" {
        violations.push(format!(
            "{loc}adversarial verdict is falsified; finding must not ship"
        ));
    }
    let dead_urls = finding
        .pointer("/extensions/harness/citationStatus/deadUrls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for dead in dead_urls {
        violations.push(format!(
            "{loc}citation URL listed dead: {}",
            jq_tostring(dead)
        ));
    }
}

/// Checks every findings file in `paths` for citation integrity.
///
/// Each file is either a single MIF finding object or a JSON array of them.
/// Resolves the `features.internalCitations` opt-in from `config_path`
/// (defaulting to `false` if the config is missing, unreadable, or does not
/// set the flag).
///
/// A missing file or one that fails to parse as JSON is itself recorded as
/// a violation (matching the original script), not a hard error — this
/// function never returns `Result`.
#[must_use]
pub fn check_citation_integrity(
    paths: &[PathBuf],
    config_path: Option<&Path>,
) -> CitationIntegrityReport {
    let internal_ok = internal_citations_enabled(config_path);
    let mut violations = Vec::new();

    for path in paths {
        let file = path.display().to_string();
        if !path.is_file() {
            violations.push(format!("{file}: file not found"));
            continue;
        }
        let parsed = std::fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str::<Value>(&contents).ok());
        let Some(parsed) = parsed else {
            violations.push(format!("{file}: not valid JSON"));
            continue;
        };
        let findings: Vec<&Value> = match &parsed {
            Value::Array(items) => items.iter().collect(),
            other => vec![other],
        };
        for (index, finding) in findings.iter().enumerate() {
            check_finding(&file, finding, index, internal_ok, &mut violations);
        }
    }

    CitationIntegrityReport {
        violations,
        files_checked: paths.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::check_citation_integrity;
    use std::fs;
    use std::path::PathBuf;

    fn write(dir: &std::path::Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn passes_a_well_formed_finding_with_a_url_citation() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "good.json",
            r#"{"@id":"urn:mif:f1","citations":[{"url":"https://example.com","citationRole":"supports","title":"Example"}]}"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert!(report.ok(), "{:?}", report.violations);
        assert_eq!(report.files_checked, 1);
    }

    #[test]
    fn flags_a_finding_with_no_citations() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "bad.json",
            r#"{"@id":"urn:mif:f1","citations":[]}"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert_eq!(report.violations.len(), 1);
        assert!(report.violations[0].contains("no citations"));
    }

    #[test]
    fn flags_a_citation_missing_a_well_formed_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "bad.json",
            r#"{"@id":"urn:mif:f1","citations":[{"url":"ftp://example.com","citationRole":"supports","title":"Bad"}]}"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.contains("citation missing well-formed http(s) URL: Bad"))
        );
    }

    #[test]
    fn accepts_an_internal_citation_only_when_the_feature_flag_is_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "finding.json",
            r#"{"@id":"urn:mif:f1","citations":[{"citationType":"internal:doc","note":"quoted evidence","citationRole":"supports","title":"Internal"}]}"#,
        );

        let strict = check_citation_integrity(std::slice::from_ref(&path), None);
        assert!(
            strict
                .violations
                .iter()
                .any(|v| v.contains("citation missing well-formed http(s) URL"))
        );

        let config = write(
            dir.path(),
            "harness.config.json",
            r#"{"features":{"internalCitations":true}}"#,
        );
        let lenient = check_citation_integrity(&[path], Some(&config));
        assert!(lenient.ok(), "{:?}", lenient.violations);
    }

    #[test]
    fn flags_a_citation_missing_citation_role() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "bad.json",
            r#"{"@id":"urn:mif:f1","citations":[{"url":"https://example.com","title":"NoRole"}]}"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.contains("citation missing citationRole: NoRole"))
        );
    }

    #[test]
    fn flags_a_falsified_finding() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "bad.json",
            r#"{"@id":"urn:mif:f1","citations":[{"url":"https://example.com","citationRole":"supports"}],
                "extensions":{"harness":{"verification":{"verdict":"falsified"}}}}"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.contains("adversarial verdict is falsified"))
        );
    }

    #[test]
    fn flags_a_citation_url_listed_dead() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "bad.json",
            r#"{"@id":"urn:mif:f1","citations":[{"url":"https://example.com","citationRole":"supports"}],
                "extensions":{"harness":{"citationStatus":{"deadUrls":["https://example.com"]}}}}"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.contains("citation URL listed dead: https://example.com"))
        );
    }

    #[test]
    fn handles_an_array_of_findings_and_derives_a_positional_id_when_id_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "many.json",
            r#"[{"citations":[]},{"@id":"urn:mif:f2","citations":[]}]"#,
        );

        let report = check_citation_integrity(&[path], None);
        assert_eq!(report.violations.len(), 2);
        assert!(report.violations[0].contains(":#0: no citations"));
        assert!(report.violations[1].contains(":urn:mif:f2: no citations"));
    }

    #[test]
    fn records_a_missing_file_as_a_violation_not_a_hard_error() {
        let report = check_citation_integrity(&[PathBuf::from("/no/such/file.json")], None);
        assert_eq!(report.violations.len(), 1);
        assert!(report.violations[0].contains("file not found"));
    }

    #[test]
    fn records_invalid_json_as_a_violation() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "broken.json", "not json");

        let report = check_citation_integrity(&[path], None);
        assert_eq!(report.violations.len(), 1);
        assert!(report.violations[0].contains("not valid JSON"));
    }
}
