//! Splits one MIF document into the passages that get embedded.
//!
//! A long document embedded as a single vector is represented only by the
//! text that fits the model's input window, which for a templated document
//! is mostly text every sibling document shares. [`passages`] instead
//! produces a summary passage built from the document's identifying fields,
//! followed by one or more passages per body section, each small enough for
//! the model to read whole.
//!
//! The stored document is never changed: this only decides what text is
//! embedded for it.

use serde_json::Value;

/// The most tokens any passage from [`passages`] holds.
///
/// The model's two special tokens are not counted.
/// `sentence-transformers/all-MiniLM-L6-v2` was trained on inputs of this
/// length, and embedding quality drops past it.
pub const PASSAGE_MAX_TOKENS: usize = 256;

/// The size a section's passages are packed toward.
///
/// It leaves room under [`PASSAGE_MAX_TOKENS`] for the heading prefix and
/// the overlap carried from the previous passage.
pub const PASSAGE_TARGET_TOKENS: usize = 200;

/// How many trailing tokens of a passage open the next one.
///
/// The overlap applies between passages of the same section, so a sentence
/// cut by the split is still read whole in one of them.
pub const PASSAGE_OVERLAP_TOKENS: usize = 32;

/// The `[CLS]` and `[SEP]` tokens the tokenizer adds around every input.
const SPECIAL_TOKENS: usize = 2;

/// Section headings, in lowercase, whose content is not embedded.
///
/// Both hold link lists rather than prose: a `Relationships` section
/// mirrors the frontmatter `relationships`, and a `Navigation` section is a
/// generated footer.
const SKIPPED_SECTIONS: [&str; 2] = ["relationships", "navigation"];

/// One unit of text to embed for a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// The body heading this passage was taken from. `None` for the summary
    /// passage and for body text that precedes the first heading.
    pub heading: Option<String>,
    /// The text to embed: for a section passage, the document title and the
    /// heading on the first line, then the section text.
    pub text: String,
}

/// Builds the passages to embed for the JSON-LD projection of a MIF
/// document.
///
/// The first passage is always the summary: the `title`, the `entity.name`
/// when it is not the title itself, the `description`, and the
/// `entity.definition` or, without one, the first `entity.statements`
/// entry. A document with neither a description nor a definition gets its
/// first body paragraph instead. The summary's `heading` is `None`.
///
/// The remaining passages come from `content`, split at headings of level 1
/// to 3. Markdown link targets, bare URLs, `**Related:**` lines, table rule
/// rows, and the sections named in the module documentation are removed.
/// Each passage starts with `Title: Heading`, and a section longer than
/// [`PASSAGE_TARGET_TOKENS`] is split at paragraph or table-row boundaries
/// with [`PASSAGE_OVERLAP_TOKENS`] of overlap.
///
/// `count_tokens` returns the number of model tokens in a piece of text,
/// without special tokens; [`crate::Embedder::token_count`] is the real one.
/// Every passage returned holds at most [`PASSAGE_MAX_TOKENS`] by that
/// count, provided the title and heading together stay under it. The result
/// is never empty: a document with no usable text yields one summary
/// passage with empty text.
///
/// # Examples
///
/// ```rust
/// use mif_embed::passages::passages;
///
/// let document = serde_json::json!({
///     "title": "Record: Campaign Management",
///     "entity": { "name": "Campaign Management", "definition": "The ability to run campaigns" },
///     "content": "# Record: Campaign Management\n\n## Scope\n\nEmail and events.\n",
/// });
/// let words = |text: &str| text.split_whitespace().count();
/// let passages = passages(&document, &words);
///
/// assert_eq!(passages[0].heading, None);
/// assert!(passages[0].text.contains("The ability to run campaigns"));
/// assert_eq!(passages[1].heading.as_deref(), Some("Scope"));
/// assert_eq!(passages[1].text, "Record: Campaign Management: Scope\nEmail and events.");
/// ```
#[must_use]
pub fn passages(jsonld: &Value, count_tokens: &dyn Fn(&str) -> usize) -> Vec<Passage> {
    let title = text_field(jsonld, "title");
    let content = text_field(jsonld, "content").unwrap_or_default();
    let sections = split_sections(content);

    let mut out = vec![summary(jsonld, title, &sections, count_tokens)];
    for section in &sections {
        let prefix = prefix(title, section.heading.as_deref());
        out.extend(
            pack(&prefix, &section.units, count_tokens)
                .into_iter()
                .map(|text| Passage {
                    heading: section.heading.clone(),
                    text,
                }),
        );
    }
    out
}

