//! Artifact-to-channel rendering (rht Category B, Story #293).
//!
//! Ports rht's `scripts/render-artifact.sh`: renders one typed Artifact
//! (`schemas/artifact.schema.json`) to an output channel. Three channels:
//! `report` (the canonical MIF Level-3 markdown report, write-then-validated
//! by `mif-project.sh`), `blog`, and `book` (published, MIF Level-1
//! channels — exempt from L3 conformance, no internal finding identity
//! leaks into their prose).

use serde_json::{Value, json};

use crate::error::MifRhError;
use crate::harness_markdown::{dedupe_sections, secblock, source_link_line};

/// The pre-resolved, caller-supplied inputs [`render_artifact`] needs
/// beyond the artifact JSON itself.
///
/// Path/version arithmetic the original script does against the live
/// filesystem (resolving `$OUT`'s repo-relative slug path, reading a
/// prior version to increment) stays a CLI-layer concern, not a
/// pure-rendering one.
pub struct RenderInputs<'a> {
    /// The parsed `artifact.json` document. Its `.namespace` field (or the
    /// `"harness/report"` default) is the report/blog/book namespace —
    /// derived internally, not a separate input.
    pub artifact: &'a Value,
    /// The output file's slug (its basename, minus `.md`).
    pub slug: &'a str,
    /// The output file's repo-root-relative path (for the `slug:`
    /// frontmatter field astro-rehype-relative-markdown-links needs).
    pub slugpath: &'a str,
    /// The RFC 3339 `created` timestamp.
    pub created: &'a str,
    /// This revision's version number (prior version + 1, or 1).
    pub version: u64,
    /// A falsification verdict (`extensions.harness.verification`) to fold
    /// into the `report` channel's frontmatter. Required for `report`;
    /// ignored for `blog`/`book`.
    pub verification: Option<&'a Value>,
}

/// Renders `inputs.artifact` to `channel` (`"report"`, `"blog"`, or
/// `"book"`), returning the complete markdown file contents (frontmatter +
/// body).
///
/// # Errors
///
/// Returns [`MifRhError::InvalidToggleValue`] if `channel` is not one of
/// the three recognized values.
pub fn render_artifact(inputs: &RenderInputs<'_>, channel: &str) -> Result<String, MifRhError> {
    match channel {
        "report" => render_report(inputs),
        "blog" => Ok(render_blog(inputs)),
        "book" => Ok(render_book(inputs)),
        other => Err(MifRhError::InvalidToggleValue {
            field: "channel".to_string(),
            value: other.to_string(),
            allowed: "report|blog|book".to_string(),
        }),
    }
}

fn namespace_of(artifact: &Value) -> &str {
    artifact
        .get("namespace")
        .and_then(Value::as_str)
        .unwrap_or("harness/report")
}

fn sections_of(artifact: &Value) -> Vec<Value> {
    let raw = artifact
        .get("sections")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    dedupe_sections(&raw)
}

