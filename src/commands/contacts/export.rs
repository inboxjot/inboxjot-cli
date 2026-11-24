use std::path::PathBuf;
use std::time::Duration;

use clap::Args;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Activity, Job, JobStatus, wait_for_job};
use crate::api::Api;
use crate::commands::segments::resolve_segment_id;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{format_count, format_size};
use crate::time::parse_duration;
use crate::ui::mark::Mark;
use crate::ui::style::{DIM, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

#[derive(Debug, Args)]
pub struct ExportArgs {
  #[arg(
    long,
    value_name = "SEGMENT",
    help = "Export only the contacts in this segment, by name or ID"
  )]
  pub segment: Option<String>,

  #[arg(
    short,
    long,
    value_name = "PATH",
    help = "File to write; defaults to contacts-<date>.csv"
  )]
  pub output: Option<PathBuf>,

  #[arg(
    long,
    default_value = "10m",
    value_parser = parse_duration,
    help = "Stop waiting for the export after this long"
  )]
  pub timeout: Duration,
}

#[derive(Debug, Deserialize)]
struct FinishedExport {
  download_url: String,
  #[serde(default)]
  row_count: u64,
}

pub async fn run(args: ExportArgs, context: &Context) -> Result<()> {
  let api = context.api().await?;
  let segment_id = match &args.segment {
    Some(selector) => Some(resolve_segment_id(&api, selector).await?),
    None => None,
  };
  let (label, total) = if context.output.is_rich() {
    describe_audience(&api, segment_id.as_deref())
      .await
      .unwrap_or_else(|_| ("contacts".to_owned(), None))
  } else {
    ("contacts".to_owned(), None)
  };

  if context.output.is_rich() {
    let mut spans = vec![Span::styled(format!("  Exporting {label}"), TITLE)];
    if let Some(total) = total {
      spans.push(Span::styled(format!("  {} contacts", format_count(total)), SECONDARY));
    }
    anstream::eprintln!("{}", Line::new(spans));
    anstream::eprintln!();
  }
  let progress = context.output.start_progress("contacts", &[]);

  let exports = ["v1", "exports"];
  let body = json!({ "kind": "contacts", "filters": { "segment_id": segment_id } });
  let created = api.post(&exports, &body).await?;
  let id = Job::deserialize(&created)?.id;
  let export = [&exports[..], &[id.as_str()]].concat();
  let describe = move |job: &Value| Activity {
    done: job["row_count"].as_u64().unwrap_or_default(),
    total,
  };
  let (finished, status) =
    wait_for_job(&api, &export, created, &describe, args.timeout, &progress).await?;
  if status == JobStatus::Failed {
    progress.finish();
    return Err(Error::failed("export_failed", "The export failed."));
  }
  let result = FinishedExport::deserialize(&finished)?;

  let path = args.output.unwrap_or_else(|| {
    PathBuf::from(format!("contacts-{}.csv", jiff::Zoned::now().strftime("%Y-%m-%d")))
  });
  let bytes = download(&result.download_url, &path).await?;
  progress.finish();

  if context.output.is_json() {
    return context.output.print_json(&finished);
  }

  let noun = if result.row_count == 1 { "contact" } else { "contacts" };
  let saved =
    format!("Saved {} {noun} to {}", format_count(result.row_count), path.display());
  if !context.output.is_stdout_tty() {
    context.output.print_line(&path.display().to_string());
  }
  if !context.output.is_rich() {
    context.output.success(&saved);
    return Ok(());
  }

  anstream::eprintln!();
  anstream::eprintln!(
    "{}",
    Line::new(vec![
      Span::raw("  "),
      Mark::Done.span(),
      Span::raw(format!(" {saved}")),
      Span::styled(format!("  {}", format_size(bytes.len() as u64)), DIM),
    ])
  );
  let columns = read_columns(&bytes);
  if !columns.is_empty() {
    anstream::eprintln!(
      "{}",
      Line::new(vec![Span::styled(format!("    {}", columns.join(", ")), DIM)])
    );
  }
  Ok(())
}

async fn describe_audience(
  api: &Api,
  segment_id: Option<&str>,
) -> Result<(String, Option<u64>)> {
  let Some(id) = segment_id else {
    let page = api.get(&["v1", "contacts"], &json!({ "limit": 1 })).await?;
    return Ok(("all contacts".to_owned(), page["total"].as_u64()));
  };
  let segment = api.get(&["v1", "segments", id], &()).await?;
  let name = segment["name"].as_str().unwrap_or(id).to_owned();
  Ok((name, segment["contact_count"].as_u64()))
}

fn read_columns(bytes: &[u8]) -> Vec<String> {
  let text = String::from_utf8_lossy(bytes);
  csv::ReaderBuilder::new()
    .from_reader(text.trim_start_matches('\u{feff}').as_bytes())
    .headers()
    .map(|headers| headers.iter().map(str::to_owned).collect())
    .unwrap_or_default()
}

async fn download(url: &str, path: &PathBuf) -> Result<Vec<u8>> {
  let response = reqwest::get(url).await?;
  let status = response.status();
  if !status.is_success() {
    return Err(Error::failed(
      "download_failed",
      format!("The download failed with status {}.", status.as_u16()),
    ));
  }

  let bytes = response.bytes().await?.to_vec();
  tokio::fs::write(path, &bytes).await?;
  Ok(bytes)
}
