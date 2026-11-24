use std::io::{Read, Write};

use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use serde_json::{Map, Value};

use anstyle::{AnsiColor, Style};

use super::webhooks::parse_header;
use crate::api::{Api, RawResponse, read_api_error};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::ui::style::{DANGER, DIM, DONE, WARNING};
use crate::ui::text::{Line, Span};

const KEY: Style = AnsiColor::Blue.on_default();

#[derive(Debug, clap::Args)]
pub struct ApiArgs {
  #[arg(
    value_name = "PATH",
    num_args = 1..=2,
    required = true,
    help = "API path, such as /v1/domains, with the HTTP method first if you like: GET /v1/domains"
  )]
  pub target: Vec<String>,

  #[arg(
    short = 'X',
    long,
    value_name = "METHOD",
    help = "HTTP method; defaults to GET, or POST when you add fields or input"
  )]
  pub method: Option<String>,

  #[arg(
    short = 'f',
    long = "raw-field",
    value_name = "KEY=VALUE",
    help = "Add a string field; repeat for each field"
  )]
  pub raw_fields: Vec<String>,

  #[arg(
    short = 'F',
    long = "field",
    value_name = "KEY=VALUE",
    help = "Add a typed field: number, true, false, null or @file; repeat for each field"
  )]
  pub fields: Vec<String>,

  #[arg(
    long,
    value_name = "FILE",
    help = "File with the raw request body, or - for stdin"
  )]
  pub input: Option<String>,

  #[arg(
    short = 'H',
    long = "header",
    value_name = "HEADER",
    help = "Add a request header as 'Name: value'; repeat for each header"
  )]
  pub headers: Vec<String>,

  #[arg(long, help = "Follow next_cursor and merge the data of every page")]
  pub paginate: bool,

  #[arg(short, long, help = "Print the status line and response headers first")]
  pub include: bool,
}

pub async fn run(args: ApiArgs, context: &Context) -> Result<()> {
  let fields = read_fields(&args)?;
  let has_body = !fields.is_empty() || args.input.is_some();
  let (named_method, path) = match args.target.as_slice() {
    [method, path] => (Some(method.clone()), path.clone()),
    [path] => (None, path.clone()),
    _ => {
      return Err(Error::usage("missing_input", "Add an API path, such as /v1/domains."));
    }
  };
  let method = match named_method.as_ref().or(args.method.as_ref()) {
    Some(method) => {
      Method::from_bytes(method.to_uppercase().as_bytes()).map_err(|_| {
        Error::usage("invalid_method", format!("{method} is not an HTTP method."))
      })?
    }
    None if has_body => Method::POST,
    None => Method::GET,
  };
  if args.paginate && method != Method::GET {
    return Err(Error::usage("invalid_option", "Use --paginate only with GET."));
  }

  let mut headers = HeaderMap::new();
  for header in &args.headers {
    let (name, value) = parse_header(header)?;
    headers.append(name, value);
  }

  let fields_in_query = method == Method::GET || args.input.is_some();
  let query = if fields_in_query { to_query(&fields) } else { Vec::new() };
  let body = match &args.input {
    Some(input) => Some(read_input(input)?),
    None if !fields_in_query => Some(serde_json::to_vec(&Value::Object(fields))?),
    None => None,
  };

  let api = context.api().await?.loading(format!("Requesting {path}…"));
  let url = build_url(api.base_url(), &path)?;
  let label = format!("{method} {url}");
  let started = std::time::Instant::now();
  let response = if args.paginate {
    fetch_pages(&api, url, &query, &headers).await?
  } else {
    api.send_raw(method, url, &query, &headers, body).await?
  };

  if args.include {
    print_head(&response);
  }

  if context.output.is_rich() && !args.include {
    print_status(&response, &label, started.elapsed());
    if context.output.is_stdout_tty()
      && let Ok(value) = serde_json::from_slice::<Value>(&response.body)
    {
      for line in render_json(&value) {
        anstream::println!("{line}");
      }
    } else {
      write_raw(&response.body)?;
    }
    if response.status.is_success() {
      return Ok(());
    }
    let mut error = read_api_error(response.status, &response.body);
    error.kind = crate::error::Kind::Quiet;
    return Err(error);
  }

  if !response.status.is_success() {
    write_raw(&response.body)?;
    let mut error = read_api_error(response.status, &response.body);
    error.message = format!("{} (HTTP {})", error.message, response.status.as_u16());
    error.details.clear();
    error.request_id = None;
    return Err(error);
  }

  print_body(&response.body, context)
}

