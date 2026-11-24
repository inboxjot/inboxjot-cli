use clap::builder::styling::{AnsiColor, Styles};
use clap::{Args, Parser, Subcommand};

use crate::api::Page;
use crate::commands::{
  agent, api, api_keys, auth, automations, brands, campaigns, completion, contacts,
  domains, emails, events, help, images, init, listen, logs, open, segments, stats,
  suppressions, templates, topics, trigger, webhooks,
};

const STYLES: Styles = Styles::styled()
  .header(AnsiColor::Magenta.on_default().bold())
  .usage(AnsiColor::Magenta.on_default().bold())
  .literal(anstyle::Style::new().bold())
  .placeholder(AnsiColor::Cyan.on_default());

#[derive(Debug, Parser)]
#[command(
  name = "inboxjot",
  version,
  about = "Send email and manage Inboxjot from the terminal",
  disable_help_subcommand = true,
  propagate_version = true,
  max_term_width = 100,
  styles = STYLES,
  after_help = "Learn more: https://inboxjot.com/docs/cli"
)]
pub struct Cli {
  #[command(flatten)]
  pub global: GlobalArgs,

  #[command(subcommand)]
  pub command: Option<Command>,
}

#[derive(Debug, Default, Args)]
pub struct GlobalArgs {
  #[arg(long, global = true, help = "Print JSON on stdout and never prompt")]
  pub json: bool,

