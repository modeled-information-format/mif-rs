//! `SQLite`-backed search index over a corpus's findings: an FTS5
//! full-text table (for `search`) plus embedding vectors (for
//! `find_similar`).
//!
//! A finding is a durable, cross-topic-reusable research artifact — this
//! index always spans every topic in a corpus, never just the topic(s) a
//! particular `review` invocation happened to classify, so a scoped
//! `--topic` review never narrows what future recall can find.
//!
//! This is a new, `mif-rh`-owned schema — not a reuse of
//! `mif_store::VectorStore` — since that store is scoped to one flat table
//! of single MIF documents with no topic concept, while rht's corpus is
//! `reports/<topic>/findings/*.json`. Built/refreshed only by
//! [`crate::build_search_index`] (opt-in via `mif-rh-cli review
//! --build-index`, deliberately not run on every `review`), as a derived,
//! gitignored artifact conventionally kept at `reports/_meta/search-index.sqlite`.

use std::path::Path;

use rusqlite::Connection;

use crate::error::MifRhError;

/// One finding to index: its identity, topic, textual content (for FTS5),
/// and embedding vector (for cosine similarity).
#[derive(Debug, Clone)]
pub struct IndexedFinding {
    /// The finding's id.
    pub finding_id: String,
    /// The finding's topic.
    pub topic: String,
    /// The text indexed for full-text search.
    pub content: String,
    /// The finding's embedding vector.
    pub vector: Vec<f32>,
}

/// One full-text search match.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    /// The matching finding's id.
    pub finding_id: String,
    /// The matching finding's topic.
    pub topic: String,
    /// A short snippet of the matching content.
    pub snippet: String,
    /// FTS5's bm25 relevance score for this match (more negative is more
    /// relevant — bm25's own convention, not normalized to `0.0..=1.0`).
    pub score: f64,
}

/// One similarity match, ranked by cosine similarity.
#[derive(Debug, Clone, PartialEq)]
pub struct SimilarFinding {
    /// The matching finding's id.
    pub finding_id: String,
    /// The matching finding's topic.
    pub topic: String,
    /// Cosine similarity to the query, in `-1.0..=1.0`.
    pub score: f32,
}

