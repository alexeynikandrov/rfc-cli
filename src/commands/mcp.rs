use serde_json::{json, Map, Value};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::rfclib::{index, rfc};

const JSONRPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Executes the `mcp` command: runs the MCP server over stdio.
pub fn execute(project_root: &Path) -> Result<(), String> {
    serve_stdio(project_root)
}

fn serve_stdio(project_root: &Path) -> Result<(), String> {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());

    while let Some(request) = read_message(&mut reader)? {
        if let Some(response) = handle_request(project_root, &request)? {
            write_message(&mut writer, &response)?;
            writer
                .flush()
                .map_err(|e| format!("Failed to flush stdout: {}", e))?;
        }
    }

    Ok(())
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<Value>, String> {
    let first_non_whitespace = loop {
        let buffer = reader
            .fill_buf()
            .map_err(|e| format!("Failed to read MCP message: {}", e))?;
        if buffer.is_empty() {
            return Ok(None);
        }

        let mut consumed = 0usize;
        while consumed < buffer.len() && matches!(buffer[consumed], b' ' | b'\t' | b'\r' | b'\n') {
            consumed += 1;
        }

        if consumed > 0 {
            reader.consume(consumed);
            continue;
        }

        break buffer[0];
    };

    if first_non_whitespace == b'{' || first_non_whitespace == b'[' {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read MCP JSON line: {}", e))?;
        if bytes == 0 {
            return Ok(None);
        }
        let message = serde_json::from_str(line.trim())
            .map_err(|e| format!("Failed to parse MCP JSON message: {}", e))?;
        return Ok(Some(message));
    }

    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read MCP header line: {}", e))?;
        if bytes == 0 {
            return Ok(None);
        }

        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }

        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = Some(value.trim().parse::<usize>().map_err(|e| {
                    format!("Invalid Content-Length value '{}': {}", value.trim(), e)
                })?);
            }
        }
    }

    let length = content_length.ok_or_else(|| "Missing Content-Length header".to_string())?;
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|e| format!("Failed to read MCP message body: {}", e))?;

    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| format!("Failed to parse MCP JSON message: {}", e))
}

fn write_message<W: Write>(writer: &mut W, message: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(message)
        .map_err(|e| format!("Failed to serialize MCP response: {}", e))?;
    writer
        .write_all(&body)
        .map_err(|e| format!("Failed to write MCP response: {}", e))?;
    writer
        .write_all(b"\n")
        .map_err(|e| format!("Failed to write MCP response newline: {}", e))?;
    Ok(())
}

fn handle_request(project_root: &Path, request: &Value) -> Result<Option<Value>, String> {
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "MCP message missing method".to_string())?;
    let Some(id) = request.get("id").cloned() else {
        return Ok(None);
    };
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    let response = match method {
        "initialize" => success(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "rfc-cli",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "ping" => success(id, json!({})),
        "tools/list" => success(id, json!({ "tools": tool_specs() })),
        "tools/call" => handle_tool_call(project_root, id, &params),
        _ => error(id, -32601, &format!("Method not found: {}", method)),
    };

    Ok(Some(response))
}

fn handle_tool_call(project_root: &Path, id: Value, params: &Value) -> Value {
    let name = match params.get("name").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => name,
        _ => return error(id, -32602, "tools/call requires non-empty field: name"),
    };
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match execute_tool(project_root, name, &arguments) {
        Ok(structured) => {
            let text = match serde_json::to_string(&structured) {
                Ok(text) => text,
                Err(e) => return error(id, -32603, &format!("Failed to serialize result: {}", e)),
            };
            success(
                id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "structuredContent": structured,
                    "isError": false
                }),
            )
        }
        Err(ToolError::InvalidParams(message)) => error(id, -32602, &message),
        Err(ToolError::Internal(message)) => error(id, -32603, &message),
    }
}

#[derive(Debug)]
enum ToolError {
    InvalidParams(String),
    Internal(String),
}

fn execute_tool(project_root: &Path, name: &str, arguments: &Value) -> Result<Value, ToolError> {
    match name {
        "ping" => Ok(json!({"ok": true, "message": "pong"})),
        "list_rfcs" => list_rfcs(project_root, arguments),
        "view_rfc" => view_rfc(project_root, arguments),
        "get_rfc_status" => get_rfc_status(project_root, arguments),
        "get_rfc_dependencies" => get_rfc_dependencies(project_root, arguments),
        _ => Err(invalid_params(format!("Unknown tool: {}", name))),
    }
}

