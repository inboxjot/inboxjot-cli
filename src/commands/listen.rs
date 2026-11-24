use std::fs;
use std::io::ErrorKind;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::Args;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::Url;
use reqwest::header::CONTENT_TYPE;
use sha2::Sha256;

use super::stream::{Event, EventStream, EventType, Item, run_until_stopped};
use crate::api::CLI_USER_AGENT;
use crate::context::Context;
use crate::credentials::{get_config_dir, write_private_file};
use crate::error::{Error, Result};
use crate::output::clean;
use crate::time::parse_duration;
use crate::ui::feed::Feed;
use crate::ui::mark::Mark;
use crate::ui::style::{
  DANGER, DIM, DONE, FAINT, SECONDARY, TITLE, WARNING, get_event_style,
};
use crate::ui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

const SECRET_FILE: &str = "listen-secret";
const SECRET_PREFIX: &str = "whsec_";
const FORWARD_TIMEOUT: Duration = Duration::from_secs(10);
const EVENT_WIDTH: usize = 18;
const PROBLEM_LENGTH: usize = 120;

#[derive(Debug, Args)]
pub struct ListenArgs {
  #[arg(
    long,
    value_name = "URL",
    help = "Post each event to this URL, such as localhost:3000/webhooks"
  )]
  pub forward_to: Option<String>,

  #[arg(
    long,
    value_enum,
    value_delimiter = ',',
    hide_possible_values = true,
    help = "Show only these event types, separated by commas"
  )]
  pub events: Vec<EventType>,

  #[arg(long, help = "Print the signing secret of forwarded events and exit")]
  pub print_secret: bool,

  #[arg(long, value_parser = parse_duration, help = "Stop listening after this long")]
  pub timeout: Option<Duration>,
}

struct Forward {
  http: reqwest::Client,
  url: Url,
  label: String,
  key: Vec<u8>,
}

pub async fn run(args: ListenArgs, context: &Context) -> Result<()> {
  let secret = read_secret()?;
  if args.print_secret {
    context.output.print_line(&secret);
    return Ok(());
  }

  let forward = args
    .forward_to
    .as_deref()
    .map(|url| build_forward(url, &secret))
    .transpose()?;
  let mut stream = EventStream::connect(context).await?;
  print_header(forward.as_ref(), &secret, context);

  let mut feed = Feed::new(context.output.is_rich(), true, "Waiting for events");
  let work = async {
    loop {
      match stream.next().await? {
        Item::Event(event) => {
          if !args.events.is_empty() && !args.events.contains(&event.get_kind()) {
            continue;
          }
          let delivery = match &forward {
            Some(forward) => Some(send_event(forward, &event).await),
            None => None,
          };
          if context.output.is_json() {
            let _ = context.output.print_json_line(&event.payload);
          } else {
            feed.push(render_event(&event, delivery.as_ref(), context), true);
          }
        }
        Item::Lost { retry_in } => {
          let note = format!("Lost connection. Retrying in {}s…", retry_in.as_secs());
          feed.push(vec![render_note(Mark::Warning, &note)], false);
        }
        Item::Reconnected => {
          feed.push(vec![render_note(Mark::Done, "Reconnected")], false);
        }
      }
    }
  };
  let result = run_until_stopped(work, args.timeout).await;
  feed.finish();
  result
}

fn print_header(forward: Option<&Forward>, secret: &str, context: &Context) {
  let workspace = context.get_workspace();
  if !context.output.is_rich() {
    match forward {
      Some(forward) => {
        context
          .output
          .success(&format!("Forwarding events to {}", forward.url));
        context.output.info(&format!("Secret {}", mask_secret(secret)));
      }
      None => context.output.success("Listening for events"),
    }
    return;
  }

  let source = workspace.unwrap_or_else(|| "events".to_owned());
  let title = match forward {
    Some(forward) => format!("Forwarding {source} to {}", forward.label),
    None => format!("Listening to {source}"),
  };
  let mut hint = Vec::new();
  if forward.is_some() {
    hint.push(Span::styled(format!("  Secret {}", mask_secret(secret)), SECONDARY));
    hint.push(Span::styled(" · ", FAINT));
    hint.push(Span::styled("ctrl+c", DIM));
  } else {
    hint.push(Span::styled("  ctrl+c", DIM));
  }
  hint.push(Span::styled(" stop", FAINT));

  anstream::eprintln!("{}", Line::from(Span::styled(format!("  {title}"), TITLE)));
  anstream::eprintln!("{}", Line::new(hint));
  anstream::eprintln!();
}

