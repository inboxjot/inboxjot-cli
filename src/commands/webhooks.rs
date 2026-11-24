mod deliveries;
mod secret;

use std::path::PathBuf;

use clap::{Args, Subcommand};
use inboxjot::webhooks::Error as SignatureError;
use inboxjot::{CreateWebhookEndpoint, EndpointStatus, UpdateWebhookEndpoint};
use reqwest::Url;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};

use crate::cli::ListArgs;
use crate::commands::stream::{EVENT_TYPES, EventType};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column, status_column};
use crate::ui::style::{DIM, TITLE};
use crate::ui::text::{Line, Span};

const LIST_COLUMNS: [Column; 6] = [
  column("ID", "id"),
  column("Name", "name"),
  column("URL", "url"),
  status_column("Status", "status"),
  column("Success", "success_rate"),
  column("Last delivery", "last_delivery_at"),
];

const DETAIL_ROWS: [Column; 12] = [
  column("Name", "name"),
  column("ID", "id"),
  column("URL", "url"),
  status_column("Status", "status"),
  column("Disabled reason", "disabled_reason"),
  column("Events", "events"),
  column("Deliveries (24h)", "deliveries_24h"),
  column("Success rate", "success_rate"),
  column("Avg duration ms", "avg_duration_ms"),
  column("Retrying", "retrying"),
  column("Created", "created_at"),
  column("Updated", "updated_at"),
];

const DELIVERY_COLUMNS: [Column; 5] = [
  column("Event", "event_type"),
  status_column("Status", "status"),
  column("Attempt", "attempt"),
  column("Duration ms", "duration_ms"),
  column("Sent", "created_at"),
];

#[derive(Debug, Subcommand)]
pub enum WebhooksCommand {
  #[command(
    about = "List webhook endpoints",
    after_help = examples!("inboxjot webhooks list"),
    alias = "ls",
  )]
  List(ListArgs),
  #[command(
    about = "Show a webhook endpoint and its recent deliveries",
    after_help = examples!("inboxjot webhooks inspect whk_123"),
  )]
  Inspect(EndpointArgs),
  #[command(
    about = "Create a webhook endpoint and print its signing secret",
    after_help = examples!("inboxjot webhooks create --url https://acme.com/webhooks --events email.delivered,email.bounced"),
  )]
  Create(CreateArgs),
  #[command(
    about = "Change a webhook endpoint",
    after_help = examples!("inboxjot webhooks update whk_123 --disable"),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Delete a webhook endpoint",
    after_help = examples!("inboxjot webhooks delete whk_123"),
    alias = "rm",
  )]
  Delete(EndpointArgs),
  #[command(
    about = "Send a test event to a webhook endpoint",
    after_help = examples!("inboxjot webhooks test whk_123"),
  )]
  Test(EndpointArgs),
  #[command(subcommand, about = "Print or rotate the signing secret of an endpoint")]
  Secret(secret::SecretCommand),
  #[command(subcommand, about = "List, inspect and replay webhook deliveries")]
  Deliveries(deliveries::DeliveriesCommand),
  #[command(
    about = "Check the signature of a webhook payload offline",
    after_help = examples!("inboxjot webhooks verify --secret \"$WEBHOOK_SECRET\" --payload body.json --id msg_123 --timestamp 1794220000 --signature \"v1,K5oZ…\""),
  )]
  Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct EndpointArgs {
  #[arg(value_name = "ENDPOINT", help = "Webhook endpoint ID, such as whk_123")]
  pub endpoint: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "HTTPS URL that receives the events")]
  pub url: String,

  #[arg(long, help = "Name of the endpoint; defaults to the URL host")]
  pub name: Option<String>,

  #[arg(
    long,
    value_enum,
    value_delimiter = ',',
    value_name = "EVENTS",
    help = "Comma-separated event types; defaults to every event"
  )]
  pub events: Vec<EventType>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(value_name = "ENDPOINT", help = "Webhook endpoint ID, such as whk_123")]
  pub endpoint: String,

  #[arg(long, help = "New HTTPS URL that receives the events")]
  pub url: Option<String>,

  #[arg(long, help = "New name of the endpoint")]
  pub name: Option<String>,

  #[arg(
    long,
    value_enum,
    value_delimiter = ',',
    value_name = "EVENTS",
    help = "Comma-separated event types that replace the current list"
  )]
  pub events: Option<Vec<EventType>>,

  #[arg(long, conflicts_with = "disable", help = "Turn the endpoint on")]
  pub enable: bool,

  #[arg(long, help = "Turn the endpoint off")]
  pub disable: bool,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
  #[arg(
    long,
    env = "INBOXJOT_WEBHOOK_SECRET",
    hide_env_values = true,
    value_name = "SECRET",
    help = "Signing secret of the endpoint, starting with whsec_"
  )]
  pub secret: String,

  #[arg(
    long,
    value_name = "FILE",
    default_value = "-",
    help = "File with the raw request body, or - for stdin"
  )]
  pub payload: PathBuf,

  #[arg(long, value_name = "ID", help = "Value of the webhook-id header")]
  pub id: Option<String>,

  #[arg(long, value_name = "SECONDS", help = "Value of the webhook-timestamp header")]
  pub timestamp: Option<String>,

  #[arg(long, value_name = "SIGNATURES", help = "Value of the webhook-signature header")]
  pub signature: Option<String>,

  #[arg(
    short = 'H',
    long = "header",
    value_name = "HEADER",
    help = "Request header as 'Name: value'; repeat for each header"
  )]
  pub headers: Vec<String>,
}

