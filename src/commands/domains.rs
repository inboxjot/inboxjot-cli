use std::time::{Duration, Instant};

use clap::{Args, Subcommand, ValueEnum};
use inboxjot::CreateDomain;
use serde_json::{Value, json};

use unicode_width::UnicodeWidthStr;

use crate::api::{Api, Page};
use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, clean, column};
use crate::time::parse_duration;
use crate::ui::mark::Mark;
use crate::ui::spinner::get_name_or;
use crate::ui::style::{ACTION, DANGER, DIM, DONE, SECONDARY, TITLE, WARNING, get_width};
use crate::ui::text::{Line, Span};

const POLL_INTERVAL: Duration = Duration::from_secs(10);

const LIST_COLUMNS: [Column; 5] = [
  column("Name", "name"),
  column("Status", "status"),
  column("Region", "region"),
  column("Sent (30d)", "sent_last_30_days"),
  column("Created", "created_at"),
];

const DETAIL_ROWS: [Column; 8] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Status", "status"),
  column("Region", "region"),
  column("Return path", "mail_from_domain"),
  column("Sent (30d)", "sent_last_30_days"),
  column("Verified", "verified_at"),
  column("Created", "created_at"),
];

const RECORD_COLUMNS: [Column; 5] = [
  column("Type", "type"),
  column("Name", "name"),
  column("Value", "value"),
  column("Priority", "priority"),
  column("Status", "status"),
];

#[derive(Debug, Subcommand)]
pub enum DomainsCommand {
  #[command(
    about = "Add a domain and print its DNS records",
    after_help = examples!(
      "inboxjot domains add acme.com",
      "inboxjot domains add acme.com --region eu-west-1 --return-path send",
    ),
  )]
  Add(AddArgs),
  #[command(
    about = "List domains",
    after_help = examples!("inboxjot domains list"),
    alias = "ls",
  )]
  List(ListArgs),
  #[command(
    about = "Show a domain and its DNS records",
    after_help = examples!("inboxjot domains inspect acme.com"),
  )]
  Inspect(DomainArgs),
  #[command(
    about = "Print the DNS records of a domain",
    after_help = examples!(
      "inboxjot domains records acme.com",
      "inboxjot domains records acme.com --format bind > acme.zone",
    ),
  )]
  Records(RecordsArgs),
  #[command(
    about = "Check the DNS records of a domain",
    after_help = examples!("inboxjot domains verify acme.com --wait"),
  )]
  Verify(VerifyArgs),
  #[command(
    about = "Delete a domain",
    after_help = examples!("inboxjot domains delete acme.com"),
    alias = "rm",
  )]
  Delete(DomainArgs),
}

#[derive(Debug, Args)]
pub struct AddArgs {
  #[arg(value_name = "DOMAIN", help = "Domain name, such as acme.com")]
  pub name: String,

  #[arg(long, help = "AWS region that sends the email, such as us-east-1")]
  pub region: Option<String>,

  #[arg(long, value_name = "SUBDOMAIN", help = "Subdomain for bounces, such as send")]
  pub return_path: Option<String>,
}

#[derive(Debug, Args)]
pub struct DomainArgs {
  #[arg(value_name = "DOMAIN", help = "Domain name or ID, such as acme.com")]
  pub domain: String,
}

#[derive(Debug, Args)]
pub struct RecordsArgs {
  #[arg(value_name = "DOMAIN", help = "Domain name or ID, such as acme.com")]
  pub domain: String,

  #[arg(long, value_enum, default_value_t = RecordFormat::Table, help = "Output format")]
  pub format: RecordFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum RecordFormat {
  Table,
  Bind,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
  #[arg(value_name = "DOMAIN", help = "Domain name or ID, such as acme.com")]
  pub domain: String,

  #[arg(long, help = "Check again every 10 seconds until the domain is verified")]
  pub wait: bool,

  #[arg(
    long,
    default_value = "30m",
    value_parser = parse_duration,
    requires = "wait",
    help = "Stop waiting after this long"
  )]
  pub timeout: Duration,
}

pub async fn run(command: DomainsCommand, context: &Context) -> Result<()> {
  match command {
    DomainsCommand::Add(args) => add(args, context).await,
    DomainsCommand::List(args) => list(&args, context).await,
    DomainsCommand::Inspect(args) => inspect(&args, context).await,
    DomainsCommand::Records(args) => print_records(&args, context).await,
    DomainsCommand::Verify(args) => verify(&args, context).await,
    DomainsCommand::Delete(args) => delete(&args, context).await,
  }
}

