use anstyle::Style;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::ui::prompt::{Prompt, Step};
use crate::ui::style::{DANGER, DIM, TITLE};
use crate::ui::text::{Line, Span};

type Validate = Box<dyn Fn(&str) -> Result<(), String>>;

pub struct Input {
  prompt: String,
  value: String,
  error: Option<String>,
  validate: Validate,
}

impl Input {
  pub fn new(prompt: impl Into<String>) -> Self {
    Self {
      prompt: prompt.into(),
      value: String::new(),
      error: None,
      validate: Box::new(|_| Ok(())),
    }
  }

  pub fn with_validate(
    mut self,
    validate: impl Fn(&str) -> Result<(), String> + 'static,
  ) -> Self {
    self.validate = Box::new(validate);
    self
  }

  fn delete_word(&mut self) {
    let trimmed = self.value.trim_end_matches(char::is_whitespace);
    let start = trimmed.rfind(char::is_whitespace).map_or(0, |index| index + 1);
    self.value.truncate(start);
  }
}

impl Prompt for Input {
  type Output = String;

  fn update(&mut self, key: KeyEvent) -> Step<String> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
      KeyCode::Enter => {
        let value = self.value.trim().to_owned();
        match (self.validate)(&value) {
          Ok(()) => return Step::Submit(value),
          Err(message) => self.error = Some(message),
        }
      }
      KeyCode::Backspace => {
        self.value.pop();
        self.error = None;
      }
      KeyCode::Char('u') if control => {
        self.value.clear();
        self.error = None;
      }
      KeyCode::Char('w') if control => {
        self.delete_word();
        self.error = None;
      }
      KeyCode::Char(character) if !control && !character.is_control() => {
        self.value.push(character);
        self.error = None;
      }
      _ => {}
    }
    Step::Continue
  }

  fn view(&self, _width: usize, _height: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::new(vec![
      Span::raw("  "),
      Span::styled(self.prompt.clone(), TITLE),
      Span::raw(" "),
      Span::raw(self.value.clone()),
      Span::styled(" ", Style::new().invert()),
    ])];
    if let Some(error) = &self.error {
      lines.push(Line::new(vec![Span::raw("  "), Span::styled(error.clone(), DANGER)]));
    }
    lines
  }

  fn summary(&self, output: &String) -> Vec<Line<'static>> {
    vec![Line::new(vec![
      Span::styled(format!("  {}", self.prompt), TITLE),
      Span::raw(" "),
      Span::raw(output.clone()),
    ])]
  }

  fn title(&self) -> Line<'static> {
    Line::new(vec![Span::styled(format!("  {}", self.prompt), TITLE)])
  }

  fn cancelled(&self) -> Vec<Line<'static>> {
    vec![Line::new(vec![
      Span::styled(format!("  {}", self.prompt), TITLE),
      Span::raw(" "),
      Span::raw(self.value.clone()),
      Span::styled("^C", DIM),
    ])]
  }
}

#[cfg(test)]
mod tests {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

  use super::Input;
  use crate::ui::prompt::{Prompt, Step};

  fn type_text(input: &mut Input, text: &str) {
    for character in text.chars() {
      let _ = input.update(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
  }

  fn enter(input: &mut Input) -> Option<String> {
    match input.update(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
      Step::Submit(value) => Some(value),
      Step::Continue => None,
    }
  }

  #[test]
  fn keeps_asking_until_the_value_is_valid() {
    let mut input = Input::new("Type 12408 to send:").with_validate(|value| {
      if value == "12408" { Ok(()) } else { Err("Type the number exactly.".to_owned()) }
    });
    type_text(&mut input, "124");
    assert_eq!(enter(&mut input), None);
    assert!(input.view(80, 24)[1].render(false).contains("exactly"));

    type_text(&mut input, "08");
    assert_eq!(enter(&mut input), Some("12408".to_owned()));
  }

  #[test]
  fn edits_like_a_shell() {
    let mut input = Input::new("Name:");
    type_text(&mut input, "acme prod");
    let _ = input.update(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(input.value, "acme ");
    let _ = input.update(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(input.value, "acme");
    let _ = input.update(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(input.value, "");
  }

  #[test]
  fn trims_the_answer() {
    let mut input = Input::new("Name:");
    type_text(&mut input, "  acme  ");
    assert_eq!(enter(&mut input), Some("acme".to_owned()));
  }
}
