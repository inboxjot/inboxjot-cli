use clap::{Args, Subcommand, ValueEnum};
use inboxjot::{CreateTopic, TopicVisibility, UpdateTopic};
use serde_json::{Value, json};

use crate::api::{Api, Page};
use crate::cli::ListArgs;
use crate::commands::brands::resolve_brand_id;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column};

const LIST_COLUMNS: [Column; 5] = [
  column("Name", "name"),
  column("Visibility", "visibility"),
  column("Default", "default_subscribed"),
  column("Subscribed", "counts.subscribed"),
  column("Created", "created_at"),
];

const DETAIL_ROWS: [Column; 16] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Brand", "brand_id"),
  column("Description", "description"),
  column("Visibility", "visibility"),
  column("Default", "default_subscribed"),
  column("Double opt-in", "double_opt_in"),
  column("Form", "form_enabled"),
  column("Sender name", "sender_name"),
  column("Sender email", "sender_email"),
  column("Redirect URL", "redirect_url"),
  column("Subscribed", "counts.subscribed"),
  column("Pending", "counts.pending"),
  column("Unsubscribed", "counts.unsubscribed"),
  column("Created", "created_at"),
  column("Updated", "updated_at"),
];

#[derive(Debug, Subcommand)]
pub enum TopicsCommand {
  #[command(
    about = "List topics",
    after_help = examples!("inboxjot topics list"),
    alias = "ls",
  )]
  List(ListArgs),
  #[command(
    about = "Show a topic",
    after_help = examples!("inboxjot topics inspect top_123"),
  )]
  Inspect(TopicArgs),
  #[command(
    about = "Create a topic",
    after_help = examples!(
      "inboxjot topics create --name \"Product updates\" --double-opt-in true",
      "inboxjot topics create --name \"Labs news\" --brand \"Acme Labs\"",
    ),
  )]
  Create(CreateArgs),
  #[command(
    about = "Change a topic",
    after_help = examples!("inboxjot topics update top_123 --visibility private"),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Delete a topic",
    after_help = examples!("inboxjot topics delete top_123"),
    alias = "rm",
  )]
  Delete(TopicArgs),
}

#[derive(Debug, Args)]
pub struct TopicArgs {
  #[arg(value_name = "TOPIC", help = "Topic name or ID")]
  pub topic: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Topic name, such as \"Product updates\"")]
  pub name: String,

  #[arg(
    long,
    value_name = "BRAND",
    help = "Brand name or ID that owns the topic. Defaults to the default brand"
  )]
  pub brand: Option<String>,

  #[command(flatten)]
  pub fields: TopicFields,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(value_name = "TOPIC", help = "Topic name or ID")]
  pub topic: String,

  #[arg(long, help = "New topic name")]
  pub name: Option<String>,

  #[command(flatten)]
  pub fields: TopicFields,

  #[arg(long, value_enum, value_name = "FIELD", help = "Clear a field")]
  pub clear: Vec<ClearField>,
}

#[derive(Debug, Args)]
pub struct TopicFields {
  #[arg(long, help = "Description shown on the preference page")]
  pub description: Option<String>,

  #[arg(long, value_name = "BOOL", help = "Subscribe new contacts until they choose")]
  pub default_subscribed: Option<bool>,

  #[arg(long, value_enum, help = "Show or hide the topic on the preference page")]
  pub visibility: Option<Visibility>,

  #[arg(long, value_name = "BOOL", help = "Ask form sign-ups to confirm by email")]
  pub double_opt_in: Option<bool>,

  #[arg(long, value_name = "BOOL", help = "Turn the hosted sign-up form on or off")]
  pub form_enabled: Option<bool>,

  #[arg(long, help = "From name for confirmation emails")]
  pub sender_name: Option<String>,

  #[arg(long, help = "From address for confirmation emails")]
  pub sender_email: Option<String>,

