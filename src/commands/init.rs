use std::fs;
use std::path::Path;

use crate::rfclib::index::{self, Index};

/// Executes the `init` command: creates docs/rfcs/ directory and .index.json
pub fn execute(project_root: &Path) -> Result<(), String> {
    let rfcs_dir = project_root.join("docs/rfcs");
    let index_path = rfcs_dir.join(".index.json");

    let mut already_exists = true;

    if !rfcs_dir.exists() {
        fs::create_dir_all(&rfcs_dir)
            .map_err(|e| format!("Failed to create {}: {}", rfcs_dir.display(), e))?;
        println!("Created {}", rfcs_dir.display());
        already_exists = false;
    }

    if !index_path.exists() {
        let empty_index = Index::empty();
        index::save_index(project_root, &empty_index)?;
        println!("Created {}", index_path.display());
        already_exists = false;
    }

    if already_exists {
        println!("Already initialized.");
    }

    update_mcp_json(project_root)?;

    Ok(())
}

fn update_mcp_json(project_root: &Path) -> Result<(), String> {
    use serde_json::{json, Value};

    let path = project_root.join(".mcp.json");
    let mut config: Value = match fs::read_to_string(&path) {
        Ok(content) if !content.trim().is_empty() => serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?,
        Ok(_) => json!({}),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("Failed to read {}: {}", path.display(), e)),
    };

    if !config.is_object() {
        return Err(format!("{} must contain a JSON object", path.display()));
    }

    let servers = config
        .as_object_mut()
        .expect("checked that MCP config is an object")
        .entry("mcpServers")
        .or_insert_with(|| json!({}));

    if !servers.is_object() {
        return Err(format!(
            "`mcpServers` in {} must be a JSON object",
            path.display()
        ));
    }

    servers
        .as_object_mut()
        .expect("checked that mcpServers is an object")
        .insert(
            "rfc-cli".to_string(),
            json!({ "command": "rfc-cli", "args": ["mcp"] }),
        );

    let mut content = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize {}: {}", path.display(), e))?;
    content.push('\n');
    fs::write(&path, content).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}
