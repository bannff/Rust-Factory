#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)] // Trait methods share the typed DefinitionError contract.

//! Transport-independent agent definitions and a bounded local runtime.
//!
//! The agent-facing MCP surface lives in [`mcp`], behind the `mcp` feature, so
//! this crate's default build carries no transport or framework dependency.

#[cfg(feature = "mcp")]
pub mod mcp;

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Mutex;

/// The supported agent schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DefinitionVersion {
    V1,
}

impl DefinitionVersion {
    /// Returns the stable wire representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
        }
    }
}

/// A validated, stable agent identifier.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AgentId(String);
impl AgentId {
    /// Creates an ID matching `[a-z0-9][a-z0-9_-]{0,127}`.
    pub fn new(value: impl Into<String>) -> Result<Self, DefinitionError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = value.len() <= 128
            && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(DefinitionError::InvalidId)
        }
    }
    /// Returns the stable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

macro_rules! invocation_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);
        impl $name {
            /// Creates an invocation ID matching `[a-z0-9][a-z0-9_-]{0,127}`.
            pub fn new(value: impl Into<String>) -> Result<Self, DefinitionError> {
                let value = value.into();
                let mut bytes = value.bytes();
                let valid = value.len() <= 128
                    && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
                    && bytes.all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'_' | b'-')
                    });
                valid
                    .then_some(Self(value))
                    .ok_or(DefinitionError::InvalidRequest)
            }

            /// Returns the validated identifier.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

invocation_id!(TenantId);
invocation_id!(PrincipalId);
invocation_id!(RequestId);
invocation_id!(CorrelationId);

/// Trusted, request-scoped identity propagated only to Agent-owned effect ports.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)] // Contract fields intentionally retain their distinct ID names.
pub struct InvocationContextV1 {
    tenant_id: TenantId,
    principal_id: PrincipalId,
    request_id: RequestId,
    correlation_id: CorrelationId,
}
impl InvocationContextV1 {
    #[must_use]
    pub const fn new(
        tenant_id: TenantId,
        principal_id: PrincipalId,
        request_id: RequestId,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            tenant_id,
            principal_id,
            request_id,
            correlation_id,
        }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    #[must_use]
    pub const fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    #[must_use]
    pub const fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }
}

/// Typed model selection policy without provider configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelPolicy {
    pub reference: String,
}
/// Typed memory policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryPolicy {
    pub enabled: bool,
    pub max_items: u32,
}
/// Typed knowledge policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgePolicy {
    pub enabled: bool,
    pub namespace: String,
    pub max_results: u32,
}
/// Typed sandbox policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxPolicy {
    pub allow_execution: bool,
}
/// Typed communication policy. Networking is intentionally not implemented.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommunicationPolicy {
    pub allow_messages: bool,
}
/// Rust Factory hard ceiling for configured or emitted tool calls.
pub const MAX_TOOL_CALLS: u32 = 64;
/// Rust Factory hard ceiling for configured or emitted output bytes.
pub const MAX_OUTPUT_BYTES: u32 = 65_536;
/// Rust Factory hard ceiling for an invocation or provider input string.
pub const MAX_INPUT_BYTES: usize = 16_384;
/// Rust Factory hard ceiling for a memory-write value.
pub const MAX_MEMORY_WRITE_VALUE_BYTES: usize = 16_384;
/// Rust Factory hard ceiling for a tool request input.
pub const MAX_TOOL_REQUEST_INPUT_BYTES: usize = 16_384;
/// Rust Factory hard ceiling for sandbox arguments per request.
pub const MAX_SANDBOX_ARGUMENTS: usize = 32;
/// Rust Factory hard ceiling for each sandbox argument.
pub const MAX_SANDBOX_ARGUMENT_BYTES: usize = 4_096;
/// Rust Factory hard ceiling for references in any definition capability set.
pub const MAX_CAPABILITY_REFERENCES: usize = 64;

/// Positive, attempt-local execution limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionLimits {
    pub max_tool_calls: u32,
    pub max_output_bytes: u32,
}

/// Complete version-one agent definition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDefinitionV1 {
    pub version: DefinitionVersion,
    pub id: AgentId,
    pub name: String,
    pub description: String,
    pub model: ModelPolicy,
    pub instructions: String,
    pub skills: Vec<String>,
    pub steering: Vec<String>,
    pub allowed_tool_ids: Vec<String>,
    pub memory: MemoryPolicy,
    pub knowledge: KnowledgePolicy,
    pub sandbox: SandboxPolicy,
    pub communication: CommunicationPolicy,
    pub limits: ExecutionLimits,
}

/// Per-invocation capability ceiling supplied by an external grant boundary.
///
/// This model can only narrow an agent definition. It is intentionally not an
/// identity or authorization model.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // Mirrors the four independently grantable capability families.
pub struct EffectiveCapabilityCeilingV1 {
    pub allowed_tool_ids: Vec<String>,
    pub memory_enabled: bool,
    pub knowledge_enabled: bool,
    pub sandbox_execution_allowed: bool,
    pub communication_allowed: bool,
}
impl EffectiveCapabilityCeilingV1 {
    /// Returns a ceiling that preserves every capability configured by a definition.
    #[must_use]
    pub fn full_definition(definition: &AgentDefinitionV1) -> Self {
        Self {
            allowed_tool_ids: definition.allowed_tool_ids.clone(),
            memory_enabled: definition.memory.enabled,
            knowledge_enabled: definition.knowledge.enabled,
            sandbox_execution_allowed: definition.sandbox.allow_execution,
            communication_allowed: definition.communication.allow_messages,
        }
    }

    /// Intersects this external ceiling with a definition without permitting elevation.
    pub fn intersect(
        &self,
        definition: &AgentDefinitionV1,
    ) -> Result<EffectiveCapabilityCeilingV1, DefinitionError> {
        validate_effective_capability_ceiling(self)?;
        let definition_tools = definition.allowed_tool_ids.iter().collect::<BTreeSet<_>>();
        let mut allowed_tool_ids = self
            .allowed_tool_ids
            .iter()
            .filter(|tool_id| definition_tools.contains(tool_id))
            .cloned()
            .collect::<Vec<_>>();
        allowed_tool_ids.sort();
        allowed_tool_ids.dedup();
        Ok(Self {
            allowed_tool_ids,
            memory_enabled: self.memory_enabled && definition.memory.enabled,
            knowledge_enabled: self.knowledge_enabled && definition.knowledge.enabled,
            sandbox_execution_allowed: self.sandbox_execution_allowed
                && definition.sandbox.allow_execution,
            communication_allowed: self.communication_allowed
                && definition.communication.allow_messages,
        })
    }
}

