#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use assert_cmd::Command;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Machine {
  home: TempDir,
  bin: TempDir,
}

impl Machine {
  fn new() -> Self {
    Self {
      home: TempDir::new().unwrap(),
      bin: TempDir::new().unwrap(),
    }
  }

  fn add_binary(&self, name: &str) {
    let path = self.bin.path().join(name);
    let log = self.bin.path().join(format!("{name}.log"));
    let script = format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display());
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
  }

  fn read_log(&self, name: &str) -> String {
    fs::read_to_string(self.bin.path().join(format!("{name}.log"))).unwrap_or_default()
  }

  fn get_path(&self, path: &str) -> PathBuf {
    self.home.path().join(path)
  }

  fn write(&self, path: &str, content: &str) {
    let path = self.get_path(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
  }

  fn read(&self, path: &str) -> String {
    fs::read_to_string(self.get_path(path)).unwrap()
  }

  fn inboxjot(&self) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("inboxjot"));
    for name in [
      "CLAUDECODE",
      "CODEX_SANDBOX",
      "GEMINI_CLI",
      "AGENT",
      "CI",
      "INBOXJOT_MCP_URL",
      "CLAUDE_CONFIG_DIR",
      "CODEX_HOME",
    ] {
      command.env_remove(name);
    }

    command
      .env("INBOXJOT_HOME", self.home.path())
      .env("PATH", self.bin.path())
      .env("NO_COLOR", "1");
    command
  }

  fn set_up(&self, args: &[&str]) -> Value {
    self.set_up_with(args, &[])
  }

  fn set_up_with(&self, args: &[&str], envs: &[(&str, PathBuf)]) -> Value {
    let output = self
      .inboxjot()
      .args(["agent", "setup", "--json", "--yes"])
      .args(args)
      .envs(envs.iter().map(|(name, value)| (*name, value)))
      .output()
      .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
  }
}

fn get_outcomes(report: &Value) -> Vec<(String, String)> {
  report["clients"]
    .as_array()
    .unwrap()
    .iter()
    .map(|client| {
      (
        client["client"].as_str().unwrap().to_owned(),
        client["outcome"].as_str().unwrap().to_owned(),
      )
    })
    .collect()
}

#[test]
fn merges_the_cursor_config_and_keeps_other_servers() {
  let machine = Machine::new();
  machine.write(
    ".cursor/mcp.json",
    r#"{ "theme": "dark", "mcpServers": { "github": { "command": "gh-mcp" } } }"#,
  );

  let report = machine.set_up(&["--client", "cursor"]);

  assert_eq!(
    report,
    json!({ "clients": [{
      "client": "cursor",
      "outcome": "configured",
      "path": machine.get_path(".cursor/mcp.json"),
    }] })
  );
  let config: Value = serde_json::from_str(&machine.read(".cursor/mcp.json")).unwrap();
  assert_eq!(
    config,
    json!({
      "theme": "dark",
      "mcpServers": {
        "github": { "command": "gh-mcp" },
        "inboxjot": { "url": "https://mcp.inboxjot.com" },
      },
    })
  );
}

#[test]
fn merges_the_codex_config_and_keeps_comments() {
  let machine = Machine::new();
  let original = "# My Codex settings\nmodel = \"o3\" # fast\n\n[mcp_servers.github]\ncommand = \"gh-mcp\"\n";
  machine.write(".codex/config.toml", original);

  machine.set_up(&["--client", "codex"]);

  assert_eq!(
    machine.read(".codex/config.toml"),
    format!("{original}\n[mcp_servers.inboxjot]\nurl = \"https://mcp.inboxjot.com\"\n")
  );
}

#[test]
fn reports_unchanged_on_a_second_run() {
  let machine = Machine::new();
  machine.write(".codex/config.toml", "model = \"o3\"\n");

  let first = machine.set_up(&["--client", "cursor,codex"]);
  let cursor = machine.read(".cursor/mcp.json");
  let codex = machine.read(".codex/config.toml");
  let second = machine.set_up(&["--client", "codex", "--client", "cursor"]);

  let configured = |client: &str| (client.to_owned(), "configured".to_owned());
  let unchanged = |client: &str| (client.to_owned(), "unchanged".to_owned());
  assert_eq!(get_outcomes(&first), [configured("cursor"), configured("codex")]);
  assert_eq!(get_outcomes(&second), [unchanged("cursor"), unchanged("codex")]);
  assert_eq!(machine.read(".cursor/mcp.json"), cursor);
  assert_eq!(machine.read(".codex/config.toml"), codex);
}

#[test]
fn adds_the_server_to_claude_code_and_installs_the_skill() {
  let machine = Machine::new();
  machine.add_binary("claude");

  let report = machine.set_up(&["--client", "claude-code"]);

  assert_eq!(
    get_outcomes(&report),
    [("claude-code".to_owned(), "configured".to_owned())]
  );
  assert_eq!(
    machine.read_log("claude"),
    "mcp add --transport http --scope user inboxjot https://mcp.inboxjot.com\n"
  );
  assert_eq!(
    machine.read(".agents/skills/inboxjot/SKILL.md"),
    include_str!("../skills/inboxjot/SKILL.md")
  );
  assert!(
    machine
      .get_path(".agents/skills/inboxjot/references/smtp.md")
      .is_file()
  );
  assert_eq!(
    fs::read_link(machine.get_path(".claude/skills/inboxjot")).unwrap(),
    machine.get_path(".agents/skills/inboxjot")
  );

  machine.write(
    ".claude.json",
    r#"{ "mcpServers": { "inboxjot": { "type": "http", "url": "https://mcp.inboxjot.com" } } }"#,
  );
  let second = machine.set_up(&["--client", "claude-code"]);
  assert_eq!(get_outcomes(&second), [("claude-code".to_owned(), "unchanged".to_owned())]);
  assert_eq!(machine.read_log("claude").lines().count(), 1);
}

