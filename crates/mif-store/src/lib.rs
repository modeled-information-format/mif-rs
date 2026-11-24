//! `SQLite`-backed vector store for document embeddings in the MIF (Modeled
//! Information Format) ecosystem.
//!
//! [`VectorStore`] wraps a single `SQLite` connection and two tables:
//! `embeddings`, one row per document (id, dim, vector, `content_hash`,
//! `updated_at`, source, namespace), and `passages`, the vectors of the
//! passages a document was split into (see "Documents and Passages"). Vectors
//! are stored as a raw little-endian `f32` blob — see "Vector Blob Layout"
//! below for the exact on-disk format. This crate does not compute
//! embeddings or timestamps; both are supplied by the caller, keeping the
//! store itself deterministic and easy to test.
//!
//! A single [`VectorStore`] is always rooted at one database file. Querying
//! *across* several roots at once (e.g. a project-local store layered with
//! a shared central one) is not a method on [`VectorStore`] itself — it is
//! the free functions [`multi_root_top_k_similar`], [`multi_root_get`], and
//! [`multi_root_stats`], which each open every given root independently and
//! merge the results. See their doc comments for the merge and failure
//! semantics.
//!
//! # Documents and Passages
//!
//! A document's `embeddings` row holds one vector that stands for the whole
//! document; [`VectorStore::top_k_similar`] ranks those, which is what
//! document-to-document similarity wants. [`VectorStore::upsert_document`]
//! also stores a vector per passage, and [`VectorStore::search`] ranks each
//! document by its best-matching passage and reports that passage's
//! heading. A document stored without passages (by [`VectorStore::upsert`],
//! or by a release before passages existed) is ranked by its document
//! vector, so a store holding both kinds stays searchable.
//!
//! # Schema Versions
//!
//! `PRAGMA user_version` records the schema: 0 is the single-table layout
//! of releases up to 0.11.0, and 1 adds the `passages` table and the
//! `source` and `namespace` columns. [`VectorStore::open`] upgrades an
//! older database in place, in one transaction, without rewriting any row.
//! A release that predates passages can still read and write an upgraded
//! database; it sees only the `embeddings` table, so a document it writes
//! keeps whatever passages were stored for it until the document is stored
//! again by a release that knows about them.
//!
//! # Vector Blob Layout
//!
//! The `vector` column holds `dim * 4` bytes: each `f32` component encoded
//! as 4 bytes, little-endian, in component order, with no header or
//! padding. This is a private, on-disk format read and written only by this
//! crate — it is not a public interchange format.

use std::collections::{HashMap, HashSet, hash_map::Entry};
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};

use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

/// Errors from opening or operating on a [`VectorStore`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The parent directory of the requested database path does not exist.
    /// This crate never creates it — that is the caller's responsibility.
    #[error("parent directory {path} of the database path does not exist")]
    MissingParentDir {
        /// The missing parent directory.
        path: String,
    },
    /// Failed to open the `SQLite` database or initialize its schema.
    #[error("failed to open database at {path}: {source}")]
    Open {
        /// The path that failed to open.
        path: String,
        /// The underlying `SQLite` error.
        #[source]
        source: rusqlite::Error,
    },
    /// A `SQLite` query failed.
    #[error("sqlite query failed: {source}")]
    Query {
        /// The underlying `SQLite` error.
        #[source]
        source: rusqlite::Error,
    },
    /// The vector to store has more components than can be represented in
    /// the `dim` column.
    #[error("vector has {len} dimensions, which cannot be represented in the embeddings table")]
    DimensionTooLarge {
        /// The offending vector's length.
        len: usize,
    },
    /// The `dim` value read back from the database does not fit in a
    /// [`usize`]. This can only happen if the database was modified outside
    /// this crate.
    #[error("dim column value {value} read from the database is corrupt")]
    CorruptDimension {
        /// The corrupt value read from the `dim` column.
        value: i64,
    },
    /// The `vector` blob's decoded component count does not match the row's
    /// own `dim` column. This can only happen if the database was modified
    /// outside this crate (external tampering, a crash mid-write, disk
    /// bit-rot) — the same threat model as [`Self::CorruptDimension`], but
    /// the corruption lives in the `vector` blob rather than the `dim`
    /// column.
    #[error(
        "vector blob for '{id}' decoded to {actual_len} components, but the dim column says {expected_dim}"
    )]
    VectorBlobMismatch {
        /// The row's id.
        id: String,
        /// The dim column's expected component count.
        expected_dim: usize,
        /// The actual number of complete `f32` components decoded from the
        /// blob.
        actual_len: usize,
    },
    /// A multi-root operation ([`multi_root_top_k_similar`],
    /// [`multi_root_get`], [`multi_root_stats`]) failed while querying one
    /// specific, already-opened root. Wraps the underlying [`StoreError`]
    /// with the root path that produced it, so a [`Self::Query`] failure
    /// (which carries no path of its own) is still diagnosable in a
    /// multi-root context, matching the diagnosability [`Self::Open`]/
    /// [`Self::MissingParentDir`] already have on their own — those two
    /// variants are returned unwrapped from a multi-root call, since they
    /// already name the failing root's path directly.
    #[error("multi-root operation failed at root {root}: {source}")]
    RootFailure {
        /// The root that produced the failure, in the order the caller's
        /// `roots` slice was given.
        root: String,
        /// The underlying failure.
        #[source]
        source: Box<Self>,
    },
}

impl StoreError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::MissingParentDir { .. } => ProblemMeta {
                slug: "missing-parent-dir",
                version: "v1",
                title: "Database path's parent directory does not exist",
                status: 400,
                exit_code: 2,
            },
            Self::Open { .. } => ProblemMeta {
                slug: "open-database-failure",
                version: "v1",
                title: "Failed to open the database or initialize its schema",
                status: 500,
                exit_code: 1,
            },
            Self::Query { .. } => ProblemMeta {
                slug: "sqlite-query-failure",
                version: "v1",
                title: "A SQLite query failed",
                status: 500,
                exit_code: 1,
            },
            Self::DimensionTooLarge { .. } => ProblemMeta {
                slug: "dimension-too-large",
                version: "v1",
                title: "Vector has too many dimensions to store",
                status: 400,
                exit_code: 2,
            },
            Self::CorruptDimension { .. } => ProblemMeta {
                slug: "corrupt-dimension",
                version: "v1",
                title: "Stored dim column value is corrupt",
                status: 500,
                exit_code: 1,
            },
            Self::VectorBlobMismatch { .. } => ProblemMeta {
                slug: "vector-blob-mismatch",
                version: "v1",
                title: "Stored vector blob does not match its dim column",
                status: 500,
                exit_code: 1,
            },
            Self::RootFailure { .. } => ProblemMeta {
                slug: "multi-root-operation-failure",
                version: "v1",
                title: "A multi-root operation failed at one root",
                status: 500,
                exit_code: 1,
            },
        }
    }
}

