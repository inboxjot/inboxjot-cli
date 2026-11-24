use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use super::{EmailArgs, get_id};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column};

const ATTACHMENT_COLUMNS: [Column; 5] = [
  column("ID", "id"),
  column("Filename", "filename"),
  column("Type", "content_type"),
  column("Size", "size"),
  column("Disposition", "content_disposition"),
];

#[derive(Debug, Subcommand)]
pub enum AttachmentsCommand {
  #[command(
    about = "List the attachments of an email",
    after_help = examples!("inboxjot emails attachments list eml_123"),
    alias = "ls",
  )]
  List(EmailArgs),
  #[command(
    about = "Download the attachments of an email",
    after_help = examples!("inboxjot emails attachments download eml_123 --output ./files"),
  )]
  Download(DownloadArgs),
}

#[derive(Debug, Args)]
pub struct DownloadArgs {
  #[arg(value_name = "ID", help = "Email ID, such as eml_123")]
  pub id: String,

  #[arg(
    short,
    long,
    value_name = "DIR",
    default_value = ".",
    help = "Folder to save the files in"
  )]
  pub output: PathBuf,
}

pub async fn run(command: AttachmentsCommand, context: &Context) -> Result<()> {
  match command {
    AttachmentsCommand::List(args) => list(&args, context).await,
    AttachmentsCommand::Download(args) => download(&args, context).await,
  }
}

async fn list(args: &EmailArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading attachments…");
  let attachments = api.get(&["v1", "emails", &args.id, "attachments"], &()).await?;

  context
    .output
    .print_list(&attachments, &ATTACHMENT_COLUMNS, "attachments", None)
}

async fn download(args: &DownloadArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Downloading the attachment…");
  let attachments = api.get(&["v1", "emails", &args.id, "attachments"], &()).await?;
  let items = attachments
    .get("data")
    .and_then(Value::as_array)
    .cloned()
    .unwrap_or_default();
  std::fs::create_dir_all(&args.output)?;

  let mut saved = Vec::new();
  for attachment in &items {
    let filename = attachment.get("filename").and_then(Value::as_str).unwrap_or_default();
    let Some(url) = attachment.get("download_url").and_then(Value::as_str) else {
      context
        .output
        .warn(&format!("{filename} was deleted by data retention."));
      continue;
    };

    let bytes = download_file(url).await?;
    let (path, mut file) =
      create_unique_file(&args.output, &get_safe_filename(filename, attachment))?;
    file.write_all(&bytes)?;
    saved.push(json!({ "id": attachment.get("id"), "filename": filename, "path": path }));
  }

  if context.output.is_json() {
    return context.output.print_json(&json!({ "data": saved }));
  }

  for file in &saved {
    context.output.print_line(file["path"].as_str().unwrap_or_default());
  }
  context
    .output
    .success(&format!("Downloaded {} attachments", saved.len()));
  Ok(())
}

async fn download_file(url: &str) -> Result<Vec<u8>> {
  let response = reqwest::get(url).await?;
  let status = response.status();
  if !status.is_success() {
    let message =
      format!("The attachment download failed with status {}.", status.as_u16());
    return Err(Error::failed("download_failed", message));
  }

  Ok(response.bytes().await?.to_vec())
}

fn get_safe_filename(filename: &str, attachment: &Value) -> String {
  let get_last_part = |name: &str| {
    Path::new(name)
      .file_name()
      .and_then(|name| name.to_str())
      .map(str::to_owned)
  };
  let name = get_last_part(filename)
    .or_else(|| get_last_part(get_id(attachment)))
    .unwrap_or_else(|| "attachment".to_owned());

  if name.starts_with('.') { format!("_{name}") } else { name }
}

fn create_unique_file(folder: &Path, name: &str) -> Result<(PathBuf, File)> {
  let path = Path::new(name);
  let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or(name);
  let extension = path.extension().and_then(|extension| extension.to_str());

  for attempt in 0..1000 {
    let candidate = match (attempt, extension) {
      (0, _) => name.to_owned(),
      (_, Some(extension)) => format!("{stem} ({attempt}).{extension}"),
      (_, None) => format!("{stem} ({attempt})"),
    };
    let path = folder.join(candidate);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
      Ok(file) => return Ok((path, file)),
      Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
      Err(error) => return Err(error.into()),
    }
  }

  Err(Error::failed("download_failed", format!("Too many files are named {name}.")))
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{create_unique_file, get_safe_filename};

  #[test]
  fn keeps_names_inside_the_folder_and_hides_no_dotfiles() {
    let attachment = json!({ "id": "att_1" });

    assert_eq!(get_safe_filename("../../etc/passwd", &attachment), "passwd");
    assert_eq!(get_safe_filename(".bashrc", &attachment), "_.bashrc");
    assert_eq!(get_safe_filename("..", &attachment), "att_1");
    assert_eq!(get_safe_filename("report.pdf", &attachment), "report.pdf");
  }

  #[test]
  fn never_replaces_an_existing_file() {
    let folder = tempfile::tempdir().expect("a temp folder");
    std::fs::write(folder.path().join("report.pdf"), "ORIGINAL")
      .expect("the file is written");

    let (path, _) = create_unique_file(folder.path(), "report.pdf").expect("a new file");

    assert_eq!(path, folder.path().join("report (1).pdf"));
    assert_eq!(
      std::fs::read_to_string(folder.path().join("report.pdf")).expect("the file reads"),
      "ORIGINAL"
    );
  }
}
