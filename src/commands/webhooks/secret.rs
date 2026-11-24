use clap::Subcommand;
use serde_json::Value;

use super::EndpointArgs;
use crate::context::Context;
use crate::error::Result;

#[derive(Debug, Subcommand)]
pub enum SecretCommand {
  #[command(
    about = "Print the signing secret of a webhook endpoint",
    after_help = examples!("inboxjot webhooks secret show whk_123"),
  )]
  Show(EndpointArgs),
  #[command(
    about = "Replace the signing secret of a webhook endpoint",
    after_help = examples!("inboxjot webhooks secret rotate whk_123"),
  )]
  Rotate(EndpointArgs),
}

pub async fn run(command: SecretCommand, context: &Context) -> Result<()> {
  match command {
    SecretCommand::Show(args) => show(&args, context).await,
    SecretCommand::Rotate(args) => rotate(&args, context).await,
  }
}

async fn show(args: &EndpointArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the secret…");
  let segments = ["v1", "webhooks", &args.endpoint, "secret"];
  let body = api.get(&segments, &()).await?;

  if context.output.is_json() {
    return context.output.print_json(&body);
  }

  context.output.print_line(get_secret(&body));
  Ok(())
}

async fn rotate(args: &EndpointArgs, context: &Context) -> Result<()> {
  let id = &args.endpoint;
  context.confirm(&format!(
    "Rotate the signing secret of {id}? The old secret stops working at once."
  ))?;

  let api = context.api().await?.loading("Rotating the secret…");
  let segments = ["v1", "webhooks", id, "secret"];
  let body = api.post(&segments, &()).await?;

  if context.output.is_json() {
    return context.output.print_json(&body);
  }

  context.output.success(&format!("Rotated the signing secret of {id}"));
  context
    .output
    .info("The new signing secret is on stdout. The old secret no longer works.");
  context.output.print_line(get_secret(&body));
  Ok(())
}

fn get_secret(body: &Value) -> &str {
  body.get("secret").and_then(Value::as_str).unwrap_or_default()
}
