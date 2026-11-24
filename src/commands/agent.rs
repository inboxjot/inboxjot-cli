mod clients;

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::ui::prompt::{self, Choice, MultiSelect};

const DEFAULT_MCP_URL: &str = "https://mcp.inboxjot.com";

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
  #[command(
    about = "Add the Inboxjot MCP server and skill to Claude Code, Cursor, Codex and VS Code",
    after_help = examples!(
      "inboxjot agent setup",
      "inboxjot agent setup --client claude-code",
    ),
  )]
  Setup(SetupArgs),
}

#[derive(Debug, Args)]
pub struct SetupArgs {
  #[arg(
    long = "client",
    value_name = "CLIENT",
    value_enum,
    value_delimiter = ',',
    help = "Set up only this client; repeat it or separate with commas"
  )]
  pub clients: Vec<Client>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Client {
  ClaudeCode,
  Cursor,
  Codex,
  Vscode,
}

impl Client {
  fn get_name(self) -> &'static str {
    match self {
      Self::ClaudeCode => "Claude Code",
      Self::Cursor => "Cursor",
      Self::Codex => "Codex",
      Self::Vscode => "VS Code",
    }
  }

  fn get_site(self) -> &'static str {
    match self {
      Self::ClaudeCode => "claude.com/claude-code",
      Self::Cursor => "cursor.com",
      Self::Codex => "openai.com/codex",
      Self::Vscode => "code.visualstudio.com",
    }
  }

  fn get_binary(self) -> &'static str {
    match self {
      Self::ClaudeCode => "claude",
      Self::Cursor => "cursor",
      Self::Codex => "codex",
      Self::Vscode => "code",
    }
  }

  fn get_config_dir(self, home: &Path) -> PathBuf {
    match self {
      Self::ClaudeCode => {
        get_env_dir("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude"))
      }
      Self::Cursor => home.join(".cursor"),
      Self::Codex => get_env_dir("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
      Self::Vscode => home.join(".vscode"),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
  Configured,
  Unchanged,
  Skipped,
}

enum Step {
  Run { program: PathBuf, args: Vec<String> },
  Write { path: PathBuf, content: String },
  InstallSkill { dir: PathBuf, source: PathBuf },
}

struct Plan {
  client: Client,
  path: PathBuf,
  steps: Vec<Step>,
  skip_reason: Option<String>,
  manual_command: Option<String>,
}

impl Plan {
  fn new(client: Client, path: PathBuf) -> Self {
    Self {
      client,
      path,
      steps: Vec::new(),
      skip_reason: None,
      manual_command: None,
    }
  }

  fn get_outcome(&self) -> Outcome {
    if self.skip_reason.is_some() {
      Outcome::Skipped
    } else if self.steps.is_empty() {
      Outcome::Unchanged
    } else {
      Outcome::Configured
    }
  }
}

#[derive(Serialize)]
struct Report<'a> {
  client: Client,
  outcome: Outcome,
  path: &'a Path,
}

pub fn run(command: AgentCommand, context: &Context) -> Result<()> {
  match command {
    AgentCommand::Setup(args) => setup(args, context),
  }
}

fn setup(args: SetupArgs, context: &Context) -> Result<()> {
  let home = get_home()?;
  let url = get_mcp_url();
  let mut clients = args.clients;
  if clients.is_empty() {
    clients = detect_clients(&home);
    if context.interactive && !context.yes && !clients.is_empty() {
      clients = pick_clients(&clients, &home, &url)?;
    }
  }
  clients.sort();
  clients.dedup();

  if clients.is_empty() {
    return Err(
      Error::failed("no_clients", "No supported AI coding agent was found.").with_next(
        "inboxjot agent setup --client claude-code",
        "Name the client to set up",
      ),
    );
  }

  let plans = clients
    .iter()
    .map(|client| clients::plan(*client, &home, &url))
    .collect::<Result<Vec<_>>>()?;
  let pending: Vec<&Plan> = plans.iter().filter(|plan| !plan.steps.is_empty()).collect();

  if !pending.is_empty() {
    if context.interactive && !context.yes {
      anstream::eprintln!();
    }
    let names: Vec<&str> = pending.iter().map(|plan| plan.client.get_name()).collect();
    context.proceed(&format!("Configure {}?", join_names(&names)))?;
    if context.interactive && !context.yes {
      anstream::eprintln!();
    }
    for plan in &pending {
      apply(plan)?;
    }
  }

  print_report(&plans, &home, context)
}

pub struct ToolStatus {
  pub name: &'static str,
  pub flag: String,
  pub is_set_up: bool,
}

pub fn get_tool_statuses() -> Vec<ToolStatus> {
  let Ok(home) = get_home() else {
    return Vec::new();
  };
  let url = get_mcp_url();
  detect_clients(&home)
    .into_iter()
    .map(|client| ToolStatus {
      name: client.get_name(),
      flag: client
        .to_possible_value()
        .map(|value| value.get_name().to_owned())
        .unwrap_or_default(),
      is_set_up: clients::plan(client, &home, &url)
        .is_ok_and(|plan| plan.steps.is_empty()),
    })
    .collect()
}

fn get_home() -> Result<PathBuf> {
  if let Some(home) = std::env::var_os("INBOXJOT_HOME").filter(|home| !home.is_empty()) {
    return Ok(PathBuf::from(home));
  }

  etcetera::home_dir().map_err(|error| {
    Error::failed("config_error", format!("Could not find the home directory: {error}"))
  })
}

fn get_env_dir(name: &str) -> Option<PathBuf> {
  std::env::var_os(name)
    .filter(|dir| !dir.is_empty())
    .map(PathBuf::from)
}

