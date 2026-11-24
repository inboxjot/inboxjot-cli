use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::Result;
use crate::output::{Column, column};

const LIST_COLUMNS: [Column; 7] = [
  column("ID", "id"),
  column("Name", "name"),
  column("Status", "status"),
  column("Trigger", "trigger_label"),
  column("Active", "active_count"),
  column("Sent", "sent_count"),
  column("Updated", "updated_at"),
];

const DETAIL_ROWS: [Column; 12] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Status", "status"),
  column("Description", "description"),
  column("Trigger", "trigger_label"),
  column("Entered", "entered_count"),
  column("Active", "active_count"),
  column("Completed", "completed_count"),
  column("Exited", "exited_count"),
  column("Sent", "sent_count"),
  column("Turned on", "turned_on_at"),
  column("Updated", "updated_at"),
];

#[derive(Debug, Subcommand)]
pub enum AutomationsCommand {
  #[command(
    about = "List automations",
    after_help = examples!("inboxjot automations list --status on"),
    alias = "ls",
  )]
  List(ListAutomationsArgs),
  #[command(
    about = "Show an automation and its problems",
    after_help = examples!("inboxjot automations inspect aut_123"),
  )]
  Inspect(AutomationArgs),
  #[command(
    about = "Turn an automation on",
    after_help = examples!("inboxjot automations enable aut_123"),
  )]
  Enable(AutomationArgs),
  #[command(
    about = "Turn an automation off",
    after_help = examples!("inboxjot automations disable aut_123"),
  )]
  Disable(AutomationArgs),
}

#[derive(Debug, Args)]
pub struct ListAutomationsArgs {
  #[arg(long, value_enum, help = "Show only automations with this status")]
  pub status: Option<Status>,

  #[arg(long, help = "Show only automations whose name contains this text")]
  pub query: Option<String>,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct AutomationArgs {
  #[arg(value_name = "AUTOMATION_ID", help = "Automation ID")]
  pub id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
  Draft,
  On,
  Off,
}

#[derive(Debug, Serialize)]
struct ListQuery<'a> {
  status: Option<Status>,
  query: Option<&'a str>,
}

pub async fn run(command: AutomationsCommand, context: &Context) -> Result<()> {
  match command {
    AutomationsCommand::List(args) => list(&args, context).await,
    AutomationsCommand::Inspect(args) => inspect(&args, context).await,
    AutomationsCommand::Enable(args) => set_status(&args, "on", context).await,
    AutomationsCommand::Disable(args) => set_status(&args, "off", context).await,
  }
}

async fn list(args: &ListAutomationsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading automations…");
  let query = ListQuery { status: args.status, query: args.query.as_deref() };
  let url = ["v1", "automations"];
  let page = api.list(&url, &query, &args.list.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "automations", None)
}

async fn inspect(args: &AutomationArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the automation…");
  let url = ["v1", "automations", &args.id];
  let automation = api.get(&url, &()).await?;

  context.output.print_detail(&automation, &DETAIL_ROWS)?;
  if !context.output.is_json() {
    print_problems(&automation, context);
  }
  Ok(())
}

async fn set_status(
  args: &AutomationArgs,
  status: &str,
  context: &Context,
) -> Result<()> {
  let api = context.api().await?.loading(if status == "enabled" {
    "Turning on the automation…"
  } else {
    "Turning off the automation…"
  });
  let url = ["v1", "automations", &args.id, "status"];
  let automation = api.post(&url, &json!({ "status": status })).await?;

  let name = automation.get("name").and_then(Value::as_str).unwrap_or(&args.id);
  let headline = format!("Turned {status} {name}");
  context
    .output
    .print_result(&automation, &headline, "id", &[column("Status", "status")])
}

fn print_problems(automation: &Value, context: &Context) {
  let problems = automation.get("problems").and_then(Value::as_array);
  let messages: Vec<&str> = problems
    .into_iter()
    .flatten()
    .filter_map(|problem| problem.get("message").and_then(Value::as_str))
    .collect();
  if messages.is_empty() {
    return;
  }

  context.output.print_line("");
  context.output.print_line("Problems");
  for message in messages {
    context.output.print_line(&format!("  {message}"));
  }
}
