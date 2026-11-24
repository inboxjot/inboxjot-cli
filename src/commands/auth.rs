use std::io::Read;

use inboxjot::ErrorName;
use serde_json::{Value, json};

use super::open::open_browser;
use crate::api::Api;
use crate::context::Context;
use crate::credentials::{self, OAuthTokens, Profile, Secrets, Source};
use crate::error::{Error, Kind, Result};
use crate::login_page::Access;
use crate::oauth::{self, Grant};
use crate::output::column;
use crate::ui::mark::Mark;
use crate::ui::spinner;
use crate::ui::style::{ACTION, DANGER, DIM, FAINT, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

const KEY_PREFIX: &str = "ibx_";

#[derive(Debug, clap::Args)]
pub struct LoginArgs {
  #[arg(
    long,
    value_name = "-",
    help = "Read an API key from stdin instead of logging in with the browser, such as --api-key - < key.txt"
  )]
  pub api_key: Option<String>,
}

pub async fn login(args: LoginArgs, context: &Context) -> Result<()> {
  if args.api_key.is_some() || !context.interactive {
    return login_with_key(args.api_key.as_deref(), context).await;
  }

  context.output.print_header();
  if context.output.is_rich() {
    anstream::eprintln!();
  }
  login_with_browser(context).await
}

async fn login_with_key(flag: Option<&str>, context: &Context) -> Result<()> {
  let api_key = read_api_key(flag)?;
  let api = context.api_with_key(api_key.clone())?;
  let permission = get_permission(&api).await?;

  let secrets = Secrets { api_key: Some(api_key.clone()), oauth: None };
  let source = credentials::save_secrets(&context.profile, &secrets)?;
  let profile = Profile {
    api_url: Some(context.get_api_url()?),
    dashboard_url: context.read_dashboard_url()?,
    ..Profile::default()
  };
  credentials::save_profile(&context.profile, profile)?;

  let result = json!({
    "profile": context.profile,
    "workspace": null,
    "key": get_key_hint(&api_key),
    "permission": permission,
    "source": source.label(),
  });
  print_login(&result, source, context)
}

async fn login_with_browser(context: &Context) -> Result<()> {
  let api_url = context.get_api_url()?;
  let http = oauth::build_http()?;
  let discovery = oauth::read_discovery(&http, &api_url).await?;

  let login_code = oauth::create_login_code()?;
  let mut spinner = None;
  let mut opened = false;
  let grant = oauth::authorize(&http, &discovery, &api_url, &login_code, |url| {
    opened = open_browser(url);
    if !opened {
      context
        .output
        .info(&format!("Open this address in your browser: {url}"));
    }
    let hint = Line::new(vec![
      Span::styled("  ctrl+c", DIM),
      Span::styled(" cancel · nothing is saved until you approve", FAINT),
    ]);
    spinner = Some(context.output.start_spinner(
      &format!("Approve in the browser if it shows the code {login_code}"),
      &[hint],
    ));
  })
  .await;
  if let Some(spinner) = spinner {
    spinner.finish();
  }
  if grant.is_ok() && context.output.is_rich() {
    let action = if opened { "Opened the browser" } else { "Approved in the browser" };
    anstream::eprintln!(
      "{}",
      Line::new(vec![
        Span::raw("  "),
        Mark::Done.span(),
        Span::raw(format!(" {action}")),
        Span::styled("  code ", DIM),
        Span::styled(login_code.clone(), TITLE),
        Span::styled(" matched", DIM),
      ])
    );
  }

  let Grant {
    tokens,
    workspace_slug,
    workspace_name,
    user_name,
    user_email,
  } = grant?;
  let workspace_label = workspace_name.clone();
  let workspace = workspace_slug.ok_or_else(|| {
    Error::failed("login_failed", "The login did not name a workspace.")
  })?;

  revoke_saved_login(context).await;

  let secrets = Secrets { api_key: None, oauth: Some(tokens) };
  let source = credentials::save_secrets(&context.profile, &secrets)?;
  let profile = Profile {
    api_url: Some(api_url),
    dashboard_url: context.read_dashboard_url()?,
    workspace: Some(workspace.clone()),
    workspace_name: workspace_name.clone(),
    user_name: user_name.clone(),
    user_email,
    token_endpoint: Some(discovery.token_endpoint),
    revocation_endpoint: Some(discovery.revocation_endpoint),
  };
  credentials::save_profile(&context.profile, profile)?;

  let result = json!({
    "profile": context.profile,
    "workspace": workspace,
    "source": source.label(),
  });
  if !context.output.is_rich() {
    return print_login(&result, source, context);
  }

  let label = workspace_label.unwrap_or_else(|| workspace.clone());
  match user_name {
    Some(name) => {
      let detail = if label == workspace {
        workspace.clone()
      } else {
        format!("{label} · {workspace}")
      };
      context
        .output
        .done_with(&[("Logged in as ", false), (&name, true)], &detail);
    }
    None => {
      let detail = if label == workspace { String::new() } else { workspace.clone() };
      context.output.done(&format!("Logged in to {label}"), &detail);
    }
  }
  context.output.print_next(&[("set up this project", "inboxjot init")]);
  Ok(())
}

