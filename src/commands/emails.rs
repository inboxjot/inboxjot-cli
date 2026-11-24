mod attachments;
mod batch;
mod list;
mod tail;
mod timeline;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::{Args, Subcommand};
use inboxjot::{AttachmentInput, SendEmail, Tag, TemplateRef};
use serde_json::{Map, Value};

use crate::api::Api;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Spinner, format_time};
use crate::time::{parse_duration, parse_schedule};
use timeline::Detail;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const FIRST_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SETTLED_STATUSES: [&str; 8] = [
  "delivered",
  "opened",
  "clicked",
  "complained",
  "canceled",
  "bounced",
  "rejected",
  "failed",
];
const FAILED_STATUSES: [&str; 3] = ["bounced", "rejected", "failed"];

#[derive(Debug, Subcommand)]
pub enum EmailsCommand {
  #[command(
    about = "Send an email",
    after_help = examples!(
      "inboxjot send --from \"Acme <hi@acme.com>\" --to riti@example.com --subject \"Welcome\" --text \"Thanks for signing up\"",
      "inboxjot send --from hi@acme.com --to riti@example.com --template tpl_06d2mfk8r5cx3nd8vt1qa --data '{\"name\":\"Riti\"}'",
      "inboxjot send --from hi@acme.com --to bounced@inboxjot.dev --subject Test --text Hi --wait",
    ),
  )]
  Send(Box<SendArgs>),
  #[command(
    about = "Send many emails from a JSON or JSONL file",
    after_help = examples!("inboxjot emails batch emails.jsonl"),
  )]
  Batch(batch::BatchArgs),
  #[command(
    about = "List emails",
    after_help = examples!(
      "inboxjot emails list --status bounced --since 7d",
      "inboxjot emails list --query riti@example.com --json",
    ),
    alias = "ls",
  )]
  List(list::ListEmailsArgs),
  #[command(
    about = "Show an email and its status",
    after_help = examples!(
      "inboxjot emails inspect eml_123",
      "inboxjot emails inspect eml_123 --wait",
    ),
  )]
  Inspect(list::InspectArgs),
  #[command(
    about = "Change when a scheduled email goes out",
    after_help = examples!("inboxjot emails reschedule eml_123 --at \"in 2h\""),
  )]
  Reschedule(list::RescheduleArgs),
  #[command(
    about = "Cancel a scheduled email",
    after_help = examples!("inboxjot emails cancel eml_123"),
  )]
  Cancel(EmailArgs),
  #[command(subcommand, about = "List and download the attachments of an email")]
  Attachments(attachments::AttachmentsCommand),
  #[command(
    about = "Show email events as they happen",
    after_help = examples!(
      "inboxjot emails tail",
      "inboxjot emails tail --events email.bounced,email.complained",
    ),
  )]
  Tail(tail::TailArgs),
}

#[derive(Debug, Args)]
pub struct SendArgs {
  #[arg(long, value_name = "ADDRESS", help = "Sender, on a verified domain")]
  pub from: Option<String>,

