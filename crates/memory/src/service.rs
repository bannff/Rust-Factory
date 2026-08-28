//! The validating layer between a caller and an adapter.
//!
//! Every operation validates, then scopes to the context's tenant, then calls the
//! adapter.
//!
//! # What the scoping actually guarantees
//!
//! A caller handed a `&`[`MemoryContext`] cannot widen it: no request type has a
//! tenant field, so addressing another tenant is not expressible. That is the
//! whole of the guarantee. [`MemoryContext::new`] and
//! [`crate::model::TenantId::new`] are both public, so any code that can name a
//! tenant string can mint a context for it. Construction is therefore a
//! privileged operation, and keeping it privileged is a property of how a
//! composition root distributes contexts rather than something this module can
//! enforce.
//!
//! This matters most for a future request-handling surface. A tool handler that
//! built a context from its own request payload would defeat the isolation
//! entirely while every type still checked out, so such a surface must derive the
//! tenant from host-established identity before it reaches this module.

use crate::error::MemoryError;
use crate::model::{
    MAX_QUERY_LIMIT, MemoryKind, MemoryQuery, MemoryRecord, Metadata, Namespace, Provenance,
    RecordKey, RunId, TenantId, WriteOutcome,
};
use crate::port::{Clock, MemoryStore, StoreGuarantees};
use crate::validation::{validate_query, validate_record};

/// The tenant every operation is scoped to.
///
/// `Debug` prints the tenant verbatim. That is correct for a domain type, but it
/// makes a context — like a [`MemoryRecord`] — something that should not be
/// logged once a real observability seam exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryContext {
    tenant_id: TenantId,
}

impl MemoryContext {
    /// Builds a context for a tenant.
    ///
    /// **Privileged.** Nothing here verifies that `tenant_id` was established by
    /// trusted means, so calling this with a value taken from a caller's payload
    /// hands that caller another tenant's data. A composition root calls this
    /// after establishing identity itself; a request handler must not.
    #[must_use]
    pub const fn new(tenant_id: TenantId) -> Self {
        Self { tenant_id }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

/// What a caller supplies to record something.
///
/// The tenant is absent by construction: it comes from [`MemoryContext`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RememberRequest {
    pub namespace: Namespace,
    pub key: RecordKey,
    pub kind: MemoryKind,
    pub content: String,
    pub tags: Vec<String>,
    pub metadata: Metadata,
    /// The run this came from. When present the service stamps it with the
    /// injected clock, so a caller cannot backdate a memory.
    pub run_id: Option<RunId>,
}

/// Orchestrates validation, tenant scoping, and provenance stamping.
///
/// Generic over its dependencies so a caller composing statically pays no
/// dispatch cost, while `S = Arc<dyn MemoryStore>` gives a binary the runtime
/// selection that declarative configuration needs. Both work through the same
/// type.
pub struct MemoryService<S, C> {
    store: S,
    clock: C,
    result_ceiling: u32,
}

impl<S: MemoryStore, C: Clock> MemoryService<S, C> {
    /// Builds a service whose result ceiling is the core's own
    /// [`crate::model::MAX_QUERY_LIMIT`].
    #[must_use]
    pub const fn new(store: S, clock: C) -> Self {
        Self {
            store,
            clock,
            result_ceiling: MAX_QUERY_LIMIT,
        }
    }

    /// Builds a service that refuses a query asking for more than `ceiling`.
    ///
    /// A deployment may narrow the core's ceiling but never widen it. This is the
    /// seam a composition root uses to apply a configured limit: the limit is
    /// enforced here rather than in the configuration type, because a value that
    /// nothing reads is not a limit.
    ///
    /// # Errors
    ///
    /// [`MemoryError::LimitExceeded`] when `ceiling` is zero or above
    /// [`crate::model::MAX_QUERY_LIMIT`].
    pub fn with_result_ceiling(store: S, clock: C, ceiling: u32) -> Result<Self, MemoryError> {
        if ceiling == 0 || ceiling > MAX_QUERY_LIMIT {
            return Err(MemoryError::LimitExceeded);
        }
        Ok(Self {
            store,
            clock,
            result_ceiling: ceiling,
        })
    }

