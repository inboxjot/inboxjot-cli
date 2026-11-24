use std::time::Duration;

use clap::Args;
use inboxjot::{EmailSource, ListEmails, UpdateEmail};
use serde_json::Value;

use super::{EmailArgs, check_delivery, format_schedule, timeline, wait_for_email};
use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::Result;
use crate::output::{Column, column, status_column};
use crate::time::parse_past;
use crate::time::{parse_duration, parse_schedule};
use crate::ui::style::TITLE;
use crate::ui::text::{Line, Span};

const LIST_COLUMNS: [Column; 5] = [
  column("ID", "id"),
  column("To", "to"),
  column("Subject", "subject"),
  status_column("Status", "status"),
  column("Created", "created_at"),
];

const TERMINAL_COLUMNS: [Column; 5] = [
  column("ID", "id"),
  column("To", "to"),
  column("Subject", "subject"),
  status_column("Status", "status"),
  column("Sent", "listed_at"),
];

const DETAIL_ROWS: [Column; 11] = [
  column("ID", "id"),
  column("From", "from"),
  column("To", "to"),
  column("Cc", "cc"),
  column("Bcc", "bcc"),
  column("Subject", "subject"),
  column("Status", "status"),
  column("Source", "source"),
  column("Scheduled", "scheduled_at"),
  column("Sent", "sent_at"),
  column("Created", "created_at"),
];

#[derive(Debug, Args)]
pub struct ListEmailsArgs {
  #[command(flatten)]
  pub page: ListArgs,

  #[arg(long, help = "Show only emails with this status, such as bounced")]
  pub status: Option<String>,

  #[arg(long, value_name = "TEXT", help = "Search recipients and subjects")]
  pub query: Option<String>,

  #[arg(
    long,
    value_parser = parse_source,
    help = "Show only emails from api, smtp, dashboard, mcp, campaign or automation"
  )]
  pub source: Option<EmailSource>,

  #[arg(
    long,
    value_name = "TIME",
    value_parser = parse_past,
    help = "Show emails sent after this time, such as 7d or 2026-11-01T00:00:00Z"
  )]
  pub since: Option<String>,

  #[arg(
    long,
    value_name = "TIME",
    value_parser = parse_past,
    help = "Show emails sent before this time, such as 1h or 2026-11-08T00:00:00Z"
  )]
  pub until: Option<String>,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
  #[arg(value_name = "ID", help = "Email ID, such as eml_123")]
  pub id: String,

  #[arg(long, help = "Check every 2 seconds until the email is delivered or fails")]
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
pub struct RescheduleArgs {
  #[arg(value_name = "ID", help = "Email ID, such as eml_123")]
  pub id: String,

  #[arg(
    long,
    value_name = "TIME",
    value_parser = parse_schedule,
    help = "New send time, such as 2026-11-20T09:00:00Z or \"in 10m\""
  )]
  pub at: String,
}

pub async fn list(args: &ListEmailsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading emails…");
  let query = ListEmails {
    query: args.query.clone(),
    status: args.status.clone(),
    source: args.source,
    sent_after: args.since.clone(),
    sent_before: args.until.clone(),
    ..ListEmails::default()
  };
  let mut page = api.list(&["v1", "emails"], &query, &args.page.to_page()).await?;
  if context.output.is_json() || !context.output.is_stdout_tty() {
    return context.output.print_list(&page, &LIST_COLUMNS, "emails", None);
  }

  add_listed_times(&mut page);
  context.output.print_list(&page, &TERMINAL_COLUMNS, "emails", None)
}

pub async fn inspect(args: &InspectArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the email…");
  let email = if args.wait {
    wait_for_email(&api, &args.id, args.timeout, context).await?
  } else {
    api.get(&["v1", "emails", &args.id], &()).await?
  };

  if context.output.is_stdout_tty() && !context.output.is_json() {
    print_inspection(&email, context).await;
  } else {
    context.output.print_detail(&email, &DETAIL_ROWS)?;
  }
  if args.wait {
    return check_delivery(&email);
  }

  Ok(())
}

pub async fn reschedule(args: RescheduleArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Rescheduling the email…");
  let body = UpdateEmail { scheduled_at: args.at };
  let email = api.patch(&["v1", "emails", &args.id], &body).await?;

  let time = email
    .get("scheduled_at")
    .and_then(Value::as_str)
    .unwrap_or(&body.scheduled_at);
  let headline = format!("Rescheduled {} for {}", args.id, format_schedule(time));
  context.output.print_result(&email, &headline, "id", &[])
}

pub async fn cancel(args: &EmailArgs, context: &Context) -> Result<()> {
  context.confirm(&format!("Cancel the scheduled email {}?", args.id))?;

  let api = context.api().await?.loading("Canceling the email…");
  let email = api.post(&["v1", "emails", &args.id, "cancel"], &()).await?;
  context
    .output
    .print_result(&email, &format!("Canceled {}", args.id), "id", &[])
}

fn add_listed_times(page: &mut Value) {
  let Some(rows) = page.get_mut("data").and_then(Value::as_array_mut) else {
    return;
  };
  for row in rows {
    let time = ["sent_at", "scheduled_at", "created_at"]
      .into_iter()
      .find_map(|path| row.get(path).filter(|value| value.is_string()).cloned());
    if let (Some(time), Some(row)) = (time, row.as_object_mut()) {
      row.insert("listed_at".to_owned(), time);
    }
  }
}

fn parse_source(text: &str) -> Result<EmailSource, String> {
  match serde_json::from_value(Value::from(text)) {
    Ok(EmailSource::Unknown) | Err(_) => Err(format!(
      "\"{text}\" is not a source. Use api, smtp, dashboard, mcp, campaign or automation"
    )),
    Ok(source) => Ok(source),
  }
}

async fn print_inspection(email: &Value, context: &Context) {
  let id = email.get("id").and_then(Value::as_str).unwrap_or_default();
  let events = timeline::fetch_events(context, id).await;
  for line in timeline::render_summary(email) {
    anstream::println!("{line}");
  }

  let lines = timeline::render_events(email, &events, timeline::Detail::Notes);
  if !lines.is_empty() {
    anstream::println!();
    anstream::println!("{}", Line::from(Span::styled("  Events", TITLE)));
    for line in lines {
      anstream::println!("{line}");
    }
  }
  context
    .output
    .print_next(&[("open it in the dashboard", &format!("inboxjot open emails/{id}"))]);
}
