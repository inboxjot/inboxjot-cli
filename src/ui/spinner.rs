use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use indicatif::ProgressBar;

use crate::output::get_spinner_style;
use crate::ui::style::{ACTION, hide_cursor, show_cursor};
use crate::ui::text::{Line, Span};

pub const FRAMES: [&str; 9] = ["·", "✢", "✶", "✻", "✽", "✻", "✶", "✢", "✽"];
pub const TICK: Duration = Duration::from_millis(120);
const LOADING_DELAY: Duration = Duration::from_millis(300);

static SHOWN: AtomicUsize = AtomicUsize::new(0);
static COMMAND_PROGRESS: AtomicBool = AtomicBool::new(false);

pub fn get_frames(color: bool) -> Vec<String> {
  FRAMES
    .iter()
    .map(|frame| Line::from(Span::styled(*frame, ACTION)).render(color))
    .collect()
}

#[derive(Debug)]
pub struct Shown(());

impl Shown {
  pub fn new() -> Self {
    COMMAND_PROGRESS.store(true, Ordering::SeqCst);
    Self::loading()
  }

  pub fn loading() -> Self {
    SHOWN.fetch_add(1, Ordering::SeqCst);
    Self(())
  }
}

impl Drop for Shown {
  fn drop(&mut self) {
    SHOWN.fetch_sub(1, Ordering::SeqCst);
  }
}

pub fn can_show_loading() -> bool {
  SHOWN.load(Ordering::SeqCst) == 0 && !COMMAND_PROGRESS.load(Ordering::SeqCst)
}

pub async fn show_loading<T>(
  message: Option<&str>,
  future: impl Future<Output = T>,
) -> T {
  let Some(message) = message else {
    return future.await;
  };
  let mut future = std::pin::pin!(future);
  tokio::select! {
    output = &mut future => return output,
    () = tokio::time::sleep(LOADING_DELAY) => {}
  }
  if !can_show_loading() {
    return future.await;
  }

  let shown = Shown::loading();
  let bar = ProgressBar::new_spinner()
    .with_style(get_spinner_style(&[]))
    .with_message(message.to_owned());
  hide_cursor();
  bar.enable_steady_tick(TICK);
  let output = future.await;
  bar.finish_and_clear();
  show_cursor();
  drop(shown);
  output
}

pub fn get_name_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
  let is_id = value.split_once('_').is_some_and(|(prefix, rest)| {
    !prefix.is_empty()
      && prefix.bytes().all(|byte| byte.is_ascii_lowercase())
      && !rest.is_empty()
      && rest.bytes().all(|byte| byte.is_ascii_alphanumeric())
  });
  if is_id { fallback } else { value }
}

#[cfg(test)]
mod tests {
  use super::get_name_or;

  #[test]
  fn names_readable_values_and_hides_ids() {
    assert_eq!(get_name_or("acme.dev", "the domain"), "acme.dev");
    assert_eq!(get_name_or("riti@maily.to", "the contact"), "riti@maily.to");
    assert_eq!(get_name_or("ct_01ps8bxkc0rkc4bhka87", "the contact"), "the contact");
  }
}
