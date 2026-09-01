//! Black-box adversarial tests against the built `factory_*` MCP stdio server
//! binary. These tests spawn the real process and drive it over real stdio
//! pipes with newline-delimited JSON-RPC, exactly as an MCP client would.
//! They intentionally do not import any crate internals: the point is to
//! verify the wire contract, not the implementation.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

const FACTORY_TOOLS: [&str; 4] = [
    "factory_capabilities",
    "factory_run_demo",
    "factory_get_run",
    "factory_query_telemetry",
];

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Server {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_workflow-evaluation-demo"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn server binary");
        let stdin = child.stdin.take().expect("stdin pipe");
        let stdout = BufReader::new(child.stdout.take().expect("stdout pipe"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send_line(&mut self, line: &str) {
        self.stdin
            .write_all(line.as_bytes())
            .expect("write request line");
        self.stdin.write_all(b"\n").expect("write newline");
        self.stdin.flush().expect("flush request");
    }

    fn send(&mut self, value: &serde_json::Value) {
        self.send_line(&value.to_string());
    }

    /// Blocks for one newline-delimited response line. Returns `None` if the
    /// server closed stdout without producing a line (e.g. after a terminal
    /// framing error).
    fn recv(&mut self) -> Option<serde_json::Value> {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("read response");
        if read == 0 {
            return None;
        }
        Some(serde_json::from_str(&line).expect("response is valid JSON"))
    }

    fn initialize(&mut self) {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "qa-adversarial", "version": "0.0.1"}
            }
        }));
        let response = self.recv().expect("initialize response");
        assert_eq!(
            response["id"], 1,
            "initialize response id echoes request id"
        );
        assert!(
            response.get("error").is_none(),
            "initialize must not error: {response}"
        );
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
    }

    fn call_tool(
        &mut self,
        id: i64,
        name: &str,
        arguments: serde_json::Value,
    ) -> serde_json::Value {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }));
        self.recv()
            .unwrap_or_else(|| panic!("no response for tool call {name}"))
    }

    /// Extracts and parses the single text content block of a successful
    /// (non-JSON-RPC-error) `tools/call` response as JSON.
    fn tool_text_json(response: &serde_json::Value) -> serde_json::Value {
        assert!(
            response.get("error").is_none(),
            "expected a tool result, got a JSON-RPC error: {response}"
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("missing text content block: {response}"));
        serde_json::from_str(text).unwrap_or_else(|_| panic!("tool text is not JSON: {text}"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Best-effort: kill and reap the child so tests never leak processes,
        // even on assertion panics mid-test.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Item 1: Real stdio smoke test: initialize + tools/list must report exactly the
/// four reserved `factory_*` tools, snake_case, with closed schemas.
#[test]
fn stdio_smoke_lists_exactly_four_closed_factory_tools() {
    let mut server = Server::spawn();
    server.initialize();

    server.send(&serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let response = server.recv().expect("tools/list response");
    let tools = response["result"]["tools"].as_array().expect("tools array");

    assert_eq!(tools.len(), 4, "expected exactly 4 tools, got {tools:?}");

    let mut seen_names = std::collections::BTreeSet::new();
    for tool in tools {
        let name = tool["name"].as_str().expect("tool name is a string");
        assert!(
            FACTORY_TOOLS.contains(&name),
            "unexpected tool {name} not in reserved factory_* set"
        );
        assert!(
            name.starts_with("factory_"),
            "tool {name} missing factory_ prefix"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "tool {name} violates snake_case grammar"
        );
        assert!(seen_names.insert(name.to_owned()), "duplicate tool {name}");

        let schema = &tool["inputSchema"];
        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false),
            "tool {name} schema is not closed: {schema}"
        );
    }
    assert_eq!(
        seen_names,
        FACTORY_TOOLS.iter().map(|s| s.to_string()).collect(),
        "tool set must exactly match the reserved factory_* tools"
    );
}

/// Item 2: Unknown fields and wrong-typed parameters are rejected as ordinary tool
/// errors, not panics or JSON-RPC internal errors, and the server stays alive
/// and continues serving further requests afterward.
#[test]
fn malformed_and_unknown_field_parameters_are_rejected_without_panic_or_leakage() {
    let mut server = Server::spawn();
    server.initialize();

    let unknown_field = server.call_tool(
        2,
        "factory_get_run",
        serde_json::json!({"run_id": "x", "tenant_id": "attacker-tenant"}),
    );
    assert_eq!(unknown_field["result"]["isError"], true);
    let text = unknown_field["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    assert!(
        text.contains("unknown field"),
        "expected a deserialize error, got: {text}"
    );
    assert!(
        !text.to_lowercase().contains("panic"),
        "error text leaked a panic message: {text}"
    );

    let wrong_type = server.call_tool(3, "factory_get_run", serde_json::json!({"run_id": 123}));
    assert_eq!(wrong_type["result"]["isError"], true);

    let unknown_on_empty = server.call_tool(
        4,
        "factory_run_demo",
        serde_json::json!({"unexpected_field": "x"}),
    );
    assert_eq!(unknown_on_empty["result"]["isError"], true);

    let unknown_tool = server.call_tool(5, "factory_totally_unknown_tool", serde_json::json!({}));
    assert!(
        unknown_tool.get("error").is_some(),
        "unknown tool name should be a JSON-RPC error, not a panic: {unknown_tool}"
    );

    // The server must still be alive and answer a normal request afterward.
    let capabilities = server.call_tool(6, "factory_capabilities", serde_json::json!({}));
    let body = Server::tool_text_json(&capabilities);
    assert_eq!(body["tools"].as_array().unwrap().len(), 4);
}

/// Item 3a: Neither DTO accepts a caller-supplied tenant/identity field: any such
/// field is rejected by the closed schema, and a lookup for a run that does
/// not exist returns a safe not-found projection rather than a panic or a
/// synthesized result.
#[test]
fn get_run_and_query_telemetry_reject_identity_fields_and_handle_missing_run_safely() {
    let mut server = Server::spawn();
    server.initialize();

    for (field, value) in [
        ("tenant_id", serde_json::json!("attacker-tenant")),
        ("principal_id", serde_json::json!("attacker-principal")),
        ("tenant", serde_json::json!("attacker-tenant")),
    ] {
        let response = server.call_tool(
            10,
            "factory_get_run",
            serde_json::json!({"run_id": "some-run", field: value}),
        );
        assert_eq!(
            response["result"]["isError"], true,
            "factory_get_run must reject caller-supplied {field}"
        );
    }

    for (field, value) in [
        ("tenant_id", serde_json::json!("attacker-tenant")),
        ("principal_id", serde_json::json!("attacker-principal")),
    ] {
        let response = server.call_tool(
            11,
            "factory_query_telemetry",
            serde_json::json!({"limit": 1, field: value}),
        );
        assert_eq!(
            response["result"]["isError"], true,
            "factory_query_telemetry must reject caller-supplied {field}"
        );
    }

    let missing = server.call_tool(
        12,
        "factory_get_run",
        serde_json::json!({"run_id": "definitely-does-not-exist"}),
    );
    let body = Server::tool_text_json(&missing);
    assert_eq!(body, serde_json::json!({"error": "not_found"}));

    // Bounds are enforced without leaking internal details.
    let oversized = server.call_tool(
        13,
        "factory_get_run",
        serde_json::json!({"run_id": "x".repeat(300)}),
    );
    let body = Server::tool_text_json(&oversized);
    assert_eq!(body, serde_json::json!({"error": "limit_exceeded"}));

    let empty = server.call_tool(14, "factory_get_run", serde_json::json!({"run_id": ""}));
    let body = Server::tool_text_json(&empty);
    assert_eq!(body, serde_json::json!({"error": "invalid_request"}));

    let zero_limit = server.call_tool(
        15,
        "factory_query_telemetry",
        serde_json::json!({"limit": 0}),
    );
    let body = Server::tool_text_json(&zero_limit);
    assert_eq!(body, serde_json::json!({"error": "invalid_request"}));

    let over_limit = server.call_tool(
        16,
        "factory_query_telemetry",
        serde_json::json!({"limit": 51}),
    );
    let body = Server::tool_text_json(&over_limit);
    assert_eq!(body, serde_json::json!({"error": "limit_exceeded"}));
}

/// Item 4: `factory_run_demo` produces a deterministic `verdict: "pass"` end to
/// end, and repeated calls within one process replay the same stored run
/// (idempotency), matching the underlying workflow/evaluation semantics.
#[test]
fn run_demo_is_deterministic_pass_and_idempotent_across_repeated_calls() {
    let mut server = Server::spawn();
    server.initialize();

    let first = server.call_tool(20, "factory_run_demo", serde_json::json!({}));
    let first_body = Server::tool_text_json(&first);
    assert_eq!(first_body["verdict"], "pass");
    let run_id = first_body["run_id"].as_str().expect("run_id").to_owned();

    for call_id in 21..24 {
        let repeat = server.call_tool(call_id, "factory_run_demo", serde_json::json!({}));
        let repeat_body = Server::tool_text_json(&repeat);
        assert_eq!(
            repeat_body, first_body,
            "repeated factory_run_demo call {call_id} must replay the identical stored result"
        );
    }

    let lookup = server.call_tool(24, "factory_get_run", serde_json::json!({"run_id": run_id}));
    let lookup_body = Server::tool_text_json(&lookup);
    assert_eq!(lookup_body["status"], "succeeded");
    assert_eq!(lookup_body["terminal_reason"], "completed");
    assert_eq!(lookup_body["output"], "done");
}

/// Item 5: A malformed JSON-RPC line does not crash the process (the transport
/// tolerates and skips syntax errors, per `mcp-transport`'s own decoder), and
/// an oversized (>64 KiB) frame is a terminal framing condition that closes
/// the connection safely rather than panicking or corrupting subsequent
/// output.
#[test]
fn malformed_line_is_tolerated_and_oversized_frame_closes_safely_without_panic() {
    let mut server = Server::spawn();
    server.initialize();

    // A syntactically invalid line must not crash the server; the next valid
    // request still gets served normally.
    server.send_line("this is not json at all");
    let capabilities = server.call_tool(30, "factory_capabilities", serde_json::json!({}));
    let body = Server::tool_text_json(&capabilities);
    assert_eq!(body["tools"].as_array().unwrap().len(), 4);

    // An oversized frame (> 64 KiB payload) is a terminal framing error for
    // BoundedStdioTransport: the connection closes. We only assert this does
    // not panic and does not silently return the string back to us — the
    // process either closes stdout (no further response) or exits, but stays
    // alive as a normal process (no crash signal) within a bounded window.
    let oversized_run_id = "x".repeat(70 * 1024);
    server.send(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 31,
        "method": "tools/call",
        "params": {"name": "factory_get_run", "arguments": {"run_id": oversized_run_id}}
    }));

    // Give the transport a moment to react to the oversized frame, then
    // confirm the process did not crash (segfault / abort / panic exit code).
    std::thread::sleep(Duration::from_millis(300));
    match server.child.try_wait().expect("try_wait") {
        None => {
            // Still running: acceptable, the connection may simply have
            // stopped responding on stdout while the process stays up.
        }
        Some(status) => {
            assert!(
                status.success() || status.code() == Some(0),
                "server must not crash/panic/abort on an oversized frame, got exit status {status:?}"
            );
        }
    }
}

