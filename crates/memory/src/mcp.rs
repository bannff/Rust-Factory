//! Bounded MCP control-plane adapter for memory operations.
//!
//! Enabled by the `mcp` feature. Owns transport DTOs, generated schemas, the
//! policy gate, and safe response projection — never process lifecycle.
//!
//! # The MCP surface accepts a strict subset of core-valid records
//!
//! [`crate::MAX_RECORD_BYTES`] is roughly 98 KiB, and JSON escaping can multiply
//! a worst-case `content` several times over. The stdio transport's frame limit
//! is 64 KiB for the whole JSON-RPC message and overflowing it is *terminal* —
//! the session closes without an error reaching the caller. So this module
//! declares its own tighter ceilings ([`MAX_MCP_CONTENT_BYTES`] and friends) and
//! a record accepted here is deliberately smaller than one the typed API
//! accepts. The alternative is a tool that silently kills the session.
//!
//! The consequence, stated plainly: a record written through the typed Rust API
//! at full core limits can be too large to project back over MCP. That read
//! **fails** with `limit_exceeded`; it never reports absence. Confusing "too big
//! to send" with "not there" is the one outcome that would make this surface
//! unsafe to build on.
//!
//! # Refusal, never truncation
//!
//! [`crate::MemoryService::search`] refuses a query above its ceiling rather than
//! clamping it, because a short answer is indistinguishable from there being no
//! more data. This module holds the same line: an oversized projection is an
//! error, not a shortened list.
//!
//! # Authorization is capability **and** grant
//!
//! `policy::GrantV1::memory_enabled` is a live flag — `workflow` projects it into
//! its effective capability ceiling and `agent` intersects it into an agent's
//! memory scope. Checking only the capability would let a principal holding
//! `memory_enabled = false` reach memory directly under the very same grant. Both
//! must hold.
//!
//! # Capacity is shared within a tenant, and deletion is unaudited
//!
//! Two properties an agent-facing surface makes reachable that are worth stating
//! here rather than only in the core.
//!
//! [`crate::MAX_PARTITION_RECORDS`] is a per-tenant budget with no per-principal
//! quota, so one agent holding `memory_remember` can consume a tenant's whole
//! allowance and every other principal in that tenant is then refused a new key.
//! A replace always still succeeds, so nothing becomes unrepairable.
//!
//! `memory_forget` is an unconditional, irreversible delete and emits no audit
//! record: there is no observability integration in this brick. With
//! `memory_search` an agent can enumerate keys and then delete them, and
//! afterwards nothing records that it happened. Today the blast radius is one
//! process lifetime, because both adapters are in-process and say so through
//! [`crate::StoreGuarantees`]. **Composing a durable store behind this surface
//! requires an audit seam first** — inheriting irreversible unaudited deletion
//! silently is exactly the kind of undeclared guarantee change this workspace
//! forbids.
//!
//! # Blocking
//!
//! The tool functions are `async` because rmcp requires it, but every one of them
//! does entirely synchronous work and both current adapters take a `Mutex` while
//! serving a request. A composition root must not assume these calls yield.

#![allow(unknown_lints)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unused_self)]
#![allow(clippy::unused_async_trait_impl)]

use anyhow::{Context, Result};
use policy::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, PolicyResolver,
    TrustedContextV1, canonical_grant, decision_digest,
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Clock, MemoryContext, MemoryError, MemoryKind, MemoryQuery, MemoryRecord, MemoryService,
    MemoryStore, Namespace, PublicErrorCode, RecordKey, RememberRequest, RunId, TenantId,
    Timestamp, WriteOutcome,
};

pub const MEMORY_TOOLS: [&str; 5] = [
    "memory_remember",
    "memory_recall",
    "memory_search",
    "memory_forget",
    "memory_status",
];

/// Whole-request ceiling, well inside the 64 KiB transport frame.
pub const MAX_MCP_REQUEST_BYTES: usize = 16 * 1024;
/// Raw serialized response ceiling.
///
/// Half of [`MAX_MCP_ESCAPED_TOOL_TEXT_BYTES`], because a tool returns a `String`
/// that the protocol embeds as JSON string content. This bounds the brick-owned
/// projection; it does not prove the complete JSON-RPC envelope fits a transport.
pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = MAX_MCP_ESCAPED_TOOL_TEXT_BYTES / 2;

/// Ceiling after the response is JSON-string escaped as MCP tool text.
///
/// This is a brick-local projection limit with conservative composition
/// headroom. The caller-controlled request ID and remaining JSON-RPC envelope
/// are not visible here, so only a composition root can enforce a complete
/// outbound frame limit.
pub const MAX_MCP_ESCAPED_TOOL_TEXT_BYTES: usize =
    BRICK_TOOL_TEXT_BUDGET_BYTES - COMPOSITION_HEADROOM_BYTES;

/// Reference budget used only to keep the brick-owned tool text conservative.
/// It is not a claim about a complete outgoing transport frame.
const BRICK_TOOL_TEXT_BUDGET_BYTES: usize = 64 * 1024;
/// Headroom reserved for composition-owned envelope data; not a proof that an
/// arbitrary echoed request ID fits.
const COMPOSITION_HEADROOM_BYTES: usize = 8 * 1024;

/// Content ceiling for a record written through MCP, measured on the **JSON
/// serialized** form rather than the raw string.
///
/// Measuring the raw string would not bound anything: `serde_json` escapes a
/// control character to `\u00XX`, so 4 KiB of raw control bytes serializes to
/// 24 KiB. Quotes and backslashes double, control characters sextuple, and
/// multi-byte UTF-8 passes through unchanged. Only the serialized length is a
/// real bound, so that is what is checked.
pub const MAX_MCP_CONTENT_BYTES: usize = 2 * 1024;
/// Tags one record written through MCP may carry, below [`crate::MAX_TAGS`].
///
/// Named for what it bounds. An earlier version applied a constant named for
/// metadata to the tag count, which was doubly wrong: metadata is not accepted
/// over MCP at all.
pub const MAX_MCP_TAGS: usize = 8;
/// Serialized byte ceiling for one tag written through MCP.
///
/// Checked at ingress so the response budget below can be computed, rather than
/// left to the core's grammar check which runs after the policy gate.
pub const MAX_MCP_TAG_BYTES: usize = 128;
/// Records one MCP search may request.
///
/// This is chosen so that `MAX_MCP_QUERY_LIMIT` records, each at the worst case
/// this surface admits, fit inside [`MAX_MCP_SERIALIZED_RESULT_BYTES`] — see
/// [`MAX_MCP_RECORD_PROJECTION_BYTES`]. The two ceilings are consistent by
/// construction, asserted at compile time below.
pub const MAX_MCP_QUERY_LIMIT: u32 = 6;

/// Worst-case serialized size of one projected record.
///
/// Content, tags, the identifiers, the fixed field names, and JSON punctuation.
/// The identifier allowance covers `namespace` and `key` at
/// [`crate::MAX_ID_BYTES`] each.
pub const MAX_MCP_RECORD_PROJECTION_BYTES: usize =
    MAX_MCP_CONTENT_BYTES + MAX_MCP_TAGS * (MAX_MCP_TAG_BYTES + 3) + 2 * crate::MAX_ID_BYTES + 256;

/// Bytes reserved for the deferral and oversize key lists.
///
/// A deferred entry is a key, so the whole list is bounded by the query limit.
const DEFERRAL_RESERVE_BYTES: usize =
    (MAX_MCP_QUERY_LIMIT as usize) * (crate::MAX_ID_BYTES + 4) + 128;

// The response ceiling must actually be able to carry a full page. Getting this
// wrong is how a surface ends up permanently unable to answer a legitimate query,
// so it is a compile-time check rather than a comment.
const _: () = assert!(
    (MAX_MCP_QUERY_LIMIT as usize) * MAX_MCP_RECORD_PROJECTION_BYTES + DEFERRAL_RESERVE_BYTES
        <= MAX_MCP_SERIALIZED_RESULT_BYTES,
    "a full page of worst-case records must fit inside the response ceiling"
);

/// Host-owned boundary that derives trusted request context independently of MCP
/// input.
pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> Result<TrustedContextV1, MemoryError>;
}

/// Joins host-derived trusted identity with a verified closed policy decision.
pub struct MemoryPolicyContextResolver<T, P> {
    source: T,
    policy: P,
}

