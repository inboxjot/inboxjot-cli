use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::fmt::rfc2822::DateTimeParser;
use reqwest::header::{DATE, HeaderMap};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::net::TcpStream;

use crate::api::{Api, Page};
use crate::commands::agent::{ToolStatus, get_tool_statuses};
use crate::commands::auth::{format_expiry, get_permission};
use crate::context::Context;
use crate::credentials;
use crate::error::{Error, Result};
use crate::output::clean;
use crate::ui::mark::Mark;
use crate::ui::style::{ACTION, DANGER, DONE, FAINT, SECONDARY, TITLE, WARNING};
use crate::ui::text::{Line, Span};

const SMTP_HOST: &str = "smtp.inboxjot.com";
const SMTP_PORTS: [u16; 3] = [587, 2525, 465];
const SMTP_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_CLOCK_SKEW_SECONDS: i64 = 30;
const LABEL_WIDTH: usize = 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
  Pass,
  Warn,
  Fail,
}

#[derive(Debug, Serialize)]
struct Check {
  name: String,
  status: Status,
  message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  fix: Option<String>,
}

fn check(name: impl Into<String>, status: Status, message: impl Into<String>) -> Check {
  Check {
    name: name.into(),
    status,
    message: message.into(),
    fix: None,
  }
}

impl Check {
  fn with_fix(mut self, fix: impl Into<String>) -> Self {
    self.fix = Some(fix.into());
    self
  }
}

pub async fn run(context: &Context) -> Result<()> {
  if context.output.is_rich() {
    anstream::eprintln!("{}", Line::from(Span::styled("  Checking your setup", TITLE)));
    anstream::eprintln!();
  }

  let mut checks = vec![check("cli", Status::Pass, env!("CARGO_PKG_VERSION"))];
  let api = match context.api().await.map(|api| api.loading("Checking the setup…")) {
    Ok(api) => {
      checks.push(check("credential", Status::Pass, describe_login(context)));
      Some(api)
    }
    Err(error) => {
      checks.push(
        check("credential", Status::Fail, error.message.clone())
          .with_fix("inboxjot login"),
      );
      None
    }
  };

  if let Some(api) = &api {
    let can_read = check_api(api, &mut checks).await;
    checks.push(check_project_key(context).await);
    if can_read {
      check_domains(api, &mut checks).await;
    }
  }
  if let Some(agents) = check_agents() {
    checks.push(agents);
  }
  checks.push(check_smtp().await);

  print_checks(&checks, context)
}

fn describe_login(context: &Context) -> String {
  if let Ok(Some(credential)) = credentials::read_credential(&context.profile) {
    return format!("API key in {}", credential.source.label());
  }
  let profile = credentials::read_profile(&context.profile).unwrap_or_default();
  let expires_at = credentials::read_secrets(&context.profile)
    .ok()
    .flatten()
    .and_then(|(secrets, _)| secrets.oauth)
    .and_then(|tokens| tokens.refresh_expires_at);
  let who = profile.user_name.unwrap_or_else(|| "Browser login".to_owned());
  let mut text = match profile.workspace {
    Some(workspace) => format!("{who} in {workspace}"),
    None => who,
  };
  if let Some(expiry) = format_expiry(expires_at) {
    text.push_str(&format!(" · {expiry}"));
  }
  text
}

async fn check_project_key(context: &Context) -> Check {
  let Some((file, key)) = find_project_key() else {
    return check("api_key", Status::Warn, "No INBOXJOT_API_KEY in this folder")
      .with_fix("inboxjot init");
  };
  let access = match context
    .api_with_key(key)
    .map(|api| api.loading("Checking the setup…"))
  {
    Ok(api) => match get_permission(&api).await {
      Ok("full") => "Full access",
      Ok(_) => "Sending access",
      Err(_) => {
        return check(
          "api_key",
          Status::Fail,
          format!("INBOXJOT_API_KEY in {file} does not work"),
        )
        .with_fix("inboxjot init");
      }
    },
    Err(_) => "unknown access",
  };
  check("api_key", Status::Pass, format!("INBOXJOT_API_KEY in {file} · {access}"))
}

fn find_project_key() -> Option<(&'static str, String)> {
  [".env.local", ".env"].into_iter().find_map(|file| {
    let text = std::fs::read_to_string(file).ok()?;
    let key = text.lines().find_map(|line| {
      let (name, value) = line.split_once('=')?;
      let name = name.trim();
      let name = name.strip_prefix("export ").unwrap_or(name).trim();
      (name == "INBOXJOT_API_KEY")
        .then(|| value.trim().trim_matches(['"', '\'']).to_owned())
        .filter(|value| !value.is_empty())
    })?;
    Some((file, key))
  })
}

