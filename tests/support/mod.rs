#![allow(dead_code)]

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;
use wiremock::MockServer;

pub const API_KEY: &str = "ibx_test1234_secretsecretsecretsecret";
pub const OAUTH_TOKEN: &str = "oauth_at_testaccesstoken";
pub const WORKSPACE: &str = "acme";

pub struct Harness {
  pub server: MockServer,
  pub config_dir: TempDir,
}

impl Harness {
  pub async fn start() -> Self {
    Self {
      server: MockServer::start().await,
      config_dir: TempDir::new().unwrap(),
    }
  }

  pub fn inboxjot(&self) -> Command {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("inboxjot"));
    for name in [
      "CLAUDECODE",
      "CODEX_SANDBOX",
      "GEMINI_CLI",
      "AGENT",
      "CI",
      "INBOXJOT_PROFILE",
      "INBOXJOT_DASHBOARD_URL",
    ] {
      command.env_remove(name);
    }

    command
      .env("INBOXJOT_API_URL", self.server.uri())
      .env("INBOXJOT_API_KEY", API_KEY)
      .env("INBOXJOT_CONFIG_DIR", self.config_dir.path())
      .env("__INBOXJOT_FILE_CREDENTIALS", "1")
      .env("NO_COLOR", "1");
    command
  }

  pub fn save_browser_login(&self, expires_at: i64) {
    let uri = self.server.uri();
    let config = format!(
      "[profiles.default]\nworkspace = \"{WORKSPACE}\"\ntoken_endpoint = \"{uri}/v1/oauth/token\"\nrevocation_endpoint = \"{uri}/v1/oauth/revoke\"\n"
    );
    let credentials = format!(
      "[profiles.default.oauth]\nclient_id = \"oac_test\"\naccess_token = \"{OAUTH_TOKEN}\"\nrefresh_token = \"oauth_rt_testrefresh\"\nexpires_at = {expires_at}\nscopes = [\"domains:read\"]\n"
    );

    std::fs::write(self.config_dir.path().join("config.toml"), config).unwrap();
    std::fs::write(self.config_dir.path().join("credentials.toml"), credentials).unwrap();
  }

  pub async fn requests(&self) -> Vec<wiremock::Request> {
    self.server.received_requests().await.unwrap_or_default()
  }
}

pub fn parse_json(output: &[u8]) -> Value {
  serde_json::from_slice(output).unwrap()
}
