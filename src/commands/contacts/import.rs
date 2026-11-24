use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Args;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Activity, Job, JobStatus, wait_for_job};
use crate::api::Api;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column, format_count};
use crate::time::parse_duration;
use crate::ui::mark::Mark;
use crate::ui::style::{DIM, FAINT, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

const EMAIL_NAMES: [&str; 4] = ["email", "email_address", "e_mail", "mail"];
const FIRST_NAME_NAMES: [&str; 5] =
  ["first_name", "firstname", "first", "given_name", "fname"];
const LAST_NAME_NAMES: [&str; 6] =
  ["last_name", "lastname", "last", "surname", "family_name", "lname"];
const MAPPING_COLUMNS: [Column; 2] =
  [column("Column", "column"), column("Field", "field")];
const MAX_SHOWN_ERRORS: usize = 5;

#[derive(Debug, Args)]
pub struct ImportArgs {
  #[arg(value_name = "FILE", help = "CSV file with a header row")]
  pub file: PathBuf,

  #[arg(
    long,
    value_name = "SEGMENT",
    help = "Add the contacts to this static segment ID"
  )]
  pub segment: Option<String>,

  #[arg(long, help = "Print how the columns map to contact fields and stop")]
  pub dry_run: bool,

  #[arg(
    long,
    default_value = "10m",
    value_parser = parse_duration,
    help = "Stop waiting for the import after this long"
  )]
  pub timeout: Duration,
}

#[derive(Debug, Deserialize)]
struct UploadTarget {
  upload_url: String,
  key: String,
}

#[derive(Debug, Deserialize)]
struct FinishedImport {
  #[serde(default)]
  errors: Vec<ImportError>,
}

#[derive(Debug, Deserialize)]
struct ImportError {
  row: u64,
  #[serde(default)]
  email: Option<String>,
  reason: String,
}

pub async fn run(args: ImportArgs, context: &Context) -> Result<()> {
  let file_name = args
    .file
    .file_name()
    .unwrap_or_default()
    .to_string_lossy()
    .into_owned();
  let bytes = tokio::fs::read(&args.file).await.map_err(|error| {
    Error::usage(
      "unreadable_file",
      format!("Cannot read {}: {error}.", args.file.display()),
    )
  })?;
  let text = String::from_utf8_lossy(&bytes);
  let headers = read_header(&text);

  if args.dry_run {
    let mapping = map_columns(&headers, None);
    return print_mapping(&mapping, context);
  }

  let api = context.api().await?;
  let property_keys = list_property_keys(&api).await?;
  let mapping = map_columns(&headers, Some(&property_keys));
  if !mapping.iter().any(|(_, field)| field.as_deref() == Some("email")) {
    return Err(Error::usage(
      "missing_email_column",
      "No column holds the email address. Name one column email.",
    ));
  }

  let total_rows = count_rows(&text);
  let started = Instant::now();
  if context.output.is_rich() {
    anstream::eprintln!(
      "{}",
      Line::new(vec![
        Span::styled(format!("  Importing {file_name}"), TITLE),
        Span::styled(
          format!(
            "  {} rows · {} columns",
            format_count(total_rows as u64),
            format_count(headers.len() as u64)
          ),
          SECONDARY
        ),
      ])
    );
    anstream::eprintln!();
  }
  let mut below = render_mapping(&mapping);
  below.push(Line::blank());
  below.push(Line::new(vec![
    Span::styled("  ctrl+c", DIM),
    Span::styled(" stop watching · the import keeps going", FAINT),
  ]));
  let progress = context.output.start_progress("rows", &below);

  let imports = ["v1", "contact-imports"];
  let presign = [&imports[..], &["presign"]].concat();
  let target = api
    .post(&presign, &json!({ "content_type": "text/csv", "size": bytes.len() }))
    .await?;
  let target = UploadTarget::deserialize(&target)?;
  api.upload(&target.upload_url, bytes.clone(), "text/csv").await?;

  let body = json!({
    "file_key": target.key,
    "file_name": file_name,
    "total_rows": total_rows,
    "mapping": to_mapping_body(&mapping),
    "segment_id": args.segment,
  });
  let created = api.post(&imports, &body).await?;
  let id = Job::deserialize(&created)?.id;
  let import = [&imports[..], &[id.as_str()]].concat();
  let (finished, status) =
    wait_for_job(&api, &import, created, &describe_progress, args.timeout, &progress)
      .await?;
  progress.finish();

  if status == JobStatus::Failed {
    return Err(Error::failed(
      "import_failed",
      format!("The import of {file_name} failed."),
    ));
  }

  print_counts(&finished, &mapping, started.elapsed(), context)
}

