//! Search over stored passages: semantic ranking, hybrid ranking, and the
//! sections a match points into.
//!
//! [`VectorStore::query`] ranks documents one of two ways (see
//! [`SearchMode`]). Semantic ranking is the cosine similarity of the query
//! to each document's best passage. Hybrid ranking also ranks documents by
//! `SQLite` FTS5 `bm25` over passage text, and fuses the two rankings by
//! reciprocal rank fusion: a document's score is `1 / (60 + r)` summed over
//! its rank `r` in each ranking it appears in. A query for an identifier
//! or an exact term, which an embedding places poorly, is then found by
//! the text that holds it.

use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{OptionalExtension, params};

use crate::{
    PassageSpan, RootedSearchMatch, SEARCH_SQL, SEARCH_V0_SQL, SEARCH_V1_SQL, SearchFilter,
    SearchMatch, StoreError, VectorStore, checked_vector, cosine_similarity, norm, query_error,
    wrap_root_error,
};

/// The constant of reciprocal rank fusion: how much a rank counts falls off
/// as `1 / (RRF_K + rank)`.
const RRF_K: f32 = 60.0;

/// A query term is distinctive when it occurs in one passage, or in at
/// most one passage in this many.
const DISTINCTIVE_SHARE: i64 = 20;

/// How [`VectorStore::query`] ranks documents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SearchMode {
    /// By the cosine similarity of the query vector to each document's
    /// best-matching passage.
    Semantic,
    /// By reciprocal rank fusion of the semantic ranking and a lexical
    /// (`bm25`) ranking of passage text.
    #[default]
    Hybrid,
}

/// One search, for [`VectorStore::query`].
#[derive(Debug, Clone, Copy)]
pub struct SearchQuery<'a> {
    /// The query's embedding vector.
    pub vector: &'a [f32],
    /// The query as text, for the lexical ranking. Unused under
    /// [`SearchMode::Semantic`].
    pub text: &'a str,
    /// How to rank.
    pub mode: SearchMode,
    /// The most documents to return.
    pub limit: usize,
    /// Which stored documents to rank.
    pub filter: &'a SearchFilter,
}

/// One section of a stored document, from [`VectorStore::sections`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSection {
    /// The section's heading. `None` for the document's summary and for
    /// body text that has no heading.
    pub heading: Option<String>,
    /// The section's markdown as it was written, or, for the summary, the
    /// summary passage's text.
    pub text: String,
    /// Where the section sits in the document's body. `None` for the
    /// summary alone.
    pub span: Option<PassageSpan>,
}

/// One passage of a document: which, and what it is headed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PassageRef {
    /// The passage's position in its document. `None` for a document
    /// stored without passages.
    ord: Option<i64>,
    heading: Option<String>,
}

/// A document's best lexical match.
#[derive(Debug, Clone, PartialEq)]
struct Lexical {
    bm25: f32,
    /// The document's place in its own store's lexical ranking, from 1.
    rank: usize,
    /// The document's best passage holding a distinctive query term (see
    /// [`DISTINCTIVE_SHARE`]). `None` when the document matched on common
    /// words only: such a match counts toward its rank, but says nothing
    /// about which of its passages answers the query.
    passage: Option<PassageRef>,
}

/// One document that passed the filter, scored but not yet ranked.
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    id: String,
    cosine: f32,
    cosine_passage: PassageRef,
    lexical: Option<Lexical>,
}

/// A ranked candidate: its fused score and the passage to report for it.
#[derive(Debug, Clone, PartialEq)]
struct Ranked {
    /// Which store the candidate came from, as an index into the roots
    /// queried.
    root: usize,
    candidate: Candidate,
    score: f32,
    passage: PassageRef,
}

/// The FTS5 phrases of free text: each word that holds a letter or a digit,
/// quoted. Quoting keeps punctuation from being read as query syntax, and
/// makes a hyphenated identifier (`ADR-007`) match as the adjacent tokens
/// it is indexed as. Control characters are dropped: FTS5 reads a NUL as
/// the end of the query.
fn fts_phrases(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .map(|word| {
            let word: String = word
                .chars()
                .filter(|c| !c.is_control())
                .map(|c| if c == '"' { ' ' } else { c })
                .collect();
            format!("\"{word}\"")
        })
        .collect()
}

/// The FTS5 query matching any of `phrases`. `None` when there are none.
fn fts_query(phrases: &[String]) -> Option<String> {
    Some(phrases.join(" OR ")).filter(|query| !query.is_empty())
}

/// Whether a stored type passes an include list and an exclude list.
fn type_passes(stored: Option<&str>, include: &[String], exclude: &[String]) -> bool {
    let listed = |list: &[String]| stored.is_some_and(|stored| list.iter().any(|t| t == stored));
    (include.is_empty() || listed(include)) && !listed(exclude)
}

/// What a rank is worth under reciprocal rank fusion.
fn reciprocal(rank: usize) -> f32 {
    // A rank is a position in a list held in memory; any rounding this far
    // up changes nothing that could reorder two documents.
    #[allow(clippy::cast_precision_loss)]
    let rank = rank as f32;
    1.0 / (RRF_K + rank)
}

/// Ranks candidates gathered from one or more stores and keeps the top
/// `limit`.
///
/// Cosine similarities come from one embedding model and are ranked
/// together across stores. A `bm25` value depends on the store it was
/// computed in, so each document keeps the lexical rank it has in its own
/// store. Ties go to the higher cosine, then the lower id, then the
/// earlier store.
fn fuse(candidates: Vec<(usize, Candidate)>, mode: SearchMode, limit: usize) -> Vec<Ranked> {
    let mut by_cosine = candidates;
    by_cosine.sort_by(|(root_a, a), (root_b, b)| {
        b.cosine
            .total_cmp(&a.cosine)
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| root_a.cmp(root_b))
    });
    let mut ranked: Vec<Ranked> = by_cosine
        .into_iter()
        .enumerate()
        .map(|(index, (root, candidate))| {
            let cosine_rank = index + 1;
            let lexical = candidate
                .lexical
                .as_ref()
                .filter(|_| mode == SearchMode::Hybrid);
            let score = match mode {
                SearchMode::Semantic => candidate.cosine,
                SearchMode::Hybrid => {
                    reciprocal(cosine_rank) + lexical.map_or(0.0, |l| reciprocal(l.rank))
                },
            };
            // Report the passage from whichever ranking places the document
            // higher, since that is what got it into the results; but a
            // match on common words alone names no passage, and then the
            // passage nearest in meaning stands.
            let passage = lexical
                .filter(|l| l.rank < cosine_rank)
                .and_then(|l| l.passage.clone())
                .unwrap_or_else(|| candidate.cosine_passage.clone());
            Ranked {
                root,
                candidate,
                score,
                passage,
            }
        })
        .collect();
    // The sort is stable, so equal scores keep their cosine order.
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    ranked.truncate(limit);
    ranked
}

