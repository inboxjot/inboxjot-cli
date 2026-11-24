use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};

use crate::api::{Api, Page};
use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column};
use crate::ui::style::{ACTION, DIM, TITLE};
use crate::ui::text::{Line, Span};

const LIST_COLUMNS: [Column; 7] = [
  column("ID", "id"),
  column("Name", "name"),
  column("Token", "short_token"),
  column("Permission", "permission"),
  column("Domain", "domain_name"),
  column("Last used", "last_used_at"),
  column("Created", "created_at"),
];

const DETAIL_ROWS: [Column; 14] = [
  column("Name", "name"),
  column("ID", "id"),
  column("Token", "short_token"),
  column("Permission", "permission"),
  column("Status", "status"),
  column("Domain", "domain_name"),
  column("IP allowlist", "ip_allowlist"),
  column("Sent (30d)", "sent_last_30_days"),
  column("Requests (30d)", "requests_last_30_days"),
  column("Errors (30d)", "errors_last_30_days"),
  column("Created by", "creator_name"),
  column("Last used", "last_used_at"),
  column("Expires", "expires_at"),
  column("Created", "created_at"),
];

#[derive(Debug, Subcommand)]
pub enum ApiKeysCommand {
  #[command(
    about = "List API keys",
    after_help = examples!("inboxjot api-keys list"),
    alias = "ls",
  )]
  List(ListArgs),
  #[command(
    about = "Show an API key and its usage",
    after_help = examples!("inboxjot api-keys inspect api_123"),
  )]
  Inspect(KeyArgs),
  #[command(
    about = "Create an API key and print it once",
    after_help = examples!(
      "inboxjot api-keys create --name Production --permission sending --domain acme.com",
      "inboxjot api-keys create --name CI --permission full --expires-in 90d --allow-ip 203.0.113.7",
    ),
  )]
  Create(CreateArgs),
  #[command(
    about = "Revoke an API key",
    after_help = examples!("inboxjot api-keys revoke api_123"),
    alias = "rm",
  )]
  Revoke(KeyArgs),
  #[command(
    about = "Replace the secret of an API key and print the new key once",
    after_help = examples!("inboxjot api-keys rotate api_123 --old-key-expires-in 1h"),
  )]
  Rotate(RotateArgs),
}

#[derive(Debug, Args)]
pub struct KeyArgs {
  #[arg(value_name = "KEY_ID", help = "API key ID")]
  pub id: String,
}

#[derive(Debug, Args)]
pub struct CreateArgs {
  #[arg(long, help = "Name of the key, such as Production")]
  pub name: String,

  #[arg(long, value_enum, help = "What the key can do")]
  pub permission: Permission,

  #[arg(long, help = "Limit a sending key to this domain name or ID")]
  pub domain: Option<String>,

  #[arg(long, value_enum, default_value_t = KeyExpiry::Never, help = "When the key stops working")]
  pub expires_in: KeyExpiry,

