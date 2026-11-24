use std::path::PathBuf;

use clap::{Args, Subcommand};
use inboxjot::{CampaignInput, ListCampaigns, SendCampaign, SendTestCampaign};
use serde_json::{Value, json};

use super::templates::{
  delete_document, get_name, list_versions, read_text_file, require_change,
  restore_version,
};
use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::Result;
use crate::output::{Column, column, format_count, format_time, get_path};
use crate::time::parse_schedule;
use crate::ui::style::{ACTION, DIM, TITLE};
use crate::ui::text::{Line, Span};

const LIST_COLUMNS: [Column; 6] = [
  column("ID", "id"),
  column("Name", "name"),
  column("Status", "status"),
  column("Recipients", "audience_count"),
  column("Delivered", "stats.delivered"),
  column("Updated", "updated_at"),
];

const DETAIL_ROWS: [Column; 13] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Status", "status"),
  column("From", "from"),
  column("Subject", "subject"),
  column("Preheader", "preheader"),
  column("Template", "template.name"),
  column("Audience", "audience"),
  column("Queued", "queued_count"),
  column("Scheduled", "scheduled_at"),
  column("Started", "started_at"),
  column("Completed", "completed_at"),
  column("Created", "created_at"),
];

#[derive(Debug, Subcommand)]
pub enum CampaignsCommand {
  #[command(
    about = "List campaigns",
    after_help = examples!("inboxjot campaigns list --status draft"),
    alias = "ls",
  )]
  List(ListCampaignsArgs),
  #[command(
    about = "Show a campaign",
    after_help = examples!("inboxjot campaigns inspect cpg_06d2mfk8r7hm4ps9wb2kd"),
  )]
  Inspect(CampaignArgs),
  #[command(
    about = "Create a draft campaign",
    after_help = examples!("inboxjot campaigns create --name \"November news\" --subject \"What's new\" --from \"Acme <news@acme.com>\" --html-file news.html --segment seg_123"),
  )]
  Create(CreateArgs),
  #[command(
    about = "Change a draft campaign",
    after_help = examples!("inboxjot campaigns update cpg_06d2mfk8r7hm4ps9wb2kd --subject \"What's new in November\""),
  )]
  Update(UpdateArgs),
  #[command(
    about = "Send a test of a campaign to a few addresses",
    after_help = examples!("inboxjot campaigns test cpg_06d2mfk8r7hm4ps9wb2kd --to riti@example.com"),
  )]
  Test(TestArgs),
  #[command(
    about = "Send a campaign now or at a set time",
    after_help = examples!(
      "inboxjot campaigns send cpg_06d2mfk8r7hm4ps9wb2kd",
      "inboxjot campaigns send cpg_06d2mfk8r7hm4ps9wb2kd --at 2026-11-20T09:00:00Z",
    ),
  )]
  Send(SendArgs),
  #[command(
    about = "Pause a sending campaign",
    after_help = examples!("inboxjot campaigns pause cpg_06d2mfk8r7hm4ps9wb2kd"),
  )]
  Pause(CampaignArgs),
  #[command(
    about = "Resume sending a paused campaign",
    after_help = examples!("inboxjot campaigns resume cpg_06d2mfk8r7hm4ps9wb2kd"),
  )]
  Resume(CampaignArgs),
  #[command(
    about = "Cancel a scheduled or sending campaign",
    after_help = examples!("inboxjot campaigns cancel cpg_06d2mfk8r7hm4ps9wb2kd"),
  )]
  Cancel(CampaignArgs),
  #[command(subcommand, about = "List and restore saved versions of a campaign")]
  Versions(VersionsCommand),
  #[command(
    about = "Delete a campaign",
    after_help = examples!("inboxjot campaigns delete cpg_06d2mfk8r7hm4ps9wb2kd"),
    alias = "rm",
  )]
  Delete(CampaignArgs),
}

#[derive(Debug, Subcommand)]
pub enum VersionsCommand {
  #[command(
    about = "List the saved versions of a campaign",
    after_help = examples!("inboxjot campaigns versions list cpg_06d2mfk8r7hm4ps9wb2kd"),
    alias = "ls",
  )]
  List(VersionsListArgs),
  #[command(
    about = "Copy a version into a campaign",
    after_help = examples!("inboxjot campaigns versions restore cpg_06d2mfk8r7hm4ps9wb2kd ver_06d2mfk8r5cx3nd8vt1qb"),
  )]
  Restore(RestoreArgs),
}

#[derive(Debug, Args)]
pub struct ListCampaignsArgs {
  #[arg(long, help = "Show only campaigns whose name has this text")]
  pub query: Option<String>,

