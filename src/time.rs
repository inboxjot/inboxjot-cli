use std::time::Duration;

use jiff::{SignedDuration, Span, SpanRelativeTo, Timestamp, Unit, Zoned};

pub fn parse_duration(text: &str) -> Result<Duration, String> {
  let invalid =
    || format!("\"{text}\" is not a duration. Use a value such as 30s, 5m or 1h");
  let span: Span = text.trim().parse().map_err(|_| invalid())?;
  let duration = span
    .to_duration(SpanRelativeTo::days_are_24_hours())
    .map_err(|_| invalid())?;

  Duration::try_from(duration).map_err(|_| invalid())
}

pub fn parse_schedule(text: &str) -> Result<String, String> {
  let text = text.trim();
  if let Ok(timestamp) = text.parse::<Timestamp>() {
    return Ok(timestamp.to_string());
  }

  let offset = text.strip_prefix("in ").unwrap_or(text);
  let duration = parse_duration(offset).map_err(|_| {
    format!("\"{text}\" is not a time. Use an ISO 8601 time such as 2026-11-20T09:00:00Z, or \"in 10m\"")
  })?;
  let signed = SignedDuration::try_from(duration).map_err(|error| error.to_string())?;
  let timestamp = Timestamp::now()
    .checked_add(signed)
    .map_err(|error| error.to_string())?;

  Ok(
    timestamp
      .round(Unit::Second)
      .map_err(|error| error.to_string())?
      .to_string(),
  )
}

pub fn parse_past(text: &str) -> Result<String, String> {
  let text = text.trim();
  if let Ok(timestamp) = text.parse::<Timestamp>() {
    return Ok(timestamp.to_string());
  }

  let invalid = || {
    format!("\"{text}\" is not a time. Use a duration such as 7d, or an ISO 8601 time")
  };
  let span: Span = text
    .strip_suffix(" ago")
    .unwrap_or(text)
    .parse()
    .map_err(|_| invalid())?;
  let zoned = Zoned::now().checked_sub(span).map_err(|_| invalid())?;
  let timestamp = zoned.timestamp().round(Unit::Second).map_err(|_| invalid())?;

  Ok(timestamp.to_string())
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::{parse_duration, parse_past, parse_schedule};

  #[test]
  fn parses_short_durations() {
    assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
    assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5_400));
    assert_eq!(parse_duration("2d").unwrap(), Duration::from_secs(172_800));
    assert!(parse_duration("soon").is_err());
  }

  #[test]
  fn keeps_iso_times_and_adds_offsets() {
    assert_eq!(parse_schedule("2026-11-20T09:00:00Z").unwrap(), "2026-11-20T09:00:00Z");
    assert!(parse_schedule("in 10m").unwrap().ends_with('Z'));
    assert!(parse_schedule("tomorrow").is_err());
  }

  #[test]
  fn parses_past_times() {
    assert_eq!(parse_past("2026-11-01T00:00:00Z").unwrap(), "2026-11-01T00:00:00Z");
    assert!(parse_past("7d ago").unwrap().ends_with('Z'));
    assert!(parse_past("last week").is_err());
  }
}
