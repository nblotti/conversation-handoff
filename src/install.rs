use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use toml_edit::{value, DocumentMut, Item, Table};

const SERVER_NAME: &str = "conversation-handoff";
const SKILL_MD: &str = include_str!("../skills/handoff/SKILL.md");

pub struct InstallReport {
    pub binary: PathBuf,
    pub lines: Vec<String>,
}

pub fn install(binary: Option<PathBuf>, write_instructions: bool) -> Result<InstallReport> {
    let binary = match binary {
        Some(p) => p,
        None => std::env::current_exe().context("resolve current executable")?,
    };
    let binary = binary.canonicalize().unwrap_or(binary);
    let mut lines = Vec::new();
    lines.push(format!("Binary: {}", binary.display()));

    match install_claude(&binary) {
        Ok(msg) => lines.push(format!("Claude Code: {msg}")),
        Err(e) => lines.push(format!("Claude Code: skipped ({e})")),
    }
    match install_codex(&binary) {
        Ok(msg) => lines.push(format!("Codex: {msg}")),
        Err(e) => lines.push(format!("Codex: skipped ({e})")),
    }
    match write_skills() {
        Ok(msg) => lines.push(format!("Skills: {msg}")),
        Err(e) => lines.push(format!("Skills: skipped ({e})")),
    }

    if write_instructions {
        match write_claude_instructions() {
            Ok(msg) => lines.push(format!("Claude instructions: {msg}")),
            Err(e) => lines.push(format!("Claude instructions: skipped ({e})")),
        }
        match write_codex_instructions() {
            Ok(msg) => lines.push(format!("Codex instructions: {msg}")),
            Err(e) => lines.push(format!("Codex instructions: skipped ({e})")),
        }
    }

    lines.push(String::new());
    lines.push("Manual Claude Code (~/.claude.json or project .mcp.json):".into());
    lines.push(claude_snippet(&binary));
    lines.push(String::new());
    lines.push("Manual Codex (~/.codex/config.toml):".into());
    lines.push(codex_snippet(&binary));
    lines.push(String::new());
    lines.push("Agent instructions (paste into CLAUDE.md / AGENTS.md):".into());
    lines.push(instruction_block().into());

    Ok(InstallReport { binary, lines })
}

fn install_claude(binary: &Path) -> Result<String> {
    if command_exists("claude") {
        let status = Command::new("claude")
            .args([
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "stdio",
                SERVER_NAME,
                "--",
            ])
            .arg(binary)
            .status()
            .context("run claude mcp add")?;
        if status.success() {
            return Ok("registered with `claude mcp add --scope user`".into());
        }
        // Fall through and write the file if the CLI rejected a duplicate, etc.
    }
    write_claude_json(binary)
}

