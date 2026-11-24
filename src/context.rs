use std::io::IsTerminal;

use crate::api::{Api, DEFAULT_API_URL};
use crate::cli::GlobalArgs;
use crate::credentials;
use crate::error::{Error, Result};
use crate::oauth;
use crate::output::{Format, Output, clean};
use crate::ui::prompt::{self, Confirm, Input};
use crate::ui::style::{SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

const TOKEN_MARGIN_SECONDS: i64 = 60;
const AGENT_VARIABLES: [&str; 4] = ["CLAUDECODE", "CODEX_SANDBOX", "GEMINI_CLI", "AGENT"];

#[derive(Debug)]
pub struct Context {
  pub output: Output,
  pub interactive: bool,
  pub yes: bool,
  pub profile: String,
  debug: bool,
  api_url: Option<String>,
  dashboard_url: Option<String>,
}

impl Context {
  pub fn new(global: &GlobalArgs, argv: Vec<String>) -> Self {
    let is_agent = is_agent();
    let is_json = global.json || global.jq.is_some() || is_agent;
    let format = if is_json { Format::Json } else { Format::Text };
    let is_agent_mode = global.non_interactive.unwrap_or(is_agent);

    let has_terminal = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    let is_ci = std::env::var_os("CI").is_some();
    let interactive = match global.non_interactive {
      Some(true) => false,
      Some(false) => has_terminal && !is_ci && !global.json && global.jq.is_none(),
      None => has_terminal && !is_ci && !is_json,
    };

    Self {
      output: Output::new(format, global.jq.clone(), is_agent_mode, argv),
      interactive,
      yes: global.yes,
      profile: global.profile.clone(),
      debug: global.debug,
      api_url: global.api_url.clone(),
      dashboard_url: global.dashboard_url.clone(),
    }
  }

  pub async fn api(&self) -> Result<Api> {
    if let Some(credential) = credentials::read_credential(&self.profile)? {
      return self.build_api(credential.api_key);
    }

    let (access_token, _) =
      self.read_session().await?.ok_or_else(Error::not_logged_in)?;
    self.build_api(access_token)
  }

  pub fn get_workspace(&self) -> Option<String> {
    if credentials::read_credential(&self.profile).ok().flatten().is_some() {
      return None;
    }
    credentials::read_profile(&self.profile).ok()?.workspace
  }

  pub async fn login_api(&self) -> Result<Api> {
    let (access_token, _) =
      self.read_session().await?.ok_or_else(Error::needs_browser_login)?;
    self.build_api(access_token)
  }

  async fn read_session(&self) -> Result<Option<(String, String)>> {
    let profile = credentials::read_profile(&self.profile)?;
    let Some((mut secrets, _)) = credentials::read_secrets(&self.profile)? else {
      return Ok(None);
    };
    let (Some(tokens), Some(workspace)) = (secrets.oauth.clone(), profile.workspace)
    else {
      return Ok(None);
    };

    let is_expiring =
      tokens.expires_at - TOKEN_MARGIN_SECONDS <= jiff::Timestamp::now().as_second();
    if !is_expiring {
      return Ok(Some((tokens.access_token, workspace)));
    }

    let Some(token_endpoint) = profile.token_endpoint else {
      return Ok(None);
    };
    let api_url = self.get_api_url()?;
    let grant =
      oauth::refresh(&oauth::build_http()?, &token_endpoint, &api_url, &tokens).await?;
    let access_token = grant.tokens.access_token.clone();
    secrets.oauth = Some(grant.tokens);
    credentials::save_secrets(&self.profile, &secrets)?;
    if grant.user_name.is_some() {
      let mut profile = credentials::read_profile(&self.profile)?;
      profile.user_name = grant.user_name;
      profile.user_email = grant.user_email;
      profile.workspace_name = grant.workspace_name.or(profile.workspace_name);
      credentials::save_profile(&self.profile, profile)?;
    }

    Ok(Some((access_token, workspace)))
  }

  fn build_api(&self, token: String) -> Result<Api> {
    Ok(
      Api::new(&self.get_api_url()?, token, self.debug)?
        .allow_loading(self.output.shows_loading()),
    )
  }

  pub fn api_with_key(&self, api_key: String) -> Result<Api> {
    self.build_api(api_key)
  }

  pub fn get_api_url(&self) -> Result<String> {
    if let Some(url) = &self.api_url {
      return Ok(url.clone());
    }

    let url = credentials::read_profile(&self.profile)?.api_url;
    Ok(url.unwrap_or_else(|| DEFAULT_API_URL.to_owned()))
  }

  pub fn read_dashboard_url(&self) -> Result<Option<String>> {
    if let Some(url) = &self.dashboard_url {
      return Ok(Some(url.clone()));
    }

    Ok(credentials::read_profile(&self.profile)?.dashboard_url)
  }

  pub fn confirm(&self, question: &str) -> Result<()> {
    if self.yes {
      return Ok(());
    }

    if !self.interactive {
      return Err(self.get_confirmation_error(question));
    }

    let confirmed = prompt::run(Confirm::new(clean(question), false))?;
    if confirmed { Ok(()) } else { Err(Error::canceled()) }
  }

  pub fn confirm_delete(&self, question: &str, consequence: &str) -> Result<()> {
    if self.yes {
      return Ok(());
    }

    if !self.interactive {
      return Err(self.get_confirmation_error(&format!("{question} {consequence}")));
    }

    anstream::eprintln!(
      "{}",
      Line::from(Span::styled(format!("  {}", clean(question)), TITLE))
    );
    anstream::eprintln!(
      "{}",
      Line::from(Span::styled(format!("  {}", clean(consequence)), SECONDARY))
    );
    anstream::eprintln!();
    let confirmed = prompt::run(Confirm::new(clean(question), false))?;
    if confirmed { Ok(()) } else { Err(Error::canceled()) }
  }

  pub fn proceed(&self, question: &str) -> Result<()> {
    if self.yes {
      return Ok(());
    }

    if !self.interactive {
      return Err(self.get_confirmation_error(question));
    }

    let confirmed = prompt::run(Confirm::new(clean(question), true))?;
    if confirmed { Ok(()) } else { Err(Error::canceled()) }
  }

  pub fn confirm_name(&self, noun: &str, name: &str, consequence: &str) -> Result<()> {
    self.confirm_typed(
      &[name],
      &format!("Type {} to confirm:", clean(name)),
      &format!("Type the {noun} name exactly, or press esc to cancel."),
      consequence,
    )
  }

  pub fn confirm_typed(
    &self,
    accepted: &[&str],
    question: &str,
    mismatch: &str,
    consequence: &str,
  ) -> Result<()> {
    if self.yes {
      return Ok(());
    }

    if !self.interactive {
      return Err(self.get_confirmation_error(consequence));
    }

    let accepted: Vec<String> =
      accepted.iter().map(|answer| (*answer).to_owned()).collect();
    let mismatch = mismatch.to_owned();
    let input = Input::new(question).with_validate(move |answer| {
      if accepted.iter().any(|accepted| accepted == answer) {
        Ok(())
      } else {
        Err(mismatch.clone())
      }
    });
    prompt::run(input)?;
    Ok(())
  }

  fn get_confirmation_error(&self, question: &str) -> Error {
    let message = format!("{question} Add --yes to confirm.");
    Error::usage("confirmation_required", message).with_next(
      self.output.get_command_with(&["--yes"]),
      "Run the command again after the user approves it",
    )
  }
}

fn is_agent() -> bool {
  AGENT_VARIABLES
    .iter()
    .any(|name| std::env::var(name).is_ok_and(|value| !value.is_empty() && value != "0"))
}
