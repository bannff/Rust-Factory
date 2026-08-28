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
    Clock, EventName, EventTarget, ObservabilityError, PublicErrorCode, Severity, TelemetryContext,
    TelemetryQueryV1, TelemetryReader, TelemetryRecordV1, TelemetryService, TelemetrySink,
    TenantId, Timestamp,
};

pub const OBSERVABILITY_TOOLS: [&str; 2] = [
    "observability_telemetry_query",
    "observability_telemetry_status",
];
pub const MAX_MCP_REQUEST_BYTES: usize = 16 * 1024;
/// Raw serialized response ceiling, sized for worst-case JSON string escaping.
pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = MAX_MCP_ESCAPED_TOOL_TEXT_BYTES / 2;
/// Brick-local ceiling after the response is escaped as MCP tool text.
///
/// The complete JSON-RPC envelope and caller-controlled request ID are owned by
/// the composition root, so this constant does not claim full-frame safety.
pub const MAX_MCP_ESCAPED_TOOL_TEXT_BYTES: usize =
    BRICK_TOOL_TEXT_BUDGET_BYTES - COMPOSITION_HEADROOM_BYTES;
const BRICK_TOOL_TEXT_BUDGET_BYTES: usize = 64 * 1024;
const COMPOSITION_HEADROOM_BYTES: usize = 8 * 1024;

pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> Result<TrustedContextV1, ObservabilityError>;
}

