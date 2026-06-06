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
pub mod openai;

// Re-export providers for convenience
pub use local::LocalProvider;
pub use openai::OpenAIProvider;

use crate::error::Result;

/// Embedding dimension for different models
pub const OPENAI_EMBEDDING_DIM: usize = 1536; // text-embedding-3-small
pub const LOCAL_EMBEDDING_DIM: usize = 384; // all-MiniLM-L6-v2

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

/// Embed all symbols that don't have embeddings yet.
pub fn embed_missing_symbols<P: EmbeddingProvider + ?Sized>(
    db: &crate::db::Database,
    provider: &P,
    batch_size: usize,
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

        // Generate embeddings
        let embeddings = provider.embed_batch(&text_refs)?;

        // Store embeddings
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

    #[test]
    fn test_embedding_json_roundtrip() {
        let emb = Embedding::new(vec![0.1, 0.2, 0.3]);
        let json = emb.to_json().unwrap();
        let restored = Embedding::from_json(&json).unwrap();
        assert_eq!(emb.vector, restored.vector);
    }
}
