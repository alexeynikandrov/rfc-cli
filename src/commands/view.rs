use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::rfclib::rfc;

/// Executes the `view` command: prints RFC content to stdout,
/// or renders it through an external viewer set via $RFC_VIEWER.
pub fn execute(project_root: &Path, number: &str, raw: bool) -> Result<(), String> {
    let normalized = rfc::normalize_number(number)?;
    let path = rfc::rfc_path(project_root, number)?;

    if !path.exists() {
        return Err(format!("RFC-{} not found.", normalized));
    }

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // --raw forces raw output and ignores RFC_VIEWER
    if raw {
        print!("{}", content);
        return Ok(());
    }

    // RFC_VIEWER is set and non-empty — render through an external program
    match env::var("RFC_VIEWER") {
        Ok(viewer) if !viewer.trim().is_empty() => render_with_viewer(&viewer, &content),
        _ => {
            print!("{}", content);
            Ok(())
        }
    }
}

/// Runs the external viewer, passing RFC content on stdin.
fn render_with_viewer(viewer: &str, content: &str) -> Result<(), String> {
    let mut parts = viewer.split_whitespace();
    let program = parts.next().unwrap(); // non-empty string guaranteed by caller
    let args: Vec<&str> = parts.collect();

    let mut child = Command::new(program)
        .args(&args)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to launch viewer '{}': {}", viewer, e))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("Failed to write to viewer '{}': {}", viewer, e))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for viewer '{}': {}", viewer, e))?;

    if !status.success() {
        return Err(format!("Viewer '{}' exited with error", viewer));
    }

    Ok(())
}
