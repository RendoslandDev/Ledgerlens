//! Capability traits the pipeline needs. Synchronous on purpose: inference is blocking,
//! CPU-bound work, so async lives in the job/command layer via `spawn_blocking`.
use crate::{document::DocKind, error::Result, record::RecordCandidate, text::PageText};

pub trait TextEmbedder: Send + Sync {
    fn dim(&self) -> usize;
    fn embed_passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Separate from passages: many embedding models use a different prefix for queries.
    fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

pub trait FieldExtractor: Send + Sync {
    fn extract(&self, page: &PageText, kind: DocKind) -> Result<RecordCandidate>;
}