#[test]
fn detects_clients_from_path_and_config_dirs() {
  let machine = Machine::new();
  machine.add_binary("code");
  fs::create_dir_all(machine.get_path(".codex")).unwrap();

  let output = machine
    .inboxjot()
    .args(["agent", "setup", "--json", "--yes"])
    .env("INBOXJOT_MCP_URL", "http://localhost:4000/mcp")
    .output()
    .unwrap();

  assert!(output.status.success());
  let report: Value = serde_json::from_slice(&output.stdout).unwrap();
  let outcomes: Vec<String> =
    get_outcomes(&report).into_iter().map(|(client, _)| client).collect();
  assert_eq!(outcomes, ["codex", "vscode"]);
  assert_eq!(
    machine.read_log("code"),
    "--add-mcp {\"name\":\"inboxjot\",\"type\":\"http\",\"url\":\"http://localhost:4000/mcp\"}\n"
  );
  assert!(
    machine
      .read(".codex/config.toml")
      .contains("url = \"http://localhost:4000/mcp\"")
  );
}

#[test]
fn asks_for_yes_when_it_cannot_prompt() {
  let machine = Machine::new();

  let output = machine
    .inboxjot()
    .args(["agent", "setup", "--client", "cursor", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let error: Value = serde_json::from_slice(&output.stderr).unwrap();
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(error["error"]["message"], "Configure Cursor? Add --yes to confirm.");
  assert!(!machine.get_path(".cursor/mcp.json").exists());
}

#[test]
fn fails_when_no_client_is_found() {
  let machine = Machine::new();

  let output = machine
    .inboxjot()
    .args(["agent", "setup", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let error: Value = serde_json::from_slice(&output.stderr).unwrap();
  assert_eq!(error["error"]["name"], "no_clients");
}

#[test]
fn links_each_client_to_one_shared_skill() {
  let machine = Machine::new();

  machine.set_up(&["--client", "cursor,codex"]);

  let shared = machine.get_path(".agents/skills/inboxjot");
  assert_eq!(
    machine.read(".agents/skills/inboxjot/SKILL.md"),
    include_str!("../skills/inboxjot/SKILL.md")
  );
  assert_eq!(fs::read_link(machine.get_path(".cursor/skills/inboxjot")).unwrap(), shared);
  assert!(!shared.is_symlink());
}

#[test]
fn replaces_an_old_skill_copy_with_a_link() {
  let machine = Machine::new();
  machine.write(".cursor/skills/inboxjot/SKILL.md", "old");
  machine.write(".cursor/skills/inboxjot/references/gone.md", "old");

  let report = machine.set_up(&["--client", "cursor"]);

  assert_eq!(get_outcomes(&report), [("cursor".to_owned(), "configured".to_owned())]);
  assert_eq!(
    machine.read(".cursor/skills/inboxjot/SKILL.md"),
    include_str!("../skills/inboxjot/SKILL.md")
  );
  assert!(
    !machine
      .get_path(".cursor/skills/inboxjot/references/gone.md")
      .exists()
  );
}

#[test]
fn keeps_the_shared_skill_when_a_skills_folder_links_to_it() {
  let machine = Machine::new();
  fs::create_dir_all(machine.get_path(".agents/skills")).unwrap();
  fs::create_dir_all(machine.get_path(".cursor")).unwrap();
  std::os::unix::fs::symlink(
    machine.get_path(".agents/skills"),
    machine.get_path(".cursor/skills"),
  )
  .unwrap();

  machine.set_up(&["--client", "cursor"]);
  let second = machine.set_up(&["--client", "cursor"]);

  assert_eq!(get_outcomes(&second), [("cursor".to_owned(), "unchanged".to_owned())]);
  assert_eq!(
    machine.read(".agents/skills/inboxjot/SKILL.md"),
    include_str!("../skills/inboxjot/SKILL.md")
  );
  assert!(!machine.get_path(".agents/skills/inboxjot").is_symlink());
}

#[test]
fn uses_the_codex_home_folder() {
  let machine = Machine::new();
  let codex_home = machine.get_path("codex-home");
  fs::create_dir_all(&codex_home).unwrap();

  let report = machine.set_up_with(&[], &[("CODEX_HOME", codex_home.clone())]);

  assert_eq!(get_outcomes(&report), [("codex".to_owned(), "configured".to_owned())]);
  assert!(
    machine
      .read("codex-home/config.toml")
      .contains("url = \"https://mcp.inboxjot.com\"")
  );
  assert!(!machine.get_path(".codex").exists());
}

#[test]
fn uses_the_claude_config_folder() {
  let machine = Machine::new();
  machine.add_binary("claude");
  let config_dir = machine.get_path("claude-config");
  let envs = [("CLAUDE_CONFIG_DIR", config_dir.clone())];

  machine.set_up_with(&["--client", "claude-code"], &envs);
  machine.write(
    "claude-config/.claude.json",
    r#"{ "mcpServers": { "inboxjot": { "url": "https://mcp.inboxjot.com" } } }"#,
  );
  let second = machine.set_up_with(&["--client", "claude-code"], &envs);

  assert_eq!(
    fs::read_link(config_dir.join("skills/inboxjot")).unwrap(),
    machine.get_path(".agents/skills/inboxjot")
  );
  assert!(!machine.get_path(".claude").exists());
  assert_eq!(get_outcomes(&second), [("claude-code".to_owned(), "unchanged".to_owned())]);
}
