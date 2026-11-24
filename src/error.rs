use std::borrow::Cow;
use std::ops::{Deref, DerefMut};
use std::process::ExitCode;

use inboxjot::ErrorName;
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
  Failed,
  Quiet,
  Usage,
  Auth,
  Canceled,
}

#[derive(Debug, thiserror::Error)]
#[error("{}", .0.message)]
pub struct Error(Box<ErrorBody>);

#[derive(Debug)]
pub struct ErrorBody {
  pub kind: Kind,
  pub name: Cow<'static, str>,
  pub message: String,
  pub reason: Option<String>,
  pub details: Vec<ErrorItem>,
  pub request_id: Option<String>,
  pub next: Vec<Next>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Next {
  pub command: String,
  pub description: &'static str,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ErrorItem {
  pub message: String,
  pub location: String,
}

impl Error {
  pub fn new(
    kind: Kind,
    name: impl Into<Cow<'static, str>>,
    message: impl Into<String>,
  ) -> Self {
    Self(Box::new(ErrorBody {
      kind,
      name: name.into(),
      message: message.into(),
      reason: None,
      details: Vec::new(),
      request_id: None,
      next: Vec::new(),
    }))
  }

  pub fn failed(name: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
    Self::new(Kind::Failed, name, message)
  }

  pub fn usage(name: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
    Self::new(Kind::Usage, name, message)
  }

  pub fn auth(name: impl Into<Cow<'static, str>>, message: impl Into<String>) -> Self {
    Self::new(Kind::Auth, name, message)
  }

  pub fn canceled() -> Self {
    Self::new(Kind::Canceled, "canceled", "Canceled.")
  }

  pub fn not_logged_in() -> Self {
    Self::auth("not_logged_in", "You are not logged in.")
      .with_next("inboxjot login", "Log in with the browser")
  }

  pub fn needs_browser_login() -> Self {
    Self::auth("needs_browser_login", "This command needs a browser login.")
      .with_next("inboxjot login", "Log in with the browser")
  }

  pub fn with_next(
    mut self,
    command: impl Into<String>,
    description: &'static str,
  ) -> Self {
    self.next.push(Next { command: command.into(), description });
    self
  }

  pub fn explain_lost_login(mut self) -> Self {
    self.message = "This login no longer works".to_owned();
    self
      .with_reason("It was revoked from the dashboard or it expired.")
      .with_next("inboxjot login", "Log in again")
  }

  pub fn is_lost_login(&self) -> bool {
    self.kind == Kind::Auth
      && matches!(self.name.as_ref(), "unauthorized" | "login_expired")
  }

  pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
    self.reason = Some(reason.into());
    self
  }

  pub fn has_name(&self, name: ErrorName) -> bool {
    let value: std::result::Result<ErrorName, serde::de::value::Error> =
      ErrorName::deserialize(self.name.as_ref().into_deserializer());
    value.is_ok_and(|value| value == name)
  }

  pub fn is_action_required(&self) -> bool {
    matches!(self.kind, Kind::Usage | Kind::Auth)
  }

  pub fn exit_code(&self) -> ExitCode {
    match self.kind {
      Kind::Failed | Kind::Quiet => ExitCode::from(1),
      Kind::Usage => ExitCode::from(2),
      Kind::Auth => ExitCode::from(3),
      Kind::Canceled => ExitCode::from(130),
    }
  }
}

impl Deref for Error {
  type Target = ErrorBody;

  fn deref(&self) -> &ErrorBody {
    &self.0
  }
}

impl DerefMut for Error {
  fn deref_mut(&mut self) -> &mut ErrorBody {
    &mut self.0
  }
}

impl From<reqwest::Error> for Error {
  fn from(error: reqwest::Error) -> Self {
    let host = error
      .url()
      .and_then(|url| {
        let host = url.host_str()?;
        Some(
          url
            .port()
            .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}")),
        )
      })
      .unwrap_or_else(|| "the Inboxjot API".to_owned());
    let reason = if error.is_timeout() {
      format!("The request timed out after {}s.", crate::api::TIMEOUT.as_secs())
    } else if error.is_connect() && format!("{error:?}").contains("dns error") {
      format!("{host} could not be found. Check the address and the network.")
    } else if error.is_connect() {
      "The connection was refused.".to_owned()
    } else {
      format!("The request failed: {error}")
    };

    Self::failed("network_error", format!("Can't reach {host}"))
      .with_reason(reason)
      .with_next("inboxjot doctor", "Check the setup and the connection")
  }
}

impl From<std::io::Error> for Error {
  fn from(error: std::io::Error) -> Self {
    Self::failed("io_error", error.to_string())
  }
}

impl From<serde_json::Error> for Error {
  fn from(error: serde_json::Error) -> Self {
    Self::failed("invalid_json", format!("The JSON is not valid: {error}"))
  }
}
