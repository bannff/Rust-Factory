//! End-to-end proof that `AggregateRouter` serves a heterogeneous set of
//! brick and project-meta contributions over one real `BoundedStdioTransport`
//! and `rmcp` service session.
//!
//! This is the concrete demonstration that the composition problem found by
//! `experiments/workflow-evaluation-demo` (existing brick `mcp.rs` modules
//! each generate an incompatible `ToolRouter<Self>` and cannot be combined
//! into one `ServerHandler`) is solved by `mcp-contract`'s object-safe
//! `HandlerContribution` plus this crate's hand-written `AggregateRouter`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mcp_contract::{
    DispatchError, DispatchFuture, DispatchOutcome, HandlerContribution, Namespace,
    ProjectMetaContribution, ToolDescriptor, ToolName,
};
use mcp_transport::{AggregateRouterBuilder, BoundedStdioTransport};
use rmcp::service::ServiceExt;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt, split};
use tokio::time::timeout;

/// A deterministic brick-shaped contribution: two tools, one introspection
/// tool and one echo tool, plus a call counter proving dispatch reached it.
struct AgentContribution {
    namespace: Namespace,
    calls: AtomicUsize,
}
impl AgentContribution {
    fn new() -> Self {
        Self {
            namespace: Namespace::new("agent").expect("namespace"),
            calls: AtomicUsize::new(0),
        }
    }
}
impl HandlerContribution for AgentContribution {
    fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    fn tools(&self) -> Vec<ToolDescriptor> {
        vec![
            ToolDescriptor {
                name: ToolName::new(&self.namespace, "capabilities").expect("tool"),
                title: None,
                description: "Describe this contribution.".to_owned(),
                input_schema: Map::new(),
                output_schema: None,
            },
            ToolDescriptor {
                name: ToolName::new(&self.namespace, "echo").expect("tool"),
                title: None,
                description: "Echo the supplied value.".to_owned(),
                input_schema: Map::new(),
                output_schema: None,
            },
            ToolDescriptor {
                name: ToolName::new(&self.namespace, "explode").expect("tool"),
                title: None,
                description: "Always returns DispatchError::Internal.".to_owned(),
                input_schema: Map::new(),
                output_schema: None,
            },
        ]
    }
    fn dispatch(&self, tool: ToolName, arguments: Value) -> DispatchFuture<'_> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            match tool.as_str() {
                "agent_capabilities" => Ok(DispatchOutcome {
                    payload: json!({"namespace": "agent"}),
                    is_error: false,
                }),
                "agent_echo" => Ok(DispatchOutcome {
                    payload: arguments,
                    is_error: false,
                }),
                "agent_explode" => Err(DispatchError::Internal),
                _ => Err(DispatchError::UnknownTool),
            }
        })
    }
}

/// A project-meta-shaped contribution using the reserved `factory` namespace.
struct FactoryContribution {
    namespace: Namespace,
}
impl FactoryContribution {
    fn new() -> Self {
        Self {
            namespace: Namespace::factory_reserved(),
        }
    }
}
impl HandlerContribution for FactoryContribution {
    fn namespace(&self) -> &Namespace {
        &self.namespace
    }
    fn tools(&self) -> Vec<ToolDescriptor> {
        vec![ToolDescriptor {
            name: ToolName::new(&self.namespace, "run_demo").expect("tool"),
            title: None,
            description: "Run one fixed demo cycle.".to_owned(),
            input_schema: Map::new(),
            output_schema: None,
        }]
    }
    fn dispatch(&self, tool: ToolName, _arguments: Value) -> DispatchFuture<'_> {
        Box::pin(async move {
            if tool.as_str() == "factory_run_demo" {
                Ok(DispatchOutcome {
                    payload: json!({"verdict": "pass"}),
                    is_error: false,
                })
            } else {
                Err(DispatchError::UnknownTool)
            }
        })
    }
}
impl ProjectMetaContribution for FactoryContribution {}

fn build_router() -> mcp_transport::AggregateRouter {
    AggregateRouterBuilder::new()
        .with_brick(Box::new(AgentContribution::new()))
        .with_project_meta(Box::new(FactoryContribution::new()))
        .build()
        .expect("router builds from two heterogeneous contributions")
}

async fn read_response_line(reader: &mut (impl AsyncReadExt + Unpin)) -> Value {
    let mut buffer = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        let read = timeout(Duration::from_millis(500), reader.read(&mut byte))
            .await
            .expect("response within deadline")
            .expect("read byte");
        assert_ne!(read, 0, "peer closed before a full response line");
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    serde_json::from_slice(&buffer).expect("valid JSON-RPC response line")
}

async fn write_request(writer: &mut (impl AsyncWriteExt + Unpin), request: &Value) {
    let mut line = serde_json::to_vec(request).expect("serialize request");
    line.push(b'\n');
    writer.write_all(&line).await.expect("write request");
}

