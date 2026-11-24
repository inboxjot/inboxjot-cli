use crate::error::Result;

const SERVICE: &str = "inboxjot";

pub enum Lookup {
  Found(String),
  Missing,
  Unavailable,
}

#[cfg(target_os = "macos")]
mod store {
  use std::io::Write as _;
  use std::process::{Command, Stdio};

  use base64::Engine;
  use base64::engine::general_purpose::STANDARD as BASE64;

  use super::{Lookup, SERVICE};
  use crate::error::{Error, Result};

  const SECURITY: &str = "/usr/bin/security";
  const NOT_FOUND: i32 = 44;

  pub fn read(profile: &str) -> Result<Lookup> {
    let Ok(output) = Command::new(SECURITY)
      .args(["find-generic-password", "-s", SERVICE, "-a", profile, "-w"])
      .stderr(Stdio::null())
      .output()
    else {
      return Ok(Lookup::Unavailable);
    };

    match output.status.code() {
      Some(0) => {}
      Some(NOT_FOUND) => return Ok(Lookup::Missing),
      _ => return Ok(Lookup::Unavailable),
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let Some(text) = BASE64
      .decode(text.trim())
      .ok()
      .and_then(|bytes| String::from_utf8(bytes).ok())
    else {
      return Ok(Lookup::Missing);
    };
    Ok(Lookup::Found(text))
  }

  pub fn write(profile: &str, text: &str) -> Result<bool> {
    if profile
      .chars()
      .any(|character| matches!(character, '"' | '\\') || character.is_control())
    {
      return Err(Error::failed(
        "invalid_profile",
        "A profile name can't contain quotes, backslashes or control characters.",
      ));
    }

    let Ok(mut child) = Command::new(SECURITY)
      .arg("-i")
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
    else {
      return Ok(false);
    };

    let command = format!(
      "add-generic-password -U -s {SERVICE} -a \"{profile}\" -w {}\n",
      BASE64.encode(text)
    );
    let written = child
      .stdin
      .take()
      .is_some_and(|mut stdin| stdin.write_all(command.as_bytes()).is_ok());
    let stored = child.wait().is_ok_and(|status| status.success());
    Ok(written && stored)
  }

  pub fn delete(profile: &str) -> Result<bool> {
    let status = Command::new(SECURITY)
      .args(["delete-generic-password", "-s", SERVICE, "-a", profile])
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status();
    Ok(status.is_ok_and(|status| status.success()))
  }
}

#[cfg(not(target_os = "macos"))]
mod store {
  use super::{Lookup, SERVICE};
  use crate::error::{Error, Result};

  pub fn read(profile: &str) -> Result<Lookup> {
    match get_entry(profile)?.get_password() {
      Ok(text) => Ok(Lookup::Found(text)),
      Err(keyring::Error::NoEntry) => Ok(Lookup::Missing),
      Err(keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_)) => {
        Ok(Lookup::Unavailable)
      }
      Err(error) => Err(get_error(&error)),
    }
  }

  pub fn write(profile: &str, text: &str) -> Result<bool> {
    Ok(get_entry(profile)?.set_password(text).is_ok())
  }

  pub fn delete(profile: &str) -> Result<bool> {
    match get_entry(profile)?.delete_credential() {
      Ok(()) => Ok(true),
      Err(
        keyring::Error::NoEntry
        | keyring::Error::NoStorageAccess(_)
        | keyring::Error::PlatformFailure(_),
      ) => Ok(false),
      Err(error) => Err(get_error(&error)),
    }
  }

  fn get_entry(profile: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, profile).map_err(|error| get_error(&error))
  }

  fn get_error(error: &keyring::Error) -> Error {
    Error::failed("keychain_error", format!("The keychain failed: {error}"))
  }
}

pub fn read(profile: &str) -> Result<Lookup> {
  store::read(profile)
}

pub fn write(profile: &str, text: &str) -> Result<bool> {
  store::write(profile, text)
}

pub fn delete(profile: &str) -> Result<bool> {
  store::delete(profile)
}
