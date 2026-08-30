//! Deterministic process-local retrieval over an immutable configured corpus.

use std::collections::BTreeMap;

use crate::validation::{MAX_STATIC_DOCUMENTS, MAX_STATIC_TEXT_BYTES, checked_add_with_limit};
use crate::{
    DocumentId, KnowledgeDocument, KnowledgeError, KnowledgeIndex, NamespaceId, SearchRequest,
    TenantId,
};

type ScopedDocumentId = (TenantId, NamespaceId, DocumentId);

/// An immutable, bounded, in-process knowledge index.
pub struct StaticKnowledgeIndex {
    documents: BTreeMap<ScopedDocumentId, KnowledgeDocument>,
}

impl StaticKnowledgeIndex {
    /// Validates and stores an immutable corpus in canonical scoped order.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::LimitExceeded`] when corpus count or aggregate
    /// text exceeds its ceiling, and [`KnowledgeError::InvalidRequest`] for a
    /// duplicate scoped document identifier.
    pub fn new(documents: Vec<KnowledgeDocument>) -> Result<Self, KnowledgeError> {
        if documents.len() > MAX_STATIC_DOCUMENTS {
            return Err(KnowledgeError::LimitExceeded);
        }

        let mut total_text_bytes = 0;
        let mut ordered = BTreeMap::new();
        for document in documents {
            total_text_bytes = checked_add_with_limit(
                total_text_bytes,
                document.text().len(),
                MAX_STATIC_TEXT_BYTES,
            )?;
            let key = (
                document.tenant_id().clone(),
                document.namespace().clone(),
                document.document_id().clone(),
            );
            if ordered.insert(key, document).is_some() {
                return Err(KnowledgeError::InvalidRequest);
            }
        }

        Ok(Self { documents: ordered })
    }
}

impl KnowledgeIndex for StaticKnowledgeIndex {
    fn search(&self, request: &SearchRequest) -> Result<Vec<KnowledgeDocument>, KnowledgeError> {
        Ok(self
            .documents
            .values()
            .filter(|document| {
                document.tenant_id() == request.context().tenant_id()
                    && document.namespace() == request.namespace()
                    && document.text().contains(request.query().as_str())
            })
            .take(request.limit().get() as usize)
            .cloned()
            .collect())
    }
}
