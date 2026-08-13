use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use conversation_handoff::engine::Engine;
use conversation_handoff::install::{self, print_report};
use conversation_handoff::mcp::ConversationService;
use conversation_handoff::store::Store;
use rmcp::{transport::stdio, ServiceExt};

#[derive(Parser)]
#[command(
    name = "conversation-handoff",
    about = "Hand off a full Claude or Codex conversation to a new chat, and recall only the parts that still matter.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run as an MCP stdio server (default). This is what Claude Code and Codex launch.
    Mcp,
    /// Checkpoint notes on a conversation before the window fills.
    Remember {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        text_file: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
    },
    /// Link a full thread to a new conversation and print a one-line reference.
    Handoff {
        #[arg(long)]
        thread_id: String,
        #[arg(long)]
        new_id: String,
        #[arg(long)]
        latest_message: String,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        context_file: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
    },
    /// Load the stored brief for a continuation id.
    Load {
        #[arg(long)]
        conversation_id: String,
    },
    /// Search the parent chain, or pull extra past context if query is omitted.
    Recall {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        max_results: Option<u32>,
    },
    /// Print the linked conversation chain.
    Thread {
        #[arg(long)]
        conversation_id: String,
    },
    /// Write a sample YAML config (file / sqlite / postgres).
    InitConfig {
        /// Where to write the file (defaults to the platform config path).
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Register the MCP server with Claude Code and Codex.
    Install {
        /// Explicit path to this binary (defaults to the running executable).
        #[arg(long)]
        command: Option<PathBuf>,
        /// Also append a short how-to to ~/.claude/CLAUDE.md and ~/.codex/AGENTS.md.
        #[arg(long)]
        write_instructions: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Mcp) {
        Command::Mcp => run_mcp().await,
        Command::Remember {
            conversation_id,
            text,
            text_file,
            title,
        } => {
            let text = read_input(text, text_file)?;
            print_json(&engine()?.remember(&conversation_id, &text, title)?)
        }
        Command::Handoff {
            thread_id,
            new_id,
            latest_message,
            context,
            context_file,
            title,
        } => {
            let context = read_input(context, context_file)?;
            print_json(&engine()?.handoff(
                &thread_id,
                &new_id,
                &latest_message,
                &context,
                title,
            )?)
        }
        Command::Load { conversation_id } => print_json(&engine()?.load(&conversation_id)?),
        Command::Recall {
            conversation_id,
            query,
            max_results,
        } => print_json(&engine()?.recall(
            &conversation_id,
            query.as_deref(),
            max_results,
        )?),
        Command::Thread { conversation_id } => {
            print_json(&engine()?.thread(&conversation_id)?)
        }
        Command::InitConfig { path } => {
            let written = conversation_handoff::config::write_sample(path)?;
            println!("Wrote {}", written.display());
            println!("Edit store.type (file, sqlite, or postgres), url, user, and password.");
            Ok(())
        }
        Command::Install {
            command,
            write_instructions,
        } => {
            let report = install::install(command, write_instructions)?;
            print_report(&report, &mut io::stdout())?;
            Ok(())
        }
    }
}

async fn run_mcp() -> Result<()> {
    let store = Store::open()?;
    let service = ConversationService::new(store)
        .serve(stdio())
        .await
        .context("start MCP stdio server")?;
    service.waiting().await.context("MCP server exited")?;
    Ok(())
}

fn engine() -> Result<Engine> {
    Ok(Engine::new(Store::open()?))
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn read_input(inline: Option<String>, file: Option<PathBuf>) -> Result<String> {
    if let Some(path) = file {
        return std::fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()));
    }
    if let Some(text) = inline {
        return Ok(text);
    }
    if io::stdin().is_terminal() {
        anyhow::bail!("pass --text/--context, --text-file/--context-file, or pipe on stdin");
    }
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}
