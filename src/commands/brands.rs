use clap::{Args, Subcommand};
use inboxjot::BrandInput;
use serde_json::{Value, json};

use crate::api::{Api, Page};
use crate::cli::ListArgs;
use crate::commands::topics::find_id_by_name;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column};

const LIST_COLUMNS: [Column; 4] = [
  column("Name", "name"),
  column("Default", "is_default"),
  column("Topics", "topic_count"),
  column("Created", "created_at"),
];

const DETAIL_ROWS: [Column; 8] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Slug", "slug"),
  column("Default", "is_default"),
  column("Logo", "logo_url"),
  column("Topics", "topic_count"),
  column("Created", "created_at"),
  column("Updated", "updated_at"),
];

#[derive(Debug, Subcommand)]
pub enum BrandsCommand {
  #[command(
    about = "List brands",
    after_help = examples!("inboxjot brands list"),
    alias = "ls",
  )]
  List(ListArgs),
  #[command(
    about = "Show a brand",
    after_help = examples!("inboxjot brands inspect brd_123"),
  )]
  Inspect(BrandArgs),
  #[command(
    about = "Create a brand",
    after_help = examples!("inboxjot brands create --name \"Acme Labs\""),
  )]
  Create(CreateArgs),
  #[command(
    about = "Rename a brand",
    after_help = examples!("inboxjot brands update brd_123 --name \"Acme Studio\""),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Delete a brand",
    after_help = examples!("inboxjot brands delete brd_123"),
    alias = "rm",
  )]
  Delete(BrandArgs),
}

#[derive(Debug, Args)]
pub struct BrandArgs {
  #[arg(value_name = "BRAND", help = "Brand name or ID")]
  pub brand: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Brand name that readers see, such as \"Acme Labs\"")]
  pub name: String,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(value_name = "BRAND", help = "Brand name or ID")]
  pub brand: String,

  #[arg(long, help = "New brand name")]
  pub name: String,
}

pub async fn run(command: BrandsCommand, context: &Context) -> Result<()> {
  match command {
    BrandsCommand::List(args) => list(&args, context).await,
    BrandsCommand::Inspect(args) => inspect(&args, context).await,
    BrandsCommand::Create(args) => create(args, context).await,
    BrandsCommand::Update(args) => update(args, context).await,
    BrandsCommand::Delete(args) => delete(&args, context).await,
  }
}

async fn list(args: &ListArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading brands…");
  let page = api.list(&["v1", "brands"], &(), &args.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "brands", None)
}

async fn inspect(args: &BrandArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the brand…");
  let id = resolve_brand_id(&api, &args.brand).await?;
  let brand = api.get(&["v1", "brands", &id], &()).await?;

  context.output.print_detail(&brand, &DETAIL_ROWS)
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Creating the brand…");
  let body = BrandInput { name: args.name };
  let brand = api.post(&["v1", "brands"], &body).await?;

  let headline = format!("Created {}", get_name(&brand));
  context.output.print_result(&brand, &headline, "id", &[])
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Saving the brand…");
  let id = resolve_brand_id(&api, &args.brand).await?;
  let body = BrandInput { name: args.name };
  let brand = api.patch(&["v1", "brands", &id], &body).await?;

  let headline = format!("Updated {}", get_name(&brand));
  context.output.print_result(&brand, &headline, "id", &[])
}

async fn delete(args: &BrandArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Deleting the brand…");
  let id = resolve_brand_id(&api, &args.brand).await?;
  let brand = api.get(&["v1", "brands", &id], &()).await?;
  let name = get_name(&brand).to_owned();

  context
    .confirm_delete(&format!("Delete {name}?"), "Contacts lose this brand choice.")?;

  api.delete(&["v1", "brands", &id]).await?;
  let result = json!({ "id": id, "name": name, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {name}"), "id", &[])
}

pub async fn resolve_brand_id(api: &Api, selector: &str) -> Result<String> {
  if selector.starts_with("brd_") {
    return Ok(selector.to_owned());
  }

  let page = api
    .list(&["v1", "brands"], &(), &Page { all: true, ..Default::default() })
    .await?;
  find_id_by_name(&page, selector).ok_or_else(|| {
    Error::failed("not_found", format!("No brand is named {selector}."))
      .with_next("inboxjot brands list", "List the brands")
  })
}

fn get_name(brand: &Value) -> &str {
  brand.get("name").and_then(Value::as_str).unwrap_or_default()
}