async fn add(args: AddArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading(format!("Adding {}…", args.name));
  let body = CreateDomain {
    name: args.name,
    region: args.region,
    return_path: args.return_path,
  };
  let domain = api.post(&["v1", "domains"], &body).await?;

  let name = get_name(&domain);
  if context.output.is_json() {
    return context.output.print_json(&domain);
  }
  if !context.output.is_stdout_tty()
    && let Some(id) = domain.get("id").and_then(Value::as_str)
  {
    context.output.print_line(id);
  }

  if context.output.is_rich() {
    let region = domain.get("region").and_then(Value::as_str).unwrap_or_default();
    context.output.done(&format!("Added {name}"), region);
    eprint_record_blocks(&domain, true);
    let zone = format!("inboxjot domains records {name} --format bind > {name}.zone");
    let verify = format!("inboxjot domains verify {name} --wait");
    anstream::eprintln!();
    anstream::eprintln!(
      "{}",
      Line::new(vec![Span::styled("  Zone file  ", DIM), Span::styled(zone, ACTION)])
    );
    anstream::eprintln!(
      "{}",
      Line::new(vec![Span::styled("  next       ", DIM), Span::styled(verify, ACTION)])
    );
    return Ok(());
  }

  context.output.success(&format!("Added {name}"));

  eprint_records(&domain);
  context
    .output
    .info("Add these records at your DNS provider, then run:");
  context
    .output
    .next("", &format!("inboxjot domains verify {name} --wait"));
  Ok(())
}

async fn list(args: &ListArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading domains…");
  let page = api.list(&["v1", "domains"], &(), &args.to_page()).await?;

  context.output.print_list(
    &page,
    &LIST_COLUMNS,
    "domains",
    Some(("add one", "inboxjot domains add <domain>")),
  )
}

async fn inspect(args: &DomainArgs, context: &Context) -> Result<()> {
  let api = context
    .api()
    .await?
    .loading(format!("Loading {}…", get_name_or(&args.domain, "the domain")));
  let domain = get_domain(&api, &args.domain).await?;

  context.output.print_detail(&domain, &DETAIL_ROWS)?;
  if !context.output.is_json() {
    context.output.print_line("");
    context.output.print_table(get_records(&domain), &RECORD_COLUMNS);
  }

  Ok(())
}

async fn print_records(args: &RecordsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the DNS records…");
  let domain = get_domain(&api, &args.domain).await?;

  if context.output.is_json() {
    return context.output.print_json(&json!(get_records(&domain)));
  }

  match args.format {
    RecordFormat::Table => {
      context.output.print_table(get_records(&domain), &RECORD_COLUMNS);
    }
    RecordFormat::Bind => {
      for record in get_records(&domain) {
        context.output.print_line(&format_bind_record(record));
      }
    }
  }

  Ok(())
}

async fn verify(args: &VerifyArgs, context: &Context) -> Result<()> {
  let api = context.api().await?;
  let id = resolve_domain_id(&api, &args.domain).await?;
  let started = Instant::now();
  let mut domain = check_domain(&api, &id).await?;

  if args.wait {
    let spinner = context.output.start_spinner("Checking DNS…", &[]);
    while !is_verified(&domain) && !is_failed(&domain) {
      if started.elapsed() >= args.timeout {
        spinner.finish();
        let name = get_name(&domain);
        return Err(
          Error::failed("timeout", format!("{name} is not verified yet.")).with_next(
            format!("inboxjot domains verify {name} --wait"),
            "Keep waiting for the verification",
          ),
        );
      }

      spinner.set_message(format!("Checking DNS… {}", get_progress(&domain)));
      tokio::time::sleep(POLL_INTERVAL).await;
      domain = check_domain(&api, &id).await?;
    }
    spinner.finish();
  }

  let name = get_name(&domain).to_owned();
  if context.output.is_json() {
    context.output.print_json(&domain)?;
  } else if is_verified(&domain) {
    let elapsed = format_elapsed(started.elapsed());
    let headline = if args.wait {
      format!("Verified {name} in {elapsed}")
    } else {
      format!("Verified {name}")
    };
    context.output.success(&headline);
  } else {
    context.output.info(&format!(
      "{name} is {}. {}",
      get_status(&domain),
      get_progress(&domain)
    ));
    if context.output.is_rich() {
      eprint_record_blocks(&domain, false);
    } else {
      eprint_records(&domain);
    }
    if !args.wait {
      context
        .output
        .info("Waiting on DNS propagation. This can take up to 48 hours.");
      context
        .output
        .next("Wait for it with:", &format!("inboxjot domains verify {name} --wait"));
    }
  }

  if is_failed(&domain) {
    return Err(Error::failed(
      "verification_failed",
      format!("{name} failed verification."),
    ));
  }

  Ok(())
}