fn get_mcp_url() -> String {
  std::env::var("INBOXJOT_MCP_URL")
    .ok()
    .filter(|url| !url.is_empty())
    .unwrap_or_else(|| DEFAULT_MCP_URL.to_owned())
}

fn detect_clients(home: &Path) -> Vec<Client> {
  Client::value_variants()
    .iter()
    .copied()
    .filter(|client| {
      find_binary(*client).is_some() || client.get_config_dir(home).is_dir()
    })
    .collect()
}

fn pick_clients(detected: &[Client], home: &Path, url: &str) -> Result<Vec<Client>> {
  let all = Client::value_variants();
  let order: Vec<Client> = all
    .iter()
    .copied()
    .filter(|client| detected.contains(client))
    .chain(all.iter().copied().filter(|client| !detected.contains(client)))
    .collect();

  let choices = order
    .iter()
    .map(|client| {
      let found = detected.contains(client);
      let is_set_up = found
        && clients::plan(*client, home, url).is_ok_and(|plan| plan.steps.is_empty());
      Choice {
        label: client.get_name().to_owned(),
        detail: if found { String::new() } else { client.get_site().to_owned() },
        note: if is_set_up { "set up already".to_owned() } else { String::new() },
        group: if found { "Found on this machine" } else { "Not installed" }.to_owned(),
        enabled: found,
        selected: found,
      }
    })
    .collect();

  let picked =
    prompt::run(MultiSelect::new("Which tools should use Inboxjot?", choices))?;
  Ok(picked.into_iter().map(|index| order[index]).collect())
}

fn join_names(names: &[&str]) -> String {
  match names {
    [] => String::new(),
    [name] => (*name).to_owned(),
    [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
  }
}

fn find_binary(client: Client) -> Option<PathBuf> {
  which::which(client.get_binary()).ok()
}

fn shorten_path(path: &Path, home: &Path) -> PathBuf {
  match path.strip_prefix(home) {
    Ok(relative) => Path::new("~").join(relative),
    Err(_) => path.to_owned(),
  }
}

fn apply(plan: &Plan) -> Result<()> {
  for step in &plan.steps {
    match step {
      Step::Run { program, args } => run_program(program, args)?,
      Step::Write { path, content } => write_file(path, content)?,
      Step::InstallSkill { dir, source } => install_skill(dir, source)?,
    }
  }
  Ok(())
}

fn install_skill(dir: &Path, source: &Path) -> Result<()> {
  write_skill_files(source)?;
  if clients::is_same_dir(dir, source) {
    return Ok(());
  }

  remove_path(dir)?;
  if let Some(parent) = dir.parent() {
    fs::create_dir_all(parent)?;
  }
  if link_dir(source, dir).is_err() {
    write_skill_files(dir)?;
  }
  Ok(())
}

fn write_skill_files(dir: &Path) -> Result<()> {
  if clients::has_skill_files(dir) {
    return Ok(());
  }

  for (name, content) in clients::SKILL_FILES {
    write_file(&dir.join(name), content)?;
  }
  Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
  match fs::symlink_metadata(path) {
    Ok(metadata) if metadata.is_dir() => fs::remove_dir_all(path)?,
    Ok(_) => fs::remove_file(path)?,
    Err(error) if error.kind() == ErrorKind::NotFound => {}
    Err(error) => return Err(error.into()),
  }
  Ok(())
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> io::Result<()> {
  std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn link_dir(_target: &Path, _link: &Path) -> io::Result<()> {
  Err(ErrorKind::Unsupported.into())
}

fn run_program(program: &Path, args: &[String]) -> Result<()> {
  let output = Command::new(program).args(args).output()?;
  if output.status.success() {
    return Ok(());
  }

  let name = program.file_stem().unwrap_or_default().to_string_lossy();
  let stderr = String::from_utf8_lossy(&output.stderr);
  Err(Error::failed(
    "command_failed",
    format!("{name} {} failed: {}", args.join(" "), stderr.trim()),
  ))
}

fn write_file(path: &Path, content: &str) -> Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  fs::write(path, content)?;
  Ok(())
}

fn print_report(plans: &[Plan], home: &Path, context: &Context) -> Result<()> {
  if context.output.is_json() {
    let clients: Vec<Report> = plans
      .iter()
      .map(|plan| Report {
        client: plan.client,
        outcome: plan.get_outcome(),
        path: &plan.path,
      })
      .collect();
    return context.output.print_json(&json!({ "clients": clients }));
  }

  for plan in plans {
    let name = plan.client.get_name();
    match plan.get_outcome() {
      Outcome::Configured => context.output.success(&format!("Configured {name}")),
      Outcome::Unchanged => context.output.success(&format!("{name} is already set up")),
      Outcome::Skipped => {
        let reason = plan.skip_reason.as_deref().unwrap_or_default();
        context.output.warn(&format!("Skipped {name}. {reason}"));
      }
    }
    context
      .output
      .info(&shorten_path(&plan.path, home).display().to_string());
    if let Some(command) = &plan.manual_command {
      context.output.next("Run this to add the MCP server:", command);
    } else if plan.get_outcome() != Outcome::Skipped {
      clients::print_next_step(plan.client, context);
    }
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::join_names;

  #[test]
  fn joins_names_like_a_sentence() {
    assert_eq!(join_names(&["Claude Code"]), "Claude Code");
    assert_eq!(join_names(&["Claude Code", "Cursor"]), "Claude Code and Cursor");
    assert_eq!(
      join_names(&["Claude Code", "Cursor", "Codex"]),
      "Claude Code, Cursor and Codex"
    );
  }
}