impl ToProblem for StoreError {
    fn to_problem(&self) -> ProblemDetails {
        let (fix, action) = match self {
            Self::MissingParentDir { .. } => (
                SuggestedFix::new(
                    "Create the parent directory of the database path, then retry.",
                    Applicability::MachineApplicable,
                ),
                CodeAction::new(
                    "Create the missing parent directory",
                    "quickfix",
                    Applicability::MachineApplicable,
                ),
            ),
            Self::DimensionTooLarge { .. } => (
                SuggestedFix::new(
                    "Supply a shorter embedding vector.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "Reduce the vector's dimensionality",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            ),
            Self::Open { .. }
            | Self::Query { .. }
            | Self::CorruptDimension { .. }
            | Self::VectorBlobMismatch { .. } => (
                SuggestedFix::new(
                    "This indicates a corrupt or inaccessible database file. Verify the file's \
                     permissions and integrity, or delete it to let mif-store recreate it.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "Inspect or recreate the database file",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            ),
            Self::RootFailure { .. } => (
                SuggestedFix::new(
                    "Inspect the wrapped error to see what happened at the named root; fix that \
                     root specifically (e.g. its permissions, integrity, or missing parent \
                     directory) rather than assuming every queried root failed.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "Inspect the failing root",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            ),
        };
        self.meta()
            .into_details(env!("CARGO_PKG_NAME"), self.to_string())
            .with_suggested_fix(fix)
            .with_code_action(action)
    }
}

const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS embeddings (
    id TEXT PRIMARY KEY,
    dim INTEGER NOT NULL,
    vector BLOB NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at TEXT NOT NULL
)";

/// The schema version this crate reads and writes (see "Schema Versions"
/// in the module documentation).
const SCHEMA_VERSION: i64 = 1;

/// Upgrades a version-0 database to version 1.
const MIGRATE_V1_SQL: &str = "ALTER TABLE embeddings ADD COLUMN source TEXT;
ALTER TABLE embeddings ADD COLUMN namespace TEXT;
CREATE TABLE IF NOT EXISTS passages (
    doc_id TEXT NOT NULL,
    ord INTEGER NOT NULL,
    heading TEXT,
    dim INTEGER NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (doc_id, ord)
);";

/// Every vector [`VectorStore::search`] scores: each passage of each
/// document that passes the filter, or the document's own vector when it
/// has no passages. A `NULL` filter parameter matches every row; a row
/// whose `source` or `namespace` is `NULL` matches no prefix.
const SEARCH_SQL: &str =
    "SELECT e.id, p.heading, COALESCE(p.dim, e.dim), COALESCE(p.vector, e.vector)
FROM embeddings e
LEFT JOIN passages p ON p.doc_id = e.id
WHERE (?1 IS NULL OR substr(e.id, 1, length(?1)) = ?1)
  AND (?2 IS NULL OR substr(e.source, 1, length(?2)) = ?2)
  AND (?3 IS NULL OR substr(e.namespace, 1, length(?3)) = ?3)";

/// One passage vector of a document, for [`VectorStore::upsert_document`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassageVector<'a> {
    /// The heading of the section the passage came from, if it has one.
    pub heading: Option<&'a str>,
    /// The passage's embedding vector.
    pub vector: &'a [f32],
}

/// Everything [`VectorStore::upsert_document`] stores for one document.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DocumentVectors<'a> {
    /// The document's id.
    pub id: &'a str,
    /// The vector that stands for the whole document, ranked by
    /// [`VectorStore::top_k_similar`].
    pub vector: &'a [f32],
    /// The document's passages, in order. May be empty, in which case
    /// [`VectorStore::search`] ranks the document by `vector`.
    pub passages: &'a [PassageVector<'a>],
    /// Hash of the content the vectors were computed from.
    pub content_hash: &'a str,
    /// RFC3339 UTC timestamp of this write.
    pub updated_at: &'a str,
    /// Where the document was read from (a path), for
    /// [`SearchFilter::source_prefix`].
    pub source: Option<&'a str>,
    /// The document's MIF namespace, for
    /// [`SearchFilter::namespace_prefix`].
    pub namespace: Option<&'a str>,
}

/// Limits [`VectorStore::search`] to some of the stored documents.
///
/// Every field set must match; the default matches every document. A
/// document stored without a `source` or `namespace` matches no prefix on
/// that field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilter {
    /// Only documents whose id starts with this.
    pub id_prefix: Option<String>,
    /// Only documents whose source starts with this.
    pub source_prefix: Option<String>,
    /// Only documents whose namespace starts with this.
    pub namespace_prefix: Option<String>,
}

/// One ranked document from [`VectorStore::search`].
#[derive(Debug, Clone, PartialEq)]
pub struct SearchMatch {
    /// The matching document's id.
    pub id: String,
    /// Cosine similarity of the query to the document's best-matching
    /// passage, in `-1.0..=1.0`.
    pub score: f32,
    /// The heading of that passage. `None` when the best match is the
    /// summary passage, text before the first heading, or a document stored
    /// without passages.
    pub heading: Option<String>,
}

/// One ranked document from [`multi_root_search`], with the store root it
/// came from. See [`RootedMatch`] for why the root matters.
#[derive(Debug, Clone, PartialEq)]
pub struct RootedSearchMatch {
    /// The matching document's id.
    pub id: String,
    /// Cosine similarity of the query to the document's best-matching
    /// passage, in `-1.0..=1.0`.
    pub score: f32,
    /// The heading of that passage; see [`SearchMatch::heading`].
    pub heading: Option<String>,
    /// The store root ([`VectorStore::open`] path) this match came from.
    pub root: PathBuf,
}

/// How much of a [`VectorStore`] is stored as passages, from
/// [`VectorStore::passage_stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassageStats {
    /// Total number of passage vectors stored.
    pub passage_count: u64,
    /// Documents with no passages, which [`VectorStore::search`] ranks by
    /// their document vector alone. A non-zero count after an upgrade means
    /// those documents have not been ingested again yet.
    pub documents_without_passages: u64,
}

/// A single embedding row read back from a [`VectorStore`].
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVector {
    /// Number of `f32` components in `vector`.
    pub dim: usize,
    /// The embedding vector.
    pub vector: Vec<f32>,
    /// Hash of the content the vector was computed from, supplied by the
    /// caller at upsert time.
    pub content_hash: String,
    /// RFC3339 UTC timestamp of the last write, supplied by the caller at
    /// upsert time.
    pub updated_at: String,
}

/// One ranked result from [`VectorStore::top_k_similar`].
#[derive(Debug, Clone, PartialEq)]
pub struct SimilarityMatch {
    /// The matching row's id.
    pub id: String,
    /// Cosine similarity to the query vector, in `-1.0..=1.0` (close to
    /// `1.0` for near-duplicates of a normalized embedding space).
    pub score: f32,
}

/// One ranked result from [`multi_root_top_k_similar`], additionally
/// carrying which store root produced it.
///
/// Unlike [`SimilarityMatch`], the root is load-bearing here: two distinct
/// [`VectorStore`] roots can independently contain a row with the same `id`
/// (e.g. a document ingested into both a project-local store and a central
/// one), so a caller that only has an `id` cannot tell which physical row
/// a match refers to without it. This also matters for correctness, not
/// just display: excluding a `find-similar` anchor by `id` must be applied
/// to the full multi-root merged result set (every root's copy of that
/// `id`, if more than one root has it), not just the single root the
/// anchor happened to be looked up in — losing the root would make it easy
/// to filter only the home root's copy and leak a duplicate through from
/// another root.
#[derive(Debug, Clone, PartialEq)]
pub struct RootedMatch {
    /// The matching row's id.
    pub id: String,
    /// Cosine similarity to the query vector, in `-1.0..=1.0`.
    pub score: f32,
    /// The store root ([`VectorStore::open`] path) this match came from.
    pub root: PathBuf,
}

/// Summary statistics over a [`VectorStore`]'s contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusStats {
    /// Total number of embeddings stored.
    pub count: u64,
    /// The dimensionality of a stored embedding, if the store is non-empty.
    /// Different rows may carry different dimensionalities (e.g. after an
    /// embedding-model change); this reports one representative value, not
    /// a guarantee that every row shares it.
    pub dim: Option<usize>,
}

/// Summary statistics over multiple [`VectorStore`] roots, from
/// [`multi_root_stats`].
///
/// Deliberately does not collapse `dim` across roots into one value the way
/// [`CorpusStats::dim`] does within a single store: two roots are far more
/// likely than two rows in one store to have been populated by different
/// embedding models (e.g. a project-local store and a central store
/// maintained by an entirely separate tool), so picking one representative
/// `dim` across roots would silently hide that. `per_root` preserves each
/// root's own [`CorpusStats`] (including its own representative `dim`)
/// verbatim; only the row count is meaningfully summed workspace-wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiRootStats {
    /// Each queried root's path, paired with its own [`CorpusStats`], in
    /// the same order the roots were given.
    pub per_root: Vec<(PathBuf, CorpusStats)>,
    /// The sum of every root's [`CorpusStats::count`].
    pub total_count: u64,
}

/// A `SQLite`-backed store of document embedding vectors.
#[derive(Debug)]
pub struct VectorStore {
    conn: Connection,
}

