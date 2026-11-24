use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::commands::emails::parse_data;
use crate::context::Context;
use crate::error::Result;

#[derive(Debug, Subcommand)]
pub enum EventsCommand {
  #[command(
    about = "Record an event for a contact",
    after_help = examples!(
      "inboxjot events track signed_up --contact riti@example.com --properties '{\"plan\":\"pro\"}'",
      "inboxjot events track paid --user-id usr_8842 --occurred-at 2026-11-14T09:30:00Z",
    ),
  )]
  Track(TrackArgs),
}

#[derive(Debug, Args)]
pub struct TrackArgs {
  #[arg(value_name = "NAME", help = "Event name, such as signed_up")]
  pub name: String,

  #[arg(
    long,
    value_name = "EMAIL",
    required_unless_present = "user_id",
    help = "Contact address"
  )]
  pub contact: Option<String>,

  #[arg(long, value_name = "ID", help = "Your ID for the contact")]
  pub user_id: Option<String>,

  #[arg(
    long,
    alias = "data",
    value_name = "JSON",
    value_parser = parse_data,
    help = "Event properties as a JSON object"
  )]
  pub properties: Option<Map<String, Value>>,

  #[arg(long, value_name = "TIME", help = "When it happened, as ISO 8601")]
  pub occurred_at: Option<String>,

  #[arg(
    long,
    value_name = "ID",
    help = "Your ID for the event, so a retry is recorded once"
  )]
  pub id: Option<String>,
}

pub async fn run(command: EventsCommand, context: &Context) -> Result<()> {
  match command {
    EventsCommand::Track(args) => track(args, context).await,
  }
}

async fn track(args: TrackArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading(format!("Tracking {}…", args.name));
  let contact = args
    .contact
    .as_deref()
    .or(args.user_id.as_deref())
    .unwrap_or_default();
  let headline = format!("Tracked {} for {contact}", args.name);
  let body = json!({
    "id": args.id,
    "event": args.name,
    "email": args.contact,
    "user_id": args.user_id,
    "occurred_at": args.occurred_at,
    "properties": args.properties.unwrap_or_default(),
  });
  let tracked = api.post(&["v1", "events"], &body).await?;

  context.output.print_result(&tracked, &headline, "contact_id", &[])
}