/// A reduced, safe discovery view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSummary {
    pub id: AgentId,
    pub name: String,
    pub description: String,
}
impl From<&AgentDefinitionV1> for AgentSummary {
    fn from(value: &AgentDefinitionV1) -> Self {
        Self {
            id: value.id.clone(),
            name: value.name.clone(),
            description: value.description.clone(),
        }
    }
}

/// Stable public error categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidRequest,
    InvalidDefinition,
    InvalidReference,
    ReferenceUnavailable,
    NotFound,
    BuiltinProtected,
    UnknownTool,
    ToolDisallowed,
    MemoryDenied,
    KnowledgeDenied,
    SandboxDenied,
    AdapterFailure,
    LimitExceeded,
    Cancelled,
    DeadlineExceeded,
}
/// Typed core errors without adapter details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefinitionError {
    InvalidRequest,
    InvalidId,
    InvalidDefinition,
    InvalidReference,
    ReferenceUnavailable,
    NotFound,
    BuiltinProtected,
    UnknownTool(String),
    ToolDisallowed(String),
    MemoryDenied,
    KnowledgeDenied,
    SandboxDenied,
    AdapterFailure,
    LimitExceeded,
    Cancelled,
    DeadlineExceeded,
}
impl DefinitionError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
            Self::InvalidRequest => PublicErrorCode::InvalidRequest,
            Self::InvalidId | Self::InvalidDefinition => PublicErrorCode::InvalidDefinition,
            Self::InvalidReference => PublicErrorCode::InvalidReference,
            Self::ReferenceUnavailable => PublicErrorCode::ReferenceUnavailable,
            Self::NotFound => PublicErrorCode::NotFound,
            Self::BuiltinProtected => PublicErrorCode::BuiltinProtected,
            Self::UnknownTool(_) => PublicErrorCode::UnknownTool,
            Self::ToolDisallowed(_) => PublicErrorCode::ToolDisallowed,
            Self::MemoryDenied => PublicErrorCode::MemoryDenied,
            Self::KnowledgeDenied => PublicErrorCode::KnowledgeDenied,
            Self::SandboxDenied => PublicErrorCode::SandboxDenied,
            Self::AdapterFailure => PublicErrorCode::AdapterFailure,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
            Self::Cancelled => PublicErrorCode::Cancelled,
            Self::DeadlineExceeded => PublicErrorCode::DeadlineExceeded,
        }
    }
}
impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "agent definition operation failed: {:?}",
            self.public_code()
        )
    }
}
impl std::error::Error for DefinitionError {}

/// Validates definition fields that do not require an external capability catalog.
pub fn validate_definition(definition: &AgentDefinitionV1) -> Result<(), DefinitionError> {
    if definition.name.trim().is_empty()
        || definition.description.trim().is_empty()
        || definition.instructions.trim().is_empty()
        || definition.instructions.len() > MAX_INPUT_BYTES
        || definition.skills.len() > MAX_CAPABILITY_REFERENCES
        || definition.steering.len() > MAX_CAPABILITY_REFERENCES
        || definition.allowed_tool_ids.len() > MAX_CAPABILITY_REFERENCES
        || definition.limits.max_tool_calls == 0
        || definition.limits.max_tool_calls > MAX_TOOL_CALLS
        || definition.limits.max_output_bytes == 0
        || definition.limits.max_output_bytes > MAX_OUTPUT_BYTES
        || (definition.memory.enabled && definition.memory.max_items == 0)
        || (definition.knowledge.enabled && definition.knowledge.max_results == 0)
        || definition.knowledge.max_results > knowledge::MAX_SEARCH_LIMIT
        || knowledge::NamespaceId::new(definition.knowledge.namespace.clone()).is_err()
    {
        return Err(DefinitionError::InvalidDefinition);
    }
    if !valid_reference(&definition.model.reference)
        || !definition.skills.iter().all(|value| valid_reference(value))
        || !definition
            .steering
            .iter()
            .all(|value| valid_reference(value))
        || !definition
            .allowed_tool_ids
            .iter()
            .all(|value| valid_reference(value))
    {
        return Err(DefinitionError::InvalidReference);
    }
    Ok(())
}

/// Validates an external capability ceiling before it is applied to a definition.
pub fn validate_effective_capability_ceiling(
    ceiling: &EffectiveCapabilityCeilingV1,
) -> Result<(), DefinitionError> {
    if ceiling.allowed_tool_ids.len() > MAX_CAPABILITY_REFERENCES {
        return Err(DefinitionError::InvalidDefinition);
    }
    if ceiling
        .allowed_tool_ids
        .iter()
        .all(|value| valid_reference(value))
    {
        Ok(())
    } else {
        Err(DefinitionError::InvalidReference)
    }
}

/// Catalog port that owns availability of logical model and capability references.
pub trait ReferenceCatalog: Send + Sync {
    fn contains_model(&self, reference: &str) -> Result<bool, DefinitionError>;
    fn contains_skill(&self, reference: &str) -> Result<bool, DefinitionError>;
    fn contains_steering(&self, reference: &str) -> Result<bool, DefinitionError>;
    fn contains_tool(&self, reference: &str) -> Result<bool, DefinitionError>;
}
/// Backwards-compatible semantic name for a reference catalog used as a capability catalog.
pub trait CapabilityCatalog: ReferenceCatalog {}
impl<T: ReferenceCatalog + ?Sized> CapabilityCatalog for T {}

