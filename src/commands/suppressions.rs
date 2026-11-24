use std::io::Read;

use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};

use crate::api::Api;
use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column, status_column};
use crate::ui::spinner::get_name_or;

const MAX_BATCH: usize = 1000;

const LIST_COLUMNS: [Column; 6] = [
  column("ID", "id"),
  column("Email", "email"),
  column("Reason", "reason"),
  column("Scope", "scope"),
  column("Source", "source"),
  column("Created", "created_at"),
];

const TERMINAL_COLUMNS: [Column; 4] = [
  column("Email", "email"),
  status_column("Reason", "reason"),
  column("Source", "source").secondary(),
  column("Created", "created_at"),
];

#[derive(Debug, Subcommand)]
pub enum SuppressionsCommand {
  #[command(
    about = "List suppressed addresses",
    after_help = examples!("inboxjot suppressions list --reason bounce"),
    alias = "ls",
  )]
  List(ListSuppressionsArgs),
  #[command(
    about = "Stop sending to email addresses",
    after_help = examples!("inboxjot suppressions add riti@example.com"),
  )]
  Add(AddArgs),
  #[command(
    about = "Send to a suppressed address again",
    after_help = examples!("inboxjot suppressions remove riti@example.com"),
    alias = "rm",
  )]
  Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct ListSuppressionsArgs {
  #[arg(long, value_enum, help = "Show only suppressions with this reason")]
  pub reason: Option<Reason>,

  #[arg(long, help = "Show only addresses that contain this text")]
  pub query: Option<String>,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct AddArgs {
  #[arg(
    value_name = "EMAIL",
    required = true,
    help = "Email address, or - to read one per line from stdin"
  )]
  pub emails: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
  #[arg(value_name = "EMAIL", help = "Suppressed email address or suppression ID")]
  pub suppression: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
  Bounce,
  Complaint,
  Manual,
}

#[derive(Debug, Serialize)]
struct ListQuery<'a> {
  reason: Option<Reason>,
  query: Option<&'a str>,
}

pub async fn run(command: SuppressionsCommand, context: &Context) -> Result<()> {
  match command {
    SuppressionsCommand::List(args) => list(&args, context).await,
    SuppressionsCommand::Add(args) => add(&args, context).await,
    SuppressionsCommand::Remove(args) => remove(&args, context).await,
  }
}

async fn list(args: &ListSuppressionsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading suppressions…");
  let query = ListQuery { reason: args.reason, query: args.query.as_deref() };
  let url = ["v1", "suppressions"];
  let mut page = api.list(&url, &query, &args.list.to_page()).await?;
  if context.output.is_json() || !context.output.is_stdout_tty() {
    return context.output.print_list(&page, &LIST_COLUMNS, "suppressions", None);
  }

  name_sources(&mut page);
  context.output.print_list(
    &page,
    &TERMINAL_COLUMNS,
    "suppressions",
    Some(("add one", "inboxjot suppressions add <email>")),
  )
}

async fn add(args: &AddArgs, context: &Context) -> Result<()> {
  let emails = read_emails(&args.emails)?;
  let api = context.api().await?.loading(match emails.as_slice() {
    [email] => format!("Adding {email}…"),
    _ => format!("Adding {} addresses…", emails.len()),
  });
  let url = ["v1", "suppressions"];

  let (mut added, mut skipped) = (0, 0);
  for chunk in emails.chunks(MAX_BATCH) {
    let result = api.post(&url, &json!({ "emails": chunk })).await?;
    added += result.get("added").and_then(Value::as_u64).unwrap_or(0);
    skipped += result.get("skipped").and_then(Value::as_u64).unwrap_or(0);
  }

  let result = json!({ "added": added, "skipped": skipped });
  if context.output.is_json() {
    return context.output.print_json(&result);
  }

  let noun = if added == 1 { "address" } else { "addresses" };
  context.output.success(&format!("Suppressed {added} {noun}"));
  if skipped > 0 {
    let verb = if skipped == 1 { "was" } else { "were" };
    context
      .output
      .info(&format!("Skipped {skipped} that {verb} suppressed already."));
  }
  Ok(())
}

async fn remove(args: &RemoveArgs, context: &Context) -> Result<()> {
  let api = context
    .api()
    .await?
    .loading(format!("Removing {}…", get_name_or(&args.suppression, "the address")));
  let ids = resolve_ids(&api, &args.suppression).await?;
  context.confirm(&format!(
    "Remove {} from the suppression list? Email to it sends again.",
    args.suppression
  ))?;

  for id in &ids {
    api.delete(&["v1", "suppressions", id]).await?;
  }

  let result = json!({ "ids": ids, "email": args.suppression, "removed": true });
  context
    .output
    .print_removal(&result, &format!("Removed {}", args.suppression), "", &[])
}

fn name_sources(page: &mut Value) {
  let Some(rows) = page.get_mut("data").and_then(Value::as_array_mut) else {
    return;
  };
  for row in rows {
    let name = match row.get("source").and_then(Value::as_str) {
      Some(source) if source.len() <= 3 => source.to_uppercase(),
      Some(source) => {
        let mut characters = source.chars();
        characters
          .next()
          .map(|first| first.to_uppercase().chain(characters).collect())
          .unwrap_or_default()
      }
      None => continue,
    };
    row["source"] = Value::from(name);
  }
}

async fn resolve_ids(api: &Api, selector: &str) -> Result<Vec<String>> {
  if !selector.contains('@') {
    return Ok(vec![selector.to_owned()]);
  }

  let query = json!({ "query": selector, "per_page": 100 });
  let page = api.get(&["v1", "suppressions"], &query).await?;
  let ids: Vec<String> = page
    .get("data")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter(|row| {
      row
        .get("email")
        .and_then(Value::as_str)
        .is_some_and(|email| email.eq_ignore_ascii_case(selector))
    })
    .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_owned))
    .collect();

  if ids.is_empty() {
    return Err(Error::failed("not_found", format!("{selector} is not suppressed.")));
  }

  Ok(ids)
}

fn read_emails(args: &[String]) -> Result<Vec<String>> {
  let mut emails = Vec::new();
  for arg in args {
    if arg != "-" {
      emails.push(arg.trim().to_owned());
      continue;
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    emails.extend(
      input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned),
    );
  }

  if emails.is_empty() {
    return Err(Error::usage("missing_input", "Add at least one email address."));
  }

  Ok(emails)
}