fn build_url(base_url: &Url, path: &str) -> Result<Url> {
  let mut base_url = base_url.clone();
  if !base_url.path().ends_with('/') {
    base_url.set_path(&format!("{}/", base_url.path()));
  }

  let invalid = || Error::usage("invalid_path", format!("{path} is not an API path."));
  let url = base_url.join(path.trim_start_matches('/')).map_err(|_| invalid())?;
  if url.origin() != base_url.origin() || !url.path().starts_with(base_url.path()) {
    return Err(invalid());
  }

  Ok(url)
}

async fn fetch_pages(
  api: &Api,
  url: Url,
  query: &[(String, String)],
  headers: &HeaderMap,
) -> Result<RawResponse> {
  let mut items = Vec::new();
  let mut url = url;

  loop {
    let mut response =
      api.send_raw(Method::GET, url.clone(), query, headers, None).await?;
    if !response.status.is_success() {
      return Ok(response);
    }

    let mut page: Value = serde_json::from_slice(&response.body)?;
    if let Some(Value::Array(data)) = page.get_mut("data").map(Value::take) {
      items.extend(data);
    }

    let Some(cursor) = page.get("next_cursor").and_then(Value::as_str) else {
      page["data"] = Value::Array(items);
      response.body = serde_json::to_vec(&page)?;
      return Ok(response);
    };

    url = set_after(&url, cursor);
  }
}

fn set_after(url: &Url, cursor: &str) -> Url {
  let pairs: Vec<(String, String)> = url
    .query_pairs()
    .filter(|(key, _)| key != "after")
    .map(|(key, value)| (key.into_owned(), value.into_owned()))
    .collect();
  let mut url = url.clone();
  url
    .query_pairs_mut()
    .clear()
    .extend_pairs(pairs)
    .append_pair("after", cursor);

  url
}

fn read_fields(args: &ApiArgs) -> Result<Map<String, Value>> {
  let mut fields = Map::new();
  for field in &args.raw_fields {
    let (key, value) = split_field(field)?;
    fields.insert(key.to_owned(), Value::from(value));
  }
  for field in &args.fields {
    let (key, value) = split_field(field)?;
    fields.insert(key.to_owned(), parse_typed_value(value)?);
  }

  Ok(fields)
}

fn split_field(field: &str) -> Result<(&str, &str)> {
  field
    .split_once('=')
    .filter(|(key, _)| !key.is_empty())
    .ok_or_else(|| {
      Error::usage("invalid_field", format!("Write the field {field} as key=value."))
    })
}

fn parse_typed_value(value: &str) -> Result<Value> {
  if let Some(path) = value.strip_prefix('@') {
    let bytes = read_input(path)?;
    return Ok(Value::from(String::from_utf8_lossy(&bytes).into_owned()));
  }

  Ok(match value {
    "true" => Value::Bool(true),
    "false" => Value::Bool(false),
    "null" => Value::Null,
    _ => serde_json::from_str::<serde_json::Number>(value)
      .map_or_else(|_| Value::from(value), Value::Number),
  })
}

fn to_query(fields: &Map<String, Value>) -> Vec<(String, String)> {
  fields
    .iter()
    .map(|(key, value)| {
      let value = match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
      };
      (key.clone(), value)
    })
    .collect()
}

fn read_input(path: &str) -> Result<Vec<u8>> {
  if path == "-" {
    let mut buffer = Vec::new();
    std::io::stdin().read_to_end(&mut buffer)?;
    return Ok(buffer);
  }

  std::fs::read(path).map_err(|error| {
    Error::usage("invalid_input", format!("Could not read {path}: {error}"))
  })
}

fn print_head(response: &RawResponse) {
  let reason = response.status.canonical_reason().unwrap_or_default();
  anstream::println!("HTTP {} {reason}", response.status.as_u16());
  for (name, value) in &response.headers {
    anstream::println!("{name}: {}", String::from_utf8_lossy(value.as_bytes()));
  }
  anstream::println!();
}

fn print_status(response: &RawResponse, label: &str, took: std::time::Duration) {
  let status = response.status;
  let style = match status.as_u16() {
    200..=299 => DONE,
    400..=499 => WARNING,
    _ => DANGER,
  };
  let reason = status.canonical_reason().unwrap_or_default();
  anstream::eprintln!(
    "{}",
    Line::new(vec![
      Span::styled(format!("  {} {reason}", status.as_u16()), style),
      Span::styled(format!("  {}ms · {label}", took.as_millis()), DIM),
    ])
  );
  anstream::eprintln!();
}

