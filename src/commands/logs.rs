use std::collections::HashSet;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand, ValueEnum};
use jiff::Timestamp;
use serde::Serialize;
use serde_json::Value;

use jiff::tz::TimeZone;

use crate::api::Api;
use crate::cli::ListArgs;
use crate::commands::stream::run_until_stopped;
use crate::context::Context;
use crate::error::Result;
use crate::output::{Column, column, format_value, get_path};
use crate::time::parse_duration;
use crate::ui::feed::Feed;
use crate::ui::style::{DANGER, DIM, DONE, FAINT, SECONDARY, TITLE, WARNING};
use crate::ui::text::{Line, Span};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const TAIL_START_ROWS: u32 = 10;
const TAIL_PAGE_ROWS: u32 = 100;
const PATH_WIDTH: usize = 28;

const LIST_COLUMNS: [Column; 6] = [
  column("Request ID", "request_id"),
  column("Method", "method"),
  column("Path", "path"),
  column("Status", "status"),
  column("API key", "api_key_name"),
  column("Time", "occurred_at"),
];

const DETAIL_ROWS: [Column; 11] = [
  column("Request ID", "request_id"),
  column("Method", "method"),
  column("Path", "path"),
  column("Status", "status"),
  column("Summary", "summary"),
  column("Duration (ms)", "duration_ms"),
  column("API key", "api_key_name"),
  column("API key ID", "api_key_id"),
  column("Resource", "resource_id"),
  column("IP address", "ip_address"),
  column("Time", "occurred_at"),
];

#[derive(Debug, Args)]
pub struct LogsArgs {
  #[command(subcommand)]
  pub command: Option<LogsCommand>,

  #[arg(short, long, help = "Print new API requests as they arrive")]
  pub follow: bool,

  #[command(flatten)]
  pub filters: FilterArgs,

  #[arg(long, value_parser = parse_duration, requires = "follow", help = "Stop following after this long, such as 10m")]
  pub timeout: Option<Duration>,
}

#[derive(Debug, Subcommand)]
pub enum LogsCommand {
  #[command(
    about = "List API requests",
    after_help = examples!("inboxjot logs list --outcome failed"),
    alias = "ls",
  )]
  List(ListLogsArgs),
  #[command(
    about = "Show an API request with its bodies",
    after_help = examples!("inboxjot logs inspect 5f0c6a2e-8d1b-4c7a-9e3f-2b6d4a1c8e90"),
  )]
  Inspect(InspectArgs),
  #[command(
    about = "Print new API requests as they arrive",
    after_help = examples!(
      "inboxjot logs tail",
      "inboxjot logs tail --outcome failed",
    ),
  )]
  Tail(TailArgs),
}

#[derive(Debug, Args)]
pub struct FilterArgs {
  #[arg(long, value_enum, help = "Show only requests with this outcome")]
  pub outcome: Option<Outcome>,

  #[arg(long, value_name = "KEY_ID", help = "Show only requests made with this API key")]
  pub api_key: Option<String>,

  #[arg(
    long,
    help = "Show requests whose method and path contain this text, or with this request ID"
  )]
  pub query: Option<String>,
}

#[derive(Debug, Args)]
pub struct ListLogsArgs {
  #[command(flatten)]
  pub filters: FilterArgs,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
  #[arg(value_name = "REQUEST_ID", help = "Request ID")]
  pub id: String,
}

#[derive(Debug, Args)]
pub struct TailArgs {
  #[command(flatten)]
  pub filters: FilterArgs,

  #[arg(long, value_parser = parse_duration, help = "Stop after this long, such as 10m")]
  pub timeout: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
  Succeeded,
  Failed,
}

#[derive(Debug, Serialize)]
struct LogQuery<'a> {
  outcome: Option<Outcome>,
  api_key_id: Option<&'a str>,
  query: Option<&'a str>,
}

impl<'a> LogQuery<'a> {
  fn new(filters: &'a FilterArgs) -> Self {
    Self {
      outcome: filters.outcome,
      api_key_id: filters.api_key.as_deref(),
      query: filters.query.as_deref(),
    }
  }
}