fn mask_secret(secret: &str) -> String {
  let tail: String = secret
    .chars()
    .rev()
    .take(4)
    .collect::<Vec<_>>()
    .into_iter()
    .rev()
    .collect();
  format!("{SECRET_PREFIX}••••••••{tail}")
}

fn render_note(mark: Mark, text: &str) -> Line<'static> {
  let clock = jiff::Zoned::now().strftime("%H:%M:%S").to_string();
  Line::new(vec![
    Span::styled(format!("  {clock}  "), DIM),
    mark.span(),
    Span::raw(format!(" {text}")),
  ])
}

fn render_event(
  event: &Event,
  delivery: Option<&Delivery>,
  context: &Context,
) -> Vec<Line<'static>> {
  if !context.output.is_rich() {
    let result = delivery
      .map(|delivery| format!("  {}", delivery.describe()))
      .unwrap_or_default();
    return vec![Line::from(Span::raw(format!(
      "{}  {:<22}  {}{result}",
      event.format_clock(),
      clean(event.get_type()),
      clean(event.get_subject_id())
    )))];
  }

  let kind = event.get_type().to_owned();
  let width = EVENT_WIDTH.max(kind.width() + 2);
  let mut line = Line::new(vec![
    Span::styled(format!("  {}  ", event.format_clock()), DIM),
    Mark::Status(get_event_style(&kind)).span(),
    Span::raw(" "),
    Span::raw(format!("{kind:<width$}")),
    Span::styled(format!("{}  ", event.get_subject_id()), SECONDARY),
  ]);
  let mut lines = Vec::new();
  if let Some(delivery) = delivery {
    line.spans.extend(delivery.spans());
    lines.push(line);
    if let Some(problem) = delivery.problem() {
      lines.push(Line::new(vec![
        Span::raw(" ".repeat(12)),
        Span::styled("└ ", DIM),
        Span::styled(problem, DANGER),
      ]));
    }
  } else {
    lines.push(line);
  }
  lines
}

enum Delivery {
  Answered { status: u16, millis: u128, problem: Option<String> },
  TimedOut,
  Unreachable,
  Failed,
}

impl Delivery {
  fn describe(&self) -> String {
    match self {
      Self::Answered { status, millis, .. } => format!("→ {status} {millis}ms"),
      Self::TimedOut => "→ Timed out".to_owned(),
      Self::Unreachable => "→ Could not connect".to_owned(),
      Self::Failed => "→ Failed".to_owned(),
    }
  }

  fn problem(&self) -> Option<String> {
    match self {
      Self::Answered { problem, .. } => problem.clone(),
      _ => None,
    }
  }

  fn spans(&self) -> Vec<Span<'static>> {
    match self {
      Self::Answered { status, millis, .. } => {
        let style = match status {
          200..=299 => DONE,
          400..=499 => WARNING,
          _ => DANGER,
        };
        vec![
          Span::styled(status.to_string(), style),
          Span::styled(format!("  {millis}ms"), DIM),
        ]
      }
      Self::TimedOut => vec![Span::styled("timed out", DANGER)],
      Self::Unreachable => vec![Span::styled("could not connect", DANGER)],
      Self::Failed => vec![Span::styled("failed", DANGER)],
    }
  }
}

