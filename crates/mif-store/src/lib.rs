//! `SQLite`-backed vector store for document embeddings in the MIF (Modeled
//! Information Format) ecosystem.
//!
//! [`VectorStore`] wraps a single `SQLite` connection and a single
//! `embeddings` table (id, dim, vector, `content_hash`, `updated_at`). Vectors
//! are stored as a raw little-endian `f32` blob — see "Vector Blob Layout"
//! below for the exact on-disk format. This crate does not compute
//! embeddings or timestamps; both are supplied by the caller, keeping the
//! store itself deterministic and easy to test.
//!
//! # Vector Blob Layout
//!
//! The `vector` column holds `dim * 4` bytes: each `f32` component encoded
//! as 4 bytes, little-endian, in component order, with no header or
//! padding. This is a private, on-disk format read and written only by this
//! crate — it is not a public interchange format.

use std::path::Path;

use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};
use rusqlite::{Connection, OptionalExtension, params};

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

/// A `SQLite`-backed store of document embedding vectors.
#[derive(Debug)]
pub struct VectorStore {
    conn: Connection,
}

impl VectorStore {
    /// Opens the `SQLite` database at `path`, creating the `embeddings` table
    /// if it does not already exist. The database file itself is created by
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

        let conn = Connection::open(path).map_err(|source| StoreError::Open {
            path: path.display().to_string(),
            source,
        })?;

        conn.execute(SCHEMA_SQL, [])
            .map_err(|source| StoreError::Open {
                path: path.display().to_string(),
                source,
            })?;

        Ok(Self { conn })
    }

    /// Inserts a new embedding, or replaces the existing one for `id`.
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

        self.conn
            .execute(
                "INSERT INTO embeddings (id, dim, vector, content_hash, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    dim = excluded.dim,
                    vector = excluded.vector,
                    content_hash = excluded.content_hash,
                    updated_at = excluded.updated_at",
                params![id, dim, blob, content_hash, updated_at],
            )
            .map_err(|source| StoreError::Query { source })?;

        Ok(())
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
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
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
            "https://mif-spec.dev/errors/missing-parent-dir/v1"
        );
        assert_eq!(missing.status, 400);
        assert_eq!(
            missing.suggested_fix.as_ref().unwrap().applicability,
            mif_problem::Applicability::MachineApplicable
        );

        let too_large = StoreError::DimensionTooLarge { len: usize::MAX }.to_problem();
        assert_eq!(
            too_large.problem_type,
            "https://mif-spec.dev/errors/dimension-too-large/v1"
        );
        assert_ne!(missing.problem_type, too_large.problem_type);
    }
}