pub async fn run(command: WebhooksCommand, context: &Context) -> Result<()> {
  match command {
    WebhooksCommand::List(args) => list(&args, context).await,
    WebhooksCommand::Inspect(args) => inspect(&args, context).await,
    WebhooksCommand::Create(args) => create(args, context).await,
    WebhooksCommand::Update(args) => update(args, context).await,
    WebhooksCommand::Delete(args) => delete(&args, context).await,
    WebhooksCommand::Test(args) => send_test(&args, context).await,
    WebhooksCommand::Secret(command) => secret::run(command, context).await,
    WebhooksCommand::Deliveries(command) => deliveries::run(command, context).await,
    WebhooksCommand::Verify(args) => verify(&args, context).await,
  }
}

async fn list(args: &ListArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading webhooks…");
  let page = api.list(&["v1", "webhooks"], &(), &args.to_page()).await?;

  context.output.print_list(
    &page,
    &LIST_COLUMNS,
    "webhook endpoints",
    Some(("add one", "inboxjot webhooks create --url <url>")),
  )
}

async fn inspect(args: &EndpointArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the webhook…");
  let endpoint = api.get(&["v1", "webhooks", &args.endpoint], &()).await?;

  context.output.print_detail(&endpoint, &DETAIL_ROWS)?;
  let deliveries = endpoint
    .get("recent_deliveries")
    .and_then(Value::as_array)
    .map_or(&[][..], Vec::as_slice);
  if !context.output.is_json() && !deliveries.is_empty() {
    context.output.print_line("");
    context.output.print_table(deliveries, &DELIVERY_COLUMNS);
  }

  Ok(())
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let url = parse_https_url(&args.url)?;
  let events = if args.events.is_empty() { EVENT_TYPES.to_vec() } else { args.events };
  let body = CreateWebhookEndpoint {
    name: args
      .name
      .unwrap_or_else(|| url.host_str().unwrap_or_default().to_owned()),
    url: args.url,
    events: events.into_iter().map(|event| event.0).collect(),
  };

  let api = context.api().await?.loading("Creating the webhook…");
  let endpoint = api.post(&["v1", "webhooks"], &body).await?;
  if context.output.is_json() {
    return context.output.print_json(&endpoint);
  }

  let id = endpoint.get("id").and_then(Value::as_str).unwrap_or_default();
  let secret = endpoint.get("secret").and_then(Value::as_str).unwrap_or_default();
  if context.output.is_rich() && context.output.is_stdout_tty() {
    let events = endpoint
      .get("events")
      .and_then(Value::as_array)
      .map(|events| events.iter().filter_map(Value::as_str).collect::<Vec<_>>())
      .unwrap_or_default();
    let events = if events.len() >= EVENT_TYPES.len() {
      "all events".to_owned()
    } else {
      events.join(", ")
    };
    let url = endpoint.get("url").and_then(Value::as_str).unwrap_or_default();
    let field = |label: &str, value: String| {
      Line::new(vec![Span::styled(format!("  {label:<10}"), DIM), Span::raw(value)])
    };
    context.output.done("Created a webhook endpoint", id);
    anstream::eprintln!("{}", field("url", url.to_owned()));
    anstream::eprintln!("{}", field("events", events));
    anstream::println!();
    anstream::println!("{}", Line::from(Span::styled("  Signing secret", TITLE)));
    anstream::println!("{}", Line::from(Span::styled(format!("  {secret}"), TITLE)));
    anstream::println!();
    context
      .output
      .warn("Shown once. Your server uses it to check the webhook-signature header.");
    context.output.print_next(&[
      ("send a test event", "inboxjot trigger email.delivered"),
      ("check a signature by hand", "inboxjot webhooks verify --help"),
    ]);
    return Ok(());
  }

  context.output.success(&format!("Created the webhook endpoint {id}"));
  context
    .output
    .info("The signing secret is on stdout. It is shown once.");
  context.output.print_line(secret);
  Ok(())
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  if let Some(url) = &args.url {
    parse_https_url(url)?;
  }
  let status = match (args.enable, args.disable) {
    (true, _) => Some(EndpointStatus::Enabled),
    (_, true) => Some(EndpointStatus::Disabled),
    _ => None,
  };
  let body = UpdateWebhookEndpoint {
    name: args.name,
    url: args.url,
    events: args
      .events
      .map(|events| events.into_iter().map(|event| event.0).collect()),
    status,
  };
  if body == UpdateWebhookEndpoint::default() {
    return Err(Error::usage(
      "missing_input",
      "Add --url, --name, --events, --enable or --disable.",
    ));
  }

  let api = context.api().await?.loading("Saving the webhook…");
  let endpoint = api.patch(&["v1", "webhooks", &args.endpoint], &body).await?;
  context.output.print_result(
    &endpoint,
    &format!("Updated the webhook endpoint {}", args.endpoint),
    "id",
    &[column("Status", "status"), column("URL", "url")],
  )
}