  #[arg(
    long,
    value_delimiter = ',',
    required_unless_present = "input",
    value_name = "ADDRESS",
    help = "Recipient. Repeat it for more"
  )]
  pub to: Vec<String>,

  #[arg(
    long,
    value_delimiter = ',',
    value_name = "ADDRESS",
    help = "Carbon copy recipients"
  )]
  pub cc: Vec<String>,

  #[arg(
    long,
    value_delimiter = ',',
    value_name = "ADDRESS",
    help = "Blind carbon copy recipients"
  )]
  pub bcc: Vec<String>,

  #[arg(
    long,
    value_delimiter = ',',
    value_name = "ADDRESS",
    help = "Addresses that replies go to"
  )]
  pub reply_to: Vec<String>,

  #[arg(long, value_name = "TEXT", help = "Subject line")]
  pub subject: Option<String>,

  #[arg(long, conflicts_with = "html_file", value_name = "HTML", help = "HTML body")]
  pub html: Option<String>,

  #[arg(long, value_name = "PATH", help = "Read the HTML body from a file")]
  pub html_file: Option<PathBuf>,

  #[arg(long, conflicts_with = "text_file", help = "Plain text body")]
  pub text: Option<String>,

  #[arg(long, value_name = "PATH", help = "Read the plain text body from a file")]
  pub text_file: Option<PathBuf>,

  #[arg(
    long,
    value_name = "ID",
    conflicts_with_all = ["html", "html_file", "text", "text_file"],
    help = "Send a template instead of a body"
  )]
  pub template: Option<String>,

  #[arg(
    long,
    value_name = "JSON",
    value_parser = parse_data,
    requires = "template",
    help = "Template variables"
  )]
  pub data: Option<Map<String, Value>>,

  #[arg(long, value_name = "PATH", help = "Attach a file; repeat for more files")]
  pub attach: Vec<PathBuf>,

  #[arg(
    long = "tag",
    value_name = "NAME=VALUE",
    value_parser = parse_tag,
    help = "Add a tag; repeat for more tags"
  )]
  pub tags: Vec<Tag>,

  #[arg(
    long = "header",
    value_name = "'NAME: VALUE'",
    value_parser = parse_header,
    help = "Add an email header; repeat for more headers"
  )]
  pub headers: Vec<(String, String)>,

  #[arg(
    long,
    value_name = "TIME",
    value_parser = parse_schedule,
    help = "Send later, such as 2026-11-20T09:00:00Z or \"in 10m\""
  )]
  pub scheduled_at: Option<String>,

  #[arg(long, value_name = "BOOL", help = "Turn open tracking on or off for this email")]
  pub open_tracking: Option<bool>,

  #[arg(
    long,
    value_name = "BOOL",
    help = "Turn click tracking on or off for this email"
  )]
  pub click_tracking: Option<bool>,

  #[arg(
    long,
    value_name = "PATH",
    conflicts_with_all = [
      "from", "to", "cc", "bcc", "reply_to", "subject", "html", "html_file", "text",
      "text_file", "template", "data", "attach", "tags", "headers", "scheduled_at",
      "open_tracking", "click_tracking",
    ],
    help = "Read the whole JSON body from a file, or - for stdin"
  )]
  pub input: Option<String>,

  #[arg(long, conflicts_with = "wait", help = "Print the JSON body and send nothing")]
  pub dry_run: bool,

  #[arg(
    long,
    conflicts_with = "scheduled_at",
    help = "Wait until it is delivered or bounced"
  )]
  pub wait: bool,

  #[arg(
    long,
    default_value = "2m",
    value_parser = parse_duration,
    requires = "wait",
    help = "Stop waiting after this long"
  )]
  pub timeout: Duration,
}

#[derive(Debug, Args)]
pub struct EmailArgs {
  #[arg(value_name = "ID", help = "Email ID, such as eml_123")]
  pub id: String,
}

pub async fn run(command: EmailsCommand, context: &Context) -> Result<()> {
  match command {
    EmailsCommand::Send(args) => send(*args, context).await,
    EmailsCommand::Batch(args) => batch::send_batch(&args, context).await,
    EmailsCommand::List(args) => list::list(&args, context).await,
    EmailsCommand::Inspect(args) => list::inspect(&args, context).await,
    EmailsCommand::Reschedule(args) => list::reschedule(args, context).await,
    EmailsCommand::Cancel(args) => list::cancel(&args, context).await,
    EmailsCommand::Attachments(command) => attachments::run(command, context).await,
    EmailsCommand::Tail(args) => tail::tail(args, context).await,
  }
}

