//! Markdown-rendering text transforms shared by `render-artifact.sh`'s
//! three output channels (rht Category B, Story #293).
//!
//! Ports the `DEF` jq function library embedded in `render-artifact.sh`:
//! autolinking, glob/emphasis-character escaping (outside links), trailing
//! whitespace stripping, and fence-aware body rendering. Kept in its own
//! module since these are pure text transforms with no I/O, reusable
//! across all three render channels.

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use serde_json::Value;

// Both patterns are fixed string literals validated by this module's own
// tests on every build; a compile failure here would be a bug in the
// pattern itself, not a runtime condition a caller can hit.
#[allow(clippy::expect_used)]
fn email_pattern() -> &'static fancy_regex::Regex {
    static RE: OnceLock<fancy_regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        fancy_regex::Regex::new(r"(?<![\w.<])([\w.%+-]+@[\w.-]+\.[A-Za-z]{2,})(?![\w>])")
            .expect("email autolink pattern is a fixed, compile-time-valid regex")
    })
}

#[allow(clippy::expect_used)]
fn url_pattern() -> &'static fancy_regex::Regex {
    static RE: OnceLock<fancy_regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        fancy_regex::Regex::new(r#"(?<!\]\()(?<![<"])(https?://[^\s)<>"]*[^\s)<>".,;:!?])"#)
            .expect("URL autolink pattern is a fixed, compile-time-valid regex")
    })
}

/// Wraps bare emails / http(s) URLs in angle-bracket autolinks.
///
/// Keeps the rendered body markdownlint-clean (MD034). Skips anything
/// already inside a markdown link `](...)` or an existing `<...>`
/// autolink (via lookbehind), so links are not double-wrapped.
#[must_use]
pub fn autolink(text: &str) -> String {
    let step1 = email_pattern().replace_all(text, "<$1>");
    url_pattern().replace_all(&step1, "<$1>").into_owned()
}

/// Escapes literal `*` and space-flanked `_` in `text` (research prose
/// carries math operators, glob/wildcard tokens, and stray asterisks, none
/// of which are intended markdown emphasis).
fn escape_prose(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '*' => out.push_str("\\*"),
            '_' => {
                let space_before = i > 0 && chars[i - 1].is_whitespace();
                let space_after = i + 1 < chars.len() && chars[i + 1].is_whitespace();
                if space_before || space_after {
                    out.push_str("\\_");
                } else {
                    out.push('_');
                }
            },
            other => out.push(other),
        }
    }
    out
}

/// The end index (inclusive) of an autolink span `<[^<>\s]*>` starting at
/// `start`, if `chars[start]` is `<` and a valid span exists.
fn autolink_span_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) != Some(&'<') {
        return None;
    }
    let mut j = start + 1;
    while let Some(&c) = chars.get(j) {
        match c {
            '>' => return Some(j),
            '<' => return None,
            c if c.is_whitespace() => return None,
            _ => {},
        }
        j += 1;
    }
    None
}

/// The end index (inclusive) of a link-target span `\]\([^)]*\)` starting
/// at `start`, if `chars[start..start+2]` is `](` and a closing `)`
/// follows.
fn link_target_span_end(chars: &[char], start: usize) -> Option<usize> {
    if chars.get(start) != Some(&']') || chars.get(start + 1) != Some(&'(') {
        return None;
    }
    let mut j = start + 2;
    while let Some(&c) = chars.get(j) {
        if c == ')' {
            return Some(j);
        }
        j += 1;
    }
    None
}

/// Escapes every literal `*` and space-flanked `_` in prose outside links.
///
/// Characters inside a `<...>` autolink or `](...)` markdown link target
/// must not be escaped, since a `\*` inside an autolink target is
/// rendered literally and breaks the link.
#[must_use]
pub fn deglob(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = autolink_span_end(&chars, i) {
            out.push_str(&escape_prose(&plain));
            plain.clear();
            out.extend(&chars[i..=end]);
            i = end + 1;
        } else if let Some(end) = link_target_span_end(&chars, i) {
            out.push_str(&escape_prose(&plain));
            plain.clear();
            out.extend(&chars[i..=end]);
            i = end + 1;
        } else {
            plain.push(chars[i]);
            i += 1;
        }
    }
    out.push_str(&escape_prose(&plain));
    out
}