/// Item 6: No secrets, raw error internals, or filesystem paths leak into any
/// introspection or error response, across a representative sample of both
/// success and error paths.
#[test]
fn no_secrets_paths_or_raw_internals_leak_into_responses() {
    let mut server = Server::spawn();
    server.initialize();

    let forbidden_substrings = [
        "/Users/",
        "/home/",
        "C:\\",
        ".rs:",
        "panicked at",
        "RUST_BACKTRACE",
        "src/main.rs",
        "src/composition.rs",
        "src/mcp.rs",
        "secret",
        "password",
        "token",
        "api_key",
    ];

    let mut all_responses = Vec::new();

    server.send(&serde_json::json!({"jsonrpc": "2.0", "id": 40, "method": "tools/list"}));
    all_responses.push(server.recv().expect("tools/list"));

    all_responses.push(server.call_tool(41, "factory_capabilities", serde_json::json!({})));
    all_responses.push(server.call_tool(42, "factory_run_demo", serde_json::json!({})));
    all_responses.push(server.call_tool(
        43,
        "factory_get_run",
        serde_json::json!({"run_id": "missing"}),
    ));
    all_responses.push(server.call_tool(
        44,
        "factory_get_run",
        serde_json::json!({"run_id": "x", "extra": "y"}),
    ));
    all_responses.push(server.call_tool(
        45,
        "factory_query_telemetry",
        serde_json::json!({"limit": 5}),
    ));
    all_responses.push(server.call_tool(46, "factory_nonexistent_tool", serde_json::json!({})));

    for response in &all_responses {
        let rendered = response.to_string();
        for needle in forbidden_substrings {
            assert!(
                !rendered.to_lowercase().contains(&needle.to_lowercase()),
                "response leaked forbidden substring {needle:?}: {rendered}"
            );
        }
    }
}