  #[arg(long, help = "Show only campaigns with this status, such as draft or sent")]
  pub status: Option<String>,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct CampaignArgs {
  #[arg(
    value_name = "CAMPAIGN",
    help = "Campaign ID, such as cpg_06d2mfk8r7hm4ps9wb2kd"
  )]
  pub campaign: String,
}

#[derive(Debug, Args)]
pub struct ContentArgs {
  #[arg(long, help = "Subject line")]
  pub subject: Option<String>,

  #[arg(long, help = "From address on a verified domain, such as hello@acme.com")]
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

  #[arg(long, value_name = "TEMPLATE", help = "Template ID to start from")]
  pub template: Option<String>,

  #[arg(
    long = "segment",
    value_name = "SEGMENT",
    value_delimiter = ',',
    help = "Segment ID to send to; repeat it or separate IDs with commas"
  )]
  pub segments: Option<Vec<String>>,

  #[arg(
    long = "exclude-segment",
    value_name = "SEGMENT",
    value_delimiter = ',',
    help = "Segment ID to leave out; repeat it or separate IDs with commas"
  )]
  pub exclude_segments: Option<Vec<String>>,

  #[arg(long, help = "Topic ID that recipients must be subscribed to")]
  pub topic: Option<String>,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Campaign name")]
  pub name: String,

  #[command(flatten)]
  pub content: ContentArgs,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
  #[arg(
    value_name = "CAMPAIGN",
    help = "Campaign ID, such as cpg_06d2mfk8r7hm4ps9wb2kd"
  )]
  pub campaign: String,

  #[arg(long, help = "Campaign name")]
  pub name: Option<String>,

  #[command(flatten)]
  pub content: ContentArgs,
}

#[derive(Debug, Args)]
pub struct TestArgs {
  #[arg(
    value_name = "CAMPAIGN",
    help = "Campaign ID, such as cpg_06d2mfk8r7hm4ps9wb2kd"
  )]
  pub campaign: String,

  #[arg(
    long,
    required = true,
    value_name = "EMAIL",
    value_delimiter = ',',
    help = "Address to send the test to; up to 5, separated by commas"
  )]
  pub to: Vec<String>,
}

#[derive(Debug, Args)]
pub struct SendArgs {
  #[arg(
    value_name = "CAMPAIGN",
    help = "Campaign ID, such as cpg_06d2mfk8r7hm4ps9wb2kd"
  )]
  pub campaign: String,

  #[arg(
    long,
    value_name = "TIME",
    value_parser = parse_schedule,
    help = "Send at this time, such as 2026-11-20T09:00:00Z or \"in 2h\""
  )]
  pub at: Option<String>,
}

#[derive(Debug, Args)]
pub struct VersionsListArgs {
  #[arg(
    value_name = "CAMPAIGN",
    help = "Campaign ID, such as cpg_06d2mfk8r7hm4ps9wb2kd"
  )]
  pub campaign: String,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
  #[arg(
    value_name = "CAMPAIGN",
    help = "Campaign ID, such as cpg_06d2mfk8r7hm4ps9wb2kd"
  )]
  pub campaign: String,

  #[arg(value_name = "VERSION", help = "Version ID, such as ver_06d2mfk8r5cx3nd8vt1qb")]
  pub version_id: String,
}

pub async fn run(command: CampaignsCommand, context: &Context) -> Result<()> {
  match command {
    CampaignsCommand::List(args) => list(&args, context).await,
    CampaignsCommand::Inspect(args) => inspect(&args, context).await,
    CampaignsCommand::Create(args) => create(args, context).await,
    CampaignsCommand::Update(args) => update(args, context).await,
    CampaignsCommand::Test(args) => send_test(args, context).await,
    CampaignsCommand::Send(args) => send(args, context).await,
    CampaignsCommand::Pause(args) => pause(&args, context).await,
    CampaignsCommand::Resume(args) => resume(&args, context).await,
    CampaignsCommand::Cancel(args) => cancel(&args, context).await,
    CampaignsCommand::Versions(VersionsCommand::List(args)) => {
      list_versions("campaigns", &args.campaign, &args.list, context).await
    }
    CampaignsCommand::Versions(VersionsCommand::Restore(args)) => {
      restore_version("campaigns", &args.campaign, &args.version_id, context).await
    }
    CampaignsCommand::Delete(args) => {
      delete_document("campaigns", "campaign", &args.campaign, context).await
    }
  }
}

async fn list(args: &ListCampaignsArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading campaigns…");
  let query = ListCampaigns {
    query: args.query.clone(),
    status: args.status.clone(),
    ..Default::default()
  };
  let page = api.list(&["v1", "campaigns"], &query, &args.list.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "campaigns", None)
}

