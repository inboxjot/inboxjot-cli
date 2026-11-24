use std::path::PathBuf;

use clap::{Args, Subcommand};
use inboxjot::ListImages;
use serde_json::{Value, json};

use anstyle::Style;
use indicatif::{ProgressBar, ProgressStyle};
use unicode_width::UnicodeWidthStr;

use crate::api::Api;
use crate::cli::ListArgs;
use crate::context::Context;
use crate::error::{Error, Result};
use crate::output::{Column, column, format_size};
use crate::ui::bar::render_thin;
use crate::ui::mark::Mark;
use crate::ui::spinner::{self, Shown};
use crate::ui::style::{ACTION, DANGER, SECONDARY, has_color, hide_cursor, show_cursor};
use crate::ui::text::{Line, Span};

const BAR_WIDTH: usize = 28;

const LIST_COLUMNS: [Column; 5] = [
  column("ID", "id"),
  column("File", "file_name"),
  column("Type", "content_type"),
  column("Bytes", "size_bytes"),
  column("Created", "created_at"),
];

const DETAIL_ROWS: [Column; 9] = [
  column("File", "file_name"),
  column("ID", "id"),
  column("URL", "url"),
  column("Type", "content_type"),
  column("Bytes", "size_bytes"),
  column("Width", "width"),
  column("Height", "height"),
  column("Uploaded by", "creator_name"),
  column("Created", "created_at"),
];

#[derive(Debug, Subcommand)]
pub enum ImagesCommand {
  #[command(
    about = "Upload PNG, JPEG or GIF images and print their public URLs",
    after_help = examples!("inboxjot images upload hero.png logo.png"),
  )]
  Upload(UploadArgs),
  #[command(
    about = "List images",
    after_help = examples!("inboxjot images list"),
    alias = "ls",
  )]
  List(ListImagesArgs),
  #[command(
    about = "Show an image",
    after_help = examples!("inboxjot images inspect img_06d2mfk8r3wn5gt7kc2pb"),
  )]
  Inspect(ImageArgs),
  #[command(
    about = "Delete an image",
    after_help = examples!("inboxjot images delete img_06d2mfk8r3wn5gt7kc2pb"),
    alias = "rm",
  )]
  Delete(ImageArgs),
}

