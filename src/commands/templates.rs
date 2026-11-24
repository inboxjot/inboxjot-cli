use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Subcommand};
use inboxjot::{CreateTemplate, ListTemplates, UpdateTemplate};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column};

const LIST_COLUMNS: [Column; 5] = [
  column("ID", "id"),
  column("Name", "name"),
  column("Subject", "subject"),
  column("Sent", "stats.sent"),
  column("Updated", "updated_at"),
];

const DETAIL_ROWS: [Column; 10] = [
  column("Name", "name"),
  column("ID", "id"),
  column("From", "from"),
  column("Reply to", "reply_to"),
  column("Subject", "subject"),
  column("Preheader", "preheader"),
  column("Published", "published_version_id"),
  column("Draft changes", "has_unpublished_changes"),
  column("Created", "created_at"),
  column("Updated", "updated_at"),
];

const VERSION_COLUMNS: [Column; 4] = [
  column("ID", "id"),
  column("Name", "name"),
  column("Reason", "reason"),
  column("Created", "created_at"),
];

#[derive(Debug, Subcommand)]
pub enum TemplatesCommand {
  #[command(
    about = "List templates",
    after_help = examples!("inboxjot templates list"),
    alias = "ls",
  )]
  List(ListTemplatesArgs),
  #[command(
    about = "Show a template",
    after_help = examples!("inboxjot templates inspect tpl_06d2mfk8r5cx3nd8vt1qa"),
  )]
  Inspect(TemplateArgs),
  #[command(
    about = "Create a template",
    after_help = examples!("inboxjot templates create --name Welcome --subject \"Welcome to Acme\" --html-file welcome.html"),
  )]
  Create(CreateArgs),
  #[command(
    about = "Change the draft of a template",
    after_help = examples!("inboxjot templates update tpl_06d2mfk8r5cx3nd8vt1qa --html-file welcome.html"),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Print the draft HTML of a template",
    after_help = examples!(
      "inboxjot templates preview tpl_06d2mfk8r5cx3nd8vt1qa --open",
      "inboxjot templates preview tpl_06d2mfk8r5cx3nd8vt1qa --text",
    ),
  )]
  Preview(PreviewArgs),
  #[command(
    about = "Publish the draft so sends use it",
    after_help = examples!("inboxjot templates publish tpl_06d2mfk8r5cx3nd8vt1qa"),
  )]
  Publish(TemplateArgs),
  #[command(subcommand, about = "List and restore saved versions of a template")]
  Versions(VersionsCommand),
  #[command(
    about = "Delete a template",
    after_help = examples!("inboxjot templates delete tpl_06d2mfk8r5cx3nd8vt1qa"),
    alias = "rm",
  )]
  Delete(TemplateArgs),
}

#[derive(Debug, Subcommand)]
pub enum VersionsCommand {
  #[command(
    about = "List the saved versions of a template",
    after_help = examples!("inboxjot templates versions list tpl_06d2mfk8r5cx3nd8vt1qa"),
    alias = "ls",
  )]
  List(VersionsListArgs),
  #[command(
    about = "Copy a version into the draft of a template",
    after_help = examples!("inboxjot templates versions restore tpl_06d2mfk8r5cx3nd8vt1qa ver_06d2mfk8r5cx3nd8vt1qb"),
  )]
  Restore(RestoreArgs),
}

#[derive(Debug, Args)]
pub struct ListTemplatesArgs {
  #[arg(long, help = "Show only templates whose name has this text")]
  pub query: Option<String>,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct TemplateArgs {
  #[arg(
    value_name = "TEMPLATE",
    help = "Template ID, such as tpl_06d2mfk8r5cx3nd8vt1qa"
  )]
  pub template: String,
}

#[derive(Debug, Args)]
pub struct ContentArgs {
  #[arg(long, help = "From address, such as \"Acme <hello@acme.com>\"")]
  pub from: Option<String>,

  #[arg(long, value_name = "EMAIL", help = "Reply-to address")]
  pub reply_to: Option<String>,

