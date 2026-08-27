#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)] // Trait methods share the typed DefinitionError contract.

//! Transport-independent agent definitions and a bounded local runtime.

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
/// Rust Factory hard ceiling for provider capability requests per invocation.
pub const MAX_CAPABILITY_REQUESTS: usize = 64;
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
}
/// Typed core errors without adapter details.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefinitionError {
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
}
impl DefinitionError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
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

/// Provider-neutral model request with the complete resolved policy scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRequest {
    pub agent_id: AgentId,
    pub model_reference: String,
    pub instructions: String,
    pub input: String,
    pub capability_scope: ResolvedCapabilityScope,
}
/// A tool call requested by a provider. Input is opaque data for the named typed tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub tool_id: String,
    pub input: String,
}
/// An adapter operation requested by a provider under the resolved capability scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityRequest {
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
/// Normalized provider result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelResponse {
    pub output: String,
    pub tool_calls: Vec<ToolCall>,
    pub capability_requests: Vec<CapabilityRequest>,
}
/// Model-provider port.
pub trait ModelProvider: Send + Sync {
    fn invoke(&self, request: ModelRequest) -> Result<ModelResponse, DefinitionError>;
}
/// Registered typed tool metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDescriptor {
    pub id: String,
}
/// A policy-scoped request to a named tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolRequest {
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub input: String,
}
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
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub query: String,
    pub limit: u32,
}
pub trait MemoryStore: Send + Sync {
    fn recall(&self, request: MemoryRequest) -> Result<Vec<String>, DefinitionError>;
    fn write(&self, request: MemoryRequest, value: String) -> Result<(), DefinitionError>;
}
/// Scoped knowledge-search request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeRequest {
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub query: String,
    pub limit: u32,
}
pub trait KnowledgeStore: Send + Sync {
    fn search(&self, request: KnowledgeRequest) -> Result<Vec<String>, DefinitionError>;
}
/// A typed sandbox action identifier and arguments, never source code or a shell command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SandboxRequest {
    pub agent_id: AgentId,
    pub capability_scope: ResolvedCapabilityScope,
    pub action: String,
    pub arguments: Vec<String>,
}
pub trait Sandbox: Send + Sync {
    fn execute(&self, request: SandboxRequest) -> Result<String, DefinitionError>;
}

/// Static deterministic model adapter for local tests and demos.
#[derive(Clone, Debug)]
pub struct StaticModelProvider {
    response: ModelResponse,
}
impl StaticModelProvider {
    #[must_use]
    pub fn new(response: ModelResponse) -> Self {
        Self { response }
    }
}
impl ModelProvider for StaticModelProvider {
    fn invoke(&self, _: ModelRequest) -> Result<ModelResponse, DefinitionError> {
        Ok(self.response.clone())
    }
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
    values: Mutex<Vec<String>>,
}
impl MemoryStore for InMemoryMemoryStore {
    fn recall(&self, request: MemoryRequest) -> Result<Vec<String>, DefinitionError> {
        Ok(self
            .values
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)?
            .iter()
            .filter(|value| value.contains(&request.query))
            .take(request.limit as usize)
            .cloned()
            .collect())
    }
    fn write(&self, _: MemoryRequest, value: String) -> Result<(), DefinitionError> {
        self.values
            .lock()
            .map_err(|_| DefinitionError::AdapterFailure)?
            .push(value);
        Ok(())
    }
}
/// Deterministic static knowledge adapter.
#[derive(Default)]
pub struct StaticKnowledgeStore {
    values: Vec<String>,
}
impl StaticKnowledgeStore {
    #[must_use]
    pub fn new(values: Vec<String>) -> Self {
        Self { values }
    }
}
impl KnowledgeStore for StaticKnowledgeStore {
    fn search(&self, request: KnowledgeRequest) -> Result<Vec<String>, DefinitionError> {
        Ok(self
            .values
            .iter()
            .filter(|value| value.contains(&request.query))
            .take(request.limit as usize)
            .cloned()
            .collect())
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

/// Ordered normalized runtime event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvocationEvent {
    ModelInvoked,
    MemoryRecalled { values: Vec<String> },
    MemoryWritten,
    KnowledgeSearched { results: Vec<String> },
    SandboxCompleted { output: String },
    ToolCompleted { tool_id: String, output: String },
}
/// Terminal result for one local, attempt-local invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationResult {
    pub capability_scope_digest: String,
    pub events: Vec<InvocationEvent>,
    pub output: String,
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
    M: ModelProvider,
    T: ToolRegistry,
    MM: MemoryStore,
    K: KnowledgeStore,
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
    /// Invokes using the definition's complete capability set for compatibility.
    pub fn invoke(&self, id: &AgentId, input: String) -> Result<InvocationResult, DefinitionError> {
        let definition = self.registry.get(id)?;
        let ceiling = EffectiveCapabilityCeilingV1::full_definition(&definition);
        self.invoke_definition(id, input, &definition, &ceiling)
    }

