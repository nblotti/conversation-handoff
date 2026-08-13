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
    /// Checkpoint notes on a conversation since the last save.
    #[command(visible_alias = "remember")]
    Save {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        text_file: Option<PathBuf>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        summary: Option<String>,
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
        #[arg(long)]
        summary: Option<String>,
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
    /// List conversations as one-sentence summaries.
    List {
        /// Only older than this, e.g. 30d.
        #[arg(long)]
        older_than: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        contains: Option<String>,
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        include_pruned: bool,
    },
    /// Drop stored content, keep a one-sentence summary (unless --purge).
    Forget {
        #[arg(long)]
        conversation_id: Option<String>,
        #[arg(long)]
        older_than: Option<String>,
        #[arg(long)]
        purge: bool,
    },
    /// Attach a png/jpeg/gif/webp to a conversation.
    AttachImage {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        caption: Option<String>,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        data_base64: Option<String>,
        #[arg(long)]
        mime: Option<String>,
    },
    /// Write an attached image to a file.
    Image {
        /// Reference like conversation-id#1.
        #[arg(long)]
        reference: String,
        /// Where to write the bytes.
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a sample YAML config (sqlite / postgres).
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Mcp) {
        Command::Mcp => {
            // Open the store before Tokio starts. The sync postgres client
            // builds its own runtime and panics if that happens on a worker.
            let store = Store::open()?;
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("tokio runtime")?;
            rt.block_on(run_mcp(store))
        }
        Command::Save {
            conversation_id,
            text,
            text_file,
            title,
            summary,
        } => {
            let text = read_input(text, text_file)?;
            print_json(&engine()?.save(&conversation_id, &text, title, summary)?)
        }
        Command::Handoff {
            thread_id,
            new_id,
            latest_message,
            context,
            context_file,
            title,
            summary,
        } => {
            let context = read_input(context, context_file)?;
            print_json(&engine()?.handoff(
                &thread_id,
                &new_id,
                &latest_message,
                &context,
                title,
                summary,
            )?)
        }
        Command::Load { conversation_id } => print_json(&engine()?.load(&conversation_id)?),
        Command::Recall {
            conversation_id,
            query,
            max_results,
        } => print_json(&engine()?.recall(&conversation_id, query.as_deref(), max_results)?),
        Command::Thread { conversation_id } => print_json(&engine()?.thread(&conversation_id)?),
        Command::List {
            older_than,
            limit,
            contains,
            thread,
            include_pruned,
        } => print_json(&engine()?.list(
            older_than.as_deref(),
            limit,
            contains.as_deref(),
            thread.as_deref(),
            include_pruned,
        )?),
        Command::Forget {
            conversation_id,
            older_than,
            purge,
        } => print_json(&engine()?.forget(
            conversation_id.as_deref(),
            older_than.as_deref(),
            purge,
        )?),
        Command::AttachImage {
            conversation_id,
            caption,
            path,
            data_base64,
            mime,
        } => print_json(&engine()?.attach_image(
            &conversation_id,
            caption.as_deref(),
            path.as_deref().and_then(|p| p.to_str()),
            data_base64.as_deref(),
            mime.as_deref(),
        )?),
        Command::Image { reference, out } => {
            let img = engine()?.image(&reference)?;
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out, img.bytes).with_context(|| format!("write {}", out.display()))?;
            println!("wrote {} ({} bytes)", out.display(), img.meta.byte_len);
            Ok(())
        }
        Command::InitConfig { path } => {
            let written = conversation_handoff::config::write_sample(path)?;
            println!("Wrote {}", written.display());
            println!("Edit store.type (sqlite or postgres), url, user, and password.");
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

async fn run_mcp(store: Store) -> Result<()> {
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
        return std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()));
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