  #[arg(
    long,
    global = true,
    value_name = "EXPR",
    help = "Filter JSON output with a jq expression"
  )]
  pub jq: Option<String>,

  #[arg(
    long,
    global = true,
    env = "INBOXJOT_PROFILE",
    default_value = "default",
    hide_env_values = true,
    help = "Use the credentials saved under this name"
  )]
  pub profile: String,

  #[arg(
    short,
    long,
    global = true,
    help = "Confirm destructive actions without a prompt"
  )]
  pub yes: bool,

  #[arg(
    long,
    global = true,
    value_name = "BOOL",
    num_args = 0..=1,
    require_equals = true,
    default_missing_value = "true",
    help = "Never prompt; missing input fails and names the option"
  )]
  pub non_interactive: Option<bool>,

  #[arg(long, global = true, help = "Print without color")]
  pub no_color: bool,

  #[arg(short, long, global = true, help = "Print each API request on stderr")]
  pub debug: bool,

  #[arg(long, global = true, hide = true, env = "INBOXJOT_API_URL")]
  pub api_url: Option<String>,

  #[arg(long, global = true, hide = true, env = "INBOXJOT_DASHBOARD_URL")]
  pub dashboard_url: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
  #[command(
    about = "Log in with the browser, or save an API key",
    after_help = examples!(
      "inboxjot login",
      "inboxjot login --api-key - < key.txt",
    ),
  )]
  Login(auth::LoginArgs),
  #[command(
    about = "Log out and revoke the saved login",
    after_help = examples!(
      "inboxjot logout",
      "inboxjot logout --profile staging",
    ),
  )]
  Logout,
  #[command(
    about = "Show the current login and its access",
    after_help = examples!(
      "inboxjot whoami",
      "inboxjot whoami --json",
    ),
  )]
  Whoami,

  #[command(
    about = "Send an email",
    after_help = examples!(
      "inboxjot send --from \"Acme <hi@acme.com>\" --to riti@example.com --subject \"Welcome\" --text \"Thanks for signing up\"",
      "inboxjot send --from hi@acme.com --to riti@example.com --template tpl_06d2mfk8r5cx3nd8vt1qa --data '{\"name\":\"Riti\"}'",
      "inboxjot send --from hi@acme.com --to bounced@inboxjot.dev --subject Test --text Hi --wait",
    ),
  )]
  Send(Box<emails::SendArgs>),
  #[command(subcommand, about = "Send and track emails")]
  Emails(emails::EmailsCommand),
  #[command(subcommand, about = "Add and verify sending domains")]
  Domains(domains::DomainsCommand),
  #[command(subcommand, about = "Record contact events for automations")]
  Events(events::EventsCommand),

  #[command(subcommand, about = "Manage contacts")]
  Contacts(contacts::ContactsCommand),
  #[command(subcommand, about = "Manage segments and their members")]
  Segments(segments::SegmentsCommand),
  #[command(subcommand, about = "Manage brands and their unsubscribe pages")]
  Brands(brands::BrandsCommand),
  #[command(subcommand, about = "Manage subscription topics")]
  Topics(topics::TopicsCommand),

  #[command(subcommand, about = "Manage email templates")]
  Templates(templates::TemplatesCommand),
  #[command(subcommand, about = "Create, test and send campaigns")]
  Campaigns(campaigns::CampaignsCommand),
  #[command(subcommand, about = "Upload and manage hosted images")]
  Images(images::ImagesCommand),
  #[command(subcommand, about = "Turn automations on and off")]
  Automations(automations::AutomationsCommand),
  #[command(subcommand, about = "Manage suppressed email addresses")]
  Suppressions(suppressions::SuppressionsCommand),
  #[command(
    about = "Show sending stats for the workspace",
    after_help = examples!(
      "inboxjot stats",
      "inboxjot stats --period 7d",
    ),
  )]
  Stats(stats::StatsArgs),

  #[command(subcommand, name = "api-keys", about = "Create, rotate and revoke API keys")]
  ApiKeys(api_keys::ApiKeysCommand),
  #[command(
    about = "Inspect and follow API requests",
    args_conflicts_with_subcommands = true,
    after_help = examples!("inboxjot logs --follow", "inboxjot logs list --outcome failed"),
  )]
  Logs(logs::LogsArgs),
  #[command(
    about = "Create an API key for this project and save it to .env.local",
    after_help = examples!(
      "inboxjot init",
      "inboxjot init --dir ./web --permission full",
    ),
  )]
  Init(init::InitArgs),
  #[command(
    about = "Open the dashboard in the browser",
    after_help = examples!(
      "inboxjot open",
      "inboxjot open emails eml_123",
    ),
  )]
  Open(open::OpenArgs),

  #[command(subcommand, about = "Manage webhook endpoints and verify payloads")]
  Webhooks(webhooks::WebhooksCommand),
  #[command(
    about = "Show live events and forward them to a local URL",
    after_help = examples!(
      "inboxjot listen",
      "inboxjot listen --forward-to localhost:3000/webhooks",
      "inboxjot listen --events email.bounced,email.complained",
    ),
  )]
  Listen(listen::ListenArgs),
  #[command(
    about = "Cause a test event in the workspace",
    after_help = examples!(
      "inboxjot trigger email.bounced",
      "inboxjot trigger contact.created",
    ),
  )]
  Trigger(trigger::TriggerArgs),
  #[command(
    about = "Make an authenticated request to the Inboxjot API",
    after_help = examples!(
      "inboxjot api /v1/domains --paginate",
      "inboxjot api /v1/contacts -F email=riti@example.com -F subscribed=true",
    ),
  )]
  Api(api::ApiArgs),
  #[command(
    about = "Check the setup, the connection and every domain",
    after_help = examples!("inboxjot doctor"),
  )]
  Doctor,
  #[command(subcommand, about = "Connect AI coding agents to Inboxjot")]
  Agent(agent::AgentCommand),
  #[command(
    about = "Print a shell completion script",
    after_help = examples!("inboxjot completion zsh > ~/.zfunc/_inboxjot"),
  )]
  Completion(completion::CompletionArgs),
  #[command(
    about = "Show help for a command, or search all commands",
    after_help = examples!(
      "inboxjot help domains add",
      "inboxjot help search webhook",
    ),
  )]
  Help(help::HelpArgs),
}

#[derive(Clone, Debug, Args)]
pub struct ListArgs {
  #[arg(
    short = 'n',
    long,
    default_value_t = 50,
    value_parser = clap::value_parser!(u32).range(1..),
    help = "Show at most this many rows"
  )]
  pub limit: u32,

  #[arg(long, help = "Show every row, one page after another")]
  pub all: bool,

  #[arg(long, value_name = "CURSOR", help = "Show the rows after this cursor")]
  pub after: Option<String>,

  #[arg(
    long,
    value_name = "CURSOR",
    conflicts_with_all = ["after", "all"],
    help = "Show the rows before this cursor"
  )]
  pub before: Option<String>,
}

impl ListArgs {
  pub fn to_page(&self) -> Page {
    Page {
      limit: self.limit as usize,
      all: self.all,
      after: self.after.clone(),
      before: self.before.clone(),
    }
  }
}

#[cfg(test)]
mod tests {
  use clap::CommandFactory;

  use super::Cli;

  #[test]
  fn has_a_valid_command_tree() {
    Cli::command().debug_assert();
  }
}