/// A non-empty, trimmed string field of a JSON object.
fn text_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// The summary passage: the document's identifying fields, one per line.
fn summary(
    jsonld: &Value,
    title: Option<&str>,
    sections: &[Section],
    count_tokens: &dyn Fn(&str) -> usize,
) -> Passage {
    let entity = jsonld.get("entity");
    let name = entity
        .and_then(|entity| text_field(entity, "name"))
        .filter(|name| Some(*name) != title);
    let description = text_field(jsonld, "description");
    let definition = entity.and_then(|entity| {
        text_field(entity, "definition").or_else(|| {
            entity
                .get("statements")
                .and_then(Value::as_array)
                .and_then(|statements| statements.first())
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|statement| !statement.is_empty())
        })
    });
    let first_paragraph = if description.is_none() && definition.is_none() {
        sections
            .iter()
            .find_map(|section| section.units.first())
            .map(String::as_str)
    } else {
        None
    };

    let text = [title, name, description, definition, first_paragraph]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    Passage {
        heading: None,
        text: pack("", &[text], count_tokens)
            .into_iter()
            .next()
            .unwrap_or_default(),
    }
}

/// The first line of a section passage: the title and heading that say
/// what the text below is about.
fn prefix(title: Option<&str>, heading: Option<&str>) -> String {
    match (title, heading) {
        (Some(title), Some(heading)) if title != heading => format!("{title}: {heading}\n"),
        (Some(line), _) | (None, Some(line)) => format!("{line}\n"),
        (None, None) => String::new(),
    }
}

/// One body section: its heading and its paragraphs and table rows, cleaned.
#[derive(Debug, Default)]
struct Section {
    heading: Option<String>,
    units: Vec<String>,
}

/// Accumulates body lines into [`Section`]s.
#[derive(Debug, Default)]
struct Splitter {
    done: Vec<Section>,
    current: Section,
    paragraph: Vec<String>,
    in_fence: bool,
    /// The level of the skipped section being passed over, if any.
    skipping: Option<usize>,
}

impl Splitter {
    fn end_paragraph(&mut self) {
        if !self.paragraph.is_empty() {
            self.current.units.push(self.paragraph.join("\n"));
            self.paragraph.clear();
        }
    }

    fn end_section(&mut self) {
        self.end_paragraph();
        let section = std::mem::take(&mut self.current);
        if !section.units.is_empty() {
            self.done.push(section);
        }
    }

    fn heading(&mut self, level: usize, text: &str) {
        self.end_section();
        if self.skipping.is_some_and(|skipped| level <= skipped) {
            self.skipping = None;
        }
        if self.skipping.is_some() {
            return;
        }
        let heading = clean_inline(text);
        if SKIPPED_SECTIONS.contains(&heading.to_lowercase().as_str()) {
            self.skipping = Some(level);
            return;
        }
        self.current.heading = Some(heading).filter(|heading| !heading.is_empty());
    }

    fn line(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            self.in_fence = !self.in_fence;
            return;
        }
        if self.in_fence {
            if self.skipping.is_none() && !trimmed.is_empty() {
                self.paragraph.push(trimmed.to_string());
            }
            return;
        }
        let text = match heading(trimmed) {
            Some((level, text)) if level <= 3 => return self.heading(level, text),
            Some((_, text)) => text,
            None => trimmed,
        };
        if self.skipping.is_some() {
            return;
        }
        if text.is_empty() || is_rule(text) {
            return self.end_paragraph();
        }
        if text.starts_with("**Related:**") {
            return;
        }
        let cleaned = clean_inline(text);
        if cleaned.is_empty() {
            return;
        }
        if text.starts_with('|') {
            // Each table row is its own unit, so a long table splits
            // between rows rather than through one.
            self.end_paragraph();
            self.current.units.push(cleaned);
        } else {
            self.paragraph.push(cleaned);
        }
    }
}

/// Splits a markdown body into sections at headings of level 1 to 3.
fn split_sections(content: &str) -> Vec<Section> {
    let mut splitter = Splitter::default();
    for line in content.lines() {
        splitter.line(line);
    }
    splitter.end_section();
    splitter.done
}

