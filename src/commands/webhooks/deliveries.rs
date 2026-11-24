use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::Result;
use crate::output::{Column, column};

const LIST_COLUMNS: [Column; 6] = [
  column("ID", "delivery_id"),
  column("Event", "event_type"),
  column("Endpoint", "endpoint_name"),
  column("Status", "status"),
  column("Attempt", "attempt"),
  column("Sent", "created_at"),
];

const DETAIL_ROWS: [Column; 8] = [
  column("ID", "delivery_id"),
  column("Event", "event_type"),
  column("Event ID", "event_id"),
  column("Endpoint", "endpoint_name"),
  column("URL", "endpoint_url"),
  column("Email", "email_id"),
  column("Sent", "created_at"),
  column("Next attempt", "next_attempt_at"),
];

const ATTEMPT_COLUMNS: [Column; 5] = [
  column("Attempt", "attempt"),
  column("Status", "status"),
  column("Duration ms", "duration_ms"),
  column("Error", "error"),
  column("Tried", "attempted_at"),
];

#[derive(Debug, Subcommand)]
pub enum DeliveriesCommand {
  #[command(
    about = "List webhook deliveries",
    after_help = examples!("inboxjot webhooks deliveries list --status failed"),
    alias = "ls",
  )]
  List(ListDeliveriesArgs),
  #[command(
    about = "Show the request and every attempt of a delivery",
    after_help = examples!("inboxjot webhooks deliveries inspect whd_123"),
  )]
  Inspect(DeliveryArgs),
  #[command(
    about = "Send a delivery again",
    after_help = examples!("inboxjot webhooks deliveries replay whd_123"),
  )]
  Replay(DeliveryArgs),
}

#[derive(Debug, Args)]
pub struct ListDeliveriesArgs {
  #[arg(long, value_name = "ENDPOINT", help = "Show only deliveries to this endpoint")]
  pub endpoint: Option<String>,

  #[arg(long, value_enum, help = "Show only deliveries with this outcome")]
  pub status: Option<Outcome>,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct DeliveryArgs {
  #[arg(value_name = "DELIVERY", help = "Delivery ID, such as whd_123")]
  pub delivery: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
  Succeeded,
  Retrying,
  Failed,
}

pub async fn run(command: DeliveriesCommand, context: &Context) -> Result<()> {
  match command {
    DeliveriesCommand::List(args) => list(&args, context).await,
    DeliveriesCommand::Inspect(args) => inspect(&args, context).await,
    DeliveriesCommand::Replay(args) => replay(&args, context).await,
  }
}

async fn list(args: &ListDeliveriesArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading deliveries…");
  let query = json!({ "endpoint_id": args.endpoint, "outcome": args.status });
  let segments = ["v1", "webhook-deliveries"];
  let page = api.list(&segments, &query, &args.list.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "deliveries", None)
}

async fn inspect(args: &DeliveryArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the delivery…");
  let segments = ["v1", "webhook-deliveries", &args.delivery];
  let delivery = api.get(&segments, &()).await?;

  context.output.print_detail(&delivery, &DETAIL_ROWS)?;
  if context.output.is_json() {
    return Ok(());
  }

  context.output.print_line("");
  for (name, value) in delivery["headers"].as_object().into_iter().flatten() {
    context
      .output
      .print_line(&format!("{name}: {}", value.as_str().unwrap_or_default()));
  }
  print_payload(&delivery, context);

  let attempts = delivery["attempts"].as_array().map_or(&[][..], Vec::as_slice);
  context.output.print_line("");
  context.output.print_table(attempts, &ATTEMPT_COLUMNS);

  let response = attempts.last().and_then(|attempt| attempt["response_body"].as_str());
  if let Some(response) = response.filter(|body| !body.is_empty()) {
    context.output.print_line("");
    context.output.print_line(response);
  }

  Ok(())
}

fn print_payload(delivery: &Value, context: &Context) {
  let payload = delivery["payload"].as_str().unwrap_or_default();
  if payload.is_empty() {
    context.output.info("The payload is past the retention period.");
    return;
  }

  let pretty = serde_json::from_str::<Value>(payload)
    .ok()
    .and_then(|value| serde_json::to_string_pretty(&value).ok());
  context.output.print_line("");
  context.output.print_line(pretty.as_deref().unwrap_or(payload));
}

async fn replay(args: &DeliveryArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Replaying the delivery…");
  let id = &args.delivery;
  let segments = ["v1", "webhook-deliveries", id, "replay"];
  let replayed = api.post(&segments, &()).await?;

  context.output.print_result(
    &replayed,
    &format!("Queued a replay of {id}"),
    "delivery_id",
    &[column("New delivery", "delivery_id")],
  )
}