    /// The largest result this service will return.
    #[must_use]
    pub const fn result_ceiling(&self) -> u32 {
        self.result_ceiling
    }

    /// Validates and records one memory.
    ///
    /// Provenance is stamped from the injected clock rather than accepted from
    /// the caller, so recorded time is the service's account of when it learned
    /// something, not a claim the caller makes.
    ///
    /// # Errors
    ///
    /// A [`MemoryError`] when the record violates a rule in
    /// [`crate::validation`], or [`MemoryError::AdapterFailure`] from the
    /// backend.
    pub fn remember(
        &self,
        context: &MemoryContext,
        request: RememberRequest,
    ) -> Result<WriteOutcome, MemoryError> {
        let provenance = request.run_id.map(|run_id| Provenance {
            run_id,
            recorded_at: self.clock.now(),
        });
        let record = MemoryRecord {
            tenant_id: context.tenant_id.clone(),
            namespace: request.namespace,
            key: request.key,
            kind: request.kind,
            content: request.content,
            tags: request.tags,
            metadata: request.metadata,
            provenance,
        };
        validate_record(&record)?;
        self.store.put(record)
    }

    /// Reads one record within the context's tenant.
    ///
    /// # Errors
    ///
    /// [`MemoryError::AdapterFailure`] from the backend. A record belonging to
    /// another tenant reports `Ok(None)`, indistinguishable from absence.
    pub fn recall(
        &self,
        context: &MemoryContext,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        let found = self.store.get(&context.tenant_id, namespace, key)?;
        // Defence in depth: the port already requires isolation, but a buggy or
        // hostile adapter must not be able to leak through this service. All
        // three of tenant, namespace, and key are checked, not just tenant: a
        // same-tenant adapter bug that crossed namespaces would otherwise pass
        // straight through, and namespace separation is a contract clause in its
        // own right.
        Ok(found.filter(|record| Self::is_addressed(context, namespace, Some(key), record)))
    }

    /// Returns matching records within the context's tenant.
    ///
    /// # Errors
    ///
    /// A [`MemoryError`] when the query is invalid, or
    /// [`MemoryError::AdapterFailure`] from the backend.
    pub fn search(
        &self,
        context: &MemoryContext,
        namespace: &Namespace,
        query: &MemoryQuery,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        validate_query(query)?;
        if query.limit > self.result_ceiling {
            // Refused rather than clamped: silently returning fewer records than
            // asked for is indistinguishable from there being no more data, which
            // is how a caller ends up acting on a partial view believing it is
            // complete.
            return Err(MemoryError::LimitExceeded);
        }
        let mut found = self.store.query(&context.tenant_id, namespace, query)?;
        // Same defence in depth, plus the limit, so an adapter that over-returns
        // cannot make this service exceed its own contract. The key is not checked
        // here because a query legitimately spans keys.
        found.retain(|record| Self::is_addressed(context, namespace, None, record));
        found.truncate(query.limit as usize);
        Ok(found)
    }

    /// Removes one record, reporting whether it existed.
    ///
    /// # Errors
    ///
    /// [`MemoryError::AdapterFailure`] from the backend.
    pub fn forget(
        &self,
        context: &MemoryContext,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<bool, MemoryError> {
        self.store.delete(&context.tenant_id, namespace, key)
    }

    /// What the configured backend guarantees.
    #[must_use]
    pub fn guarantees(&self) -> StoreGuarantees {
        self.store.guarantees()
    }

    /// Reports whether a record is one the request actually addressed.
    ///
    /// `key` is `None` for a query, which spans keys by design.
    fn is_addressed(
        context: &MemoryContext,
        namespace: &Namespace,
        key: Option<&RecordKey>,
        record: &MemoryRecord,
    ) -> bool {
        record.tenant_id == context.tenant_id
            && &record.namespace == namespace
            && key.is_none_or(|addressed| &record.key == addressed)
    }
}