pub async fn send(args: SendArgs, context: &Context) -> Result<()> {
  let body = build_send_body(&args)?;
  if args.dry_run {
    return context.output.print_json(&body);
  }

  let api = context.api().await?.loading("Sending the email…");
  let queued = api.post(&["v1", "emails"], &body).await?;
  let id = get_id(&queued).to_owned();

  if args.wait {
    let email = wait_for_email(&api, &id, args.timeout, context).await?;
    if context.output.is_rich() {
      return print_delivery(&email, context).await;
    }
    if context.output.is_json() {
      context.output.print_json(&email)?;
    } else if !is_failed(&email) {
      let headline = format!("{} {id}", format_status(get_status(&email)));
      context.output.print_result(&email, &headline, "id", &[])?;
    }
    return check_delivery(&email);
  }

  let headline = match body.get("scheduled_at").and_then(Value::as_str) {
    Some(time) => format!("Scheduled {id} for {}", format_schedule(time)),
    None => format!("Sent {id}"),
  };
  context.output.print_result(&queued, &headline, "id", &[])?;
  context
    .output
    .next("Follow it with:", &format!("inboxjot emails inspect {id} --wait"));
  Ok(())
}

async fn print_delivery(email: &Value, context: &Context) -> Result<()> {
  let id = get_id(email);
  let events = timeline::fetch_events(context, id).await;
  for line in timeline::render_sent(email) {
    anstream::eprintln!("{line}");
  }
  let lines = timeline::render_events(email, &events, Detail::Offsets);
  if !lines.is_empty() {
    anstream::eprintln!();
    for line in lines {
      anstream::eprintln!("{line}");
    }
  }
  anstream::eprintln!();

  if is_failed(email) {
    return Err(get_delivery_error(email, &events));
  }

  let to = timeline::get_first_recipient(email);
  let took = timeline::get_seconds_to_settle(email, &events)
    .map_or_else(String::new, |seconds| {
      format!(" in {}", timeline::format_duration(seconds))
    });
  if get_status(email) == "complained" {
    context
      .output
      .warn(&format!("Delivered to {to}, then marked as spam{took}"));
  } else {
    context
      .output
      .success(&format!("{} to {to}{took}", format_status(get_status(email))));
  }
  context
    .output
    .print_next(&[("every event", &format!("inboxjot emails inspect {id}"))]);
  Ok(())
}

fn build_send_body(args: &SendArgs) -> Result<Value> {
  if let Some(input) = &args.input {
    let body: Value = serde_json::from_str(&read_input(input)?)?;
    if !body.is_object() {
      return Err(Error::usage("invalid_input", "The input must be a JSON object."));
    }
    return Ok(body);
  }

  let html = args.html.clone().or(read_optional_text(args.html_file.as_deref())?);
  let text = args.text.clone().or(read_optional_text(args.text_file.as_deref())?);
  if html.is_none() && text.is_none() && args.template.is_none() {
    return Err(Error::usage("missing_content", "Add --html, --text or --template."));
  }

  let template = args.template.clone().map(|id| TemplateRef {
    id,
    variables: args.data.clone().unwrap_or_default(),
  });
  let email = SendEmail {
    from: args.from.clone(),
    to: args.to.clone(),
    cc: args.cc.clone(),
    bcc: args.bcc.clone(),
    reply_to: args.reply_to.clone(),
    subject: args.subject.clone(),
    template,
    html,
    text,
    headers: args.headers.iter().cloned().collect(),
    tags: args.tags.clone(),
    scheduled_at: args.scheduled_at.clone(),
    attachments: args
      .attach
      .iter()
      .map(|path| read_attachment(path))
      .collect::<Result<_>>()?,
    open_tracking: args.open_tracking,
    click_tracking: args.click_tracking,
  };

  Ok(serde_json::to_value(email)?)
}

async fn wait_for_email(
  api: &Api,
  id: &str,
  timeout: Duration,
  context: &Context,
) -> Result<Value> {
  let spinner = context.output.start_spinner("Waiting for delivery…", &[]);
  let result = poll_email(api, id, timeout, &spinner).await;
  spinner.finish();
  result
}

