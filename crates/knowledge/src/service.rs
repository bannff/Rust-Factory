use crate::validation::{MAX_RESULT_TEXT_BYTES, checked_add_with_limit};
use crate::{KnowledgeError, KnowledgeHit, KnowledgeIndex, SearchRequest, SearchResult};

/// Validates index output and projects it into safe consumer-facing results.
pub struct KnowledgeService<'a, I: KnowledgeIndex + ?Sized> {
    index: &'a I,
}

impl<'a, I: KnowledgeIndex + ?Sized> KnowledgeService<'a, I> {
    /// Borrows an index without taking ownership of its lifecycle.
    #[must_use]
    pub const fn new(index: &'a I) -> Self {
        Self { index }
    }

    /// Searches the index and validates the complete response before projection.
    ///
    /// # Errors
    ///
    /// Returns the index error unchanged, [`KnowledgeError::LimitExceeded`] for
    /// response ceilings, or [`KnowledgeError::ProtocolViolation`] for invalid
    /// response scope or ordering.
    pub fn search(&self, request: &SearchRequest) -> Result<SearchResult, KnowledgeError> {
        let documents = self.index.search(request)?;
        if documents.len() > request.limit().get() as usize {
            return Err(KnowledgeError::LimitExceeded);
        }

        let mut previous_id = None;
        let mut total_text_bytes = 0;
        for document in &documents {
            if document.tenant_id() != request.context().tenant_id()
                || document.namespace() != request.namespace()
            {
                return Err(KnowledgeError::ProtocolViolation);
            }
            if previous_id.is_some_and(|previous| previous >= document.document_id()) {
                return Err(KnowledgeError::ProtocolViolation);
            }
            previous_id = Some(document.document_id());
            total_text_bytes = checked_add_with_limit(
                total_text_bytes,
                document.text().len(),
                MAX_RESULT_TEXT_BYTES,
            )?;
        }

        let hits = documents
            .into_iter()
            .map(|document| {
                KnowledgeHit::new(document.document_id().clone(), document.text().to_owned())
            })
            .collect();
        Ok(SearchResult::new(hits))
    }
}
