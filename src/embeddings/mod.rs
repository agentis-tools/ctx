//! Semantic search via embeddings.
//!
//! This module provides embedding generation and vector similarity search
//! for semantic code search. It supports multiple embedding providers:
//!
//! - **OpenAI**: Uses text-embedding-3-small for high-quality embeddings
//! - **Local**: Uses fastembed for local, offline embeddings
//!
//! # Architecture
//!
//! Embeddings are stored in SQLite as JSON-encoded float arrays. Similarity
//! search is performed in Rust using cosine similarity for accuracy and
//! portability (no native vector extensions required).
//!
//! # Usage
//!
//! ```ignore
//! let provider = OpenAIProvider::new("sk-...")?;
//! let embedding = provider.embed("fn authenticate(user: &str)")?;
//!
//! let results = search_similar(&db, "authentication functions", 10)?;
//! ```

pub mod local;
pub mod ollama;
pub mod openai;

// Re-export providers for convenience
pub use local::LocalProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAIProvider;

use rayon::prelude::*;

use crate::error::{CtxError, Result};

/// Embedding dimension for different models
pub const OPENAI_EMBEDDING_DIM: usize = 1536; // text-embedding-3-small
pub const LOCAL_EMBEDDING_DIM: usize = 384; // all-MiniLM-L6-v2

/// Which embedding backend to use.
///
/// `local` (fastembed) is the zero-config default; `openai` needs `OPENAI_API_KEY`;
/// `ollama` talks to a local/remote Ollama server (`OLLAMA_HOST`,
/// `OLLAMA_EMBED_MODEL`). Embeddings from different providers/models live in
/// different vector spaces, so switching provider requires re-embedding.
#[derive(clap::ValueEnum, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// fastembed, local + offline (all-MiniLM-L6-v2, 384-dim).
    #[default]
    Local,
    /// OpenAI API (text-embedding-3-small, 1536-dim). Requires `OPENAI_API_KEY`.
    Openai,
    /// Ollama server (model-dependent dimension). Local + offline.
    Ollama,
}

impl Provider {
    /// Resolve the effective provider by precedence:
    /// `--provider` flag > deprecated `--openai` flag > `.ctx/config.toml`
    /// (`[embedding].provider`) > built-in default (`local`).
    pub fn resolve(
        provider: Option<Provider>,
        openai_flag: bool,
        config_default: Option<Provider>,
    ) -> Provider {
        match provider {
            Some(p) => p,
            None if openai_flag => Provider::Openai,
            None => config_default.unwrap_or_default(),
        }
    }

    /// Human-readable name matching `EmbeddingProvider::name()`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::Local => "local",
            Provider::Openai => "openai",
            Provider::Ollama => "ollama",
        }
    }
}

/// Build the embedding provider for the given backend, applying any
/// provider-specific settings from `.ctx/config.toml` (`embedding`). This is the
/// single place providers are constructed, so a new backend wires in once.
///
/// Env vars still take precedence over the config values (see the Ollama
/// resolvers); pass `&EmbeddingConfig::default()` when there is no config.
pub fn build_provider(
    provider: Provider,
    embedding: &crate::config::EmbeddingConfig,
) -> Result<Box<dyn EmbeddingProvider>> {
    match provider {
        Provider::Local => Ok(Box::new(local::LocalProvider::new()?)),
        Provider::Openai => {
            let p = openai::OpenAIProvider::from_env().map_err(|_| {
                CtxError::embedding(
                    "OPENAI_API_KEY environment variable not set.\n\
                     Set it with: export OPENAI_API_KEY=sk-...",
                )
            })?;
            Ok(Box::new(p))
        }
        Provider::Ollama => Ok(Box::new(ollama::OllamaProvider::from_config(
            embedding.model.as_deref(),
            embedding.host.as_deref(),
        )?)),
    }
}

/// Warn (to stderr) when the query provider/dimension differs from what the index
/// was embedded with. Embeddings from different providers/models occupy different
/// vector spaces, so mixing them yields meaningless similarities — the fix is to
/// re-embed. No-op when the index is empty or consistent.
pub fn warn_index_mismatch(db: &crate::db::Database, provider: &dyn EmbeddingProvider) {
    let query_dim = provider.dimension();
    let query_name = provider.name();
    if let Ok(metadata) = db.get_embedding_metadata() {
        for (stored_provider, _model, stored_dim, count) in &metadata {
            let stored_dim = *stored_dim as usize;
            if stored_dim != query_dim || stored_provider != query_name {
                eprintln!("Warning: embedding provider/dimension mismatch with the index!");
                eprintln!(
                    "  Index: {count} embeddings from '{stored_provider}' (dim {stored_dim})"
                );
                eprintln!("  Query: '{query_name}' (dim {query_dim})");
                eprintln!(
                    "  Results may be inaccurate. Re-run `ctx embed --provider {query_name}` \
                     to regenerate embeddings."
                );
                eprintln!();
                break;
            }
        }
    }
}