async fn send_event(forward: &Forward, event: &Event) -> Delivery {
  let id = event.get_text("id");
  let timestamp = jiff::Timestamp::now().as_second().to_string();
  let signature = sign_payload(&forward.key, id, &timestamp, &event.body);
  let started = Instant::now();

  let response = forward
    .http
    .post(forward.url.clone())
    .header("webhook-id", id)
    .header("webhook-timestamp", &timestamp)
    .header("webhook-signature", format!("v1,{signature}"))
    .header(CONTENT_TYPE, "application/json")
    .body(event.body.clone())
    .send()
    .await;

  match response {
    Ok(response) => {
      let status = response.status();
      let millis = started.elapsed().as_millis();
      let problem = if status.is_success() {
        None
      } else {
        read_problem(&response.text().await.unwrap_or_default())
      };
      Delivery::Answered { status: status.as_u16(), millis, problem }
    }
    Err(error) if error.is_timeout() => Delivery::TimedOut,
    Err(error) if error.is_connect() => Delivery::Unreachable,
    Err(_) => Delivery::Failed,
  }
}

fn read_problem(body: &str) -> Option<String> {
  if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
    for path in ["error.message", "message", "error"] {
      if let Some(text) =
        crate::output::get_path(&value, path).and_then(|value| value.as_str())
      {
        return Some(clean(text).chars().take(PROBLEM_LENGTH).collect());
      }
    }
  }
  body
    .lines()
    .map(str::trim)
    .find(|line| !line.is_empty() && !line.starts_with('<'))
    .map(|line| clean(line).chars().take(PROBLEM_LENGTH).collect())
}

fn build_forward(url: &str, secret: &str) -> Result<Forward> {
  let invalid = || Error::usage("invalid_url", format!("The URL {url} is not valid."));
  let text = if url.contains("://") { url.to_owned() } else { format!("http://{url}") };
  let url = Url::parse(&text)
    .ok()
    .filter(|url| matches!(url.scheme(), "http" | "https"))
    .ok_or_else(invalid)?;
  let http = reqwest::Client::builder()
    .timeout(FORWARD_TIMEOUT)
    .user_agent(CLI_USER_AGENT)
    .tls_certs_merge(crate::api::get_fallback_roots())
    .build()?;
  let key = BASE64.decode(&secret[SECRET_PREFIX.len()..]).map_err(|_| {
    Error::failed("invalid_secret", "The saved signing secret is not valid.")
  })?;

  let label = url
    .as_str()
    .trim_start_matches(url.scheme())
    .trim_start_matches("://");
  let label = if url.scheme() == "https" { url.as_str() } else { label }.to_owned();
  Ok(Forward { http, url, label, key })
}

fn sign_payload(key: &[u8], id: &str, timestamp: &str, body: &str) -> String {
  let mut mac =
    Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any size");
  mac.update(format!("{id}.{timestamp}.{body}").as_bytes());
  BASE64.encode(mac.finalize().into_bytes())
}

fn read_secret() -> Result<String> {
  let path = get_config_dir()?.join(SECRET_FILE);
  match fs::read_to_string(&path) {
    Ok(saved) if saved.trim().starts_with(SECRET_PREFIX) => {
      return Ok(saved.trim().to_owned());
    }
    Ok(_) => {
      return Err(Error::failed(
        "invalid_secret",
        format!(
          "{} is not a signing secret. Delete it to make a new one.",
          path.display()
        ),
      ));
    }
    Err(error) if error.kind() == ErrorKind::NotFound => {}
    Err(error) => return Err(error.into()),
  }

  let mut bytes = [0_u8; 24];
  getrandom::fill(&mut bytes).map_err(|error| {
    Error::failed("random_error", format!("Could not create a secret: {error}"))
  })?;
  let secret = format!("{SECRET_PREFIX}{}", BASE64.encode(bytes));
  write_private_file(&path, secret.as_bytes())?;
  Ok(secret)
}

#[cfg(test)]
mod tests {
  use super::sign_payload;

  #[test]
  fn signs_the_standard_webhooks_content() {
    let signature = sign_payload(b"key", "evt_1", "1700000000", "{}");
    assert_eq!(signature, "qlxS+Rx59BdwbM0DVnBBVM2EZr7V1DuM8yaNlT+iZiY=");
  }
}
