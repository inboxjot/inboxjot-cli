mod export;
mod import;

use std::time::{Duration, Instant};

use clap::{Args, Subcommand, ValueEnum};
use inboxjot::{
  BrandChoice, BrandStatus, ContactStatus, CreateContact, EraseContact, ListContacts,
  TopicChoice, TopicStatus, UpdateContact,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::api::Api;
use crate::cli::ListArgs;
use crate::commands::brands::resolve_brand_id;
use crate::commands::segments::resolve_segment_id;
use crate::commands::topics::resolve_topic_id;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, Progress, column, status_column};
use crate::ui::spinner::get_name_or;

const LIST_COLUMNS: [Column; 5] = [
  column("Email", "email"),
  column("First name", "first_name"),
  column("Last name", "last_name"),
  status_column("Status", "status"),
  column("Added", "created_at"),
];

const DETAIL_ROWS: [Column; 11] = [
  column("Email", "email"),
  column("ID", "id"),
  column("First name", "first_name"),
  column("Last name", "last_name"),
  status_column("Status", "status"),
  column("Language", "language"),
  column("Segments", "segment_names"),
  column("Topics", "topic_names"),
  column("Brands", "brand_names"),
  column("Added", "created_at"),
  column("Updated", "updated_at"),
];

const POLL_INTERVAL: Duration = Duration::from_secs(1);

const PROPERTY_COLUMNS: [Column; 2] =
  [column("Property", "key"), column("Value", "value")];

#[derive(Debug, Subcommand)]
pub enum ContactsCommand {
  #[command(
    about = "List contacts",
    after_help = examples!(
      "inboxjot contacts list --status subscribed",
      "inboxjot contacts list --segment seg_123 --all --json",
    ),
    alias = "ls",
  )]
  List(ListContactsArgs),
  #[command(
    about = "Show a contact",
    after_help = examples!("inboxjot contacts inspect riti@example.com"),
  )]
  Inspect(ContactArgs),
  #[command(
    about = "Add a contact, or change the contact with that email",
    after_help = examples!(
      "inboxjot contacts create --email riti@example.com --first-name Riti --property plan=pro",
      "inboxjot contacts create --email sukha@example.com --subscribed true --segment seg_123",
    ),
  )]
  Create(CreateArgs),
  #[command(
    about = "Change a contact",
    after_help = examples!(
      "inboxjot contacts update riti@example.com --property plan=team",
      "inboxjot contacts update riti@example.com --subscribe-to top_123",
      "inboxjot contacts update riti@example.com --unsubscribe-from-brand \"Acme Labs\"",
    ),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Delete a contact",
    after_help = examples!("inboxjot contacts delete riti@example.com"),
    alias = "rm",
  )]
  Delete(ContactArgs),
  #[command(
    about = "Erase all data for an email address",
    after_help = examples!("inboxjot contacts erase riti@example.com --yes"),
  )]
  Erase(EraseArgs),
  #[command(
    about = "Import contacts from a CSV file",
    after_help = examples!(
      "inboxjot contacts import customers.csv --dry-run",
      "inboxjot contacts import customers.csv --segment seg_123",
    ),
  )]
  Import(import::ImportArgs),
  #[command(
    about = "Export contacts to a CSV file",
    after_help = examples!("inboxjot contacts export --output contacts.csv"),
  )]
  Export(export::ExportArgs),
}

#[derive(Debug, Args)]
pub struct ListContactsArgs {
  #[arg(long, help = "Show contacts whose name or email matches this text")]
  pub query: Option<String>,

  #[arg(long, value_name = "SEGMENT", help = "Show only contacts in this segment")]
  pub segment: Option<String>,

  #[arg(long, value_enum, help = "Show only contacts with this status")]
  pub status: Option<Status>,

  #[command(flatten)]
  pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct ContactArgs {
  #[arg(value_name = "CONTACT", help = "Contact ID or email")]
  pub contact: String,
}

#[derive(Debug, Args)]
pub struct EraseArgs {
  #[arg(value_name = "EMAIL", help = "Email address to erase")]
  pub email: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Email address of the contact")]
  pub email: String,