/// A vector embedding.
#[derive(Debug, Clone)]
pub struct Embedding {
    /// The embedding vector
    pub vector: Vec<f32>,
    /// Number of tokens in the input
    #[allow(dead_code)]
    pub token_count: Option<usize>,
}

impl Embedding {
    /// Create a new embedding from a vector.
    pub fn new(vector: Vec<f32>) -> Self {
        Self {
            vector,
            token_count: None,
        }
    }

    /// Get the dimension of this embedding.
    #[allow(dead_code)]
    pub fn dim(&self) -> usize {
        self.vector.len()
    }

    /// Compute cosine similarity with another embedding.
    #[allow(dead_code)]
    pub fn cosine_similarity(&self, other: &Embedding) -> f32 {
        cosine_similarity(&self.vector, &other.vector)
    }

    /// Serialize to JSON for storage.
    #[allow(dead_code)]
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.vector)?)
    }

    /// Deserialize from JSON.
    #[allow(dead_code)]
    pub fn from_json(json: &str) -> Result<Self> {
        let vector: Vec<f32> = serde_json::from_str(json)?;
        Ok(Self::new(vector))
    }
}

/// Trait for embedding providers.
pub trait EmbeddingProvider: Send + Sync {
    /// Get the name of this provider.
    fn name(&self) -> &str;

    /// Get the embedding dimension for this provider.
    fn dimension(&self) -> usize;

    /// Generate an embedding for a single text.
    fn embed(&self, text: &str) -> Result<Embedding>;

    /// Generate embeddings for multiple texts (batch).
    /// Default implementation calls embed() for each text.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
}

/// Compute cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Compute dot product similarity between two vectors.
#[allow(dead_code)]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Normalize a vector to unit length.
#[allow(dead_code)]
pub fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Search result from similarity search.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Symbol ID
    pub symbol_id: String,
    /// Similarity score (0.0 to 1.0)
    pub score: f32,
    /// Symbol name
    pub name: String,
    /// Symbol kind
    pub kind: String,
    /// File path
    pub file_path: String,
    /// Line number
    pub line: u32,
}

/// Perform semantic similarity search using embeddings.
///
/// This automatically uses the fast vector search (sqlite-vec) when available,
/// falling back to O(n) cosine similarity search otherwise.
pub fn semantic_search(
    db: &crate::db::Database,
    query_embedding: &Embedding,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // Try fast vector search first (sqlite-vec, O(log n))
    if db.has_vector_embeddings() {
        if let Ok(results) = db.vector_search(&query_embedding.vector, limit) {
            if !results.is_empty() {
                // Convert L2 distance to similarity score (0-1 range)
                // L2 distance 0 = identical, higher = less similar
                // We use 1/(1+d) to convert to similarity
                return Ok(results
                    .into_iter()
                    .map(
                        |(symbol_id, name, kind, file_path, line, distance)| SearchResult {
                            symbol_id,
                            score: 1.0 / (1.0 + distance),
                            name,
                            kind,
                            file_path,
                            line,
                        },
                    )
                    .collect());
            }
        }
    }

    // Fallback to O(n) cosine similarity search
    semantic_search_slow(db, query_embedding, limit)
}

/// O(n) semantic search using cosine similarity.
/// This loads all embeddings and computes similarity for each.
fn semantic_search_slow(
    db: &crate::db::Database,
    query_embedding: &Embedding,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    // Get all embeddings from database
    let all_embeddings = db.get_all_embeddings()?;

    // Compute similarity for each
    let mut scored: Vec<_> = all_embeddings
        .into_iter()
        .map(|(symbol_id, name, kind, file_path, line, vector)| {
            let score = cosine_similarity(&query_embedding.vector, &vector);
            SearchResult {
                symbol_id,
                score,
                name,
                kind,
                file_path,
                line,
            }
        })
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);

    Ok(scored)
}