impl VectorStore {
    /// Ranks stored documents against a query and returns the top
    /// `query.limit`, best first, each with the passage that matched.
    ///
    /// Under [`SearchMode::Semantic`] each document is ranked by the cosine
    /// similarity of its best-matching passage, and [`SearchMatch::score`]
    /// is that similarity. Under [`SearchMode::Hybrid`] that ranking is
    /// fused with a `bm25` ranking of passage text (see the module
    /// documentation). A document stored without passage text takes part
    /// in the semantic ranking only, and so does every document of a
    /// database that could not be upgraded.
    ///
    /// Only documents passing `query.filter` are ranked. A vector whose
    /// dimensionality differs from the query's, or whose score is not a
    /// finite number, is skipped.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::CorruptDimension`] if a row's `dim` column does
    /// not fit a [`usize`], [`StoreError::VectorBlobMismatch`] if a row's
    /// decoded vector length does not match its `dim` column, or
    /// [`StoreError::Query`] if an underlying `SQLite` statement fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mif_store::{
    ///     DocumentVectors, PassageVector, SearchFilter, SearchMode, SearchQuery, VectorStore,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let dir = tempfile::tempdir()?;
    /// let store = VectorStore::open(&dir.path().join("vectors.db"))?;
    /// for (id, vector, text) in [
    ///     ("urn:mif:near", [1.0, 0.0], "Notes on billing"),
    ///     ("urn:mif:exact", [0.0, 1.0], "Practice P23 covers consent"),
    /// ] {
    ///     store.upsert_document(&DocumentVectors {
    ///         id,
    ///         vector: &vector,
    ///         passages: &[PassageVector {
    ///             vector: &vector,
    ///             text: Some(text),
    ///             ..Default::default()
    ///         }],
    ///         content_hash: "h",
    ///         updated_at: "2026-01-01T00:00:00Z",
    ///         ..Default::default()
    ///     })?;
    /// }
    ///
    /// // The query vector sits nearer the first document; the term decides.
    /// let hits = store.query(&SearchQuery {
    ///     vector: &[0.8, 0.6],
    ///     text: "P23",
    ///     mode: SearchMode::Hybrid,
    ///     limit: 10,
    ///     filter: &SearchFilter::default(),
    /// })?;
    /// assert_eq!(hits[0].id, "urn:mif:exact");
    /// assert_eq!(hits[0].text.as_deref(), Some("Practice P23 covers consent"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn query(&self, query: &SearchQuery<'_>) -> Result<Vec<SearchMatch>, StoreError> {
        let candidates = self
            .candidates(query)?
            .into_iter()
            .map(|candidate| (0, candidate))
            .collect();
        fuse(candidates, query.mode, query.limit)
            .into_iter()
            .map(|ranked| self.hydrate(ranked))
            .collect()
    }

    /// Scores every document that passes the filter: its best cosine
    /// passage and, under [`SearchMode::Hybrid`], its best lexical one.
    fn candidates(&self, query: &SearchQuery<'_>) -> Result<Vec<Candidate>, StoreError> {
        let sql = if self.has_text {
            SEARCH_SQL
        } else if self.has_passages {
            SEARCH_V1_SQL
        } else {
            SEARCH_V0_SQL
        };
        let filter = query.filter;
        // An empty prefix asks for no filtering, the same as an unset one.
        let prefix = |prefix: &'_ Option<String>| -> Option<String> {
            prefix.clone().filter(|prefix| !prefix.is_empty())
        };
        let mut stmt = self.conn.prepare(sql).map_err(query_error)?;
        let mut rows = stmt
            .query(params![
                prefix(&filter.id_prefix),
                prefix(&filter.source_prefix),
                prefix(&filter.namespace_prefix)
            ])
            .map_err(query_error)?;

        let query_norm = norm(query.vector);
        let mut best: HashMap<String, Candidate> = HashMap::new();
        // Which passage each stored text belongs to, for the lexical pass.
        let mut by_text: HashMap<i64, (String, PassageRef)> = HashMap::new();
        while let Some(row) = rows.next().map_err(query_error)? {
            let id: String = row.get(0).map_err(query_error)?;
            let entity_type: Option<String> = row.get(6).map_err(query_error)?;
            let concept_type: Option<String> = row.get(7).map_err(query_error)?;
            if !type_passes(
                entity_type.as_deref(),
                &filter.entity_types,
                &filter.exclude_entity_types,
            ) || !type_passes(
                concept_type.as_deref(),
                &filter.concept_types,
                &filter.exclude_concept_types,
            ) {
                continue;
            }
            let dim_raw: i64 = row.get(2).map_err(query_error)?;
            let blob: Vec<u8> = row.get(3).map_err(query_error)?;
            let vector = checked_vector(&id, dim_raw, &blob)?;
            if vector.len() != query.vector.len() {
                continue;
            }
            let cosine = cosine_similarity(query.vector, &vector, query_norm);
            if !cosine.is_finite() {
                continue;
            }
            let passage = PassageRef {
                ord: row.get(4).map_err(query_error)?,
                heading: row.get(1).map_err(query_error)?,
            };
            if let Some(text_rowid) = row.get::<_, Option<i64>>(5).map_err(query_error)? {
                by_text.insert(text_rowid, (id.clone(), passage.clone()));
            }
            // Rows come in passage order, so the earliest of several equally
            // scored passages is the one kept.
            match best.get_mut(&id) {
                Some(candidate) if cosine > candidate.cosine => {
                    candidate.cosine = cosine;
                    candidate.cosine_passage = passage;
                },
                Some(_) => {},
                None => {
                    best.insert(
                        id.clone(),
                        Candidate {
                            id,
                            cosine,
                            cosine_passage: passage,
                            lexical: None,
                        },
                    );
                },
            }
        }

        if query.mode == SearchMode::Hybrid && self.has_text {
            self.rank_lexical(query.text, &by_text, &mut best)?;
        }
        Ok(best.into_values().collect())
    }

    /// Gives each candidate with a passage holding a query term its best
    /// such passage and its place in the lexical ranking.
    fn rank_lexical(
        &self,
        text: &str,
        by_text: &HashMap<i64, (String, PassageRef)>,
        candidates: &mut HashMap<String, Candidate>,
    ) -> Result<(), StoreError> {
        let phrases = fts_phrases(text);
        let Some(fts) = fts_query(&phrases) else {
            return Ok(());
        };
        let best = self.best_passages(&fts, by_text)?;
        let distinctive = match fts_query(&self.distinctive(&phrases)?) {
            Some(fts) => self.best_passages(&fts, by_text)?,
            None => HashMap::new(),
        };

        let mut order: Vec<(&str, f32)> = best
            .iter()
            .map(|(id, (bm25, _))| (id.as_str(), *bm25))
            .collect();
        order.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        // Documents with the same bm25 share a rank. Text that many
        // documents hold word for word (a template's boilerplate) matches a
        // common query word equally in all of them, and an order made up
        // among equals must not move any of them past a better match.
        let mut rank = 0;
        let mut previous: Option<f32> = None;
        for (index, (id, bm25)) in order.into_iter().enumerate() {
            if previous.is_none_or(|previous| previous.total_cmp(&bm25).is_ne()) {
                rank = index + 1;
                previous = Some(bm25);
            }
            if let Some(candidate) = candidates.get_mut(id) {
                candidate.lexical = Some(Lexical {
                    bm25,
                    rank,
                    passage: distinctive.get(id).map(|(_, passage)| passage.clone()),
                });
            }
        }
        Ok(())
    }

    /// Each candidate document's best passage for an FTS5 query, with its
    /// bm25: the lowest bm25, then the earliest passage.
    fn best_passages(
        &self,
        fts: &str,
        by_text: &HashMap<i64, (String, PassageRef)>,
    ) -> Result<HashMap<String, (f32, PassageRef)>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT rowid, bm25(passage_text) FROM passage_text
                 WHERE passage_text MATCH ?1",
            )
            .map_err(query_error)?;
        let hits = stmt
            .query_map([fts], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(query_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_error)?;

        let mut best: HashMap<String, (f32, PassageRef)> = HashMap::new();
        for (rowid, bm25) in hits {
            // Text left by a replaced passage, or of a filtered-out
            // document, belongs to no candidate.
            let Some((id, passage)) = by_text.get(&rowid) else {
                continue;
            };
            // bm25 is an f64 whose magnitude is far inside f32's range.
            #[allow(clippy::cast_possible_truncation)]
            let bm25 = bm25 as f32;
            let better = best.get(id).is_none_or(|(held, held_passage)| {
                bm25 < *held || (bm25.total_cmp(held).is_eq() && passage.ord < held_passage.ord)
            });
            if better {
                best.insert(id.clone(), (bm25, passage.clone()));
            }
        }
        Ok(best)
    }

    /// The phrases that are distinctive in this store: those found in one
    /// passage, or in at most one passage in [`DISTINCTIVE_SHARE`].
    fn distinctive(&self, phrases: &[String]) -> Result<Vec<String>, StoreError> {
        let total: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM passage_text", [], |row| row.get(0))
            .map_err(query_error)?;
        let mut count = self
            .conn
            .prepare("SELECT COUNT(*) FROM passage_text WHERE passage_text MATCH ?1")
            .map_err(query_error)?;
        let mut kept = Vec::new();
        for phrase in phrases {
            let found: i64 = count
                .query_row([phrase], |row| row.get(0))
                .map_err(query_error)?;
            if found == 1 || (found > 0 && found.saturating_mul(DISTINCTIVE_SHARE) <= total) {
                kept.push(phrase.clone());
            }
        }
        Ok(kept)
    }

    /// Reads back what a match reports beyond its scores: the document's
    /// source, title, and types, and the passage's text and span.
    fn hydrate(&self, ranked: Ranked) -> Result<SearchMatch, StoreError> {
        let Ranked {
            candidate,
            score,
            passage,
            ..
        } = ranked;
        let text =
            |column: usize| move |row: &rusqlite::Row<'_>| row.get::<_, Option<String>>(column);
        let (source, title, entity_type, concept_type) = if self.has_text {
            self.conn
                .query_row(
                    "SELECT source,
                            CASE WHEN meta_hash = content_hash THEN title END,
                            CASE WHEN meta_hash = content_hash THEN entity_type END,
                            CASE WHEN meta_hash = content_hash THEN concept_type END
                     FROM embeddings
                     WHERE id = ?1",
                    [&candidate.id],
                    |row| Ok((text(0)(row)?, text(1)(row)?, text(2)(row)?, text(3)(row)?)),
                )
                .optional()
                .map_err(query_error)?
                .unwrap_or_default()
        } else if self.has_passages {
            let source = self
                .conn
                .query_row(
                    "SELECT source FROM embeddings WHERE id = ?1",
                    [&candidate.id],
                    text(0),
                )
                .optional()
                .map_err(query_error)?
                .flatten();
            (source, None, None, None)
        } else {
            (None, None, None, None)
        };
        let (passage_text, span) = match passage.ord {
            Some(ord) if self.has_text => self
                .conn
                .query_row(
                    "SELECT t.text, p.start_char, p.end_char, p.start_line, p.end_line
                     FROM passages p
                     LEFT JOIN passage_text t ON t.rowid = p.text_rowid
                     WHERE p.doc_id = ?1 AND p.ord = ?2",
                    params![candidate.id, ord],
                    |row| Ok((text(0)(row)?, span_at(row, 1)?)),
                )
                .optional()
                .map_err(query_error)?
                .unwrap_or_default(),
            _ => (None, None),
        };
        Ok(SearchMatch {
            id: candidate.id,
            score,
            cosine: candidate.cosine,
            bm25: candidate.lexical.map(|lexical| lexical.bm25),
            heading: passage.heading,
            text: passage_text,
            span,
            source,
            title,
            entity_type,
            concept_type,
        })
    }

    /// The sections of a stored document headed `heading`, in document
    /// order, as they were written. With no `heading`: the document's
    /// summary, then any body section that has no heading (the text before
    /// the first one), which is what a match with no heading points at; the
    /// summary has no span and such a section has one. `None` when nothing
    /// is stored for `id`.
    ///
    /// A heading that occurs more than once in the document gives one entry
    /// per occurrence. The list is empty when the document has no such
    /// section, and for any document stored without sections (by a release
    /// that predates them, or by [`Self::upsert`]).
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] if an underlying `SQLite` statement
    /// fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mif_store::{DocumentVectors, PassageSpan, SectionText, VectorStore};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let dir = tempfile::tempdir()?;
    /// let store = VectorStore::open(&dir.path().join("vectors.db"))?;
    /// store.upsert_document(&DocumentVectors {
    ///     id: "urn:mif:doc",
    ///     vector: &[1.0, 0.0],
    ///     content_hash: "h",
    ///     updated_at: "2026-01-01T00:00:00Z",
    ///     sections: &[SectionText {
    ///         heading: Some("Scope"),
    ///         text: "## Scope\n\nSee [the plan](plan.md).",
    ///         span: PassageSpan { start_char: 0, end_char: 34, start_line: 1, end_line: 3 },
    ///     }],
    ///     ..Default::default()
    /// })?;
    ///
    /// let sections = store.sections("urn:mif:doc", Some("Scope"))?.expect("stored");
    /// assert_eq!(sections[0].text, "## Scope\n\nSee [the plan](plan.md).");
    /// assert!(store.sections("urn:mif:absent", None)?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn sections(
        &self,
        id: &str,
        heading: Option<&str>,
    ) -> Result<Option<Vec<StoredSection>>, StoreError> {
        let stored: bool = self
            .conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM embeddings WHERE id = ?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(query_error)?;
        if !stored {
            return Ok(None);
        }
        if !self.has_text {
            return Ok(Some(Vec::new()));
        }
        let Some(heading) = heading else {
            return self.unheaded_sections(id).map(Some);
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.heading, s.text, s.start_char, s.end_char, s.start_line, s.end_line
                 FROM embeddings e
                 JOIN sections s ON s.doc_id = e.id AND s.content_hash = e.content_hash
                 WHERE e.id = ?1 AND s.heading = ?2
                 ORDER BY s.ord",
            )
            .map_err(query_error)?;
        let sections = stmt
            .query_map(params![id, heading], |row| {
                Ok(StoredSection {
                    heading: row.get(0)?,
                    text: row.get(1)?,
                    span: span_at(row, 2)?,
                })
            })
            .map_err(query_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_error)?;
        Ok(Some(sections))
    }
}

