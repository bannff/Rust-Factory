use std::num::NonZeroU32;

use crate::KnowledgeError;
use crate::validation::{
    MAX_DOCUMENT_TEXT_BYTES, MAX_QUERY_BYTES, MAX_SEARCH_LIMIT, validate_identifier,
};

macro_rules! identifier {
    ($(#[$attribute:meta])* $name:ident) => {
        $(#[$attribute])*
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Validates and constructs an identifier.
            ///
            /// # Errors
            ///
            /// Returns [`KnowledgeError::InvalidRequest`] when the value does not
            /// satisfy the identifier grammar or byte ceiling.
            pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            /// Returns the identifier exactly as supplied at construction.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

identifier!(/// Identifies the tenant whose corpus may be searched.
    TenantId);
identifier!(/// Identifies the trusted principal performing a search.
    PrincipalId);
identifier!(/// Identifies a tenant-scoped knowledge namespace.
    NamespaceId);
identifier!(/// Identifies a document within a tenant and namespace.
    DocumentId);

/// Exact, validated text used to match knowledge documents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query(String);

impl Query {
    /// Constructs a non-whitespace query within the fixed byte ceiling.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::InvalidRequest`] when the query is all
    /// whitespace or exceeds the byte ceiling.
    pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError> {
        let value = value.into();
        if value.len() > MAX_QUERY_BYTES
            || !value.chars().any(|character| !character.is_whitespace())
        {
            return Err(KnowledgeError::InvalidRequest);
        }

        Ok(Self(value))
    }

    /// Returns the query exactly as supplied at construction.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A nonzero bounded maximum hit count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchLimit(NonZeroU32);

impl SearchLimit {
    /// Constructs a search limit in `1..=MAX_SEARCH_LIMIT`.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::InvalidRequest`] when the limit is zero or
    /// exceeds [`MAX_SEARCH_LIMIT`].
    pub fn new(value: u32) -> Result<Self, KnowledgeError> {
        if value > MAX_SEARCH_LIMIT {
            return Err(KnowledgeError::InvalidRequest);
        }

        NonZeroU32::new(value)
            .map(Self)
            .ok_or(KnowledgeError::InvalidRequest)
    }

    /// Returns the bounded hit count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// Trusted tenant and principal context for a search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchContext {
    tenant_id: TenantId,
    principal_id: PrincipalId,
}

impl SearchContext {
    /// Constructs search context from validated identifiers.
    #[must_use]
    pub const fn new(tenant_id: TenantId, principal_id: PrincipalId) -> Self {
        Self {
            tenant_id,
            principal_id,
        }
    }

    /// Returns the trusted tenant identifier.
    #[must_use]
    pub const fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    /// Returns the trusted principal identifier.
    #[must_use]
    pub const fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }
}

/// A fully validated request to search one tenant namespace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRequest {
    context: SearchContext,
    namespace: NamespaceId,
    query: Query,
    limit: SearchLimit,
}

impl SearchRequest {
    /// Constructs a search request from validated components.
    #[must_use]
    pub const fn new(
        context: SearchContext,
        namespace: NamespaceId,
        query: Query,
        limit: SearchLimit,
    ) -> Self {
        Self {
            context,
            namespace,
            query,
            limit,
        }
    }

    /// Returns the trusted search context.
    #[must_use]
    pub const fn context(&self) -> &SearchContext {
        &self.context
    }

    /// Returns the definition-selected namespace.
    #[must_use]
    pub const fn namespace(&self) -> &NamespaceId {
        &self.namespace
    }

    /// Returns the exact query.
    #[must_use]
    pub const fn query(&self) -> &Query {
        &self.query
    }

    /// Returns the maximum hit count.
    #[must_use]
    pub const fn limit(&self) -> SearchLimit {
        self.limit
    }
}

/// A validated document exchanged across the index boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeDocument {
    tenant_id: TenantId,
    namespace: NamespaceId,
    document_id: DocumentId,
    text: String,
}

impl KnowledgeDocument {
    /// Constructs a scoped document with nonempty, bounded text.
    ///
    /// # Errors
    ///
    /// Returns [`KnowledgeError::InvalidRequest`] when text is empty or exceeds
    /// the per-document byte ceiling.
    pub fn new(
        tenant_id: TenantId,
        namespace: NamespaceId,
        document_id: DocumentId,
        text: impl Into<String>,
    ) -> Result<Self, KnowledgeError> {
        let text = text.into();
        if text.is_empty() || text.len() > MAX_DOCUMENT_TEXT_BYTES {
            return Err(KnowledgeError::InvalidRequest);
        }

        Ok(Self {
            tenant_id,
            namespace,
            document_id,
            text,
        })
    }

    /// Returns the document tenant.
    #[must_use]
    pub const fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    /// Returns the document namespace.
    #[must_use]
    pub const fn namespace(&self) -> &NamespaceId {
        &self.namespace
    }

    /// Returns the document identifier.
    #[must_use]
    pub const fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    /// Returns the document text exactly as supplied at construction.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// One validated, safely projected search hit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeHit {
    document_id: DocumentId,
    text: String,
}

impl KnowledgeHit {
    pub(crate) const fn new(document_id: DocumentId, text: String) -> Self {
        Self { document_id, text }
    }

    /// Returns the hit's document identifier.
    #[must_use]
    pub const fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    /// Returns the hit text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A completely validated search result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    hits: Vec<KnowledgeHit>,
}

impl SearchResult {
    pub(crate) const fn new(hits: Vec<KnowledgeHit>) -> Self {
        Self { hits }
    }

    /// Returns all hits in strict ascending document identifier order.
    #[must_use]
    pub fn hits(&self) -> &[KnowledgeHit] {
        &self.hits
    }
}