/// Aggregate statistics over the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexStats {
    /// Number of distinct topics represented in the index.
    pub topics: u64,
    /// Total number of indexed findings.
    pub findings: u64,
}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS findings (
    finding_id TEXT PRIMARY KEY,
    topic TEXT NOT NULL,
    content TEXT NOT NULL,
    dim INTEGER NOT NULL,
    vector BLOB NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS findings_fts USING fts5(
    finding_id UNINDEXED,
    topic UNINDEXED,
    content
);
";

/// A `SQLite`-backed index of a corpus's findings.
#[derive(Debug)]
pub struct FindingIndex {
    conn: Connection,
}

impl FindingIndex {
    /// Opens (creating if absent) the index database at `path`, requiring
    /// its parent directory to already exist.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::Index`] if `SQLite` fails to open the database
    /// or initialize its schema.
    pub fn open(path: &Path) -> Result<Self, MifRhError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self { conn })
    }

    /// Rebuilds the index from scratch with exactly `findings` — matching
    /// `ontology-map.json`'s own "rebuild deterministically from disk"
    /// convention. Any finding previously indexed but absent from
    /// `findings` is dropped.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::Index`] if the underlying `SQLite` statements
    /// fail.
    pub fn rebuild(&mut self, findings: &[IndexedFinding]) -> Result<(), MifRhError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM findings", [])?;
        tx.execute("DELETE FROM findings_fts", [])?;
        for finding in findings {
            let blob = encode_vector(&finding.vector);
            tx.execute(
                "INSERT INTO findings (finding_id, topic, content, dim, vector)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    finding.finding_id,
                    finding.topic,
                    finding.content,
                    i64::try_from(finding.vector.len()).unwrap_or(i64::MAX),
                    blob
                ],
            )?;
            tx.execute(
                "INSERT INTO findings_fts (finding_id, topic, content) VALUES (?1, ?2, ?3)",
                rusqlite::params![finding.finding_id, finding.topic, finding.content],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Full-text query, ranked by FTS5's built-in relevance ranking.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::Index`] if the underlying `SQLite` statement
    /// fails (including a malformed FTS5 query string).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchMatch>, MifRhError> {
        let mut stmt = self.conn.prepare(
            "SELECT finding_id, topic, snippet(findings_fts, 2, '', '', '...', 8), bm25(findings_fts)
             FROM findings_fts WHERE findings_fts MATCH ?1
             ORDER BY rank LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(rusqlite::params![query, limit], |row| {
            Ok(SearchMatch {
                finding_id: row.get(0)?,
                topic: row.get(1)?,
                snippet: row.get(2)?,
                score: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Ranks every indexed finding against `query` by cosine similarity,
    /// excluding `exclude_finding_id` if given. Brute-force, matching
    /// `mif_store::VectorStore::top_k_similar`'s own approach at this
    /// corpus scale.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::Index`] if the underlying `SQLite` statement
    /// fails.
    pub fn find_similar(
        &self,
        query: &[f32],
        limit: usize,
        exclude_finding_id: Option<&str>,
    ) -> Result<Vec<SimilarFinding>, MifRhError> {
        let mut stmt = self
            .conn
            .prepare("SELECT finding_id, topic, dim, vector FROM findings")?;
        let rows = stmt.query_map([], |row| {
            let finding_id: String = row.get(0)?;
            let topic: String = row.get(1)?;
            let dim: i64 = row.get(2)?;
            let blob: Vec<u8> = row.get(3)?;
            Ok((finding_id, topic, dim, blob))
        })?;

        let query_norm = norm(query);
        let mut matches = Vec::new();
        for row in rows {
            let (finding_id, topic, dim, blob) = row?;
            if exclude_finding_id.is_some_and(|excluded| excluded == finding_id) {
                continue;
            }
            let vector = decode_vector(&blob);
            let dim = usize::try_from(dim).unwrap_or(usize::MAX);
            if vector.len() != dim || dim != query.len() {
                continue;
            }
            let score = cosine_similarity_with_norm(query, &vector, query_norm);
            matches.push(SimilarFinding {
                finding_id,
                topic,
                score,
            });
        }

        matches.sort_by(|a, b| b.score.total_cmp(&a.score));
        matches.truncate(limit);
        Ok(matches)
    }

    /// Aggregate statistics over the index.
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::Index`] if the underlying `SQLite` statements
    /// fail.
    pub fn stats(&self) -> Result<IndexStats, MifRhError> {
        let findings: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM findings", [], |row| row.get(0))?;
        let topics: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT topic) FROM findings", [], |row| {
                    row.get(0)
                })?;
        Ok(IndexStats {
            #[allow(clippy::cast_sign_loss)]
            topics: topics as u64,
            #[allow(clippy::cast_sign_loss)]
            findings: findings as u64,
        })
    }
}

fn norm(vector: &[f32]) -> f32 {
    vector.iter().map(|c| c * c).sum::<f32>().sqrt()
}

fn cosine_similarity_with_norm(a: &[f32], b: &[f32], a_norm: f32) -> f32 {
    let b_norm = norm(b);
    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    dot / (a_norm * b_norm)
}

/// Cosine similarity between two vectors, in `-1.0..=1.0`.
///
/// Computes both norms internally, for one-shot pairwise comparisons (e.g.
/// `mif-rh-mcp`'s `suggest_type`, ranking a query against a handful of
/// candidate descriptions). A hot loop ranking many vectors against one
/// fixed query (see `find_similar` above) precomputes that query's norm
/// once and calls the internal norm-carrying version directly instead, to
/// avoid recomputing it on every row.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    cosine_similarity_with_norm(a, b, norm(a))
}

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|c| c.to_le_bytes()).collect()
}

fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{FindingIndex, IndexedFinding};

    fn open_temp() -> (tempfile::TempDir, FindingIndex) {
        let dir = tempfile::tempdir().unwrap();
        let index = FindingIndex::open(&dir.path().join("index.sqlite")).unwrap();
        (dir, index)
    }

    fn finding(id: &str, topic: &str, content: &str, vector: Vec<f32>) -> IndexedFinding {
        IndexedFinding {
            finding_id: id.to_string(),
            topic: topic.to_string(),
            content: content.to_string(),
            vector,
        }
    }

    #[test]
    fn rebuild_then_search_finds_matching_content() {
        let (_dir, mut index) = open_temp();
        index
            .rebuild(&[
                finding(
                    "f-1",
                    "edu",
                    "a great textbook about algebra",
                    vec![1.0, 0.0],
                ),
                finding("f-2", "edu", "quarterly revenue report", vec![0.0, 1.0]),
            ])
            .unwrap();

        let matches = index.search("textbook", 10).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].finding_id, "f-1");
    }

    #[test]
    fn rebuild_drops_findings_no_longer_present() {
        let (_dir, mut index) = open_temp();
        index
            .rebuild(&[finding("f-1", "edu", "first", vec![1.0])])
            .unwrap();
        assert_eq!(index.stats().unwrap().findings, 1);

        index
            .rebuild(&[finding("f-2", "edu", "second", vec![1.0])])
            .unwrap();
        let stats = index.stats().unwrap();
        assert_eq!(stats.findings, 1);
        assert!(index.search("first", 10).unwrap().is_empty());
    }

    #[test]
    fn find_similar_ranks_the_closest_vector_first_and_excludes_the_anchor() {
        let (_dir, mut index) = open_temp();
        index
            .rebuild(&[
                finding("same", "edu", "x", vec![1.0, 0.0]),
                finding("orthogonal", "edu", "x", vec![0.0, 1.0]),
                finding("anchor", "edu", "x", vec![1.0, 0.0]),
            ])
            .unwrap();

        let matches = index.find_similar(&[1.0, 0.0], 10, Some("anchor")).unwrap();
        assert_eq!(matches[0].finding_id, "same");
        assert!(!matches.iter().any(|m| m.finding_id == "anchor"));
    }

    #[test]
    fn stats_counts_distinct_topics() {
        let (_dir, mut index) = open_temp();
        index
            .rebuild(&[
                finding("f-1", "edu", "x", vec![1.0]),
                finding("f-2", "eng", "x", vec![1.0]),
                finding("f-3", "eng", "x", vec![1.0]),
            ])
            .unwrap();
        let stats = index.stats().unwrap();
        assert_eq!(stats.topics, 2);
        assert_eq!(stats.findings, 3);
    }
}