impl VectorStore {
    /// What a stored document holds under no heading: its summary, then
    /// any body section that has none (the text before its first heading).
    /// A match that reports no heading is one of these.
    fn unheaded_sections(&self, id: &str) -> Result<Vec<StoredSection>, StoreError> {
        let summary = self
            .conn
            .query_row(
                "SELECT t.text FROM embeddings e
                 JOIN passages p ON p.doc_id = e.id AND p.content_hash = e.content_hash
                 JOIN passage_text t ON t.rowid = p.text_rowid
                 WHERE e.id = ?1 AND p.ord = 0",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(query_error)?;
        let mut sections: Vec<StoredSection> = summary
            .map(|text| StoredSection {
                heading: None,
                text,
                span: None,
            })
            .into_iter()
            .collect();
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.text, s.start_char, s.end_char, s.start_line, s.end_line
                 FROM embeddings e
                 JOIN sections s ON s.doc_id = e.id AND s.content_hash = e.content_hash
                 WHERE e.id = ?1 AND s.heading IS NULL
                 ORDER BY s.ord",
            )
            .map_err(query_error)?;
        let unheaded = stmt
            .query_map([id], |row| {
                Ok(StoredSection {
                    heading: None,
                    text: row.get(0)?,
                    span: span_at(row, 1)?,
                })
            })
            .map_err(query_error)?;
        for section in unheaded {
            sections.push(section.map_err(query_error)?);
        }
        Ok(sections)
    }
}