  #[command(flatten)]
  pub fields: ContactFields,

  #[arg(
    long = "segment",
    value_name = "SEGMENT",
    help = "Add the contact to this static segment"
  )]
  pub segments: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(value_name = "CONTACT", help = "Contact ID or email")]
  pub contact: String,

  #[command(flatten)]
  pub fields: ContactFields,

  #[arg(long, value_name = "TOPIC", help = "Subscribe the contact to this topic")]
  pub subscribe_to: Vec<String>,

  #[arg(long, value_name = "TOPIC", help = "Unsubscribe the contact from this topic")]
  pub unsubscribe_from: Vec<String>,

  #[arg(long, value_name = "BRAND", help = "Subscribe the contact to this brand")]
  pub subscribe_to_brand: Vec<String>,

  #[arg(
    long,
    value_name = "BRAND",
    help = "Unsubscribe the contact from this brand and all its topics"
  )]
  pub unsubscribe_from_brand: Vec<String>,

  #[arg(long, value_enum, value_name = "FIELD", help = "Clear a field")]
  pub clear: Vec<ClearField>,
}

#[derive(Debug, Args)]
pub struct ContactFields {
  #[arg(long, help = "First name")]
  pub first_name: Option<String>,

  #[arg(long, help = "Last name")]
  pub last_name: Option<String>,

  #[arg(long, help = "Language code, such as en or pt-BR")]
  pub language: Option<String>,

  #[arg(long, help = "Private notes")]
  pub notes: Option<String>,