  #[arg(
    long = "allow-ip",
    value_name = "IP",
    help = "Accept requests only from this IP address"
  )]
  pub ip_allowlist: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RotateArgs {
  #[arg(value_name = "KEY_ID", help = "API key ID")]
  pub id: String,

  #[arg(long, value_enum, default_value_t = OldKeyExpiry::Now, help = "When the old secret stops working")]
  pub old_key_expires_in: OldKeyExpiry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Permission {
  Sending,
  Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum KeyExpiry {
  Never,
  #[value(name = "30d")]
  ThirtyDays,
  #[value(name = "90d")]
  NinetyDays,
  #[value(name = "1y")]
  OneYear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum OldKeyExpiry {
  Now,
  #[value(name = "1h")]
  OneHour,
  #[value(name = "24h")]
  OneDay,
}

pub async fn run(command: ApiKeysCommand, context: &Context) -> Result<()> {
  match command {
    ApiKeysCommand::List(args) => list(&args, context).await,
    ApiKeysCommand::Inspect(args) => inspect(&args, context).await,
    ApiKeysCommand::Create(args) => create(&args, context).await,
    ApiKeysCommand::Revoke(args) => revoke(&args, context).await,
    ApiKeysCommand::Rotate(args) => rotate(&args, context).await,
  }
}

async fn list(args: &ListArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading API keys…");
  let page = api.list(&["v1", "api-keys"], &(), &args.to_page()).await?;

  context.output.print_list(
    &page,
    &LIST_COLUMNS,
    "API keys",
    Some(("add one", "inboxjot api-keys create --name <name>")),
  )
}

async fn inspect(args: &KeyArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the API key…");
  let key = api.get(&["v1", "api-keys", &args.id], &()).await?;

  context.output.print_detail(&key, &DETAIL_ROWS)
}

async fn create(args: &CreateArgs, context: &Context) -> Result<()> {
  let api = context.login_api().await?.loading("Creating the API key…");
  let domain_id = match &args.domain {
    Some(domain) => Some(resolve_domain_id(&api, domain).await?),
    None => None,
  };

  let key = create_key(
    &api,
    &args.name,
    args.permission,
    domain_id,
    args.expires_in,
    &args.ip_allowlist,
  )
  .await?;
  let scope = args.domain.clone().unwrap_or_else(|| "all domains".to_owned());
  print_secret(&key, &format!("Created {}", args.name), &scope, true, context)
}

async fn revoke(args: &KeyArgs, context: &Context) -> Result<()> {
  let api = context.login_api().await?.loading("Revoking the API key…");
  let name = get_key_name(&api, &args.id).await?;
  context.confirm_delete(&format!("Revoke {name}?"), "Requests with it fail at once.")?;

  api.delete(&["v1", "api-keys", &args.id]).await?;
  let result = json!({ "id": args.id, "name": name, "revoked": true });
  context
    .output
    .print_removal(&result, &format!("Revoked {name}"), "id", &[])
}

async fn rotate(args: &RotateArgs, context: &Context) -> Result<()> {
  let api = context.login_api().await?.loading("Rotating the API key…");
  let name = get_key_name(&api, &args.id).await?;
  let (expiry, when) = match args.old_key_expires_in {
    OldKeyExpiry::Now => ("now", "at once"),
    OldKeyExpiry::OneHour => ("1h", "in 1 hour"),
    OldKeyExpiry::OneDay => ("24h", "in 24 hours"),
  };
  context.confirm(&format!(
    "Rotate the API key {name}? The old secret stops working {when}."
  ))?;

  let body = json!({ "old_key_expires_in": expiry });
  let key = api.post(&["v1", "api-keys", &args.id, "roll"], &body).await?;
  print_secret(&key, &format!("Rotated {name}"), "", false, context)
}

pub async fn create_key(
  api: &Api,
  name: &str,
  permission: Permission,
  domain_id: Option<String>,
  expires_in: KeyExpiry,
  ip_allowlist: &[String],
) -> Result<Value> {
  let permission = match permission {
    Permission::Sending => "sending",
    Permission::Full => "full",
  };
  let expires_in = match expires_in {
    KeyExpiry::Never => "never",
    KeyExpiry::ThirtyDays => "30d",
    KeyExpiry::NinetyDays => "90d",
    KeyExpiry::OneYear => "1y",
  };
  let body = json!({
    "name": name,
    "permission": permission,
    "domain_id": domain_id,
    "expires_in": expires_in,
    "ip_allowlist": ip_allowlist,
  });

  api.post(&["v1", "api-keys"], &body).await
}

pub fn get_full_key(key: &Value) -> String {
  let field = |name: &str| key.get(name).and_then(Value::as_str).unwrap_or_default();
  format!("{}_{}_{}", field("prefix"), field("short_token"), field("long_token"))
}

fn print_secret(
  key: &Value,
  headline: &str,
  scope: &str,
  is_new: bool,
  context: &Context,
) -> Result<()> {
  if context.output.is_json() {
    return context.output.print_json(key);
  }

  if context.output.is_rich() && context.output.is_stdout_tty() {
    let id = key.get("id").and_then(Value::as_str).unwrap_or_default();
    let access = match key.get("permission").and_then(Value::as_str) {
      Some("full") => "Full access",
      _ => "Sending access",
    };
    let detail = if scope.is_empty() {
      format!("{id}  {access}")
    } else {
      format!("{id}  {access} · {scope}")
    };
    context.output.done(headline, &detail);
    anstream::println!();
    anstream::println!(
      "{}",
      Line::from(Span::styled(format!("  {}", get_full_key(key)), TITLE))
    );
    anstream::println!();
    context
      .output
      .warn("This is the only time the key is shown. Store it now.");
    anstream::eprintln!(
      "{}",
      Line::new(vec![
        Span::styled("    Lost it? ", DIM),
        Span::styled(format!("inboxjot api-keys rotate {id}"), ACTION),
      ])
    );
    if is_new {
      anstream::eprintln!();
      anstream::eprintln!(
        "{}",
        Line::new(vec![
          Span::styled("  In a script: ", DIM),
          Span::styled(
            "inboxjot api-keys create --name CI | gh secret set INBOXJOT_API_KEY",
            ACTION
          ),
        ])
      );
    }
    return Ok(());
  }

  context.output.print_line(&get_full_key(key));
  context.output.success(headline);
  context.output.info("Copy the key now. It is shown only once.");
  Ok(())
}

async fn get_key_name(api: &Api, id: &str) -> Result<String> {
  let key = api.get(&["v1", "api-keys", id], &()).await?;
  Ok(key.get("name").and_then(Value::as_str).unwrap_or(id).to_owned())
}

async fn resolve_domain_id(api: &Api, selector: &str) -> Result<String> {
  if selector.starts_with("dom_") {
    return Ok(selector.to_owned());
  }

  let page = api
    .list(&["v1", "domains"], &(), &Page { all: true, ..Page::default() })
    .await?;
  page
    .get("data")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .find(|domain| {
      domain
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case(selector))
    })
    .and_then(|domain| domain.get("id").and_then(Value::as_str))
    .map(str::to_owned)
    .ok_or_else(|| {
      Error::failed("not_found", format!("No domain is named {selector}."))
        .with_next("inboxjot domains list", "List the domains")
    })
}