fn render_mapping(mapping: &[(String, Option<String>)]) -> Vec<Line<'static>> {
  let width = mapping.iter().map(|(column, _)| column.width()).max().unwrap_or(0) + 2;
  mapping
    .iter()
    .map(|(column, field)| {
      let name = format!("{column:<width$}");
      match field {
        Some(field) => Line::new(vec![
          Span::raw("  "),
          Mark::Done.span(),
          Span::raw(format!(" {name}")),
          Span::styled("→ ", DIM),
          Span::raw(field.clone()),
        ]),
        None => Line::new(vec![
          Span::raw("  "),
          Mark::Removed.span(),
          Span::styled(format!(" {name}"), SECONDARY),
          Span::styled("→ skipped", DIM),
        ]),
      }
    })
    .collect()
}

fn print_mapping(mapping: &[(String, Option<String>)], context: &Context) -> Result<()> {
  if context.output.is_json() {
    return context.output.print_json(&to_mapping_body(mapping));
  }

  if !context.output.is_rich() {
    context
      .output
      .print_table(&to_mapping_rows(mapping), &MAPPING_COLUMNS);
    return Ok(());
  }

  for line in render_mapping(mapping) {
    anstream::eprintln!("{line}");
  }
  anstream::eprintln!();
  anstream::eprintln!(
    "{}",
    Line::from(Span::styled(
      "  Other columns become properties only when the workspace has that property.",
      SECONDARY
    ))
  );
  Ok(())
}

fn print_counts(
  import: &Value,
  mapping: &[(String, Option<String>)],
  took: Duration,
  context: &Context,
) -> Result<()> {
  if context.output.is_json() {
    return context.output.print_json(import);
  }

  let count = |key: &str| import[key].as_u64().unwrap_or_default();
  let (created, updated, skipped) =
    (count("created_count"), count("updated_count"), count("skipped_count"));
  let total = created + updated;
  let noun = if total == 1 { "contact" } else { "contacts" };
  let imported = format!("Imported {} {noun}", format_count(total));
  if !context.output.is_stdout_tty()
    && let Some(id) = import.get("id").and_then(Value::as_str)
  {
    context.output.print_line(id);
  }

  if !context.output.is_rich() {
    context.output.success(&imported);
  } else {
    anstream::eprintln!();
    for line in render_mapping(mapping) {
      anstream::eprintln!("{line}");
    }
    anstream::eprintln!();
    anstream::eprintln!(
      "{}",
      Line::new(vec![
        Span::raw("  "),
        Mark::Done.span(),
        Span::raw(format!(" {imported}")),
        Span::styled(format!("  in {}", format_seconds(took.as_secs_f64())), DIM),
      ])
    );
  }
  if updated > 0 || skipped > 0 {
    context.output.info(&format!(
      "{} new · {} updated · {} skipped",
      format_count(created),
      format_count(updated),
      format_count(skipped)
    ));
  }

  let finished = FinishedImport::deserialize(import)?;
  for error in finished.errors.iter().take(MAX_SHOWN_ERRORS) {
    let label = match error.email.as_deref().filter(|email| !email.is_empty()) {
      Some(email) => format!("Row {} ({email})", error.row),
      None => format!("Row {}", error.row),
    };
    context.output.info(&format!("{label}: {}", error.reason));
  }

  Ok(())
}

fn format_seconds(seconds: f64) -> String {
  if seconds < 60.0 {
    return format!("{seconds:.1}s");
  }
  let whole = seconds.round() as u64;
  format!("{}m {}s", whole / 60, whole % 60)
}

fn describe_progress(import: &Value) -> Activity {
  let count = |key: &str| import[key].as_u64().unwrap_or_default();
  let total = count("total_rows");
  Activity {
    done: count("processed_rows"),
    total: (total > 0).then_some(total),
  }
}

