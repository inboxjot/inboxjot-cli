use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;
use serde_json::{Value, json};
use toml_edit::{DocumentMut, Item, Table};

use super::{Client, Plan, Step, find_binary, get_env_dir};
use crate::context::Context;
use crate::error::{Error, Result};

const SHARED_SKILL_DIR: &str = ".agents/skills/inboxjot";

pub const SKILL_FILES: [(&str, &str); 10] = [
  ("SKILL.md", include_str!("../../../skills/inboxjot/SKILL.md")),
  (
    "references/api-keys.md",
    include_str!("../../../skills/inboxjot/references/api-keys.md"),
  ),
  (
    "references/audience.md",
    include_str!("../../../skills/inboxjot/references/audience.md"),
  ),
  (
    "references/campaigns-and-automations.md",
    include_str!("../../../skills/inboxjot/references/campaigns-and-automations.md"),
  ),
  (
    "references/domains.md",
    include_str!("../../../skills/inboxjot/references/domains.md"),
  ),
  (
    "references/errors-and-limits.md",
    include_str!("../../../skills/inboxjot/references/errors-and-limits.md"),
  ),
  ("references/mcp.md", include_str!("../../../skills/inboxjot/references/mcp.md")),
  (
    "references/sending.md",
    include_str!("../../../skills/inboxjot/references/sending.md"),
  ),
  ("references/smtp.md", include_str!("../../../skills/inboxjot/references/smtp.md")),
  (
    "references/webhooks.md",
    include_str!("../../../skills/inboxjot/references/webhooks.md"),
  ),
];

pub fn plan(client: Client, home: &Path, url: &str) -> Result<Plan> {
  let mut plan = match client {
    Client::ClaudeCode => plan_claude_code(home, url),
    Client::Cursor => plan_cursor(home, url),
    Client::Codex => plan_codex(home, url),
    Client::Vscode => plan_vscode(home, url),
  }?;

  let dir = get_skill_dir(client, home);
  let source = home.join(SHARED_SKILL_DIR);
  if !is_skill_installed(&dir, &source) {
    plan.steps.push(Step::InstallSkill { dir, source });
  }
  Ok(plan)
}

pub fn has_skill_files(dir: &Path) -> bool {
  SKILL_FILES.iter().all(|(name, content)| {
    fs::read_to_string(dir.join(name)).is_ok_and(|current| current == *content)
  })
}

pub fn is_same_dir(dir: &Path, other: &Path) -> bool {
  match (fs::canonicalize(dir), fs::canonicalize(other)) {
    (Ok(dir), Ok(other)) => dir == other,
    _ => false,
  }
}

pub fn get_skill_dir(client: Client, home: &Path) -> PathBuf {
  match client {
    Client::ClaudeCode | Client::Cursor => {
      client.get_config_dir(home).join("skills/inboxjot")
    }
    Client::Codex => home.join(SHARED_SKILL_DIR),
    Client::Vscode => home.join(".copilot/skills/inboxjot"),
  }
}

fn is_skill_installed(dir: &Path, source: &Path) -> bool {
  has_skill_files(source)
    && (is_same_dir(dir, source) || (!cfg!(unix) && has_skill_files(dir)))
}

pub fn print_next_step(client: Client, context: &Context) {
  match client {
    Client::ClaudeCode => {
      context
        .output
        .info("Start Claude Code and run /mcp to sign in to Inboxjot.");
    }
    Client::Cursor => {
      context
        .output
        .info("Open Cursor Settings, then MCP, and sign in to inboxjot.");
    }
    Client::Codex => context
      .output
      .next("Sign in to Inboxjot:", "codex mcp login inboxjot"),
    Client::Vscode => context
      .output
      .info("In VS Code, run MCP: List Servers, then start inboxjot to sign in."),
  }
}

fn plan_claude_code(home: &Path, url: &str) -> Result<Plan> {
  let path = get_env_dir("CLAUDE_CONFIG_DIR")
    .map_or_else(|| home.join(".claude.json"), |dir| dir.join(".claude.json"));
  let config = read_json(&path)?;
  let mut plan = Plan::new(Client::ClaudeCode, path);
  let current = config.pointer("/mcpServers/inboxjot/url").and_then(Value::as_str);

  if current != Some(url) {
    let add = ["mcp", "add", "--transport", "http", "--scope", "user", "inboxjot", url];
    match find_binary(Client::ClaudeCode) {
      Some(program) => {
        if current.is_some() {
          let remove = ["mcp", "remove", "--scope", "user", "inboxjot"];
          plan.steps.push(get_run_step(&program, &remove));
        }
        plan.steps.push(get_run_step(&program, &add));
      }
      None => {
        let words = std::iter::once("claude").chain(add);
        plan.manual_command = shlex::try_join(words).ok();
      }
    }
  }

  Ok(plan)
}