async fn delete(args: &DomainArgs, context: &Context) -> Result<()> {
  let api = context
    .api()
    .await?
    .loading(format!("Deleting {}…", get_name_or(&args.domain, "the domain")));
  let domain = get_domain(&api, &args.domain).await?;
  let name = get_name(&domain).to_owned();
  let id = domain
    .get("id")
    .and_then(Value::as_str)
    .unwrap_or_default()
    .to_owned();

  let keys = count_domain_keys(&api, &id).await;
  let consequence = match keys {
    0 => "Emails from this domain stop sending.".to_owned(),
    1 => {
      "Emails from this domain stop sending, and 1 API key limited to it stops working."
        .to_owned()
    }
    count => format!(
      "Emails from this domain stop sending, and {count} API keys limited to it stop working."
    ),
  };
  context.confirm_delete(&format!("Delete {name}?"), &consequence)?;

  api.delete(&["v1", "domains", &id]).await?;
  let result = json!({ "id": id, "name": name, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {name}"), "id", &[])
}

async fn count_domain_keys(api: &Api, domain_id: &str) -> usize {
  let page = Page { all: true, ..Page::default() };
  api
    .list(&["v1", "api-keys"], &(), &page)
    .await
    .ok()
    .and_then(|page| page.get("data").and_then(Value::as_array).cloned())
    .unwrap_or_default()
    .iter()
    .filter(|key| key.get("domain_id").and_then(Value::as_str) == Some(domain_id))
    .count()
}

pub async fn resolve_domain_id(api: &Api, selector: &str) -> Result<String> {
  if selector.starts_with("dom_") {
    return Ok(selector.to_owned());
  }

  let page = api
    .list(&["v1", "domains"], &(), &Page { all: true, ..Default::default() })
    .await?;
  let domains = page
    .get("data")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  domains
    .iter()
    .find(|domain| {
      domain
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case(selector))
    })
    .and_then(|domain| domain.get("id").and_then(Value::as_str))
    .map(str::to_owned)
    .ok_or_else(|| {
      Error::failed("not_found", format!("No domain is named {selector}."))
        .with_next("inboxjot domains list", "List the domains")
    })
}

async fn get_domain(api: &Api, selector: &str) -> Result<Value> {
  let id = resolve_domain_id(api, selector).await?;
  api.get(&["v1", "domains", &id], &()).await
}

async fn check_domain(api: &Api, id: &str) -> Result<Value> {
  api.post(&["v1", "domains", id, "verify"], &()).await
}

const STATUS_COLUMN: usize = 46;

fn eprint_record_blocks(domain: &Value, with_title: bool) {
  let records = get_records(domain);
  if records.is_empty() {
    return;
  }

  let width = get_width();
  anstream::eprintln!();
  if with_title {
    anstream::eprintln!(
      "{}",
      Line::from(Span::styled("  Add these records at your DNS provider", TITLE))
    );
    anstream::eprintln!();
  }
  for record in records {
    for line in render_record(record, get_name(domain), width) {
      anstream::eprintln!("{line}");
    }
    anstream::eprintln!();
  }
}

fn render_record(record: &Value, domain: &str, width: usize) -> Vec<Line<'static>> {
  let field =
    |key: &str| record.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
  let kind = field("kind");
  let (title, purpose) = match kind.as_str() {
    "dkim" => ("DKIM", "signs your email"),
    "return_path" => ("Return path", "bounces come back here"),
    "dmarc" => ("DMARC", "tells inboxes your policy"),
    "tracking_cname" => ("Tracking", "counts opens and clicks"),
    "tracking_caa" => ("CAA", "allows the tracking certificate"),
    _ => ("Record", ""),
  };
  let status = field("status");
  let state = match (kind.as_str(), status.as_str()) {
    (_, "verified") => {
      vec![Mark::Status(DONE).span(), Span::styled(" verified", SECONDARY)]
    }
    ("dmarc", _) => vec![Span::styled("recommended", DIM)],
    (_, "failed") => {
      vec![Mark::Status(DANGER).span(), Span::styled(" failed", SECONDARY)]
    }
    _ => vec![Mark::Status(WARNING).span(), Span::styled(" pending", SECONDARY)],
  };

  let head = format!("  {title}  {purpose}");
  let mut first = vec![
    Span::styled(format!("  {title}"), TITLE),
    Span::styled(format!("  {purpose}"), DIM),
  ];
  first.push(Span::raw(" ".repeat(STATUS_COLUMN.saturating_sub(head.width()).max(2))));
  first.extend(state);

  let name = field("name");
  let suffix = format!(".{domain}");
  let name_spans = match name.strip_suffix(&suffix) {
    Some(host) if !host.is_empty() => {
      vec![Span::styled(host.to_owned(), TITLE), Span::styled(suffix.clone(), DIM)]
    }
    _ => vec![Span::styled(name.clone(), TITLE)],
  };
  let label = |text: &str| Span::styled(format!("    {text:<7}"), DIM);
  let mut lines = vec![
    Line::new(first),
    Line::new(vec![label("Type"), Span::raw(field("type"))]),
    Line::new([vec![label("Name")], name_spans].concat()),
  ];
  if let Some(priority) = record.get("priority").and_then(Value::as_u64) {
    lines.push(Line::new(vec![label("Priority"), Span::raw(priority.to_string())]));
  }
  let value = field("value");
  if value.width() + 11 <= width {
    lines.push(Line::new(vec![label("Value"), Span::raw(value)]));
  } else {
    lines.push(Line::new(vec![label("Value"), Span::styled("↓", DIM)]));
    lines.push(Line::new(vec![Span::raw("    "), Span::raw(value)]));
  }
  lines
}

