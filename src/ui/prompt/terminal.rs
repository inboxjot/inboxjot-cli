use std::io::{self, IsTerminal, Write};

use crossterm::cursor::{Hide, MoveToColumn, MoveUp, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};

use crate::error::{Error, Result};
use crate::ui::prompt::{Prompt, Step};
use crate::ui::style::has_color;
use crate::ui::text::Line;

struct RawMode;

impl RawMode {
  fn enable() -> io::Result<Self> {
    terminal::enable_raw_mode()?;
    if let Err(error) = execute!(io::stderr(), Hide) {
      let _ = terminal::disable_raw_mode();
      return Err(error);
    }
    Ok(Self)
  }
}

impl Drop for RawMode {
  fn drop(&mut self) {
    let _ = execute!(io::stderr(), Show);
    let _ = terminal::disable_raw_mode();
  }
}

#[derive(Default)]
struct Screen {
  drawn: usize,
}

impl Screen {
  fn draw(
    &mut self,
    out: &mut impl Write,
    lines: &[Line<'_>],
    width: usize,
  ) -> io::Result<()> {
    self.rewind(out)?;
    let color = has_color();
    let room = width.saturating_sub(1).max(1);
    for (index, line) in lines.iter().enumerate() {
      if index > 0 {
        out.write_all(b"\r\n")?;
      }
      out.write_all(line.clone().truncate(room).render(color).as_bytes())?;
    }
    self.drawn = lines.len().max(1);
    out.flush()
  }

  fn rewind(&mut self, out: &mut impl Write) -> io::Result<()> {
    queue!(out, MoveToColumn(0))?;
    if self.drawn > 1 {
      let up = u16::try_from(self.drawn - 1).unwrap_or(u16::MAX);
      queue!(out, MoveUp(up))?;
    }
    queue!(out, Clear(ClearType::FromCursorDown))?;
    self.drawn = 0;
    Ok(())
  }
}

pub fn run<P: Prompt>(mut prompt: P) -> Result<P::Output> {
  if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
    return Err(Error::usage("not_interactive", "This needs a terminal to answer in."));
  }

  let mut out = io::stderr();
  let mut screen = Screen::default();
  let guard = RawMode::enable()?;

  let result = loop {
    let (width, height) = get_size();
    screen.draw(&mut out, &prompt.view(width, height), width)?;

    match event::read()? {
      Event::Key(key) if key.kind != KeyEventKind::Release => {
        if is_cancel(&key) {
          break None;
        }
        if let Step::Submit(output) = prompt.update(key) {
          break Some(output);
        }
      }
      _ => {}
    }
  };

  screen.rewind(&mut out)?;
  let lines = match &result {
    Some(output) => prompt.summary(output),
    None => {
      let mut lines = prompt.cancelled();
      lines.push(Line::blank());
      lines
    }
  };
  let room = get_size().0.saturating_sub(1).max(1);
  for line in lines {
    write!(out, "{}\r\n", line.truncate(room).render(has_color()))?;
  }
  out.flush()?;
  drop(guard);
  result.ok_or_else(Error::canceled)
}

fn is_cancel(key: &KeyEvent) -> bool {
  match key.code {
    KeyCode::Esc => true,
    KeyCode::Char('c' | 'd') => key.modifiers.contains(KeyModifiers::CONTROL),
    _ => false,
  }
}

fn get_size() -> (usize, usize) {
  terminal::size()
    .map(|(columns, rows)| (usize::from(columns.max(20)), usize::from(rows.max(5))))
    .unwrap_or((80, 24))
}
