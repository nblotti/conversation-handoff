use serde::Serialize;

use crate::config::{self, Config};

#[derive(Debug, Clone, Serialize)]
pub struct HelpCommand {
    pub typed: String,
    pub does: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HelpResult {
    /// Human-readable help to show the user.
    pub text: String,
    pub commands: Vec<HelpCommand>,
    pub config_file: String,
    pub owner: String,
    pub encryption: bool,
}

pub fn commands() -> Vec<HelpCommand> {
    vec![
        HelpCommand {
            typed: "/handoff".into(),
            does: "save notes since the last checkpoint".into(),
        },
        HelpCommand {
            typed: "/handoff save".into(),
            does: "same as /handoff".into(),
        },
        HelpCommand {
            typed: "/handoff new".into(),
            does: "hand off to a new chat; you only get conversation-handoff: <id>".into(),
        },
        HelpCommand {
            typed: "/handoff list".into(),
            does: "show your conversations (one-sentence summaries)".into(),
        },
        HelpCommand {
            typed: "/handoff list 30d".into(),
            does: "only conversations older than 30 days".into(),
        },
        HelpCommand {
            typed: "/handoff use <id>".into(),
            does: "open one conversation".into(),
        },
        HelpCommand {
            typed: "/handoff rm <id>".into(),
            does: "drop stored content, keep the summary".into(),
        },
        HelpCommand {
            typed: "/handoff clean 30d".into(),
            does: "prune every conversation older than 30 days".into(),
        },
        HelpCommand {
            typed: "/handoff img <path>".into(),
            does: "attach a screenshot".into(),
        },
        HelpCommand {
            typed: "/handoff help".into(),
            does: "show this command list and where owner / encryption_key go".into(),
        },
    ]
}

pub fn from_config_file() -> HelpResult {
    let path = config::config_path();
    let (owner, encryption) = match Config::load() {
        Ok(cfg) => (
            cfg.store.owner.trim().to_string(),
            !cfg.store.encryption_key.trim().is_empty(),
        ),
        Err(_) => (String::new(), false),
    };
    build(path.display().to_string(), owner, encryption)
}

pub fn build(config_file: String, owner: String, encryption: bool) -> HelpResult {
    let commands = commands();
    let owner_line = if owner.is_empty() {
        "(not set — postgres will use your login name)".to_string()
    } else {
        owner.clone()
    };
    let enc_line = if encryption {
        "on"
    } else {
        "off — set store.encryption_key"
    };
    let mut text = String::from("conversation-handoff commands\n\n");
    for c in &commands {
        text.push_str(&format!("  {:<22} {}\n", c.typed, c.does));
    }
    text.push_str(&format!(
        "\nConfig file:\n  {config_file}\n\n\
Current:\n  owner: {owner_line}\n  encryption: {enc_line}\n\n\
Put owner and encryption_key under store: in that YAML file:\n\n\
  store:\n\
    type: postgres\n\
    url: \"host:5432/dbname\"\n\
    user: sashiko\n\
    password: \"...\"\n\
    owner: your-name\n\
    encryption_key: \"a long secret only you know\"\n\n\
owner: list/load only your rows when several people share the database.\n\
encryption_key: title, summary, topic, brief, notes, and images are ciphertext without it.\n\
Or set CONVERSATION_HANDOFF_OWNER and CONVERSATION_HANDOFF_ENCRYPTION_KEY.\n"
    ));
    HelpResult {
        text,
        commands,
        config_file,
        owner,
        encryption,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_lists_commands_and_config_keys() {
        let help = build("/tmp/config.yaml".into(), "nblotti".into(), true);
        assert!(help.text.contains("/handoff list"));
        assert!(help.text.contains("/handoff use <id>"));
        assert!(help.text.contains("/handoff help"));
        assert!(help.text.contains("owner: your-name"));
        assert!(help.text.contains("encryption_key"));
        assert_eq!(help.owner, "nblotti");
        assert!(help.encryption);
    }
}