async fn delete(args: &EndpointArgs, context: &Context) -> Result<()> {
  let id = &args.endpoint;
  context
    .confirm_delete(&format!("Delete {id}?"), "Events stop going to this endpoint.")?;

  let api = context.api().await?.loading("Deleting the webhook…");
  api.delete(&["v1", "webhooks", id]).await?;
  let result = json!({ "id": id, "deleted": true });
  context.output.print_removal(
    &result,
    &format!("Deleted the webhook endpoint {id}"),
    "id",
    &[],
  )
}

async fn send_test(args: &EndpointArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Sending a test event…");
  let id = &args.endpoint;
  let queued = api.post(&["v1", "webhooks", id, "test"], &()).await?;

  let headline = format!("Queued a test event for {id}");
  context.output.print_result(&queued, &headline, "delivery_id", &[])?;
  let delivery = queued.get("delivery_id").and_then(Value::as_str).unwrap_or_default();
  context.output.next(
    "See the result with:",
    &format!("inboxjot webhooks deliveries inspect {delivery}"),
  );
  Ok(())
}

async fn verify(args: &VerifyArgs, context: &Context) -> Result<()> {
  let headers = build_signature_headers(args)?;
  let payload = if args.payload.as_os_str() == "-" {
    let mut buffer = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut tokio::io::stdin(), &mut buffer).await?;
    buffer
  } else {
    tokio::fs::read(&args.payload).await?
  };

  match inboxjot::webhooks::verify(&args.secret, &headers, &payload) {
    Ok(_) | Err(SignatureError::InvalidPayload(_)) => {}
    Err(SignatureError::MissingHeader(name)) => {
      return Err(Error::usage(
        "missing_header",
        format!("The {name} header is missing. Add it with -H '{name}: …'."),
      ));
    }
    Err(error) => {
      return Err(Error::failed(
        "invalid_signature",
        format!("The webhook is not valid: {error}."),
      ));
    }
  }

  if context.output.is_json() {
    return context.output.print_json(&json!({ "valid": true }));
  }

  context.output.success("Valid signature");
  Ok(())
}

fn build_signature_headers(args: &VerifyArgs) -> Result<HeaderMap> {
  let mut headers = HeaderMap::new();
  for header in &args.headers {
    let (name, value) = parse_header(header)?;
    headers.insert(name, value);
  }

  let flags = [
    ("webhook-id", &args.id),
    ("webhook-timestamp", &args.timestamp),
    ("webhook-signature", &args.signature),
  ];
  for (name, value) in flags {
    if let Some(value) = value {
      let value = HeaderValue::from_str(value).map_err(|_| {
        Error::usage("invalid_header", format!("The {name} value is not valid."))
      })?;
      headers.insert(name, value);
    }
  }

  Ok(headers)
}

pub fn parse_header(header: &str) -> Result<(HeaderName, HeaderValue)> {
  let invalid = || {
    Error::usage("invalid_header", format!("Write the header {header} as 'Name: value'."))
  };
  let (name, value) = header.split_once(':').ok_or_else(invalid)?;
  let name = HeaderName::from_bytes(name.trim().as_bytes()).map_err(|_| invalid())?;
  let value = HeaderValue::from_str(value.trim()).map_err(|_| invalid())?;

  Ok((name, value))
}

fn parse_https_url(url: &str) -> Result<Url> {
  Url::parse(url)
    .ok()
    .filter(|url| url.scheme() == "https")
    .ok_or_else(|| {
      Error::usage("invalid_url", format!("The URL {url} must start with https://."))
    })
}

#[cfg(test)]
mod tests {
  use super::parse_header;

  #[test]
  fn splits_a_header_at_the_first_colon() {
    let (name, value) = parse_header("webhook-id: msg:1").unwrap();
    assert_eq!(name.as_str(), "webhook-id");
    assert_eq!(value, "msg:1");
    assert!(parse_header("no colon").is_err());
  }
}