  #[arg(long, value_name = "URL", help = "Page to show after someone signs up")]
  pub redirect_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Visibility {
  Public,
  Private,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ClearField {
  Description,
  SenderName,
  SenderEmail,
  RedirectUrl,
}

pub async fn run(command: TopicsCommand, context: &Context) -> Result<()> {
  match command {
    TopicsCommand::List(args) => list(&args, context).await,
    TopicsCommand::Inspect(args) => inspect(&args, context).await,
    TopicsCommand::Create(args) => create(args, context).await,
    TopicsCommand::Update(args) => update(args, context).await,
    TopicsCommand::Delete(args) => delete(&args, context).await,
  }
}

async fn list(args: &ListArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading topics…");
  let page = api.list(&["v1", "topics"], &(), &args.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "topics", None)
}

async fn inspect(args: &TopicArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the topic…");
  let id = resolve_topic_id(&api, &args.topic).await?;
  let topic = api.get(&["v1", "topics", &id], &()).await?;

  context.output.print_detail(&topic, &DETAIL_ROWS)
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Creating the topic…");
  let brand_id = match &args.brand {
    Some(brand) => Some(resolve_brand_id(&api, brand).await?),
    None => None,
  };
  let fields = args.fields;
  let body = CreateTopic {
    name: args.name,
    description: fields.description,
    default_subscribed: fields.default_subscribed,
    visibility: fields.visibility.map(to_topic_visibility),
    double_opt_in: fields.double_opt_in,
    form_enabled: fields.form_enabled,
    sender_name: fields.sender_name,
    sender_email: fields.sender_email,
    redirect_url: fields.redirect_url,
    brand_id,
  };
  let topic = api.post(&["v1", "topics"], &body).await?;

  let headline = format!("Created {}", get_name(&topic));
  context.output.print_result(&topic, &headline, "id", &[])
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  let body = build_update(args.name, args.fields, &args.clear);
  if body == UpdateTopic::default() {
    return Err(Error::usage("missing_input", "Add an option to change the topic."));
  }

  let api = context.api().await?.loading("Saving the topic…");
  let id = resolve_topic_id(&api, &args.topic).await?;
  let topic = api.patch(&["v1", "topics", &id], &body).await?;

  let headline = format!("Updated {}", get_name(&topic));
  context.output.print_result(&topic, &headline, "id", &[])
}

async fn delete(args: &TopicArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Deleting the topic…");
  let id = resolve_topic_id(&api, &args.topic).await?;
  let topic = api.get(&["v1", "topics", &id], &()).await?;
  let name = get_name(&topic).to_owned();

  context.confirm_delete(
    &format!("Delete {name}?"),
    "Contacts lose this subscription choice.",
  )?;

  api.delete(&["v1", "topics", &id]).await?;
  let result = json!({ "id": id, "name": name, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {name}"), "id", &[])
}

pub async fn resolve_topic_id(api: &Api, selector: &str) -> Result<String> {
  if selector.starts_with("top_") {
    return Ok(selector.to_owned());
  }

  let page = api
    .list(&["v1", "topics"], &(), &Page { all: true, ..Default::default() })
    .await?;
  find_id_by_name(&page, selector).ok_or_else(|| {
    Error::failed("not_found", format!("No topic is named {selector}."))
      .with_next("inboxjot topics list", "List the topics")
  })
}

pub fn find_id_by_name(page: &Value, name: &str) -> Option<String> {
  page
    .get("data")
    .and_then(Value::as_array)?
    .iter()
    .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
    .and_then(|item| item.get("id").and_then(Value::as_str))
    .map(str::to_owned)
}

fn build_update(
  name: Option<String>,
  fields: TopicFields,
  clear: &[ClearField],
) -> UpdateTopic {
  let clearable = |value: Option<String>, field: ClearField| {
    if clear.contains(&field) { Some(None) } else { value.map(Some) }
  };

  UpdateTopic {
    name,
    description: clearable(fields.description, ClearField::Description),
    default_subscribed: fields.default_subscribed,
    visibility: fields.visibility.map(to_topic_visibility),
    double_opt_in: fields.double_opt_in,
    form_enabled: fields.form_enabled,
    sender_name: clearable(fields.sender_name, ClearField::SenderName),
    sender_email: clearable(fields.sender_email, ClearField::SenderEmail),
    redirect_url: clearable(fields.redirect_url, ClearField::RedirectUrl),
  }
}

fn to_topic_visibility(visibility: Visibility) -> TopicVisibility {
  match visibility {
    Visibility::Public => TopicVisibility::Public,
    Visibility::Private => TopicVisibility::Private,
  }
}

fn get_name(topic: &Value) -> &str {
  topic.get("name").and_then(Value::as_str).unwrap_or_default()
}