impl VectorStore {
    /// Opens the `SQLite` database at `path`, creating its tables if they do
    /// not already exist and upgrading an older database's schema in place
    /// (see "Schema Versions" in the module documentation). The database file itself is created by
    /// `SQLite` if absent; the parent directory of `path` is not — a missing
    /// parent directory is reported as an error rather than silently
    /// created.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::MissingParentDir`] if `path` has a parent
    /// directory that does not exist, or [`StoreError::Open`] if `SQLite`
    /// fails to open the database or initialize its schema.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(StoreError::MissingParentDir {
                path: parent.display().to_string(),
            });
        }

        let open_error = |source| StoreError::Open {
            path: path.display().to_string(),
            source,
        };
        let mut conn = Connection::open(path).map_err(open_error)?;
        conn.execute(SCHEMA_SQL, []).map_err(open_error)?;
        migrate(&mut conn).map_err(open_error)?;

        Ok(Self { conn })
    }

    /// Inserts a new embedding, or replaces the existing one for `id`.
    ///
    /// Any passages stored for `id` are removed, since they were computed
    /// from the content this call replaces; [`Self::search`] then ranks the
    /// document by `vector`. Use [`Self::upsert_document`] to store passages.
    ///
    /// `vector` is stored as a raw little-endian `f32` blob (see the module
    /// documentation). `updated_at` is stored verbatim and should be an
    /// RFC3339 UTC timestamp supplied by the caller.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::DimensionTooLarge`] if `vector` is too long to
    /// represent in the `dim` column, or [`StoreError::Query`] if the
    /// underlying `SQLite` statement fails.
    pub fn upsert(
        &self,
        id: &str,
        vector: &[f32],
        content_hash: &str,
        updated_at: &str,
    ) -> Result<(), StoreError> {
        let dim = i64::try_from(vector.len())
            .map_err(|_| StoreError::DimensionTooLarge { len: vector.len() })?;
        let blob = encode_vector(vector);

        let tx = self.conn.unchecked_transaction().map_err(query_error)?;
        tx.execute(
            "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                dim = excluded.dim,
                vector = excluded.vector,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at",
            params![id, dim, blob, content_hash, updated_at],
        )
        .map_err(query_error)?;
        tx.execute("DELETE FROM passages WHERE doc_id = ?1", [id])
            .map_err(query_error)?;
        tx.commit().map_err(query_error)
    }

    /// Stores a document's vector and its passages, replacing whatever was
    /// stored for the same id. The write is one transaction: a reader sees
    /// the document's old vectors or its new ones, never a mix.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::DimensionTooLarge`] if a vector, or the number
    /// of passages, is too large to record, or [`StoreError::Query`] if an
    /// underlying `SQLite` statement fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use mif_store::{DocumentVectors, PassageVector, SearchFilter, VectorStore};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let dir = tempfile::tempdir()?;
    /// let store = VectorStore::open(&dir.path().join("vectors.db"))?;
    /// store.upsert_document(&DocumentVectors {
    ///     id: "urn:mif:doc",
    ///     vector: &[1.0, 0.0],
    ///     passages: &[
    ///         PassageVector { heading: None, vector: &[1.0, 0.0] },
    ///         PassageVector { heading: Some("Scope"), vector: &[0.0, 1.0] },
    ///     ],
    ///     content_hash: "h",
    ///     updated_at: "2026-01-01T00:00:00Z",
    ///     source: Some("records/doc.md"),
    ///     namespace: None,
    /// })?;
    ///
    /// let hits = store.search(&[0.1, 0.9], 10, &SearchFilter::default())?;
    /// assert_eq!(hits[0].id, "urn:mif:doc");
    /// assert_eq!(hits[0].heading.as_deref(), Some("Scope"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn upsert_document(&self, document: &DocumentVectors<'_>) -> Result<(), StoreError> {
        let tx = self.conn.unchecked_transaction().map_err(query_error)?;
        tx.execute(
            "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at, source, namespace)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                dim = excluded.dim,
                vector = excluded.vector,
                content_hash = excluded.content_hash,
                updated_at = excluded.updated_at,
                source = excluded.source,
                namespace = excluded.namespace",
            params![
                document.id,
                dim_of(document.vector)?,
                encode_vector(document.vector),
                document.content_hash,
                document.updated_at,
                document.source,
                document.namespace,
            ],
        )
        .map_err(query_error)?;
        tx.execute("DELETE FROM passages WHERE doc_id = ?1", [document.id])
            .map_err(query_error)?;
        for (ord, passage) in document.passages.iter().enumerate() {
            let ord = i64::try_from(ord).map_err(|_| StoreError::DimensionTooLarge { len: ord })?;
            tx.execute(
                "INSERT INTO passages (doc_id, ord, heading, dim, vector)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    document.id,
                    ord,
                    passage.heading,
                    dim_of(passage.vector)?,
                    encode_vector(passage.vector),
                ],
            )
            .map_err(query_error)?;
        }
        tx.commit().map_err(query_error)
    }

    /// Removes a document's vector and passages. Returns whether anything
    /// was stored for `id`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] if an underlying `SQLite` statement
    /// fails.
    pub fn remove(&self, id: &str) -> Result<bool, StoreError> {
        let tx = self.conn.unchecked_transaction().map_err(query_error)?;
        let removed = delete_document(&tx, id)?;
        tx.commit().map_err(query_error)?;
        Ok(removed)
    }

    /// Removes every document whose id is not in `keep`, with its passages,
    /// in one transaction. Returns the removed ids in ascending order.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] if an underlying `SQLite` statement
    /// fails.
    pub fn prune<S: BuildHasher>(
        &self,
        keep: &HashSet<String, S>,
    ) -> Result<Vec<String>, StoreError> {
        let tx = self.conn.unchecked_transaction().map_err(query_error)?;
        let stale = {
            let mut stmt = tx
                .prepare("SELECT id FROM embeddings UNION SELECT doc_id FROM passages ORDER BY 1")
                .map_err(query_error)?;
            let ids = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(query_error)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(query_error)?;
            ids.into_iter()
                .filter(|id| !keep.contains(id))
                .collect::<Vec<_>>()
        };
        for id in &stale {
            delete_document(&tx, id)?;
        }
        tx.commit().map_err(query_error)?;
        Ok(stale)
    }

    /// How much of the store is stored as passages.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] if the underlying `SQLite` statement
    /// fails.
    pub fn passage_stats(&self) -> Result<PassageStats, StoreError> {
        let (passages, without): (i64, i64) = self
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM passages),
                        (SELECT COUNT(*) FROM embeddings e
                         WHERE NOT EXISTS (SELECT 1 FROM passages p WHERE p.doc_id = e.id))",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(query_error)?;
        // COUNT(*) is never negative, so these narrowings are always exact.
        #[allow(clippy::cast_sign_loss)]
        let stats = PassageStats {
            passage_count: passages as u64,
            documents_without_passages: without as u64,
        };
        Ok(stats)
    }

    /// Ranks stored documents against `query`, each by the cosine
    /// similarity of its best-matching passage, returning the top `limit`
    /// in descending score order (ties in ascending id order).
    ///
    /// A document stored without passages is scored on its document vector.
    /// Only documents passing `filter` are scored. Like
    /// [`Self::top_k_similar`], this is brute-force, skips vectors whose
    /// dimensionality differs from the query's, and aborts on a vector
    /// whose stored length does not match its `dim` column.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::CorruptDimension`] if a row's `dim` column does
    /// not fit a [`usize`], [`StoreError::VectorBlobMismatch`] if a row's
    /// decoded vector length does not match its `dim` column, or
    /// [`StoreError::Query`] if the underlying `SQLite` statement fails.
    pub fn search(
        &self,
        query: &[f32],
        limit: usize,
        filter: &SearchFilter,
    ) -> Result<Vec<SearchMatch>, StoreError> {
        let mut stmt = self.conn.prepare(SEARCH_SQL).map_err(query_error)?;
        let rows = stmt
            .query_map(
                params![
                    filter.id_prefix,
                    filter.source_prefix,
                    filter.namespace_prefix
                ],
                |row| {
                    let id: String = row.get(0)?;
                    let heading: Option<String> = row.get(1)?;
                    let dim_raw: i64 = row.get(2)?;
                    let blob: Vec<u8> = row.get(3)?;
                    Ok((id, heading, dim_raw, blob))
                },
            )
            .map_err(query_error)?;

        let query_norm = norm(query);
        let mut best: HashMap<String, (f32, Option<String>)> = HashMap::new();
        for row in rows {
            let (id, heading, dim_raw, blob) = row.map_err(query_error)?;
            let vector = checked_vector(&id, dim_raw, &blob)?;
            if vector.len() != query.len() {
                continue;
            }
            let score = cosine_similarity(query, &vector, query_norm);
            match best.entry(id) {
                Entry::Occupied(mut entry) if score > entry.get().0 => {
                    entry.insert((score, heading));
                },
                Entry::Occupied(_) => {},
                Entry::Vacant(entry) => {
                    entry.insert((score, heading));
                },
            }
        }

        let mut matches: Vec<SearchMatch> = best
            .into_iter()
            .map(|(id, (score, heading))| SearchMatch { id, score, heading })
            .collect();
        matches.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
        matches.truncate(limit);
        Ok(matches)
    }

    /// Looks up the embedding stored for `id`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::CorruptDimension`] if the stored `dim` value
    /// does not fit in a [`usize`], [`StoreError::VectorBlobMismatch`] if the
    /// decoded `vector` blob's component count does not match `dim`, or
    /// [`StoreError::Query`] if the underlying `SQLite` statement fails.
    pub fn get(&self, id: &str) -> Result<Option<StoredVector>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT dim, vector, content_hash, updated_at FROM embeddings WHERE id = ?1")
            .map_err(|source| StoreError::Query { source })?;

        let row = stmt
            .query_row([id], |row| {
                let dim_raw: i64 = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                let content_hash: String = row.get(2)?;
                let updated_at: String = row.get(3)?;
                Ok((dim_raw, blob, content_hash, updated_at))
            })
            .optional()
            .map_err(|source| StoreError::Query { source })?;

        row.map(|(dim_raw, blob, content_hash, updated_at)| {
            let dim = usize::try_from(dim_raw)
                .map_err(|_| StoreError::CorruptDimension { value: dim_raw })?;
            let vector = decode_vector(&blob);
            if vector.len() != dim {
                return Err(StoreError::VectorBlobMismatch {
                    id: id.to_string(),
                    expected_dim: dim,
                    actual_len: vector.len(),
                });
            }
            Ok(StoredVector {
                dim,
                vector,
                content_hash,
                updated_at,
            })
        })
        .transpose()
    }

    /// Returns the total number of embeddings in the store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] if the underlying `SQLite` statement
    /// fails.
    pub fn count(&self) -> Result<u64, StoreError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .map_err(|source| StoreError::Query { source })?;

        // COUNT(*) is never negative, so this narrowing is always exact.
        #[allow(clippy::cast_sign_loss)]
        let count = count as u64;
        Ok(count)
    }

    /// Summary statistics over the store's contents.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Query`] if the underlying `SQLite` statement
    /// fails, or [`StoreError::CorruptDimension`] if the sampled row's `dim`
    /// column does not fit a [`usize`].
    pub fn stats(&self) -> Result<CorpusStats, StoreError> {
        let count = self.count()?;
        let dim_raw: Option<i64> = self
            .conn
            .query_row("SELECT dim FROM embeddings LIMIT 1", [], |row| row.get(0))
            .optional()
            .map_err(|source| StoreError::Query { source })?;
        let dim = dim_raw
            .map(|value| usize::try_from(value).map_err(|_| StoreError::CorruptDimension { value }))
            .transpose()?;
        Ok(CorpusStats { count, dim })
    }

    /// Ranks every stored embedding against `query` by cosine similarity,
    /// returning the top `limit` matches in descending score order.
    ///
    /// Brute-force: decodes and scores every row in the store. At the
    /// corpus sizes this crate targets (a few thousand rows), this is
    /// simpler and fast enough that no approximate-nearest-neighbor index
    /// is warranted. Rows whose `dim` differs from `query.len()` are
    /// skipped (an incomparable embedding space, e.g. after a model
    /// change — not data corruption); rows whose decoded vector length
    /// does not match their own `dim` column are data corruption and abort
    /// the query with [`StoreError::VectorBlobMismatch`], matching
    /// [`Self::get`]'s behavior, rather than silently skipping them.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::CorruptDimension`] if a row's `dim` column does
    /// not fit a [`usize`], [`StoreError::VectorBlobMismatch`] if a row's
    /// decoded vector length does not match its `dim` column, or
    /// [`StoreError::Query`] if the underlying `SQLite` statement fails.
    pub fn top_k_similar(
        &self,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<SimilarityMatch>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, dim, vector FROM embeddings")
            .map_err(|source| StoreError::Query { source })?;

        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let dim_raw: i64 = row.get(1)?;
                let blob: Vec<u8> = row.get(2)?;
                Ok((id, dim_raw, blob))
            })
            .map_err(|source| StoreError::Query { source })?;

        let query_norm = norm(query);
        let mut matches = Vec::new();
        for row in rows {
            let (id, dim_raw, blob) = row.map_err(|source| StoreError::Query { source })?;
            let dim = usize::try_from(dim_raw)
                .map_err(|_| StoreError::CorruptDimension { value: dim_raw })?;
            let vector = decode_vector(&blob);
            if vector.len() != dim {
                return Err(StoreError::VectorBlobMismatch {
                    id,
                    expected_dim: dim,
                    actual_len: vector.len(),
                });
            }
            if dim != query.len() {
                continue;
            }
            let score = cosine_similarity(query, &vector, query_norm);
            matches.push(SimilarityMatch { id, score });
        }

        matches.sort_by(|a, b| b.score.total_cmp(&a.score));
        matches.truncate(limit);
        Ok(matches)
    }
}