pub async fn run(args: LogsArgs, context: &Context) -> Result<()> {
  match args.command {
    Some(LogsCommand::List(args)) => list(&args, context).await,
    Some(LogsCommand::Inspect(args)) => inspect(&args, context).await,
    Some(LogsCommand::Tail(args)) => tail(&args.filters, args.timeout, context).await,
    None if args.follow => tail(&args.filters, args.timeout, context).await,
    None => {
      let page = ListArgs { limit: 50, all: false, after: None, before: None };
      let list_args = ListLogsArgs { filters: args.filters, list: page };
      list(&list_args, context).await
    }
  }
}

async fn list(args: &ListLogsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading logs…");
  let query = LogQuery::new(&args.filters);
  let page = api.list(&["v1", "logs"], &query, &args.list.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "requests", None)
}

async fn inspect(args: &InspectArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the request…");
  let request = api.get(&["v1", "logs", &args.id], &()).await?;

  context.output.print_detail(&request, &DETAIL_ROWS)?;
  if context.output.is_json() {
    return Ok(());
  }

  for (label, key) in
    [("Request body", "request_body"), ("Response body", "response_body")]
  {
    let body = request.get(key).and_then(Value::as_str).unwrap_or_default();
    if body.is_empty() {
      continue;
    }

    context.output.print_line("");
    context.output.print_line(label);
    context.output.print_line(&format_body(body));
  }

  Ok(())
}

async fn tail(
  filters: &FilterArgs,
  timeout: Option<Duration>,
  context: &Context,
) -> Result<()> {
  let api = context.api().await?;
  let query = LogQuery::new(filters);
  let started = Instant::now();
  let mut newest: Option<Timestamp> = None;
  let mut seen = HashSet::new();
  let mut per_page = TAIL_START_ROWS;

  if context.output.is_rich() {
    let source = context.get_workspace().unwrap_or_else(|| "this workspace".to_owned());
    anstream::eprintln!(
      "{}",
      Line::from(Span::styled(format!("  Following API requests in {source}"), TITLE))
    );
    anstream::eprintln!(
      "{}",
      Line::new(vec![Span::styled("  ctrl+c", DIM), Span::styled(" stop", FAINT)])
    );
    anstream::eprintln!();
  }

  let mut feed = Feed::new(context.output.is_rich(), false, "Waiting for requests");
  let work = async {
    loop {
      let rows = fetch_latest(&api, &query, per_page).await?;
      for row in rows.iter().rev().filter(|row| !is_own_poll(row)) {
        if take_new(row, &mut newest, &mut seen) {
          if context.output.is_rich() {
            feed.push(render_row(row), true);
          } else {
            print_row(row, context)?;
          }
        }
      }
      per_page = TAIL_PAGE_ROWS;
      if timeout.is_some_and(|timeout| started.elapsed() + POLL_INTERVAL > timeout) {
        return Ok(());
      }
      tokio::time::sleep(POLL_INTERVAL).await;
    }
  };
  let result = run_until_stopped(work, timeout).await;
  feed.finish();
  result
}

fn is_own_poll(row: &Value) -> bool {
  let text = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or_default();
  text("method") == "GET" && text("path").split('?').next() == Some("/v1/logs")
}