/// Strips trailing spaces/tabs from every line (MD009).
#[must_use]
pub fn detrail(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Renders a section body, applying `autolink` + `deglob` to prose only.
///
/// A fenced ``` code block (e.g. a Mermaid diagram) passes through
/// verbatim — escaping `*`/`_` or autolinking a URL inside a fence would
/// corrupt it. A fence opens/closes only on a line whose first non-space
/// run is ``` (matching `CommonMark`); an unclosed fence keeps the rest of
/// the body verbatim.
#[must_use]
pub fn render_body(text: &str) -> String {
    let mut out_lines = Vec::new();
    let mut in_fence = false;
    for line in text.split('\n') {
        let trimmed_start = line.trim_start_matches([' ', '\t']);
        if trimmed_start.starts_with("```") {
            out_lines.push(line.to_string());
            in_fence = !in_fence;
        } else if in_fence {
            out_lines.push(line.to_string());
        } else {
            out_lines.push(deglob(&autolink(line)));
        }
    }
    detrail(&out_lines.join("\n"))
}

/// Disambiguates repeated section headings (two findings can share a
/// title) so the rendered body has no duplicate H2s (markdownlint MD024).
/// Appends `" (N)"` to the Nth repeat of a heading.
#[must_use]
pub fn dedupe_sections(sections: &[Value]) -> Vec<Value> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    sections
        .iter()
        .map(|section| {
            let heading = section
                .get("heading")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let count = seen.entry(heading.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                let mut section = section.clone();
                section["heading"] = Value::String(format!("{heading} ({count})"));
                section
            } else {
                section.clone()
            }
        })
        .collect()
}

/// Escapes `"` (to `'`) and strips embedded newlines from a Mermaid node
/// label/edge type, so a MIF `@id`/name never breaks the diagram's quoted
/// string syntax.
fn mermaid_escape(text: &str, extra: &[char]) -> String {
    text.chars()
        .map(|c| {
            if c == '"' || extra.contains(&c) {
                '\''
            } else {
                c
            }
        })
        .collect::<String>()
        .replace(['\n', '\r'], " ")
}

/// Generates a Mermaid `graph TD` diagram of a section's entities/relationships.
///
/// Entities become nodes, typed relationships become edges; an empty
/// `Vec` if the section carries neither. Node ids are index-synthesised
/// (`n0`, `n1`, …) so MIF `urn:`/`@id` targets never leak special
/// characters into Mermaid syntax.
#[must_use]
pub fn mermaid_graph(section: &Value) -> Vec<String> {
    let entities: Vec<Value> = section
        .get("entities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let relationships: Vec<Value> = section
        .get("relationships")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let src = section
        .get("supports")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str);

    if entities.is_empty() && relationships.is_empty() {
        return Vec::new();
    }

    let mut ids: BTreeSet<String> = BTreeSet::new();
    if let Some(s) = src {
        ids.insert(s.to_string());
    }
    for entity in &entities {
        if let Some(id) = entity.get("id").and_then(Value::as_str) {
            ids.insert(id.to_string());
        }
    }
    for relationship in &relationships {
        if let Some(target) = relationship.get("target").and_then(Value::as_str) {
            ids.insert(target.to_string());
        }
    }
    let ids: Vec<String> = ids.into_iter().collect();
    let node_id: HashMap<&str, String> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), format!("n{i}")))
        .collect();

    let heading = section
        .get("heading")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let label_for = |id: &str| -> String {
        if Some(id) == src {
            heading.to_string()
        } else if let Some(entity) = entities
            .iter()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(id))
        {
            let name = entity
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let entity_type = entity
                .get("entityType")
                .and_then(Value::as_str)
                .unwrap_or("entity");
            format!("{name} ({entity_type})")
        } else {
            id.rsplit([':', '/']).next().unwrap_or(id).to_string()
        }
    };

    let mut lines = vec![
        String::new(),
        "```mermaid".to_string(),
        "graph TD".to_string(),
    ];
    for id in &ids {
        let label = mermaid_escape(&label_for(id), &[]);
        lines.push(format!("  {}[\"{label}\"]", node_id[id.as_str()]));
    }
    if let Some(src) = src {
        for relationship in &relationships {
            let Some(target) = relationship.get("target").and_then(Value::as_str) else {
                continue;
            };
            let rel_type = relationship
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("relates-to");
            let rel_type = mermaid_escape(rel_type, &['|']);
            lines.push(format!(
                "  {} -->|{rel_type}| {}",
                node_id[src], node_id[target]
            ));
        }
    }
    lines.push("```".to_string());
    lines
}

