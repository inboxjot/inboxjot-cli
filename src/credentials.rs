use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::keychain::{self, Lookup};

const CONFIG_FILE: &str = "config.toml";
const CREDENTIALS_FILE: &str = "credentials.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
  Env,
  Keychain,
  File,
}

impl Source {
  pub fn label(self) -> &'static str {
    match self {
      Self::Env => "INBOXJOT_API_KEY",
      Self::Keychain => "keychain",
      Self::File => "credentials file",
    }
  }
}

#[derive(Clone, Debug)]
pub struct Credential {
  pub api_key: String,
  pub source: Source,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Secrets {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub api_key: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub oauth: Option<OAuthTokens>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OAuthTokens {
  pub client_id: String,
  pub access_token: String,
  pub refresh_token: String,
  pub expires_at: i64,
  #[serde(default)]
  pub scopes: Vec<String>,
  #[serde(default)]
  pub scopes_supported: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub refresh_expires_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
  #[serde(default)]
  pub profiles: BTreeMap<String, Profile>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Profile {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub api_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub dashboard_url: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub workspace: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub workspace_name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub user_name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub user_email: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub token_endpoint: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub revocation_endpoint: Option<String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct CredentialsFile {
  #[serde(default)]
  profiles: BTreeMap<String, Secrets>,
}

pub fn get_config_dir() -> Result<PathBuf> {
  if let Some(dir) = std::env::var_os("INBOXJOT_CONFIG_DIR") {
    return Ok(PathBuf::from(dir));
  }

  let strategy = etcetera::choose_base_strategy().map_err(|error| {
    Error::failed("config_error", format!("Could not find the home directory: {error}"))
  })?;

  Ok(strategy.config_dir().join("inboxjot"))
}

pub fn read_config() -> Result<Config> {
  read_toml(&get_config_dir()?.join(CONFIG_FILE))
}

pub fn write_config(config: &Config) -> Result<()> {
  write_toml(&get_config_dir()?.join(CONFIG_FILE), config)
}

pub fn read_profile(profile: &str) -> Result<Profile> {
  Ok(read_config()?.profiles.remove(profile).unwrap_or_default())
}

pub fn save_profile(profile: &str, value: Profile) -> Result<()> {
  let mut config = read_config()?;
  config.profiles.insert(profile.to_owned(), value);
  write_config(&config)
}

pub fn read_env_api_key() -> Option<String> {
  std::env::var("INBOXJOT_API_KEY").ok().filter(|key| !key.is_empty())
}

pub fn read_credential(profile: &str) -> Result<Option<Credential>> {
  if let Some(api_key) = read_env_api_key() {
    return Ok(Some(Credential { api_key, source: Source::Env }));
  }

  let Some((secrets, source)) = read_secrets(profile)? else {
    return Ok(None);
  };

  Ok(secrets.api_key.map(|api_key| Credential { api_key, source }))
}

pub fn read_secrets(profile: &str) -> Result<Option<(Secrets, Source)>> {
  if is_keyring_enabled()
    && let Lookup::Found(text) = keychain::read(profile)?
  {
    return Ok(Some((parse_secrets(&text), Source::Keychain)));
  }

  let file: CredentialsFile = read_toml(&get_credentials_path()?)?;
  Ok(
    file
      .profiles
      .get(profile)
      .map(|secrets| (secrets.clone(), Source::File)),
  )
}

pub fn save_secrets(profile: &str, secrets: &Secrets) -> Result<Source> {
  let text = serde_json::to_string(secrets)?;
  if is_keyring_enabled() && keychain::write(profile, &text)? {
    remove_file_secrets(profile)?;
    return Ok(Source::Keychain);
  }

  let path = get_credentials_path()?;
  let mut file: CredentialsFile = read_toml(&path)?;
  file.profiles.insert(profile.to_owned(), secrets.clone());
  write_toml(&path, &file)?;

  Ok(Source::File)
}

pub fn delete_secrets(profile: &str) -> Result<bool> {
  let deleted = is_keyring_enabled() && keychain::delete(profile)?;
  Ok(remove_file_secrets(profile)? || deleted)
}

fn parse_secrets(text: &str) -> Secrets {
  serde_json::from_str(text)
    .unwrap_or_else(|_| Secrets { api_key: Some(text.to_owned()), oauth: None })
}

fn remove_file_secrets(profile: &str) -> Result<bool> {
  let path = get_credentials_path()?;
  let mut file: CredentialsFile = read_toml(&path)?;
  if file.profiles.remove(profile).is_none() {
    return Ok(false);
  }

  write_toml(&path, &file)?;
  Ok(true)
}

fn is_keyring_enabled() -> bool {
  std::env::var_os("__INBOXJOT_FILE_CREDENTIALS").is_none()
}

fn get_credentials_path() -> Result<PathBuf> {
  Ok(get_config_dir()?.join(CREDENTIALS_FILE))
}

fn read_toml<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
  let text = match fs::read_to_string(path) {
    Ok(text) => text,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
    Err(error) => return Err(error.into()),
  };

  toml::from_str(&text).map_err(|error| {
    Error::failed("config_error", format!("{} is not valid: {error}", path.display()))
  })
}

fn write_toml(path: &Path, value: &impl Serialize) -> Result<()> {
  let text = toml::to_string_pretty(value).map_err(|error| {
    Error::failed("config_error", format!("Could not write {}: {error}", path.display()))
  })?;

  write_private_file(path, text.as_bytes())
}

pub fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
  if let Some(dir) = path.parent() {
    fs::create_dir_all(dir)?;
  }

  let mut options = fs::OpenOptions::new();
  options.write(true).create(true).truncate(true);
  #[cfg(unix)]
  std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);

  std::io::Write::write_all(&mut options.open(path)?, bytes)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::parse_secrets;

  #[test]
  fn reads_a_plain_key_saved_by_an_older_version() {
    let secrets = parse_secrets("ibx_abcd1234_secret");
    assert_eq!(secrets.api_key.as_deref(), Some("ibx_abcd1234_secret"));
    assert!(secrets.oauth.is_none());

    let secrets = parse_secrets(r#"{"api_key":"ibx_a_b"}"#);
    assert_eq!(secrets.api_key.as_deref(), Some("ibx_a_b"));
  }
}
