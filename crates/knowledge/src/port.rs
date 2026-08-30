use crate::{KnowledgeDocument, KnowledgeError, SearchRequest};

/// A synchronous adapter seam for bounded knowledge retrieval.
pub trait KnowledgeIndex: Send + Sync {
    /// Returns matching documents in strict ascending document identifier order.
    ///
    /// # Errors
    ///
    /// Returns a closed [`KnowledgeError`](crate::KnowledgeError) when the index
    /// cannot satisfy the request or detects a protocol failure.
    fn search(&self, request: &SearchRequest) -> Result<Vec<KnowledgeDocument>, KnowledgeError>;
}