  #[arg(
    long = "property",
    value_name = "KEY=VALUE",
    value_parser = parse_property,
    help = "Set a property, such as plan=pro; numbers, true, false and null keep their type"
  )]
  pub properties: Vec<(String, Value)>,

  #[arg(long, value_name = "BOOL", help = "Subscribe to or unsubscribe from all email")]
  pub subscribed: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Status {
  Subscribed,
  Pending,
  NotSubscribed,
  Unsubscribed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ClearField {
  FirstName,
  LastName,
  Notes,
}

pub async fn run(command: ContactsCommand, context: &Context) -> Result<()> {
  match command {
    ContactsCommand::List(args) => list(args, context).await,
    ContactsCommand::Inspect(args) => inspect(&args, context).await,
    ContactsCommand::Create(args) => create(args, context).await,
    ContactsCommand::Update(args) => update(args, context).await,
    ContactsCommand::Delete(args) => delete(&args, context).await,
    ContactsCommand::Erase(args) => erase(&args, context).await,
    ContactsCommand::Import(args) => import::run(args, context).await,
    ContactsCommand::Export(args) => export::run(args, context).await,
  }
}

async fn list(args: ListContactsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading contacts…");
  let segment_id = match &args.segment {
    Some(segment) => Some(resolve_segment_id(&api, segment).await?),
    None => None,
  };
  let query = ListContacts {
    status: args.status.map(to_contact_status),
    segment_id,
    query: args.query,
    ..Default::default()
  };
  let page = api.list(&["v1", "contacts"], &query, &args.page.to_page()).await?;

  context.output.print_list(
    &page,
    &LIST_COLUMNS,
    "contacts",
    Some(("add them from a CSV", "inboxjot contacts import <file>")),
  )
}

async fn inspect(args: &ContactArgs, context: &Context) -> Result<()> {
  let api = context
    .api()
    .await?
    .loading(format!("Loading {}…", get_name_or(&args.contact, "the contact")));
  let contact = api.get(&["v1", "contacts", &args.contact], &()).await?;
  if context.output.is_json() {
    return context.output.print_json(&contact);
  }

  context
    .output
    .print_detail(&add_display_names(&contact), &DETAIL_ROWS)?;
  let properties = get_property_rows(&contact);
  if !properties.is_empty() {
    context.output.print_line("");
    context.output.print_table(&properties, &PROPERTY_COLUMNS);
  }

  Ok(())
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Creating the contact…");
  let mut segment_ids = Vec::with_capacity(args.segments.len());
  for segment in &args.segments {
    segment_ids.push(resolve_segment_id(&api, segment).await?);
  }

  let fields = args.fields;
  let body = CreateContact {
    email: args.email,
    user_id: None,
    first_name: fields.first_name,
    last_name: fields.last_name,
    language: fields.language,
    notes: fields.notes,
    properties: to_properties(fields.properties),
    subscribed: fields.subscribed,
    segment_ids: (!segment_ids.is_empty()).then_some(segment_ids),
  };
  let contact = api.post(&["v1", "contacts"], &body).await?;

  let headline = format!("Saved {}", get_email(&contact));
  context
    .output
    .print_result(&contact, &headline, "id", &[column("Status", "status")])
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  let api = context
    .api()
    .await?
    .loading(format!("Saving {}…", get_name_or(&args.contact, "the contact")));
  let mut topics = Vec::new();
  for (selectors, status) in [
    (&args.subscribe_to, TopicStatus::Subscribed),
    (&args.unsubscribe_from, TopicStatus::Unsubscribed),
  ] {
    for selector in selectors {
      topics.push(TopicChoice {
        id: resolve_topic_id(&api, selector).await?,
        status,
      });
    }
  }

  let mut brands = Vec::new();
  for (selectors, status) in [
    (&args.subscribe_to_brand, BrandStatus::Subscribed),
    (&args.unsubscribe_from_brand, BrandStatus::Unsubscribed),
  ] {
    for selector in selectors {
      brands.push(BrandChoice {
        id: resolve_brand_id(&api, selector).await?,
        status,
      });
    }
  }

  let body = build_update(args.fields, &args.clear, topics, brands);
  if body == UpdateContact::default() {
    return Err(Error::usage("missing_input", "Add an option to change the contact."));
  }

  let contact = api.patch(&["v1", "contacts", &args.contact], &body).await?;

  let headline = format!("Updated {}", get_email(&contact));
  context.output.print_result(&contact, &headline, "id", &[])
}

async fn delete(args: &ContactArgs, context: &Context) -> Result<()> {
  let api = context
    .api()
    .await?
    .loading(format!("Deleting {}…", get_name_or(&args.contact, "the contact")));
  let contact = api.get(&["v1", "contacts", &args.contact], &()).await?;
  let email = get_email(&contact).to_owned();
  let id = contact
    .get("id")
    .and_then(Value::as_str)
    .unwrap_or_default()
    .to_owned();

  context.confirm_delete(
    &format!("Delete {email}?"),
    "The emails already sent to them stay in the logs.",
  )?;

  api.delete(&["v1", "contacts", &id]).await?;
  let result = json!({ "id": id, "email": email, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {email}"), "id", &[])
}

async fn erase(args: &EraseArgs, context: &Context) -> Result<()> {
  let email = &args.email;
  let consequence = format!("This erases all data for {email}.");
  if context.interactive && !context.yes {
    context.output.info(&format!("{consequence} This cannot be undone."));
  }
  context.confirm_name("contact", email, &consequence)?;

  let api = context.api().await?.loading(format!("Erasing {email}…"));
  let body = EraseContact { email: email.clone() };
  let result = api.post(&["v1", "contacts", "erase"], &body).await?;

  context
    .output
    .print_result(&result, &format!("Queued the erase of {email}"), "id", &[])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
  Queued,
  Importing,
  Exporting,
  Completed,
  Failed,
  #[serde(other)]
  Unknown,
}

#[derive(Debug, Deserialize)]
struct Job {
  id: String,
  status: JobStatus,
}

pub struct Activity {
  pub done: u64,
  pub total: Option<u64>,
}

async fn wait_for_job(
  api: &Api,
  segments: &[&str],
  mut job: Value,
  describe: &dyn Fn(&Value) -> Activity,
  timeout: Duration,
  progress: &Progress,
) -> Result<(Value, JobStatus)> {
  let started = Instant::now();

  loop {
    let activity = describe(&job);
    progress.update(activity.done, activity.total);
    let status = Job::deserialize(&job)?.status;
    match status {
      JobStatus::Completed | JobStatus::Failed => return Ok((job, status)),
      JobStatus::Queued | JobStatus::Importing | JobStatus::Exporting => {}
      JobStatus::Unknown => {
        return Err(Error::failed(
          "unknown_status",
          format!(
            "The job has the status {}, which this CLI does not know.",
            job["status"]
          ),
        ));
      }
    }

    if started.elapsed() >= timeout {
      return Err(Error::failed("timeout", "The job did not finish in time."));
    }

    tokio::time::sleep(POLL_INTERVAL).await;
    job = api.get(segments, &()).await?;
  }
}

fn build_update(
  fields: ContactFields,
  clear: &[ClearField],
  topics: Vec<TopicChoice>,
  brands: Vec<BrandChoice>,
) -> UpdateContact {
  let clearable = |value: Option<String>, field: ClearField| {
    if clear.contains(&field) { Some(None) } else { value.map(Some) }
  };

  UpdateContact {
    user_id: None,
    first_name: clearable(fields.first_name, ClearField::FirstName),
    last_name: clearable(fields.last_name, ClearField::LastName),
    language: fields.language,
    notes: clearable(fields.notes, ClearField::Notes),
    properties: to_properties(fields.properties),
    subscribed: fields.subscribed,
    segment_ids: None,
    topics: (!topics.is_empty()).then_some(topics),
    brands: (!brands.is_empty()).then_some(brands),
  }
}

fn parse_property(text: &str) -> Result<(String, Value), String> {
  let (key, raw) =
    text
      .split_once('=')
      .filter(|(key, _)| !key.is_empty())
      .ok_or_else(|| {
        format!("\"{text}\" is not a property. Use KEY=VALUE, such as plan=pro")
      })?;
  let value = match serde_json::from_str::<Value>(raw) {
    Ok(value @ (Value::Number(_) | Value::Bool(_) | Value::Null)) => value,
    _ => Value::String(raw.to_owned()),
  };

  Ok((key.to_owned(), value))
}

fn to_properties(properties: Vec<(String, Value)>) -> Option<Map<String, Value>> {
  (!properties.is_empty()).then(|| properties.into_iter().collect())
}

fn add_display_names(contact: &Value) -> Value {
  let names = |key: &str, is_shown: fn(&Value) -> bool| {
    let items = contact.get(key).and_then(Value::as_array);
    let names = items.into_iter().flatten().filter(|item| is_shown(item));
    Value::Array(names.filter_map(|item| item.get("name").cloned()).collect())
  };

  let mut display = contact.clone();
  display["segment_names"] = names("segments", |_| true);
  display["topic_names"] =
    names("topics", |topic| topic.get("subscribed") == Some(&Value::Bool(true)));
  display["brand_names"] = names("brands", |brand| {
    brand.get("status").and_then(Value::as_str) == Some("subscribed")
  });
  display
}

fn get_property_rows(contact: &Value) -> Vec<Value> {
  let properties = contact.get("properties").and_then(Value::as_object);
  properties
    .into_iter()
    .flatten()
    .map(|(key, value)| json!({ "key": key, "value": value }))
    .collect()
}

fn to_contact_status(status: Status) -> ContactStatus {
  match status {
    Status::Subscribed => ContactStatus::Subscribed,
    Status::Pending => ContactStatus::Pending,
    Status::NotSubscribed => ContactStatus::NotSubscribed,
    Status::Unsubscribed => ContactStatus::Unsubscribed,
  }
}

fn get_email(contact: &Value) -> &str {
  contact.get("email").and_then(Value::as_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use serde_json::{Value, json};

  use super::parse_property;

  #[test]
  fn types_property_values() {
    let parse = |text| parse_property(text).unwrap().1;

    assert_eq!(parse("plan=pro"), json!("pro"));
    assert_eq!(parse("seats=3"), json!(3));
    assert_eq!(parse("vip=true"), json!(true));
    assert_eq!(parse("plan=null"), Value::Null);
    assert_eq!(parse("zip=02134"), json!("02134"));
    assert_eq!(parse("note=a=b"), json!("a=b"));
    assert!(parse_property("=x").is_err());
    assert!(parse_property("plan").is_err());
  }
}