  #[arg(long, help = "Preview text that shows after the subject")]
  pub preheader: Option<String>,

  #[arg(
    long,
    value_name = "PATH",
    help = "Read the HTML from this file, or - for stdin"
  )]
  pub html_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Template name")]
  pub name: String,

  #[arg(long, help = "Subject line")]
  pub subject: String,

  #[command(flatten)]
  pub content: ContentArgs,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(
    value_name = "TEMPLATE",
    help = "Template ID, such as tpl_06d2mfk8r5cx3nd8vt1qa"
  )]
  pub template: String,

  #[arg(long, help = "Template name")]
  pub name: Option<String>,

  #[arg(long, help = "Subject line")]
  pub subject: Option<String>,

  #[command(flatten)]
  pub content: ContentArgs,
}

#[derive(Debug, Args)]
pub struct PreviewArgs {
  #[arg(
    value_name = "TEMPLATE",
    help = "Template ID, such as tpl_06d2mfk8r5cx3nd8vt1qa"
  )]
  pub template: String,

  #[arg(long, help = "Print the plain text instead of the HTML")]
  pub text: bool,

  #[arg(long, conflicts_with = "text", help = "Open the HTML in your browser")]
  pub open: bool,
}

#[derive(Debug, Args)]
pub struct VersionsListArgs {
  #[arg(
    value_name = "TEMPLATE",
    help = "Template ID, such as tpl_06d2mfk8r5cx3nd8vt1qa"
  )]
  pub template: String,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
  #[arg(
    value_name = "TEMPLATE",
    help = "Template ID, such as tpl_06d2mfk8r5cx3nd8vt1qa"
  )]
  pub template: String,

  #[arg(value_name = "VERSION", help = "Version ID, such as ver_06d2mfk8r5cx3nd8vt1qb")]
  pub version_id: String,
}

pub async fn run(command: TemplatesCommand, context: &Context) -> Result<()> {
  match command {
    TemplatesCommand::List(args) => list(&args, context).await,
    TemplatesCommand::Inspect(args) => inspect(&args, context).await,
    TemplatesCommand::Create(args) => create(args, context).await,
    TemplatesCommand::Update(args) => update(args, context).await,
    TemplatesCommand::Preview(args) => preview(&args, context).await,
    TemplatesCommand::Publish(args) => publish(&args, context).await,
    TemplatesCommand::Versions(VersionsCommand::List(args)) => {
      list_versions("templates", &args.template, &args.list, context).await
    }
    TemplatesCommand::Versions(VersionsCommand::Restore(args)) => {
      restore_version("templates", &args.template, &args.version_id, context).await
    }
    TemplatesCommand::Delete(args) => {
      delete_document("templates", "template", &args.template, context).await
    }
  }
}

async fn list(args: &ListTemplatesArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading templates…");
  let query = ListTemplates { query: args.query.clone(), ..Default::default() };
  let page = api.list(&["v1", "templates"], &query, &args.list.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "templates", None)
}

async fn inspect(args: &TemplateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the template…");
  let template = api.get(&["v1", "templates", &args.template], &()).await?;

  context.output.print_detail(&template, &DETAIL_ROWS)
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Creating the template…");
  let body = CreateTemplate {
    name: args.name,
    from: args.content.from,
    reply_to: args.content.reply_to.map(Some),
    subject: Some(args.subject),
    preheader: args.content.preheader,
    html: args.content.html_file.as_deref().map(read_text_file).transpose()?,
    ..Default::default()
  };
  let template = api.post(&["v1", "templates"], &body).await?;

  let name = get_name(&template);
  context
    .output
    .print_result(&template, &format!("Created {name}"), "id", &[])?;
  context.output.next(
    "Publish it so sends can use it:",
    &format!("inboxjot templates publish {}", get_id(&template)),
  );
  Ok(())
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  let body = UpdateTemplate {
    name: args.name,
    from: args.content.from,
    reply_to: args.content.reply_to.map(Some),
    subject: args.subject,
    preheader: args.content.preheader,
    html: args.content.html_file.as_deref().map(read_text_file).transpose()?,
    ..Default::default()
  };
  require_change(&body)?;

  let api = context.api().await?.loading("Saving the template…");
  let template = api.patch(&["v1", "templates", &args.template], &body).await?;
  let name = get_name(&template);
  context
    .output
    .print_result(&template, &format!("Updated {name}"), "id", &[])
}