    /// Invokes with an external ceiling that can only reduce the definition's scope.
    pub fn invoke_with_ceiling(
        &self,
        id: &AgentId,
        input: String,
        ceiling: &EffectiveCapabilityCeilingV1,
    ) -> Result<InvocationResult, DefinitionError> {
        validate_effective_capability_ceiling(ceiling)?;
        let definition = self.registry.get(id)?;
        self.invoke_definition(id, input, &definition, ceiling)
    }

    #[allow(clippy::too_many_lines)] // Coordinates each typed, policy-gated adapter action for one invocation.
    fn invoke_definition(
        &self,
        id: &AgentId,
        input: String,
        definition: &AgentDefinitionV1,
        ceiling: &EffectiveCapabilityCeilingV1,
    ) -> Result<InvocationResult, DefinitionError> {
        if input.len() > MAX_INPUT_BYTES {
            return Err(DefinitionError::LimitExceeded);
        }
        let effective_ceiling = ceiling.intersect(definition)?;
        let resolved = effective_ceiling
            .allowed_tool_ids
            .iter()
            .map(|tool_id| self.tools.resolve(tool_id))
            .collect::<Result<Vec<_>, _>>()?;
        let scope =
            ResolvedCapabilityScope::from_definition(definition, &resolved, &effective_ceiling);
        let response = self.model.invoke(ModelRequest {
            agent_id: id.clone(),
            model_reference: definition.model.reference.clone(),
            instructions: definition.instructions.clone(),
            input,
            capability_scope: scope.clone(),
        })?;
        if response.tool_calls.len() > definition.limits.max_tool_calls as usize
            || response.tool_calls.len() > MAX_TOOL_CALLS as usize
            || response.capability_requests.len() > MAX_CAPABILITY_REQUESTS
        {
            return Err(DefinitionError::LimitExceeded);
        }
        let mut output_bytes =
            checked_output_bytes(&response.output, 0, definition.limits.max_output_bytes)?;
        let allowed = scope
            .allowed_tool_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut events = vec![InvocationEvent::ModelInvoked];
        for request in response.capability_requests {
            match request {
                CapabilityRequest::MemoryRecall { query } => {
                    if query.len() > MAX_INPUT_BYTES {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    if !scope.memory.enabled {
                        return Err(DefinitionError::MemoryDenied);
                    }
                    let values = self.memory.recall(MemoryRequest {
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
                CapabilityRequest::MemoryWrite { value } => {
                    if value.len() > MAX_MEMORY_WRITE_VALUE_BYTES {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    if !scope.memory.enabled {
                        return Err(DefinitionError::MemoryDenied);
                    }
                    self.memory.write(
                        MemoryRequest {
                            agent_id: id.clone(),
                            capability_scope: scope.clone(),
                            query: String::new(),
                            limit: scope.memory.max_items,
                        },
                        value,
                    )?;
                    events.push(InvocationEvent::MemoryWritten);
                }
                CapabilityRequest::KnowledgeSearch { query } => {
                    if query.len() > MAX_INPUT_BYTES {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    if !scope.knowledge.enabled {
                        return Err(DefinitionError::KnowledgeDenied);
                    }
                    let results = self.knowledge.search(KnowledgeRequest {
                        agent_id: id.clone(),
                        capability_scope: scope.clone(),
                        query,
                        limit: scope.knowledge.max_results,
                    })?;
                    output_bytes = checked_values_output_bytes(
                        &results,
                        output_bytes,
                        scope.limits.max_output_bytes,
                    )?;
                    events.push(InvocationEvent::KnowledgeSearched { results });
                }
                CapabilityRequest::SandboxExecute { action, arguments } => {
                    if action.len() > MAX_INPUT_BYTES
                        || arguments.len() > MAX_SANDBOX_ARGUMENTS
                        || arguments
                            .iter()
                            .any(|value| value.len() > MAX_SANDBOX_ARGUMENT_BYTES)
                    {
                        return Err(DefinitionError::LimitExceeded);
                    }
                    if !scope.sandbox.allow_execution {
                        return Err(DefinitionError::SandboxDenied);
                    }
                    let output = self.sandbox.execute(SandboxRequest {
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
        for call in response.tool_calls {
            if call.input.len() > MAX_TOOL_REQUEST_INPUT_BYTES {
                return Err(DefinitionError::LimitExceeded);
            }
            if !allowed.contains(&call.tool_id) {
                return Err(DefinitionError::ToolDisallowed(call.tool_id));
            }
            let tool = self.tools.resolve(&call.tool_id)?;
            let output = self.tools.invoke(
                &tool,
                ToolRequest {
                    agent_id: id.clone(),
                    capability_scope: scope.clone(),
                    input: call.input,
                },
            )?;
            output_bytes =
                checked_output_bytes(&output, output_bytes, scope.limits.max_output_bytes)?;
            events.push(InvocationEvent::ToolCompleted {
                tool_id: tool.id,
                output,
            });
        }
        let _ = output_bytes;
        Ok(InvocationResult {
            capability_scope_digest: scope.digest,
            events,
            output: response.output,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::sync::Arc;
    fn catalog() -> StaticReferenceCatalog {
        StaticReferenceCatalog::new(
            ["static-model".to_owned()],
            ["rust".to_owned()],
            ["factory".to_owned()],
            ["allowed".to_owned()],
        )
    }
    fn definition(id: &str, tools: Vec<&str>) -> AgentDefinitionV1 {
        AgentDefinitionV1 {
            version: DefinitionVersion::V1,
            id: AgentId::new(id).expect("id"),
            name: "Agent".to_owned(),
            description: "A deterministic agent".to_owned(),
            model: ModelPolicy {
                reference: "static-model".to_owned(),
            },
            instructions: "Respond safely.".to_owned(),
            skills: vec!["rust".to_owned()],
            steering: vec!["factory".to_owned()],
            allowed_tool_ids: tools.into_iter().map(str::to_owned).collect(),
            memory: MemoryPolicy {
                enabled: false,
                max_items: 0,
            },
            knowledge: KnowledgePolicy {
                enabled: false,
                max_results: 0,
            },
            sandbox: SandboxPolicy {
                allow_execution: false,
            },
            communication: CommunicationPolicy {
                allow_messages: false,
            },
            limits: ExecutionLimits {
                max_tool_calls: 2,
                max_output_bytes: 100,
            },
        }
    }
    fn runtime<'a>(
        definition: AgentDefinitionV1,
        response: ModelResponse,
        tools: &'a FixedToolRegistry,
        memory: &'a InMemoryMemoryStore,
        knowledge: &'a StaticKnowledgeStore,
        sandbox: &'a DenySandbox,
    ) -> LocalAgentRuntime<
        'a,
        InMemoryDefinitionStore,
        StaticReferenceCatalog,
        StaticModelProvider,
        FixedToolRegistry,
        InMemoryMemoryStore,
        StaticKnowledgeStore,
        DenySandbox,
    > {
        let registry = Box::leak(Box::new(
            AgentRegistry::new(
                vec![definition],
                InMemoryDefinitionStore::default(),
                catalog(),
            )
            .expect("registry"),
        ));
        let model = Box::leak(Box::new(StaticModelProvider::new(response)));
        LocalAgentRuntime::new(registry, model, tools, memory, knowledge, sandbox)
    }
    struct RecordingModel {
        request: Mutex<Option<ModelRequest>>,
        response: ModelResponse,
    }
    impl ModelProvider for RecordingModel {
        fn invoke(&self, request: ModelRequest) -> Result<ModelResponse, DefinitionError> {
            *self.request.lock().expect("request lock") = Some(request);
            Ok(self.response.clone())
        }
    }
    struct RecordingDefinitionStore {
        definition: AgentDefinitionV1,
        loads: Arc<Mutex<u32>>,
    }
    impl DefinitionStore for RecordingDefinitionStore {
        fn load(&self, _: &AgentId) -> Result<Option<AgentDefinitionV1>, DefinitionError> {
            *self.loads.lock().expect("load count lock") += 1;
            Ok(Some(self.definition.clone()))
        }

        fn list(&self) -> Result<Vec<AgentDefinitionV1>, DefinitionError> {
            Ok(vec![self.definition.clone()])
        }

        fn save(&self, _: AgentDefinitionV1) -> Result<(), DefinitionError> {
            Err(DefinitionError::AdapterFailure)
        }

        fn delete(&self, _: &AgentId) -> Result<(), DefinitionError> {
            Err(DefinitionError::AdapterFailure)
        }
    }
    #[derive(Default)]
    struct RecordingToolRegistry {
        resolved: Mutex<Vec<String>>,
        invoked: Mutex<Vec<String>>,
    }
    impl ToolRegistry for RecordingToolRegistry {
        fn resolve(&self, id: &str) -> Result<ToolDescriptor, DefinitionError> {
            self.resolved
                .lock()
                .expect("resolved tools lock")
                .push(id.to_owned());
            Ok(ToolDescriptor { id: id.to_owned() })
        }

        fn invoke(&self, tool: &ToolDescriptor, _: ToolRequest) -> Result<String, DefinitionError> {
            self.invoked
                .lock()
                .expect("invoked tools lock")
                .push(tool.id.clone());
            Ok(String::new())
        }
    }
    #[test]
    fn invalid_definition_and_references_are_rejected() {
        let mut value = definition("valid", vec![]);
        value.instructions.clear();
        assert_eq!(
            validate_definition(&value),
            Err(DefinitionError::InvalidDefinition)
        );
        for reference in ["/tmp/x", "provider:model", "../skill", "a b", "UPPER", "a/"] {
            value = definition("valid", vec![]);
            value.model.reference = reference.to_owned();
            assert_eq!(
                validate_definition(&value),
                Err(DefinitionError::InvalidReference)
            );
        }
    }
    #[test]
    fn builtins_win_collisions_and_are_protected() {
        let registry = AgentRegistry::new(
            vec![definition("built-in", vec![])],
            InMemoryDefinitionStore::default(),
            catalog(),
        )
        .expect("registry");
        assert_eq!(
            registry.register(definition("built-in", vec![])),
            Err(DefinitionError::BuiltinProtected)
        );
        assert_eq!(
            registry.delete(&AgentId::new("built-in").expect("id")),
            Err(DefinitionError::BuiltinProtected)
        );
    }
    #[test]
    fn scope_digest_canonicalizes_sets_and_all_policy_fields() {
        let mut left = definition("agent", vec!["b", "a", "a"]);
        left.skills = vec!["b".to_owned(), "a".to_owned()];
        left.steering = vec!["y".to_owned(), "x".to_owned()];
        let mut right = left.clone();
        right.skills.reverse();
        right.steering.reverse();
        right.allowed_tool_ids.reverse();
        let tools = vec![
            ToolDescriptor { id: "a".to_owned() },
            ToolDescriptor { id: "b".to_owned() },
        ];
        let digest = capability_scope_digest(&left, &tools);
        assert_eq!(digest.len(), 64);
        assert_eq!(
            digest,
            "f5c2d8563ad523b7683d7045d18c3a9936ff43e283e974aeb0ea303851e79af3"
        );
        assert_eq!(digest, capability_scope_digest(&right, &tools));
        for mutate in [0_u8, 1, 2, 3, 4, 5, 6, 7] {
            let mut changed = left.clone();
            match mutate {
                0 => changed.model.reference.push('x'),
                1 => changed.memory.enabled = true,
                2 => {
                    changed.knowledge.enabled = true;
                    changed.knowledge.max_results = 1;
                }
                3 => changed.sandbox.allow_execution = true,
                4 => changed.communication.allow_messages = true,
                5 => changed.limits.max_tool_calls = 3,
                6 => changed.limits.max_output_bytes = 101,
                _ => changed.instructions.push('!'),
            }
            assert_ne!(
                capability_scope_digest(&left, &tools),
                capability_scope_digest(&changed, &tools)
            );
        }
        for changed in [
            AgentDefinitionV1 {
                skills: vec!["other".to_owned()],
                ..left.clone()
            },
            AgentDefinitionV1 {
                steering: vec!["other".to_owned()],
                ..left.clone()
            },
        ] {
            assert_ne!(
                capability_scope_digest(&left, &tools),
                capability_scope_digest(&changed, &tools)
            );
        }
        assert_ne!(
            capability_scope_digest(&left, &tools),
            capability_scope_digest(&left, &[ToolDescriptor { id: "a".to_owned() }])
        );
    }
    #[test]
    fn runtime_scopes_model_and_all_enabled_capabilities() {
        let mut value = definition("agent", vec!["allowed"]);
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        value.knowledge = KnowledgePolicy {
            enabled: true,
            max_results: 1,
        };
        let tools = FixedToolRegistry::new([(String::from("allowed"), String::from("tool"))]);
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::new(vec!["knowledge".to_owned()]);
        let sandbox = DenySandbox;
        let result = runtime(
            value,
            ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![ToolCall {
                    tool_id: "allowed".to_owned(),
                    input: String::new(),
                }],
                capability_requests: vec![
                    CapabilityRequest::MemoryWrite {
                        value: "memory".to_owned(),
                    },
                    CapabilityRequest::MemoryRecall {
                        query: "memory".to_owned(),
                    },
                    CapabilityRequest::KnowledgeSearch {
                        query: "knowledge".to_owned(),
                    },
                ],
            },
            &tools,
            &memory,
            &knowledge,
            &sandbox,
        )
        .invoke(&AgentId::new("agent").expect("id"), String::new())
        .expect("result");
        assert_eq!(result.events.len(), 5);
    }
    #[test]
    fn provider_requested_unknown_tool_is_rejected() {
        let tools = FixedToolRegistry::new([(String::from("allowed"), String::from("tool"))]);
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;

        let result = runtime(
            definition("agent", vec!["allowed"]),
            ModelResponse {
                output: String::new(),
                tool_calls: vec![ToolCall {
                    tool_id: "unknown".to_owned(),
                    input: String::new(),
                }],
                capability_requests: vec![],
            },
            &tools,
            &memory,
            &knowledge,
            &sandbox,
        )
        .invoke(&AgentId::new("agent").expect("id"), String::new());

        assert_eq!(
            result,
            Err(DefinitionError::ToolDisallowed("unknown".to_owned()))
        );
    }

    #[test]
    fn provider_requested_definition_disallowed_tool_is_rejected() {
        let tools = FixedToolRegistry::new([
            (String::from("allowed"), String::from("allowed-output")),
            (String::from("known"), String::from("known-output")),
        ]);
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;

        let result = runtime(
            definition("agent", vec!["allowed"]),
            ModelResponse {
                output: String::new(),
                tool_calls: vec![ToolCall {
                    tool_id: "known".to_owned(),
                    input: String::new(),
                }],
                capability_requests: vec![],
            },
            &tools,
            &memory,
            &knowledge,
            &sandbox,
        )
        .invoke(&AgentId::new("agent").expect("id"), String::new());

        assert_eq!(
            result,
            Err(DefinitionError::ToolDisallowed("known".to_owned()))
        );
    }

    #[test]
    fn execution_enabled_sandbox_request_is_denied_by_deny_sandbox() {
        let tools = FixedToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let mut value = definition("agent", vec![]);
        value.sandbox.allow_execution = true;

        let result = runtime(
            value,
            ModelResponse {
                output: String::new(),
                tool_calls: vec![],
                capability_requests: vec![CapabilityRequest::SandboxExecute {
                    action: "inspect".to_owned(),
                    arguments: vec![],
                }],
            },
            &tools,
            &memory,
            &knowledge,
            &sandbox,
        )
        .invoke(&AgentId::new("agent").expect("id"), String::new());

        assert_eq!(result, Err(DefinitionError::SandboxDenied));
    }

    #[test]
    fn disabled_capabilities_reject_before_adapter_calls() {
        let tools = FixedToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        for request in [
            CapabilityRequest::MemoryRecall {
                query: String::new(),
            },
            CapabilityRequest::KnowledgeSearch {
                query: String::new(),
            },
            CapabilityRequest::SandboxExecute {
                action: "test".to_owned(),
                arguments: vec![],
            },
        ] {
            let result = runtime(
                definition("agent", vec![]),
                ModelResponse {
                    output: String::new(),
                    tool_calls: vec![],
                    capability_requests: vec![request],
                },
                &tools,
                &memory,
                &knowledge,
                &sandbox,
            )
            .invoke(&AgentId::new("agent").expect("id"), String::new());
            assert!(matches!(
                result,
                Err(DefinitionError::MemoryDenied
                    | DefinitionError::KnowledgeDenied
                    | DefinitionError::SandboxDenied)
            ));
        }
    }
    #[test]
    fn absent_references_are_rejected_before_persistence() {
        let store = InMemoryDefinitionStore::default();
        let registry = AgentRegistry::new(vec![], store, catalog()).expect("registry");
        let mut missing = definition("missing", vec![]);
        missing.model.reference = "missing-model".to_owned();
        assert_eq!(
            registry.register(missing),
            Err(DefinitionError::ReferenceUnavailable)
        );
        assert!(registry.list().expect("list").is_empty());
    }
    #[test]
    fn factory_hard_ceilings_reject_oversized_definition_and_requests() {
        let mut oversized = definition("agent", vec![]);
        oversized.limits.max_tool_calls = MAX_TOOL_CALLS + 1;
        assert_eq!(
            validate_definition(&oversized),
            Err(DefinitionError::InvalidDefinition)
        );
        let tools = FixedToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let result = runtime(
            definition("agent", vec![]),
            ModelResponse {
                output: String::new(),
                tool_calls: vec![],
                capability_requests: vec![CapabilityRequest::MemoryWrite {
                    value: "x".repeat(MAX_MEMORY_WRITE_VALUE_BYTES + 1),
                }],
            },
            &tools,
            &memory,
            &knowledge,
            &sandbox,
        )
        .invoke(&AgentId::new("agent").expect("id"), String::new());
        assert_eq!(result, Err(DefinitionError::LimitExceeded));
    }

    #[test]
    fn effective_ceiling_intersection_is_canonical_and_cannot_elevate() {
        let mut value = definition("agent", vec!["allowed"]);
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        value.sandbox.allow_execution = true;
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![
                "unknown".to_owned(),
                "allowed".to_owned(),
                "allowed".to_owned(),
            ],
            memory_enabled: true,
            knowledge_enabled: true,
            sandbox_execution_allowed: false,
            communication_allowed: true,
        };

        assert_eq!(
            ceiling.intersect(&value).expect("intersection"),
            EffectiveCapabilityCeilingV1 {
                allowed_tool_ids: vec!["allowed".to_owned()],
                memory_enabled: true,
                knowledge_enabled: false,
                sandbox_execution_allowed: false,
                communication_allowed: false,
            }
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn effective_ceiling_intersection_is_order_independent_and_non_elevating(
            definition_indices in prop::collection::vec(0_u8..8, 0..=16),
            ceiling_indices in prop::collection::vec(0_u8..8, 0..=16),
            capability_flags in prop::array::uniform4(any::<(bool, bool)>())
        ) {
            let [(definition_memory, ceiling_memory),
                (definition_knowledge, ceiling_knowledge),
                (definition_sandbox, ceiling_sandbox),
                (definition_communication, ceiling_communication)] = capability_flags;
            let definition_tools = definition_indices
                .iter()
                .map(|index| format!("tool{index}"))
                .collect::<Vec<_>>();
            let mut value = definition(
                "agent",
                definition_tools.iter().map(String::as_str).collect(),
            );
            value.memory = MemoryPolicy {
                enabled: definition_memory,
                max_items: u32::from(definition_memory),
            };
            value.knowledge = KnowledgePolicy {
                enabled: definition_knowledge,
                max_results: u32::from(definition_knowledge),
            };
            value.sandbox.allow_execution = definition_sandbox;
            value.communication.allow_messages = definition_communication;
            prop_assert_eq!(validate_definition(&value), Ok(()));

            let ceiling = EffectiveCapabilityCeilingV1 {
                allowed_tool_ids: ceiling_indices
                    .iter()
                    .map(|index| format!("tool{index}"))
                    .collect(),
                memory_enabled: ceiling_memory,
                knowledge_enabled: ceiling_knowledge,
                sandbox_execution_allowed: ceiling_sandbox,
                communication_allowed: ceiling_communication,
            };
            prop_assert_eq!(validate_effective_capability_ceiling(&ceiling), Ok(()));

            let intersection = ceiling.intersect(&value).expect("intersection");
            let expected_tools = definition_indices
                .iter()
                .filter(|index| ceiling_indices.contains(index))
                .map(|index| format!("tool{index}"))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            prop_assert_eq!(&intersection.allowed_tool_ids, &expected_tools);
            prop_assert!(intersection.allowed_tool_ids.windows(2).all(|pair| pair[0] < pair[1]));
            let all_tools_are_shared = intersection.allowed_tool_ids.iter().all(|tool_id| {
                value.allowed_tool_ids.contains(tool_id) && ceiling.allowed_tool_ids.contains(tool_id)
            });
            prop_assert!(all_tools_are_shared);
            prop_assert_eq!(intersection.memory_enabled, definition_memory && ceiling_memory);
            prop_assert_eq!(intersection.knowledge_enabled, definition_knowledge && ceiling_knowledge);
            prop_assert_eq!(
                intersection.sandbox_execution_allowed,
                definition_sandbox && ceiling_sandbox
            );
            prop_assert_eq!(
                intersection.communication_allowed,
                definition_communication && ceiling_communication
            );

            let mut reordered_definition = value.clone();
            reordered_definition.allowed_tool_ids.reverse();
            reordered_definition
                .allowed_tool_ids
                .extend(value.allowed_tool_ids.clone());
            let mut reordered_ceiling = ceiling.clone();
            reordered_ceiling.allowed_tool_ids.reverse();
            reordered_ceiling
                .allowed_tool_ids
                .extend(ceiling.allowed_tool_ids.clone());
            prop_assert_eq!(
                reordered_ceiling
                    .intersect(&reordered_definition)
                    .expect("reordered intersection"),
                intersection
            );
        }
    }

    #[test]
    fn effective_ceiling_changes_scope_digest() {
        let value = definition("agent", vec!["allowed"]);
        let tools = vec![ToolDescriptor {
            id: "allowed".to_owned(),
        }];
        let full = EffectiveCapabilityCeilingV1::full_definition(&value);
        let denied = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };

        assert_ne!(
            capability_scope_digest_with_ceiling(&value, &tools, &full).expect("digest"),
            capability_scope_digest_with_ceiling(&value, &tools, &denied).expect("digest")
        );
    }

    #[test]
    fn ceiling_scope_excludes_denied_capabilities_and_compatibility_invoke_is_full_scope() {
        let mut value = definition("agent", vec!["allowed"]);
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        value.knowledge = KnowledgePolicy {
            enabled: true,
            max_results: 1,
        };
        value.sandbox.allow_execution = true;
        value.communication.allow_messages = true;
        let registry = AgentRegistry::new(
            vec![value.clone()],
            InMemoryDefinitionStore::default(),
            catalog(),
        )
        .expect("registry");
        let model = RecordingModel {
            request: Mutex::new(None),
            response: ModelResponse {
                output: String::new(),
                tool_calls: vec![],
                capability_requests: vec![],
            },
        };
        let tools = FixedToolRegistry::new([(String::from("allowed"), String::new())]);
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let runtime =
            LocalAgentRuntime::new(&registry, &model, &tools, &memory, &knowledge, &sandbox);
        let id = AgentId::new("agent").expect("id");
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };

        let denied = runtime
            .invoke_with_ceiling(&id, String::new(), &ceiling)
            .expect("denied scope invocation");
        let denied_scope = model
            .request
            .lock()
            .expect("request lock")
            .as_ref()
            .expect("model request")
            .capability_scope
            .clone();
        assert!(denied_scope.allowed_tool_ids.is_empty());
        assert!(!denied_scope.memory.enabled);
        assert!(!denied_scope.knowledge.enabled);
        assert!(!denied_scope.sandbox.allow_execution);
        assert!(!denied_scope.communication.allow_messages);
        let full = runtime
            .invoke(&id, String::new())
            .expect("compatibility invocation");
        assert_ne!(denied.capability_scope_digest, full.capability_scope_digest);
        assert_eq!(
            full.capability_scope_digest,
            capability_scope_digest(
                &value,
                &[ToolDescriptor {
                    id: "allowed".to_owned()
                }]
            )
        );
    }

    #[test]
    fn ceiling_denies_requests_before_the_corresponding_adapter_operation() {
        let mut value = definition("agent", vec!["allowed"]);
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        value.knowledge = KnowledgePolicy {
            enabled: true,
            max_results: 1,
        };
        value.sandbox.allow_execution = true;
        let tools = FixedToolRegistry::new([(String::from("allowed"), String::new())]);
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };
        for (response, expected) in [
            (
                ModelResponse {
                    output: String::new(),
                    tool_calls: vec![ToolCall {
                        tool_id: "allowed".to_owned(),
                        input: String::new(),
                    }],
                    capability_requests: vec![],
                },
                DefinitionError::ToolDisallowed("allowed".to_owned()),
            ),
            (
                ModelResponse {
                    output: String::new(),
                    tool_calls: vec![],
                    capability_requests: vec![CapabilityRequest::MemoryRecall {
                        query: String::new(),
                    }],
                },
                DefinitionError::MemoryDenied,
            ),
            (
                ModelResponse {
                    output: String::new(),
                    tool_calls: vec![],
                    capability_requests: vec![CapabilityRequest::KnowledgeSearch {
                        query: String::new(),
                    }],
                },
                DefinitionError::KnowledgeDenied,
            ),
            (
                ModelResponse {
                    output: String::new(),
                    tool_calls: vec![],
                    capability_requests: vec![CapabilityRequest::SandboxExecute {
                        action: "inspect".to_owned(),
                        arguments: vec![],
                    }],
                },
                DefinitionError::SandboxDenied,
            ),
        ] {
            let result = runtime(
                value.clone(),
                response,
                &tools,
                &memory,
                &knowledge,
                &sandbox,
            )
            .invoke_with_ceiling(
                &AgentId::new("agent").expect("id"),
                String::new(),
                &ceiling,
            );
            assert_eq!(result, Err(expected));
        }
    }

    #[test]
    fn invalid_ceiling_is_rejected_before_the_model_is_called() {
        let value = definition("agent", vec![]);
        let registry =
            AgentRegistry::new(vec![value], InMemoryDefinitionStore::default(), catalog())
                .expect("registry");
        let model = RecordingModel {
            request: Mutex::new(None),
            response: ModelResponse {
                output: String::new(),
                tool_calls: vec![],
                capability_requests: vec![],
            },
        };
        let tools = RecordingToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let runtime =
            LocalAgentRuntime::new(&registry, &model, &tools, &memory, &knowledge, &sandbox);
        let malformed_ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec!["Invalid Tool".to_owned()],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };

        assert_eq!(
            runtime.invoke_with_ceiling(
                &AgentId::new("agent").expect("id"),
                String::new(),
                &malformed_ceiling,
            ),
            Err(DefinitionError::InvalidReference)
        );
        assert!(model.request.lock().expect("request lock").is_none());
        assert!(
            tools
                .resolved
                .lock()
                .expect("resolved tools lock")
                .is_empty()
        );
    }

    #[test]
    fn invocation_paths_load_one_definition_snapshot_each() {
        let value = definition("agent", vec![]);
        let loads = Arc::new(Mutex::new(0));
        let registry = AgentRegistry::new(
            vec![],
            RecordingDefinitionStore {
                definition: value,
                loads: Arc::clone(&loads),
            },
            catalog(),
        )
        .expect("registry");
        let model = StaticModelProvider::new(ModelResponse {
            output: String::new(),
            tool_calls: vec![],
            capability_requests: vec![],
        });
        let tools = FixedToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let runtime =
            LocalAgentRuntime::new(&registry, &model, &tools, &memory, &knowledge, &sandbox);
        let id = AgentId::new("agent").expect("id");
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };

        runtime
            .invoke(&id, String::new())
            .expect("legacy invocation");
        assert_eq!(*loads.lock().expect("load count lock"), 1);
        runtime
            .invoke_with_ceiling(&id, String::new(), &ceiling)
            .expect("ceiling invocation");
        assert_eq!(*loads.lock().expect("load count lock"), 2);
    }

    #[test]
    fn ceiling_denied_model_tool_is_rejected_before_registry_resolution_or_invocation() {
        let value = definition("agent", vec!["allowed"]);
        let registry =
            AgentRegistry::new(vec![value], InMemoryDefinitionStore::default(), catalog())
                .expect("registry");
        let model = StaticModelProvider::new(ModelResponse {
            output: String::new(),
            tool_calls: vec![ToolCall {
                tool_id: "allowed".to_owned(),
                input: String::new(),
            }],
            capability_requests: vec![],
        });
        let tools = RecordingToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let runtime =
            LocalAgentRuntime::new(&registry, &model, &tools, &memory, &knowledge, &sandbox);
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };

        assert_eq!(
            runtime.invoke_with_ceiling(
                &AgentId::new("agent").expect("id"),
                String::new(),
                &ceiling,
            ),
            Err(DefinitionError::ToolDisallowed("allowed".to_owned()))
        );
        assert!(
            tools
                .resolved
                .lock()
                .expect("resolved tools lock")
                .is_empty()
        );
        assert!(tools.invoked.lock().expect("invoked tools lock").is_empty());
    }

    #[test]
    fn ceiling_denied_tools_do_not_reach_the_tool_registry() {
        let value = definition("agent", vec!["allowed"]);
        let registry =
            AgentRegistry::new(vec![value], InMemoryDefinitionStore::default(), catalog())
                .expect("registry");
        let model = StaticModelProvider::new(ModelResponse {
            output: String::new(),
            tool_calls: vec![],
            capability_requests: vec![],
        });
        let tools = RecordingToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let runtime =
            LocalAgentRuntime::new(&registry, &model, &tools, &memory, &knowledge, &sandbox);
        let denying_ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };

        runtime
            .invoke_with_ceiling(
                &AgentId::new("agent").expect("id"),
                String::new(),
                &denying_ceiling,
            )
            .expect("a ceiling may remove every tool without blocking invocation");
        assert!(
            tools
                .resolved
                .lock()
                .expect("resolved tools lock")
                .is_empty(),
            "a ceiling-denied tool must not reach ToolRegistry::resolve"
        );
        assert!(
            tools.invoked.lock().expect("invoked tools lock").is_empty(),
            "a ceiling-denied tool must not reach ToolRegistry::invoke"
        );
    }
}
