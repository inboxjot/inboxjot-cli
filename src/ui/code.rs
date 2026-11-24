use anstyle::{AnsiColor, Style};

use crate::ui::style::DONE;
use crate::ui::text::{Line, Span};

const KEYWORD: Style = AnsiColor::Magenta.on_default();
const CALL: Style = AnsiColor::Blue.on_default();
const KEYWORDS: [&str; 10] =
  ["import", "from", "const", "let", "new", "await", "use", "async", "return", "func"];

pub fn highlight(line: &str) -> Line<'static> {
  let characters: Vec<char> = line.chars().collect();
  let mut spans = Vec::new();
  let mut plain = String::new();
  let mut index = 0;

  while index < characters.len() {
    let character = characters[index];
    if character == '\'' || character == '"' {
      let end = find_closing(&characters, index);
      flush(&mut spans, &mut plain);
      spans.push(Span::styled(characters[index..end].iter().collect::<String>(), DONE));
      index = end;
      continue;
    }

    if character.is_alphabetic() || character == '_' {
      let start = index;
      while index < characters.len()
        && (characters[index].is_alphanumeric() || characters[index] == '_')
      {
        index += 1;
      }
      let word: String = characters[start..index].iter().collect();
      let is_call = characters.get(index) == Some(&'(');
      let is_key =
        characters.get(index) == Some(&':') && characters.get(index + 1) != Some(&':');
      let style = if is_key {
        None
      } else if KEYWORDS.contains(&word.as_str()) {
        Some(KEYWORD)
      } else if is_call {
        Some(CALL)
      } else {
        None
      };
      match style {
        Some(style) => {
          flush(&mut spans, &mut plain);
          spans.push(Span::styled(word, style));
        }
        None => plain.push_str(&word),
      }
      continue;
    }

    plain.push(character);
    index += 1;
  }

  flush(&mut spans, &mut plain);
  Line::new(spans)
}

fn find_closing(characters: &[char], start: usize) -> usize {
  let quote = characters[start];
  let mut index = start + 1;
  while index < characters.len() {
    match characters[index] {
      '\\' => index += 2,
      character if character == quote => return index + 1,
      _ => index += 1,
    }
  }
  characters.len()
}

fn flush(spans: &mut Vec<Span<'static>>, plain: &mut String) {
  if !plain.is_empty() {
    spans.push(Span::raw(std::mem::take(plain)));
  }
}

#[cfg(test)]
mod tests {
  use anstyle::Style;

  use super::{CALL, KEYWORD, highlight};
  use crate::ui::style::DONE;

  fn styles(line: &str) -> Vec<(String, Style)> {
    highlight(line)
      .spans
      .into_iter()
      .map(|span| (span.content.into_owned(), span.style))
      .collect()
  }

  #[test]
  fn colors_keywords_strings_and_calls() {
    assert_eq!(
      styles("import { Inboxjot } from 'inboxjot';"),
      [
        ("import".to_owned(), KEYWORD),
        (" { Inboxjot } ".to_owned(), Style::new()),
        ("from".to_owned(), KEYWORD),
        (" ".to_owned(), Style::new()),
        ("'inboxjot'".to_owned(), DONE),
        (";".to_owned(), Style::new()),
      ]
    );
    assert!(
      styles("const inboxjot = new Inboxjot();").contains(&("Inboxjot".to_owned(), CALL))
    );
    assert!(styles("await inboxjot.emails.send({").contains(&("send".to_owned(), CALL)));
    assert_eq!(styles("  from: 'hi'")[0], ("  from: ".to_owned(), Style::new()));
  }

  #[test]
  fn keeps_an_escaped_quote_inside_the_string() {
    assert!(styles(r#"  "it\"s"  "#).contains(&(r#""it\"s""#.to_owned(), DONE)));
  }
}