fn sources_of(artifact: &Value) -> Vec<Value> {
    artifact
        .get("sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn sources_list_block(artifact: &Value) -> Vec<String> {
    let mut lines = vec![String::new(), "## Sources".to_string(), String::new()];
    for source in &sources_of(artifact) {
        lines.push(source_link_line(source));
    }
    lines
}

fn render_report(inputs: &RenderInputs<'_>) -> Result<String, MifRhError> {
    let artifact = inputs.artifact;
    let namespace = namespace_of(artifact);
    let genre = artifact
        .get("genre")
        .and_then(Value::as_str)
        .unwrap_or("general");
    let finding_count = artifact
        .get("finding_refs")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    let mut body_lines = vec![format!(
        "This {genre} synthesis covers {finding_count} surviving finding(s) across the research."
    )];
    for section in &sections_of(artifact) {
        body_lines.extend(secblock(section, true, true));
    }
    body_lines.extend(sources_list_block(artifact));
    let body = body_lines.join("\n");

    let citations: Vec<Value> = sources_of(artifact)
        .iter()
        .map(|source| {
            let mut citation = json!({
                "@type": "Citation",
                "citationType": source.get("citationType"),
                "citationRole": source.get("citationRole"),
                "title": source.get("title"),
                "url": source.get("url"),
            });
            if let Some(note) = source.get("note") {
                citation["note"] = note.clone();
            }
            citation
        })
        .collect();

    let mut concept = json!({
        "@context": "https://mif-spec.dev/schema/context.jsonld",
        "@type": "Concept",
        "@id": format!("urn:mif:report:{namespace}:{}", inputs.slug),
        "slug": inputs.slugpath,
        "version": inputs.version,
        "conceptType": "semantic",
        "namespace": namespace,
        "title": artifact.get("title"),
        "genre": genre,
        "created": inputs.created,
        "provenance": {
            "@type": "Provenance",
            "sourceType": "system_generated",
            "confidence": 0.9,
            "trustLevel": "moderate_confidence",
        },
        "citations": citations,
        "extensions": { "harness": { "dimension": "synthesis" } },
    });
    if let Some(verification) = inputs.verification {
        concept["extensions"]["harness"]["verification"] = verification.clone();
    }

    // serde_json::Value's Serialize impl is format-agnostic, so serializing
    // it through serde_norway's Serializer produces the same YAML a
    // dedicated YAML type would, matching `yq -p=json -o=yaml`.
    let frontmatter_yaml = serde_norway::to_string(&concept)
        .map_err(|source| MifRhError::FrontmatterYamlSerialize { source })?;
    Ok(format!("---\n{frontmatter_yaml}---\n\n{body}\n"))
}

fn render_blog(inputs: &RenderInputs<'_>) -> String {
    let artifact = inputs.artifact;
    let namespace = namespace_of(artifact);
    let title = artifact
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();

    let mut lines = vec![
        "---".to_string(),
        "\"@context\": https://mif-spec.dev/schema/context.jsonld".to_string(),
        "\"@type\": Concept".to_string(),
        format!("\"@id\": urn:mif:blog:{namespace}:{}", inputs.slug),
        format!("slug: {}", inputs.slugpath),
        format!("version: {}", inputs.version),
        "conceptType: semantic".to_string(),
        format!("created: \"{}\"", inputs.created),
        format!("namespace: {namespace}"),
        "---".to_string(),
        String::new(),
        format!("# {title}"),
    ];
    for section in &sections_of(artifact) {
        lines.extend(secblock(section, false, true));
    }
    lines.extend(sources_list_block(artifact));
    format!("{}\n", lines.join("\n"))
}

fn render_book(inputs: &RenderInputs<'_>) -> String {
    let artifact = inputs.artifact;
    let namespace = namespace_of(artifact);
    let title = artifact
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let genre = artifact
        .get("genre")
        .and_then(Value::as_str)
        .unwrap_or("general");
    let audience = artifact
        .get("audience")
        .and_then(Value::as_str)
        .unwrap_or("general");

    let mut lines = vec![
        "---".to_string(),
        "\"@context\": https://mif-spec.dev/schema/context.jsonld".to_string(),
        "\"@type\": Concept".to_string(),
        format!("\"@id\": urn:mif:book:{namespace}:{}", inputs.slug),
        format!("slug: {}", inputs.slugpath),
        format!("version: {}", inputs.version),
        "conceptType: semantic".to_string(),
        format!("created: \"{}\"", inputs.created),
        format!("namespace: {namespace}"),
        "---".to_string(),
        String::new(),
        format!("# Chapter: {title}"),
        String::new(),
        format!("> Genre: {genre} · audience: {audience}"),
    ];
    for section in &sections_of(artifact) {
        lines.extend(secblock(section, false, false));
    }
    lines.push(String::new());
    lines.push("## Endnotes".to_string());
    lines.push(String::new());
    for (i, source) in sources_of(artifact).iter().enumerate() {
        let title = source
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = title.trim_matches([' ', '\t']);
        let url = source
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        lines.push(format!("[{}] {title} — <{url}>", i + 1));
    }
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{RenderInputs, render_artifact};
    use serde_json::json;

    fn artifact() -> serde_json::Value {
        json!({
            "title": "Widget Synthesis",
            "genre": "landscape",
            "audience": "engineers",
            "namespace": "harness/widgets",
            "finding_refs": ["urn:mif:f1", "urn:mif:f2"],
            "sections": [
                {
                    "heading": "First finding",
                    "body": "Some prose about widgets.",
                    "dimension": "landscape",
                    "verdict": "survived",
                    "sources": [{"title": "Source A", "url": "https://a.example"}],
                },
            ],
            "sources": [{"title": "Source A", "url": "https://a.example"}],
        })
    }

    fn inputs(artifact: &serde_json::Value) -> RenderInputs<'_> {
        RenderInputs {
            artifact,
            slug: "widget-report",
            slugpath: "reports/widgets/widget-report",
            created: "2026-01-01T00:00:00Z",
            version: 1,
            verification: None,
        }
    }

    #[test]
    fn report_channel_produces_yaml_frontmatter_and_l3_concept_fields() {
        let art = artifact();
        let rendered = render_artifact(&inputs(&art), "report").unwrap();
        assert!(rendered.starts_with("---\n"));
        assert!(rendered.contains("'@id': urn:mif:report:harness/widgets:widget-report"));
        assert!(rendered.contains("conceptType: semantic"));
        assert!(rendered.contains("## First finding"));
        assert!(rendered.contains("## Sources"));
        // The report body carries no H1 — the title lives in frontmatter.
        assert!(!rendered.contains("\n# Widget Synthesis"));
    }

    #[test]
    fn blog_channel_carries_a_body_h1_and_no_dimension_meta_line() {
        let art = artifact();
        let rendered = render_artifact(&inputs(&art), "blog").unwrap();
        assert!(rendered.contains("# Widget Synthesis"));
        assert!(!rendered.contains("_Dimension:"));
        assert!(rendered.contains("Evidence:"));
    }

    #[test]
    fn book_channel_has_chapter_heading_and_numbered_endnotes_not_inline_evidence() {
        let art = artifact();
        let rendered = render_artifact(&inputs(&art), "book").unwrap();
        assert!(rendered.contains("# Chapter: Widget Synthesis"));
        assert!(rendered.contains("> Genre: landscape · audience: engineers"));
        assert!(rendered.contains("## Endnotes"));
        assert!(rendered.contains("[1] Source A"));
        assert!(!rendered.contains("Evidence:"));
    }

    #[test]
    fn rejects_an_unknown_channel() {
        let art = artifact();
        let error = render_artifact(&inputs(&art), "podcast").unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::InvalidToggleValue { .. }
        ));
    }

    #[test]
    fn report_channel_folds_in_a_supplied_verification_verdict() {
        let art = artifact();
        let verification = json!({"verdict": "survived", "attempted_at": "2026-01-01"});
        let mut opts = inputs(&art);
        opts.verification = Some(&verification);
        let rendered = render_artifact(&opts, "report").unwrap();
        assert!(rendered.contains("verdict: survived"));
    }
}