impl<T, P> MemoryPolicyContextResolver<T, P>
where
    T: TrustedContextSource,
    P: PolicyResolver,
{
    #[must_use]
    pub const fn new(source: T, policy: P) -> Self {
        Self { source, policy }
    }

    /// Derives the trusted tenant and proves the capability is granted.
    ///
    /// The returned [`MemoryContext`] is built from the **trusted** tenant, never
    /// from anything a caller sent, which is what makes a forged `tenant_id` in a
    /// request body inert rather than merely rejected.
    fn authorize(&self, capability: CapabilityV1) -> Result<MemoryContext, MemoryError> {
        // A failure to establish identity is a refusal, not a backend fault.
        let trusted = self
            .source
            .resolve()
            .map_err(|_| MemoryError::Unauthorized)?;
        let request = AuthorizationRequestV1 {
            context: trusted.clone(),
            capability,
        };
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest: supplied_digest,
        } = self.policy.authorize(request.clone())
        else {
            return Err(MemoryError::Unauthorized);
        };
        let effective_grant =
            canonical_grant(&effective_grant).map_err(|_| MemoryError::Unauthorized)?;
        // The grant flag is checked as well as the capability. `workflow` and
        // `agent` both honour `memory_enabled`, so a surface that ignored it would
        // be a way around a ceiling those bricks already enforce.
        // Verified before it is acted on. `memory_enabled` is inside the digest's
        // canonical bytes, so a flag flipped after the decision was signed changes
        // the expected digest and is caught here rather than trusted.
        let expected_digest = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant: effective_grant.clone(),
                decision_digest: String::new(),
            },
        )
        .map_err(|_| MemoryError::Unauthorized)?;
        if supplied_digest != expected_digest {
            return Err(MemoryError::Unauthorized);
        }
        // Only now, on a verified decision, is the grant flag consulted.
        // `workflow` and `agent` both honour `memory_enabled`, so a surface that
        // ignored it would be a way around a ceiling those bricks enforce.
        if !effective_grant.memory_enabled {
            return Err(MemoryError::Unauthorized);
        }
        let tenant_id =
            TenantId::new(trusted.tenant_id.as_str()).map_err(|_| MemoryError::Unauthorized)?;
        Ok(MemoryContext::new(tenant_id))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKindInput {
    Factual,
    Preference,
    Procedural,
    Episodic,
}

impl From<MemoryKindInput> for MemoryKind {
    fn from(value: MemoryKindInput) -> Self {
        match value {
            MemoryKindInput::Factual => Self::Factual,
            MemoryKindInput::Preference => Self::Preference,
            MemoryKindInput::Procedural => Self::Procedural,
            MemoryKindInput::Episodic => Self::Episodic,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RememberInput {
    pub namespace: String,
    pub key: String,
    pub kind: MemoryKindInput,
    pub content: String,
    pub tags: Vec<String>,
    pub run_id: Option<String>,
}

impl RememberInput {
    fn into_core(self) -> Result<RememberRequest, MemoryError> {
        Ok(RememberRequest {
            namespace: Namespace::new(self.namespace)?,
            key: RecordKey::new(self.key)?,
            kind: self.kind.into(),
            content: self.content,
            tags: self.tags,
            // Metadata is not accepted over MCP in V1: it is an opaque map with no
            // core meaning, and admitting it multiplies the worst-case frame for
            // no capability a caller cannot get from tags.
            metadata: crate::Metadata::new(),
            run_id: self.run_id.map(RunId::new).transpose()?,
        })
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RecordAddressInput {
    pub namespace: String,
    pub key: String,
}

impl RecordAddressInput {
    fn into_core(self) -> Result<(Namespace, RecordKey), MemoryError> {
        Ok((Namespace::new(self.namespace)?, RecordKey::new(self.key)?))
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInput {
    pub namespace: String,
    pub kinds: Vec<MemoryKindInput>,
    pub tags: Vec<String>,
    pub term: Option<String>,
    pub since_micros: Option<u64>,
    pub until_micros: Option<u64>,
    pub limit: u32,
}

impl SearchInput {
    fn into_core(self) -> Result<(Namespace, MemoryQuery), MemoryError> {
        let namespace = Namespace::new(self.namespace)?;
        let query = MemoryQuery {
            kinds: self.kinds.into_iter().map(Into::into).collect(),
            tags: self.tags,
            term: self.term,
            since: self.since_micros.map(Timestamp::from_micros),
            until: self.until_micros.map(Timestamp::from_micros),
            limit: self.limit,
        };
        Ok((namespace, query))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatusInput {}

/// Safe per-record projection.
///
/// Omits `tenant_id` (never a caller's business — it is derived, not supplied),
/// `metadata` (opaque, unbounded in shape), and `provenance.run_id` (identifies
/// another actor's run). `recorded_at` is projected because a caller filtering on
/// a time window needs to interpret its own results.
#[derive(Serialize)]
struct RecordOutput<'a> {
    namespace: &'a str,
    key: &'a str,
    kind: &'static str,
    content: &'a str,
    tags: &'a [String],
    recorded_at_micros: Option<u64>,
}

pub struct MemoryMcp<S, C, T, P>
where
    S: MemoryStore,
    C: Clock,
    T: TrustedContextSource,
    P: PolicyResolver,
{
    service: MemoryService<S, C>,
    resolver: MemoryPolicyContextResolver<T, P>,
    tool_router: ToolRouter<Self>,
}

impl<S, C, T, P> MemoryMcp<S, C, T, P>
where
    S: MemoryStore + 'static,
    C: Clock + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[must_use]
    pub fn new(service: MemoryService<S, C>, resolver: MemoryPolicyContextResolver<T, P>) -> Self {
        Self {
            service,
            resolver,
            tool_router: Self::tool_router(),
        }
    }

    /// # Ordering
    ///
    /// Only transport ceilings run before the policy gate; every semantic check
    /// runs after it. That split is deliberate and narrow: a ceiling reveals
    /// nothing an attacker does not already know from the published schema, but a
    /// validation *result* is an oracle — an unauthorized caller who can tell
    /// `invalid_id` from a refusal has learned that its request reached the
    /// validator, which is a probe this surface should not answer.
    ///
    /// Core record validation therefore happens inside
    /// [`MemoryService::remember`], after authorization, and identifier parsing is
    /// deferred to match. A record cannot be built before the trusted tenant is
    /// known anyway, and the tenant is only known once the gate has run.
    fn remember_json(&self, input: RememberInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        validate_mcp_record(&input).map_err(public_error)?;
        let context = self
            .resolver
            .authorize(CapabilityV1::MemoryRemember)
            .map_err(public_error)?;
        let request = input.into_core().map_err(public_error)?;
        let outcome = self
            .service
            .remember(&context, request)
            .map_err(public_error)?;
        // Fixed shape and infallible: a post-effect serialization failure would
        // report failure for a write that already applied.
        Ok(match outcome {
            WriteOutcome::Created => r#"{"outcome":"created"}"#.to_owned(),
            WriteOutcome::Replaced => r#"{"outcome":"replaced"}"#.to_owned(),
        })
    }

    fn recall_json(&self, input: RecordAddressInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        let context = self
            .resolver
            .authorize(CapabilityV1::MemoryRecall)
            .map_err(public_error)?;
        let (namespace, key) = input.into_core().map_err(public_error)?;
        let found = self
            .service
            .recall(&context, &namespace, &key)
            .map_err(public_error)?;
        match found {
            // Absence is reported as absence, and only as absence.
            None => Ok(r#"{"record":null}"#.to_owned()),
            Some(record) => {
                let output = record_output(&record);
                // Refused rather than truncated. A record that exists but cannot
                // be projected must not be reported as missing.
                serialize(&json!({"record": output}))
            }
        }
    }

    fn search_json(&self, input: SearchInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        // The MCP limit is stricter than the core's, so this is checked here
        // rather than left to the service.
        if input.limit == 0 || input.limit > MAX_MCP_QUERY_LIMIT {
            return Err(public_error(MemoryError::LimitExceeded));
        }
        let context = self
            .resolver
            .authorize(CapabilityV1::MemorySearch)
            .map_err(public_error)?;
        let (namespace, query) = input.into_core().map_err(public_error)?;
        let records = self
            .service
            .search(&context, &namespace, &query)
            .map_err(public_error)?;
        serialize_search(&records)
    }

    fn forget_json(&self, input: RecordAddressInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        let context = self
            .resolver
            .authorize(CapabilityV1::MemoryForget)
            .map_err(public_error)?;
        let (namespace, key) = input.into_core().map_err(public_error)?;
        let forgotten = self
            .service
            .forget(&context, &namespace, &key)
            .map_err(public_error)?;
        // Fixed shape and infallible, for the same reason as `remember_json`.
        Ok(if forgotten {
            r#"{"forgotten":true}"#.to_owned()
        } else {
            r#"{"forgotten":false}"#.to_owned()
        })
    }

    fn status_json(&self, input: StatusInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        let _context = self
            .resolver
            .authorize(CapabilityV1::MemoryStatus)
            .map_err(public_error)?;
        let guarantees = self.service.guarantees();
        // The configured backend is deliberately not named: a caller needs to know
        // what is guaranteed, not which crate is behind it.
        serialize(&json!({
            "durable_across_restart": guarantees.durable_across_restart,
            "visible_across_processes": guarantees.visible_across_processes,
            "crash_atomic": guarantees.crash_atomic,
            "result_ceiling": self.service.result_ceiling().min(MAX_MCP_QUERY_LIMIT),
            "max_content_bytes": MAX_MCP_CONTENT_BYTES,
        }))
    }
}

#[tool_router(router = tool_router)]
impl<S, C, T, P> MemoryMcp<S, C, T, P>
where
    S: MemoryStore + 'static,
    C: Clock + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[tool(
        name = "memory_remember",
        description = "Record one tenant-scoped memory, replacing any existing record for the same key."
    )]
    async fn memory_remember(&self, Parameters(input): Parameters<RememberInput>) -> String {
        tool_response(self.remember_json(input))
    }

    #[tool(
        name = "memory_recall",
        description = "Read one tenant-scoped memory by namespace and key."
    )]
    async fn memory_recall(&self, Parameters(input): Parameters<RecordAddressInput>) -> String {
        tool_response(self.recall_json(input))
    }

    #[tool(
        name = "memory_search",
        description = "Search one tenant-scoped namespace with a bounded closed filter."
    )]
    async fn memory_search(&self, Parameters(input): Parameters<SearchInput>) -> String {
        tool_response(self.search_json(input))
    }

    #[tool(
        name = "memory_forget",
        description = "Remove one tenant-scoped memory by namespace and key."
    )]
    async fn memory_forget(&self, Parameters(input): Parameters<RecordAddressInput>) -> String {
        tool_response(self.forget_json(input))
    }

    #[tool(
        name = "memory_status",
        description = "Report the configured memory adapter guarantees and bounds."
    )]
    async fn memory_status(&self, Parameters(input): Parameters<StatusInput>) -> String {
        tool_response(self.status_json(input))
    }
}