pub struct ObservabilityPolicyContextResolver<T, P> {
    source: T,
    policy: P,
}
impl<T, P> ObservabilityPolicyContextResolver<T, P>
where
    T: TrustedContextSource,
    P: PolicyResolver,
{
    #[must_use]
    pub const fn new(source: T, policy: P) -> Self {
        Self { source, policy }
    }

    fn authorize(&self, capability: CapabilityV1) -> Result<TelemetryContext, ObservabilityError> {
        let trusted = self
            .source
            .resolve()
            .map_err(|_| ObservabilityError::AdapterFailure)?;
        let request = AuthorizationRequestV1 {
            context: trusted.clone(),
            capability,
        };
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest: supplied_digest,
        } = self.policy.authorize(request.clone())
        else {
            return Err(ObservabilityError::AdapterFailure);
        };
        let effective_grant =
            canonical_grant(&effective_grant).map_err(|_| ObservabilityError::AdapterFailure)?;
        let expected_digest = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest: String::new(),
            },
        )
        .map_err(|_| ObservabilityError::AdapterFailure)?;
        if supplied_digest != expected_digest {
            return Err(ObservabilityError::AdapterFailure);
        }
        let tenant_id = TenantId::new(trusted.tenant_id.as_str())
            .map_err(|_| ObservabilityError::AdapterFailure)?;
        Ok(TelemetryContext::new(tenant_id))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeverityInput {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
impl From<SeverityInput> for Severity {
    fn from(value: SeverityInput) -> Self {
        match value {
            SeverityInput::Trace => Self::Trace,
            SeverityInput::Debug => Self::Debug,
            SeverityInput::Info => Self::Info,
            SeverityInput::Warn => Self::Warn,
            SeverityInput::Error => Self::Error,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryQueryInput {
    pub since_unix_nanos: Option<u64>,
    pub until_unix_nanos: Option<u64>,
    pub minimum_severity: Option<SeverityInput>,
    pub event_name: Option<String>,
    pub target: Option<String>,
    pub limit: usize,
}
impl TelemetryQueryInput {
    fn into_core(self) -> Result<TelemetryQueryV1, ObservabilityError> {
        Ok(TelemetryQueryV1 {
            since: self.since_unix_nanos.map(Timestamp::from_unix_nanos),
            until: self.until_unix_nanos.map(Timestamp::from_unix_nanos),
            minimum_severity: self.minimum_severity.map(Into::into),
            event_name: self.event_name.map(EventName::new).transpose()?,
            target: self.target.map(EventTarget::new).transpose()?,
            limit: self.limit,
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryStatusInput {}

#[derive(Serialize)]
struct RecordOutput<'a> {
    sequence: u64,
    timestamp_unix_nanos: u64,
    severity: &'static str,
    event_name: &'a str,
    target: &'a str,
}

pub struct ObservabilityMcp<S, R, C, T, P>
where
    S: TelemetrySink,
    R: TelemetryReader,
    C: Clock,
    T: TrustedContextSource,
    P: PolicyResolver,
{
    service: TelemetryService<S, R, C>,
    resolver: ObservabilityPolicyContextResolver<T, P>,
    tool_router: ToolRouter<Self>,
}
impl<S, R, C, T, P> ObservabilityMcp<S, R, C, T, P>
where
    S: TelemetrySink + 'static,
    R: TelemetryReader + 'static,
    C: Clock + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[must_use]
    pub fn new(
        service: TelemetryService<S, R, C>,
        resolver: ObservabilityPolicyContextResolver<T, P>,
    ) -> Self {
        Self {
            service,
            resolver,
            tool_router: Self::tool_router(),
        }
    }

    fn query_json(&self, input: TelemetryQueryInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        let context = self
            .resolver
            .authorize(CapabilityV1::ObservabilityTelemetryQuery)
            .map_err(public_error)?;
        let query = input.into_core().map_err(public_error)?;
        self.service.validate_query(&query).map_err(public_error)?;
        let records = self.service.query(&context, &query).map_err(public_error)?;
        serialize_records(&records)
    }

    fn status_json(&self, input: TelemetryStatusInput) -> Result<String> {
        validate_request(&input).map_err(public_error)?;
        let _context = self
            .resolver
            .authorize(CapabilityV1::ObservabilityTelemetryStatus)
            .map_err(public_error)?;
        let guarantees = self.service.guarantees();
        serialize(&json!({
            "durable_across_restart": guarantees.durable_across_restart,
            "visible_across_processes": guarantees.visible_across_processes,
            "delivery_confirmed": guarantees.delivery_confirmed,
            "queryable": guarantees.queryable,
        }))
    }
}

#[tool_router(router = tool_router)]
impl<S, R, C, T, P> ObservabilityMcp<S, R, C, T, P>
where
    S: TelemetrySink + 'static,
    R: TelemetryReader + 'static,
    C: Clock + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[tool(
        name = "observability_telemetry_query",
        description = "Query bounded tenant-scoped operational log telemetry."
    )]
    async fn observability_telemetry_query(
        &self,
        Parameters(input): Parameters<TelemetryQueryInput>,
    ) -> String {
        tool_response(self.query_json(input))
    }

    #[tool(
        name = "observability_telemetry_status",
        description = "Report the configured telemetry adapter guarantees."
    )]
    async fn observability_telemetry_status(
        &self,
        Parameters(input): Parameters<TelemetryStatusInput>,
    ) -> String {
        tool_response(self.status_json(input))
    }
}
#[tool_handler(router = self.tool_router)]
impl<S, R, C, T, P> ServerHandler for ObservabilityMcp<S, R, C, T, P>
where
    S: TelemetrySink + 'static,
    R: TelemetryReader + 'static,
    C: Clock + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}

fn serialize_records(records: &[TelemetryRecordV1]) -> Result<String> {
    let outputs = records.iter().map(record_output).collect::<Vec<_>>();
    for length in (0..=outputs.len()).rev() {
        let value = serde_json::to_string(&json!({
            "records": &outputs[..length],
            "truncated": length < outputs.len(),
        }))
        .context("could not serialize MCP response")?;
        if fits_result_ceiling(&value)? {
            return Ok(value);
        }
    }
    Err(anyhow::anyhow!("limit_exceeded"))
}
fn record_output(record: &TelemetryRecordV1) -> RecordOutput<'_> {
    RecordOutput {
        sequence: record.sequence,
        timestamp_unix_nanos: record.envelope.timestamp.as_unix_nanos(),
        severity: severity_name(record.envelope.event.severity),
        event_name: record.envelope.event.name.as_str(),
        target: record.envelope.event.target.as_str(),
    }
}
fn serialize(value: &serde_json::Value) -> Result<String> {
    let value = serde_json::to_string(value).context("could not serialize MCP response")?;
    fits_result_ceiling(&value)?
        .then_some(value)
        .ok_or_else(|| anyhow::anyhow!("limit_exceeded"))
}

fn fits_result_ceiling(value: &str) -> Result<bool> {
    Ok(value.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES
        && framed_len(value)? <= MAX_MCP_ESCAPED_TOOL_TEXT_BYTES)
}

/// Length after the response is escaped into an MCP tool result's text field.
fn framed_len(value: &str) -> Result<usize> {
    serde_json::to_string(value)
        .map(|escaped| escaped.len())
        .context("could not measure MCP response")
}
fn validate_request<T: Serialize>(input: &T) -> Result<(), ObservabilityError> {
    let bytes = serde_json::to_vec(input).map_err(|_| ObservabilityError::InvalidQuery)?;
    (bytes.len() <= MAX_MCP_REQUEST_BYTES)
        .then_some(())
        .ok_or(ObservabilityError::LimitExceeded)
}
fn public_error(error: ObservabilityError) -> anyhow::Error {
    anyhow::anyhow!(public_code(error))
}
fn public_code(error: ObservabilityError) -> &'static str {
    match error.public_code() {
        PublicErrorCode::InvalidId => "invalid_id",
        PublicErrorCode::InvalidEvent => "invalid_event",
        PublicErrorCode::InvalidQuery => "invalid_query",
        PublicErrorCode::InvalidConfiguration => "invalid_configuration",
        PublicErrorCode::LimitExceeded => "limit_exceeded",
        PublicErrorCode::OperationFailed => "operation_failed",
    }
}
fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Trace => "trace",
        Severity::Debug => "debug",
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}
fn tool_response(response: Result<String>) -> String {
    response.unwrap_or_else(|error| {
        let code = error.to_string();
        json!({"error": if matches!(code.as_str(), "invalid_id" | "invalid_event" | "invalid_query" | "invalid_configuration" | "limit_exceeded" | "operation_failed") { code.as_str() } else { "operation_failed" }}).to_string()
    })
}
#[must_use]
pub const fn tool_names() -> [&'static str; 2] {
    OBSERVABILITY_TOOLS
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use policy::{
        CorrelationId, GrantV1, PrincipalId, RequestId, TenantId as PolicyTenantId, allow_decision,
        deny_decision,
    };
    use serde_json::{Value, json};

    use super::*;
    use crate::{MAX_QUERY_LIMIT, TelemetryEnvelopeV1, TelemetryEventV1, TelemetryGuarantees};

    #[derive(Clone)]
    struct Source(Result<TrustedContextV1, ObservabilityError>);
    impl TrustedContextSource for Source {
        fn resolve(&self) -> Result<TrustedContextV1, ObservabilityError> {
            self.0.clone()
        }
    }

    #[derive(Clone)]
    struct Policy {
        allow: bool,
        tamper: bool,
        calls: Arc<Mutex<Vec<CapabilityV1>>>,
    }
    impl PolicyResolver for Policy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            self.calls.lock().expect("calls").push(request.capability);
            if !self.allow {
                return deny_decision();
            }
            let grant =
                GrantV1::new(Vec::<String>::new(), false, false, false, false).expect("grant");
            let mut decision = allow_decision(&request, &grant).expect("decision");
            if self.tamper {
                let AuthorizationDecisionV1::Allow {
                    decision_digest, ..
                } = &mut decision
                else {
                    unreachable!()
                };
                *decision_digest = "tampered".to_owned();
            }
            decision
        }
    }

    #[derive(Clone, Default)]
    struct Reader {
        calls: Arc<Mutex<Vec<String>>>,
        records: Arc<Mutex<Vec<TelemetryRecordV1>>>,
    }
    impl TelemetryReader for Reader {
        fn query(
            &self,
            tenant: &TenantId,
            _: &TelemetryQueryV1,
        ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError> {
            self.calls
                .lock()
                .expect("calls")
                .push(tenant.as_str().to_owned());
            Ok(self.records.lock().expect("records").clone())
        }
        fn guarantees(&self) -> TelemetryGuarantees {
            TelemetryGuarantees {
                durable_across_restart: false,
                visible_across_processes: false,
                delivery_confirmed: false,
                queryable: true,
            }
        }
    }
    struct Sink;
    impl TelemetrySink for Sink {
        fn emit(&self, _: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
            Ok(())
        }
        fn guarantees(&self) -> TelemetryGuarantees {
            TelemetryGuarantees {
                durable_across_restart: false,
                visible_across_processes: false,
                delivery_confirmed: true,
                queryable: false,
            }
        }
    }
    struct FixedClock;
    impl Clock for FixedClock {
        fn now(&self) -> Result<Timestamp, ObservabilityError> {
            Ok(Timestamp::from_unix_nanos(1))
        }
    }

    type Mcp = ObservabilityMcp<Sink, Reader, FixedClock, Source, Policy>;
    fn trusted() -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: PolicyTenantId::new("trusted-tenant").expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }
    fn mcp(
        allow: bool,
        tamper: bool,
        source: Result<TrustedContextV1, ObservabilityError>,
    ) -> (Mcp, Reader, Arc<Mutex<Vec<CapabilityV1>>>) {
        let reader = Reader::default();
        let calls = Arc::new(Mutex::new(vec![]));
        let service = TelemetryService::new(Sink, reader.clone(), FixedClock, MAX_QUERY_LIMIT)
            .expect("service");
        let resolver = ObservabilityPolicyContextResolver::new(
            Source(source),
            Policy {
                allow,
                tamper,
                calls: Arc::clone(&calls),
            },
        );
        (ObservabilityMcp::new(service, resolver), reader, calls)
    }
    fn input(limit: usize) -> TelemetryQueryInput {
        TelemetryQueryInput {
            since_unix_nanos: None,
            until_unix_nanos: None,
            minimum_severity: None,
            event_name: None,
            target: None,
            limit,
        }
    }

    #[test]
    fn exact_tools_and_closed_schemas_expose_no_identity_or_policy_fields() {
        assert_eq!(
            tool_names(),
            [
                "observability_telemetry_query",
                "observability_telemetry_status"
            ]
        );
        for schema in [
            serde_json::to_value(schemars::schema_for!(TelemetryQueryInput)).expect("query schema"),
            serde_json::to_value(schemars::schema_for!(TelemetryStatusInput))
                .expect("status schema"),
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
        assert!(
            serde_json::from_value::<TelemetryQueryInput>(json!({"limit":1,"tenant_id":"forged"}))
                .is_err()
        );
        assert!(serde_json::from_value::<TelemetryStatusInput>(json!({"policy":"allow"})).is_err());

        let query_tool = Mcp::observability_telemetry_query_tool_attr();
        let status_tool = Mcp::observability_telemetry_status_tool_attr();
        assert_eq!(query_tool.name.as_ref(), "observability_telemetry_query");
        assert_eq!(status_tool.name.as_ref(), "observability_telemetry_status");
        assert_eq!(query_tool.input_schema["additionalProperties"], false);
        assert_eq!(status_tool.input_schema["additionalProperties"], false);
    }

    #[test]
    fn request_ceiling_is_byte_exact_and_checked_before_trusted_context() {
        assert!(validate_request(&"x".repeat(MAX_MCP_REQUEST_BYTES - 2)).is_ok());
        assert_eq!(
            validate_request(&"x".repeat(MAX_MCP_REQUEST_BYTES - 1)),
            Err(ObservabilityError::LimitExceeded)
        );
        let (server, reader, policy_calls) = mcp(true, false, Ok(trusted()));
        assert_eq!(
            tool_response(server.query_json(TelemetryQueryInput {
                event_name: Some("x".repeat(MAX_MCP_REQUEST_BYTES)),
                ..input(1)
            })),
            r#"{"error":"limit_exceeded"}"#
        );
        assert!(reader.calls.lock().expect("reader").is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());
    }

    #[test]
    fn deny_tampered_digest_and_source_failure_precede_reader_effects_and_are_safe() {
        for (allow, tamper, source) in [
            (false, false, Ok(trusted())),
            (true, true, Ok(trusted())),
            (true, false, Err(ObservabilityError::InvalidId)),
        ] {
            let (server, reader, policy_calls) = mcp(allow, tamper, source.clone());
            assert_eq!(
                tool_response(server.query_json(input(1))),
                r#"{"error":"operation_failed"}"#
            );
            assert!(reader.calls.lock().expect("reader").is_empty());
            if source.is_ok() {
                assert_eq!(
                    policy_calls.lock().expect("policy").as_slice(),
                    &[CapabilityV1::ObservabilityTelemetryQuery]
                );
            } else {
                assert!(policy_calls.lock().expect("policy").is_empty());
            }
        }
    }

    #[test]
    fn denied_queries_do_not_reveal_semantic_validity() {
        for malformed in [
            TelemetryQueryInput {
                event_name: Some("Invalid".to_owned()),
                ..input(1)
            },
            TelemetryQueryInput {
                since_unix_nanos: Some(2),
                until_unix_nanos: Some(1),
                ..input(1)
            },
            input(0),
        ] {
            let (server, reader, policy_calls) = mcp(false, false, Ok(trusted()));
            assert_eq!(
                tool_response(server.query_json(malformed)),
                r#"{"error":"operation_failed"}"#
            );
            assert!(reader.calls.lock().expect("reader").is_empty());
            assert_eq!(
                policy_calls.lock().expect("policy").as_slice(),
                &[CapabilityV1::ObservabilityTelemetryQuery]
            );
        }
    }

    #[test]
    fn authorized_malformed_queries_are_validated_after_policy_but_before_reader() {
        for (malformed, expected) in [
            (
                TelemetryQueryInput {
                    event_name: Some("Invalid".to_owned()),
                    ..input(1)
                },
                r#"{"error":"invalid_event"}"#,
            ),
            (
                TelemetryQueryInput {
                    since_unix_nanos: Some(2),
                    until_unix_nanos: Some(1),
                    ..input(1)
                },
                r#"{"error":"invalid_query"}"#,
            ),
            (input(0), r#"{"error":"invalid_query"}"#),
        ] {
            let (server, reader, policy_calls) = mcp(true, false, Ok(trusted()));
            assert_eq!(tool_response(server.query_json(malformed)), expected);
            assert!(reader.calls.lock().expect("reader").is_empty());
            assert_eq!(
                policy_calls.lock().expect("policy").as_slice(),
                &[CapabilityV1::ObservabilityTelemetryQuery]
            );
        }
    }

    #[test]
    fn allowed_query_derives_trusted_tenant_and_status_uses_exact_capability_without_reader() {
        let (server, reader, policy_calls) = mcp(true, false, Ok(trusted()));
        assert_eq!(
            server.query_json(input(1)).expect("query"),
            r#"{"records":[],"truncated":false}"#
        );
        assert_eq!(
            reader.calls.lock().expect("reader").as_slice(),
            &["trusted-tenant"]
        );
        assert_eq!(
            policy_calls.lock().expect("policy").as_slice(),
            &[CapabilityV1::ObservabilityTelemetryQuery]
        );

        reader.calls.lock().expect("reader").clear();
        policy_calls.lock().expect("policy").clear();
        let status: Value =
            serde_json::from_str(&server.status_json(TelemetryStatusInput {}).expect("status"))
                .expect("JSON");
        assert_eq!(
            status,
            json!({"durable_across_restart":false,"visible_across_processes":false,"delivery_confirmed":true,"queryable":true})
        );
        assert!(reader.calls.lock().expect("reader").is_empty());
        assert_eq!(
            policy_calls.lock().expect("policy").as_slice(),
            &[CapabilityV1::ObservabilityTelemetryStatus]
        );
    }

    fn record(sequence: u64) -> TelemetryRecordV1 {
        TelemetryRecordV1 {
            sequence,
            envelope: TelemetryEnvelopeV1 {
                tenant_id: TenantId::new("trusted-tenant").expect("tenant"),
                timestamp: Timestamp::from_unix_nanos(sequence),
                event: TelemetryEventV1::new(
                    EventName::new("e".repeat(crate::MAX_IDENTIFIER_BYTES)).expect("name"),
                    EventTarget::new("t".repeat(crate::MAX_IDENTIFIER_BYTES)).expect("target"),
                    Severity::Info,
                    "é".repeat(crate::MAX_BODY_BYTES / 2),
                    BTreeMap::new(),
                )
                .expect("event"),
            },
        }
    }

    #[test]
    fn result_ceiling_checks_raw_and_escaped_tool_text_lengths() {
        assert!(fits_result_ceiling("bounded").expect("measure bounded text"));
        assert!(
            !fits_result_ceiling(&"\\".repeat(MAX_MCP_SERIALIZED_RESULT_BYTES))
                .expect("measure escaped text"),
            "a raw-budget response must still be rejected when tool-text escaping exceeds the framed budget"
        );
    }

    #[test]
    fn result_projection_is_bounded_by_truncating_whole_records() {
        let records = (1..=crate::MAX_QUERY_LIMIT)
            .map(|sequence| record(u64::try_from(sequence).expect("bounded sequence")))
            .collect::<Vec<_>>();
        let serialized = serialize_records(&records).expect("bounded result");
        assert!(serialized.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES);
        let escaped_len = framed_len(&serialized).expect("framed length");
        assert!(
            escaped_len > serialized.len(),
            "tool text must be measured after JSON string escaping"
        );
        assert!(escaped_len <= MAX_MCP_ESCAPED_TOOL_TEXT_BYTES);
        assert!(
            escaped_len + COMPOSITION_HEADROOM_BYTES <= BRICK_TOOL_TEXT_BUDGET_BYTES,
            "escaped tool text plus conservative composition headroom exceeds the brick budget"
        );
        let value: Value = serde_json::from_str(&serialized).expect("JSON");
        let projected = value["records"].as_array().expect("records");
        assert!(!projected.is_empty());
        assert!(projected.len() < records.len());
        assert_eq!(
            value["truncated"], true,
            "partial projection must be explicit"
        );
        let projected_sequences = projected
            .iter()
            .map(|record| record["sequence"].as_u64().expect("whole record sequence"))
            .collect::<Vec<_>>();
        assert_eq!(
            projected_sequences,
            (1..=u64::try_from(projected.len()).expect("bounded length")).collect::<Vec<_>>(),
            "truncation must retain complete records in their original prefix order"
        );
        assert!(!serialized.contains("trusted-tenant"));
        assert!(!serialized.contains("é"));
        assert!(projected[0].get("body").is_none());
        assert!(projected[0].get("attributes").is_none());
    }

    #[test]
    fn public_error_projection_never_leaks_internal_text() {
        assert_eq!(
            tool_response(Err(anyhow::anyhow!("secret backend path /tmp/private"))),
            r#"{"error":"operation_failed"}"#
        );
        assert_eq!(
            tool_response(Err(public_error(ObservabilityError::InvalidQuery))),
            r#"{"error":"invalid_query"}"#
        );
    }
}
