use crossterm::event::{KeyCode, KeyEvent};

use crate::ui::prompt::{Prompt, Step};
use crate::ui::style::{ACTION, DIM, DONE, FAINT, SECONDARY, TITLE};
use crate::ui::text::{Line, Span};

pub struct Select {
  title: String,
  note: String,
  choices: Vec<String>,
  cursor: usize,
}

impl Select {
  pub fn new(title: impl Into<String>, choices: Vec<String>, cursor: usize) -> Self {
    let cursor = cursor.min(choices.len().saturating_sub(1));
    Self {
      title: title.into(),
      note: String::new(),
      choices,
      cursor,
    }
  }

  pub fn with_note(mut self, note: impl Into<String>) -> Self {
    self.note = note.into();
    self
  }

  fn render_head(&self) -> Vec<Line<'static>> {
    let mut lines = vec![self.title()];
    if !self.note.is_empty() {
      lines.push(Line::new(vec![Span::styled(format!("  {}", self.note), SECONDARY)]));
    }
    lines
  }

  fn render_choice(&self, index: usize) -> Line<'static> {
    let label = self.choices[index].clone();
    if index == self.cursor {
      return Line::new(vec![Span::styled("  > ", ACTION), Span::styled(label, DONE)]);
    }
    Line::new(vec![Span::styled(format!("    {label}"), SECONDARY)])
  }
}

impl Prompt for Select {
  type Output = usize;

  fn update(&mut self, key: KeyEvent) -> Step<usize> {
    match key.code {
      KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
      KeyCode::Down | KeyCode::Char('j') => {
        self.cursor = (self.cursor + 1).min(self.choices.len().saturating_sub(1));
      }
      KeyCode::Enter if !self.choices.is_empty() => return Step::Submit(self.cursor),
      _ => {}
    }
    Step::Continue
  }

  fn view(&self, _width: usize, _height: usize) -> Vec<Line<'static>> {
    let mut lines = self.render_head();
    lines.extend((0..self.choices.len()).map(|index| self.render_choice(index)));
    lines.push(Line::blank());
    lines.push(Line::new(vec![
      Span::styled("  ↑↓", DIM),
      Span::styled(" move · ", FAINT),
      Span::styled("enter", DIM),
      Span::styled(" pick", FAINT),
    ]));
    lines
  }

  fn summary(&self, output: &usize) -> Vec<Line<'static>> {
    let mut lines = self.render_head();
    lines.push(Line::new(vec![
      Span::styled("  > ", ACTION),
      Span::styled(self.choices[*output].clone(), DONE),
    ]));
    lines
  }

  fn title(&self) -> Line<'static> {
    Line::new(vec![Span::styled(format!("  {}", self.title), TITLE)])
  }
}

#[cfg(test)]
mod tests {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

  use super::Select;
  use crate::ui::prompt::{Prompt, Step};

  fn press(select: &mut Select, code: KeyCode) -> Option<usize> {
    match select.update(KeyEvent::new(code, KeyModifiers::NONE)) {
      Step::Submit(index) => Some(index),
      Step::Continue => None,
    }
  }

  fn sdks() -> Select {
    let choices = ["Node.js", "Python", "cURL"].map(str::to_owned).to_vec();
    Select::new("Which SDK should the snippet use?", choices, 0)
      .with_note("Found package.json in this folder.")
  }

  #[test]
  fn starts_on_the_suggested_choice_and_moves_within_the_list() {
    let mut select = sdks();
    let _ = press(&mut select, KeyCode::Up);
    assert_eq!(press(&mut select, KeyCode::Enter), Some(0));
    for _ in 0..5 {
      let _ = press(&mut select, KeyCode::Down);
    }
    assert_eq!(press(&mut select, KeyCode::Enter), Some(2));
  }

  #[test]
  fn keeps_the_question_and_the_answer() {
    let lines: Vec<String> =
      sdks().summary(&0).iter().map(|line| line.render(false)).collect();
    assert_eq!(
      lines,
      [
        "  Which SDK should the snippet use?",
        "  Found package.json in this folder.",
        "  > Node.js",
      ]
    );
  }
}