fn render_row(row: &Value) -> Vec<Line<'static>> {
  let text = |path: &str| get_path(row, path).and_then(Value::as_str).unwrap_or_default();
  let status = row.get("status").and_then(Value::as_u64).unwrap_or(0);
  let style = match status {
    200..=299 => DONE,
    400..=499 => WARNING,
    _ => DANGER,
  };
  let clock = text("occurred_at")
    .parse::<Timestamp>()
    .map(|time| time.to_zoned(TimeZone::system()).strftime("%H:%M:%S").to_string())
    .unwrap_or_default();
  let path = text("path");
  let width = PATH_WIDTH.max(path.len() + 2);
  let mut spans = vec![
    Span::styled(format!("  {clock}  "), DIM),
    Span::raw(format!("{:<7}{path:<width$}", text("method"))),
    Span::styled(status.to_string(), style),
  ];
  if let Some(millis) = row.get("duration_ms").and_then(Value::as_u64) {
    spans.push(Span::styled(format!("{:>7}", format!("{millis}ms")), DIM));
  }
  spans.push(Span::styled(format!("  {}", text("api_key_name")), SECONDARY));
  let mut lines = vec![Line::new(spans)];

  if status >= 400 {
    let name = text("error_name");
    let message = if text("error_message").is_empty() {
      text("summary")
    } else {
      text("error_message")
    };
    let mut problem = vec![Span::raw(" ".repeat(12)), Span::styled("└ ", DIM)];
    if !name.is_empty() {
      problem.push(Span::styled(name.to_owned(), style));
      if !message.is_empty() {
        problem.push(Span::styled(format!("  {message}"), SECONDARY));
      }
    } else if !message.is_empty() {
      problem.push(Span::styled(message.to_owned(), style));
    }
    if problem.len() > 2 {
      lines.push(Line::new(problem));
    }
  }
  lines
}

async fn fetch_latest(
  api: &Api,
  query: &LogQuery<'_>,
  per_page: u32,
) -> Result<Vec<Value>> {
  let mut pairs = crate::api::to_query_pairs(query)?;
  pairs.push(("per_page".to_owned(), per_page.to_string()));
  let url = ["v1", "logs"];
  let mut page = api.send(reqwest::Method::GET, &url, &pairs, None).await?;

  match page.get_mut("data").map(Value::take) {
    Some(Value::Array(rows)) => Ok(rows),
    _ => Ok(Vec::new()),
  }
}

fn take_new(
  row: &Value,
  newest: &mut Option<Timestamp>,
  seen: &mut HashSet<String>,
) -> bool {
  let id = row.get("request_id").and_then(Value::as_str).unwrap_or_default();
  let Some(time) = row
    .get("occurred_at")
    .and_then(Value::as_str)
    .and_then(|text| text.parse().ok())
  else {
    return false;
  };

  match *newest {
    Some(current) if time < current => false,
    Some(current) if time == current => seen.insert(id.to_owned()),
    _ => {
      *newest = Some(time);
      seen.clear();
      seen.insert(id.to_owned());
      true
    }
  }
}

fn print_row(row: &Value, context: &Context) -> Result<()> {
  if context.output.is_json() {
    context.output.print_json_line(row)?;
    return Ok(());
  }

  let field = |path: &str| format_value(get_path(row, path), path, false, false);
  context.output.print_line(&format!(
    "{}  {} {:<4} {}  {}",
    field("occurred_at"),
    field("status"),
    field("method"),
    field("path"),
    field("request_id"),
  ));
  Ok(())
}

fn format_body(body: &str) -> String {
  serde_json::from_str::<Value>(body)
    .ok()
    .and_then(|value| serde_json::to_string_pretty(&value).ok())
    .unwrap_or_else(|| body.to_owned())
}

#[cfg(test)]
mod tests {
  use std::collections::HashSet;

  use serde_json::json;

  use super::take_new;

  #[test]
  fn takes_each_row_once_and_skips_older_rows() {
    let mut newest = None;
    let mut seen = HashSet::new();
    let first = json!({ "request_id": "req_1", "occurred_at": "2026-11-09T10:00:00Z" });
    let second = json!({ "request_id": "req_2", "occurred_at": "2026-11-09T10:00:00Z" });
    let older = json!({ "request_id": "req_0", "occurred_at": "2026-11-09T09:00:00Z" });

    assert!(take_new(&first, &mut newest, &mut seen));
    assert!(take_new(&second, &mut newest, &mut seen));
    assert!(!take_new(&first, &mut newest, &mut seen));
    assert!(!take_new(&older, &mut newest, &mut seen));
  }
}