/// The span held in the four columns starting at `first`. `None` when any
/// of them is `NULL` or is not a position.
fn span_at(row: &rusqlite::Row<'_>, first: usize) -> rusqlite::Result<Option<PassageSpan>> {
    let part = |offset: usize| -> rusqlite::Result<Option<usize>> {
        Ok(row
            .get::<_, Option<i64>>(first + offset)?
            .and_then(|value| usize::try_from(value).ok()))
    };
    Ok(match (part(0)?, part(1)?, part(2)?, part(3)?) {
        (Some(start_char), Some(end_char), Some(start_line), Some(end_line)) => Some(PassageSpan {
            start_char,
            end_char,
            start_line,
            end_line,
        }),
        _ => None,
    })
}

/// Runs one search across multiple [`VectorStore`] roots, merged into one
/// ranked list in which each match is paired with the root it came from.
///
/// Under [`SearchMode::Semantic`] the merged list is ordered by cosine
/// similarity. Under [`SearchMode::Hybrid`] cosine similarities are ranked
/// together across the roots, while each document keeps the lexical rank it
/// has within its own root, since a `bm25` value computed in one store
/// cannot be compared with one from another.
///
/// Follows the same fail-closed policy as
/// [`crate::multi_root_top_k_similar`]: any root that fails to open or
/// query aborts the call, and a not-yet-created root is empty.
///
/// # Errors
///
/// Returns the first failure encountered opening or querying any root, in
/// the order `roots` was given: an open failure is returned as-is, and a
/// query failure is wrapped in [`StoreError::RootFailure`] naming the root.
pub fn multi_root_query(
    roots: &[PathBuf],
    query: &SearchQuery<'_>,
) -> Result<Vec<RootedSearchMatch>, StoreError> {
    let mut stores = Vec::with_capacity(roots.len());
    let mut candidates = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let store = VectorStore::open(root)?;
        candidates.extend(
            wrap_root_error(root, store.candidates(query))?
                .into_iter()
                .map(|candidate| (index, candidate)),
        );
        stores.push(store);
    }
    let mut matches = Vec::new();
    for ranked in fuse(candidates, query.mode, query.limit) {
        let index = ranked.root;
        let (Some(store), Some(root)) = (stores.get(index), roots.get(index)) else {
            continue;
        };
        matches.push(RootedSearchMatch {
            document: wrap_root_error(root, store.hydrate(ranked))?,
            root: root.clone(),
        });
    }
    Ok(matches)
}