async fn list_property_keys(api: &Api) -> Result<Vec<String>> {
  let properties = api.get(&["v1", "contact-properties"], &()).await?;
  let properties = properties.as_array().map_or(&[][..], Vec::as_slice);

  Ok(
    properties
      .iter()
      .filter_map(|property| property["key"].as_str().map(str::to_owned))
      .collect(),
  )
}

fn map_columns(
  headers: &[String],
  property_keys: Option<&[String]>,
) -> Vec<(String, Option<String>)> {
  let mut mapping: Vec<(String, Option<String>)> = Vec::with_capacity(headers.len());

  for header in headers {
    let key = normalize_header(header);
    let field = match key.as_str() {
      name if EMAIL_NAMES.contains(&name) => Some("email".to_owned()),
      name if FIRST_NAME_NAMES.contains(&name) => Some("first_name".to_owned()),
      name if LAST_NAME_NAMES.contains(&name) => Some("last_name".to_owned()),
      _ => match property_keys {
        Some(keys) => keys.contains(&key).then_some(key),
        None => is_property_key(&key).then_some(key),
      },
    };
    let is_taken = mapping.iter().any(|(_, used)| used.is_some() && *used == field);
    mapping.push((header.clone(), field.filter(|_| !is_taken)));
  }

  mapping
}

fn to_mapping_body(mapping: &[(String, Option<String>)]) -> Value {
  mapping
    .iter()
    .map(|(column, field)| (column.clone(), json!(field)))
    .collect::<Map<String, Value>>()
    .into()
}

fn to_mapping_rows(mapping: &[(String, Option<String>)]) -> Vec<Value> {
  mapping
    .iter()
    .map(|(column, field)| {
      let field = match field.as_deref() {
        None => "skipped".to_owned(),
        Some(core @ ("email" | "first_name" | "last_name")) => core.to_owned(),
        Some(key) => format!("property {key}"),
      };
      json!({ "column": column, "field": field })
    })
    .collect()
}

fn normalize_header(header: &str) -> String {
  let lowered = header.trim().to_lowercase();
  let words: Vec<&str> = lowered
    .split(|char: char| !char.is_ascii_alphanumeric())
    .filter(|word| !word.is_empty())
    .collect();

  words.join("_")
}

fn is_property_key(key: &str) -> bool {
  key.len() <= 64 && key.starts_with(|char: char| char.is_ascii_lowercase())
}

fn read_header(text: &str) -> Vec<String> {
  build_reader(text)
    .headers()
    .map(|headers| headers.iter().map(str::to_owned).collect())
    .unwrap_or_default()
}

fn count_rows(text: &str) -> usize {
  build_reader(text)
    .records()
    .filter_map(std::result::Result::ok)
    .filter(|record| record.iter().any(|field| !field.trim().is_empty()))
    .count()
}

fn build_reader(text: &str) -> csv::Reader<&[u8]> {
  csv::ReaderBuilder::new()
    .flexible(true)
    .from_reader(text.trim_start_matches('\u{feff}').as_bytes())
}

#[cfg(test)]
mod tests {
  use super::{count_rows, map_columns, read_header};

  #[test]
  fn reads_a_quoted_header_and_counts_rows() {
    let text = "\u{feff}\"E-mail\",\"Notes, long\",Plan\r\na@x.dev,\"two\nlines\",pro\r\n,,\r\nb@x.dev,,\n";
    assert_eq!(read_header(text), ["E-mail", "Notes, long", "Plan"]);
    assert_eq!(count_rows(text), 2);
  }

  #[test]
  fn maps_columns_by_name() {
    let headers = ["Email Address", "First Name", "Surname", "Plan", "Email", "Color"]
      .map(str::to_owned);
    let keys = ["plan".to_owned()];
    let mapping = map_columns(&headers, Some(&keys));
    let fields: Vec<Option<&str>> =
      mapping.iter().map(|(_, field)| field.as_deref()).collect();

    assert_eq!(
      fields,
      [Some("email"), Some("first_name"), Some("last_name"), Some("plan"), None, None]
    );
  }
}