/// Ranks every stored embedding across multiple [`VectorStore`] roots
/// against `query` by cosine similarity, returning the top `limit` matches
/// in descending score order.
///
/// Each root is opened via [`VectorStore::open`] and queried independently
/// with [`VectorStore::top_k_similar`] (itself asked for the same `limit`,
/// which is always enough: any row in the true top `limit` across every
/// root must also be in its own root's top `limit`, so truncating per root
/// before merging never drops a genuine match), then every root's matches
/// are merged, re-sorted by score, and truncated to `limit` again.
///
/// **Failure policy: fails closed.** The first root that fails to open or
/// query aborts the whole call with that root's [`StoreError`] — the same
/// behavior a single-root caller already gets from [`VectorStore::open`] or
/// [`VectorStore::top_k_similar`] directly, just applied per root instead of
/// once. Roots are not silently skipped on error, including
/// [`StoreError::MissingParentDir`]: a caller that wants a not-yet-used
/// root (no artifacts ingested there yet) to be silently treated as empty
/// gets that automatically and for free, with **no special-casing needed
/// here** — [`VectorStore::open`] already creates an empty, schema-ready
/// database file the first time it opens a path whose *parent* directory
/// exists but whose database file does not yet exist (see its own doc
/// comment). `MissingParentDir` only fires when the parent directory itself
/// is missing, which signals a genuine misconfiguration (e.g. a central
/// root's expected directory was never created at all) that a caller is
/// better served seeing than having silently ignored.
///
/// # Errors
///
/// Returns the first failure encountered opening or querying any root, in
/// the order `roots` was given: an open failure ([`StoreError::Open`]/
/// [`StoreError::MissingParentDir`]) is returned as-is; a query failure is
/// wrapped in [`StoreError::RootFailure`] naming the root, since the
/// wrapped variant (e.g. [`StoreError::Query`]) carries no path of its own.
///
/// # Examples
///
/// ```rust
/// use mif_store::{VectorStore, multi_root_top_k_similar};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dir_a = tempfile::tempdir()?;
/// let dir_b = tempfile::tempdir()?;
/// let root_a = dir_a.path().join("vectors.db");
/// let root_b = dir_b.path().join("vectors.db");
///
/// VectorStore::open(&root_a)?.upsert("a-doc", &[1.0, 0.0], "h", "t")?;
/// VectorStore::open(&root_b)?.upsert("b-doc", &[0.0, 1.0], "h", "t")?;
///
/// let matches = multi_root_top_k_similar(&[root_a.clone(), root_b.clone()], &[1.0, 0.0], 10)?;
/// assert_eq!(matches[0].id, "a-doc");
/// assert_eq!(matches[0].root, root_a);
/// # Ok(())
/// # }
/// ```
pub fn multi_root_top_k_similar(
    roots: &[PathBuf],
    query: &[f32],
    limit: usize,
) -> Result<Vec<RootedMatch>, StoreError> {
    let mut matches = Vec::new();
    for root in roots {
        let store = VectorStore::open(root)?;
        matches.extend(
            wrap_root_error(root, store.top_k_similar(query, limit))?
                .into_iter()
                .map(|m| RootedMatch {
                    id: m.id,
                    score: m.score,
                    root: root.clone(),
                }),
        );
    }
    matches.sort_by(|a, b| b.score.total_cmp(&a.score));
    matches.truncate(limit);
    Ok(matches)
}

/// Runs [`VectorStore::search`] over multiple [`VectorStore`] roots,
/// returning the top `limit` documents across all of them in descending
/// score order.
///
/// Each root is asked for its own top `limit`, which is always enough: a
/// root returns one row per document, so any document in the true top
/// `limit` across every root is also in its own root's top `limit`. A
/// document stored in two roots appears once per root, each with its root.
/// Follows the same fail-closed policy as [`multi_root_top_k_similar`].
///
/// # Errors
///
/// Returns the first failure encountered opening or querying any root, as
/// [`multi_root_top_k_similar`] does.
///
/// # Examples
///
/// ```rust
/// use mif_store::{SearchFilter, VectorStore, multi_root_search};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dir_a = tempfile::tempdir()?;
/// let dir_b = tempfile::tempdir()?;
/// let root_a = dir_a.path().join("vectors.db");
/// let root_b = dir_b.path().join("vectors.db");
///
/// VectorStore::open(&root_a)?.upsert("a-doc", &[1.0, 0.0], "h", "t")?;
/// VectorStore::open(&root_b)?.upsert("b-doc", &[0.0, 1.0], "h", "t")?;
///
/// let roots = [root_a, root_b.clone()];
/// let matches = multi_root_search(&roots, &[0.0, 1.0], 10, &SearchFilter::default())?;
/// assert_eq!(matches[0].id, "b-doc");
/// assert_eq!(matches[0].root, root_b);
/// # Ok(())
/// # }
/// ```
pub fn multi_root_search(
    roots: &[PathBuf],
    query: &[f32],
    limit: usize,
    filter: &SearchFilter,
) -> Result<Vec<RootedSearchMatch>, StoreError> {
    let mut matches = Vec::new();
    for root in roots {
        let store = VectorStore::open(root)?;
        matches.extend(
            wrap_root_error(root, store.search(query, limit, filter))?
                .into_iter()
                .map(|m| RootedSearchMatch {
                    id: m.id,
                    score: m.score,
                    heading: m.heading,
                    root: root.clone(),
                }),
        );
    }
    matches.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    matches.truncate(limit);
    Ok(matches)
}

/// Looks up the embedding stored for `id` across multiple [`VectorStore`]
/// roots, returning the first root (in `roots` order) that has it, paired
/// with which root it came from.
///
/// Follows the same fail-closed policy as [`multi_root_top_k_similar`]: a
/// root that fails to open or query aborts the call, and a not-yet-used
/// root (missing database file but present parent directory) is opened as
/// empty rather than erroring, with no special-casing needed here either.
///
/// # Errors
///
/// Returns the first failure encountered opening or querying any root, in
/// the order `roots` was given: an open failure ([`StoreError::Open`]/
/// [`StoreError::MissingParentDir`]) is returned as-is; a query failure is
/// wrapped in [`StoreError::RootFailure`] naming the root, since the
/// wrapped variant (e.g. [`StoreError::Query`]) carries no path of its own.
///
/// # Examples
///
/// ```rust
/// use mif_store::{VectorStore, multi_root_get};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dir_a = tempfile::tempdir()?;
/// let dir_b = tempfile::tempdir()?;
/// let root_a = dir_a.path().join("vectors.db");
/// let root_b = dir_b.path().join("vectors.db");
///
/// VectorStore::open(&root_b)?.upsert("only-in-b", &[1.0, 0.0], "h", "t")?;
///
/// let found = multi_root_get(&[root_a, root_b.clone()], "only-in-b")?;
/// let (root, stored) = found.expect("row exists in root_b");
/// assert_eq!(root, root_b);
/// assert_eq!(stored.vector, vec![1.0, 0.0]);
/// # Ok(())
/// # }
/// ```
pub fn multi_root_get(
    roots: &[PathBuf],
    id: &str,
) -> Result<Option<(PathBuf, StoredVector)>, StoreError> {
    for root in roots {
        let store = VectorStore::open(root)?;
        if let Some(stored) = wrap_root_error(root, store.get(id))? {
            return Ok(Some((root.clone(), stored)));
        }
    }
    Ok(None)
}