fn write_claude_json(binary: &Path) -> Result<String> {
    let path = claude_json_path()?;
    let mut root = if path.exists() {
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !root.is_object() {
        root = json!({});
    }
    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    servers[SERVER_NAME] = json!({
        "type": "stdio",
        "command": binary.to_string_lossy(),
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&root)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(format!("wrote {}", path.display()))
}

fn install_codex(binary: &Path) -> Result<String> {
    if command_exists("codex") {
        let status = Command::new("codex")
            .args(["mcp", "add", SERVER_NAME, "--"])
            .arg(binary)
            .status()
            .context("run codex mcp add")?;
        if status.success() {
            return Ok("registered with `codex mcp add`".into());
        }
    }
    write_codex_toml(binary)
}

fn write_codex_toml(binary: &Path) -> Result<String> {
    let path = codex_config_path()?;
    let mut doc = if path.exists() {
        let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        text.parse::<DocumentMut>()
            .with_context(|| format!("parse {}", path.display()))?
    } else {
        DocumentMut::new()
    };

    if !doc.contains_key("mcp_servers") {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = doc["mcp_servers"]
        .as_table_mut()
        .context("mcp_servers is not a table")?;
    let mut table = Table::new();
    table["command"] = value(binary.to_string_lossy().to_string());
    servers.insert(SERVER_NAME, Item::Table(table));

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, doc.to_string()).with_context(|| format!("write {}", path.display()))?;
    Ok(format!("wrote {}", path.display()))
}

fn write_claude_instructions() -> Result<String> {
    let dir = home_dir()?.join(".claude");
    fs::create_dir_all(&dir)?;
    let path = dir.join("CLAUDE.md");
    append_unique_section(&path, instruction_block())?;
    Ok(path.display().to_string())
}

fn write_codex_instructions() -> Result<String> {
    let dir = home_dir()?.join(".codex");
    fs::create_dir_all(&dir)?;
    let path = dir.join("AGENTS.md");
    append_unique_section(&path, instruction_block())?;
    Ok(path.display().to_string())
}

fn append_unique_section(path: &Path, section: &str) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    let heading = "## Conversation handoff";
    let mut out = if let Some(start) = existing.find(heading) {
        let before = &existing[..start];
        let after = existing[start + heading.len()..]
            .find("\n## ")
            .map(|rel| &existing[start + heading.len() + rel..])
            .unwrap_or("");
        let mut s = before.trim_end().to_string();
        if !s.is_empty() {
            s.push_str("\n\n");
        }
        s.push_str(section);
        if !after.is_empty() {
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s.push('\n');
            s.push_str(after.trim_start());
        }
        s
    } else {
        let mut s = existing;
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(section);
        s
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    fs::write(path, out)?;
    Ok(())
}

pub fn instruction_block() -> &'static str {
    "\
## Conversation handoff

If the MCP server is missing, install the prebuilt binary. Do not install Rust or run Cargo. Linux: `curl -fsSL https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.sh | bash`. Windows: `irm https://raw.githubusercontent.com/nblotti/conversation-handoff/main/scripts/install.ps1 | iex`. Linux builds are static musl (no GLIBC upgrade). If the binary cannot run, stop; do not compile from source.

When the context window is nearly full:

1. Call `handoff`. Show the user only the one-line `reference` (`conversation-handoff: <id>`). Do not paste a brief.
2. In the new chat, call `load` with that id and work from the brief.
3. If the user asks to look in the past conversation, call `recall`. A specific question finds matching parts.
4. During long sessions, call `save` with work since `last_saved_at`.
5. Chat commands: `/handoff` (save text + images), `/handoff new`, `/handoff list`, `/handoff use <id>`, `/handoff rm <id>`, `/handoff img <path>`, `/handoff help`.
"
}

fn write_skills() -> Result<String> {
    let home = home_dir()?;
    let claude = home.join(".claude/skills/handoff/SKILL.md");
    let agents = home.join(".agents/skills/handoff/SKILL.md");
    write_skill_file(&claude)?;
    write_skill_file(&agents)?;
    let _ = fs::remove_dir_all(home.join(".claude/skills/conversation-handoff"));
    let _ = fs::remove_dir_all(home.join(".agents/skills/conversation-handoff"));
    Ok(format!("{} and {}", claude.display(), agents.display()))
}

fn write_skill_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, SKILL_MD).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn claude_snippet(binary: &Path) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"{SERVER_NAME}\": {{\n      \"type\": \"stdio\",\n      \"command\": \"{}\"\n    }}\n  }}\n}}",
        escape_json(&binary.to_string_lossy())
    )
}

fn codex_snippet(binary: &Path) -> String {
    format!(
        "[mcp_servers.{SERVER_NAME}]\ncommand = \"{}\"",
        escape_toml(&binary.to_string_lossy())
    )
}

fn claude_json_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join(".claude.json"));
    }
    Ok(home_dir()?.join(".claude.json"))
}

fn codex_config_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("CODEX_HOME") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    Ok(home_dir()?.join(".codex").join("config.toml"))
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not resolve home directory")
}

fn command_exists(name: &str) -> bool {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("where");
        c.arg(name);
        c
    } else {
        let mut c = Command::new("which");
        c.arg(name);
        c
    };
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn print_report(report: &InstallReport, out: &mut impl io::Write) -> io::Result<()> {
    for line in &report.lines {
        writeln!(out, "{line}")?;
    }
    Ok(())
}