fn render_json(value: &Value) -> Vec<Line<'static>> {
  let mut lines = Vec::new();
  let mut current = Vec::new();
  write_value(value, 0, true, &mut current, &mut lines);
  if !current.is_empty() {
    lines.push(Line::new(current));
  }
  lines
}

fn write_value(
  value: &Value,
  depth: usize,
  expand: bool,
  current: &mut Vec<Span<'static>>,
  lines: &mut Vec<Line<'static>>,
) {
  let indent = "  ".repeat(depth + 1);
  let close_indent = "  ".repeat(depth);
  match value {
    Value::Object(map) if map.is_empty() => current.push(Span::raw("{}")),
    Value::Object(map) => {
      current.push(Span::raw("{"));
      lines.push(Line::new(std::mem::take(current)));
      let count = map.len();
      for (index, (key, item)) in map.iter().enumerate() {
        current.push(Span::raw(indent.clone()));
        current.push(Span::styled(format!("{}", Value::from(key.as_str())), KEY));
        current.push(Span::raw(": "));
        let keep = depth == 0 && key == "data";
        write_value(item, depth + 1, keep, current, lines);
        if index + 1 < count {
          current.push(Span::raw(","));
        }
        lines.push(Line::new(std::mem::take(current)));
      }
      current.push(Span::raw(format!("{close_indent}}}")));
    }
    Value::Array(items) if items.is_empty() => current.push(Span::raw("[]")),
    Value::Array(items) if !expand && depth > 0 && items.iter().any(Value::is_object) => {
      let noun = if items.len() == 1 { "item" } else { "items" };
      current.push(Span::raw("["));
      current.push(Span::styled(format!(" … {} {noun} ", items.len()), DIM));
      current.push(Span::raw("]"));
    }
    Value::Array(items) => {
      current.push(Span::raw("["));
      lines.push(Line::new(std::mem::take(current)));
      for (index, item) in items.iter().enumerate() {
        current.push(Span::raw(indent.clone()));
        write_value(item, depth + 1, false, current, lines);
        if index + 1 < items.len() {
          current.push(Span::raw(","));
        }
        lines.push(Line::new(std::mem::take(current)));
      }
      current.push(Span::raw(format!("{close_indent}]")));
    }
    Value::String(_) => current.push(Span::styled(value.to_string(), DONE)),
    Value::Null => current.push(Span::styled("null", DIM)),
    other => current.push(Span::raw(other.to_string())),
  }
}

fn print_body(body: &[u8], context: &Context) -> Result<()> {
  let wants_pretty = context.output.is_json() || context.output.is_stdout_tty();
  if wants_pretty && let Ok(value) = serde_json::from_slice::<Value>(body) {
    return context.output.print_json(&value);
  }

  write_raw(body)
}

fn write_raw(body: &[u8]) -> Result<()> {
  if body.is_empty() {
    return Ok(());
  }

  let mut stdout = std::io::stdout().lock();
  stdout.write_all(body)?;
  if !body.ends_with(b"\n") {
    stdout.write_all(b"\n")?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use reqwest::Url;
  use serde_json::json;

  use super::{build_url, parse_typed_value};

  #[test]
  fn joins_the_path_to_the_base_url() {
    let base = Url::parse("http://x/api").unwrap();
    let join = |path| build_url(&base, path).map(|url| url.to_string());

    assert_eq!(
      join("/v1/contacts/ada%40x.com").unwrap(),
      "http://x/api/v1/contacts/ada%40x.com"
    );
    assert_eq!(
      join("v1/emails?per_page=5").unwrap(),
      "http://x/api/v1/emails?per_page=5"
    );
    assert!(join("https://evil.dev/v1").is_err());
    assert!(join("//evil.dev/v1").is_ok_and(|url| url.starts_with("http://x/api/")));
    assert!(join("../v1").is_err());
  }

  #[test]
  fn types_field_values() {
    assert_eq!(parse_typed_value("42").unwrap(), json!(42));
    assert_eq!(parse_typed_value("1.5").unwrap(), json!(1.5));
    assert_eq!(parse_typed_value("true").unwrap(), json!(true));
    assert_eq!(parse_typed_value("null").unwrap(), json!(null));
    assert_eq!(parse_typed_value("ada").unwrap(), json!("ada"));
  }
}