/// Deterministic static reference catalog for local use and tests.
#[derive(Clone, Debug, Default)]
pub struct StaticReferenceCatalog {
    models: BTreeSet<String>,
    skills: BTreeSet<String>,
    steering: BTreeSet<String>,
    tools: BTreeSet<String>,
}
impl StaticReferenceCatalog {
    #[must_use]
    pub fn new(
        models: impl IntoIterator<Item = String>,
        skills: impl IntoIterator<Item = String>,
        steering: impl IntoIterator<Item = String>,
        tools: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            models: models.into_iter().collect(),
            skills: skills.into_iter().collect(),
            steering: steering.into_iter().collect(),
            tools: tools.into_iter().collect(),
        }
    }
}
impl ReferenceCatalog for StaticReferenceCatalog {
    fn contains_model(&self, reference: &str) -> Result<bool, DefinitionError> {
        Ok(self.models.contains(reference))
    }
    fn contains_skill(&self, reference: &str) -> Result<bool, DefinitionError> {
        Ok(self.skills.contains(reference))
    }
    fn contains_steering(&self, reference: &str) -> Result<bool, DefinitionError> {
        Ok(self.steering.contains(reference))
    }
    fn contains_tool(&self, reference: &str) -> Result<bool, DefinitionError> {
        Ok(self.tools.contains(reference))
    }
}

fn validate_references<C: ReferenceCatalog>(
    definition: &AgentDefinitionV1,
    catalog: &C,
) -> Result<(), DefinitionError> {
    if !catalog
        .contains_model(&definition.model.reference)
        .map_err(|_| DefinitionError::ReferenceUnavailable)?
    {
        return Err(DefinitionError::ReferenceUnavailable);
    }
    for reference in &definition.skills {
        if !catalog
            .contains_skill(reference)
            .map_err(|_| DefinitionError::ReferenceUnavailable)?
        {
            return Err(DefinitionError::ReferenceUnavailable);
        }
    }
    for reference in &definition.steering {
        if !catalog
            .contains_steering(reference)
            .map_err(|_| DefinitionError::ReferenceUnavailable)?
        {
            return Err(DefinitionError::ReferenceUnavailable);
        }
    }
    for reference in &definition.allowed_tool_ids {
        if !catalog
            .contains_tool(reference)
            .map_err(|_| DefinitionError::ReferenceUnavailable)?
        {
            return Err(DefinitionError::ReferenceUnavailable);
        }
    }
    Ok(())
}

/// A stable logical reference is 1–128 lowercase ASCII letters/digits separated by `-`, `_`, or `.`.
/// It starts and ends with an alphanumeric character; paths, URIs, provider configuration, and whitespace are invalid.
fn valid_reference(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

/// User-definition persistence port. Built-ins never use this port.
pub trait DefinitionStore: Send + Sync {
    fn load(&self, id: &AgentId) -> Result<Option<AgentDefinitionV1>, DefinitionError>;
    fn list(&self) -> Result<Vec<AgentDefinitionV1>, DefinitionError>;
    fn save(&self, definition: AgentDefinitionV1) -> Result<(), DefinitionError>;
    fn delete(&self, id: &AgentId) -> Result<(), DefinitionError>;
}
/// Deterministic in-memory definition store.
#[derive(Default)]
pub struct InMemoryDefinitionStore {
    definitions: Mutex<BTreeMap<AgentId, AgentDefinitionV1>>,
}
impl DefinitionStore for InMemoryDefinitionStore {
    fn load(&self, id: &AgentId) -> Result<Option<AgentDefinitionV1>, DefinitionError> {
        self.definitions
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)
            .map(|items| items.get(id).cloned())
    }
    fn list(&self) -> Result<Vec<AgentDefinitionV1>, DefinitionError> {
        self.definitions
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)
            .map(|items| items.values().cloned().collect())
    }
    fn save(&self, definition: AgentDefinitionV1) -> Result<(), DefinitionError> {
        self.definitions
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)?
            .insert(definition.id.clone(), definition);
        Ok(())
    }
    fn delete(&self, id: &AgentId) -> Result<(), DefinitionError> {
        self.definitions
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)?
            .remove(id);
        Ok(())
    }
}
/// Built-in precedence and user-definition registry with catalog-backed reference validation.
pub struct AgentRegistry<S, C> {
    builtins: BTreeMap<AgentId, AgentDefinitionV1>,
    store: S,
    catalog: C,
}
impl<S: DefinitionStore, C: ReferenceCatalog> AgentRegistry<S, C> {
    pub fn new(
        builtins: Vec<AgentDefinitionV1>,
        store: S,
        catalog: C,
    ) -> Result<Self, DefinitionError> {
        let mut indexed = BTreeMap::new();
        for definition in builtins {
            validate_definition(&definition)?;
            validate_references(&definition, &catalog)?;
            if indexed.insert(definition.id.clone(), definition).is_some() {
                return Err(DefinitionError::InvalidDefinition);
            }
        }
        Ok(Self {
            builtins: indexed,
            store,
            catalog,
        })
    }
    /// Validates a definition against syntax, hard limits, and the injected catalog.
    pub fn validate(&self, definition: &AgentDefinitionV1) -> Result<(), DefinitionError> {
        validate_definition(definition)?;
        validate_references(definition, &self.catalog)
    }
    pub fn get(&self, id: &AgentId) -> Result<AgentDefinitionV1, DefinitionError> {
        if let Some(definition) = self.builtins.get(id) {
            return Ok(definition.clone());
        }
        self.store
            .load(id)?
            .ok_or(DefinitionError::NotFound)
            .and_then(|definition| {
                validate_definition(&definition)?;
                validate_references(&definition, &self.catalog)?;
                Ok(definition)
            })
    }
    pub fn list(&self) -> Result<Vec<AgentSummary>, DefinitionError> {
        let mut merged = self.builtins.clone();
        for definition in self.store.list()? {
            validate_definition(&definition)?;
            validate_references(&definition, &self.catalog)?;
            merged.entry(definition.id.clone()).or_insert(definition);
        }
        Ok(merged.values().map(AgentSummary::from).collect())
    }
    pub fn register(&self, definition: AgentDefinitionV1) -> Result<(), DefinitionError> {
        validate_definition(&definition)?;
        validate_references(&definition, &self.catalog)?;
        if self.builtins.contains_key(&definition.id) {
            Err(DefinitionError::BuiltinProtected)
        } else {
            self.store.save(definition)
        }
    }
    pub fn delete(&self, id: &AgentId) -> Result<(), DefinitionError> {
        if self.builtins.contains_key(id) {
            Err(DefinitionError::BuiltinProtected)
        } else {
            self.store.delete(id)
        }
    }
}