async fn inspect(args: &CampaignArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the campaign…");
  let campaign = api.get(&["v1", "campaigns", &args.campaign], &()).await?;

  if context.output.is_json() {
    return context.output.print_json(&campaign);
  }

  let mut detail = campaign.clone();
  detail["audience"] = json!(format_audience(&campaign));
  context.output.print_detail(&detail, &DETAIL_ROWS)
}

async fn create(args: CreateArgs, context: &Context) -> Result<()> {
  let body = to_campaign_input(Some(args.name), args.content)?;
  let api = context.api().await?.loading("Creating the campaign…");
  let campaign = api.post(&["v1", "campaigns"], &body).await?;

  let name = get_name(&campaign);
  context
    .output
    .print_result(&campaign, &format!("Created {name}"), "id", &[])
}

async fn update(args: UpdateArgs, context: &Context) -> Result<()> {
  let body = to_campaign_input(args.name, args.content)?;
  require_change(&body)?;

  let api = context.api().await?.loading("Saving the campaign…");
  let campaign = api.patch(&["v1", "campaigns", &args.campaign], &body).await?;
  let name = get_name(&campaign);
  context
    .output
    .print_result(&campaign, &format!("Updated {name}"), "id", &[])
}

async fn send_test(args: TestArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Sending a test email…");
  let headline = format!("Sent a test to {}", args.to.join(", "));
  let body = SendTestCampaign { to: args.to };
  let queued = api.post(&["v1", "campaigns", &args.campaign, "test"], &body).await?;

  if context.output.is_json() {
    return context.output.print_json(&queued);
  }

  context.output.success(&headline);
  if !context.output.is_stdout_tty() {
    for id in queued.get("ids").and_then(Value::as_array).into_iter().flatten() {
      context.output.print_line(id.as_str().unwrap_or_default());
    }
  }
  Ok(())
}

async fn send(args: SendArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Sending the campaign…");
  let campaign = api.get(&["v1", "campaigns", &args.campaign], &()).await?;
  let name = get_name(&campaign).to_owned();

  confirm_send(&campaign, args.at.as_deref(), context)?;

  let headline = match &args.at {
    Some(at) => format!("Scheduled {name} for {at}"),
    None => format!("Sending {name}"),
  };
  let body = SendCampaign { scheduled_at: args.at };
  let sent = api.post(&["v1", "campaigns", &args.campaign, "send"], &body).await?;
  context
    .output
    .print_result(&sent, &headline, "id", &[column("Status", "status")])
}

fn confirm_send(campaign: &Value, at: Option<&str>, context: &Context) -> Result<()> {
  let name = get_name(campaign);
  let consequence = format!("This sends the campaign {name}.");
  if context.yes || !context.interactive {
    return context.confirm_name("campaign", name, &consequence);
  }

  let field = |key: &str| get_path(campaign, key).and_then(Value::as_str).unwrap_or("-");
  let sends = at
    .map(|at| format_time(at, true, true).unwrap_or_else(|| at.to_owned()))
    .unwrap_or_else(|| "now".to_owned());
  let count = campaign.get("audience_count").and_then(Value::as_u64);
  let title = match count {
    Some(count) => format!("Send “{name}” to {} contacts?", format_count(count)),
    None => format!("Send “{name}”?"),
  };

  anstream::eprintln!("{}", Line::from(Span::styled(format!("  {title}"), TITLE)));
  anstream::eprintln!();
  let segments = get_names(campaign, "include_segments");
  let mut audience = if segments.is_empty() { "Everyone".to_owned() } else { segments };
  let excluded = get_names(campaign, "exclude_segments");
  if !excluded.is_empty() {
    audience.push_str(&format!(", excluding {excluded}"));
  }
  if let Some(count) = count {
    audience.push_str(&format!(" · {} contacts", format_count(count)));
  }
  let mut rows = Vec::new();
  if let Some(from) = get_path(campaign, "from")
    .and_then(Value::as_str)
    .filter(|from| !from.is_empty())
  {
    rows.push(("from", from.to_owned()));
  }
  let subject = field("subject");
  let subject =
    if subject.is_empty() || subject == "-" { "(no subject)" } else { subject };
  rows.push(("subject", subject.to_owned()));
  rows.push(("audience", audience));
  if let Some(topic) = get_path(campaign, "topic.name").and_then(Value::as_str) {
    rows.push(("topic", topic.to_owned()));
  }
  rows.push(("when", sends));
  for (label, value) in rows {
    let line =
      Line::new(vec![Span::styled(format!("  {label:<10}"), DIM), Span::raw(value)]);
    anstream::eprintln!("{line}");
  }
  anstream::eprintln!();
  context.output.warn("This sends real email and can’t be undone.");
  let id = campaign.get("id").and_then(Value::as_str).unwrap_or_default();
  anstream::eprintln!(
    "{}",
    Line::new(vec![
      Span::styled("    Test it first: ", DIM),
      Span::styled(format!("inboxjot campaigns test {id} --to <email>"), ACTION),
    ])
  );
  anstream::eprintln!();

  match count {
    Some(count) => {
      let digits = count.to_string();
      let grouped = format_count(count);
      context.confirm_typed(
        &[&digits, &grouped],
        &format!("Type {digits} to send:"),
        "Type the number of contacts exactly, or press esc to cancel.",
        &consequence,
      )
    }
    None => context.confirm_name("campaign", name, &consequence),
  }
}