fn print_login(result: &Value, source: Source, context: &Context) -> Result<()> {
  let workspace = result.get("workspace").and_then(Value::as_str);
  let headline = workspace
    .map_or_else(|| "Logged in".to_owned(), |slug| format!("Logged in to {slug}"));

  let columns = if workspace.is_some() {
    vec![
      column("Profile", "profile"),
      column("Workspace", "workspace"),
      column("Stored in", "source"),
    ]
  } else {
    vec![
      column("Profile", "profile"),
      column("Key", "key"),
      column("Permission", "permission"),
      column("Stored in", "source"),
    ]
  };
  context.output.print_result(result, &headline, "profile", &columns)?;

  if source == Source::File {
    let path = credentials::get_config_dir()?.join("credentials.toml");
    context.output.warn(&format!(
      "No keychain is available, so the credentials are in {} with access for you only.",
      path.display()
    ));
  }

  Ok(())
}

pub async fn logout(context: &Context) -> Result<()> {
  let workspace = credentials::read_profile(&context.profile)
    .ok()
    .and_then(|profile| profile.workspace);
  let saved = credentials::read_secrets(&context.profile).ok().flatten();
  let had_login = saved.as_ref().is_some_and(|(secrets, _)| secrets.oauth.is_some());
  let store = saved.as_ref().map(|(_, source)| match source {
    Source::Keychain => "the keychain",
    _ => "the credentials file",
  });
  let message = context.output.shows_loading().then_some("Logging out…");
  let revoked = spinner::show_loading(message, revoke_saved_login(context)).await;

  let deleted = credentials::delete_secrets(&context.profile)?;
  let mut config = credentials::read_config()?;
  if config.profiles.remove(&context.profile).is_some() {
    credentials::write_config(&config)?;
  }

  let outcome = if deleted { "deleted" } else { "unchanged" };
  let result = json!({ "profile": context.profile, "outcome": outcome });
  if context.output.is_json() {
    context.output.print_json(&result)?;
  } else if deleted {
    let headline = match &workspace {
      Some(workspace) => format!("Logged out of {workspace}"),
      None => "Logged out".to_owned(),
    };
    context.output.removed(&headline);
    let store = store.unwrap_or("the credentials file");
    let detail = match (had_login, revoked) {
      (true, true) => format!("Revoked the login and removed it from {store}."),
      (true, false) => format!(
        "Removed the login from {store}. The server did not answer, so it ends when it expires."
      ),
      _ => format!("Removed the API key from {store}."),
    };
    context.output.info(&detail);
  } else {
    context
      .output
      .info(&format!("No credentials are saved for the profile {}.", context.profile));
  }

  if credentials::read_env_api_key().is_some() && !context.output.is_json() {
    if context.output.is_rich() {
      anstream::eprintln!();
    }
    context.output.warn("INBOXJOT_API_KEY is still set in this shell.");
    if context.output.is_rich() {
      anstream::eprintln!(
        "{}",
        Line::new(vec![
          Span::raw("    "),
          Span::styled("unset INBOXJOT_API_KEY", ACTION),
          Span::styled("   to stop using it", DIM),
        ])
      );
    }
  }

  Ok(())
}

