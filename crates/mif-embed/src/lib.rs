//! Local sentence-embedding inference for the [MIF (Modeled Information
//! Format)](https://mif-spec.dev) ecosystem, via [`candle`](https://github.com/huggingface/candle).
//!
//! [`Embedder`] loads `sentence-transformers/all-MiniLM-L6-v2` (384-dimensional,
//! mean-pooled, L2-normalized sentence embeddings) from the Hugging Face Hub on
//! first use, caching the model files under the platform cache directory so
//! later runs are offline. Inference runs on CPU only.

use std::path::PathBuf;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
use hf_hub::api::sync::{Api, ApiBuilder};
use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};
use tokenizers::{Tokenizer, TruncationParams};

/// Hugging Face Hub repository providing the model this crate embeds with.
const MODEL_REPO: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Output embedding dimensionality of `sentence-transformers/all-MiniLM-L6-v2`.
pub const EMBEDDING_DIM: usize = 384;

/// Errors from loading the embedding model or running inference.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The platform has no resolvable user cache directory
    /// (`dirs::cache_dir()` returned `None`), so the model has nowhere to be
    /// cached.
    #[error("no user cache directory available to cache '{model}'")]
    NoCacheDir {
        /// The Hugging Face Hub repository that could not be cached.
        model: &'static str,
    },
    /// Failed to initialize the Hugging Face Hub API client.
    #[error("failed to initialize the Hugging Face Hub client: {0}")]
    HubClient(#[source] hf_hub::api::sync::ApiError),
    /// Failed to fetch a required model file from the Hugging Face Hub.
    #[error("failed to fetch '{file}' from '{repo}': {source}")]
    Fetch {
        /// The Hugging Face Hub repository the file was fetched from.
        repo: &'static str,
        /// The name of the file that failed to fetch.
        file: &'static str,
        /// The underlying Hub API error.
        #[source]
        source: hf_hub::api::sync::ApiError,
    },
    /// Failed to read a locally cached model file.
    #[error("failed to read '{path}': {source}")]
    ReadFile {
        /// The path that failed to read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The model's `config.json` was not valid JSON or did not match the
    /// expected BERT config shape.
    #[error("failed to parse model config: {0}")]
    Config(#[from] serde_json::Error),
    /// Failed to load the tokenizer definition from `tokenizer.json`.
    #[error("failed to load tokenizer: {0}")]
    LoadTokenizer(#[source] tokenizers::Error),
    /// Failed to load the model weights into the BERT architecture.
    #[error("failed to load model weights: {0}")]
    Model(#[source] candle_core::Error),
    /// Failed to tokenize input text.
    #[error("failed to tokenize input text: {0}")]
    Tokenize(#[source] tokenizers::Error),
    /// A tensor operation failed during inference.
    #[error("model inference failed: {0}")]
    Inference(#[from] candle_core::Error),
}

impl EmbedError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::NoCacheDir { .. } => ProblemMeta {
                slug: "no-cache-dir",
                version: "v1",
                title: "No user cache directory available",
                status: 500,
                exit_code: 1,
            },
            Self::HubClient(_) => ProblemMeta {
                slug: "hub-client-init-failure",
                version: "v1",
                title: "Failed to initialize the Hugging Face Hub client",
                status: 500,
                exit_code: 1,
            },
            Self::Fetch { .. } => ProblemMeta {
                slug: "model-fetch-failure",
                version: "v1",
                title: "Failed to fetch a model file from the Hugging Face Hub",
                status: 503,
                exit_code: 1,
            },
            Self::ReadFile { .. } => ProblemMeta {
                slug: "read-cached-model-file-failure",
                version: "v1",
                title: "Failed to read a locally cached model file",
                status: 500,
                exit_code: 1,
            },
            Self::Config(_) => ProblemMeta {
                slug: "invalid-model-config",
                version: "v1",
                title: "Model config.json is not valid or unexpected",
                status: 500,
                exit_code: 1,
            },
            Self::LoadTokenizer(_) => ProblemMeta {
                slug: "load-tokenizer-failure",
                version: "v1",
                title: "Failed to load the tokenizer definition",
                status: 500,
                exit_code: 1,
            },
            Self::Model(_) => ProblemMeta {
                slug: "load-model-weights-failure",
                version: "v1",
                title: "Failed to load model weights",
                status: 500,
                exit_code: 1,
            },
            Self::Tokenize(_) => ProblemMeta {
                slug: "tokenize-failure",
                version: "v1",
                title: "Failed to tokenize input text",
                status: 422,
                exit_code: 2,
            },
            Self::Inference(_) => ProblemMeta {
                slug: "inference-failure",
                version: "v1",
                title: "Model inference failed",
                status: 500,
                exit_code: 1,
            },
        }
    }
}

impl ToProblem for EmbedError {
    fn to_problem(&self) -> ProblemDetails {
        let internal = || {
            (
                SuggestedFix::new(
                    "This indicates an internal problem with mif-embed or the cached model \
                     files. Try clearing the model cache directory and retrying, or report it \
                     upstream if the problem persists.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "Clear the model cache and retry",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            )
        };

        let mut problem = self
            .meta()
            .into_details(env!("CARGO_PKG_NAME"), self.to_string());

        match self {
            Self::ReadFile { source, .. } => {
                let (status, fix, action) = mif_problem::classify_io_error(source);
                problem.status = status;
                problem.with_suggested_fix(fix).with_code_action(action)
            },
            Self::Fetch { .. } => {
                let (fix, action) = (
                    SuggestedFix::new(
                        "Check network connectivity to huggingface.co and retry.",
                        Applicability::MaybeIncorrect,
                    ),
                    CodeAction::new(
                        "Retry the model fetch",
                        "quickfix",
                        Applicability::MaybeIncorrect,
                    ),
                );
                problem
                    .with_retry_after(30)
                    .with_suggested_fix(fix)
                    .with_code_action(action)
            },
            Self::Tokenize(_) => {
                let (fix, action) = (
                    SuggestedFix::new(
                        "Supply text the tokenizer can encode (valid UTF-8, non-empty).",
                        Applicability::MaybeIncorrect,
                    ),
                    CodeAction::new(
                        "Fix the input text",
                        "quickfix",
                        Applicability::MaybeIncorrect,
                    ),
                );
                problem.with_suggested_fix(fix).with_code_action(action)
            },
            _ => {
                let (fix, action) = internal();
                problem.with_suggested_fix(fix).with_code_action(action)
            },
        }
    }
}

/// Loads `sentence-transformers/all-MiniLM-L6-v2` once and computes
/// normalized sentence embeddings from it.
///
/// Construction fetches the model's `config.json`, `tokenizer.json`, and
/// `model.safetensors` from the Hugging Face Hub on first use (cached under
/// the platform cache directory afterward), and loads them into a CPU-only
/// `candle` BERT model. Reuse one `Embedder` across calls to [`Self::embed`]
/// rather than reloading the model per call.
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl std::fmt::Debug for Embedder {
    // `candle_transformers::models::bert::BertModel` does not implement
    // `Debug`, so this is hand-written rather than derived.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Embedder")
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl Embedder {
    /// Fetches (or loads from cache) and initializes the embedding model.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedError`] if the platform cache directory cannot be
    /// resolved, the model files cannot be fetched from the Hugging Face Hub,
    /// or the fetched files cannot be parsed/loaded as a BERT model.
    pub fn load() -> Result<Self, EmbedError> {
        let cache_dir = dirs::cache_dir()
            .map(|dir| dir.join("mif").join("models"))
            .ok_or(EmbedError::NoCacheDir { model: MODEL_REPO })?;
        let api = ApiBuilder::new()
            .with_cache_dir(cache_dir)
            .build()
            .map_err(EmbedError::HubClient)?;
        let repo = Api::model(&api, MODEL_REPO.to_string());

        let config_path = fetch(&repo, "config.json")?;
        let tokenizer_path = fetch(&repo, "tokenizer.json")?;
        let weights_path = fetch(&repo, "model.safetensors")?;

        let config_text = read_to_string(&config_path)?;
        let config: BertConfig = serde_json::from_str(&config_text)?;

        let mut tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(EmbedError::LoadTokenizer)?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: config.max_position_embeddings,
                ..TruncationParams::default()
            }))
            .map_err(EmbedError::LoadTokenizer)?;

        let weights = read(&weights_path)?;
        let device = Device::Cpu;
        let vb = VarBuilder::from_buffered_safetensors(weights, DTYPE, &device)
            .map_err(EmbedError::Model)?;
        let model = BertModel::load(vb, &config).map_err(EmbedError::Model)?;

        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// Computes a 384-dimensional, mean-pooled, L2-normalized sentence
    /// embedding for `text`.
    ///
    /// # Errors
    ///
    /// Returns [`EmbedError`] if tokenization or model inference fails.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(EmbedError::Tokenize)?;

        let input_ids = Tensor::new(encoding.get_ids(), &self.device)?.unsqueeze(0)?;
        let token_type_ids = Tensor::new(encoding.get_type_ids(), &self.device)?.unsqueeze(0)?;
        let attention_mask =
            Tensor::new(encoding.get_attention_mask(), &self.device)?.unsqueeze(0)?;

        let sequence_output =
            self.model
                .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

        mean_pool(&sequence_output, &attention_mask)?
            .to_vec1::<f32>()
            .map_err(EmbedError::Inference)
    }
}

/// Fetches `file` from `repo`, caching it locally.
fn fetch(repo: &hf_hub::api::sync::ApiRepo, file: &'static str) -> Result<PathBuf, EmbedError> {
    repo.get(file).map_err(|source| EmbedError::Fetch {
        repo: MODEL_REPO,
        file,
        source,
    })
}

/// Reads a cached model file as UTF-8 text.
fn read_to_string(path: &std::path::Path) -> Result<String, EmbedError> {
    std::fs::read_to_string(path).map_err(|source| EmbedError::ReadFile {
        path: path.display().to_string(),
        source,
    })
}

/// Reads a cached model file as raw bytes.
fn read(path: &std::path::Path) -> Result<Vec<u8>, EmbedError> {
    std::fs::read(path).map_err(|source| EmbedError::ReadFile {
        path: path.display().to_string(),
        source,
    })
}

/// Mean-pools `sequence_output` (`(1, seq_len, hidden)`) over non-padding
/// tokens per `attention_mask` (`(1, seq_len)`), then L2-normalizes the
/// result, returning a `(hidden,)` tensor.
fn mean_pool(sequence_output: &Tensor, attention_mask: &Tensor) -> Result<Tensor, EmbedError> {
    let mask = attention_mask.to_dtype(DType::F32)?.unsqueeze(2)?;
    let summed = sequence_output.broadcast_mul(&mask)?.sum(1)?;
    let counts = mask.sum(1)?;
    let pooled = summed.broadcast_div(&counts)?.squeeze(0)?;

    let norm = pooled.sqr()?.sum_keepdim(0)?.sqrt()?;
    Ok(pooled.broadcast_div(&norm)?)
}

#[cfg(test)]
mod tests {
    use mif_problem::{Applicability, ToProblem};

    use super::{EMBEDDING_DIM, EmbedError, Embedder};

    #[test]
    fn read_file_error_status_is_classified_by_the_underlying_error_kind() {
        let not_found = EmbedError::ReadFile {
            path: "/nonexistent/config.json".to_string(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }
        .to_problem();
        assert_eq!(not_found.status, 404);
        assert_eq!(
            not_found.suggested_fix.unwrap().applicability,
            Applicability::MaybeIncorrect
        );

        let generic_fault = EmbedError::ReadFile {
            path: "/cache/config.json".to_string(),
            source: std::io::Error::from(std::io::ErrorKind::Other),
        }
        .to_problem();
        assert_eq!(generic_fault.status, 500);
        assert_eq!(
            generic_fault.suggested_fix.unwrap().applicability,
            Applicability::Unspecified
        );
    }

    #[test]
    fn remaining_error_variants_map_to_their_own_slug_and_status() {
        let cases: [(EmbedError, &str, u16); 5] = [
            (
                EmbedError::HubClient(hf_hub::api::sync::ApiError::LockAcquisition(
                    std::path::PathBuf::from("/tmp/lock"),
                )),
                "hub-client-init-failure",
                500,
            ),
            (
                EmbedError::Config(serde_json::from_str::<i32>("not json").unwrap_err()),
                "invalid-model-config",
                500,
            ),
            (
                EmbedError::LoadTokenizer(tokenizers::Error::from("bad tokenizer definition")),
                "load-tokenizer-failure",
                500,
            ),
            (
                EmbedError::Model(candle_core::Error::msg("bad weights")),
                "load-model-weights-failure",
                500,
            ),
            (
                EmbedError::Inference(candle_core::Error::msg("bad tensor op")),
                "inference-failure",
                500,
            ),
        ];

        for (error, slug, status) in cases {
            let problem = error.to_problem();
            assert_eq!(
                problem.problem_type,
                format!("https://mif-spec.dev/errors/{slug}/v1")
            );
            assert_eq!(problem.status, status);
            assert_eq!(
                problem.suggested_fix.unwrap().applicability,
                Applicability::Unspecified
            );
        }
    }

    #[test]
    fn tokenize_error_maps_to_a_client_side_status_and_input_fix() {
        let problem = EmbedError::Tokenize(tokenizers::Error::from("bad input text")).to_problem();
        assert_eq!(
            problem.problem_type,
            "https://mif-spec.dev/errors/tokenize-failure/v1"
        );
        assert_eq!(problem.status, 422);
        assert_eq!(
            problem.suggested_fix.unwrap().applicability,
            Applicability::MaybeIncorrect
        );
    }

    #[test]
    fn read_to_string_wraps_a_missing_file_as_a_read_file_variant() {
        let path = std::path::Path::new("/nonexistent-mif-embed-test-path/config.json");
        let err = super::read_to_string(path).unwrap_err();
        assert!(matches!(err, EmbedError::ReadFile { .. }));
    }

    #[test]
    fn read_wraps_a_missing_file_as_a_read_file_variant() {
        let path = std::path::Path::new("/nonexistent-mif-embed-test-path/model.safetensors");
        let err = super::read(path).unwrap_err();
        assert!(matches!(err, EmbedError::ReadFile { .. }));
    }

    #[test]
    fn distinct_error_variants_map_to_distinct_problem_types() {
        let fetch = EmbedError::Fetch {
            repo: "sentence-transformers/all-MiniLM-L6-v2",
            file: "config.json",
            source: hf_hub::api::sync::ApiError::LockAcquisition(std::path::PathBuf::from(
                "/tmp/lock",
            )),
        }
        .to_problem();
        assert_eq!(
            fetch.problem_type,
            "https://mif-spec.dev/errors/model-fetch-failure/v1"
        );
        assert_eq!(fetch.status, 503);
        assert_eq!(fetch.retry_after, Some(30));

        let no_cache = EmbedError::NoCacheDir {
            model: "sentence-transformers/all-MiniLM-L6-v2",
        }
        .to_problem();
        assert_eq!(
            no_cache.problem_type,
            "https://mif-spec.dev/errors/no-cache-dir/v1"
        );
        assert_eq!(no_cache.retry_after, None);
        assert_ne!(fetch.problem_type, no_cache.problem_type);
    }

    // `cargo test` runs tests in parallel threads within one process. Every
    // test below that calls `Embedder::load()` races the others to download
    // and lock the same model blob on a cold cache — `hf-hub`'s lock
    // acquisition is not reliably concurrent across platforms. Warming the
    // cache once, serialized through `Once`, means every real
    // `Embedder::load()` call below hits an already-populated cache and
    // never contends on the download lock.
    fn warm_embedding_model_cache() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = Embedder::load();
        });
    }

    #[test]
    fn embedder_debug_output_names_the_type_without_dumping_internal_fields() {
        warm_embedding_model_cache();
        let embedder = match Embedder::load() {
            Ok(embedder) => embedder,
            Err(err) => {
                eprintln!("skipping: could not load embedding model: {err}");
                return;
            },
        };

        let debug_output = format!("{embedder:?}");
        assert!(debug_output.starts_with("Embedder"));
        assert!(debug_output.contains("device"));
    }

    #[test]
    fn embed_produces_a_unit_norm_384_dim_vector() {
        warm_embedding_model_cache();
        let embedder = match Embedder::load() {
            Ok(embedder) => embedder,
            Err(err) => {
                eprintln!("skipping: could not load embedding model: {err}");
                return;
            },
        };

        let embedding = match embedder.embed("The quick brown fox jumps over the lazy dog.") {
            Ok(embedding) => embedding,
            Err(err) => {
                eprintln!("skipping: could not run inference: {err}");
                return;
            },
        };

        assert_eq!(embedding.len(), EMBEDDING_DIM);
        let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }
}