fn eprint_records(domain: &Value) {
  let records = get_records(domain);
  if records.is_empty() {
    return;
  }

  anstream::eprintln!();
  for record in records {
    let field =
      |key: &str| record.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    anstream::eprintln!(
      "  {:<6} {:<40} {}  {}",
      clean(&field("type")),
      clean(&field("name")),
      clean(&field("value")),
      clean(&field("status"))
    );
  }
  anstream::eprintln!();
}

fn get_records(domain: &Value) -> &[Value] {
  domain
    .get("records")
    .and_then(Value::as_array)
    .map_or(&[], Vec::as_slice)
}

fn get_name(domain: &Value) -> &str {
  domain.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn get_status(domain: &Value) -> &str {
  domain.get("status").and_then(Value::as_str).unwrap_or("unknown")
}

fn is_verified(domain: &Value) -> bool {
  get_status(domain) == "verified"
}

fn is_failed(domain: &Value) -> bool {
  get_status(domain) == "failed"
}

fn get_progress(domain: &Value) -> String {
  let records = get_records(domain);
  let verified = records
    .iter()
    .filter(|record| record.get("status").and_then(Value::as_str) == Some("verified"))
    .count();

  format!("{verified} of {} records verified", records.len())
}

fn format_elapsed(elapsed: Duration) -> String {
  let seconds = elapsed.as_secs();
  if seconds < 60 {
    return format!("{seconds}s");
  }

  format!("{}m {}s", seconds / 60, seconds % 60)
}

fn format_bind_record(record: &Value) -> String {
  let field = |key: &str| record.get(key).and_then(Value::as_str).unwrap_or_default();
  let record_type = field("type");
  let value = match record_type {
    "TXT" => format!("\"{}\"", field("value").replace('"', "\\\"")),
    _ => format!("{}.", field("value").trim_end_matches('.')),
  };
  let priority = record
    .get("priority")
    .and_then(Value::as_i64)
    .map(|priority| format!("{priority} "))
    .unwrap_or_default();

  format!(
    "{}.\t3600\tIN\t{record_type}\t{priority}{value}",
    field("name").trim_end_matches('.')
  )
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{format_bind_record, get_progress};

  #[test]
  fn writes_bind_records() {
    let txt = json!({ "type": "TXT", "name": "acme.com", "value": "v=spf1 ~all" });
    assert_eq!(format_bind_record(&txt), "acme.com.\t3600\tIN\tTXT\t\"v=spf1 ~all\"");

    let mx = json!({ "type": "MX", "name": "send.acme.com", "value": "feedback.example.com", "priority": 10 });
    assert_eq!(
      format_bind_record(&mx),
      "send.acme.com.\t3600\tIN\tMX\t10 feedback.example.com."
    );
  }

  #[test]
  fn counts_verified_records() {
    let domain =
      json!({ "records": [{ "status": "verified" }, { "status": "pending" }] });
    assert_eq!(get_progress(&domain), "1 of 2 records verified");
  }
}
