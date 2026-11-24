use clap::Args;
use serde_json::{Value, json};

use super::{get_id, read_input};
use crate::context::Context;
use crate::error::{Error, Result};

const BATCH_SIZE: usize = 100;

#[derive(Debug, Args)]
pub struct BatchArgs {
  #[arg(
    value_name = "PATH",
    help = "JSON array or JSONL file of emails, or - for stdin"
  )]
  pub input: String,
}

pub async fn send_batch(args: &BatchArgs, context: &Context) -> Result<()> {
  let emails = parse_batch(&read_input(&args.input)?)?;
  let api = context.api().await?;
  let total = emails.len();
  let spinner = context.output.start_spinner(&format!("Sending {total} emails…"), &[]);
  let mut sent = Vec::new();
  let mut errors = Vec::new();

  for (index, chunk) in emails.chunks(BATCH_SIZE).enumerate() {
    let start = index * BATCH_SIZE;
    spinner.set_message(format!("Sending {} of {total} emails…", start + chunk.len()));
    match api.post(&["v1", "emails", "batch"], chunk).await {
      Ok(mut response) => match response.get_mut("data").map(Value::take) {
        Some(Value::Array(ids)) => sent.extend(ids),
        _ => {
          return Err(Error::failed("invalid_response", "The batch answer has no data."));
        }
      },
      Err(error) => errors.push((start, chunk.len(), error)),
    }
  }
  spinner.finish();

  let failed: usize = errors.iter().map(|(_, count, _)| count).sum();
  if sent.is_empty() && !errors.is_empty() {
    return Err(errors.swap_remove(0).2);
  }

  print_batch_result(&sent, context)?;
  for (start, count, error) in &errors {
    let range = format!("{} to {}", start + 1, start + count);
    context
      .output
      .info(&format!("Emails {range} failed: {}", error.message));
  }

  if failed > 0 {
    let message = format!("Sent {} of {total} emails. {failed} failed.", sent.len());
    return Err(Error::failed("batch_failed", message));
  }

  Ok(())
}

fn parse_batch(text: &str) -> Result<Vec<Value>> {
  let values = serde_json::Deserializer::from_str(text)
    .into_iter::<Value>()
    .collect::<std::result::Result<Vec<_>, _>>()?;
  let emails = match <[Value; 1]>::try_from(values) {
    Ok([Value::Array(items)]) => items,
    Ok([value]) => vec![value],
    Err(values) => values,
  };

  if emails.is_empty() {
    return Err(Error::usage("invalid_input", "The input has no emails."));
  }

  if let Some(index) = emails.iter().position(|email| !email.is_object()) {
    let message = format!("Email {} is not a JSON object.", index + 1);
    return Err(Error::usage("invalid_input", message));
  }

  Ok(emails)
}

fn print_batch_result(sent: &[Value], context: &Context) -> Result<()> {
  if context.output.is_json() {
    return context.output.print_json(&json!({ "data": sent }));
  }

  context.output.success(&format!("Sent {} emails", sent.len()));
  if !context.output.is_stdout_tty() {
    for email in sent {
      context.output.print_line(get_id(email));
    }
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::parse_batch;

  #[test]
  fn reads_json_arrays_and_json_lines() {
    let array = parse_batch(r#"[{"to":["a@x.dev"]},{"to":["b@x.dev"]}]"#).unwrap();
    let lines = parse_batch("{\"to\":[\"a@x.dev\"]}\n{\"to\":[\"b@x.dev\"]}\n").unwrap();

    assert_eq!(array, lines);
    assert_eq!(array[1], json!({ "to": ["b@x.dev"] }));
    assert!(parse_batch("[1]").is_err());
    assert!(parse_batch("").is_err());
  }
}