#[tool_handler(router = self.tool_router)]
impl<S, C, T, P> ServerHandler for MemoryMcp<S, C, T, P>
where
    S: MemoryStore + 'static,
    C: Clock + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}

fn record_output(record: &MemoryRecord) -> RecordOutput<'_> {
    RecordOutput {
        namespace: record.namespace.as_str(),
        key: record.key.as_str(),
        kind: record.kind.as_str(),
        content: &record.content,
        tags: &record.tags,
        recorded_at_micros: record
            .provenance
            .as_ref()
            .map(|provenance| provenance.recorded_at.as_micros()),
    }
}

/// Applies the MCP-only record ceilings.
///
/// Separate from [`crate::validation::validate_record`] because these are
/// transport limits, not domain rules: a record failing only these is still a
/// perfectly valid memory, just too large to carry in a frame.
fn validate_mcp_record(input: &RememberInput) -> Result<(), MemoryError> {
    // Serialized, not raw. See MAX_MCP_CONTENT_BYTES: escaping can sextuple a
    // control-character payload, so the raw length bounds nothing.
    if serialized_len(&input.content)? > MAX_MCP_CONTENT_BYTES {
        return Err(MemoryError::LimitExceeded);
    }
    if input.tags.len() > MAX_MCP_TAGS {
        return Err(MemoryError::LimitExceeded);
    }
    for tag in &input.tags {
        if serialized_len(tag)? > MAX_MCP_TAG_BYTES {
            return Err(MemoryError::LimitExceeded);
        }
    }
    Ok(())
}

/// Serialized JSON length of one string, including its quotes.
fn serialized_len(value: &str) -> Result<usize, MemoryError> {
    serde_json::to_string(value)
        .map(|encoded| encoded.len())
        .map_err(|_| MemoryError::InvalidRecord)
}

/// Projects a page of records, deferring rather than dropping what will not fit.
///
/// # Why not simply refuse
///
/// Refusing the whole page is honest but leaves an agent no way forward: one
/// record written through the typed API at full core limits would make a
/// namespace permanently unsearchable, at every limit. Silently shortening the
/// list is worse — it is indistinguishable from there being no more data, which
/// is exactly what [`MemoryService::search`] refuses to do.
///
/// So neither. A record that does not fit in the remaining budget is named in
/// `deferred_keys`, and a record too large to project even alone is named in
/// `oversized_keys`. Both are explicit, so a caller always knows the page is
/// partial and always has a key to act on.
fn serialize_search(records: &[MemoryRecord]) -> Result<String> {
    let budget = MAX_MCP_SERIALIZED_RESULT_BYTES - DEFERRAL_RESERVE_BYTES;
    let mut outputs = Vec::new();
    let mut used = 0;
    let mut deferred = Vec::new();
    let mut oversized = Vec::new();

    for record in records {
        let output = record_output(record);
        let size = serde_json::to_string(&output)
            .context("could not serialize MCP response")?
            .len()
            + 1;
        if size > budget {
            // Unprojectable on its own, so no smaller page would ever carry it.
            oversized.push(record.key.as_str());
        } else if used + size > budget {
            deferred.push(record.key.as_str());
        } else {
            used += size;
            outputs.push(output);
        }
    }

    serialize(&json!({
        "records": outputs,
        "deferred_keys": deferred,
        "oversized_keys": oversized,
    }))
}

fn serialize(value: &serde_json::Value) -> Result<String> {
    let value = serde_json::to_string(value).context("could not serialize MCP response")?;
    if value.len() > MAX_MCP_SERIALIZED_RESULT_BYTES {
        return Err(anyhow::anyhow!("limit_exceeded"));
    }
    // Measured, not estimated. The raw ceiling above assumes the worst case; this
    // checks what the transport will actually write, which for ordinary text is
    // far smaller. Both bounds exist because the first keeps the page arithmetic
    // predictable and the second is the one that is actually true.
    let framed = framed_len(&value)?;
    (framed <= MAX_MCP_ESCAPED_TOOL_TEXT_BYTES)
        .then_some(value)
        .ok_or_else(|| anyhow::anyhow!("limit_exceeded"))
}

/// Length of `value` once escaped into the tool result's `text` field.
fn framed_len(value: &str) -> Result<usize> {
    serde_json::to_string(value)
        .map(|escaped| escaped.len())
        .context("could not measure MCP response")
}

fn validate_request<T: Serialize>(input: &T) -> Result<(), MemoryError> {
    let bytes = serde_json::to_vec(input).map_err(|_| MemoryError::InvalidQuery)?;
    (bytes.len() <= MAX_MCP_REQUEST_BYTES)
        .then_some(())
        .ok_or(MemoryError::LimitExceeded)
}

fn public_error(error: MemoryError) -> anyhow::Error {
    anyhow::anyhow!(public_code(error))
}

fn public_code(error: MemoryError) -> &'static str {
    // Sourced from the core's own projection, so `TenantMismatch` arrives here
    // already collapsed to `not_found`.
    match error.public_code() {
        PublicErrorCode::InvalidId => "invalid_id",
        PublicErrorCode::InvalidRecord => "invalid_record",
        PublicErrorCode::InvalidQuery => "invalid_query",
        PublicErrorCode::LimitExceeded => "limit_exceeded",
        // Merged deliberately. The core already collapses `TenantMismatch` into
        // `NotFound` before it reaches here, so this arm is belt and braces: even
        // if a future path produced the code directly, the boundary would still
        // refuse to distinguish a foreign record from a missing one.
        PublicErrorCode::NotFound | PublicErrorCode::TenantMismatch => "not_found",
        // Distinct from `adapter_failure` on purpose: a caller must be able to
        // tell a permanent refusal from a transient fault, or it retries forever.
        PublicErrorCode::Unauthorized => "unauthorized",
        PublicErrorCode::AdapterFailure => "adapter_failure",
    }
}

fn tool_response(response: Result<String>) -> String {
    response.unwrap_or_else(|error| {
        let code = error.to_string();
        json!({"error": if matches!(
            code.as_str(),
            "invalid_id"
                | "invalid_record"
                | "invalid_query"
                | "limit_exceeded"
                | "not_found"
                | "unauthorized"
                | "adapter_failure"
        ) {
            code.as_str()
        } else {
            "adapter_failure"
        }})
        .to_string()
    })
}