#[derive(Debug, Args)]
pub struct UploadArgs {
  #[arg(value_name = "FILE", required = true, help = "Image files, such as logo.png")]
  pub files: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ListImagesArgs {
  #[arg(long, help = "Show only images whose file name has this text")]
  pub query: Option<String>,

  #[command(flatten)]
  pub list: ListArgs,
}

#[derive(Debug, Args)]
pub struct ImageArgs {
  #[arg(value_name = "IMAGE", help = "Image ID, such as img_06d2mfk8r3wn5gt7kc2pb")]
  pub image: String,
}

pub async fn run(command: ImagesCommand, context: &Context) -> Result<()> {
  match command {
    ImagesCommand::Upload(args) => upload(&args, context).await,
    ImagesCommand::List(args) => list(&args, context).await,
    ImagesCommand::Inspect(args) => inspect(&args, context).await,
    ImagesCommand::Delete(args) => delete(&args, context).await,
  }
}

struct ImageFile {
  name: String,
  bytes: Vec<u8>,
  content_type: &'static str,
}

async fn upload(args: &UploadArgs, context: &Context) -> Result<()> {
  let files = args.files.iter().map(read_image).collect::<Result<Vec<_>>>()?;
  let api = context.api().await?;
  let width = files.iter().map(|file| file.name.width()).max().unwrap_or(0).max(10) + 2;
  let color = has_color();

  let mut images = Vec::new();
  let mut failure = None;
  for file in files {
    let size = format_size(file.bytes.len() as u64);
    let _shown = context.output.is_rich().then(Shown::new);
    let live = context.output.is_rich().then(|| {
      let frames = spinner::get_frames(color);
      let frames: Vec<&str> = frames.iter().map(String::as_str).collect();
      let style = ProgressStyle::with_template("  {spinner} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_strings(&frames);
      let bar = ProgressBar::new_spinner().with_style(style);
      hide_cursor();
      bar.enable_steady_tick(spinner::TICK);
      bar
    });
    let total = file.bytes.len() as u64;
    let name = file.name.clone();
    let progress = live.clone();
    let render = move |sent: u64| {
      let fraction = if total == 0 { 1.0 } else { sent as f64 / total as f64 };
      let mut spans = vec![
        Span::raw(format!("{name:<width$}")),
        Span::styled(format!("{size:>7}  "), SECONDARY),
      ];
      spans.extend(render_thin(BAR_WIDTH, fraction, Style::new()));
      spans.push(Span::raw(format!("  {:.0}%", fraction * 100.0)));
      Line::new(spans).render(color)
    };
    if let Some(bar) = &live {
      bar.set_message(render(0));
    }
    let result =
      upload_image(&api, file.bytes, file.content_type, &file.name, move |sent| {
        if let Some(bar) = &progress {
          bar.set_message(render(sent));
        }
      })
      .await;
    if let Some(bar) = live {
      bar.finish_and_clear();
      show_cursor();
    }

    match result {
      Ok(image) => {
        print_uploaded(&image, &file.name, width, context);
        images.push(image);
      }
      Err(error) => {
        if context.output.is_rich() {
          anstream::eprintln!(
            "{}",
            Line::new(vec![
              Span::raw("  "),
              Mark::Error.span(),
              Span::raw(format!(" {:<width$}", file.name)),
              Span::styled(error.message.clone(), DANGER),
            ])
          );
        }
        failure.get_or_insert(error);
      }
    }
  }

  if context.output.is_json() {
    let value = if images.len() == 1 { images.remove(0) } else { Value::Array(images) };
    context.output.print_json(&value)?;
  }
  match failure {
    Some(mut error) if context.output.is_rich() => {
      error.kind = crate::error::Kind::Quiet;
      Err(error)
    }
    Some(error) => Err(error),
    None => Ok(()),
  }
}

fn read_image(path: &PathBuf) -> Result<ImageFile> {
  let shown = path.display();
  let bytes = std::fs::read(path).map_err(|error| {
    Error::usage("unreadable_file", format!("Could not read {shown}: {error}"))
  })?;
  let content_type = get_image_content_type(&bytes).ok_or_else(|| {
    Error::usage("unsupported_image", format!("{shown} is not a PNG, JPEG or GIF image."))
  })?;
  let name = path
    .file_name()
    .map_or_else(|| shown.to_string(), |name| name.to_string_lossy().into_owned());
  Ok(ImageFile { name, bytes, content_type })
}

fn print_uploaded(image: &Value, name: &str, width: usize, context: &Context) {
  let url = image.get("url").and_then(Value::as_str).unwrap_or_default();
  if !context.output.is_stdout_tty() && !context.output.is_json() {
    context.output.print_line(url);
  }
  if !context.output.is_rich() {
    context.output.success(&format!("Uploaded {name}"));
    if context.output.is_stdout_tty() {
      context.output.print_line(url);
    }
    return;
  }

  let size = image
    .get("size_bytes")
    .and_then(Value::as_u64)
    .map(format_size)
    .unwrap_or_default();
  let dimensions = match (
    image.get("width").and_then(Value::as_u64),
    image.get("height").and_then(Value::as_u64),
  ) {
    (Some(width), Some(height)) => format!("{width}×{height}"),
    _ => String::new(),
  };
  anstream::eprintln!(
    "{}",
    Line::new(vec![
      Span::raw("  "),
      Mark::Done.span(),
      Span::raw(format!(" {name:<width$}")),
      Span::styled(format!("{size:>7}  {dimensions:<10}"), SECONDARY),
      Span::styled(url.to_owned(), ACTION),
    ])
  );
}

async fn upload_image(
  api: &Api,
  bytes: Vec<u8>,
  content_type: &str,
  file_name: &str,
  on_progress: impl Fn(u64) + Send + Sync + 'static,
) -> Result<Value> {
  let presign = json!({ "content_type": content_type, "size": bytes.len() });
  let target = api.post(&["v1", "images", "presign"], &presign).await?;
  let field = |key: &str| target.get(key).and_then(Value::as_str).unwrap_or_default();

  api
    .upload_with_progress(field("upload_url"), bytes, content_type, on_progress)
    .await?;

  let body = json!({ "key": field("key"), "file_name": file_name });
  api.post(&["v1", "images"], &body).await
}

async fn list(args: &ListImagesArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading images…");
  let query = ListImages { query: args.query.clone(), ..Default::default() };
  let page = api.list(&["v1", "images"], &query, &args.list.to_page()).await?;

  context.output.print_list(&page, &LIST_COLUMNS, "images", None)
}

async fn inspect(args: &ImageArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Loading the image…");
  let image = api.get(&["v1", "images", &args.image], &()).await?;

  context.output.print_detail(&image, &DETAIL_ROWS)
}

async fn delete(args: &ImageArgs, context: &Context) -> Result<()> {
  let api = context.api().await?.loading("Deleting the image…");
  let image = api.get(&["v1", "images", &args.image], &()).await?;
  let name = image
    .get("file_name")
    .and_then(Value::as_str)
    .unwrap_or_default()
    .to_owned();

  context
    .confirm_delete(&format!("Delete {name}?"), "Emails that show it lose the image.")?;

  api.delete(&["v1", "images", &args.image]).await?;
  let result = json!({ "id": args.image, "file_name": name, "deleted": true });
  context
    .output
    .print_removal(&result, &format!("Deleted {name}"), "id", &[])
}

fn get_image_content_type(bytes: &[u8]) -> Option<&'static str> {
  if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
    Some("image/png")
  } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
    Some("image/jpeg")
  } else if bytes.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
    Some("image/gif")
  } else {
    None
  }
}