/// Summary statistics over multiple [`VectorStore`] roots.
///
/// Follows the same fail-closed policy as [`multi_root_top_k_similar`].
///
/// # Errors
///
/// Returns the first failure encountered opening or querying any root, in
/// the order `roots` was given: an open failure ([`StoreError::Open`]/
/// [`StoreError::MissingParentDir`]) is returned as-is; a query failure is
/// wrapped in [`StoreError::RootFailure`] naming the root, since the
/// wrapped variant (e.g. [`StoreError::Query`]) carries no path of its own.
///
/// # Examples
///
/// ```rust
/// use mif_store::{VectorStore, multi_root_stats};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let dir_a = tempfile::tempdir()?;
/// let dir_b = tempfile::tempdir()?;
/// let root_a = dir_a.path().join("vectors.db");
/// let root_b = dir_b.path().join("vectors.db");
///
/// VectorStore::open(&root_a)?.upsert("a-doc", &[1.0, 0.0], "h", "t")?;
/// VectorStore::open(&root_b)?.upsert("b-doc", &[1.0, 0.0, 0.0], "h", "t")?;
///
/// let stats = multi_root_stats(&[root_a, root_b])?;
/// assert_eq!(stats.total_count, 2);
/// assert_eq!(stats.per_root.len(), 2);
/// # Ok(())
/// # }
/// ```
pub fn multi_root_stats(roots: &[PathBuf]) -> Result<MultiRootStats, StoreError> {
    let mut per_root = Vec::with_capacity(roots.len());
    let mut total_count: u64 = 0;
    for root in roots {
        let store = VectorStore::open(root)?;
        let stats = wrap_root_error(root, store.stats())?;
        total_count = total_count.saturating_add(stats.count);
        per_root.push((root.clone(), stats));
    }
    Ok(MultiRootStats {
        per_root,
        total_count,
    })
}

/// Wraps a [`VectorStore`] query's error (e.g. [`StoreError::Query`], which
/// carries no path of its own) in [`StoreError::RootFailure`] naming `root`,
/// so a multi-root query failure is as diagnosable as a single-root one.
/// `VectorStore::open`'s own errors ([`StoreError::Open`]/
/// [`StoreError::MissingParentDir`]) already carry their path and are left
/// unwrapped — this only covers the class of error that doesn't.
fn wrap_root_error<T>(root: &Path, result: Result<T, StoreError>) -> Result<T, StoreError> {
    result.map_err(|source| StoreError::RootFailure {
        root: root.display().to_string(),
        source: Box::new(source),
    })
}

/// Brings a database's schema up to [`SCHEMA_VERSION`].
fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    if schema_version(conn)? >= SCHEMA_VERSION {
        return Ok(());
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Another process may have upgraded the database between the check
    // above and this transaction taking the write lock.
    if schema_version(&tx)? < SCHEMA_VERSION {
        tx.execute_batch(MIGRATE_V1_SQL)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    tx.commit()
}

/// The schema version recorded in the database.
fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

/// Wraps a `SQLite` failure as [`StoreError::Query`].
const fn query_error(source: rusqlite::Error) -> StoreError {
    StoreError::Query { source }
}

/// Deletes a document's row and passages, reporting whether either existed.
fn delete_document(conn: &Connection, id: &str) -> Result<bool, StoreError> {
    let passages = conn
        .execute("DELETE FROM passages WHERE doc_id = ?1", [id])
        .map_err(query_error)?;
    let documents = conn
        .execute("DELETE FROM embeddings WHERE id = ?1", [id])
        .map_err(query_error)?;
    Ok(passages + documents > 0)
}

/// A vector's length as the `dim` column stores it.
fn dim_of(vector: &[f32]) -> Result<i64, StoreError> {
    i64::try_from(vector.len()).map_err(|_| StoreError::DimensionTooLarge { len: vector.len() })
}

/// Decodes a stored vector, checking it against its `dim` column.
fn checked_vector(id: &str, dim_raw: i64, blob: &[u8]) -> Result<Vec<f32>, StoreError> {
    let dim =
        usize::try_from(dim_raw).map_err(|_| StoreError::CorruptDimension { value: dim_raw })?;
    let vector = decode_vector(blob);
    if vector.len() != dim {
        return Err(StoreError::VectorBlobMismatch {
            id: id.to_string(),
            expected_dim: dim,
            actual_len: vector.len(),
        });
    }
    Ok(vector)
}

/// The Euclidean norm of `vector`.
fn norm(vector: &[f32]) -> f32 {
    vector
        .iter()
        .map(|component| component * component)
        .sum::<f32>()
        .sqrt()
}

/// Cosine similarity between `a` and `b`, given `a`'s precomputed norm.
/// Returns `0.0` if either vector is the zero vector (cosine similarity is
/// undefined there; treating it as "no similarity" is safer than dividing
/// by zero).
fn cosine_similarity(a: &[f32], b: &[f32], a_norm: f32) -> f32 {
    let b_norm = norm(b);
    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    dot / (a_norm * b_norm)
}

/// Encodes `vector` as a little-endian `f32` blob (see the module
/// documentation for the exact layout).
fn encode_vector(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|component| component.to_le_bytes())
        .collect()
}

/// Decodes a little-endian `f32` blob produced by [`encode_vector`].
///
/// Any trailing bytes that do not form a complete 4-byte component are
/// dropped, since this crate never writes such a blob itself.
fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    let (chunks, _remainder) = bytes.as_chunks::<4>();
    chunks
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

#[cfg(test)]
mod tests {
    use mif_problem::ToProblem;

    use super::*;

    fn open_temp_store() -> (tempfile::TempDir, VectorStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(&dir.path().join("vectors.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn open_creates_the_schema_on_a_fresh_path() {
        let (_dir, store) = open_temp_store();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn upsert_then_get_round_trips_a_vector_exactly() {
        let (_dir, store) = open_temp_store();
        let vector = vec![1.0_f32, -2.5, f32::MIN_POSITIVE, 0.1, 12_345.679];

        store
            .upsert("doc-1", &vector, "hash-1", "2026-07-02T00:00:00Z")
            .unwrap();

        let stored = store.get("doc-1").unwrap().unwrap();
        assert_eq!(stored.dim, vector.len());
        assert_eq!(stored.vector, vector);
        assert_eq!(stored.content_hash, "hash-1");
        assert_eq!(stored.updated_at, "2026-07-02T00:00:00Z");
    }

    #[test]
    fn upsert_twice_with_the_same_id_updates_rather_than_duplicates() {
        let (_dir, store) = open_temp_store();

        store
            .upsert("doc-1", &[1.0, 2.0], "hash-1", "2026-07-02T00:00:00Z")
            .unwrap();
        store
            .upsert("doc-1", &[3.0, 4.0, 5.0], "hash-2", "2026-07-02T00:01:00Z")
            .unwrap();

        assert_eq!(store.count().unwrap(), 1);
        let stored = store.get("doc-1").unwrap().unwrap();
        assert_eq!(stored.vector, vec![3.0, 4.0, 5.0]);
        assert_eq!(stored.content_hash, "hash-2");
        assert_eq!(stored.updated_at, "2026-07-02T00:01:00Z");
    }

    #[test]
    fn get_on_a_missing_id_returns_none() {
        let (_dir, store) = open_temp_store();
        assert_eq!(store.get("missing").unwrap(), None);
    }

    #[test]
    fn count_on_an_empty_store_returns_zero() {
        let (_dir, store) = open_temp_store();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn get_reports_vector_blob_mismatch_instead_of_a_silently_truncated_vector() {
        let (_dir, store) = open_temp_store();

        // Simulate external corruption `upsert` itself can never produce:
        // the `dim` column claims 4 components (16 bytes), but the blob is
        // 13 bytes — not a multiple of 4, and short of the claimed length.
        // `decode_vector`'s `chunks_exact(4)` silently drops the trailing
        // partial component, decoding only 3 floats; `get()` must reject
        // this rather than return a silently wrong-length vector.
        let mut malformed_blob = encode_vector(&[1.0_f32, 2.0, 3.0]);
        malformed_blob.push(0xFF);
        store
            .conn
            .execute(
                "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "corrupt-1",
                    4_i64,
                    malformed_blob,
                    "hash-1",
                    "2026-07-02T00:00:00Z"
                ],
            )
            .unwrap();

        let error = store.get("corrupt-1").unwrap_err();
        assert!(matches!(
            error,
            StoreError::VectorBlobMismatch { ref id, expected_dim: 4, actual_len: 3 }
                if id == "corrupt-1"
        ));
    }

    #[test]
    fn top_k_similar_ranks_the_closest_vector_first() {
        let (_dir, store) = open_temp_store();
        store.upsert("same", &[1.0, 0.0], "h", "t").unwrap();
        store.upsert("orthogonal", &[0.0, 1.0], "h", "t").unwrap();
        store.upsert("opposite", &[-1.0, 0.0], "h", "t").unwrap();

        let matches = store.top_k_similar(&[1.0, 0.0], 10).unwrap();
        let ids: Vec<&str> = matches.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["same", "orthogonal", "opposite"]);
        assert!((matches[0].score - 1.0).abs() < 1e-6);
        assert!((matches[1].score - 0.0).abs() < 1e-6);
        assert!((matches[2].score - (-1.0)).abs() < 1e-6);
    }

    #[test]
    fn top_k_similar_respects_the_limit() {
        let (_dir, store) = open_temp_store();
        for i in 0_u8..5 {
            store
                .upsert(&format!("doc-{i}"), &[1.0, f32::from(i)], "h", "t")
                .unwrap();
        }
        let matches = store.top_k_similar(&[1.0, 0.0], 2).unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn top_k_similar_skips_rows_with_a_different_dimensionality() {
        let (_dir, store) = open_temp_store();
        store.upsert("two-dim", &[1.0, 0.0], "h", "t").unwrap();
        store
            .upsert("three-dim", &[1.0, 0.0, 0.0], "h", "t")
            .unwrap();

        let matches = store.top_k_similar(&[1.0, 0.0], 10).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "two-dim");
    }

    #[test]
    fn stats_reports_count_and_a_representative_dim() {
        let (_dir, store) = open_temp_store();
        assert_eq!(
            store.stats().unwrap(),
            CorpusStats {
                count: 0,
                dim: None
            }
        );

        store.upsert("doc-1", &[1.0, 2.0, 3.0], "h", "t").unwrap();
        store.upsert("doc-2", &[4.0, 5.0, 6.0], "h", "t").unwrap();
        let stats = store.stats().unwrap();
        assert_eq!(stats.count, 2);
        assert_eq!(stats.dim, Some(3));
    }

    #[test]
    fn missing_parent_dir_and_dimension_too_large_map_to_distinct_problem_types() {
        let missing = StoreError::MissingParentDir {
            path: "/nonexistent".to_string(),
        }
        .to_problem();
        assert_eq!(
            missing.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/missing-parent-dir/v1"
        );
        assert_eq!(missing.status, 400);
        assert_eq!(
            missing.suggested_fix.as_ref().unwrap().applicability,
            mif_problem::Applicability::MachineApplicable
        );

        let too_large = StoreError::DimensionTooLarge { len: usize::MAX }.to_problem();
        assert_eq!(
            too_large.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/dimension-too-large/v1"
        );
        assert_ne!(missing.problem_type, too_large.problem_type);
    }

    #[test]
    fn open_returns_missing_parent_dir_when_the_parent_directory_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-such-subdir").join("vectors.db");

        let error = VectorStore::open(&path).unwrap_err();
        assert!(matches!(
            error,
            StoreError::MissingParentDir { ref path } if path.ends_with("no-such-subdir")
        ));
    }

    #[test]
    fn open_reports_an_open_failure_when_the_path_is_an_existing_directory() {
        let dir = tempfile::tempdir().unwrap();
        // dir.path() itself exists and is a directory (its own parent exists,
        // so this reaches Connection::open rather than the MissingParentDir
        // check); SQLite cannot open a directory as a database file.
        let error = VectorStore::open(dir.path()).unwrap_err();
        assert!(matches!(error, StoreError::Open { .. }));
    }

    #[test]
    fn open_reports_an_open_failure_when_schema_initialization_fails_on_a_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.db");
        // A non-empty file that is not a valid SQLite database. `Connection::open`
        // succeeds (SQLite defers header validation), so this exercises the
        // schema-initialization error path rather than the open path.
        std::fs::write(
            &path,
            b"not a valid sqlite database file, just garbage bytes",
        )
        .unwrap();

        let error = VectorStore::open(&path).unwrap_err();
        assert!(matches!(error, StoreError::Open { .. }));
    }