/// The level and text of an ATX heading line (`## Text`).
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line.get(level..)?;
    rest.starts_with(' ')
        .then(|| (level, rest.trim().trim_end_matches('#').trim_end()))
}

/// Whether a line is a table rule row (`|---|:--|`) or a horizontal rule
/// (`---`, `***`, `___`): structure with no words in it.
fn is_rule(line: &str) -> bool {
    let marks = line
        .chars()
        .filter(|c| matches!(c, '-' | '*' | '_'))
        .count();
    marks >= 3
        && line
            .chars()
            .all(|c| matches!(c, '-' | '*' | '_' | '|' | ':' | ' '))
}

/// Removes what a reader does not read as words: markdown link and image
/// targets (the link text stays) and bare URLs. Runs of whitespace collapse
/// to one space.
fn clean_inline(line: &str) -> String {
    strip_link_targets(line)
        .split_whitespace()
        .filter(|word| {
            let word = word.trim_start_matches(['<', '(']);
            !(word.starts_with("http://") || word.starts_with("https://"))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Replaces every `[text](target)` and `![alt](target)` with its text.
fn strip_link_targets(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let (before, bracketed) = rest.split_at(open);
        if let Some((text, after)) = link_text(bracketed) {
            out.push_str(before.strip_suffix('!').unwrap_or(before));
            out.push_str(text);
            rest = after;
        } else {
            out.push_str(before);
            out.push('[');
            rest = bracketed.get(1..).unwrap_or_default();
        }
    }
    out.push_str(rest);
    out
}

/// For text starting at the `[` of an inline link, the link text and
/// whatever follows the closing `)` of its target.
fn link_text(link: &str) -> Option<(&str, &str)> {
    let close = link.find("](")?;
    let text = link.get(1..close)?;
    if text.contains('[') {
        return None;
    }
    let target = link.get(close + 2..)?;
    let end = target.find(')')?;
    Some((text, target.get(end + 1..)?))
}

/// Packs a section's units into passage texts of about
/// [`PASSAGE_TARGET_TOKENS`], each starting with `prefix` and, after the
/// first, with the tail of the passage before it.
fn pack(prefix: &str, units: &[String], count_tokens: &dyn Fn(&str) -> usize) -> Vec<String> {
    let budget = PASSAGE_MAX_TOKENS.saturating_sub(SPECIAL_TOKENS + count_tokens(prefix));
    let target = PASSAGE_TARGET_TOKENS
        .min(budget.saturating_sub(PASSAGE_OVERLAP_TOKENS))
        .max(1);

    let mut out = Vec::new();
    let mut chunk: Vec<String> = Vec::new();
    let mut tokens = 0;
    let mut fresh = false;
    let pieces = units
        .iter()
        .flat_map(|unit| split_unit(unit, target, count_tokens));
    for (piece, size) in pieces {
        if fresh && tokens + size > target {
            let text = chunk.join("\n");
            let overlap = tail(&text, PASSAGE_OVERLAP_TOKENS, count_tokens);
            out.push(format!("{prefix}{text}"));
            chunk.clear();
            chunk.extend(Some(overlap).filter(|overlap| !overlap.is_empty()));
            tokens = 0;
        }
        chunk.push(piece);
        tokens += size;
        fresh = true;
    }
    if fresh {
        out.push(format!("{prefix}{}", chunk.join("\n")));
    }
    out
}

/// A unit and its token count, or, when it is larger than `target`, the
/// unit cut between words into pieces that each fit. A single word larger
/// than `target` is an encoded blob, not prose, and is dropped.
fn split_unit(
    unit: &str,
    target: usize,
    count_tokens: &dyn Fn(&str) -> usize,
) -> Vec<(String, usize)> {
    let size = count_tokens(unit);
    if size <= target {
        return vec![(unit.to_string(), size)];
    }
    let mut out = Vec::new();
    let mut words: Vec<&str> = Vec::new();
    let mut tokens = 0;
    for word in unit.split_whitespace() {
        let word_tokens = count_tokens(word);
        if word_tokens > target {
            continue;
        }
        if tokens + word_tokens > target {
            out.push((words.join(" "), tokens));
            words.clear();
            tokens = 0;
        }
        words.push(word);
        tokens += word_tokens;
    }
    if !words.is_empty() {
        out.push((words.join(" "), tokens));
    }
    out
}

/// The last whole words of `text` that together stay within `max` tokens.
fn tail(text: &str, max: usize, count_tokens: &dyn Fn(&str) -> usize) -> String {
    let mut words = Vec::new();
    let mut tokens = 0;
    for word in text.split_whitespace().rev() {
        tokens += count_tokens(word);
        if tokens > max {
            break;
        }
        words.push(word);
    }
    words.reverse();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        PASSAGE_MAX_TOKENS, PASSAGE_OVERLAP_TOKENS, PASSAGE_TARGET_TOKENS, Passage, passages,
    };

    /// One token per whitespace-separated word: additive across joins, like
    /// the real tokenizer counted without special tokens.
    fn words(text: &str) -> usize {
        text.split_whitespace().count()
    }

    fn build(document: &serde_json::Value) -> Vec<Passage> {
        passages(document, &words)
    }

    fn numbered(prefix: &str, count: usize) -> String {
        (0..count)
            .map(|n| format!("{prefix}{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn summary_holds_title_name_description_and_definition_in_that_order() {
        let built = build(&json!({
            "title": "Capability record: C04.02 Campaign Management",
            "description": "How campaigns are run",
            "entity": {
                "name": "Campaign Management",
                "definition": "The ability to plan and run campaigns",
                "statements": ["Not used when a definition exists"],
            },
            "content": "Body paragraph.",
        }));

        assert_eq!(built[0].heading, None);
        assert_eq!(
            built[0].text,
            "Capability record: C04.02 Campaign Management\nCampaign Management\n\
             How campaigns are run\nThe ability to plan and run campaigns"
        );
    }

    #[test]
    fn summary_leaves_out_an_entity_name_equal_to_the_title() {
        let built = build(&json!({
            "title": "Campaign Management",
            "entity": { "name": "Campaign Management", "definition": "Runs campaigns" },
        }));

        assert_eq!(built[0].text, "Campaign Management\nRuns campaigns");
    }

    #[test]
    fn summary_uses_the_first_statement_when_there_is_no_definition() {
        let built = build(&json!({
            "title": "CON-P-08",
            "entity": { "statements": ["The process shall record consent.", "Second."] },
            "content": "Body paragraph.",
        }));

        assert_eq!(built[0].text, "CON-P-08\nThe process shall record consent.");
    }

    #[test]
    fn summary_falls_back_to_the_first_body_paragraph() {
        let built = build(&json!({
            "title": "Meeting",
            "content": "# Meeting\n\nWe agreed the plan.\n\nSecond paragraph.",
        }));

        assert_eq!(built[0].text, "Meeting\nWe agreed the plan.");
    }

    #[test]
    fn a_document_with_no_text_still_yields_one_summary_passage() {
        let built = build(&json!({ "@id": "urn:mif:x" }));

        assert_eq!(
            built,
            vec![Passage {
                heading: None,
                text: String::new()
            }]
        );
    }

    #[test]
    fn sections_split_at_heading_levels_one_to_three_and_carry_title_and_heading() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "Preamble.\n\n# Doc\n\nIntro.\n\n## Scope\n\nIn scope.\n\n\
                        ### Detail\n\nFine print.\n\n#### Deeper\n\nStill detail.",
        }));

        let sections: Vec<_> = built
            .iter()
            .skip(1)
            .map(|p| (p.heading.as_deref(), p.text.as_str()))
            .collect();
        assert_eq!(
            sections,
            vec![
                (None, "Doc\nPreamble."),
                (Some("Doc"), "Doc\nIntro."),
                (Some("Scope"), "Doc: Scope\nIn scope."),
                (
                    Some("Detail"),
                    "Doc: Detail\nFine print.\nDeeper\nStill detail."
                ),
            ]
        );
    }

    #[test]
    fn relationships_and_navigation_sections_are_dropped_with_their_subsections() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "## Scope\n\nIn scope.\n\n## Relationships\n\n- relates-to [A](a.md)\n\n\
                        ### Inbound\n\n- from [B](b.md)\n\n## Sources\n\nA book.\n\n---\n\n\
                        ## Navigation\n\n[Home](../README.md)",
        }));

        let headings: Vec<_> = built.iter().filter_map(|p| p.heading.as_deref()).collect();
        assert_eq!(headings, vec!["Scope", "Sources"]);
    }

    #[test]
    fn link_targets_urls_related_lines_and_rule_rows_are_stripped() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "## Body\n\n**Purpose:** See [the standard](../04/Standard.md) and \
                        ![a figure](fig.png).\n**Related:** [X](x.md), [Y](y.md)\n\
                        Read https://example.com/a/b or <https://example.com> first.\n\n\
                        | Name | Value |\n|---|:--|\n| [Row](r.md) | 1 |\n\n\
                        Keep [[wiki]] and [unclosed text.",
        }));

        assert_eq!(
            built[1].text,
            "Doc: Body\n**Purpose:** See the standard and a figure.\nRead or first.\n\
             | Name | Value |\n| Row | 1 |\nKeep [[wiki]] and [unclosed text."
        );
    }

    #[test]
    fn fenced_code_is_kept_as_text_and_its_hashes_are_not_headings() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "## Run\n\n```bash\n# not a heading\ncargo test\n```\n",
        }));

        assert_eq!(built.len(), 2);
        assert_eq!(built[1].text, "Doc: Run\n# not a heading\ncargo test");
    }

    #[test]
    fn a_long_section_splits_at_paragraphs_with_overlap_and_every_passage_fits() {
        let paragraphs: Vec<String> = (0..6).map(|p| numbered(&format!("p{p}w"), 90)).collect();
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": format!("## Long\n\n{}", paragraphs.join("\n\n")),
        }));
        let long: Vec<_> = built.iter().skip(1).collect();

        // Two 90-word paragraphs fit the 200-token target; a third does not.
        assert_eq!(long.len(), 3);
        for passage in &long {
            assert_eq!(passage.heading.as_deref(), Some("Long"));
            assert!(passage.text.starts_with("Doc: Long\n"));
            assert!(words(&passage.text) <= PASSAGE_MAX_TOKENS);
        }
        // The second passage opens with the last words of the first.
        let overlap = numbered("p1w", 90)
            .split_whitespace()
            .skip(90 - PASSAGE_OVERLAP_TOKENS)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            long[1]
                .text
                .starts_with(&format!("Doc: Long\n{overlap}\np2w0 "))
        );
        assert!(long[0].text.ends_with(&overlap));
    }

    #[test]
    fn a_single_oversized_paragraph_is_cut_between_words() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": format!("## Wall\n\n{}", numbered("w", 450)),
        }));
        let wall: Vec<_> = built.iter().skip(1).collect();

        assert_eq!(wall.len(), 3);
        for passage in &wall {
            assert!(words(&passage.text) <= PASSAGE_MAX_TOKENS);
        }
        // Every word survives the cut.
        let text = wall
            .iter()
            .map(|p| p.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for n in 0..450 {
            assert!(
                text.split_whitespace().any(|w| w == format!("w{n}")),
                "w{n} lost"
            );
        }
    }

    #[test]
    fn table_rows_are_split_between_rows() {
        let rows: Vec<String> = (0..10)
            .map(|r| format!("| {} |", numbered(&format!("r{r}c"), 40)))
            .collect();
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": format!("## Table\n\n| Head |\n|---|\n{}", rows.join("\n")),
        }));

        for passage in built.iter().skip(1) {
            assert!(words(&passage.text) <= PASSAGE_MAX_TOKENS);
            // Apart from the overlap line, each line is one whole row.
            for line in passage.text.lines().skip(2) {
                assert!(
                    line.starts_with('|') && line.ends_with('|'),
                    "cut row: {line}"
                );
            }
        }
    }

    #[test]
    fn a_word_too_large_to_fit_any_passage_is_dropped() {
        let blob = "x".repeat(10);
        let count = |text: &str| {
            text.split_whitespace()
                .map(|w| if w == blob { 1000 } else { 1 })
                .sum()
        };
        let built = passages(
            &json!({
                "title": "Doc",
                "entity": { "definition": "d" },
                "content": format!("## Data\n\nbefore {blob} after"),
            }),
            &count,
        );

        assert_eq!(built[1].text, "Doc: Data\nbefore after");
    }

    #[test]
    fn an_oversized_summary_is_cut_to_the_target() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": numbered("d", 600) },
        }));

        assert_eq!(words(&built[0].text), PASSAGE_TARGET_TOKENS);
        assert!(built[0].text.starts_with("Doc d0 d1 "));
    }
}
