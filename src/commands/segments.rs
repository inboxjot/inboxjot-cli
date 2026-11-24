use std::io::Read;

use clap::{Args, Subcommand, ValueEnum};
use inboxjot::{
  CreateSegment, ListSegmentMembers, ListSegments, SegmentConditions, SegmentMembers,
  SegmentType, UpdateSegment,
};
use serde_json::{Value, json};

use crate::api::{Api, Page};
use crate::cli::ListArgs;
use crate::commands::topics::find_id_by_name;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column, format_count};

const MAX_MEMBERS_PER_REQUEST: usize = 1_000;

const LIST_COLUMNS: [Column; 4] = [
  column("Name", "name"),
  column("Type", "type"),
  column("Contacts", "contact_count"),
  column("Created", "created_at"),
];

const DETAIL_ROWS: [Column; 8] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Type", "type"),
  column("Description", "description"),
  column("Contacts", "contact_count"),
  column("Counted", "counted_at"),
  column("Created", "created_at"),
  column("Updated", "updated_at"),
];

const RULE_COLUMNS: [Column; 3] =
  [column("Field", "field"), column("Operator", "operator"), column("Value", "value")];

const MEMBER_COLUMNS: [Column; 5] = [
  column("Email", "email"),
  column("First name", "first_name"),
  column("Last name", "last_name"),
  column("Status", "status"),
  column("Joined", "joined_at"),
];

#[derive(Debug, Subcommand)]
pub enum SegmentsCommand {
  #[command(
    about = "List segments",
    after_help = examples!("inboxjot segments list --type dynamic"),
    alias = "ls",
  )]
  List(ListSegmentsArgs),
  #[command(
    about = "Show a segment and its rules",
    after_help = examples!("inboxjot segments inspect \"Engaged customers\""),
  )]
  Inspect(SegmentArgs),
  #[command(
    about = "Create a segment",
    after_help = examples!(
      "inboxjot segments create --name \"Beta testers\" --type static",
      "inboxjot segments create --name \"Pro plan\" --type dynamic --conditions @pro.json",
    ),
  )]
  Create(CreateArgs),
  #[command(
    about = "Change a segment",
    after_help = examples!("inboxjot segments update seg_123 --name \"Early customers\""),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Delete a segment",
    after_help = examples!("inboxjot segments delete seg_123"),
    alias = "rm",
  )]
  Delete(SegmentArgs),
  #[command(subcommand, about = "List, add and remove the contacts in a segment")]
  Members(MembersCommand),
}

#[derive(Debug, Subcommand)]
pub enum MembersCommand {
  #[command(
    about = "List the contacts in a segment",
    after_help = examples!("inboxjot segments members list \"Beta testers\""),
    alias = "ls",
  )]
  List(ListMembersArgs),
  #[command(
    about = "Add contacts to a static segment",
    after_help = examples!("inboxjot segments members add \"Beta testers\" riti@example.com"),
  )]
  Add(MembersArgs),
  #[command(
    about = "Remove contacts from a static segment",
    after_help = examples!("inboxjot segments members remove \"Beta testers\" riti@example.com"),
    alias = "rm",
  )]
  Remove(MembersArgs),
}

#[derive(Debug, Args)]
pub struct ListSegmentsArgs {
  #[arg(long = "type", value_enum, help = "Show only segments of this type")]
  pub kind: Option<Kind>,

  #[arg(long, help = "Show segments whose name matches this text")]
  pub query: Option<String>,

  #[command(flatten)]
  pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct SegmentArgs {
  #[arg(value_name = "SEGMENT", help = "Segment name or ID")]
  pub segment: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Segment name, such as \"Engaged customers\"")]
  pub name: String,

  #[arg(
    long = "type",
    value_enum,
    help = "Segment type [default: dynamic with --conditions, else static]"
  )]
  pub kind: Option<Kind>,

  #[arg(long, help = "Segment description")]
  pub description: Option<String>,

  #[arg(
    long,
    value_name = "JSON",
    help = "Rules of a dynamic segment as JSON, or @file to read them from a file"
  )]
  pub conditions: Option<String>,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(value_name = "SEGMENT", help = "Segment name or ID")]
  pub segment: String,

  #[arg(long, help = "New segment name")]
  pub name: Option<String>,

  #[arg(long, help = "New segment description")]
  pub description: Option<String>,

  #[arg(
    long,
    value_name = "JSON",
    help = "New rules as JSON, or @file to read them from a file"
  )]
  pub conditions: Option<String>,

  #[arg(long, value_enum, value_name = "FIELD", help = "Clear a field")]
  pub clear: Vec<ClearField>,
}

