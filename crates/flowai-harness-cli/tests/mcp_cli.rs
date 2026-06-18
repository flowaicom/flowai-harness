use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;

const BIN: &str = env!("CARGO_BIN_EXE_flowai-harness");

#[test]
fn mcp_toolkit_stdio_subprocess_lists_catalog_tools() {
    let env_path = write_empty_catalog_environment();
    let mut child = Command::new(BIN)
        .args([
            "--data-environment",
            env_path.to_str().unwrap(),
            "mcp",
            "toolkit",
            "--toolkit",
            "catalog",
            "--agent",
            "mcp",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn flowai-harness");
    let mut stdin = child.stdin.take().expect("stdin");
    let responses = stdout_lines(&mut child);

    write_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "cargo-test", "version": "0.1.0"}
            }
        }),
    );
    let initialize = read_json(&responses);
    assert_eq!(initialize["id"], 1);
    assert!(initialize.get("result").is_some());

    write_json(
        &mut stdin,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    );
    let tools = read_json(&responses);
    let names = tool_names(&tools);
    assert_catalog_tool_names(&names);

    terminate(child);
    let _ = std::fs::remove_file(env_path);
}

#[test]
fn mcp_toolkit_stdio_subprocess_accepts_matching_tenant_data_environment() {
    let env_path = write_empty_catalog_environment_with_tenant("acme");
    let mut child = Command::new(BIN)
        .args([
            "--data-environment",
            env_path.to_str().unwrap(),
            "mcp",
            "toolkit",
            "--toolkit",
            "catalog",
            "--agent",
            "mcp",
            "--tenant-id",
            "acme",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn flowai-harness");
    let mut stdin = child.stdin.take().expect("stdin");
    let responses = stdout_lines(&mut child);

    write_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "cargo-test", "version": "0.1.0"}
            }
        }),
    );
    let initialize = read_json(&responses);
    assert_eq!(initialize["id"], 1);

    write_json(
        &mut stdin,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    );
    let tools = read_json(&responses);
    let names = tool_names(&tools);
    assert_catalog_tool_names(&names);

    terminate(child);
    let _ = std::fs::remove_file(env_path);
}

#[tokio::test]
async fn mcp_toolkit_streamable_http_subprocess_lists_catalog_tools() {
    let env_path = write_empty_catalog_environment();
    let mut child = Command::new(BIN)
        .args([
            "--data-environment",
            env_path.to_str().unwrap(),
            "mcp",
            "toolkit",
            "--toolkit",
            "catalog",
            "--agent",
            "mcp",
            "--transport",
            "streamable-http",
            "--port",
            "0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn flowai-harness");
    let stderr = stderr_lines(&mut child);
    let endpoint = read_endpoint(&stderr);

    let client = reqwest::Client::new();
    let initialize = post_mcp(
        &client,
        &endpoint,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "cargo-test", "version": "0.1.0"}
            }
        }),
    )
    .await;
    assert_eq!(initialize["id"], 1);

    let tools = post_mcp(
        &client,
        &endpoint,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    let names = tool_names(&tools);
    assert_catalog_tool_names(&names);

    terminate(child);
    let _ = std::fs::remove_file(env_path);
}

fn write_empty_catalog_environment() -> PathBuf {
    let path = std::env::temp_dir().join(format!("flowai-mcp-{}.json", uuid::Uuid::new_v4()));
    let index_path =
        std::env::temp_dir().join(format!("flowai-mcp-index-{}", uuid::Uuid::new_v4()));
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "kv": {"kind": "memory"},
            "catalog": {"kind": "empty"},
            "catalogSearch": {"indexPath": index_path},
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn write_empty_catalog_environment_with_tenant(tenant_id: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("flowai-mcp-{}.json", uuid::Uuid::new_v4()));
    let index_path =
        std::env::temp_dir().join(format!("flowai-mcp-index-{}", uuid::Uuid::new_v4()));
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "tenantId": tenant_id,
            "kv": {"kind": "memory"},
            "catalog": {"kind": "empty"},
            "catalogSearch": {"indexPath": index_path},
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn stdout_lines(child: &mut Child) -> mpsc::Receiver<String> {
    let stdout = child.stdout.take().expect("stdout");
    line_reader(stdout)
}

fn stderr_lines(child: &mut Child) -> mpsc::Receiver<String> {
    let stderr = child.stderr.take().expect("stderr");
    line_reader(stderr)
}

fn line_reader<R>(reader: R) -> mpsc::Receiver<String>
where
    R: std::io::Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            if let Ok(line) = line {
                let _ = tx.send(line);
            }
        }
    });
    rx
}

fn write_json(stdin: &mut ChildStdin, value: serde_json::Value) {
    writeln!(stdin, "{value}").expect("write json-rpc request");
    stdin.flush().expect("flush json-rpc request");
}

fn read_json(rx: &mpsc::Receiver<String>) -> serde_json::Value {
    let line = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("timed out waiting for json-rpc response");
    let payload = line.strip_prefix("data: ").unwrap_or(&line);
    serde_json::from_str(payload).expect("json-rpc response should be JSON")
}

fn read_endpoint(rx: &mpsc::Receiver<String>) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if line.starts_with("http://") {
                    return line;
                }
                if let Some(index) = line.find("http://") {
                    return line[index..].to_string();
                }
                seen.push(line);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("MCP subprocess stderr closed before endpoint; stderr={seen:?}");
            }
        }
    }
    panic!("timed out waiting for MCP endpoint; stderr={seen:?}");
}

async fn post_mcp(
    client: &reqwest::Client,
    endpoint: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    let response = client
        .post(endpoint)
        .header("Accept", "application/json, text/event-stream")
        .json(&value)
        .send()
        .await
        .expect("send MCP request");
    let status = response.status();
    let body = response.text().await.expect("read MCP response");
    assert!(
        status.is_success(),
        "MCP request failed with {status}: {body}"
    );
    let payload = body
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap_or(&body);
    serde_json::from_str(payload).expect("MCP response should be JSON")
}

fn tool_names(value: &serde_json::Value) -> std::collections::HashSet<String> {
    value["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

fn assert_catalog_tool_names(names: &std::collections::HashSet<String>) {
    assert_eq!(names.len(), 7);
    for name in [
        "search_catalog",
        "get_catalog_entities",
        "list_schema_fields",
        "get_catalog_relations",
        "get_relation_paths_between",
        "sample_table_data",
        "execute_query",
    ] {
        assert!(names.contains(name), "missing catalog MCP tool {name}");
    }
}

fn terminate(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}
