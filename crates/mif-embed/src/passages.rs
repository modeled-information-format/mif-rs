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

/// The most tokens of the title, and of the heading, a passage prefix holds.
///
/// A longer title or heading is cut to its first words, so the prefix can
/// never crowd the section text out of the passage.
const PREFIX_PART_MAX_TOKENS: usize = 32;

/// The `[CLS]` and `[SEP]` tokens the tokenizer adds around every input.
const SPECIAL_TOKENS: usize = 2;

/// Section headings, in lowercase, whose content is not embedded.
///
/// Both hold link lists rather than prose: a `Relationships` section
/// mirrors the frontmatter `relationships`, and a `Navigation` section is a
/// generated footer.
const SKIPPED_SECTIONS: [&str; 2] = ["relationships", "navigation"];

/// The markers that open and close a fenced code block.
const FENCES: [&str; 2] = ["```", "~~~"];

/// Where a stretch of text sits in a document's body, the `content` string
/// of its JSON-LD projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// The offset of the first character, counted in Unicode scalar values
    /// from the start of the body. The first character is 0.
    pub start_char: usize,
    /// The offset just past the last character.
    pub end_char: usize,
    /// The line the text starts on. The body's first line is 1.
    pub start_line: usize,
    /// The line the text ends on.
    pub end_line: usize,
}

/// One body section as it is written, from [`sections`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSection {
    /// The section's heading, cleaned the way [`Passage::heading`] is.
    /// `None` for body text that precedes the first heading, and for a
    /// section whose heading has no words left once cleaned.
    pub heading: Option<String>,
    /// The section's markdown, unchanged: its heading line, then everything
    /// up to the next heading of level 1 to 3.
    pub text: String,
    /// Where `text` sits in the body.
    pub span: Span,
}

/// One unit of text to embed for a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    /// The body heading this passage was taken from. `None` for the summary
    /// passage and for body text that precedes the first heading.
    pub heading: Option<String>,
    /// The text to embed: for a section passage, the document title and the
    /// heading on the first line, then the section text.
    pub text: String,
    /// The passage's own text: `text` without the title and heading line
    /// and without the words carried over from the passage before it.
    pub body: String,
    /// Where the paragraphs and table rows `body` was taken from sit in the
    /// document's body. `None` for the summary passage. A paragraph too
    /// long for one passage is reported whole by each passage it is cut
    /// into.
    pub span: Option<Span>,
    /// The index, in [`sections`], of the section this passage came from.
    /// `None` for the summary passage.
    pub section: Option<usize>,
}

/// One line of a document's body and where it sits.
#[derive(Debug, Clone, Copy)]
struct Loc {
    line: usize,
    byte_start: usize,
    byte_end: usize,
    char_start: usize,
    char_end: usize,
}

/// A run of body lines, from its first to its last.
#[derive(Debug, Clone, Copy)]
struct Range {
    first: Loc,
    last: Loc,
}

impl Range {
    const fn line(loc: Loc) -> Self {
        Self {
            first: loc,
            last: loc,
        }
    }

    const fn span(self) -> Span {
        Span {
            start_char: self.first.char_start,
            end_char: self.last.char_end,
            start_line: self.first.line,
            end_line: self.last.line,
        }
    }
}

/// The range from the start of `a` to the end of `b`, when both exist.
const fn join(a: Option<Range>, b: Option<Range>) -> Option<Range> {
    match (a, b) {
        (Some(a), Some(b)) => Some(Range {
            first: a.first,
            last: b.last,
        }),
        (Some(range), None) | (None, Some(range)) => Some(range),
        (None, None) => None,
    }
}