/// Fully resolved, provider-neutral capability policy for one invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCapabilityScope {
    pub version: DefinitionVersion,
    pub agent_id: AgentId,
    pub model: ModelPolicy,
    pub instructions: String,
    pub skills: Vec<String>,
    pub steering: Vec<String>,
    pub allowed_tool_ids: Vec<String>,
    pub memory: MemoryPolicy,
    pub knowledge: KnowledgePolicy,
    pub sandbox: SandboxPolicy,
    pub communication: CommunicationPolicy,
    pub limits: ExecutionLimits,
    pub digest: String,
}
impl ResolvedCapabilityScope {
    fn from_definition(
        definition: &AgentDefinitionV1,
        tools: &[ToolDescriptor],
        ceiling: &EffectiveCapabilityCeilingV1,
    ) -> Self {
        let mut skills = definition.skills.clone();
        skills.sort();
        skills.dedup();
        let mut steering = definition.steering.clone();
        steering.sort();
        steering.dedup();
        let ceiling_tools = ceiling.allowed_tool_ids.iter().collect::<BTreeSet<_>>();
        let mut allowed_tool_ids = tools
            .iter()
            .map(|tool| &tool.id)
            .filter(|tool_id| ceiling_tools.contains(tool_id))
            .cloned()
            .collect::<Vec<_>>();
        allowed_tool_ids.sort();
        allowed_tool_ids.dedup();
        let mut scope = Self {
            version: definition.version,
            agent_id: definition.id.clone(),
            model: definition.model.clone(),
            instructions: definition.instructions.clone(),
            skills,
            steering,
            allowed_tool_ids,
            memory: MemoryPolicy {
                enabled: definition.memory.enabled && ceiling.memory_enabled,
                max_items: definition.memory.max_items,
            },
            knowledge: KnowledgePolicy {
                enabled: definition.knowledge.enabled && ceiling.knowledge_enabled,
                namespace: definition.knowledge.namespace.clone(),
                max_results: definition.knowledge.max_results,
            },
            sandbox: SandboxPolicy {
                allow_execution: definition.sandbox.allow_execution
                    && ceiling.sandbox_execution_allowed,
            },
            communication: CommunicationPolicy {
                allow_messages: definition.communication.allow_messages
                    && ceiling.communication_allowed,
            },
            limits: definition.limits.clone(),
            digest: String::new(),
        };
        scope.digest = digest_scope(&scope);
        scope
    }
}

/// Borrowed asynchronous result of one Agent invocation.
pub type InvocationFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<InvocationResult, DefinitionError>> + Send + 'a>,
>;

/// Agent-owned safe projection of a normalized model finish reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationModelFinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other,
}

/// Agent-owned safe projection of bounded token usage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvocationModelTokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

/// Agent-owned safe projection of provider idempotency support.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationModelIdempotency {
    Unsupported,
    Accepted,
}

/// Safe model evidence retained by Agent without provider payloads or errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationModelEvidence {
    pub provider_id: String,
    pub model_id: String,
    pub provider_request_id: Option<String>,
    pub finish_reason: InvocationModelFinishReason,
    pub token_usage: Option<InvocationModelTokenUsage>,
    pub idempotency: InvocationModelIdempotency,
}

/// Registered typed tool metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDescriptor {
    pub id: String,
}
/// A policy-scoped request to a named tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolRequest {
    pub context: InvocationContextV1,
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub input: String,
}
/// Tool-registry port.
///
/// Owned by the Agent family by design, not pending extraction: the tools an
/// agent may call are part of its definition and authorization scope. Isolated
/// execution of a tool or test with captured evidence is a separate concern
/// that belongs to the Sandbox family.
pub trait ToolRegistry: Send + Sync {
    fn resolve(&self, id: &str) -> Result<ToolDescriptor, DefinitionError>;
    fn invoke(
        &self,
        tool: &ToolDescriptor,
        request: ToolRequest,
    ) -> Result<String, DefinitionError>;
}
/// Scoped memory recall or write request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRequest {
    pub context: InvocationContextV1,
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub query: String,
    pub limit: u32,
}
/// Memory port.
///
/// Provisional pre-extraction port owned in the registry by the Memory family.
/// It is deliberately anemic — untyped values with no namespaces, keys,
/// eviction, or tenancy — so a durable contract is net-new design rather than a
/// move. It stays here until a consumer beyond [`LocalAgentRuntime`] shapes it.
pub trait MemoryStore: Send + Sync {
    fn recall(&self, request: MemoryRequest) -> Result<Vec<String>, DefinitionError>;
    fn write(&self, request: MemoryRequest, value: String) -> Result<(), DefinitionError>;
}
/// A typed sandbox action identifier and arguments, never source code or a shell command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxRequest {
    pub context: InvocationContextV1,
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub action: String,
    pub arguments: Vec<String>,
}
/// Sandbox-execution port.
///
/// Provisional pre-extraction port owned in the registry by the Sandbox family,
/// which will also own isolated tool and test execution with captured evidence.
/// This is the cleanest extraction candidate of the four because its request
/// payload is nearly domain-neutral; only the agent identity and resolved scope
/// fields need a neutral principal type first.
pub trait Sandbox: Send + Sync {
    fn execute(&self, request: SandboxRequest) -> Result<String, DefinitionError>;
}