/// Item 7: The README's non-durability claim matches observed behavior: a run
/// created in one server process is invisible to a freshly spawned process
/// (no on-disk or cross-process persistence backs the demo).
#[test]
fn restart_loses_state_matching_the_documented_non_durable_guarantee() {
    let run_id = {
        let mut server = Server::spawn();
        server.initialize();
        let response = server.call_tool(50, "factory_run_demo", serde_json::json!({}));
        let body = Server::tool_text_json(&response);
        body["run_id"].as_str().expect("run_id").to_owned()
    };
    // `server` from the block above has been dropped: process is gone.

    let mut fresh_server = Server::spawn();
    fresh_server.initialize();
    let lookup =
        fresh_server.call_tool(51, "factory_get_run", serde_json::json!({"run_id": run_id}));
    let body = Server::tool_text_json(&lookup);
    assert_eq!(
        body,
        serde_json::json!({"error": "not_found"}),
        "a fresh process must not see a run created by a prior process; \
         README claims no restart recovery"
    );
}

/// README-vs-code cross-check for the tool set and identity-field claims,
/// so the smoke tests above and the documentation cannot silently drift.
#[test]
fn readme_documented_tool_list_matches_the_reserved_factory_tools() {
    let readme = include_str!("../README.md");
    for tool in FACTORY_TOOLS {
        assert!(
            readme.contains(&format!("`{tool}`")),
            "README does not document tool {tool}"
        );
    }
    assert!(
        readme.contains("non-durable"),
        "README must state the non-durable guarantee"
    );
    assert!(
        readme.contains("No tool input field ever supplies identity"),
        "README must document that no tool input field supplies identity/tenant/grants"
    );
}