async fn poll_email(
  api: &Api,
  id: &str,
  timeout: Duration,
  spinner: &Spinner,
) -> Result<Value> {
  let started = Instant::now();
  let mut interval = FIRST_POLL_INTERVAL;

  loop {
    let email = api.get(&["v1", "emails", id], &()).await?;
    let status = get_status(&email);
    if SETTLED_STATUSES.contains(&status) {
      return Ok(email);
    }

    if started.elapsed() >= timeout {
      return Err(
        Error::failed("timeout", format!("{id} is still {status}.")).with_next(
          format!("inboxjot emails inspect {id} --wait"),
          "Keep waiting for the email",
        ),
      );
    }

    spinner.set_message(format!("Waiting for delivery… {status}"));
    tokio::time::sleep(interval).await;
    interval = (interval * 2).min(POLL_INTERVAL);
  }
}

fn check_delivery(email: &Value) -> Result<()> {
  if is_failed(email) {
    return Err(get_delivery_error(email, &[]));
  }
  Ok(())
}

fn get_delivery_error(email: &Value, events: &[Value]) -> Error {
  let to = timeline::get_first_recipient(email);
  Error::failed("delivery_failed", format!("Couldn't deliver to {to}"))
    .with_reason(timeline::describe_failure(email, events))
    .with_next(format!("inboxjot emails inspect {}", get_id(email)), "Every event")
}

fn is_failed(email: &Value) -> bool {
  FAILED_STATUSES.contains(&get_status(email))
}

fn get_id(value: &Value) -> &str {
  value.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn get_status(email: &Value) -> &str {
  email.get("status").and_then(Value::as_str).unwrap_or("unknown")
}

fn format_status(status: &str) -> String {
  let mut chars = status.chars();
  chars
    .next()
    .map(|first| first.to_uppercase().chain(chars).collect())
    .unwrap_or_default()
}

fn format_schedule(time: &str) -> String {
  format_time(time, true, true).unwrap_or_else(|| time.to_owned())
}

fn read_input(input: &str) -> Result<String> {
  if input == "-" {
    return Ok(std::io::read_to_string(std::io::stdin())?);
  }

  std::fs::read_to_string(input).map_err(|error| get_read_error(Path::new(input), &error))
}

fn read_optional_text(path: Option<&Path>) -> Result<Option<String>> {
  path
    .map(|path| {
      std::fs::read_to_string(path).map_err(|error| get_read_error(path, &error))
    })
    .transpose()
}

fn read_attachment(path: &Path) -> Result<AttachmentInput> {
  let bytes = std::fs::read(path).map_err(|error| get_read_error(path, &error))?;
  let filename = path.file_name().map(|name| name.to_string_lossy().into_owned());

  Ok(AttachmentInput {
    filename: filename.unwrap_or_default(),
    content: Some(BASE64.encode(bytes)),
    ..AttachmentInput::default()
  })
}

fn get_read_error(path: &Path, error: &std::io::Error) -> Error {
  Error::usage("unreadable_file", format!("Cannot read {}: {error}.", path.display()))
}

pub fn parse_data(text: &str) -> Result<Map<String, Value>, String> {
  serde_json::from_str(text)
    .map_err(|_| "Use a JSON object, such as '{\"plan\":\"pro\"}'".to_owned())
}

fn parse_tag(text: &str) -> Result<Tag, String> {
  let (name, value) = text
    .split_once('=')
    .ok_or_else(|| format!("\"{text}\" is not a tag. Use NAME=VALUE"))?;

  Ok(Tag {
    name: name.trim().to_owned(),
    value: value.trim().to_owned(),
  })
}

fn parse_header(text: &str) -> Result<(String, String), String> {
  let (name, value) = text
    .split_once(':')
    .ok_or_else(|| format!("\"{text}\" is not a header. Use 'NAME: VALUE'"))?;

  Ok((name.trim().to_owned(), value.trim().to_owned()))
}

#[cfg(test)]
mod tests {
  use super::{parse_header, parse_tag};

  #[test]
  fn parses_tags_and_headers() {
    assert_eq!(parse_tag("plan=pro").unwrap().value, "pro");
    assert!(parse_tag("plan").is_err());
    assert_eq!(
      parse_header("X-Entity-Ref: 42").unwrap(),
      ("X-Entity-Ref".to_owned(), "42".to_owned())
    );
  }
}