/// Looks up a document's sections (see [`VectorStore::sections`]) across
/// multiple [`VectorStore`] roots, returning them from the first root, in
/// `roots` order, that holds the document, with that root.
///
/// # Errors
///
/// Returns the first failure encountered opening or querying any root, in
/// the order `roots` was given: an open failure is returned as-is, and a
/// query failure is wrapped in [`StoreError::RootFailure`] naming the root.
pub fn multi_root_sections(
    roots: &[PathBuf],
    id: &str,
    heading: Option<&str>,
) -> Result<Option<(PathBuf, Vec<StoredSection>)>, StoreError> {
    for root in roots {
        let store = VectorStore::open(root)?;
        if let Some(sections) = wrap_root_error(root, store.sections(id, heading))? {
            return Ok(Some((root.clone(), sections)));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::{
        DocumentVectors, MIGRATE_V1_SQL, PassageStats, PassageVector, SCHEMA_SQL, SCHEMA_VERSION,
        SectionText, encode_vector, schema_version,
    };

    fn open_temp_store() -> (tempfile::TempDir, VectorStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(&dir.path().join("vectors.db")).unwrap();
        (dir, store)
    }

    /// Stores a document of one passage per `(heading, vector, text)`.
    fn store_document(
        store: &VectorStore,
        id: &str,
        passages: &[(Option<&str>, [f32; 2], &str)],
        entity_type: Option<&str>,
        concept_type: Option<&str>,
    ) {
        let passages: Vec<PassageVector<'_>> = passages
            .iter()
            .enumerate()
            .map(|(index, (heading, vector, text))| PassageVector {
                heading: *heading,
                vector,
                text: Some(text),
                span: Some(PassageSpan {
                    start_char: index * 10,
                    end_char: index * 10 + 5,
                    start_line: index + 1,
                    end_line: index + 2,
                }),
                section: Some(index),
            })
            .collect();
        store
            .upsert_document(&DocumentVectors {
                id,
                vector: passages[0].vector,
                passages: &passages,
                content_hash: "h",
                updated_at: "t",
                source: Some("records/doc.md"),
                title: Some("A title"),
                entity_type,
                concept_type,
                ..Default::default()
            })
            .unwrap();
    }

    fn hybrid<'a>(vector: &'a [f32], text: &'a str, filter: &'a SearchFilter) -> SearchQuery<'a> {
        SearchQuery {
            vector,
            text,
            mode: SearchMode::Hybrid,
            limit: 10,
            filter,
        }
    }

    fn ids(matches: &[SearchMatch]) -> Vec<&str> {
        matches.iter().map(|m| m.id.as_str()).collect()
    }

    fn text_rows(store: &VectorStore) -> i64 {
        store
            .conn
            .query_row("SELECT COUNT(*) FROM passage_text", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_term_only_match_outranks_a_nearer_vector_and_reports_its_passage() {
        let (_dir, store) = open_temp_store();
        store_document(
            &store,
            "near",
            &[(None, [1.0, 0.0], "Notes on billing")],
            None,
            None,
        );
        store_document(
            &store,
            "exact",
            &[
                (None, [0.0, 1.0], "A record"),
                (Some("Scope"), [0.0, 1.0], "Practice P23 covers consent"),
            ],
            Some("practice"),
            Some("semantic"),
        );
        let filter = SearchFilter::default();

        let semantic = store.search(&[0.8, 0.6], 10, &filter).unwrap();
        assert_eq!(ids(&semantic), vec!["near", "exact"]);
        assert!((semantic[0].score - semantic[0].cosine).abs() < f32::EPSILON);
        assert_eq!(semantic[0].bm25, None);

        let hits = store.query(&hybrid(&[0.8, 0.6], "p23", &filter)).unwrap();
        assert_eq!(ids(&hits), vec!["exact", "near"]);
        let exact = &hits[0];
        assert!((exact.score - (reciprocal(2) + reciprocal(1))).abs() < 1e-6);
        assert!((exact.cosine - 0.6).abs() < 1e-6);
        assert!(exact.bm25.unwrap() < 0.0);
        assert_eq!(exact.heading.as_deref(), Some("Scope"));
        assert_eq!(exact.text.as_deref(), Some("Practice P23 covers consent"));
        assert_eq!(
            exact.span,
            Some(PassageSpan {
                start_char: 10,
                end_char: 15,
                start_line: 2,
                end_line: 3
            })
        );
        assert_eq!(exact.source.as_deref(), Some("records/doc.md"));
        assert_eq!(exact.title.as_deref(), Some("A title"));
        assert_eq!(exact.entity_type.as_deref(), Some("practice"));
        assert_eq!(exact.concept_type.as_deref(), Some("semantic"));
        // The other document matched no term: it is ranked on cosine alone.
        assert_eq!(hits[1].bm25, None);
        assert!((hits[1].score - reciprocal(1)).abs() < 1e-6);
        assert_eq!(hits[1].text.as_deref(), Some("Notes on billing"));
    }

    #[test]
    fn documents_that_match_a_term_equally_share_a_lexical_rank() {
        let (_dir, store) = open_temp_store();
        // The same boilerplate in every document, and a query that shares
        // only a common word with it: the lexical ranking has nothing to
        // say, so the semantic order must stand.
        for (id, vector) in [("a", [0.6, 0.8]), ("b", [0.8, 0.6]), ("c", [1.0, 0.0])] {
            store_document(
                &store,
                id,
                &[(None, vector, "the purpose and the owner")],
                None,
                None,
            );
        }

        let hits = store
            .query(&hybrid(
                &[1.0, 0.0],
                "cats and dogs",
                &SearchFilter::default(),
            ))
            .unwrap();

        assert_eq!(ids(&hits), vec!["c", "b", "a"]);
        assert!(hits.iter().all(|hit| hit.bm25.is_some()));
        assert!((hits[0].score - (reciprocal(1) + reciprocal(1))).abs() < 1e-6);
        assert!((hits[2].score - (reciprocal(3) + reciprocal(1))).abs() < 1e-6);
    }

    #[test]
    fn a_hyphenated_identifier_matches_as_a_phrase() {
        let (_dir, store) = open_temp_store();
        store_document(
            &store,
            "a",
            &[(None, [1.0, 0.0], "See ADR 12 and rule 007")],
            None,
            None,
        );
        store_document(
            &store,
            "b",
            &[(None, [1.0, 0.0], "Decided in ADR-007 last year")],
            None,
            None,
        );

        let hits = store
            .query(&hybrid(&[1.0, 0.0], "ADR-007", &SearchFilter::default()))
            .unwrap();

        assert!(hits.iter().find(|m| m.id == "b").unwrap().bm25.is_some());
        assert_eq!(hits.iter().find(|m| m.id == "a").unwrap().bm25, None);
        assert_eq!(hits[0].id, "b");
    }

    #[test]
    fn query_text_is_never_read_as_query_syntax() {
        let (_dir, store) = open_temp_store();
        store_document(
            &store,
            "doc",
            &[(None, [1.0, 0.0], "plain words")],
            None,
            None,
        );
        let filter = SearchFilter::default();

        for text in [
            "",
            "   ",
            "--- *** ???",
            "\"unbalanced",
            "plain AND NOT (words",
            "col:umn* ^ NEAR(",
            "it's \"quoted\" - text",
            "pla\u{0}in \u{0}",
        ] {
            let hits = store.query(&hybrid(&[1.0, 0.0], text, &filter)).unwrap();
            assert_eq!(ids(&hits), vec!["doc"], "query {text:?}");
        }
        assert_eq!(fts_query(&fts_phrases("--- ???")), None);
        assert_eq!(
            fts_query(&fts_phrases("a\"b  c-d x\u{0}y\u{7}")).as_deref(),
            Some("\"a b\" OR \"c-d\" OR \"xy\"")
        );
    }

    #[test]
    fn type_filters_include_and_exclude_exact_values() {
        let (_dir, store) = open_temp_store();
        let one = [(None, [1.0, 0.0], "text")];
        store_document(
            &store,
            "adr",
            &one,
            Some("decision-record"),
            Some("semantic"),
        );
        store_document(&store, "log", &one, Some("meeting"), Some("episodic"));
        store_document(&store, "plain", &one, None, None);
        let run = |filter: SearchFilter| {
            let mut found: Vec<String> = store
                .query(&hybrid(&[1.0, 0.0], "text", &filter))
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect();
            found.sort();
            found
        };
        let list = |values: &[&str]| values.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            run(SearchFilter {
                entity_types: list(&["decision-record", "meeting"]),
                ..Default::default()
            }),
            vec!["adr", "log"]
        );
        assert_eq!(
            run(SearchFilter {
                entity_types: list(&["decision"]),
                ..Default::default()
            }),
            Vec::<String>::new()
        );
        // A document with no type survives an exclusion.
        assert_eq!(
            run(SearchFilter {
                exclude_entity_types: list(&["meeting"]),
                ..Default::default()
            }),
            vec!["adr", "plain"]
        );
        assert_eq!(
            run(SearchFilter {
                concept_types: list(&["episodic"]),
                ..Default::default()
            }),
            vec!["log"]
        );
        assert_eq!(
            run(SearchFilter {
                entity_types: list(&["decision-record", "meeting"]),
                exclude_concept_types: list(&["episodic"]),
                ..Default::default()
            }),
            vec!["adr"]
        );
        // A filtered-out document's text earns no lexical rank for anyone.
        let only = store
            .query(&hybrid(
                &[1.0, 0.0],
                "text",
                &SearchFilter {
                    entity_types: list(&["meeting"]),
                    ..Default::default()
                },
            ))
            .unwrap();
        assert!((only[0].score - reciprocal(1) - reciprocal(1)).abs() < 1e-6);
    }

    #[test]
    fn a_title_and_types_written_for_other_content_are_not_reported() {
        let (_dir, store) = open_temp_store();
        let one = [(None, [1.0, 0.0], "text")];
        store_document(&store, "replaced", &one, Some("practice"), Some("semantic"));
        store_document(
            &store,
            "older-writer",
            &one,
            Some("practice"),
            Some("semantic"),
        );
        let typed = SearchFilter {
            entity_types: vec!["practice".to_string()],
            ..Default::default()
        };
        let found = |filter: &SearchFilter| {
            let hits = store.query(&hybrid(&[1.0, 0.0], "text", filter)).unwrap();
            hits.into_iter()
                .map(|hit| (hit.id, hit.title, hit.entity_type, hit.concept_type))
                .collect::<Vec<_>>()
        };
        assert_eq!(found(&typed).len(), 2);

        // A plain upsert replaces the document, even with the same hash.
        store.upsert("replaced", &[1.0, 0.0], "h", "t").unwrap();
        // A release that predates types stores changed content: its
        // statement rewrites the hash and leaves the columns it does not
        // know of.
        store
            .conn
            .execute(
                "UPDATE embeddings SET content_hash = 'h2' WHERE id = 'older-writer'",
                [],
            )
            .unwrap();

        assert!(found(&typed).is_empty());
        let all = found(&SearchFilter::default());
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|(_, title, entity, concept)| {
            title.is_none() && entity.is_none() && concept.is_none()
        }));
    }

    #[test]
    fn a_document_stored_without_text_is_still_ranked_by_its_vector() {
        let (_dir, store) = open_temp_store();
        store_document(
            &store,
            "new",
            &[(None, [0.0, 1.0], "alpha term")],
            None,
            None,
        );
        store
            .upsert_document(&DocumentVectors {
                id: "textless",
                vector: &[1.0, 0.0],
                passages: &[PassageVector {
                    heading: Some("Old"),
                    vector: &[1.0, 0.0],
                    ..Default::default()
                }],
                content_hash: "h",
                updated_at: "t",
                ..Default::default()
            })
            .unwrap();
        store.upsert("legacy", &[0.9, 0.1], "h", "t").unwrap();

        let hits = store
            .query(&hybrid(&[1.0, 0.0], "nothing", &SearchFilter::default()))
            .unwrap();

        assert_eq!(ids(&hits), vec!["textless", "legacy", "new"]);
        assert_eq!(hits[0].heading.as_deref(), Some("Old"));
        assert_eq!(hits[0].text, None);
        assert_eq!(hits[0].span, None);
        assert_eq!(hits[1].heading, None);
        assert_eq!(
            store.passage_stats().unwrap(),
            PassageStats {
                passage_count: 2,
                documents_without_passages: 1,
                documents_without_text: 2,
            }
        );
    }

    #[test]
    fn replacing_or_removing_a_document_removes_its_text_and_sections() {
        let (_dir, store) = open_temp_store();
        let section = SectionText {
            heading: Some("Scope"),
            text: "## Scope\n\nOld words.",
            span: PassageSpan {
                start_char: 0,
                end_char: 20,
                start_line: 1,
                end_line: 3,
            },
        };
        let stored = |text: &'static str, hash: &'static str| {
            store
                .upsert_document(&DocumentVectors {
                    id: "doc",
                    vector: &[1.0, 0.0],
                    passages: &[
                        PassageVector {
                            vector: &[1.0, 0.0],
                            text: Some("Summary"),
                            ..Default::default()
                        },
                        PassageVector {
                            heading: Some("Scope"),
                            vector: &[1.0, 0.0],
                            text: Some(text),
                            ..Default::default()
                        },
                    ],
                    content_hash: hash,
                    updated_at: "t",
                    sections: &[section],
                    ..Default::default()
                })
                .unwrap();
        };
        stored("zebra", "h1");
        stored("quagga", "h2");

        assert_eq!(text_rows(&store), 2);
        let filter = SearchFilter::default();
        let old = store.query(&hybrid(&[1.0, 0.0], "zebra", &filter)).unwrap();
        assert_eq!(old[0].bm25, None);
        let new = store
            .query(&hybrid(&[1.0, 0.0], "quagga", &filter))
            .unwrap();
        assert!(new[0].bm25.is_some());
        assert_eq!(
            store.sections("doc", Some("Scope")).unwrap().unwrap().len(),
            1
        );

        // A plain upsert replaces the document with one that has no text.
        store.upsert("doc", &[1.0, 0.0], "h3", "t").unwrap();
        assert_eq!(text_rows(&store), 0);
        assert!(
            store
                .sections("doc", Some("Scope"))
                .unwrap()
                .unwrap()
                .is_empty()
        );

        stored("zebra", "h4");
        assert!(store.remove("doc").unwrap());
        assert_eq!(text_rows(&store), 0);
        assert_eq!(store.sections("doc", Some("Scope")).unwrap(), None);
        let sections: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM sections", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sections, 0);
    }

    #[test]
    fn sections_returns_each_occurrence_of_a_heading_and_the_summary() {
        let (_dir, store) = open_temp_store();
        let span = |line: usize| PassageSpan {
            start_char: line * 100,
            end_char: line * 100 + 50,
            start_line: line,
            end_line: line + 2,
        };
        store
            .upsert_document(&DocumentVectors {
                id: "doc",
                vector: &[1.0, 0.0],
                passages: &[PassageVector {
                    vector: &[1.0, 0.0],
                    text: Some("Title\nWhat it is"),
                    ..Default::default()
                }],
                content_hash: "h",
                updated_at: "t",
                sections: &[
                    SectionText {
                        heading: None,
                        text: "Preamble.",
                        span: span(1),
                    },
                    SectionText {
                        heading: Some("Notes"),
                        text: "## Notes\n\nFirst [link](a.md).",
                        span: span(3),
                    },
                    SectionText {
                        heading: Some("Scope"),
                        text: "## Scope\n\nBetween.",
                        span: span(7),
                    },
                    SectionText {
                        heading: Some("Notes"),
                        text: "## Notes\n\nSecond.",
                        span: span(11),
                    },
                ],
                ..Default::default()
            })
            .unwrap();

        let notes = store.sections("doc", Some("Notes")).unwrap().unwrap();
        assert_eq!(
            notes,
            vec![
                StoredSection {
                    heading: Some("Notes".to_string()),
                    text: "## Notes\n\nFirst [link](a.md).".to_string(),
                    span: Some(span(3)),
                },
                StoredSection {
                    heading: Some("Notes".to_string()),
                    text: "## Notes\n\nSecond.".to_string(),
                    span: Some(span(11)),
                },
            ]
        );
        assert_eq!(
            store.sections("doc", None).unwrap().unwrap(),
            vec![
                StoredSection {
                    heading: None,
                    text: "Title\nWhat it is".to_string(),
                    span: None,
                },
                // The text before the first heading, which a match with no
                // heading may also point at.
                StoredSection {
                    heading: None,
                    text: "Preamble.".to_string(),
                    span: Some(span(1)),
                },
            ]
        );
        assert!(
            store
                .sections("doc", Some("notes"))
                .unwrap()
                .unwrap()
                .is_empty()
        );
        assert_eq!(store.sections("absent", Some("Notes")).unwrap(), None);
    }

    /// A database as release 0.12.0 wrote it: schema version 1, one document
    /// with a headed passage.
    fn write_v1_database(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute(SCHEMA_SQL, []).unwrap();
        conn.execute_batch(MIGRATE_V1_SQL).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute(
            "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at, source, namespace)
             VALUES ('old-doc', 2, ?1, 'old-hash', 'old-time', 'records/old.md', 'records')",
            [encode_vector(&[1.0, 0.0])],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO passages (doc_id, ord, heading, dim, vector, content_hash)
             VALUES ('old-doc', 0, 'Kept', 2, ?1, 'old-hash')",
            [encode_vector(&[1.0, 0.0])],
        )
        .unwrap();
    }

    #[test]
    fn open_upgrades_a_version_1_database_and_keeps_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.db");
        write_v1_database(&path);

        let store = VectorStore::open(&path).unwrap();

        assert_eq!(schema_version(&store.conn).unwrap(), SCHEMA_VERSION);
        let hits = store
            .query(&hybrid(&[1.0, 0.0], "kept", &SearchFilter::default()))
            .unwrap();
        assert_eq!(ids(&hits), vec!["old-doc"]);
        assert_eq!(hits[0].heading.as_deref(), Some("Kept"));
        assert_eq!(hits[0].source.as_deref(), Some("records/old.md"));
        assert_eq!(hits[0].text, None);
        assert_eq!(hits[0].bm25, None);
        assert!(
            !store
                .is_current("old-doc", "old-hash", Some("records/old.md"))
                .unwrap()
        );
        assert_eq!(store.passage_stats().unwrap().documents_without_text, 1);
        assert!(
            store
                .sections("old-doc", Some("Kept"))
                .unwrap()
                .unwrap()
                .is_empty()
        );
        drop(store);
        // Opening it again changes nothing.
        let store = VectorStore::open(&path).unwrap();
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn a_writer_that_predates_text_leaves_no_stale_text_or_sections() {
        let (_dir, store) = open_temp_store();
        store
            .upsert_document(&DocumentVectors {
                id: "doc",
                vector: &[1.0, 0.0],
                passages: &[PassageVector {
                    vector: &[1.0, 0.0],
                    text: Some("zebra"),
                    ..Default::default()
                }],
                content_hash: "h1",
                updated_at: "t",
                sections: &[SectionText {
                    heading: Some("Scope"),
                    text: "## Scope",
                    span: PassageSpan {
                        start_char: 0,
                        end_char: 8,
                        start_line: 1,
                        end_line: 1,
                    },
                }],
                ..Default::default()
            })
            .unwrap();

        // Release 0.12.0 storing a changed document: these are its
        // statements, which know nothing of text or sections.
        store
            .conn
            .execute(
                "UPDATE embeddings SET content_hash = 'h2' WHERE id = 'doc'",
                [],
            )
            .unwrap();
        store
            .conn
            .execute("DELETE FROM passages WHERE doc_id = 'doc'", [])
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO passages (doc_id, ord, heading, dim, vector, content_hash)
                 VALUES ('doc', 0, NULL, 2, ?1, 'h2')",
                [encode_vector(&[1.0, 0.0])],
            )
            .unwrap();

        assert_eq!(text_rows(&store), 0);
        let hits = store
            .query(&hybrid(&[1.0, 0.0], "zebra", &SearchFilter::default()))
            .unwrap();
        assert_eq!(hits[0].bm25, None);
        // Its sections are of the old content, and are not served.
        assert!(
            store
                .sections("doc", Some("Scope"))
                .unwrap()
                .unwrap()
                .is_empty()
        );

        // And release 0.12.0 removing the document.
        store
            .conn
            .execute("DELETE FROM passages WHERE doc_id = 'doc'", [])
            .unwrap();
        store
            .conn
            .execute("DELETE FROM embeddings WHERE id = 'doc'", [])
            .unwrap();
        let sections: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM sections", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sections, 0);
    }

    #[test]
    fn a_read_only_version_1_database_opens_and_is_ranked_by_its_vectors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.db");
        write_v1_database(&path);
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).unwrap();

        let store = VectorStore::open(&path).unwrap();
        let filter = SearchFilter {
            source_prefix: Some("records/".to_string()),
            ..Default::default()
        };
        let hits = store.query(&hybrid(&[1.0, 0.0], "kept", &filter));
        let typed = store.query(&hybrid(
            &[1.0, 0.0],
            "kept",
            &SearchFilter {
                entity_types: vec!["meeting".to_string()],
                ..Default::default()
            },
        ));
        let sections = store.sections("old-doc", Some("Kept"));
        let stats = store.passage_stats();
        let version = schema_version(&store.conn);
        drop(store);
        // Restored before the assertions so the directory can always be
        // removed, on every platform.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();

        // A process that may write anything (root) upgrades the file even
        // through the read-only bit; there is then nothing to fall back from.
        if version.unwrap() == SCHEMA_VERSION {
            return;
        }
        let hits = hits.unwrap();
        assert_eq!(ids(&hits), vec!["old-doc"]);
        assert_eq!(hits[0].heading.as_deref(), Some("Kept"));
        assert_eq!(hits[0].source.as_deref(), Some("records/old.md"));
        assert_eq!(hits[0].text, None);
        assert!(typed.unwrap().is_empty());
        assert!(sections.unwrap().unwrap().is_empty());
        assert_eq!(
            stats.unwrap(),
            PassageStats {
                passage_count: 1,
                documents_without_passages: 0,
                documents_without_text: 1,
            }
        );
    }

    #[test]
    fn multi_root_query_ranks_cosine_together_and_lexical_within_each_root() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let root_a = dir_a.path().join("vectors.db");
        let root_b = dir_b.path().join("vectors.db");
        {
            let a = VectorStore::open(&root_a).unwrap();
            store_document(
                &a,
                "a-near",
                &[(None, [1.0, 0.0], "nothing here")],
                None,
                None,
            );
            store_document(
                &a,
                "a-term",
                &[(None, [0.0, 1.0], "zebra crossing")],
                None,
                None,
            );
            let b = VectorStore::open(&root_b).unwrap();
            store_document(&b, "b-term", &[(None, [0.6, 0.8], "zebra")], None, None);
        }
        let roots = [root_a.clone(), root_b.clone()];
        let filter = SearchFilter::default();

        let hits = multi_root_query(&roots, &hybrid(&[1.0, 0.0], "zebra", &filter)).unwrap();

        let found: Vec<_> = hits
            .iter()
            .map(|hit| (hit.document.id.as_str(), hit.root.clone()))
            .collect();
        // Each term match is first in its own root's lexical ranking, so
        // the two differ only by their place in the shared cosine ranking.
        assert_eq!(
            found,
            vec![
                ("b-term", root_b),
                ("a-term", root_a.clone()),
                ("a-near", root_a),
            ]
        );
        assert!((hits[0].document.score - (reciprocal(2) + reciprocal(1))).abs() < 1e-6);
        assert!((hits[1].document.score - (reciprocal(3) + reciprocal(1))).abs() < 1e-6);
        assert_eq!(hits[0].document.text.as_deref(), Some("zebra"));
        assert_eq!(hits[1].document.text.as_deref(), Some("zebra crossing"));

        let limited = multi_root_query(
            &roots,
            &SearchQuery {
                limit: 1,
                ..hybrid(&[1.0, 0.0], "zebra", &filter)
            },
        )
        .unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].document.id, "b-term");
    }

    #[test]
    fn multi_root_sections_come_from_the_first_root_that_holds_the_document() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let root_a = dir_a.path().join("vectors.db");
        let root_b = dir_b.path().join("vectors.db");
        VectorStore::open(&root_a).unwrap();
        store_document(
            &VectorStore::open(&root_b).unwrap(),
            "only-in-b",
            &[(None, [1.0, 0.0], "Summary text")],
            None,
            None,
        );
        let roots = [root_a, root_b.clone()];

        let (root, sections) = multi_root_sections(&roots, "only-in-b", None)
            .unwrap()
            .unwrap();
        assert_eq!(root, root_b);
        assert_eq!(sections[0].text, "Summary text");
        assert_eq!(multi_root_sections(&roots, "absent", None).unwrap(), None);
    }

    #[test]
    fn the_reported_passage_comes_from_the_ranking_that_places_the_document_higher() {
        let passage = |ord: i64| PassageRef {
            ord: Some(ord),
            heading: None,
        };
        let candidate = |id: &str, cosine: f32, rank: Option<usize>| Candidate {
            id: id.to_string(),
            cosine,
            cosine_passage: passage(1),
            lexical: rank.map(|rank| Lexical {
                bm25: -1.0,
                rank,
                // The document named "common" matched on common words
                // only.
                passage: (id != "common").then_some(passage(2)),
            }),
        };
        let candidates = || {
            vec![
                (0, candidate("first", 0.9, Some(2))),
                (0, candidate("second", 0.8, Some(1))),
                (0, candidate("third", 0.7, None)),
                (0, candidate("common", 0.6, Some(1))),
            ]
        };

        let ranked = fuse(candidates(), SearchMode::Hybrid, 10);
        let by_id = |id: &str| ranked.iter().find(|r| r.candidate.id == id).unwrap();
        // Tied on fused score, the two keep their cosine order.
        assert_eq!(ranked[0].candidate.id, "first");
        assert_eq!(by_id("first").passage, passage(1));
        assert_eq!(by_id("second").passage, passage(2));
        assert_eq!(by_id("third").passage, passage(1));
        // Placed higher by a match on common words alone: that match names
        // no passage, so the passage nearest in meaning is reported.
        assert_eq!(by_id("common").passage, passage(1));

        let semantic = fuse(candidates(), SearchMode::Semantic, 2);
        assert_eq!(semantic.len(), 2);
        assert!(semantic.iter().all(|r| r.passage == passage(1)));
        assert!((semantic[0].score - 0.9).abs() < f32::EPSILON);
    }
}