/// Compute embeddings for a batch of texts, splitting the work across rayon
/// threads. The batch is divided into chunks and each chunk's `embed_batch`
/// call runs concurrently.
///
/// Ordering is preserved: rayon's parallel `collect` keeps the chunks in
/// source order, and each chunk keeps its own internal order, so flattening
/// yields a `Vec<Embedding>` that lines up 1:1 with `texts`. Each chunk goes
/// through the provider's normal `embed_batch`, so the provider's retry/backoff
/// (e.g. the OpenAI rate-limit handling) is fully preserved.
fn embed_texts_parallel<P: EmbeddingProvider + ?Sized>(
    provider: &P,
    texts: &[&str],
) -> Result<Vec<Embedding>> {
    // One chunk per worker thread (at least one), so provider calls run
    // concurrently without over-splitting small batches.
    let num_chunks = rayon::current_num_threads().max(1);
    let chunk_size = texts.len().div_ceil(num_chunks).max(1);

    let per_chunk: Vec<Vec<Embedding>> = texts
        .par_chunks(chunk_size)
        .map(|chunk| provider.embed_batch(chunk))
        .collect::<Result<Vec<_>>>()?;

    Ok(per_chunk.into_iter().flatten().collect())
}

/// Embed all symbols that don't have embeddings yet.
///
/// Embedding computation is parallelized across rayon threads by default; pass
/// `serial = true` for the single-threaded path. Regardless of mode, every
/// `db.store_embedding` call happens serially on the owning thread (the
/// `Database` connection is not shared for writes), and embeddings are stored
/// in symbol order.
pub fn embed_missing_symbols<P: EmbeddingProvider + ?Sized>(
    db: &crate::db::Database,
    provider: &P,
    batch_size: usize,
    serial: bool,
    progress_callback: Option<&dyn Fn(usize, usize)>,
) -> Result<usize> {
    let mut total_embedded = 0;

    loop {
        // Get symbols without embeddings
        let symbols = db.get_symbols_without_embeddings(batch_size as i64)?;

        if symbols.is_empty() {
            break;
        }

        // Generate embedding text for each symbol
        let texts: Vec<String> = symbols.iter().map(|s| s.to_embedding_text()).collect();

        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        // Generate embeddings (parallel compute by default, serial on opt-out).
        // Both paths return embeddings in the same order as `text_refs`.
        let embeddings = if serial {
            provider.embed_batch(&text_refs)?
        } else {
            embed_texts_parallel(provider, &text_refs)?
        };

        // Store embeddings serially on the owning thread, in symbol order.
        for (symbol, embedding) in symbols.iter().zip(embeddings.iter()) {
            db.store_embedding(&symbol.id, provider.name(), "default", &embedding.vector)?;
        }

        total_embedded += symbols.len();

        if let Some(callback) = progress_callback {
            callback(total_embedded, 0); // 0 = unknown total
        }

        if symbols.len() < batch_size {
            break;
        }
    }

    Ok(total_embedded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);

        let d = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &d) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_normalize() {
        let mut v = vec![3.0, 4.0];
        normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
    }

    /// Deterministic provider: embeds each text as a 1-dim vector holding the
    /// byte length of the text. Lets us assert ordering without any network.
    struct LenProvider;

    impl EmbeddingProvider for LenProvider {
        fn name(&self) -> &str {
            "len"
        }
        fn dimension(&self) -> usize {
            1
        }
        fn embed(&self, text: &str) -> Result<Embedding> {
            Ok(Embedding::new(vec![text.len() as f32]))
        }
    }

    #[test]
    fn test_embed_texts_parallel_preserves_order() {
        // Distinct lengths so each text has a unique embedding.
        let texts = ["a", "bb", "ccc", "dddd", "eeeee", "ffffff", "g", "hh"];
        let refs: Vec<&str> = texts.to_vec();

        let serial = LenProvider.embed_batch(&refs).unwrap();
        let parallel = embed_texts_parallel(&LenProvider, &refs).unwrap();

        assert_eq!(serial.len(), texts.len());
        assert_eq!(parallel.len(), texts.len());
        for (i, text) in texts.iter().enumerate() {
            let expected = text.len() as f32;
            assert_eq!(serial[i].vector, vec![expected]);
            assert_eq!(parallel[i].vector, vec![expected], "order mismatch at {i}");
        }
    }

    #[test]
    fn test_embed_texts_parallel_empty() {
        let parallel = embed_texts_parallel(&LenProvider, &[]).unwrap();
        assert!(parallel.is_empty());
    }

    #[test]
    fn test_embedding_json_roundtrip() {
        let emb = Embedding::new(vec![0.1, 0.2, 0.3]);
        let json = emb.to_json().unwrap();
        let restored = Embedding::from_json(&json).unwrap();
        assert_eq!(emb.vector, restored.vector);
    }
}