fn tool_specs() -> Vec<Value> {
    vec![
        json!({
            "name": "ping",
            "description": "Health check for the RFC MCP server.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }),
        json!({
            "name": "list_rfcs",
            "description": "List RFCs, optionally filtered by status.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": rfc::VALID_STATUSES
                    }
                },
                "additionalProperties": false
            }
        }),
        json!({
            "name": "view_rfc",
            "description": "Read the complete Markdown content of an RFC.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "number": {"type": "string"}
                },
                "required": ["number"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "get_rfc_status",
            "description": "Get the current status of an RFC.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "number": {"type": "string"}
                },
                "required": ["number"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "get_rfc_dependencies",
            "description": "Get forward or reverse dependencies of an RFC.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "number": {"type": "string"},
                    "reverse": {"type": "boolean"}
                },
                "required": ["number"],
                "additionalProperties": false
            }
        }),
    ]
}

fn list_rfcs(project_root: &Path, arguments: &Value) -> Result<Value, ToolError> {
    let arguments = arg_object(arguments)?;
    let status = get_optional_string(arguments, "status")?;
    let index = load_readonly_index(project_root)?;

    let rfcs = index
        .rfcs
        .iter()
        .filter(|entry| status.map(|filter| entry.status == filter).unwrap_or(true))
        .map(index_entry_to_json)
        .collect::<Vec<_>>();

    Ok(json!({"rfcs": rfcs}))
}

fn view_rfc(project_root: &Path, arguments: &Value) -> Result<Value, ToolError> {
    let arguments = arg_object(arguments)?;
    let number = get_required_string(arguments, "number")?;
    let normalized = rfc::normalize_number(number).map_err(invalid_params)?;
    let path = rfc::rfc_path(project_root, number).map_err(internal)?;

    if !path.exists() {
        return Err(invalid_params(format!("RFC-{} not found.", normalized)));
    }

    let content = std::fs::read_to_string(&path)
        .map_err(|e| internal(format!("Failed to read {}: {}", path.display(), e)))?;
    Ok(json!({"number": normalized, "content": content}))
}

fn get_rfc_status(project_root: &Path, arguments: &Value) -> Result<Value, ToolError> {
    let arguments = arg_object(arguments)?;
    let number = get_required_string(arguments, "number")?;
    let normalized = rfc::normalize_number(number).map_err(invalid_params)?;
    let index = load_readonly_index(project_root)?;
    let entry = index
        .rfcs
        .iter()
        .find(|entry| entry.number == normalized)
        .ok_or_else(|| invalid_params(format!("RFC-{} not found.", normalized)))?;

    Ok(json!({"number": normalized, "status": entry.status}))
}

fn get_rfc_dependencies(project_root: &Path, arguments: &Value) -> Result<Value, ToolError> {
    let arguments = arg_object(arguments)?;
    let number = get_required_string(arguments, "number")?;
    let reverse = get_optional_bool(arguments, "reverse")?.unwrap_or(false);
    let normalized = rfc::normalize_number(number).map_err(invalid_params)?;
    let index = load_readonly_index(project_root)?;

    let entry = index
        .rfcs
        .iter()
        .find(|entry| entry.number == normalized)
        .ok_or_else(|| invalid_params(format!("RFC-{} not found.", normalized)))?;

    if reverse {
        let reference = format!("RFC-{}", normalized);
        let dependencies = index
            .rfcs
            .iter()
            .filter(|candidate| candidate.dependencies.contains(&reference))
            .map(index_entry_to_dependency_json)
            .collect::<Vec<_>>();
        return Ok(json!({
            "number": normalized,
            "reverse": true,
            "dependencies": dependencies
        }));
    }

    let dependencies = entry
        .dependencies
        .iter()
        .map(|reference| {
            let dependency_number = reference.strip_prefix("RFC-").unwrap_or(reference);
            match rfc::normalize_number(dependency_number) {
                Ok(normalized_dependency) => match index
                    .rfcs
                    .iter()
                    .find(|candidate| candidate.number == normalized_dependency)
                {
                    Some(candidate) => index_entry_to_dependency_json(candidate),
                    None => json!({
                        "reference": reference,
                        "number": normalized_dependency,
                        "not_found": true
                    }),
                },
                Err(_) => json!({
                    "reference": reference,
                    "invalid": true
                }),
            }
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "number": normalized,
        "reverse": false,
        "dependencies": dependencies
    }))
}