/// Renders one section as markdownlint-safe lines (blank lines around
/// headings/lists).
///
/// Includes heading, body, an optional Mermaid graph, key entities,
/// optional dimension/verdict provenance (`meta`), and optional cited
/// evidence (`evidence`).
#[must_use]
pub fn secblock(section: &Value, meta: bool, evidence: bool) -> Vec<String> {
    let heading = section
        .get("heading")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let body = section
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let entities: Vec<Value> = section
        .get("entities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut lines = vec![
        String::new(),
        format!("## {}", detrail(&deglob(heading))),
        String::new(),
        render_body(body),
    ];
    lines.extend(mermaid_graph(section));

    if !entities.is_empty() {
        let names: Vec<String> = entities
            .iter()
            .map(|entity| {
                let name = entity
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let entity_type = entity
                    .get("entityType")
                    .and_then(Value::as_str)
                    .unwrap_or("entity");
                format!("{name} ({entity_type})")
            })
            .collect();
        lines.push(String::new());
        lines.push(format!("Key entities: {}.", names.join(", ")));
    }

    if meta && let Some(dimension) = section.get("dimension").and_then(Value::as_str) {
        let verdict = section
            .get("verdict")
            .and_then(Value::as_str)
            .unwrap_or("n/a");
        let mut meta_line = format!("_Dimension: {dimension} · verification: {verdict}");
        if let Some(entity_type) = section.get("entityType").and_then(Value::as_str) {
            meta_line.push_str(" · type: ");
            meta_line.push_str(entity_type);
            if let Some(ontology) = section.get("ontology").and_then(Value::as_str) {
                meta_line.push_str(" (");
                meta_line.push_str(ontology);
                meta_line.push(')');
            }
        }
        meta_line.push_str("._");
        lines.push(String::new());
        lines.push(meta_line);
    }

    if evidence {
        let sources: Vec<Value> = section
            .get("sources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !sources.is_empty() {
            lines.push(String::new());
            lines.push("Evidence:".to_string());
            lines.push(String::new());
            for source in &sources {
                lines.push(source_link_line(source));
            }
        }
    }

    lines
}

/// Renders one source as a markdown list item: `` - [title](<url>) ``.
///
/// Trims only leading/trailing spaces/tabs from the title (matching the
/// original's `[ \t]` character class, not full Unicode whitespace).
#[must_use]
pub fn source_link_line(source: &Value) -> String {
    let title = source
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let title = title.trim_matches([' ', '\t']);
    let url = source
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("- [{title}](<{url}>)")
}

#[cfg(test)]
mod tests {
    use super::{autolink, deglob, detrail, render_body};

    #[test]
    fn deglob_escapes_bare_asterisks_and_flanked_underscores() {
        let input = "beta * epsilon and llm.token_count.* and see [link](https://x.com/*) \
             and <https://y.com/*a> plain _under_score_ end";
        let expected = "beta \\* epsilon and llm.token_count.\\* and see [link](https://x.com/*) \
             and <https://y.com/*a> plain \\_under_score\\_ end";
        assert_eq!(deglob(input), expected);
    }

    #[test]
    fn deglob_leaves_a_link_target_with_no_special_chars_untouched() {
        assert_eq!(
            deglob("see [x](https://example.com/path)"),
            "see [x](https://example.com/path)"
        );
    }

    #[test]
    fn autolink_wraps_a_bare_url() {
        assert_eq!(
            autolink("see https://example.com/x for details"),
            "see <https://example.com/x> for details"
        );
    }

    #[test]
    fn autolink_wraps_a_bare_email() {
        assert_eq!(
            autolink("contact a.b+tag@example.co.uk today"),
            "contact <a.b+tag@example.co.uk> today"
        );
    }

    #[test]
    fn autolink_does_not_double_wrap_an_existing_autolink() {
        assert_eq!(
            autolink("see <https://example.com/x> here"),
            "see <https://example.com/x> here"
        );
    }

    #[test]
    fn autolink_does_not_wrap_a_url_already_inside_a_markdown_link() {
        assert_eq!(
            autolink("see [text](https://example.com/x) here"),
            "see [text](https://example.com/x) here"
        );
    }

    #[test]
    fn detrail_strips_trailing_spaces_and_tabs_per_line() {
        assert_eq!(detrail("a  \nb\t\nc"), "a\nb\nc");
    }