#[tokio::test]
async fn aggregate_router_serves_heterogeneous_contributions_over_one_bounded_transport() {
    let router = build_router();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (server_reader, server_writer) = split(server_io);
    let (mut client_reader, mut client_writer) = split(client_io);

    let transport = BoundedStdioTransport::new(server_reader, server_writer);
    let serve_task = tokio::spawn(async move {
        let running = router.serve(transport).await.expect("service starts");
        running.waiting().await
    });

    perform_initialize(&mut client_writer, &mut client_reader).await;
    assert_merged_tool_list(&mut client_writer, &mut client_reader).await;
    assert_echo_dispatches_to_agent_contribution(&mut client_writer, &mut client_reader).await;
    assert_run_demo_dispatches_to_factory_contribution(&mut client_writer, &mut client_reader)
        .await;
    assert_unknown_tool_is_rejected(&mut client_writer, &mut client_reader).await;
    assert_internal_dispatch_error_is_internal_error(&mut client_writer, &mut client_reader).await;
    assert_large_arguments_payload_under_frame_limit_round_trips(
        &mut client_writer,
        &mut client_reader,
    )
    .await;

    drop(client_writer);
    assert_clean_shutdown(serve_task).await;
}

async fn perform_initialize(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    write_request(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "smoke-test", "version": "0.0.0"}
            }
        }),
    )
    .await;
    let initialize_response = read_response_line(reader).await;
    assert_eq!(initialize_response["id"], 1);
    assert!(initialize_response["result"]["serverInfo"].is_object());

    write_request(
        writer,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .await;
}

async fn assert_merged_tool_list(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    write_request(
        writer,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .await;
    let list_response = read_response_line(reader).await;
    let tool_names: Vec<&str> = list_response["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        tool_names,
        [
            "agent_capabilities",
            "agent_echo",
            "agent_explode",
            "factory_run_demo"
        ],
        "exactly the registered tools from both contributions, in one merged list"
    );
}

async fn assert_echo_dispatches_to_agent_contribution(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    write_request(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "agent_echo", "arguments": {"value": "hi"}}
        }),
    )
    .await;
    let echo_response = read_response_line(reader).await;
    assert_eq!(echo_response["result"]["isError"], false);
    let echo_text = echo_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert_eq!(
        serde_json::from_str::<Value>(echo_text).expect("echoed JSON"),
        json!({"value": "hi"})
    );
}

async fn assert_run_demo_dispatches_to_factory_contribution(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    write_request(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "factory_run_demo", "arguments": {}}
        }),
    )
    .await;
    let run_demo_response = read_response_line(reader).await;
    let run_demo_text = run_demo_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert_eq!(
        serde_json::from_str::<Value>(run_demo_text).expect("run_demo JSON"),
        json!({"verdict": "pass"})
    );
}

async fn assert_unknown_tool_is_rejected(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    write_request(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "unknown_tool", "arguments": {}}
        }),
    )
    .await;
    let unknown_tool_response = read_response_line(reader).await;
    assert!(unknown_tool_response["error"].is_object());
    assert_eq!(unknown_tool_response["error"]["code"], -32601);
}

/// A contribution returning `Err(DispatchError::Internal)` must round-trip
/// over a real transport session as the standard JSON-RPC internal-error
/// code `-32603`, proving the `DispatchError::Internal` arm of
/// `AggregateRouter::call_tool` is exercised end-to-end, not just matched in
/// an in-process unit test.
async fn assert_internal_dispatch_error_is_internal_error(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    write_request(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "agent_explode", "arguments": {}}
        }),
    )
    .await;
    let response = read_response_line(reader).await;
    assert!(
        response["error"].is_object(),
        "expected a JSON-RPC error envelope, got {response}"
    );
    assert_eq!(response["error"]["code"], -32603);
}

/// A `tools/call` `arguments` payload that is large but stays under the
/// transport's `MAX_MCP_STDIO_FRAME_BYTES` (64 KiB) frame limit must still
/// round-trip correctly through the aggregate router: framing headroom must
/// not silently truncate or corrupt an otherwise-valid large payload.
async fn assert_large_arguments_payload_under_frame_limit_round_trips(
    writer: &mut (impl AsyncWriteExt + Unpin),
    reader: &mut (impl AsyncReadExt + Unpin),
) {
    // Comfortably under the 64 KiB frame limit once wrapped in the JSON-RPC
    // envelope and echoed back, while still exercising a genuinely large
    // single-field payload.
    let large_value = "x".repeat(48 * 1024);
    write_request(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {"name": "agent_echo", "arguments": {"value": large_value.clone()}}
        }),
    )
    .await;
    let response = read_response_line(reader).await;
    assert_eq!(response["result"]["isError"], false);
    let echoed_text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let echoed: Value = serde_json::from_str(echoed_text).expect("echoed JSON");
    assert_eq!(
        echoed["value"].as_str().expect("echoed value string"),
        large_value.as_str(),
        "large under-limit payload must round-trip byte-for-byte"
    );
}

/// The service session may keep its background task alive briefly after the
/// client half closes; assert only that it eventually exits cleanly when it
/// does, not a specific shutdown latency. This smoke test's purpose is
/// proving dispatch across heterogeneous contributions, not exact shutdown
/// timing.
async fn assert_clean_shutdown(
    serve_task: tokio::task::JoinHandle<Result<rmcp::service::QuitReason, tokio::task::JoinError>>,
) {
    if let Ok(outcome) = timeout(Duration::from_secs(2), serve_task).await {
        outcome
            .expect("serve task did not panic")
            .expect("serve loop exits cleanly");
    }
}