async fn revoke_saved_login(context: &Context) -> bool {
  let Ok(profile) = credentials::read_profile(&context.profile) else {
    return false;
  };
  let Ok(Some((Secrets { oauth: Some(tokens), .. }, _))) =
    credentials::read_secrets(&context.profile)
  else {
    return false;
  };

  let (Some(endpoint), Ok(http)) = (&profile.revocation_endpoint, oauth::build_http())
  else {
    return false;
  };
  oauth::revoke(&http, endpoint, &tokens).await.is_ok()
}

pub async fn whoami(context: &Context) -> Result<()> {
  let secrets = credentials::read_secrets(&context.profile)?;
  let api_url = context.get_api_url()?;
  let api_url = api_url.trim_end_matches('/');

  if let Some(credential) = credentials::read_credential(&context.profile)? {
    let is_env = credential.source == Source::Env;
    let permission = get_permission(
      &context
        .api_with_key(credential.api_key.clone())?
        .loading("Checking your login…"),
    )
    .await?;
    let result = json!({
      "profile": if is_env { Value::Null } else { json!(context.profile) },
      "login": "api key",
      "key": get_key_hint(&credential.api_key),
      "permission": permission,
      "source": credential.source.label(),
      "api_url": api_url,
    });

    return context.output.print_detail(
      &result,
      &[
        column("Profile", "profile"),
        column("Login", "login"),
        column("Key", "key"),
        column("Permission", "permission"),
        column("Source", "source"),
        column("API URL", "api_url"),
      ],
    );
  }

  if !matches!(&secrets, Some((Secrets { oauth: Some(_), .. }, _))) {
    return Err(Error::not_logged_in());
  }
  let status = check_login(context).await?;
  let profile = credentials::read_profile(&context.profile)?;
  let Some((Secrets { oauth: Some(tokens), .. }, source)) =
    credentials::read_secrets(&context.profile)?
  else {
    return Err(Error::not_logged_in());
  };

  let result = json!({
    "profile": context.profile,
    "workspace": profile.workspace,
    "login": "browser",
    "status": status.label(),
    "access": get_access_label(&tokens),
    "scopes": tokens.scopes,
    "source": source.label(),
    "api_url": api_url,
  });

  if context.output.is_rich() && context.output.is_stdout_tty() {
    let slug = profile.workspace.clone().unwrap_or_default();
    let name = profile.workspace_name.clone().unwrap_or_else(|| slug.clone());
    let store = match source {
      Source::Keychain if cfg!(target_os = "macos") => "macOS Keychain",
      Source::Keychain => "the system keychain",
      _ => "the credentials file",
    };
    let field = |label: &str, value: String| {
      Line::new(vec![Span::styled(format!("  {label:<11}"), DIM), Span::raw(value)])
    };
    let mut lines = Vec::new();
    if let Some(user) = &profile.user_name {
      let mut title = vec![Span::styled(format!("  {user}"), TITLE)];
      if let Some(email) = &profile.user_email {
        title.push(Span::styled(format!("  {email}"), SECONDARY));
      }
      lines.push(Line::new(title));
      lines.push(Line::blank());
    }
    lines.push(field("workspace", name.clone()));
    if name != slug {
      lines.push(Line::new(vec![Span::raw(" ".repeat(13)), Span::styled(slug, DIM)]));
    }
    let login_label = Span::styled(format!("  {:<11}", "login"), DIM);
    lines.push(match &status {
      LoginStatus::Active => {
        let login = match format_expiry(tokens.refresh_expires_at) {
          Some(expiry) => format!("Browser · {expiry}"),
          None => "Browser".to_owned(),
        };
        field("login", login)
      }
      LoginStatus::Lost => Line::new(vec![
        login_label,
        Mark::Error.span(),
        Span::styled(" Revoked or expired", DANGER),
      ]),
      LoginStatus::Unchecked(host) => Line::new(vec![
        login_label,
        Span::styled(format!("Browser · not checked, can't reach {host}"), SECONDARY),
      ]),
    });
    lines.push(field("stored", format!("{store} · profile {}", context.profile)));
    if status == LoginStatus::Active {
      lines.push(field("access", get_access_areas(&tokens)));
    }
    for line in lines {
      anstream::println!("{line}");
    }
    context.output.print_next(&[match status {
      LoginStatus::Active => ("log out on this machine", "inboxjot logout"),
      LoginStatus::Lost => ("log in again", "inboxjot login"),
      LoginStatus::Unchecked(_) => {
        ("check the setup and the connection", "inboxjot doctor")
      }
    }]);
    return status.into_result();
  }

  context.output.print_detail(
    &result,
    &[
      column("Workspace", "workspace"),
      column("Profile", "profile"),
      column("Login", "login"),
      column("Status", "status"),
      column("Access", "access"),
      column("Source", "source"),
      column("API URL", "api_url"),
    ],
  )?;
  status.into_result()
}

