use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Compiler, Ctx, Vars, data, unwrap_valr};
use jaq_json::Val;
use serde_json::Value;

use crate::error::{Error, Result};

pub fn run_filter(filter: &str, input: &Value) -> Result<Vec<String>> {
  let input = serde_json::to_vec(input)?;
  let input = jaq_json::read::parse_single(&input)
    .map_err(|error| Error::failed("invalid_json", error.to_string()))?;

  let defs = jaq_core::defs().chain(jaq_std::defs()).chain(jaq_json::defs());
  let funs = jaq_core::funs().chain(jaq_std::funs()).chain(jaq_json::funs());

  let loader = Loader::new(defs);
  let arena = Arena::default();
  let program = File { code: filter, path: () };
  let modules = loader.load(&arena, program).map_err(|_| get_filter_error(filter))?;
  let filter = Compiler::default()
    .with_funs(funs)
    .compile(modules)
    .map_err(|_| get_filter_error(filter))?;

  let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
  let mut lines = Vec::new();
  for output in filter.id.run((ctx, input)).map(unwrap_valr) {
    let output = output.map_err(|error| {
      Error::failed("jq_error", format!("The --jq filter failed: {error}"))
    })?;
    lines.push(get_line(&output));
  }

  Ok(lines)
}

fn get_line(value: &Val) -> String {
  let text = value.to_string();
  match serde_json::from_str::<Value>(&text) {
    Ok(Value::String(raw)) => raw,
    _ => text,
  }
}

fn get_filter_error(filter: &str) -> Error {
  Error::usage("invalid_jq", format!("The --jq filter \"{filter}\" is not valid."))
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::run_filter;

  #[test]
  fn prints_strings_raw_and_values_as_json() {
    let input = json!({ "data": [{ "id": "dom_1", "n": 2 }, { "id": "dom_2", "n": 3 }] });

    assert_eq!(run_filter(".data[].id", &input).unwrap(), ["dom_1", "dom_2"]);
    assert_eq!(run_filter("[.data[].n] | add", &input).unwrap(), ["5"]);
  }

  #[test]
  fn rejects_a_bad_filter() {
    assert!(run_filter(".data[", &json!({})).is_err());
  }
}