fn load_readonly_index(project_root: &Path) -> Result<index::Index, ToolError> {
    let rfcs_dir = project_root.join("docs/rfcs");
    if !rfcs_dir.exists() {
        return Err(internal(
            "docs/rfcs/ not found. Run \"rfc-cli init\" first.".to_string(),
        ));
    }

    let mut index = index::load_index(project_root).map_err(internal)?;
    index::refresh_index_readonly(project_root, &mut index).map_err(internal)?;
    Ok(index)
}

fn index_entry_to_json(entry: &index::IndexEntry) -> Value {
    json!({
        "number": entry.number,
        "title": entry.title,
        "status": entry.status,
        "dependencies": entry.dependencies,
        "superseded_by": entry.superseded_by,
        "links": entry.links
    })
}

fn index_entry_to_dependency_json(entry: &index::IndexEntry) -> Value {
    json!({
        "number": entry.number,
        "title": entry.title,
        "status": entry.status
    })
}

fn arg_object(arguments: &Value) -> Result<&Map<String, Value>, ToolError> {
    arguments
        .as_object()
        .ok_or_else(|| invalid_params("Tool arguments must be an object"))
}

fn get_required_string<'a>(
    arguments: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, ToolError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_params(format!("Missing or invalid field: {}", field)))
}

fn get_optional_string<'a>(
    arguments: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, ToolError> {
    let Some(value) = arguments.get(field) else {
        return Ok(None);
    };

    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(Some)
        .ok_or_else(|| invalid_params(format!("Invalid field: {}", field)))
}

fn get_optional_bool(
    arguments: &Map<String, Value>,
    field: &str,
) -> Result<Option<bool>, ToolError> {
    let Some(value) = arguments.get(field) else {
        return Ok(None);
    };

    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| invalid_params(format!("Invalid field: {}", field)))
}

fn invalid_params(message: impl Into<String>) -> ToolError {
    ToolError::InvalidParams(message.into())
}

fn internal(message: impl Into<String>) -> ToolError {
    ToolError::Internal(message.into())
}

fn success(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "result": result
    })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;

    fn temp_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rfc_cli_mcp_{}", name));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(root.join("docs/rfcs")).unwrap();
        std::fs::write(root.join("docs/rfcs/.index.json"), r#"{"rfcs":[]}"#).unwrap();
        std::fs::write(
            root.join("docs/rfcs/0001.md"),
            "---\ntitle: \"RFC-0001: test\"\nstatus: draft\ndependencies: []\nsuperseded_by: null\nlinks: []\n---\n\n## Problem\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn tools_list_contains_read_only_tools() {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let response = handle_request(Path::new("."), &request).unwrap().unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();

        assert!(tools.iter().any(|tool| tool["name"] == "list_rfcs"));
        assert!(tools.iter().any(|tool| tool["name"] == "view_rfc"));
        assert!(tools.iter().any(|tool| tool["name"] == "get_rfc_status"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "get_rfc_dependencies"));
    }

    #[test]
    fn list_tool_returns_structured_content_without_writing_index() {
        let root = temp_project("list");
        let index_path = root.join("docs/rfcs/.index.json");
        let before = std::fs::read_to_string(&index_path).unwrap();
        let request = json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params":{"name":"list_rfcs","arguments":{}}
        });

        let response = handle_request(&root, &request).unwrap().unwrap();
        assert_eq!(
            response["result"]["structuredContent"]["rfcs"][0]["number"],
            "0001"
        );
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"0001\""));
        assert_eq!(std::fs::read_to_string(index_path).unwrap(), before);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_message_supports_raw_json_and_content_length() {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        let body = serde_json::to_string(&request).unwrap();

        let mut raw_reader = BufReader::new(Cursor::new(format!("{}\n", body)));
        assert_eq!(read_message(&mut raw_reader).unwrap().unwrap(), request);

        let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut framed_reader = BufReader::new(Cursor::new(framed));
        assert_eq!(read_message(&mut framed_reader).unwrap().unwrap(), request);
    }

    #[test]
    fn write_message_uses_newline_delimited_json() {
        let message = json!({"jsonrpc":"2.0","id":1,"result":{}});
        let mut output = Vec::new();
        write_message(&mut output, &message).unwrap();
        let text = String::from_utf8(output).unwrap();

        assert!(!text.contains("Content-Length"));
        assert_eq!(serde_json::from_str::<Value>(text.trim()).unwrap(), message);
    }
}