#[derive(Debug, Args)]
pub struct ListMembersArgs {
  #[arg(value_name = "SEGMENT", help = "Segment name or ID")]
  pub segment: String,

  #[arg(long, help = "Show members whose name or email matches this text")]
  pub query: Option<String>,

  #[command(flatten)]
  pub page: ListArgs,
}

#[derive(Debug, Args)]
pub struct MembersArgs {
  #[arg(value_name = "SEGMENT", help = "Segment name or ID")]
  pub segment: String,

  #[arg(
    value_name = "CONTACT",
    required = true,
    help = "Contact ID or email, or - to read one per line from stdin"
  )]
  pub contacts: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Kind {
  Static,
  Dynamic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ClearField {
  Description,
}

pub async fn run(command: SegmentsCommand, context: &Context) -> Result<()> {
  match command {
    SegmentsCommand::List(args) => list(args, context).await,
    SegmentsCommand::Inspect(args) => inspect(&args, context).await,
    SegmentsCommand::Create(args) => create(args, context).await,
    SegmentsCommand::Update(args) => update(args, context).await,
    SegmentsCommand::Delete(args) => delete(&args, context).await,
    SegmentsCommand::Members(MembersCommand::List(args)) => {
      list_members(args, context).await
    }
    SegmentsCommand::Members(MembersCommand::Add(args)) => {
      change_members(&args, true, context).await
    }
    SegmentsCommand::Members(MembersCommand::Remove(args)) => {
      change_members(&args, false, context).await
    }
  }
}

async fn list(args: ListSegmentsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading segments…");
  let query = ListSegments {
    kind: args.kind.map(to_segment_type),
    query: args.query,
    ..Default::default()
  };
  let page = api.list(&["v1", "segments"], &query, &args.page.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "segments", None)
}

async fn inspect(args: &SegmentArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the segment…");
  let id = resolve_segment_id(&api, &args.segment).await?;
  let segment = api.get(&["v1", "segments", &id], &()).await?;

  context.output.print_detail(&segment, &DETAIL_ROWS)?;
  let rules = segment
    .pointer("/conditions/rules")
    .and_then(Value::as_array)
    .map_or(&[][..], Vec::as_slice);
  if !context.output.is_json() && !rules.is_empty() {
    let matching = segment.pointer("/conditions/match").and_then(Value::as_str);
    context.output.print_line("");
    context
      .output
      .print_line(&format!("Match {} of these rules:", matching.unwrap_or("all")));
    context.output.print_table(rules, &RULE_COLUMNS);
  }

  Ok(())
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let conditions = args.conditions.as_deref().map(read_conditions).transpose()?;
  let kind = match (args.kind, &conditions) {
    (Some(kind), _) => to_segment_type(kind),
    (None, Some(_)) => SegmentType::Dynamic,
    (None, None) => SegmentType::Static,
  };
  let body = CreateSegment {
    name: args.name,
    kind,
    description: args.description,
    conditions,
  };

  let api = context.api().await?.loading("Creating the segment…");
  let segment = api.post(&["v1", "segments"], &body).await?;

  let headline = format!("Created {}", get_name(&segment));
  context
    .output
    .print_result(&segment, &headline, "id", &[column("Type", "type")])
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  let description = if args.clear.contains(&ClearField::Description) {
    Some(None)
  } else {
    args.description.map(Some)
  };
  let body = UpdateSegment {
    name: args.name,
    description,
    conditions: args.conditions.as_deref().map(read_conditions).transpose()?,
  };
  if body == UpdateSegment::default() {
    return Err(Error::usage("missing_input", "Add an option to change the segment."));
  }

  let api = context.api().await?.loading("Saving the segment…");
  let id = resolve_segment_id(&api, &args.segment).await?;
  let segment = api.patch(&["v1", "segments", &id], &body).await?;

  let headline = format!("Updated {}", get_name(&segment));
  context.output.print_result(&segment, &headline, "id", &[])
}

async fn delete(args: &SegmentArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Deleting the segment…");
  let id = resolve_segment_id(&api, &args.segment).await?;
  let segment = api.get(&["v1", "segments", &id], &()).await?;
  let name = get_name(&segment).to_owned();

  context
    .confirm_delete(&format!("Delete {name}?"), "Its contacts stay in the workspace.")?;

  api.delete(&["v1", "segments", &id]).await?;
  let result = json!({ "id": id, "name": name, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {name}"), "id", &[])
}

async fn list_members(args: ListMembersArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading members…");
  let id = resolve_segment_id(&api, &args.segment).await?;
  let query = ListSegmentMembers { query: args.query, ..Default::default() };
  let page = api
    .list(&["v1", "segments", &id, "members"], &query, &args.page.to_page())
    .await?;

  context.output.print_list(&page, &MEMBER_COLUMNS, "members", None)
}

async fn change_members(
  args: &MembersArgs,
  is_add: bool,
  context: &Context,
) -> Result<()> {
  let contacts = read_contacts(&args.contacts)?;
  let api = context.api().await?.loading(if is_add {
    "Adding members…"
  } else {
    "Removing members…"
  });
  let id = resolve_segment_id(&api, &args.segment).await?;
  let contact_ids = resolve_contact_ids(&api, &contacts).await?;

  let (key, verb, preposition) =
    if is_add { ("added", "Added", "to") } else { ("removed", "Removed", "from") };
  let mut total = 0;
  for chunk in contact_ids.chunks(MAX_MEMBERS_PER_REQUEST) {
    let body = SegmentMembers { contact_ids: chunk.to_vec() };
    let path = ["v1", "segments", &id, "members"];
    let result = if is_add {
      api.post(&path, &body).await?
    } else {
      api
        .send(reqwest::Method::DELETE, &path, &[], Some(serde_json::to_vec(&body)?))
        .await?
    };
    total += result.get(key).and_then(Value::as_u64).unwrap_or(0);
  }

  let noun = if total == 1 { "contact" } else { "contacts" };
  let headline =
    format!("{verb} {} {noun} {preposition} {}", format_count(total), args.segment);
  context
    .output
    .print_result(&json!({ key: total }), &headline, "id", &[])
}

pub async fn resolve_segment_id(api: &Api, selector: &str) -> Result<String> {
  if selector.starts_with("seg_") {
    return Ok(selector.to_owned());
  }

  let query = ListSegments {
    query: Some(selector.to_owned()),
    ..Default::default()
  };
  let page = api
    .list(&["v1", "segments"], &query, &Page { all: true, ..Default::default() })
    .await?;
  find_id_by_name(&page, selector).ok_or_else(|| {
    Error::failed("not_found", format!("No segment is named {selector}."))
      .with_next("inboxjot segments list", "List the segments")
  })
}

async fn resolve_contact_ids(api: &Api, contacts: &[String]) -> Result<Vec<String>> {
  let mut ids = Vec::with_capacity(contacts.len());
  for contact in contacts {
    if contact.starts_with("con_") {
      ids.push(contact.clone());
      continue;
    }

    let found = api.get(&["v1", "contacts", contact], &()).await?;
    let id = found.get("id").and_then(Value::as_str).unwrap_or_default();
    ids.push(id.to_owned());
  }

  Ok(ids)
}

fn read_contacts(args: &[String]) -> Result<Vec<String>> {
  let mut contacts = Vec::new();
  for arg in args {
    if arg != "-" {
      contacts.push(arg.clone());
      continue;
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    contacts.extend(parse_lines(&input));
  }

  if contacts.is_empty() {
    return Err(Error::usage("missing_input", "Add at least one contact."));
  }

  Ok(contacts)
}

fn parse_lines(input: &str) -> impl Iterator<Item = String> {
  input
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty())
    .map(str::to_owned)
}

fn read_conditions(input: &str) -> Result<SegmentConditions> {
  let text = match input.strip_prefix('@') {
    Some(path) => std::fs::read_to_string(path).map_err(|error| {
      Error::usage("invalid_conditions", format!("Could not read {path}: {error}"))
    })?,
    None => input.to_owned(),
  };

  serde_json::from_str(&text).map_err(|error| {
    Error::usage("invalid_conditions", format!("The conditions are not valid: {error}"))
  })
}

fn to_segment_type(kind: Kind) -> SegmentType {
  match kind {
    Kind::Static => SegmentType::Static,
    Kind::Dynamic => SegmentType::Dynamic,
  }
}

fn get_name(segment: &Value) -> &str {
  segment.get("name").and_then(Value::as_str).unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::{parse_lines, read_conditions};

  #[test]
  fn reads_one_contact_per_line() {
    let lines: Vec<String> = parse_lines("a@x.com\n\n  con_1 \n").collect();
    assert_eq!(lines, ["a@x.com", "con_1"]);
  }

  #[test]
  fn refuses_conditions_that_are_not_rules() {
    assert!(read_conditions(r#"{"match":"all","rules":[]}"#).is_ok());
    assert_eq!(read_conditions("[]").unwrap_err().name, "invalid_conditions");
  }
}