/// Returns the configured MCP tool names in stable order.
#[must_use]
pub const fn tool_names() -> [&'static str; 5] {
    MEMORY_TOOLS
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use policy::{
        CorrelationId, GrantV1, PrincipalId, RequestId, TenantId as PolicyTenantId, allow_decision,
        deny_decision,
    };
    use serde_json::Value;

    use super::*;

    #[derive(Clone)]
    struct Source(Result<TrustedContextV1, MemoryError>);

    impl TrustedContextSource for Source {
        fn resolve(&self) -> Result<TrustedContextV1, MemoryError> {
            self.0.clone()
        }
    }

    /// How a decision is corrupted, if at all.
    ///
    /// `Digest` replaces the signature outright. `FlagAfterSigning` is the subtler
    /// one: the grant is signed with `memory_enabled = true` and the flag is then
    /// flipped to `false`, so flag and digest disagree. That is the case a gate
    /// which trusted the flag before verifying the digest would mishandle.
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Tamper {
        None,
        Digest,
        FlagAfterSigning,
    }

    #[derive(Clone)]
    struct Policy {
        allow: bool,
        tamper: Tamper,
        memory_enabled: bool,
        calls: Arc<Mutex<Vec<CapabilityV1>>>,
    }

    impl PolicyResolver for Policy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            self.calls.lock().expect("calls").push(request.capability);
            if !self.allow {
                return deny_decision();
            }
            let signed_flag = self.memory_enabled || self.tamper == Tamper::FlagAfterSigning;
            let grant = GrantV1::new(Vec::<String>::new(), signed_flag, false, false, false)
                .expect("grant");
            let mut decision = allow_decision(&request, &grant).expect("decision");
            let AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest,
            } = &mut decision
            else {
                unreachable!()
            };
            match self.tamper {
                Tamper::None => {}
                Tamper::Digest => *decision_digest = "tampered".to_owned(),
                // Signed as enabled, delivered as disabled.
                Tamper::FlagAfterSigning => effective_grant.memory_enabled = false,
            }
            decision
        }
    }

    /// A self-contained store that records every tenant that reached it.
    ///
    /// Deliberately not the `local` adapter: keeping this module independent of
    /// the `local` feature is what makes `--features mcp` alone a valid build,
    /// which the per-feature lint and test lines in the Makefile check.
    /// Tenant, namespace, and key, in that order so the tuple sorts by tenant.
    type Slot = (String, String, String);

    #[derive(Clone, Default)]
    struct Recording {
        records: Arc<Mutex<BTreeMap<Slot, MemoryRecord>>>,
        tenants: Arc<Mutex<Vec<String>>>,
    }

    impl Recording {
        fn note(&self, tenant: &str) {
            self.tenants
                .lock()
                .expect("tenants")
                .push(tenant.to_owned());
        }

        fn slot(tenant: &TenantId, namespace: &Namespace, key: &RecordKey) -> Slot {
            (
                tenant.as_str().to_owned(),
                namespace.as_str().to_owned(),
                key.as_str().to_owned(),
            )
        }
    }

    impl MemoryStore for Recording {
        fn put(&self, record: MemoryRecord) -> Result<WriteOutcome, MemoryError> {
            crate::validation::validate_record(&record)?;
            let slot = Self::slot(&record.tenant_id, &record.namespace, &record.key);
            // Contract clause 8. A test double that skipped this would not be a
            // conformant adapter, and every capacity assertion above it would be
            // testing the double rather than the surface.
            {
                let records = self.records.lock().expect("records");
                let tenant = record.tenant_id.as_str();
                let namespaces = records
                    .keys()
                    .filter(|(held, _, _)| held == tenant)
                    .map(|(_, namespace, _)| namespace.as_str())
                    .collect::<std::collections::BTreeSet<_>>();
                let partition = records
                    .keys()
                    .filter(|(held, namespace, _)| {
                        held == tenant && namespace == record.namespace.as_str()
                    })
                    .count();
                crate::validation::check_capacity(
                    namespaces.len(),
                    !namespaces.contains(record.namespace.as_str()),
                    partition,
                    !records.contains_key(&slot),
                )?;
            }
            self.note(record.tenant_id.as_str());
            Ok(
                if self
                    .records
                    .lock()
                    .expect("records")
                    .insert(slot, record)
                    .is_some()
                {
                    WriteOutcome::Replaced
                } else {
                    WriteOutcome::Created
                },
            )
        }

        fn get(
            &self,
            tenant_id: &TenantId,
            namespace: &Namespace,
            key: &RecordKey,
        ) -> Result<Option<MemoryRecord>, MemoryError> {
            self.note(tenant_id.as_str());
            Ok(self
                .records
                .lock()
                .expect("records")
                .get(&Self::slot(tenant_id, namespace, key))
                .cloned())
        }

        fn query(
            &self,
            tenant_id: &TenantId,
            namespace: &Namespace,
            query: &MemoryQuery,
        ) -> Result<Vec<MemoryRecord>, MemoryError> {
            crate::validation::validate_query(query)?;
            self.note(tenant_id.as_str());
            let tenant = tenant_id.as_str();
            let space = namespace.as_str();
            Ok(self
                .records
                .lock()
                .expect("records")
                .iter()
                .filter(|((held_tenant, held_space, _), _)| {
                    held_tenant == tenant && held_space == space
                })
                .map(|(_, record)| record)
                .filter(|record| query.matches(record))
                .take(query.limit as usize)
                .cloned()
                .collect())
        }

        fn delete(
            &self,
            tenant_id: &TenantId,
            namespace: &Namespace,
            key: &RecordKey,
        ) -> Result<bool, MemoryError> {
            self.note(tenant_id.as_str());
            Ok(self
                .records
                .lock()
                .expect("records")
                .remove(&Self::slot(tenant_id, namespace, key))
                .is_some())
        }

        fn guarantees(&self) -> crate::StoreGuarantees {
            crate::StoreGuarantees::in_process()
        }
    }

    /// A clock frozen at a known instant, so provenance is assertable.
    #[derive(Clone, Copy)]
    struct FixedClock(u64);

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            Timestamp::from_micros(self.0)
        }
    }

    type Mcp = MemoryMcp<Recording, FixedClock, Source, Policy>;

    fn trusted() -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: PolicyTenantId::new("trusted-tenant").expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }

    /// Builds a server whose policy behaves as described.
    fn mcp(
        allow: bool,
        tamper: Tamper,
        memory_enabled: bool,
        source: Result<TrustedContextV1, MemoryError>,
    ) -> (Mcp, Recording, Arc<Mutex<Vec<CapabilityV1>>>) {
        let store = Recording::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let resolver = MemoryPolicyContextResolver::new(
            Source(source),
            Policy {
                allow,
                tamper,
                memory_enabled,
                calls: Arc::clone(&calls),
            },
        );
        let server = MemoryMcp::new(MemoryService::new(store.clone(), FixedClock(7)), resolver);
        (server, store, calls)
    }

    fn allowed() -> (Mcp, Recording, Arc<Mutex<Vec<CapabilityV1>>>) {
        mcp(true, Tamper::None, true, Ok(trusted()))
    }

    /// Two servers over one store, each with its own trusted tenant.
    ///
    /// The only way to test isolation at this boundary: a caller cannot name a
    /// tenant, so a second tenant must arrive through a second trusted source.
    fn two_tenants() -> (Mcp, Mcp) {
        let store = Recording::default();
        let build = |tenant: &str| {
            let mut context = trusted();
            context.tenant_id = PolicyTenantId::new(tenant).expect("tenant");
            let resolver = MemoryPolicyContextResolver::new(
                Source(Ok(context)),
                Policy {
                    allow: true,
                    tamper: Tamper::None,
                    memory_enabled: true,
                    calls: Arc::new(Mutex::new(vec![])),
                },
            );
            MemoryMcp::new(MemoryService::new(store.clone(), FixedClock(7)), resolver)
        };
        (build("tenant-a"), build("tenant-b"))
    }

    fn remember_input() -> RememberInput {
        RememberInput {
            namespace: "notes".to_owned(),
            key: "k".to_owned(),
            kind: MemoryKindInput::Factual,
            content: "content".to_owned(),
            tags: vec!["ci".to_owned()],
            run_id: Some("run-1".to_owned()),
        }
    }

    fn address() -> RecordAddressInput {
        RecordAddressInput {
            namespace: "notes".to_owned(),
            key: "k".to_owned(),
        }
    }

    fn search_input(limit: u32) -> SearchInput {
        SearchInput {
            namespace: "notes".to_owned(),
            kinds: vec![],
            tags: vec![],
            term: None,
            since_micros: None,
            until_micros: None,
            limit,
        }
    }

    #[test]
    fn exact_tools_and_closed_schemas_expose_no_identity_or_policy_fields() {
        assert_eq!(
            tool_names(),
            [
                "memory_remember",
                "memory_recall",
                "memory_search",
                "memory_forget",
                "memory_status"
            ]
        );
        for schema in [
            serde_json::to_value(schemars::schema_for!(RememberInput)).expect("remember schema"),
            serde_json::to_value(schemars::schema_for!(RecordAddressInput))
                .expect("address schema"),
            serde_json::to_value(schemars::schema_for!(SearchInput)).expect("search schema"),
            serde_json::to_value(schemars::schema_for!(StatusInput)).expect("status schema"),
        ] {
            assert_eq!(schema["additionalProperties"], false);
            let encoded = schema.to_string();
            for prohibited in [
                "tenant_id",
                "principal_id",
                "request_id",
                "correlation_id",
                "grant",
                "decision_digest",
                "policy",
            ] {
                assert!(!encoded.contains(prohibited), "schema exposed {prohibited}");
            }
        }
        // A forged tenant is not merely ignored; the request fails to decode.
        assert!(
            serde_json::from_value::<RecordAddressInput>(
                json!({"namespace":"notes","key":"k","tenant_id":"forged"})
            )
            .is_err()
        );

        assert_eq!(
            Mcp::memory_remember_tool_attr().name.as_ref(),
            "memory_remember"
        );
        assert_eq!(
            Mcp::memory_status_tool_attr().input_schema["additionalProperties"],
            false
        );
    }

    #[test]
    fn no_tool_reaches_the_store_without_a_verified_authorization() {
        // Five refusal modes. `FlagAfterSigning` is the one a gate that trusted
        // the grant before verifying the digest would let through, and
        // `memory_enabled = false` is the one a capability-only gate would.
        for (allow, tamper, memory_enabled, source) in [
            (false, Tamper::None, true, Ok(trusted())),
            (true, Tamper::Digest, true, Ok(trusted())),
            (true, Tamper::FlagAfterSigning, false, Ok(trusted())),
            (true, Tamper::None, false, Ok(trusted())),
            (true, Tamper::None, true, Err(MemoryError::Unauthorized)),
        ] {
            let (server, store, policy_calls) = mcp(allow, tamper, memory_enabled, source.clone());
            // Every tool, not only the mutating ones: a read is where an
            // authorization miss actually leaks data.
            let refusals = [
                tool_response(server.remember_json(remember_input())),
                tool_response(server.recall_json(address())),
                tool_response(server.search_json(search_input(4))),
                tool_response(server.forget_json(address())),
                tool_response(server.status_json(StatusInput {})),
            ];
            for refusal in &refusals {
                assert_eq!(
                    refusal, r#"{"error":"unauthorized"}"#,
                    "a refusal must be reported as a refusal, never as a backend fault"
                );
            }
            assert!(
                store.tenants.lock().expect("tenants").is_empty(),
                "no store call may happen without authorization"
            );
            if source.is_ok() {
                assert_eq!(
                    policy_calls.lock().expect("policy").as_slice(),
                    &[
                        CapabilityV1::MemoryRemember,
                        CapabilityV1::MemoryRecall,
                        CapabilityV1::MemorySearch,
                        CapabilityV1::MemoryForget,
                        CapabilityV1::MemoryStatus,
                    ],
                    "each tool must ask for exactly its own capability even when refused"
                );
            } else {
                assert!(
                    policy_calls.lock().expect("policy").is_empty(),
                    "policy is not consulted when identity cannot be established"
                );
            }
        }
    }

    #[test]
    fn a_refusal_is_distinguishable_from_a_backend_fault() {
        // An agent that cannot tell these apart retries forever on a capability it
        // will never hold.
        assert_eq!(public_code(MemoryError::Unauthorized), "unauthorized");
        assert_eq!(public_code(MemoryError::AdapterFailure), "adapter_failure");
        assert_ne!(
            public_code(MemoryError::Unauthorized),
            public_code(MemoryError::AdapterFailure)
        );
    }

    #[test]
    fn one_tenant_cannot_observe_another_through_any_tool() {
        let (alpha, beta) = two_tenants();
        alpha
            .remember_json(remember_input())
            .expect("tenant a writes");

        // Absence, empty, and false — never the record, and never a refusal that
        // would confirm the key exists elsewhere.
        assert_eq!(
            beta.recall_json(address()).expect("recall"),
            r#"{"record":null}"#
        );
        let searched: Value =
            serde_json::from_str(&beta.search_json(search_input(4)).expect("search"))
                .expect("JSON");
        assert_eq!(searched["records"].as_array().expect("records").len(), 0);
        assert_eq!(
            beta.forget_json(address()).expect("forget"),
            r#"{"forgotten":false}"#
        );

        // And the owner's record survived the other tenant's delete attempt.
        let owned: Value =
            serde_json::from_str(&alpha.recall_json(address()).expect("recall")).expect("JSON");
        assert_eq!(owned["record"]["content"], "content");
    }

    #[test]
    fn an_allowed_write_uses_the_trusted_tenant_and_reports_a_fixed_outcome() {
        let (server, store, policy_calls) = allowed();
        assert_eq!(
            server.remember_json(remember_input()).expect("remember"),
            r#"{"outcome":"created"}"#
        );
        // Writing the same key again replaces rather than duplicating.
        assert_eq!(
            server.remember_json(remember_input()).expect("remember"),
            r#"{"outcome":"replaced"}"#
        );
        assert_eq!(
            store.tenants.lock().expect("tenants").as_slice(),
            &["trusted-tenant", "trusted-tenant"],
            "the tenant reaching the store is the trusted one, never a caller's"
        );
        assert_eq!(
            policy_calls.lock().expect("policy").as_slice(),
            &[CapabilityV1::MemoryRemember, CapabilityV1::MemoryRemember]
        );
    }

    #[test]
    fn each_tool_requests_exactly_its_own_capability() {
        let (server, _store, policy_calls) = allowed();
        server.remember_json(remember_input()).expect("remember");
        server.recall_json(address()).expect("recall");
        server.search_json(search_input(4)).expect("search");
        server.forget_json(address()).expect("forget");
        server.status_json(StatusInput {}).expect("status");
        assert_eq!(
            policy_calls.lock().expect("policy").as_slice(),
            &[
                CapabilityV1::MemoryRemember,
                CapabilityV1::MemoryRecall,
                CapabilityV1::MemorySearch,
                CapabilityV1::MemoryForget,
                CapabilityV1::MemoryStatus,
            ]
        );
    }

    #[test]
    fn absence_is_reported_as_absence_and_a_round_trip_projects_safely() {
        let (server, _store, _calls) = allowed();
        assert_eq!(
            server.recall_json(address()).expect("recall"),
            r#"{"record":null}"#
        );

        server.remember_json(remember_input()).expect("remember");
        let value: Value =
            serde_json::from_str(&server.recall_json(address()).expect("recall")).expect("JSON");
        let record = &value["record"];
        assert_eq!(record["namespace"], "notes");
        assert_eq!(record["key"], "k");
        assert_eq!(record["kind"], "factual");
        assert_eq!(record["content"], "content");
        assert_eq!(record["recorded_at_micros"], 7, "the clock stamped it");
        // The projection carries neither the derived tenant nor the originating
        // run, and never invents a metadata field.
        let encoded = value.to_string();
        assert!(!encoded.contains("trusted-tenant"));
        assert!(!encoded.contains("run-1"));
        assert!(record.get("tenant_id").is_none());
        assert!(record.get("metadata").is_none());
        assert!(record.get("run_id").is_none());
    }

    #[test]
    fn forget_reports_whether_a_record_existed() {
        let (server, _store, _calls) = allowed();
        assert_eq!(
            server.forget_json(address()).expect("forget"),
            r#"{"forgotten":false}"#
        );
        server.remember_json(remember_input()).expect("remember");
        assert_eq!(
            server.forget_json(address()).expect("forget"),
            r#"{"forgotten":true}"#
        );
    }

    #[test]
    fn the_mcp_surface_accepts_a_strict_subset_of_core_valid_records() {
        // A record this large is perfectly valid to the core and cannot be carried
        // in a transport frame, so the surface must refuse it rather than let the
        // session die.
        const _: () = assert!(
            MAX_MCP_CONTENT_BYTES < crate::MAX_CONTENT_BYTES,
            "the MCP ceiling must be stricter than the core's"
        );
        let (server, store, policy_calls) = allowed();
        let oversized = RememberInput {
            content: "a".repeat(MAX_MCP_CONTENT_BYTES + 1),
            ..remember_input()
        };
        assert_eq!(
            tool_response(server.remember_json(oversized)),
            r#"{"error":"limit_exceeded"}"#
        );
        // A transport ceiling is checked before the policy gate, so an
        // unauthorized caller cannot use an oversized payload to probe anything.
        assert!(store.tenants.lock().expect("tenants").is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());

        // And the whole-request ceiling is byte exact.
        assert!(validate_request(&"x".repeat(MAX_MCP_REQUEST_BYTES - 2)).is_ok());
        assert_eq!(
            validate_request(&"x".repeat(MAX_MCP_REQUEST_BYTES - 1)),
            Err(MemoryError::LimitExceeded)
        );
    }

    #[test]
    fn the_search_limit_is_stricter_than_the_cores_and_is_never_clamped() {
        const _: () = assert!(
            MAX_MCP_QUERY_LIMIT < crate::MAX_QUERY_LIMIT,
            "the MCP limit must be stricter than the core's"
        );
        let (server, store, policy_calls) = allowed();
        for refused in [0, MAX_MCP_QUERY_LIMIT + 1, crate::MAX_QUERY_LIMIT] {
            assert_eq!(
                tool_response(server.search_json(search_input(refused))),
                r#"{"error":"limit_exceeded"}"#,
                "a limit of {refused} must be refused, not silently reduced"
            );
        }
        assert!(store.tenants.lock().expect("tenants").is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());
        assert!(
            server
                .search_json(search_input(MAX_MCP_QUERY_LIMIT))
                .is_ok()
        );
    }

    #[test]
    fn an_unprojectable_record_fails_rather_than_reporting_absence() {
        // The typed API admits a record far larger than a frame. Reading it back
        // must be an error: reporting `null` would tell a caller the data is gone.
        let (server, _store, _calls) = allowed();
        write_bulky(&server, "bulky", crate::MAX_CONTENT_BYTES);
        assert_eq!(
            tool_response(server.recall_json(RecordAddressInput {
                namespace: "notes".to_owned(),
                key: "bulky".to_owned(),
            })),
            r#"{"error":"limit_exceeded"}"#,
            "an oversized record must not be reported as missing"
        );
    }

    /// Writes past the MCP ceilings through the typed API, which is exactly how a
    /// record the MCP surface cannot project comes to exist.
    fn write_bulky(server: &Mcp, key: &str, bytes: usize) {
        write_bulky_content(server, key, &"a".repeat(bytes));
    }

    #[test]
    fn a_full_page_of_worst_case_records_fits_the_response_ceiling() {
        // The regression this guards: `MAX_MCP_QUERY_LIMIT` and the response
        // ceiling were once chosen independently, so a handful of legitimately
        // written records made search fail permanently. Now they are consistent by
        // construction, and this proves it against real serialization rather than
        // arithmetic.
        let (server, _store, _calls) = allowed();
        let content_bytes = MAX_MCP_CONTENT_BYTES - 2; // less the JSON quotes
        for index in 0..MAX_MCP_QUERY_LIMIT {
            server
                .remember_json(RememberInput {
                    key: format!("k-{index}"),
                    content: "a".repeat(content_bytes),
                    tags: (0..MAX_MCP_TAGS)
                        .map(|tag| format!("{}{tag}", "t".repeat(MAX_MCP_TAG_BYTES - 3)))
                        .collect(),
                    ..remember_input()
                })
                .expect("each record is within the MCP ceilings");
        }

        let response = server
            .search_json(search_input(MAX_MCP_QUERY_LIMIT))
            .expect("a full page must not overflow the ceiling");
        assert!(response.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES);
        let value: Value = serde_json::from_str(&response).expect("JSON");
        assert_eq!(
            value["records"].as_array().expect("records").len(),
            MAX_MCP_QUERY_LIMIT as usize,
            "every record must be returned, none deferred"
        );
        assert!(
            value["deferred_keys"]
                .as_array()
                .expect("deferred")
                .is_empty()
        );
        assert!(
            value["oversized_keys"]
                .as_array()
                .expect("oversized")
                .is_empty()
        );
    }

    #[test]
    fn an_oversized_record_is_named_rather_than_blocking_the_whole_namespace() {
        // One record written through the typed API used to make a namespace
        // permanently unsearchable at every limit. Now it is named explicitly and
        // the rest of the page still comes back.
        let (server, _store, _calls) = allowed();
        server
            .remember_json(remember_input())
            .expect("small record");
        write_bulky(&server, "zz-bulky", crate::MAX_CONTENT_BYTES);

        let value: Value =
            serde_json::from_str(&server.search_json(search_input(4)).expect("search"))
                .expect("JSON");
        let records = value["records"].as_array().expect("records");
        assert_eq!(records.len(), 1, "the projectable record still comes back");
        assert_eq!(records[0]["key"], "k");
        assert_eq!(
            value["oversized_keys"].as_array().expect("oversized"),
            &[Value::from("zz-bulky")],
            "the unprojectable record is named, not silently dropped"
        );
    }

    #[test]
    fn a_page_that_does_not_fit_defers_explicitly_rather_than_shortening_silently() {
        let (server, _store, _calls) = allowed();
        // Each record fits alone but not all three in one page: roughly 12 KiB
        // against a budget a little under 32 KiB.
        for index in 0..3 {
            write_bulky(&server, &format!("k-{index}"), 12 * 1024);
        }
        let value: Value =
            serde_json::from_str(&server.search_json(search_input(3)).expect("search"))
                .expect("JSON");
        let returned = value["records"].as_array().expect("records").len();
        let deferred = value["deferred_keys"].as_array().expect("deferred").len();
        assert_eq!(
            returned + deferred,
            3,
            "every matched record is either returned or named as deferred"
        );
        assert!(deferred > 0, "the page must not silently shorten");
    }

    #[test]
    fn search_projects_matching_records_safely_and_honours_every_filter() {
        let (server, _store, _calls) = allowed();
        server
            .remember_json(RememberInput {
                key: "fact".to_owned(),
                kind: MemoryKindInput::Factual,
                content: "alpha".to_owned(),
                tags: vec!["red".to_owned()],
                ..remember_input()
            })
            .expect("write");
        server
            .remember_json(RememberInput {
                key: "pref".to_owned(),
                kind: MemoryKindInput::Preference,
                content: "beta".to_owned(),
                tags: vec!["red".to_owned(), "blue".to_owned()],
                ..remember_input()
            })
            .expect("write");

        let keys = |input: SearchInput| {
            let value: Value =
                serde_json::from_str(&server.search_json(input).expect("search")).expect("JSON");
            value["records"]
                .as_array()
                .expect("records")
                .iter()
                .map(|record| record["key"].as_str().expect("key").to_owned())
                .collect::<Vec<_>>()
        };

        assert_eq!(keys(search_input(4)), vec!["fact", "pref"], "key order");
        assert_eq!(
            keys(SearchInput {
                kinds: vec![MemoryKindInput::Preference],
                ..search_input(4)
            }),
            vec!["pref"]
        );
        assert_eq!(
            keys(SearchInput {
                tags: vec!["red".to_owned(), "blue".to_owned()],
                ..search_input(4)
            }),
            vec!["pref"],
            "tags are conjunctive through the boundary too"
        );
        assert_eq!(
            keys(SearchInput {
                term: Some("alpha".to_owned()),
                ..search_input(4)
            }),
            vec!["fact"]
        );
        assert_eq!(
            keys(SearchInput {
                since_micros: Some(7),
                until_micros: Some(8),
                ..search_input(4)
            }),
            vec!["fact", "pref"],
            "the stamped clock value falls inside this window"
        );
        assert!(
            keys(SearchInput {
                limit: 1,
                ..search_input(1)
            })
            .len()
                == 1
        );

        // The search projection is as safe as the recall projection.
        let encoded = server.search_json(search_input(4)).expect("search");
        assert!(!encoded.contains("trusted-tenant"));
        assert!(!encoded.contains("run-1"));
        assert!(!encoded.contains("metadata"));
    }

    #[test]
    fn an_invalid_query_is_reported_as_such_through_the_boundary() {
        let (server, _store, _calls) = allowed();
        for (input, expected) in [
            (
                SearchInput {
                    term: Some(String::new()),
                    ..search_input(4)
                },
                r#"{"error":"invalid_query"}"#,
            ),
            (
                SearchInput {
                    since_micros: Some(10),
                    until_micros: Some(10),
                    ..search_input(4)
                },
                r#"{"error":"invalid_query"}"#,
            ),
            (
                SearchInput {
                    kinds: vec![MemoryKindInput::Factual, MemoryKindInput::Factual],
                    ..search_input(4)
                },
                r#"{"error":"invalid_query"}"#,
            ),
            (
                SearchInput {
                    namespace: "Bad Namespace".to_owned(),
                    ..search_input(4)
                },
                r#"{"error":"invalid_id"}"#,
            ),
        ] {
            assert_eq!(tool_response(server.search_json(input)), expected);
        }
    }

    #[test]
    fn a_malformed_identifier_or_record_is_reported_after_authorization() {
        let (server, _store, _calls) = allowed();
        for bad in ["Bad Namespace", "", "has space"] {
            assert_eq!(
                tool_response(server.recall_json(RecordAddressInput {
                    namespace: bad.to_owned(),
                    key: "k".to_owned(),
                })),
                r#"{"error":"invalid_id"}"#
            );
            assert_eq!(
                tool_response(server.forget_json(RecordAddressInput {
                    namespace: "notes".to_owned(),
                    key: bad.to_owned(),
                })),
                r#"{"error":"invalid_id"}"#
            );
        }
        assert_eq!(
            tool_response(server.remember_json(RememberInput {
                run_id: Some("Bad Run".to_owned()),
                ..remember_input()
            })),
            r#"{"error":"invalid_id"}"#
        );
        // Empty content is a core rule, reached only after the gate.
        assert_eq!(
            tool_response(server.remember_json(RememberInput {
                content: String::new(),
                ..remember_input()
            })),
            r#"{"error":"invalid_record"}"#
        );
    }

    #[test]
    fn an_unauthorized_caller_cannot_tell_a_malformed_request_from_a_refusal() {
        // Semantic validation runs after the gate precisely so this holds: a
        // validation *result* would otherwise prove the request reached the
        // validator.
        let (server, _store, _calls) = mcp(false, Tamper::None, true, Ok(trusted()));
        assert_eq!(
            tool_response(server.recall_json(RecordAddressInput {
                namespace: "Bad Namespace".to_owned(),
                key: "k".to_owned(),
            })),
            r#"{"error":"unauthorized"}"#,
            "a malformed request from an unauthorized caller is simply unauthorized"
        );
    }

    #[test]
    fn status_reports_guarantees_and_bounds_without_naming_the_backend() {
        let (server, _store, _calls) = allowed();
        let status: Value =
            serde_json::from_str(&server.status_json(StatusInput {}).expect("status"))
                .expect("JSON");
        assert_eq!(status["durable_across_restart"], false);
        assert_eq!(status["visible_across_processes"], false);
        assert_eq!(status["crash_atomic"], false);
        assert_eq!(status["result_ceiling"], MAX_MCP_QUERY_LIMIT);
        assert_eq!(status["max_content_bytes"], MAX_MCP_CONTENT_BYTES);
        let encoded = status.to_string();
        for prohibited in ["agentic", "in_process", "InProcessStore", "backend"] {
            assert!(!encoded.contains(prohibited), "status leaked {prohibited}");
        }
    }

    /// Drives the five `#[tool]` functions themselves, not the `*_json` methods.
    ///
    /// Every other test calls `*_json` directly, so a tool wired to the wrong body
    /// — `memory_forget` calling `recall_json`, say — would pass all of them while
    /// destroying the per-tool capability model that five distinct `CapabilityV1`
    /// variants exist to provide. This is the only test that would catch it.
    #[test]
    fn every_tool_function_is_wired_to_its_own_body_and_capability() {
        // A tiny runtime rather than a dependency on tokio: these tools are
        // synchronous inside and never yield, which is itself part of the contract.
        fn drive<F: std::future::Future>(future: F) -> F::Output {
            use std::pin::pin;
            use std::sync::Arc;
            use std::task::{Context, Poll, Wake, Waker};

            struct Noop;
            impl Wake for Noop {
                fn wake(self: Arc<Self>) {}
            }
            let waker = Waker::from(Arc::new(Noop));
            let mut context = Context::from_waker(&waker);
            match pin!(future).poll(&mut context) {
                Poll::Ready(output) => output,
                Poll::Pending => {
                    panic!("a memory tool must not yield: every one of them is synchronous inside")
                }
            }
        }

        let (server, _store, policy_calls) = allowed();

        assert_eq!(
            drive(server.memory_remember(Parameters(remember_input()))),
            r#"{"outcome":"created"}"#
        );
        let recalled = drive(server.memory_recall(Parameters(address())));
        assert!(
            recalled.contains(r#""key":"k""#),
            "recall returned {recalled}"
        );
        let searched = drive(server.memory_search(Parameters(search_input(4))));
        assert!(
            searched.contains("records") && searched.contains(r#""key":"k""#),
            "search returned {searched}"
        );
        let status = drive(server.memory_status(Parameters(StatusInput {})));
        assert!(status.contains("durable_across_restart"));
        // Forget last, and it must actually delete rather than read.
        assert_eq!(
            drive(server.memory_forget(Parameters(address()))),
            r#"{"forgotten":true}"#
        );
        assert_eq!(
            drive(server.memory_recall(Parameters(address()))),
            r#"{"record":null}"#,
            "the forget tool must have deleted, not merely reported"
        );

        assert_eq!(
            policy_calls.lock().expect("policy").as_slice(),
            &[
                CapabilityV1::MemoryRemember,
                CapabilityV1::MemoryRecall,
                CapabilityV1::MemorySearch,
                CapabilityV1::MemoryStatus,
                CapabilityV1::MemoryForget,
                CapabilityV1::MemoryRecall,
            ],
            "each tool must request its own capability, in call order"
        );
    }

    #[test]
    fn the_declared_tool_names_match_the_router() {
        // `MEMORY_TOOLS` is hand-maintained; the router is generated. They must
        // agree or discovery advertises a tool that does not exist.
        let attributes = [
            Mcp::memory_remember_tool_attr(),
            Mcp::memory_recall_tool_attr(),
            Mcp::memory_search_tool_attr(),
            Mcp::memory_forget_tool_attr(),
            Mcp::memory_status_tool_attr(),
        ];
        let names = attributes
            .iter()
            .map(|attribute| attribute.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(names.as_slice(), &tool_names());
        for attribute in &attributes {
            assert_eq!(
                attribute.input_schema["additionalProperties"], false,
                "{} must have a closed schema",
                attribute.name
            );
        }
    }

    #[test]
    fn mcp_ingress_bounds_are_serialized_length_not_raw_length() {
        // A control character serializes to `\u00XX` — six bytes for one. Measuring
        // the raw string would let a caller smuggle six times the intended payload.
        let (server, _store, _calls) = allowed();
        let control_chars = "\u{1}".repeat(MAX_MCP_CONTENT_BYTES / 2);
        assert!(
            control_chars.len() < MAX_MCP_CONTENT_BYTES,
            "raw length is under the ceiling"
        );
        assert_eq!(
            tool_response(server.remember_json(RememberInput {
                content: control_chars,
                ..remember_input()
            })),
            r#"{"error":"limit_exceeded"}"#,
            "the serialized form is what must be bounded"
        );

        // Multi-byte UTF-8 is emitted raw, so it is bounded by its byte length.
        assert!(
            server
                .remember_json(RememberInput {
                    content: "é".repeat(MAX_MCP_CONTENT_BYTES / 2 - 2),
                    ..remember_input()
                })
                .is_ok()
        );

        // Accept-side boundary: exactly at the ceiling is allowed.
        assert!(
            server
                .remember_json(RememberInput {
                    content: "a".repeat(MAX_MCP_CONTENT_BYTES - 2),
                    ..remember_input()
                })
                .is_ok(),
            "content whose serialized form is exactly the ceiling is accepted"
        );
    }

    #[test]
    fn tag_count_and_size_are_bounded_at_ingress_before_the_gate() {
        let (server, store, policy_calls) = mcp(false, Tamper::None, true, Ok(trusted()));
        // Refused before authorization, so an unauthorized caller gets the ceiling
        // error rather than a refusal — a transport limit is not an oracle.
        assert_eq!(
            tool_response(server.remember_json(RememberInput {
                tags: (0..=MAX_MCP_TAGS).map(|i| format!("t{i}")).collect(),
                ..remember_input()
            })),
            r#"{"error":"limit_exceeded"}"#
        );
        assert_eq!(
            tool_response(server.remember_json(RememberInput {
                tags: vec!["t".repeat(MAX_MCP_TAG_BYTES + 1)],
                ..remember_input()
            })),
            r#"{"error":"limit_exceeded"}"#,
            "tag contents are bounded here, not left to the post-gate core check"
        );
        assert!(store.tenants.lock().expect("tenants").is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());

        // Accept side: exactly at both ceilings.
        let (allowed_server, _store, _calls) = allowed();
        assert!(
            allowed_server
                .remember_json(RememberInput {
                    tags: (0..MAX_MCP_TAGS).map(|i| format!("t{i}")).collect(),
                    ..remember_input()
                })
                .is_ok()
        );
    }

    #[test]
    fn a_deployment_narrowed_ceiling_is_reported_and_enforced() {
        let store = Recording::default();
        let resolver = MemoryPolicyContextResolver::new(
            Source(Ok(trusted())),
            Policy {
                allow: true,
                tamper: Tamper::None,
                memory_enabled: true,
                calls: Arc::new(Mutex::new(vec![])),
            },
        );
        let service =
            MemoryService::with_result_ceiling(store, FixedClock(7), 2).expect("a valid narrowing");
        let server = MemoryMcp::new(service, resolver);

        let status: Value =
            serde_json::from_str(&server.status_json(StatusInput {}).expect("status"))
                .expect("JSON");
        assert_eq!(
            status["result_ceiling"], 2,
            "status must report the narrowed ceiling, not the MCP maximum"
        );
        assert_eq!(
            tool_response(server.search_json(search_input(MAX_MCP_QUERY_LIMIT))),
            r#"{"error":"limit_exceeded"}"#,
            "a query above the deployment ceiling is refused"
        );
        assert!(server.search_json(search_input(2)).is_ok());
    }

    #[test]
    fn a_request_over_the_whole_request_ceiling_is_refused_before_anything_else() {
        let (server, store, policy_calls) = allowed();
        // Built from a real DTO rather than a bare string, which is what a caller
        // actually sends.
        assert_eq!(
            tool_response(server.search_json(SearchInput {
                term: Some("x".repeat(MAX_MCP_REQUEST_BYTES)),
                ..search_input(4)
            })),
            r#"{"error":"limit_exceeded"}"#
        );
        assert!(store.tenants.lock().expect("tenants").is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());
    }

    /// The response as the transport actually writes it, not as we build it.
    ///
    /// A tool returns a `String` which the protocol embeds as a JSON *string*, so
    /// every quote and backslash is escaped a second time. An earlier revision
    /// bounded only the raw form and had 223 bytes of accidental margin against
    /// the frame; this is the test that pins the real invariant.
    #[test]
    fn every_emitted_response_respects_the_escaped_tool_text_budget() {
        let (server, _store, _calls) = allowed();

        // Escape-dense content on the EGRESS path: backslashes double, so this is
        // the worst case for framing rather than for storage.
        write_bulky_content(&server, "escaped", &"\\".repeat(crate::MAX_CONTENT_BYTES));
        let refused = tool_response(server.recall_json(RecordAddressInput {
            namespace: "notes".to_owned(),
            key: "escaped".to_owned(),
        }));
        assert_eq!(
            refused, r#"{"error":"limit_exceeded"}"#,
            "a record that cannot be framed must be refused, not written"
        );

        // Everything this surface emits must remain inside the brick-owned
        // escaped tool-text budget. The composition root still owns complete
        // envelope and request-ID bounds.
        for response in [
            server.remember_json(remember_input()).expect("remember"),
            server.recall_json(address()).expect("recall"),
            server.search_json(search_input(4)).expect("search"),
            server.status_json(StatusInput {}).expect("status"),
            server.forget_json(address()).expect("forget"),
        ] {
            let framed = framed_len(&response).expect("framed");
            assert!(
                framed <= MAX_MCP_ESCAPED_TOOL_TEXT_BYTES,
                "a {framed} byte escaped response exceeds the brick tool-text budget"
            );
            assert!(
                framed + COMPOSITION_HEADROOM_BYTES <= BRICK_TOOL_TEXT_BUDGET_BYTES,
                "escaped tool text plus conservative composition headroom exceeds the brick budget"
            );
        }
    }

    /// The genuine worst case: a full page of records at every ceiling at once —
    /// maximum identifiers, maximum tags, maximum content.
    #[test]
    fn the_true_worst_case_page_fits_both_ceilings() {
        let (server, _store, _calls) = allowed();
        let long_key = |index: u32| format!("{}{index}", "k".repeat(crate::MAX_ID_BYTES - 4));
        let long_namespace = "n".repeat(crate::MAX_ID_BYTES);
        for index in 0..MAX_MCP_QUERY_LIMIT {
            server
                .remember_json(RememberInput {
                    namespace: long_namespace.clone(),
                    key: long_key(index),
                    content: "a".repeat(MAX_MCP_CONTENT_BYTES - 2),
                    tags: (0..MAX_MCP_TAGS)
                        .map(|tag| format!("{}{tag}", "t".repeat(MAX_MCP_TAG_BYTES - 3)))
                        .collect(),
                    ..remember_input()
                })
                .expect("each record sits exactly on the MCP ceilings");
        }

        let response = server
            .search_json(SearchInput {
                namespace: long_namespace,
                ..search_input(MAX_MCP_QUERY_LIMIT)
            })
            .expect("the worst-case page must fit");
        assert!(response.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES);
        assert!(framed_len(&response).expect("framed") <= MAX_MCP_ESCAPED_TOOL_TEXT_BYTES);
        let value: Value = serde_json::from_str(&response).expect("JSON");
        assert_eq!(
            value["records"].as_array().expect("records").len(),
            MAX_MCP_QUERY_LIMIT as usize,
            "the ceilings must be consistent enough to carry a full page"
        );
    }

    /// The deferral reserve is sized for a full list of maximum-length keys.
    #[test]
    fn the_deferral_reserve_holds_a_full_list_of_maximum_length_keys() {
        let (server, _store, _calls) = allowed();
        let long_key = |index: u32| format!("{}{index}", "k".repeat(crate::MAX_ID_BYTES - 4));
        // Each record fits alone; together they cannot, so all but the first are
        // deferred and every deferred key is at the identifier ceiling.
        for index in 0..MAX_MCP_QUERY_LIMIT {
            write_bulky_content(
                &server,
                &long_key(index),
                &"a".repeat(MAX_MCP_SERIALIZED_RESULT_BYTES / 2),
            );
        }
        let response = server
            .search_json(search_input(MAX_MCP_QUERY_LIMIT))
            .expect("the reserve must accommodate the key list");
        assert!(response.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES);
        let value: Value = serde_json::from_str(&response).expect("JSON");
        let returned = value["records"].as_array().expect("records").len();
        let deferred = value["deferred_keys"].as_array().expect("deferred").len();
        let oversized = value["oversized_keys"].as_array().expect("oversized").len();
        assert_eq!(
            returned + deferred + oversized,
            MAX_MCP_QUERY_LIMIT as usize,
            "every matched record is accounted for"
        );
        assert!(
            deferred + oversized > 0,
            "the page must not silently shorten"
        );
    }

    #[test]
    fn every_policy_valid_tenant_converts_to_a_memory_tenant() {
        // `policy`'s identifier grammar is a strict subset of memory's today. If
        // either moves, every request becomes `unauthorized` with no other test
        // failing and no diagnostic, so the relation is asserted directly.
        for candidate in ["a", "9", "a_b-c", &"t".repeat(128), "0000000000"] {
            let policy_tenant = PolicyTenantId::new(candidate).expect("policy accepts it");
            assert!(
                TenantId::new(policy_tenant.as_str()).is_ok(),
                "memory must accept every tenant policy considers valid: {candidate}"
            );
        }
    }

    #[test]
    fn capacity_is_a_tenant_shared_budget_reached_through_this_surface() {
        // Documented rather than prevented: one principal can consume a tenant's
        // whole allowance and every other principal in that tenant is then refused
        // a new key. A replace must keep working, or a full partition would be
        // unrepairable.
        let (server, _store, _calls) = allowed();
        for index in 0..crate::MAX_PARTITION_RECORDS {
            server
                .remember_json(RememberInput {
                    key: format!("k-{index}"),
                    ..remember_input()
                })
                .expect("writes up to the ceiling succeed");
        }
        assert_eq!(
            tool_response(server.remember_json(RememberInput {
                key: "one-too-many".to_owned(),
                ..remember_input()
            })),
            r#"{"error":"limit_exceeded"}"#,
            "a new key past the tenant's partition ceiling is refused"
        );
        assert_eq!(
            server
                .remember_json(RememberInput {
                    key: "k-0".to_owned(),
                    content: "updated".to_owned(),
                    ..remember_input()
                })
                .expect("a replace consumes no capacity"),
            r#"{"outcome":"replaced"}"#
        );
    }

    /// Writes content past the MCP ceilings through the typed API.
    fn write_bulky_content(server: &Mcp, key: &str, content: &str) {
        let owner = MemoryContext::new(TenantId::new("trusted-tenant").expect("tenant"));
        server
            .service
            .remember(
                &owner,
                RememberRequest {
                    namespace: Namespace::new("notes").expect("namespace"),
                    key: RecordKey::new(key).expect("key"),
                    kind: MemoryKind::Factual,
                    content: content.to_owned(),
                    tags: vec![],
                    metadata: crate::Metadata::new(),
                    run_id: None,
                },
            )
            .expect("the core accepts it");
    }

    #[test]
    fn public_error_projection_never_leaks_internal_text() {
        assert_eq!(
            tool_response(Err(anyhow::anyhow!("secret backend path /tmp/private"))),
            r#"{"error":"adapter_failure"}"#
        );
        assert_eq!(
            tool_response(Err(public_error(MemoryError::InvalidRecord))),
            r#"{"error":"invalid_record"}"#
        );
        // A cross-tenant probe is indistinguishable from absence at the boundary
        // too, not only inside the core.
        assert_eq!(public_code(MemoryError::TenantMismatch), "not_found");
        assert_eq!(public_code(MemoryError::NotFound), "not_found");
    }
}