fn check_agents() -> Option<Check> {
  let tools = get_tool_statuses();
  if tools.is_empty() {
    return None;
  }
  let missing: Vec<&ToolStatus> = tools.iter().filter(|tool| !tool.is_set_up).collect();
  let ready: Vec<&str> = tools
    .iter()
    .filter(|tool| tool.is_set_up)
    .map(|tool| tool.name)
    .collect();
  if missing.is_empty() {
    return Some(check("agents", Status::Pass, format!("{} set up", join(&ready))));
  }

  let not_set_up: Vec<&str> = missing.iter().map(|tool| tool.name).collect();
  let verb = if not_set_up.len() == 1 { "is" } else { "are" };
  let message = if ready.is_empty() {
    format!("{} {verb} not set up", join(&not_set_up))
  } else {
    let ready_verb = if ready.len() == 1 { "is" } else { "are" };
    format!("{} {ready_verb} set up, {} {verb} not", join(&ready), join(&not_set_up))
  };
  let fix = match missing.as_slice() {
    [tool] => format!("inboxjot agent setup --client {}", tool.flag),
    _ => "inboxjot agent setup".to_owned(),
  };
  Some(check("agents", Status::Warn, message).with_fix(fix))
}

fn join(names: &[&str]) -> String {
  match names {
    [] => String::new(),
    [name] => (*name).to_owned(),
    [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
  }
}

async fn check_api(api: &Api, checks: &mut Vec<Check>) -> bool {
  let host = api.base_url().as_str().trim_end_matches('/');
  let host = host.split_once("://").map_or(host, |(_, rest)| rest).to_owned();
  let query = [("per_page".to_owned(), "1".to_owned())];
  let started = Instant::now();
  let url = match api.build_url(&["v1", "domains"]) {
    Ok(url) => url,
    Err(error) => {
      checks.push(check("api", Status::Fail, error.message.clone()));
      return false;
    }
  };
  let response = api.send_raw(Method::GET, url, &query, &HeaderMap::new(), None).await;
  let elapsed = started.elapsed().as_millis();

  let response = match response {
    Ok(response) => response,
    Err(error) => {
      checks.push(check("api", Status::Fail, error.message.clone()));
      return false;
    }
  };

  let (status, message) = match response.status {
    status if status.is_success() => (Status::Pass, format!("{host} · {elapsed}ms")),
    StatusCode::FORBIDDEN => {
      (Status::Warn, "The API key cannot read domains. It is a limited key.".to_owned())
    }
    StatusCode::UNAUTHORIZED => {
      (Status::Fail, "The API key is not valid. Run inboxjot login.".to_owned())
    }
    status => {
      (Status::Fail, format!("The API answered with status {}.", status.as_u16()))
    }
  };
  checks.push(check("api", status, message));
  if let Some(clock) = check_clock(&response.headers) {
    checks.push(clock);
  }

  status == Status::Pass
}

fn check_clock(headers: &HeaderMap) -> Option<Check> {
  let date = headers.get(DATE)?.to_str().ok()?;
  let server_time = DateTimeParser::new().parse_timestamp(date).ok()?;
  let skew = (Timestamp::now().as_second() - server_time.as_second()).abs();

  Some(if skew > MAX_CLOCK_SKEW_SECONDS {
    check(
      "clock",
      Status::Warn,
      format!("The clock is {skew}s off from the API. Webhook checks can fail."),
    )
  } else {
    check("clock", Status::Pass, format!("The clock is within {skew}s of the API."))
  })
}

async fn check_domains(api: &Api, checks: &mut Vec<Check>) {
  let page = Page { all: true, ..Page::default() };
  let domains = match api.list(&["v1", "domains"], &(), &page).await {
    Ok(page) => page,
    Err(error) => {
      checks.push(check("domains", Status::Fail, error.message.clone()));
      return;
    }
  };

  let domains = domains
    .get("data")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  let named = |state: &str| -> Vec<String> {
    domains
      .iter()
      .filter(|domain| domain.get("status").and_then(Value::as_str) == Some(state))
      .filter_map(|domain| domain.get("name").and_then(Value::as_str).map(str::to_owned))
      .collect()
  };
  let verified = named("verified");
  if verified.is_empty() {
    checks.push(
      check("domains", Status::Fail, "No verified domain")
        .with_fix("inboxjot domains add <domain>"),
    );
    return;
  }

  let waiting = domains.len() - verified.len();
  let mut message = verified.join(", ");
  if waiting > 0 {
    message.push_str(&format!(" · {waiting} not verified yet"));
  }
  checks.push(check("domains", Status::Pass, message));
}

async fn check_smtp() -> Check {
  let (host, ports) = get_smtp_target();
  let attempts: Vec<_> = ports
    .iter()
    .map(|&port| {
      let host = host.clone();
      tokio::spawn(async move {
        let connect = TcpStream::connect((host.as_str(), port));
        matches!(tokio::time::timeout(SMTP_TIMEOUT, connect).await, Ok(Ok(_)))
      })
    })
    .collect();

  let mut blocked = Vec::new();
  for (port, attempt) in ports.iter().zip(attempts) {
    if !attempt.await.unwrap_or(false) {
      blocked.push(port.to_string());
    }
  }

  let all = ports.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
  match blocked.len() {
    0 => check("smtp", Status::Pass, format!("Reached {host} on ports {all}.")),
    count if count == ports.len() => {
      check("smtp", Status::Fail, format!("Could not reach {host} on ports {all}."))
    }
    _ => check(
      "smtp",
      Status::Warn,
      format!("Could not reach {host} on ports {}.", blocked.join(", ")),
    ),
  }
}

fn get_smtp_target() -> (String, Vec<u16>) {
  let Some(target) = std::env::var("INBOXJOT_SMTP_HOST")
    .ok()
    .filter(|host| !host.is_empty())
  else {
    return (SMTP_HOST.to_owned(), SMTP_PORTS.to_vec());
  };

  match target
    .rsplit_once(':')
    .and_then(|(host, port)| Some((host, port.parse().ok()?)))
  {
    Some((host, port)) => (host.to_owned(), vec![port]),
    None => (target, SMTP_PORTS.to_vec()),
  }
}

fn print_checks(checks: &[Check], context: &Context) -> Result<()> {
  let count = |status| checks.iter().filter(|check| check.status == status).count();
  let (warnings, failures) = (count(Status::Warn), count(Status::Fail));

  if context.output.is_json() {
    context.output.print_json(&json!({ "checks": checks }))?;
  } else if context.output.is_rich() {
    let is_quiet = |check: &&Check| {
      matches!(check.name.as_str(), "clock" | "smtp") && check.status == Status::Pass
    };
    for check in checks.iter().filter(|check| !is_quiet(check)) {
      for line in render_check(check) {
        anstream::eprintln!("{line}");
      }
    }
    anstream::eprintln!();
    anstream::eprintln!("{}", render_summary(failures, warnings));
  } else {
    for check in checks {
      let status = match check.status {
        Status::Pass => "pass",
        Status::Warn => "warn",
        Status::Fail => "fail",
      };
      anstream::println!("{status} {} {}", check.name, clean(&check.message));
    }
  }

  if failures > 0 {
    let mut error = Error::failed(
      "checks_failed",
      format!("{failures} of {} checks failed.", checks.len()),
    );
    error.kind = crate::error::Kind::Quiet;
    return Err(error);
  }
  Ok(())
}

fn render_summary(failures: usize, warnings: usize) -> Line<'static> {
  if failures == 0 && warnings == 0 {
    return Line::new(vec![Span::styled("  All checks passed", DONE)]);
  }
  let plural = |count: usize, noun: &str| {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
  };
  let mut spans = vec![Span::raw("  ")];
  if failures > 0 {
    spans.push(Span::styled(plural(failures, "problem"), DANGER));
  }
  if warnings > 0 {
    if failures > 0 {
      spans.push(Span::styled(" · ", FAINT));
    }
    spans.push(Span::styled(plural(warnings, "warning"), WARNING));
  }
  Line::new(spans)
}

fn render_check(check: &Check) -> Vec<Line<'_>> {
  let mark = match check.status {
    Status::Pass => Mark::Done,
    Status::Warn => Mark::Warning,
    Status::Fail => Mark::Error,
  };
  let label = match check.name.as_str() {
    "cli" => "CLI",
    "credential" => "Login",
    "api" => "API",
    "api_key" => "API key",
    "clock" => "Clock",
    "domains" => "Domain",
    "agents" => "Agent tools",
    "smtp" => "SMTP",
    other => other,
  };
  let mut lines = vec![Line::new(vec![
    Span::raw("  "),
    mark.span(),
    Span::raw(" "),
    Span::raw(format!("{label:<LABEL_WIDTH$}")),
    Span::styled(check.message.as_str(), SECONDARY),
  ])];
  if let Some(fix) = &check.fix {
    lines.push(Line::new(vec![
      Span::raw(" ".repeat(LABEL_WIDTH + 4)),
      Span::styled(fix.as_str(), ACTION),
    ]));
  }
  lines
}