/// The body of a document: the `content` string of its JSON-LD projection.
fn body(jsonld: &Value) -> &str {
    jsonld
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The body sections of the JSON-LD projection of a MIF document, as they
/// are written.
///
/// These are the sections [`passages`] takes its body passages from, in the
/// same order, so [`Passage::section`] indexes this list. A section with no
/// text of its own, and the sections [`passages`] leaves out, are not
/// listed.
///
/// # Examples
///
/// ```rust
/// use mif_embed::passages::sections;
///
/// let document = serde_json::json!({
///     "content": "Intro.\n\n## Scope\n\nSee [the plan](plan.md).\n",
/// });
/// let sections = sections(&document);
///
/// assert_eq!(sections[1].heading.as_deref(), Some("Scope"));
/// assert_eq!(sections[1].text, "## Scope\n\nSee [the plan](plan.md).");
/// assert_eq!(sections[1].span.start_line, 3);
/// ```
#[must_use]
pub fn sections(jsonld: &Value) -> Vec<DocSection> {
    let content = body(jsonld);
    split_sections(content)
        .into_iter()
        .map(|section| DocSection {
            heading: section.heading,
            text: content
                .get(section.range.first.byte_start..section.range.last.byte_end)
                .unwrap_or_default()
                .to_string(),
            span: section.range.span(),
        })
        .collect()
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
/// count: a title or heading too long for the prefix is cut to its first
/// words there, and kept whole in [`Passage::heading`]. The result is never
/// empty: a document with no usable text yields one summary
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
    let sections = split_sections(body(jsonld));

    let mut out = vec![summary(jsonld, title, &sections, count_tokens)];
    for (index, section) in sections.iter().enumerate() {
        let prefix = prefix(title, section.heading.as_deref(), count_tokens);
        out.extend(
            pack(&prefix, &section.units, count_tokens)
                .into_iter()
                .map(|packed| Passage {
                    heading: section.heading.clone(),
                    text: packed.text,
                    body: packed.body,
                    span: packed.range.map(Range::span),
                    section: Some(index),
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
            .map(|unit| unit.text.as_str())
    } else {
        None
    };

    let text = [title, name, description, definition, first_paragraph]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let unit = Unit { text, range: None };
    let text = pack("", &[unit], count_tokens)
        .into_iter()
        .next()
        .map(|packed| packed.text)
        .unwrap_or_default();
    Passage {
        heading: None,
        body: text.clone(),
        text,
        span: None,
        section: None,
    }
}

/// The first line of a section passage: the title and heading that say
/// what the text below is about.
fn prefix(
    title: Option<&str>,
    heading: Option<&str>,
    count_tokens: &dyn Fn(&str) -> usize,
) -> String {
    let short = |text: &str| head(text, PREFIX_PART_MAX_TOKENS, count_tokens);
    match (title, heading) {
        (Some(title), Some(heading)) if title != heading => {
            format!("{}: {}\n", short(title), short(heading))
        },
        (Some(line), _) | (None, Some(line)) => format!("{}\n", short(line)),
        (None, None) => String::new(),
    }
}

/// One paragraph or table row, cleaned, and the body lines it came from.
/// Only the summary's text has no lines.
#[derive(Debug)]
struct Unit {
    text: String,
    range: Option<Range>,
}

/// One body section: its heading, its paragraphs and table rows, and the
/// body lines it covers.
#[derive(Debug)]
struct Section {
    heading: Option<String>,
    units: Vec<Unit>,
    range: Range,
}

/// The section being read.
#[derive(Debug, Default)]
struct OpenSection {
    heading: Option<String>,
    units: Vec<Unit>,
    /// The section's lines so far, from its heading line to its last line
    /// that is not blank.
    range: Option<Range>,
}

/// Accumulates body lines into [`Section`]s.
#[derive(Debug, Default)]
struct Splitter {
    done: Vec<Section>,
    current: OpenSection,
    paragraph: Vec<String>,
    /// The lines the paragraph being read came from.
    paragraph_range: Option<Range>,
    /// The marker that opened the code fence being read, if any.
    fence: Option<&'static str>,
    /// The level of the skipped section being passed over, if any.
    skipping: Option<usize>,
}

impl Splitter {
    /// Counts a line as part of the section being read.
    const fn touch(&mut self, loc: Loc) {
        self.current.range = join(self.current.range, Some(Range::line(loc)));
    }

    fn push_paragraph(&mut self, text: String, loc: Loc) {
        self.paragraph.push(text);
        self.paragraph_range = join(self.paragraph_range, Some(Range::line(loc)));
    }

    fn end_paragraph(&mut self) {
        let range = self.paragraph_range.take();
        if !self.paragraph.is_empty() {
            self.current.units.push(Unit {
                text: self.paragraph.join("\n"),
                range,
            });
            self.paragraph.clear();
        }
    }

    fn end_section(&mut self) {
        self.end_paragraph();
        let section = std::mem::take(&mut self.current);
        if let Some(range) = section.range
            && !section.units.is_empty()
        {
            self.done.push(Section {
                heading: section.heading,
                units: section.units,
                range,
            });
        }
    }

    fn heading(&mut self, level: usize, text: &str, loc: Loc) {
        self.end_section();
        if self.skipping.is_some_and(|skipped| level <= skipped) {
            self.skipping = None;
        }
        if self.skipping.is_some() {
            return;
        }
        let heading = clean_inline(text);
        // A level-1 heading is the document's own title: a document titled
        // "Relationships" is about relationships, and keeps its body.
        if level > 1 && SKIPPED_SECTIONS.contains(&heading.to_lowercase().as_str()) {
            self.skipping = Some(level);
            return;
        }
        self.current.heading = Some(heading).filter(|heading| !heading.is_empty());
        self.touch(loc);
    }

    fn line(&mut self, line: &str, loc: Loc) {
        let trimmed = line.trim();
        if let Some(marker) = self.fence {
            if self.skipping.is_none() && !trimmed.is_empty() {
                self.touch(loc);
            }
            if trimmed.starts_with(marker) {
                self.fence = None;
            } else if self.skipping.is_none() && !trimmed.is_empty() {
                self.push_paragraph(trimmed.to_string(), loc);
            }
            return;
        }
        if let Some(marker) = opening_fence(trimmed) {
            self.fence = Some(marker);
            if self.skipping.is_none() {
                self.touch(loc);
            }
            return;
        }
        let text = match heading(trimmed) {
            Some((level, text)) if level <= 3 => return self.heading(level, text, loc),
            Some((_, text)) => text,
            None => trimmed,
        };
        if self.skipping.is_some() {
            return;
        }
        if !trimmed.is_empty() {
            self.touch(loc);
        }
        if text.is_empty() || is_rule(text) {
            return self.end_paragraph();
        }
        let unmarked = ["- ", "* ", "+ ", "> "]
            .iter()
            .find_map(|marker| text.strip_prefix(marker))
            .unwrap_or(text);
        if unmarked.trim_start().starts_with("**Related:**") {
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
            self.current.units.push(Unit {
                text: cleaned,
                range: Some(Range::line(loc)),
            });
        } else {
            self.push_paragraph(cleaned, loc);
        }
    }
}

/// Splits a markdown body into sections at headings of level 1 to 3.
fn split_sections(content: &str) -> Vec<Section> {
    let mut splitter = Splitter::default();
    let mut byte = 0;
    let mut chars = 0;
    for (index, raw) in content.split_inclusive('\n').enumerate() {
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let loc = Loc {
            line: index + 1,
            byte_start: byte,
            byte_end: byte + line.len(),
            char_start: chars,
            char_end: chars + line.chars().count(),
        };
        splitter.line(line, loc);
        byte += raw.len();
        chars += raw.chars().count();
    }
    splitter.end_section();
    splitter.done
}

/// The marker of the code fence a line opens. A line that also closes its
/// own fence (```` ```code``` ````) opens nothing and is read as text.
fn opening_fence(line: &str) -> Option<&'static str> {
    FENCES.into_iter().find(|marker| {
        line.strip_prefix(marker)
            .is_some_and(|rest| !rest.contains(marker))
    })
}

/// The level and text of an ATX heading line (`## Text`).
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = line.get(level..)?;
    if !rest.starts_with(' ') {
        return None;
    }
    // A closing run of `#` counts only when a space sets it off, so the
    // heading "C#" keeps its name.
    let text = rest.trim();
    let unclosed = text.trim_end_matches('#');
    let closed = unclosed.is_empty() || unclosed.ends_with(' ');
    Some((level, if closed { unclosed.trim_end() } else { text }))
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
    // The target ends at the `)` that matches the opening one, so a target
    // with parentheses of its own (`Foo_(bar)`) is removed whole.
    let mut depth = 0_usize;
    let end = target.char_indices().find_map(|(index, c)| match c {
        '(' => {
            depth += 1;
            None
        },
        ')' if depth == 0 => Some(index),
        ')' => {
            depth -= 1;
            None
        },
        _ => None,
    })?;
    Some((text, target.get(end + 1..)?))
}

/// Packs a section's units into passage texts of about
/// [`PASSAGE_TARGET_TOKENS`], each starting with `prefix` and, after the
/// first, with the tail of the passage before it.
fn pack(prefix: &str, units: &[Unit], count_tokens: &dyn Fn(&str) -> usize) -> Vec<Packed> {
    let budget = PASSAGE_MAX_TOKENS.saturating_sub(SPECIAL_TOKENS + count_tokens(prefix));
    let target = PASSAGE_TARGET_TOKENS
        .min(budget.saturating_sub(PASSAGE_OVERLAP_TOKENS))
        .max(1);

    let mut out = Vec::new();
    // What the passage being packed embeds: the overlap, then its own pieces.
    let mut chunk: Vec<String> = Vec::new();
    let mut own: Vec<String> = Vec::new();
    let mut range = None;
    let mut tokens = 0;
    let mut fresh = false;
    let pieces = units.iter().flat_map(|unit| {
        split_unit(&unit.text, target, count_tokens)
            .into_iter()
            .map(|(piece, size)| (piece, size, unit.range))
    });
    for (piece, size, piece_range) in pieces {
        if fresh && tokens + size > target {
            let text = chunk.join("\n");
            let overlap = tail(&text, PASSAGE_OVERLAP_TOKENS, count_tokens);
            out.push(Packed {
                text: format!("{prefix}{text}"),
                body: own.join("\n"),
                range,
            });
            chunk.clear();
            own.clear();
            range = None;
            chunk.extend(Some(overlap).filter(|overlap| !overlap.is_empty()));
            tokens = 0;
        }
        chunk.push(piece.clone());
        own.push(piece);
        range = join(range, piece_range);
        tokens += size;
        fresh = true;
    }
    if fresh {
        out.push(Packed {
            text: format!("{prefix}{}", chunk.join("\n")),
            body: own.join("\n"),
            range,
        });
    }
    out
}

/// One passage out of [`pack`].
#[derive(Debug)]
struct Packed {
    /// The text to embed: the prefix, the overlap, and the passage's own
    /// pieces.
    text: String,
    /// The passage's own pieces alone.
    body: String,
    /// The body lines those pieces came from.
    range: Option<Range>,
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

/// The first whole words of `text` that together stay within `max` tokens.
fn head(text: &str, max: usize, count_tokens: &dyn Fn(&str) -> usize) -> String {
    let mut words = Vec::new();
    let mut tokens = 0;
    for word in text.split_whitespace() {
        tokens += count_tokens(word);
        if tokens > max {
            break;
        }
        words.push(word);
    }
    words.join(" ")
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
        PASSAGE_MAX_TOKENS, PASSAGE_OVERLAP_TOKENS, PASSAGE_TARGET_TOKENS, Passage, Span, passages,
        sections,
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
                text: String::new(),
                body: String::new(),
                span: None,
                section: None,
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

    #[test]
    fn a_document_titled_like_a_skipped_section_keeps_its_body() {
        let built = build(&json!({
            "title": "Relationships",
            "entity": { "definition": "d" },
            "content": "# Relationships\n\nHow teams relate.\n\n## Detail\n\nMore.\n\n\
                        ## Navigation\n\n[Home](../README.md)",
        }));

        let texts: Vec<_> = built.iter().skip(1).map(|p| p.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "Relationships\nHow teams relate.",
                "Relationships: Detail\nMore."
            ]
        );
    }

    #[test]
    fn a_long_title_and_heading_are_cut_in_the_prefix_and_text_still_fits() {
        let title = numbered("t", 300);
        let heading = numbered("h", 300);
        let built = build(&json!({
            "title": title,
            "entity": { "definition": "d" },
            "content": format!("## {heading}\n\n{}", numbered("w", 400)),
        }));
        let section: Vec<_> = built.iter().skip(1).collect();

        // 400 words at a target of 157 (256 - 2 special - 65 prefix - 32 overlap).
        assert_eq!(section.len(), 3);
        for passage in &section {
            assert!(words(&passage.text) <= PASSAGE_MAX_TOKENS);
            assert!(passage.text.starts_with("t0 t1 "));
            assert!(passage.text.contains(" t31: h0 "));
            assert!(!passage.text.contains("t32"));
            assert!(!passage.text.contains("h32"));
        }
        assert_eq!(section[0].heading.as_deref(), Some(heading.as_str()));
    }

    #[test]
    fn a_fence_closed_on_its_own_line_does_not_swallow_the_rest() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "## Run\n\n```cargo test```\n\n## Scope\n\nIn scope.\n\n\
                        ## Relationships\n\n- relates-to [A](a.md)",
        }));

        let headings: Vec<_> = built.iter().filter_map(|p| p.heading.as_deref()).collect();
        assert_eq!(headings, vec!["Run", "Scope"]);
    }

    #[test]
    fn a_fence_closes_only_with_the_marker_that_opened_it() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "## Run\n\n```text\n~~~\n## not a heading\n```\n\n## After\n\nDone.",
        }));

        let sections: Vec<_> = built
            .iter()
            .skip(1)
            .map(|p| (p.heading.as_deref(), p.text.as_str()))
            .collect();
        assert_eq!(
            sections,
            vec![
                (Some("Run"), "Doc: Run\n~~~\n## not a heading"),
                (Some("After"), "Doc: After\nDone."),
            ]
        );
    }

    #[test]
    fn a_heading_keeps_a_trailing_hash_that_is_part_of_its_name() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "## Migrating to C#\n\nA.\n\n## Closed ##\n\nB.",
        }));

        let headings: Vec<_> = built.iter().filter_map(|p| p.heading.as_deref()).collect();
        assert_eq!(headings, vec!["Migrating to C#", "Closed"]);
    }

    #[test]
    fn a_link_target_with_parentheses_is_removed_whole() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "See [Foo](https://en.wikipedia.org/wiki/Foo_(bar)) and \
                        [Plan](docs/Plan_(v2).md) now.",
        }));

        assert_eq!(built[1].text, "Doc\nSee Foo and Plan now.");
    }

    #[test]
    fn related_lines_written_as_list_items_or_quotes_are_dropped() {
        let built = build(&json!({
            "title": "Doc",
            "entity": { "definition": "d" },
            "content": "Kept.\n- **Related:** [A](a.md), [B](b.md)\n> **Related:** [C](c.md)\n\
                        - A list item about Related: things stays.",
        }));

        assert_eq!(
            built[1].text,
            "Doc\nKept.\n- A list item about Related: things stays."
        );
    }

    /// The characters of `content` a span covers.
    fn slice(content: &str, span: Span) -> String {
        content
            .chars()
            .skip(span.start_char)
            .take(span.end_char - span.start_char)
            .collect()
    }

    #[test]
    fn a_passage_span_covers_the_source_lines_of_its_own_text() {
        let content = "Préambule.\n\n## Scope\n\nSee [the plan](plan.md) now.\nSecond line.\n\n\
                       | a | b |\n|---|---|\n| c | d |\n";
        let built =
            build(&json!({ "title": "Doc", "entity": { "definition": "d" }, "content": content }));

        assert_eq!(built[0].span, None);
        assert_eq!(built[0].section, None);
        assert_eq!(built[0].body, built[0].text);

        let preamble = built[1].span.unwrap();
        assert_eq!((preamble.start_line, preamble.end_line), (1, 1));
        assert_eq!(slice(content, preamble), "Préambule.");

        let scope = &built[2];
        assert_eq!(scope.section, Some(1));
        assert_eq!(
            scope.body,
            "See the plan now.\nSecond line.\n| a | b |\n| c | d |"
        );
        let span = scope.span.unwrap();
        assert_eq!((span.start_line, span.end_line), (5, 10));
        assert_eq!(
            slice(content, span),
            "See [the plan](plan.md) now.\nSecond line.\n\n| a | b |\n|---|---|\n| c | d |"
        );
    }

    #[test]
    fn split_passages_keep_their_own_text_and_spans_apart() {
        let paragraphs: Vec<String> = (0..6).map(|p| numbered(&format!("p{p}w"), 90)).collect();
        let content = format!("## Long\n\n{}", paragraphs.join("\n\n"));
        let built =
            build(&json!({ "title": "Doc", "entity": { "definition": "d" }, "content": content }));
        let long: Vec<_> = built.iter().skip(1).collect();

        // No passage's own text repeats the overlap or the prefix.
        let joined = long
            .iter()
            .map(|p| p.body.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(joined, paragraphs.join("\n"));
        let lines: Vec<_> = long
            .iter()
            .map(|p| {
                let span = p.span.unwrap();
                (span.start_line, span.end_line)
            })
            .collect();
        assert_eq!(lines, vec![(3, 5), (7, 9), (11, 13)]);
        assert_eq!(
            slice(&content, long[1].span.unwrap()),
            format!("{}\n\n{}", paragraphs[2], paragraphs[3])
        );
    }

    #[test]
    fn sections_are_the_markdown_as_written_and_line_up_with_passages() {
        let content = "Intro.\n\n## Run\n\n```bash\n# not a heading\ncargo test\n```\n\n\
                       #### Deeper\n\nSee [docs](https://example.com).\n\n\n\
                       ## Relationships\n\n- [x](y.md)\n\n## Empty\n\n## Last\r\n\r\nEnd.\r\n";
        let document =
            json!({ "title": "Doc", "entity": { "definition": "d" }, "content": content });
        let listed = sections(&document);

        let headings: Vec<_> = listed.iter().map(|s| s.heading.as_deref()).collect();
        assert_eq!(headings, vec![None, Some("Run"), Some("Last")]);
        assert_eq!(listed[0].text, "Intro.");
        assert_eq!(
            listed[1].text,
            "## Run\n\n```bash\n# not a heading\ncargo test\n```\n\n\
             #### Deeper\n\nSee [docs](https://example.com)."
        );
        assert_eq!(
            (listed[1].span.start_line, listed[1].span.end_line),
            (3, 12)
        );
        assert_eq!(listed[2].text, "## Last\r\n\r\nEnd.");
        for section in &listed {
            assert_eq!(slice(content, section.span), section.text);
        }

        for passage in build(&document).iter().skip(1) {
            let section = &listed[passage.section.unwrap()];
            assert_eq!(passage.heading, section.heading);
            let span = passage.span.unwrap();
            assert!(span.start_char >= section.span.start_char);
            assert!(span.end_char <= section.span.end_char);
        }
    }
}