fn plan_cursor(home: &Path, url: &str) -> Result<Plan> {
  let path = home.join(".cursor/mcp.json");
  let mut config = read_json(&path)?;
  let mut plan = Plan::new(Client::Cursor, path.clone());
  if config.pointer("/mcpServers/inboxjot/url").and_then(Value::as_str) == Some(url) {
    return Ok(plan);
  }

  let servers = config
    .as_object_mut()
    .and_then(|config| {
      config
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    })
    .ok_or_else(|| get_invalid_config_error(&path))?;
  servers.insert("inboxjot".to_owned(), json!({ "url": url }));

  let content = format!("{}\n", serde_json::to_string_pretty(&config)?);
  plan.steps.push(Step::Write { path, content });
  Ok(plan)
}

fn plan_codex(home: &Path, url: &str) -> Result<Plan> {
  let path = Client::Codex.get_config_dir(home).join("config.toml");
  let text = read_text(&path)?.unwrap_or_default();
  let mut document: DocumentMut =
    text.parse().map_err(|_| get_invalid_config_error(&path))?;
  let mut plan = Plan::new(Client::Codex, path.clone());

  let current = document
    .get("mcp_servers")
    .and_then(|servers| servers.get("inboxjot"))
    .and_then(|server| server.get("url"))
    .and_then(Item::as_str);
  if current == Some(url) {
    return Ok(plan);
  }

  set_codex_url(&mut document, url).ok_or_else(|| get_invalid_config_error(&path))?;
  plan.steps.push(Step::Write { path, content: document.to_string() });
  Ok(plan)
}

fn set_codex_url(document: &mut DocumentMut, url: &str) -> Option<()> {
  let servers = document.entry("mcp_servers").or_insert_with(|| {
    let mut table = Table::new();
    table.set_implicit(true);
    Item::Table(table)
  });
  let server = servers
    .as_table_like_mut()?
    .entry("inboxjot")
    .or_insert(toml_edit::table());
  server.as_table_like_mut()?.insert("url", toml_edit::value(url));
  Some(())
}

fn plan_vscode(home: &Path, url: &str) -> Result<Plan> {
  let path = get_vscode_user_dir(home)?.join("mcp.json");
  let config = read_json(&path)?;
  let mut plan = Plan::new(Client::Vscode, path);
  if config.pointer("/servers/inboxjot/url").and_then(Value::as_str) == Some(url) {
    return Ok(plan);
  }

  let server = json!({ "name": "inboxjot", "type": "http", "url": url }).to_string();
  match find_binary(Client::Vscode) {
    Some(program) => plan.steps.push(get_run_step(&program, &["--add-mcp", &server])),
    None => {
      plan.skip_reason = Some("The code command is not on PATH.".to_owned());
      plan.manual_command = shlex::try_join(["code", "--add-mcp", &server]).ok();
    }
  }
  Ok(plan)
}

fn get_vscode_user_dir(home: &Path) -> Result<PathBuf> {
  let strategy = etcetera::base_strategy::choose_native_strategy().map_err(|error| {
    Error::failed("config_error", format!("Could not find the home directory: {error}"))
  })?;
  let dir =
    if cfg!(target_os = "macos") { strategy.data_dir() } else { strategy.config_dir() };
  let dir = match dir.strip_prefix(strategy.home_dir()) {
    Ok(relative) => home.join(relative),
    Err(_) => dir.clone(),
  };

  Ok(dir.join("Code/User"))
}

fn get_run_step(program: &Path, args: &[&str]) -> Step {
  Step::Run {
    program: program.to_owned(),
    args: args.iter().map(|arg| (*arg).to_owned()).collect(),
  }
}

fn read_text(path: &Path) -> Result<Option<String>> {
  match fs::read_to_string(path) {
    Ok(text) => Ok(Some(text)),
    Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
    Err(error) => Err(error.into()),
  }
}

fn read_json(path: &Path) -> Result<Value> {
  match read_text(path)? {
    Some(text) if !text.trim().is_empty() => {
      serde_json::from_str(&text).map_err(|_| get_invalid_config_error(path))
    }
    _ => Ok(json!({})),
  }
}

fn get_invalid_config_error(path: &Path) -> Error {
  Error::failed(
    "invalid_config",
    format!("{} is not valid. Fix it, then run this again.", path.display()),
  )
}