/// Fixed deterministic tool registry.
#[derive(Default)]
pub struct FixedToolRegistry {
    tools: BTreeMap<String, String>,
}
impl FixedToolRegistry {
    #[must_use]
    pub fn new(tools: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            tools: tools.into_iter().collect(),
        }
    }
}
impl ToolRegistry for FixedToolRegistry {
    fn resolve(&self, id: &str) -> Result<ToolDescriptor, DefinitionError> {
        self.tools
            .contains_key(id)
            .then(|| ToolDescriptor { id: id.to_owned() })
            .ok_or_else(|| DefinitionError::UnknownTool(id.to_owned()))
    }
    fn invoke(&self, tool: &ToolDescriptor, _: ToolRequest) -> Result<String, DefinitionError> {
        self.tools
            .get(&tool.id)
            .cloned()
            .ok_or_else(|| DefinitionError::UnknownTool(tool.id.clone()))
    }
}
/// Deterministic in-memory memory adapter.
#[derive(Default)]
pub struct InMemoryMemoryStore {
    values: Mutex<BTreeMap<TenantId, Vec<String>>>,
}
impl MemoryStore for InMemoryMemoryStore {
    fn recall(&self, request: MemoryRequest) -> Result<Vec<String>, DefinitionError> {
        Ok(self
            .values
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)?
            .get(request.context.tenant_id())
            .into_iter()
            .flatten()
            .filter(|value| value.contains(&request.query))
            .take(request.limit as usize)
            .cloned()
            .collect())
    }
    fn write(&self, request: MemoryRequest, value: String) -> Result<(), DefinitionError> {
        self.values
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)?
            .entry(request.context.tenant_id().clone())
            .or_default()
            .push(value);
        Ok(())
    }
}
/// Initial sandbox adapter that always denies execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct DenySandbox;
impl Sandbox for DenySandbox {
    fn execute(&self, _: SandboxRequest) -> Result<String, DefinitionError> {
        Err(DefinitionError::SandboxDenied)
    }
}

/// Agent-owned projection of one bounded knowledge hit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeResult {
    pub document_id: String,
    pub text: String,
}

/// Ordered normalized runtime event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvocationEvent {
    ModelInvoked,
    MemoryRecalled { values: Vec<String> },
    MemoryWritten,
    KnowledgeSearched { results: Vec<KnowledgeResult> },
    SandboxCompleted { output: String },
    ToolCompleted { tool_id: String, output: String },
}
/// Terminal result for one local, attempt-local invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationResult {
    pub capability_scope_digest: String,
    pub events: Vec<InvocationEvent>,
    pub output: String,
    pub model_evidence: InvocationModelEvidence,
}

const MEMORY_RECALL_TOOL: &str = "factory.memory.recall";
const MEMORY_WRITE_TOOL: &str = "factory.memory.write";
const KNOWLEDGE_SEARCH_TOOL: &str = "factory.knowledge.search";
const SANDBOX_EXECUTE_TOOL: &str = "factory.sandbox.execute";
const RESERVED_TOOL_NAMES: [&str; 4] = [
    MEMORY_RECALL_TOOL,
    MEMORY_WRITE_TOOL,
    KNOWLEDGE_SEARCH_TOOL,
    SANDBOX_EXECUTE_TOOL,
];
const STRING_ARGUMENT_SCHEMA: &str = r#"{"additionalProperties":false,"properties":{"input":{"type":"string"}},"required":["input"],"type":"object"}"#;
const QUERY_SCHEMA: &str = r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"required":["query"],"type":"object"}"#;
const VALUE_SCHEMA: &str = r#"{"additionalProperties":false,"properties":{"value":{"type":"string"}},"required":["value"],"type":"object"}"#;
const SANDBOX_SCHEMA: &str = r#"{"additionalProperties":false,"properties":{"action":{"type":"string"},"arguments":{"items":{"type":"string"},"type":"array"}},"required":["action","arguments"],"type":"object"}"#;

enum PlannedEffect {
    Tool {
        tool: ToolDescriptor,
        input: String,
    },
    MemoryRecall {
        query: String,
    },
    MemoryWrite {
        value: String,
    },
    KnowledgeSearch {
        query: String,
    },
    SandboxExecute {
        action: String,
        arguments: Vec<String>,
    },
}

/// Bounded local runtime with all capabilities injected through core ports.
pub struct LocalAgentRuntime<'a, S, C, M, T, MM, K, SB> {
    registry: &'a AgentRegistry<S, C>,
    model: &'a M,
    tools: &'a T,
    memory: &'a MM,
    knowledge: &'a K,
    sandbox: &'a SB,
}
impl<
    'a,
    S: DefinitionStore,
    C: ReferenceCatalog,
    M: llm_gateway::LlmProvider,
    T: ToolRegistry,
    MM: MemoryStore,
    K: knowledge::KnowledgeIndex,
    SB: Sandbox,