    #[test]
    fn top_k_similar_reports_vector_blob_mismatch_instead_of_silently_skipping_corrupt_rows() {
        let (_dir, store) = open_temp_store();

        let mut malformed_blob = encode_vector(&[1.0_f32, 2.0, 3.0]);
        malformed_blob.push(0xFF);
        store
            .conn
            .execute(
                "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "corrupt-1",
                    4_i64,
                    malformed_blob,
                    "hash-1",
                    "2026-07-02T00:00:00Z"
                ],
            )
            .unwrap();

        let error = store.top_k_similar(&[1.0, 2.0, 3.0, 4.0], 10).unwrap_err();
        assert!(matches!(
            error,
            StoreError::VectorBlobMismatch { ref id, expected_dim: 4, actual_len: 3 }
                if id == "corrupt-1"
        ));
    }

    #[test]
    fn top_k_similar_scores_a_zero_vector_row_as_zero_similarity() {
        let (_dir, store) = open_temp_store();
        store.upsert("zero", &[0.0, 0.0], "h", "t").unwrap();
        store.upsert("nonzero", &[1.0, 0.0], "h", "t").unwrap();

        let matches = store.top_k_similar(&[1.0, 0.0], 10).unwrap();
        let zero_match = matches.iter().find(|m| m.id == "zero").unwrap();
        assert!((zero_match.score - 0.0).abs() < 1e-6);
    }

    #[test]
    fn open_query_corrupt_dimension_and_vector_blob_mismatch_share_the_same_problem_fix() {
        let bogus_error = || {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.execute("SELECT * FROM no_such_table_at_all", [])
                .unwrap_err()
        };

        let open_problem = StoreError::Open {
            path: "/does/not/matter.db".to_string(),
            source: bogus_error(),
        }
        .to_problem();
        let query_problem = StoreError::Query {
            source: bogus_error(),
        }
        .to_problem();
        let corrupt_problem = StoreError::CorruptDimension { value: -1 }.to_problem();
        let mismatch_problem = StoreError::VectorBlobMismatch {
            id: "doc-1".to_string(),
            expected_dim: 4,
            actual_len: 3,
        }
        .to_problem();

        assert_eq!(
            open_problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/open-database-failure/v1"
        );
        assert_eq!(open_problem.status, 500);
        assert_eq!(
            query_problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/sqlite-query-failure/v1"
        );
        assert_eq!(query_problem.status, 500);
        assert_eq!(
            corrupt_problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/corrupt-dimension/v1"
        );
        assert_eq!(
            mismatch_problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/vector-blob-mismatch/v1"
        );

        for problem in [
            &open_problem,
            &query_problem,
            &corrupt_problem,
            &mismatch_problem,
        ] {
            assert_eq!(
                problem.suggested_fix.as_ref().unwrap().applicability,
                mif_problem::Applicability::Unspecified
            );
        }
    }

    fn open_temp_root() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.db");
        VectorStore::open(&path).unwrap();
        (dir, path)
    }

    #[test]
    fn multi_root_top_k_similar_merges_and_ranks_across_roots() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        VectorStore::open(&root_a)
            .unwrap()
            .upsert("same", &[1.0, 0.0], "h", "t")
            .unwrap();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("orthogonal", &[0.0, 1.0], "h", "t")
            .unwrap();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("opposite", &[-1.0, 0.0], "h", "t")
            .unwrap();

        let matches =
            multi_root_top_k_similar(&[root_a.clone(), root_b.clone()], &[1.0, 0.0], 10).unwrap();
        let ids: Vec<&str> = matches.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["same", "orthogonal", "opposite"]);
        assert_eq!(matches[0].root, root_a);
        assert_eq!(matches[1].root, root_b);
        assert_eq!(matches[2].root, root_b);
    }

    #[test]
    fn multi_root_top_k_similar_respects_the_limit_across_roots() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        for i in 0_u8..3 {
            VectorStore::open(&root_a)
                .unwrap()
                .upsert(&format!("a-{i}"), &[1.0, f32::from(i)], "h", "t")
                .unwrap();
            VectorStore::open(&root_b)
                .unwrap()
                .upsert(&format!("b-{i}"), &[1.0, f32::from(i) + 0.5], "h", "t")
                .unwrap();
        }

        let matches = multi_root_top_k_similar(&[root_a, root_b], &[1.0, 0.0], 2).unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn multi_root_top_k_similar_keeps_colliding_ids_from_different_roots_distinct() {
        // Two roots can each independently contain a row with the same id
        // (e.g. the same document copied into both a project-local and a
        // central store). Both must survive the merge, distinguishable only
        // by `root`.
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        VectorStore::open(&root_a)
            .unwrap()
            .upsert("shared-id", &[1.0, 0.0], "h", "t")
            .unwrap();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("shared-id", &[1.0, 0.0], "h", "t")
            .unwrap();

        let matches =
            multi_root_top_k_similar(&[root_a.clone(), root_b.clone()], &[1.0, 0.0], 10).unwrap();
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|m| m.id == "shared-id"));
        let roots: std::collections::HashSet<_> = matches.iter().map(|m| m.root.clone()).collect();
        assert_eq!(roots, std::collections::HashSet::from([root_a, root_b]));
    }

    #[test]
    fn multi_root_top_k_similar_treats_a_not_yet_created_root_as_empty() {
        // The second root's parent directory exists (a fresh tempdir), but
        // its database file has never been opened. VectorStore::open
        // creates it on demand, so this must succeed with only root_a's
        // rows, not error.
        let (_dir_a, root_a) = open_temp_root();
        VectorStore::open(&root_a)
            .unwrap()
            .upsert("only-doc", &[1.0, 0.0], "h", "t")
            .unwrap();
        let not_yet_created_dir = tempfile::tempdir().unwrap();
        let not_yet_created_root = not_yet_created_dir.path().join("vectors.db");

        let matches =
            multi_root_top_k_similar(&[root_a, not_yet_created_root], &[1.0, 0.0], 10).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "only-doc");
    }

    #[test]
    fn multi_root_top_k_similar_fails_closed_when_a_root_cannot_be_opened() {
        let (_dir_a, root_a) = open_temp_root();
        let missing_parent_dir = tempfile::tempdir().unwrap();
        let bad_root = missing_parent_dir.path().join("no-such-subdir/vectors.db");

        let error = multi_root_top_k_similar(&[root_a, bad_root], &[1.0, 0.0], 10).unwrap_err();
        assert!(matches!(error, StoreError::MissingParentDir { .. }));
    }

    #[test]
    fn wrap_root_error_attaches_the_root_to_a_query_class_failure_that_has_no_path_of_its_own() {
        // Unlike StoreError::Open/MissingParentDir, StoreError::Query carries
        // no path field at all -- this is exactly the gap multi_root_* fixes
        // by wrapping a query failure in RootFailure before returning it, so
        // a caller querying several roots can still tell which one broke.
        let bogus_source = {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.execute("SELECT * FROM no_such_table_at_all", [])
                .unwrap_err()
        };
        let root = std::path::Path::new("/some/second/root/vectors.db");

        let wrapped = wrap_root_error(
            root,
            Err::<(), _>(StoreError::Query {
                source: bogus_source,
            }),
        )
        .unwrap_err();

        let StoreError::RootFailure {
            root: got_root,
            source,
        } = wrapped
        else {
            unreachable!("wrap_root_error always wraps in StoreError::RootFailure");
        };
        assert_eq!(got_root, root.display().to_string());
        assert!(matches!(*source, StoreError::Query { .. }));
    }

    #[test]
    fn wrap_root_error_passes_success_through_unchanged() {
        let root = std::path::Path::new("/irrelevant.db");
        assert_eq!(wrap_root_error(root, Ok::<_, StoreError>(42)).unwrap(), 42);
    }

    #[test]
    fn multi_root_get_finds_the_row_in_whichever_root_has_it() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("only-in-b", &[1.0, 0.0, 0.0], "h", "t")
            .unwrap();

        let found = multi_root_get(&[root_a, root_b.clone()], "only-in-b")
            .unwrap()
            .unwrap();
        assert_eq!(found.0, root_b);
        assert_eq!(found.1.vector, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn multi_root_get_returns_none_when_absent_from_every_root() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();

        assert_eq!(multi_root_get(&[root_a, root_b], "missing").unwrap(), None);
    }

    #[test]
    fn multi_root_get_returns_the_first_matching_root_in_order_on_a_collision() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        VectorStore::open(&root_a)
            .unwrap()
            .upsert("shared-id", &[9.0], "h", "first")
            .unwrap();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("shared-id", &[9.0], "h", "second")
            .unwrap();

        let found = multi_root_get(&[root_a.clone(), root_b], "shared-id")
            .unwrap()
            .unwrap();
        assert_eq!(found.0, root_a);
        assert_eq!(found.1.updated_at, "first");
    }

    #[test]
    fn multi_root_stats_sums_counts_and_preserves_per_root_breakdown() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        VectorStore::open(&root_a)
            .unwrap()
            .upsert("a-1", &[1.0, 0.0], "h", "t")
            .unwrap();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("b-1", &[1.0, 0.0, 0.0], "h", "t")
            .unwrap();
        VectorStore::open(&root_b)
            .unwrap()
            .upsert("b-2", &[1.0, 0.0, 0.0], "h", "t")
            .unwrap();

        let stats = multi_root_stats(&[root_a.clone(), root_b.clone()]).unwrap();
        assert_eq!(stats.total_count, 3);
        assert_eq!(
            stats.per_root,
            vec![
                (
                    root_a,
                    CorpusStats {
                        count: 1,
                        dim: Some(2)
                    }
                ),
                (
                    root_b,
                    CorpusStats {
                        count: 2,
                        dim: Some(3)
                    }
                ),
            ]
        );
    }

    #[test]
    fn multi_root_stats_treats_a_not_yet_created_root_as_zero_count() {
        let (_dir_a, root_a) = open_temp_root();
        let not_yet_created_dir = tempfile::tempdir().unwrap();
        let not_yet_created_root = not_yet_created_dir.path().join("vectors.db");

        let stats = multi_root_stats(&[root_a, not_yet_created_root]).unwrap();
        assert_eq!(stats.total_count, 0);
        assert_eq!(
            stats.per_root[1].1,
            CorpusStats {
                count: 0,
                dim: None
            }
        );
    }

    fn document<'a>(
        id: &'a str,
        vector: &'a [f32],
        passages: &'a [PassageVector<'a>],
    ) -> DocumentVectors<'a> {
        DocumentVectors {
            id,
            vector,
            passages,
            content_hash: "h",
            updated_at: "t",
            source: None,
            namespace: None,
        }
    }

    fn passage<'a>(heading: Option<&'a str>, vector: &'a [f32]) -> PassageVector<'a> {
        PassageVector { heading, vector }
    }

    fn passage_rows(store: &VectorStore, id: &str) -> Vec<(i64, Option<String>)> {
        let mut stmt = store
            .conn
            .prepare("SELECT ord, heading FROM passages WHERE doc_id = ?1 ORDER BY ord")
            .unwrap();
        stmt.query_map([id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// A database as release 0.11.0 wrote it: the one table, no version.
    fn write_v0_database(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute(SCHEMA_SQL, []).unwrap();
        conn.execute(
            "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at)
             VALUES ('old-doc', 2, ?1, 'old-hash', 'old-time')",
            [encode_vector(&[1.0, 0.0])],
        )
        .unwrap();
    }

    #[test]
    fn open_upgrades_a_version_0_database_and_keeps_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.db");
        write_v0_database(&path);

        let store = VectorStore::open(&path).unwrap();

        assert_eq!(schema_version(&store.conn).unwrap(), SCHEMA_VERSION);
        let stored = store.get("old-doc").unwrap().unwrap();
        assert_eq!(stored.vector, vec![1.0, 0.0]);
        assert_eq!(stored.content_hash, "old-hash");
        assert_eq!(
            store.passage_stats().unwrap(),
            PassageStats {
                passage_count: 0,
                documents_without_passages: 1
            }
        );
        // The document is still found, ranked on its document vector.
        let hits = store
            .search(&[1.0, 0.0], 10, &SearchFilter::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "old-doc");
        assert_eq!(hits[0].heading, None);
        assert!((hits[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn open_is_idempotent_on_an_upgraded_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.db");
        write_v0_database(&path);

        VectorStore::open(&path)
            .unwrap()
            .upsert_document(&document(
                "new-doc",
                &[0.0, 1.0],
                &[passage(Some("Scope"), &[0.0, 1.0])],
            ))
            .unwrap();
        let store = VectorStore::open(&path).unwrap();

        assert_eq!(store.count().unwrap(), 2);
        assert_eq!(store.passage_stats().unwrap().passage_count, 1);
    }

    #[test]
    fn an_upgraded_database_still_serves_the_version_0_statements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vectors.db");
        write_v0_database(&path);
        drop(VectorStore::open(&path).unwrap());

        // What release 0.11.0 runs against the same file.
        let conn = Connection::open(&path).unwrap();
        conn.execute(SCHEMA_SQL, []).unwrap();
        conn.execute(
            "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at)
             VALUES ('written-by-old', 2, ?1, 'h', 't')
             ON CONFLICT(id) DO UPDATE SET vector = excluded.vector",
            [encode_vector(&[0.0, 1.0])],
        )
        .unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM (SELECT id, dim, vector FROM embeddings)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn upsert_document_replaces_every_passage_of_the_document() {
        let (_dir, store) = open_temp_store();
        store
            .upsert_document(&document(
                "doc",
                &[1.0, 0.0],
                &[
                    passage(None, &[1.0, 0.0]),
                    passage(Some("A"), &[0.0, 1.0]),
                    passage(Some("B"), &[0.5, 0.5]),
                ],
            ))
            .unwrap();
        store
            .upsert_document(&document(
                "other",
                &[1.0, 0.0],
                &[passage(Some("Kept"), &[1.0, 0.0])],
            ))
            .unwrap();

        store
            .upsert_document(&DocumentVectors {
                content_hash: "h2",
                source: Some("records/doc.md"),
                namespace: Some("records"),
                ..document("doc", &[0.0, 1.0], &[passage(Some("Only"), &[0.0, 1.0])])
            })
            .unwrap();

        assert_eq!(
            passage_rows(&store, "doc"),
            vec![(0, Some("Only".to_string()))]
        );
        assert_eq!(
            passage_rows(&store, "other"),
            vec![(0, Some("Kept".to_string()))]
        );
        let stored = store.get("doc").unwrap().unwrap();
        assert_eq!(stored.vector, vec![0.0, 1.0]);
        assert_eq!(stored.content_hash, "h2");
        assert_eq!(store.count().unwrap(), 2);
    }

    #[test]
    fn upsert_removes_the_passages_of_the_document_it_replaces() {
        let (_dir, store) = open_temp_store();
        store
            .upsert_document(&document(
                "doc",
                &[1.0, 0.0],
                &[passage(Some("Stale"), &[0.0, 1.0])],
            ))
            .unwrap();

        store.upsert("doc", &[1.0, 0.0], "h2", "t2").unwrap();

        assert!(passage_rows(&store, "doc").is_empty());
        let hits = store
            .search(&[0.0, 1.0], 10, &SearchFilter::default())
            .unwrap();
        assert_eq!(hits[0].heading, None);
        assert!(hits[0].score.abs() < 1e-6);
    }

    #[test]
    fn search_ranks_each_document_once_by_its_best_passage() {
        let (_dir, store) = open_temp_store();
        // "deep" is found only through a late passage; its document vector
        // points away from the query.
        store
            .upsert_document(&document(
                "deep",
                &[1.0, 0.0],
                &[
                    passage(None, &[1.0, 0.0]),
                    passage(Some("Boilerplate"), &[0.9, 0.1]),
                    passage(Some("Definition"), &[0.0, 1.0]),
                ],
            ))
            .unwrap();
        store
            .upsert_document(&document(
                "near",
                &[0.6, 0.8],
                &[passage(None, &[0.6, 0.8])],
            ))
            .unwrap();
        store.upsert("legacy", &[0.8, 0.6], "h", "t").unwrap();

        let hits = store
            .search(&[0.0, 1.0], 10, &SearchFilter::default())
            .unwrap();

        let ranked: Vec<_> = hits
            .iter()
            .map(|hit| (hit.id.as_str(), hit.heading.as_deref()))
            .collect();
        assert_eq!(
            ranked,
            vec![
                ("deep", Some("Definition")),
                ("near", None),
                ("legacy", None)
            ]
        );
        assert!((hits[0].score - 1.0).abs() < 1e-6);
        // Document-to-document ranking is untouched: it uses document vectors.
        let by_document = store.top_k_similar(&[0.0, 1.0], 10).unwrap();
        assert_eq!(by_document[0].id, "near");
        assert_eq!(by_document.len(), 3);
    }

    #[test]
    fn search_respects_the_limit_and_breaks_ties_by_id() {
        let (_dir, store) = open_temp_store();
        for id in ["c", "a", "b"] {
            store
                .upsert_document(&document(id, &[1.0, 0.0], &[passage(None, &[1.0, 0.0])]))
                .unwrap();
        }

        let hits = store
            .search(&[1.0, 0.0], 2, &SearchFilter::default())
            .unwrap();

        let ids: Vec<_> = hits.iter().map(|hit| hit.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn search_skips_vectors_of_another_dimensionality_and_reports_corrupt_ones() {
        let (_dir, store) = open_temp_store();
        store
            .upsert_document(&document(
                "mixed",
                &[1.0, 0.0, 0.0],
                &[
                    passage(None, &[1.0, 0.0, 0.0]),
                    passage(Some("Two"), &[0.0, 1.0]),
                ],
            ))
            .unwrap();
        store
            .upsert_document(&document(
                "three-only",
                &[1.0, 0.0, 0.0],
                &[passage(None, &[1.0, 0.0, 0.0])],
            ))
            .unwrap();

        let hits = store
            .search(&[0.0, 1.0], 10, &SearchFilter::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "mixed");
        assert_eq!(hits[0].heading.as_deref(), Some("Two"));

        store
            .conn
            .execute(
                "UPDATE passages SET dim = 5 WHERE doc_id = 'mixed' AND ord = 1",
                [],
            )
            .unwrap();
        let error = store
            .search(&[0.0, 1.0], 10, &SearchFilter::default())
            .unwrap_err();
        assert!(matches!(error, StoreError::VectorBlobMismatch { .. }));
    }

    #[test]
    fn search_filters_by_id_source_and_namespace_prefix() {
        let (_dir, store) = open_temp_store();
        let vector = [1.0_f32, 0.0];
        let passages = [passage(None, &vector)];
        for (id, source, namespace) in [
            (
                "urn:mif:cap",
                Some("07-Capability-Model/C04.md"),
                Some("capabilities/level-2"),
            ),
            (
                "urn:mif:con",
                Some("08-Constraint-Records/CON-1.md"),
                Some("constraints"),
            ),
            ("urn:mif:note", Some("Notes/Meeting.md"), None),
        ] {
            store
                .upsert_document(&DocumentVectors {
                    source,
                    namespace,
                    ..document(id, &vector, &passages)
                })
                .unwrap();
        }
        store.upsert("urn:mif:legacy", &vector, "h", "t").unwrap();
        let ids = |filter: SearchFilter| -> Vec<String> {
            store
                .search(&vector, 10, &filter)
                .unwrap()
                .into_iter()
                .map(|hit| hit.id)
                .collect()
        };

        assert_eq!(ids(SearchFilter::default()).len(), 4);
        assert_eq!(
            ids(SearchFilter {
                source_prefix: Some("07-Capability-Model/".to_string()),
                ..SearchFilter::default()
            }),
            vec!["urn:mif:cap"]
        );
        assert_eq!(
            ids(SearchFilter {
                namespace_prefix: Some("c".to_string()),
                ..SearchFilter::default()
            }),
            vec!["urn:mif:cap", "urn:mif:con"]
        );
        assert_eq!(
            ids(SearchFilter {
                id_prefix: Some("urn:mif:n".to_string()),
                ..SearchFilter::default()
            }),
            vec!["urn:mif:note"]
        );
        assert!(
            ids(SearchFilter {
                id_prefix: Some("urn:mif:c".to_string()),
                source_prefix: Some("Notes/".to_string()),
                ..SearchFilter::default()
            })
            .is_empty()
        );
        // A prefix with LIKE wildcards in it is matched literally.
        assert!(
            ids(SearchFilter {
                source_prefix: Some("%".to_string()),
                ..SearchFilter::default()
            })
            .is_empty()
        );
    }

    #[test]
    fn remove_deletes_the_document_and_its_passages() {
        let (_dir, store) = open_temp_store();
        store
            .upsert_document(&document("doc", &[1.0, 0.0], &[passage(None, &[1.0, 0.0])]))
            .unwrap();
        store.upsert("other", &[1.0, 0.0], "h", "t").unwrap();

        assert!(store.remove("doc").unwrap());
        assert!(!store.remove("doc").unwrap());

        assert!(store.get("doc").unwrap().is_none());
        assert!(passage_rows(&store, "doc").is_empty());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn prune_removes_everything_not_kept_and_reports_it_in_order() {
        let (_dir, store) = open_temp_store();
        for id in ["keep", "stale-b", "stale-a"] {
            store
                .upsert_document(&document(id, &[1.0, 0.0], &[passage(None, &[1.0, 0.0])]))
                .unwrap();
        }
        // Passages left behind without a document row are pruned too.
        store
            .conn
            .execute(
                "INSERT INTO passages (doc_id, ord, heading, dim, vector)
                 VALUES ('orphan', 0, NULL, 2, ?1)",
                [encode_vector(&[1.0, 0.0])],
            )
            .unwrap();
        let keep: HashSet<String> = ["keep".to_string(), "never-stored".to_string()].into();

        let removed = store.prune(&keep).unwrap();

        assert_eq!(removed, vec!["orphan", "stale-a", "stale-b"]);
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.passage_stats().unwrap().passage_count, 1);
        assert!(store.prune(&keep).unwrap().is_empty());
    }

    #[test]
    fn passage_stats_counts_passages_and_documents_without_any() {
        let (_dir, store) = open_temp_store();
        store
            .upsert_document(&document(
                "with",
                &[1.0, 0.0],
                &[passage(None, &[1.0, 0.0]), passage(Some("A"), &[0.0, 1.0])],
            ))
            .unwrap();
        store
            .upsert_document(&document("empty", &[1.0, 0.0], &[]))
            .unwrap();
        store.upsert("legacy", &[1.0, 0.0], "h", "t").unwrap();

        assert_eq!(
            store.passage_stats().unwrap(),
            PassageStats {
                passage_count: 2,
                documents_without_passages: 2
            }
        );
    }

    #[test]
    fn multi_root_search_merges_roots_and_keeps_colliding_ids_distinct() {
        let (_dir_a, root_a) = open_temp_root();
        let (_dir_b, root_b) = open_temp_root();
        VectorStore::open(&root_a)
            .unwrap()
            .upsert_document(&document(
                "shared",
                &[1.0, 0.0],
                &[passage(Some("From A"), &[0.0, 1.0])],
            ))
            .unwrap();
        let store_b = VectorStore::open(&root_b).unwrap();
        store_b
            .upsert_document(&document(
                "shared",
                &[1.0, 0.0],
                &[passage(Some("From B"), &[0.6, 0.8])],
            ))
            .unwrap();
        store_b.upsert("b-only", &[1.0, 0.0], "h", "t").unwrap();
        drop(store_b);
        let roots = [root_a.clone(), root_b.clone()];

        let hits = multi_root_search(&roots, &[0.0, 1.0], 10, &SearchFilter::default()).unwrap();

        let ranked: Vec<_> = hits
            .iter()
            .map(|hit| (hit.id.as_str(), hit.heading.as_deref(), hit.root.clone()))
            .collect();
        assert_eq!(
            ranked,
            vec![
                ("shared", Some("From A"), root_a),
                ("shared", Some("From B"), root_b.clone()),
                ("b-only", None, root_b),
            ]
        );
        let limited = multi_root_search(&roots, &[0.0, 1.0], 1, &SearchFilter::default()).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].heading.as_deref(), Some("From A"));
    }

    #[test]
    fn multi_root_search_fails_closed_when_a_root_cannot_be_opened() {
        let (_dir, root) = open_temp_root();
        let missing = std::path::PathBuf::from("/nonexistent-mif-store-dir/vectors.db");

        let error =
            multi_root_search(&[root, missing], &[1.0], 10, &SearchFilter::default()).unwrap_err();
        assert!(matches!(error, StoreError::MissingParentDir { .. }));
    }
}
