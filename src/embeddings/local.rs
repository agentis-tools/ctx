//! Local embedding provider using fastembed.
//!
//! Uses all-MiniLM-L6-v2 (384 dimensions) for fast, offline embeddings.
//! No API key required - models are downloaded once and cached locally.

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Mutex;

use super::{Embedding, EmbeddingProvider, LOCAL_EMBEDDING_DIM};
use crate::error::{CtxError, Result};

/// Local embedding provider using fastembed.
pub struct LocalProvider {
    model: Mutex<TextEmbedding>,
    #[allow(dead_code)]
    model_name: String,
}

impl LocalProvider {
    /// Create a new local provider with the default model (all-MiniLM-L6-v2).
    pub fn new() -> Result<Self> {
        Self::with_model(EmbeddingModel::AllMiniLML6V2)
    }

    /// Create a provider with a specific model.
    pub fn with_model(model: EmbeddingModel) -> Result<Self> {
        let model_name = format!("{:?}", model);

        let text_embedding =
            TextEmbedding::try_new(InitOptions::new(model).with_show_download_progress(true))
                .map_err(|e| CtxError::ModelNotFound(e.to_string()))?;

        Ok(Self {
            model: Mutex::new(text_embedding),
            model_name,
        })
    }

    /// Create a provider with a larger, more accurate model.
    /// Uses BGE-base-en-v1.5 (768 dimensions).
    #[allow(dead_code)]
    pub fn new_large() -> Result<Self> {
        Self::with_model(EmbeddingModel::BGEBaseENV15)
    }
}

impl Default for LocalProvider {
    fn default() -> Self {
        Self::new().expect("Failed to initialize local embedding model")
    }
}

impl EmbeddingProvider for LocalProvider {
    fn name(&self) -> &str {
        "local"
    }

    fn dimension(&self) -> usize {
        LOCAL_EMBEDDING_DIM
    }

    fn embed(&self, text: &str) -> Result<Embedding> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| CtxError::embedding(format!("Lock error: {}", e)))?;

        let embeddings = model
            .embed(vec![text], None)
            .map_err(|e| CtxError::embedding(e.to_string()))?;

        let vector = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| CtxError::embedding("Empty embedding result"))?;

        Ok(Embedding::new(vector))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        let mut model = self
            .model
            .lock()
            .map_err(|e| CtxError::embedding(format!("Lock error: {}", e)))?;

        let texts_owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();

        let embeddings = model
            .embed(texts_owned, None)
            .map_err(|e| CtxError::embedding(e.to_string()))?;

        Ok(embeddings.into_iter().map(Embedding::new).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires model download
    fn test_local_embedding() {
        let provider = LocalProvider::new().expect("Failed to create provider");
        let embedding = provider.embed("Hello, world!").expect("Embedding failed");

        assert_eq!(embedding.dim(), LOCAL_EMBEDDING_DIM);

        // Check that the embedding is normalized (approximately unit length)
        let norm: f32 = embedding.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.1, "Embedding should be normalized");
    }

    #[test]
    #[ignore] // Requires model download
    fn test_batch_embedding() {
        let provider = LocalProvider::new().expect("Failed to create provider");
        let texts = vec!["Hello", "World", "Test"];
        let embeddings = provider
            .embed_batch(&texts)
            .expect("Batch embedding failed");

        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.dim(), LOCAL_EMBEDDING_DIM);
        }
    }

    #[test]
    #[ignore] // Requires model download
    fn test_similarity() {
        let provider = LocalProvider::new().expect("Failed to create provider");

        let emb1 = provider
            .embed("The cat sat on the mat")
            .expect("Embedding failed");
        let emb2 = provider
            .embed("A feline rested on the rug")
            .expect("Embedding failed");
        let emb3 = provider
            .embed("Python is a programming language")
            .expect("Embedding failed");

        let sim_similar = emb1.cosine_similarity(&emb2);
        let sim_different = emb1.cosine_similarity(&emb3);

        // Similar sentences should have higher similarity
        assert!(
            sim_similar > sim_different,
            "Similar sentences should have higher similarity: {} vs {}",
            sim_similar,
            sim_different
        );
    }
}