> LocalAgentRuntime<'a, S, C, M, T, MM, K, SB>
{
    #[must_use]
    pub fn new(
        registry: &'a AgentRegistry<S, C>,
        model: &'a M,
        tools: &'a T,
        memory: &'a MM,
        knowledge: &'a K,
        sandbox: &'a SB,
    ) -> Self {
        Self {
            registry,
            model,
            tools,
            memory,
            knowledge,
            sandbox,
        }
    }

    /// Invokes using the definition's complete capability set.
    #[must_use]
    pub fn invoke<'b>(
        &'b self,
        context: InvocationContextV1,
        id: &'b AgentId,
        input: String,
        control: llm_gateway::InvocationControl<'b>,
    ) -> InvocationFuture<'b> {
        Box::pin(async move {
            let definition = self.registry.get(id)?;
            let ceiling = EffectiveCapabilityCeilingV1::full_definition(&definition);
            self.invoke_definition(context, id, input, definition, ceiling, control)
                .await
        })
    }

    /// Invokes with an external ceiling that can only reduce the definition's scope.
    #[must_use]
    pub fn invoke_with_ceiling<'b>(
        &'b self,
        context: InvocationContextV1,
        id: &'b AgentId,
        input: String,
        ceiling: &'b EffectiveCapabilityCeilingV1,
        control: llm_gateway::InvocationControl<'b>,
    ) -> InvocationFuture<'b> {
        Box::pin(async move {
            validate_effective_capability_ceiling(ceiling)?;
            let definition = self.registry.get(id)?;
            self.invoke_definition(context, id, input, definition, ceiling.clone(), control)
                .await
        })
    }

    #[allow(clippy::too_many_lines)]
    async fn invoke_definition(
        &self,
        context: InvocationContextV1,
        id: &AgentId,
        input: String,
        definition: AgentDefinitionV1,
        ceiling: EffectiveCapabilityCeilingV1,
        control: llm_gateway::InvocationControl<'_>,
    ) -> Result<InvocationResult, DefinitionError> {
        if input.len() > MAX_INPUT_BYTES {
            return Err(DefinitionError::LimitExceeded);
        }
        let effective_ceiling = ceiling.intersect(&definition)?;
        let resolved = effective_ceiling
            .allowed_tool_ids
            .iter()
            .map(|tool_id| self.tools.resolve(tool_id))
            .collect::<Result<Vec<_>, _>>()?;
        let scope =
            ResolvedCapabilityScope::from_definition(&definition, &resolved, &effective_ceiling);
        let (provider_id, model_id) = parse_model_reference(&definition.model.reference)?;
        let (tool_definitions, ordinary_tools) = gateway_tools(&resolved, &scope)?;
        let request = llm_gateway::GenerateRequest::new(
            provider_id,
            model_id,
            llm_gateway::Prompt::new(Some(definition.instructions.clone()), input)
                .map_err(map_gateway_error)?,
            tool_definitions,
            llm_gateway::GenerationLimits::new(definition.limits.max_output_bytes)
                .map_err(map_gateway_error)?,
        )
        .map_err(map_gateway_error)?;
        let response = self
            .model
            .generate(&request, control)
            .await
            .map_err(map_gateway_error)?;
        control.preflight().map_err(map_gateway_error)?;
        if response.tool_calls().len() > definition.limits.max_tool_calls as usize
            || response.tool_calls().len() > MAX_TOOL_CALLS as usize
        {
            return Err(DefinitionError::LimitExceeded);
        }
        let mut output_bytes =
            checked_output_bytes(response.text(), 0, definition.limits.max_output_bytes)?;
        let mut plan = Vec::with_capacity(response.tool_calls().len());
        for call in response.tool_calls() {
            let name = call.name().as_str();
            if let Some(tool) = ordinary_tools.get(name) {
                if !scope
                    .allowed_tool_ids
                    .iter()
                    .any(|allowed| allowed == &tool.id)
                {
                    return Err(DefinitionError::ToolDisallowed(tool.id.clone()));
                }
                let input = strict_string_argument(call.arguments(), "input")?;
                if input.len() > MAX_TOOL_REQUEST_INPUT_BYTES {
                    return Err(DefinitionError::LimitExceeded);
                }
                plan.push(PlannedEffect::Tool {
                    tool: tool.clone(),
                    input,
                });
                continue;
            }
            match name {
                MEMORY_RECALL_TOOL => {
                    if !scope.memory.enabled {
                        return Err(DefinitionError::MemoryDenied);
                    }
                    let query = strict_string_argument(call.arguments(), "query")?;
                    if query.len() > MAX_INPUT_BYTES {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    plan.push(PlannedEffect::MemoryRecall { query });
                }
                MEMORY_WRITE_TOOL => {
                    if !scope.memory.enabled {
                        return Err(DefinitionError::MemoryDenied);
                    }
                    let value = strict_string_argument(call.arguments(), "value")?;
                    if value.len() > MAX_MEMORY_WRITE_VALUE_BYTES {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    plan.push(PlannedEffect::MemoryWrite { value });
                }
                KNOWLEDGE_SEARCH_TOOL => {
                    if !scope.knowledge.enabled {
                        return Err(DefinitionError::KnowledgeDenied);
                    }
                    let query = strict_string_argument(call.arguments(), "query")?;
                    if query.len() > MAX_INPUT_BYTES {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    plan.push(PlannedEffect::KnowledgeSearch { query });
                }
                SANDBOX_EXECUTE_TOOL => {
                    if !scope.sandbox.allow_execution {
                        return Err(DefinitionError::SandboxDenied);
                    }
                    let (action, arguments) = strict_sandbox_arguments(call.arguments())?;
                    if action.len() > MAX_INPUT_BYTES
                        || arguments.len() > MAX_SANDBOX_ARGUMENTS
                        || arguments
                            .iter()
                            .any(|value| value.len() > MAX_SANDBOX_ARGUMENT_BYTES)
                    {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    plan.push(PlannedEffect::SandboxExecute { action, arguments });
                }
                _ => return Err(DefinitionError::ToolDisallowed(name.to_owned())),
            }
        }

        let mut events = vec![InvocationEvent::ModelInvoked];
        for effect in plan {
            match effect {
                PlannedEffect::Tool { tool, input } => {
                    control.preflight().map_err(map_gateway_error)?;
                    let output = self.tools.invoke(
                        &tool,
                        ToolRequest {
                            context: context.clone(),
                            agent_id: id.clone(),
                            capability_scope: scope.clone(),
                            input,
                        },
                    )?;
                    output_bytes =
                        checked_output_bytes(&output, output_bytes, scope.limits.max_output_bytes)?;
                    events.push(InvocationEvent::ToolCompleted {
                        tool_id: tool.id,
                        output,
                    });
                }
                PlannedEffect::MemoryRecall { query } => {
                    control.preflight().map_err(map_gateway_error)?;
                    let values = self.memory.recall(MemoryRequest {
                        context: context.clone(),
                        agent_id: id.clone(),
                        capability_scope: scope.clone(),
                        query,
                        limit: scope.memory.max_items,
                    })?;
                    output_bytes = checked_values_output_bytes(
                        &values,
                        output_bytes,
                        scope.limits.max_output_bytes,
                    )?;
                    events.push(InvocationEvent::MemoryRecalled { values });
                }
                PlannedEffect::MemoryWrite { value } => {
                    control.preflight().map_err(map_gateway_error)?;
                    self.memory.write(
                        MemoryRequest {
                            context: context.clone(),
                            agent_id: id.clone(),
                            capability_scope: scope.clone(),
                            query: String::new(),
                            limit: scope.memory.max_items,
                        },
                        value,
                    )?;
                    events.push(InvocationEvent::MemoryWritten);
                }
                PlannedEffect::KnowledgeSearch { query } => {
                    control.preflight().map_err(map_gateway_error)?;
                    let request = knowledge::SearchRequest::new(
                        knowledge::SearchContext::new(
                            knowledge::TenantId::new(context.tenant_id().as_str())
                                .map_err(map_knowledge_error)?,
                            knowledge::PrincipalId::new(context.principal_id().as_str())
                                .map_err(map_knowledge_error)?,
                        ),
                        knowledge::NamespaceId::new(scope.knowledge.namespace.clone())
                            .map_err(map_knowledge_error)?,
                        knowledge::Query::new(query).map_err(map_knowledge_error)?,
                        knowledge::SearchLimit::new(scope.knowledge.max_results)
                            .map_err(map_knowledge_error)?,
                    );
                    let result = knowledge::KnowledgeService::new(self.knowledge)
                        .search(&request)
                        .map_err(map_knowledge_error)?;
                    let results = result
                        .hits()
                        .iter()
                        .map(|hit| KnowledgeResult {
                            document_id: hit.document_id().as_str().to_owned(),
                            text: hit.text().to_owned(),
                        })
                        .collect::<Vec<_>>();
                    output_bytes = checked_knowledge_results_output_bytes(
                        &results,
                        output_bytes,
                        scope.limits.max_output_bytes,
                    )?;
                    events.push(InvocationEvent::KnowledgeSearched { results });
                }
                PlannedEffect::SandboxExecute { action, arguments } => {
                    control.preflight().map_err(map_gateway_error)?;
                    let output = self.sandbox.execute(SandboxRequest {
                        context: context.clone(),
                        agent_id: id.clone(),
                        capability_scope: scope.clone(),
                        action,
                        arguments,
                    })?;
                    output_bytes =
                        checked_output_bytes(&output, output_bytes, scope.limits.max_output_bytes)?;
                    events.push(InvocationEvent::SandboxCompleted { output });
                }
            }
        }
        let _ = output_bytes;
        Ok(InvocationResult {
            capability_scope_digest: scope.digest,
            events,
            output: response.text().to_owned(),
            model_evidence: project_model_evidence(response.evidence()),
        })
    }
}

/// V1 model references use exactly `provider.model`, with one nonempty segment
/// on each side of the single dot. This keeps provider selection explicit and
/// rejects malformed references before the provider future is created.
fn parse_model_reference(
    reference: &str,
) -> Result<(llm_gateway::ProviderId, llm_gateway::ModelId), DefinitionError> {
    let (provider, model) = reference
        .split_once('.')
        .filter(|(provider, model)| {
            !provider.is_empty() && !model.is_empty() && !model.contains('.')
        })
        .ok_or(DefinitionError::InvalidReference)?;
    Ok((
        llm_gateway::ProviderId::new(provider).map_err(map_gateway_error)?,
        llm_gateway::ModelId::new(model).map_err(map_gateway_error)?,
    ))
}

fn gateway_tools(
    resolved: &[ToolDescriptor],
    scope: &ResolvedCapabilityScope,
) -> Result<
    (
        Vec<llm_gateway::ToolDefinition>,
        BTreeMap<String, ToolDescriptor>,
    ),
    DefinitionError,
> {
    let mut definitions = Vec::with_capacity(resolved.len() + RESERVED_TOOL_NAMES.len());
    let mut ordinary = BTreeMap::new();
    for tool in resolved {
        if RESERVED_TOOL_NAMES.contains(&tool.id.as_str()) {
            return Err(DefinitionError::InvalidReference);
        }
        let name = llm_gateway::ToolName::new(tool.id.clone()).map_err(map_gateway_error)?;
        if ordinary
            .insert(name.as_str().to_owned(), tool.clone())
            .is_some()
        {
            return Err(DefinitionError::InvalidDefinition);
        }
        definitions.push(gateway_tool(name, "Agent tool", STRING_ARGUMENT_SCHEMA)?);
    }
    if scope.memory.enabled {
        definitions.push(gateway_tool_name(MEMORY_RECALL_TOOL, QUERY_SCHEMA)?);
        definitions.push(gateway_tool_name(MEMORY_WRITE_TOOL, VALUE_SCHEMA)?);
    }
    if scope.knowledge.enabled {
        definitions.push(gateway_tool_name(KNOWLEDGE_SEARCH_TOOL, QUERY_SCHEMA)?);
    }
    if scope.sandbox.allow_execution {
        definitions.push(gateway_tool_name(SANDBOX_EXECUTE_TOOL, SANDBOX_SCHEMA)?);
    }
    Ok((definitions, ordinary))
}

fn gateway_tool_name(
    name: &str,
    schema: &str,
) -> Result<llm_gateway::ToolDefinition, DefinitionError> {
    gateway_tool(
        llm_gateway::ToolName::new(name).map_err(map_gateway_error)?,
        "Agent reserved capability",
        schema,
    )
}

fn gateway_tool(
    name: llm_gateway::ToolName,
    description: &str,
    schema: &str,
) -> Result<llm_gateway::ToolDefinition, DefinitionError> {
    llm_gateway::ToolDefinition::new(
        name,
        description,
        llm_gateway::JsonObject::new(schema).map_err(map_gateway_error)?,
    )
    .map_err(map_gateway_error)
}

fn strict_string_argument(
    arguments: &llm_gateway::JsonObject,
    field: &str,
) -> Result<String, DefinitionError> {
    let value: serde_json::Value =
        serde_json::from_str(arguments.canonical()).map_err(|_| DefinitionError::AdapterFailure)?;
    let object = value.as_object().ok_or(DefinitionError::AdapterFailure)?;
    if object.len() != 1 {
        return Err(DefinitionError::InvalidDefinition);
    }
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or(DefinitionError::InvalidDefinition)
}

fn strict_sandbox_arguments(
    arguments: &llm_gateway::JsonObject,
) -> Result<(String, Vec<String>), DefinitionError> {
    let value: serde_json::Value =
        serde_json::from_str(arguments.canonical()).map_err(|_| DefinitionError::AdapterFailure)?;
    let object = value.as_object().ok_or(DefinitionError::AdapterFailure)?;
    if object.len() != 2 {
        return Err(DefinitionError::InvalidDefinition);
    }
    let action = object
        .get("action")
        .and_then(serde_json::Value::as_str)
        .ok_or(DefinitionError::InvalidDefinition)?
        .to_owned();
    let arguments = object
        .get("arguments")
        .and_then(serde_json::Value::as_array)
        .ok_or(DefinitionError::InvalidDefinition)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or(DefinitionError::InvalidDefinition)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((action, arguments))
}

fn map_gateway_error(error: llm_gateway::LlmError) -> DefinitionError {
    match error {
        llm_gateway::LlmError::LimitExceeded => DefinitionError::LimitExceeded,
        llm_gateway::LlmError::Cancelled => DefinitionError::Cancelled,
        llm_gateway::LlmError::DeadlineExceeded => DefinitionError::DeadlineExceeded,
        llm_gateway::LlmError::InvalidRequest => DefinitionError::InvalidDefinition,
        llm_gateway::LlmError::Unsupported
        | llm_gateway::LlmError::Authentication
        | llm_gateway::LlmError::RateLimited
        | llm_gateway::LlmError::Unavailable
        | llm_gateway::LlmError::ProviderRejected
        | llm_gateway::LlmError::ProtocolViolation => DefinitionError::AdapterFailure,
    }
}

fn map_knowledge_error(error: knowledge::KnowledgeError) -> DefinitionError {
    match error {
        knowledge::KnowledgeError::InvalidRequest => DefinitionError::InvalidDefinition,
        knowledge::KnowledgeError::LimitExceeded => DefinitionError::LimitExceeded,
        knowledge::KnowledgeError::Unavailable | knowledge::KnowledgeError::ProtocolViolation => {
            DefinitionError::AdapterFailure
        }
    }
}

fn project_model_evidence(evidence: &llm_gateway::GenerationEvidence) -> InvocationModelEvidence {
    InvocationModelEvidence {
        provider_id: evidence.provider_id().as_str().to_owned(),
        model_id: evidence.model_id().as_str().to_owned(),
        provider_request_id: evidence
            .provider_request_id()
            .map(|id| id.as_str().to_owned()),
        finish_reason: match evidence.finish_reason() {
            llm_gateway::FinishReason::Stop => InvocationModelFinishReason::Stop,
            llm_gateway::FinishReason::Length => InvocationModelFinishReason::Length,
            llm_gateway::FinishReason::ToolCalls => InvocationModelFinishReason::ToolCalls,
            llm_gateway::FinishReason::ContentFilter => InvocationModelFinishReason::ContentFilter,
            llm_gateway::FinishReason::Other => InvocationModelFinishReason::Other,
        },
        token_usage: evidence
            .token_usage()
            .map(|usage| InvocationModelTokenUsage {
                input_tokens: usage.input_tokens(),
                output_tokens: usage.output_tokens(),
                total_tokens: usage.total_tokens(),
            }),
        idempotency: match evidence.idempotency() {
            llm_gateway::IdempotencyDisposition::Unsupported => {
                InvocationModelIdempotency::Unsupported
            }
            llm_gateway::IdempotencyDisposition::Accepted => InvocationModelIdempotency::Accepted,
        },
    }
}

fn checked_output_bytes(value: &str, used: usize, max: u32) -> Result<usize, DefinitionError> {
    let total = used
        .checked_add(value.len())
        .ok_or(DefinitionError::LimitExceeded)?;
    (total <= max as usize)
        .then_some(total)
        .ok_or(DefinitionError::LimitExceeded)
}
fn checked_values_output_bytes(
    values: &[String],
    used: usize,
    max: u32,
) -> Result<usize, DefinitionError> {
    values
        .iter()
        .try_fold(used, |total, value| checked_output_bytes(value, total, max))
}
fn checked_knowledge_results_output_bytes(
    results: &[KnowledgeResult],
    used: usize,
    max: u32,
) -> Result<usize, DefinitionError> {
    results.iter().try_fold(used, |total, result| {
        checked_output_bytes(&result.text, total, max)
    })
}

/// Computes a stable versioned SHA-256 hexadecimal digest of the complete definition scope.
#[must_use]
pub fn capability_scope_digest(definition: &AgentDefinitionV1, tools: &[ToolDescriptor]) -> String {
    ResolvedCapabilityScope::from_definition(
        definition,
        tools,
        &EffectiveCapabilityCeilingV1::full_definition(definition),
    )
    .digest
}

/// Computes a stable versioned SHA-256 hexadecimal digest of an effective capability scope.
pub fn capability_scope_digest_with_ceiling(
    definition: &AgentDefinitionV1,
    tools: &[ToolDescriptor],
    ceiling: &EffectiveCapabilityCeilingV1,
) -> Result<String, DefinitionError> {
    Ok(
        ResolvedCapabilityScope::from_definition(
            definition,
            tools,
            &ceiling.intersect(definition)?,
        )
        .digest,
    )
}
const CAPABILITY_SCOPE_DIGEST_FORMAT: &str = "agent-core.capability-scope.sha256.v1";

fn digest_scope(scope: &ResolvedCapabilityScope) -> String {
    fn field(hasher: &mut Sha256, name: &str, value: &[u8]) {
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    fn number(value: u32) -> [u8; 4] {
        value.to_be_bytes()
    }
    let mut hasher = Sha256::new();
    field(
        &mut hasher,
        "format",
        CAPABILITY_SCOPE_DIGEST_FORMAT.as_bytes(),
    );
    field(&mut hasher, "version", scope.version.as_str().as_bytes());
    field(&mut hasher, "agent", scope.agent_id.as_str().as_bytes());
    field(&mut hasher, "model", scope.model.reference.as_bytes());
    field(&mut hasher, "instructions", scope.instructions.as_bytes());
    for (name, values) in [
        ("skill", &scope.skills),
        ("steering", &scope.steering),
        ("tool", &scope.allowed_tool_ids),
    ] {
        field(&mut hasher, name, &(values.len() as u64).to_be_bytes());
        for value in values {
            field(&mut hasher, name, value.as_bytes());
        }
    }
    for (name, value) in [
        ("memory_enabled", scope.memory.enabled),
        ("knowledge_enabled", scope.knowledge.enabled),
        ("sandbox_allowed", scope.sandbox.allow_execution),
        ("communication_allowed", scope.communication.allow_messages),
    ] {
        field(&mut hasher, name, &[u8::from(value)]);
    }
    field(
        &mut hasher,
        "memory_max_items",
        &number(scope.memory.max_items),
    );
    field(
        &mut hasher,
        "knowledge_namespace",
        scope.knowledge.namespace.as_bytes(),
    );
    field(
        &mut hasher,
        "knowledge_max_results",
        &number(scope.knowledge.max_results),
    );
    field(
        &mut hasher,
        "max_tool_calls",
        &number(scope.limits.max_tool_calls),
    );
    field(
        &mut hasher,
        "max_output_bytes",
        &number(scope.limits.max_output_bytes),
    );
    format!("{:x}", hasher.finalize())
}