    #[test]
    fn render_body_leaves_a_fenced_code_block_untouched() {
        let input = "prose with * a star\n```\nraw * code _ block\n```\nmore * prose";
        let rendered = render_body(input);
        assert!(rendered.contains("raw * code _ block"));
        assert!(rendered.contains("prose with \\* a star"));
        assert!(rendered.contains("more \\* prose"));
    }

    #[test]
    fn render_body_keeps_an_unclosed_fence_verbatim_to_the_end() {
        let input = "prose\n```\nunclosed * fence _ content";
        let rendered = render_body(input);
        assert!(rendered.contains("unclosed * fence _ content"));
    }

    use super::{dedupe_sections, mermaid_graph, secblock, source_link_line};

    #[test]
    fn dedupe_sections_disambiguates_repeated_headings() {
        let sections = serde_json::json!([
            {"heading": "Overview"},
            {"heading": "Overview"},
            {"heading": "Overview"},
            {"heading": "Detail"},
        ]);
        let deduped = dedupe_sections(sections.as_array().unwrap());
        let headings: Vec<&str> = deduped
            .iter()
            .map(|s| s["heading"].as_str().unwrap())
            .collect();
        assert_eq!(
            headings,
            ["Overview", "Overview (2)", "Overview (3)", "Detail"]
        );
    }

    #[test]
    fn mermaid_graph_is_empty_when_no_entities_or_relationships() {
        let section = serde_json::json!({"heading": "X", "supports": ["urn:mif:f1"]});
        assert!(mermaid_graph(&section).is_empty());
    }

    #[test]
    fn mermaid_graph_renders_nodes_and_edges_sorted_by_id() {
        let section = serde_json::json!({
            "heading": "Widgets",
            "supports": ["urn:mif:f1"],
            "entities": [{"id": "urn:mif:e2", "name": "Zeta", "entityType": "tool"}],
            "relationships": [{"target": "urn:mif:e2", "type": "supports"}],
        });
        let lines = mermaid_graph(&section);
        assert_eq!(lines[1], "```mermaid");
        assert_eq!(lines[2], "graph TD");
        // Sorted lexicographically: "urn:mif:e2" < "urn:mif:f1" -> n0=e2, n1=f1.
        assert!(lines.iter().any(|l| l.contains("n0[\"Zeta (tool)\"]")));
        assert!(lines.iter().any(|l| l.contains("n1[\"Widgets\"]")));
        assert!(lines.iter().any(|l| l == "  n1 -->|supports| n0"));
        assert_eq!(lines.last().unwrap(), "```");
    }

    #[test]
    fn mermaid_graph_escapes_quotes_and_newlines_in_labels() {
        let section = serde_json::json!({
            "heading": "H",
            "supports": ["urn:mif:f1"],
            "entities": [{"id": "urn:mif:e1", "name": "Weird \"Name\"\nHere", "entityType": "x"}],
            "relationships": [],
        });
        let lines = mermaid_graph(&section);
        let entity_line = lines.iter().find(|l| l.contains("Weird")).unwrap();
        assert!(!entity_line.contains('"') || entity_line.matches('"').count() == 2);
        assert!(!entity_line.contains('\n'));
    }

    #[test]
    fn secblock_includes_meta_line_only_when_requested_and_dimension_present() {
        let section = serde_json::json!({
            "heading": "H", "body": "b", "dimension": "landscape", "verdict": "survived",
        });
        let with_meta = secblock(&section, true, false);
        assert!(
            with_meta
                .iter()
                .any(|l| l.contains("_Dimension: landscape"))
        );
        let without_meta = secblock(&section, false, false);
        assert!(!without_meta.iter().any(|l| l.contains("_Dimension:")));
    }

    #[test]
    fn secblock_includes_evidence_only_when_requested_and_sources_present() {
        let section = serde_json::json!({
            "heading": "H", "body": "b",
            "sources": [{"title": "  A Title  ", "url": "https://x.com"}],
        });
        let with_ev = secblock(&section, false, true);
        assert!(with_ev.iter().any(|l| l == "- [A Title](<https://x.com>)"));
        let without_ev = secblock(&section, false, false);
        assert!(!without_ev.iter().any(|l| l.starts_with("- [")));
    }

    #[test]
    fn source_link_line_trims_only_spaces_and_tabs_from_the_title() {
        let source = serde_json::json!({"title": "\t Padded Title \t", "url": "https://x.com"});
        assert_eq!(
            source_link_line(&source),
            "- [Padded Title](<https://x.com>)"
        );
    }
}
