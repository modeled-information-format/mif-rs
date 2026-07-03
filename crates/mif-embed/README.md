# mif-embed

Local sentence-embedding inference for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem, via [`candle`](https://github.com/huggingface/candle).

`Embedder` loads `sentence-transformers/all-MiniLM-L6-v2` (384-dimensional,
mean-pooled, L2-normalized sentence embeddings, exposed as `EMBEDDING_DIM`)
from the Hugging Face Hub on first use, caching the model files under the
platform cache directory (`dirs::cache_dir()/mif/models`) so later runs are
offline. Inference runs on CPU only.

`Embedder::load()` fetches (or loads from cache) `config.json`,
`tokenizer.json`, and `model.safetensors` for the model repo and builds a
CPU-only `candle` BERT model from them; `Embedder::embed(text)` tokenizes
`text`, runs it through the model, and returns a mean-pooled,
L2-normalized `Vec<f32>` of length `EMBEDDING_DIM`. Construction is the
expensive step — reuse one `Embedder` across calls to `embed` rather than
reloading the model per call.

If the platform has no resolvable user cache directory,
`Embedder::load()` fails with `EmbedError::NoCacheDir` rather than falling
back to an ephemeral location, since a model re-fetched on every run would
defeat the point of caching. `EmbedError` covers the rest of the failure
surface — Hub client initialization, model file fetch, config/tokenizer/
weight loading, tokenization, and inference — and implements
`mif_problem::ToProblem` for RFC 9457 `application/problem+json` reporting.

## License

MIT