#[derive(PartialEq, Eq)]
enum LoginStatus {
  Active,
  Lost,
  Unchecked(String),
}

impl LoginStatus {
  fn label(&self) -> &'static str {
    match self {
      Self::Active => "active",
      Self::Lost => "revoked",
      Self::Unchecked(_) => "unchecked",
    }
  }

  fn into_result(self) -> Result<()> {
    let message = match self {
      Self::Active => return Ok(()),
      Self::Lost => "This login no longer works",
      Self::Unchecked(_) => "The login could not be checked",
    };
    let mut error = Error::failed("login_not_active", message);
    error.kind = Kind::Quiet;
    Err(error)
  }
}

async fn check_login(context: &Context) -> Result<LoginStatus> {
  let checked = match context.api().await.map(|api| api.loading("Checking your login…"))
  {
    Ok(api) => api.get(&["v1", "domains"], &[("per_page", 1)]).await.map(|_| ()),
    Err(error) => Err(error),
  };
  match checked {
    Ok(()) => Ok(LoginStatus::Active),
    Err(error) if error.has_name(ErrorName::Forbidden) => Ok(LoginStatus::Active),
    Err(error) if error.is_lost_login() => Ok(LoginStatus::Lost),
    Err(error) if error.name == "network_error" => {
      let api_url = context.get_api_url()?;
      let host = api_url.split("://").nth(1).unwrap_or(&api_url).trim_end_matches('/');
      Ok(LoginStatus::Unchecked(host.to_owned()))
    }
    Err(error) => Err(error),
  }
}

pub fn format_expiry(expires_at: Option<i64>) -> Option<String> {
  let seconds = expires_at? - jiff::Timestamp::now().as_second();
  let days = seconds / 86_400;
  Some(match days {
    _ if seconds <= 0 => "expired".to_owned(),
    0 => "expires today".to_owned(),
    1 => "expires in 1 day".to_owned(),
    days => format!("expires in {days} days"),
  })
}

fn get_access_areas(tokens: &OAuthTokens) -> String {
  let mut areas: Vec<String> = Vec::new();
  for scope in &tokens.scopes {
    let area = scope.split(':').next().unwrap_or(scope).replace('_', " ");
    if !areas.contains(&area) {
      areas.push(area);
    }
  }
  if areas.is_empty() { get_access_label(tokens) } else { areas.join(", ") }
}