async fn preview(args: &PreviewArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Rendering the preview…");
  let template = api.get(&["v1", "templates", &args.template], &()).await?;

  if context.output.is_json() {
    return context.output.print_json(&template);
  }

  let html = template.get("html").and_then(Value::as_str).unwrap_or_default();
  if args.open {
    return open_in_browser(&args.template, html, context);
  }

  let field = if args.text { "text" } else { "html" };
  let body = template.get(field).and_then(Value::as_str).unwrap_or_default();
  context.output.print_line(body);
  Ok(())
}

fn open_in_browser(id: &str, html: &str, context: &Context) -> Result<()> {
  let path = std::env::temp_dir().join(format!("inboxjot-{id}.html"));
  std::fs::write(&path, html)?;
  webbrowser::open(&path.to_string_lossy())?;

  context.output.success("Opened the preview in your browser");
  context.output.info(&path.display().to_string());
  Ok(())
}

async fn publish(args: &TemplateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Publishing the template…");
  let template = api.get(&["v1", "templates", &args.template], &()).await?;
  let name = get_name(&template).to_owned();

  context.confirm(&format!(
    "Publish the template {name}? Sends that use it switch to this version."
  ))?;

  let published = api.post(&["v1", "templates", &args.template, "publish"], &()).await?;
  context
    .output
    .print_result(&published, &format!("Published {name}"), "id", &[])
}

pub async fn list_versions(
  resource: &str,
  id: &str,
  args: &ListArgs,
  context: &Context,
) -> Result<()> {
  let api = context.api().await?.loading("Loading versions…");
  let page = api
    .list(&["v1", resource, id, "versions"], &(), &args.to_page())
    .await?;

  context.output.print_list(&page, &VERSION_COLUMNS, "versions", None)
}

pub async fn restore_version(
  resource: &str,
  id: &str,
  version: &str,
  context: &Context,
) -> Result<()> {
  let api = context.api().await?.loading("Restoring the version…");
  let document = api
    .post(&["v1", resource, id, "versions", version, "restore"], &())
    .await?;

  let name = get_name(&document);
  context.output.print_result(
    &document,
    &format!("Restored {version} into the draft of {name}"),
    "id",
    &[],
  )
}

pub async fn delete_document(
  resource: &str,
  noun: &str,
  id: &str,
  context: &Context,
) -> Result<()> {
  let api = context.api().await?.loading(format!("Deleting the {noun}…"));
  let document = api.get(&["v1", resource, id], &()).await?;
  let name = get_name(&document).to_owned();

  let consequence = match noun {
    "template" => "Sends that name it by ID start to fail.",
    _ => "This cannot be undone.",
  };
  context.confirm_delete(&format!("Delete {name}?"), consequence)?;

  api.delete(&["v1", resource, id]).await?;
  let result = json!({ "id": id, "name": name, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {name}"), "id", &[])
}

pub fn read_text_file(path: &std::path::Path) -> Result<String> {
  let mut text = String::new();
  let read = if path.as_os_str() == "-" {
    std::io::stdin().read_to_string(&mut text).map(|_| ())
  } else {
    std::fs::read_to_string(path).map(|file| text = file)
  };

  read.map(|()| text).map_err(|error| {
    Error::usage("unreadable_file", format!("Could not read {}: {error}", path.display()))
  })
}

pub fn require_change(body: &impl Serialize) -> Result<()> {
  if serde_json::to_value(body)? != json!({}) {
    return Ok(());
  }

  Err(Error::usage("missing_input", "Add at least one option to change."))
}

pub fn get_name(document: &Value) -> &str {
  document.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn get_id(document: &Value) -> &str {
  document.get("id").and_then(Value::as_str).unwrap_or_default()
}