async fn cancel(args: &CampaignArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Canceling the campaign…");
  let campaign = api.get(&["v1", "campaigns", &args.campaign], &()).await?;
  let name = get_name(&campaign).to_owned();

  context.confirm(&format!("Cancel the campaign {name}?"))?;

  let canceled = api.post(&["v1", "campaigns", &args.campaign, "cancel"], &()).await?;
  context
    .output
    .print_result(&canceled, &format!("Canceled {name}"), "id", &[])
}

async fn pause(args: &CampaignArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Pausing the campaign…");
  let segments = ["v1", "campaigns", &args.campaign, "pause"];
  let paused = api.post(&segments, &()).await?;

  let name = get_name(&paused);
  context.output.print_result(
    &paused,
    &format!("Paused {name}"),
    "id",
    &[column("Status", "status")],
  )
}

async fn resume(args: &CampaignArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Resuming the campaign…");
  let campaign_path = ["v1", "campaigns", &args.campaign];
  let campaign = api.get(&campaign_path, &()).await?;

  context.confirm(&format!("Resume sending the campaign {}?", get_name(&campaign)))?;

  let segments = [&campaign_path[..], &["resume"]].concat();
  let resumed = api.post(&segments, &()).await?;
  let name = get_name(&resumed);
  context.output.print_result(
    &resumed,
    &format!("Resumed {name}"),
    "id",
    &[column("Status", "status")],
  )
}

fn to_campaign_input(
  name: Option<String>,
  content: ContentArgs,
) -> Result<CampaignInput> {
  Ok(CampaignInput {
    name,
    from: content.from,
    reply_to: content.reply_to.map(Some),
    subject: content.subject,
    preheader: content.preheader,
    html: content.html_file.as_deref().map(read_text_file).transpose()?,
    content: None,
    template_id: content.template.map(Some),
    include_segment_ids: content.segments,
    exclude_segment_ids: content.exclude_segments,
    topic_id: content.topic,
  })
}

fn format_audience(campaign: &Value) -> String {
  let segments = get_names(campaign, "include_segments");
  let topic = get_path(campaign, "topic.name")
    .and_then(Value::as_str)
    .unwrap_or("no topic");
  let mut audience = if segments.is_empty() {
    format!("Everyone on {topic}")
  } else {
    format!("{segments} on {topic}")
  };

  let excluded = get_names(campaign, "exclude_segments");
  if !excluded.is_empty() {
    audience.push_str(&format!(", excluding {excluded}"));
  }

  if let Some(count) = campaign.get("audience_count").and_then(Value::as_u64) {
    audience.push_str(&format!(" ({} contacts)", format_count(count)));
  }

  audience
}

fn get_names(campaign: &Value, key: &str) -> String {
  let segments = campaign.get(key).and_then(Value::as_array).into_iter().flatten();
  let names: Vec<&str> = segments
    .filter_map(|segment| segment.get("name").and_then(Value::as_str))
    .collect();

  names.join(", ")
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::format_audience;

  #[test]
  fn describes_the_audience() {
    let campaign = json!({
      "include_segments": [{ "name": "Customers" }, { "name": "Trials" }],
      "exclude_segments": [{ "name": "Churned" }],
      "topic": { "name": "Product updates" },
      "audience_count": 1204
    });
    assert_eq!(
      format_audience(&campaign),
      "Customers, Trials on Product updates, excluding Churned (1,204 contacts)"
    );

    let campaign = json!({
      "include_segments": [], "exclude_segments": [],
      "topic": { "name": "News" }, "audience_count": null
    });
    assert_eq!(format_audience(&campaign), "Everyone on News");
  }
}