fn get_access_label(tokens: &OAuthTokens) -> String {
  match Access::read(tokens) {
    Access::Full => "Full Access".to_owned(),
    Access::ReadOnly => "Read Only".to_owned(),
    Access::Custom => format!("Custom ({})", tokens.scopes.join(", ")),
    Access::Unknown => "Unknown".to_owned(),
  }
}

fn read_api_key(flag: Option<&str>) -> Result<String> {
  let api_key = match flag {
    Some("-") => {
      let mut input = String::new();
      std::io::stdin().read_to_string(&mut input)?;
      input
    }
    Some(_) => {
      return Err(Error::usage(
        "api_key_in_argument",
        "Pass the API key on stdin with --api-key -, so it stays out of your shell history.",
      ));
    }
    None => {
      return Err(
        Error::usage(
          "missing_input",
          "Log in from a terminal, or pass an API key on stdin.",
        )
        .with_next(
          "inboxjot login --api-key - < key.txt",
          "Log in with an API key from a file",
        ),
      );
    }
  };

  let api_key = api_key.trim().to_owned();
  if !api_key.starts_with(KEY_PREFIX) {
    return Err(Error::usage("invalid_api_key", "API keys start with ibx_."));
  }

  Ok(api_key)
}

pub async fn get_permission(api: &Api) -> Result<&'static str> {
  match api.get(&["v1", "domains"], &[("per_page", 1)]).await {
    Ok(_) => Ok("full"),
    Err(error) if error.kind == Kind::Auth && error.has_name(ErrorName::Forbidden) => {
      Ok("limited")
    }
    Err(error) => Err(error),
  }
}

fn get_key_hint(api_key: &str) -> String {
  let mut parts = api_key.splitn(3, '_');
  match (parts.next(), parts.next(), parts.next()) {
    (Some(prefix), Some(short), Some(_)) => format!("{prefix}_{short}_…"),
    _ => format!("{}…", api_key.chars().take(8).collect::<String>()),
  }
}

#[cfg(test)]
mod tests {
  use super::{get_access_label, get_key_hint};
  use crate::credentials::OAuthTokens;

  fn build_tokens(scopes: &[&str], supported: &[&str]) -> OAuthTokens {
    OAuthTokens {
      client_id: String::new(),
      access_token: String::new(),
      refresh_token: String::new(),
      expires_at: 0,
      scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
      scopes_supported: supported.iter().map(|scope| (*scope).to_owned()).collect(),
      refresh_expires_at: None,
    }
  }

  #[test]
  fn says_when_the_login_expires() {
    let now = jiff::Timestamp::now().as_second();
    assert_eq!(
      super::format_expiry(Some(now + 29 * 86_400 + 60)).unwrap(),
      "expires in 29 days"
    );
    assert_eq!(super::format_expiry(Some(now + 60)).unwrap(), "expires today");
    assert_eq!(super::format_expiry(Some(now - 1)).unwrap(), "expired");
    assert_eq!(super::format_expiry(None), None);
  }

  #[test]
  fn labels_access_against_the_supported_scopes() {
    let supported = ["domains:read", "domains:write", "emails:read", "emails:write"];
    let label = |scopes: &[&str]| get_access_label(&build_tokens(scopes, &supported));

    assert_eq!(label(&supported), "Full Access");
    assert_eq!(label(&["emails:read", "domains:read"]), "Read Only");
    assert_eq!(label(&["domains:read"]), "Custom (domains:read)");
    assert_eq!(label(&[]), "Unknown");
    assert_eq!(
      get_access_label(&build_tokens(&["domains:read"], &[])),
      "Custom (domains:read)"
    );
  }

  #[test]
  fn hides_the_secret_part_of_a_key() {
    assert_eq!(get_key_hint("ibx_ab12cd34_secretsecretsecret"), "ibx_ab12cd34_…");
    assert_eq!(get_key_hint("ibx_short"), "ibx_shor…");
  }
}
