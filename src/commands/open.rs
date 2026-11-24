use clap::{Args, ValueEnum};
use reqwest::Url;
use serde_json::json;

use crate::context::Context;
use crate::credentials;
use crate::error::{Error, Result};

const DEFAULT_DASHBOARD_URL: &str = "https://app.inboxjot.com";

#[derive(Debug, Args)]
pub struct OpenArgs {
  #[arg(
    value_name = "PAGE",
    default_value = "home",
    value_parser = parse_target,
    help = "Dashboard page to open, such as emails or emails/eml_123"
  )]
  pub target: Target,

  #[arg(value_name = "ID", help = "Open one item of the page, such as an email ID")]
  pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
  pub page: Page,
  pub id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Page {
  Home,
  Emails,
  Domains,
  Contacts,
  Segments,
  Templates,
  Campaigns,
  Automations,
  Logs,
  ApiKeys,
  Webhooks,
}

pub fn run(args: &OpenArgs, context: &Context) -> Result<()> {
  let workspace = credentials::read_profile(&context.profile)?
    .workspace
    .ok_or_else(Error::needs_browser_login)?;
  let page = args.target.page;
  let id = args.target.id.as_ref().or(args.id.as_ref());
  if page == Page::Home && id.is_some() {
    return Err(Error::usage(
      "invalid_page",
      "Name a page to open an item, such as emails.",
    ));
  }

  let dashboard_url = context
    .read_dashboard_url()?
    .unwrap_or_else(|| DEFAULT_DASHBOARD_URL.to_owned());
  let mut url = Url::parse(&dashboard_url).map_err(|error| {
    Error::usage("invalid_url", format!("The dashboard URL is not valid: {error}"))
  })?;
  {
    let mut segments = url.path_segments_mut().map_err(|()| {
      Error::failed("invalid_url", "The dashboard URL cannot have a path.")
    })?;
    segments.pop_if_empty().push(&workspace);
    if let Some(name) = page.to_possible_value().filter(|_| page != Page::Home) {
      segments.push(name.get_name());
    }
    segments.extend(id);
  }

  if context.output.is_json() {
    context.output.print_json(&json!({ "url": url.as_str() }))?;
  } else {
    context.output.print_line(url.as_str());
  }

  if !open_browser(url.as_str()) {
    context.output.info("Open the address above in your browser.");
  }
  Ok(())
}

fn parse_target(text: &str) -> Result<Target, String> {
  let (name, id) = match text.split_once('/') {
    Some((name, id)) if !id.is_empty() => (name, Some(id.to_owned())),
    _ => (text.trim_end_matches('/'), None),
  };
  let page = Page::from_str(name, true).map_err(|_| {
    let names: Vec<String> = Page::value_variants()
      .iter()
      .filter_map(|page| {
        page.to_possible_value().map(|value| value.get_name().to_owned())
      })
      .collect();
    format!("\"{name}\" is not a page. Use {}", names.join(", "))
  })?;
  Ok(Target { page, id })
}

pub fn open_browser(url: &str) -> bool {
  let Some(command) = std::env::var("INBOXJOT_BROWSER")
    .ok()
    .filter(|value| !value.is_empty())
  else {
    return webbrowser::open(url).is_ok();
  };

  let Some((program, args)) = shlex::split(&command).and_then(|words| {
    let (program, args) = words.split_first()?;
    Some((program.clone(), args.to_vec()))
  }) else {
    return false;
  };

  std::process::Command::new(program)
    .args(args)
    .arg(url)
    .spawn()
    .is_ok()
}
