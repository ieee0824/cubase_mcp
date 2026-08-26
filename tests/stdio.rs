use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn binary_keeps_mcp_on_stdout_and_logs_on_stderr() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cubase_mcp"))
        .args(["--bridge", "mock"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "integration-test", "version": "1"}
            }
        }),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "cubase.play",
                "arguments": {"sentinel": "must-not-appear-in-logs"}
            }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "cubase.get_status", "arguments": {}}
        }),
    ];

    {
        let stdin = child.stdin.as_mut().unwrap();
        for message in messages {
            writeln!(stdin, "{message}").unwrap();
        }
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[1]["result"]["isError"], true);
    assert_eq!(
        responses[1]["result"]["structuredContent"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["connected"],
        true
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("must-not-appear-in-logs"));
    let logs: Vec<Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["method"], "system.get_status");
    assert!(logs[0].get("request_id").is_some());
    assert!(logs[0].get("duration_ms").is_some());
}
