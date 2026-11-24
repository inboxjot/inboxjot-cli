use std::io::IsTerminal;
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};

use inboxjot::ErrorName;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, RETRY_AFTER, USER_AGENT};
use reqwest::{Method, StatusCode, Url};
use serde::Serialize;
use serde_json::Value;

use crate::error::{Error, ErrorItem, Kind, Result};
use crate::ui::mark::Mark;
use crate::ui::spinner;
use crate::ui::style::{DIM, has_color};
use crate::ui::text::{Line, Span};

pub const DEFAULT_API_URL: &str = "https://api.inboxjot.com";
pub const CLI_USER_AGENT: &str = concat!("inboxjot-cli/", env!("CARGO_PKG_VERSION"));

pub const TIMEOUT: Duration = Duration::from_secs(60);
const UPLOAD_CHUNK: usize = 64 * 1024;
const MAX_RETRIES: u32 = 2;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
const MAX_PER_PAGE: usize = 100;

pub fn get_fallback_roots() -> Vec<reqwest::Certificate> {
  webpki_root_certs::TLS_SERVER_ROOT_CERTS
    .iter()
    .filter_map(|cert| reqwest::Certificate::from_der(cert).ok())
    .collect()
}

#[derive(Clone, Debug)]
pub struct Api {
  http: reqwest::Client,
  base_url: Url,
  api_key: String,
  debug: bool,
  notify: bool,
  allows_loading: bool,
  loading: Option<String>,
}

#[derive(Debug)]
pub struct RawResponse {
  pub status: StatusCode,
  pub headers: HeaderMap,
  pub body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
  pub limit: usize,
  pub all: bool,
  pub after: Option<String>,
  pub before: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct ApiErrorName {
  name: ErrorName,
}

#[derive(Debug, serde::Deserialize)]
struct ApiErrorBody {
  name: String,
  message: String,
  #[serde(default)]
  errors: Vec<ErrorItem>,
  #[serde(default)]
  request_id: Option<String>,
}

impl Api {
  pub fn new(base_url: &str, api_key: String, debug: bool) -> Result<Self> {
    let base_url = Url::parse(base_url).map_err(|error| {
      Error::usage(
        "invalid_api_url",
        format!("The API URL {base_url} is not valid: {error}"),
      )
    })?;
    let http = reqwest::Client::builder()
      .timeout(TIMEOUT)
      .tls_certs_merge(get_fallback_roots())
      .build()?;

    let notify = std::io::stderr().is_terminal();
    Ok(Self {
      http,
      base_url,
      api_key,
      debug,
      notify,
      allows_loading: false,
      loading: None,
    })
  }

  pub fn allow_loading(mut self, allowed: bool) -> Self {
    self.allows_loading = allowed;
    self
  }

  pub fn loading(mut self, message: impl Into<String>) -> Self {
    if self.allows_loading {
      self.loading = Some(message.into());
    }
    self
  }

  pub fn base_url(&self) -> &Url {
    &self.base_url
  }

  pub fn token(&self) -> &str {
    &self.api_key
  }

  pub async fn get(
    &self,
    segments: &[&str],
    query: &(impl Serialize + ?Sized),
  ) -> Result<Value> {
    let query = to_query_pairs(query)?;
    self.send(Method::GET, segments, &query, None).await
  }

  pub async fn post(
    &self,
    segments: &[&str],
    body: &(impl Serialize + ?Sized),
  ) -> Result<Value> {
    let body = serde_json::to_vec(body)?;
    self.send(Method::POST, segments, &[], Some(body)).await
  }

  pub async fn patch(
    &self,
    segments: &[&str],
    body: &(impl Serialize + ?Sized),
  ) -> Result<Value> {
    let body = serde_json::to_vec(body)?;
    self.send(Method::PATCH, segments, &[], Some(body)).await
  }

  pub async fn delete(&self, segments: &[&str]) -> Result<Value> {
    self.send(Method::DELETE, segments, &[], None).await
  }

  pub async fn list(
    &self,
    segments: &[&str],
    query: &(impl Serialize + ?Sized),
    page: &Page,
  ) -> Result<Value> {
    let base_query = to_query_pairs(query)?;
    let mut items = Vec::new();
    let mut after = page.after.clone();

    loop {
      let remaining = if page.all { MAX_PER_PAGE } else { page.limit - items.len() };
      let mut query = base_query.clone();
      query.push(("per_page".to_owned(), remaining.min(MAX_PER_PAGE).to_string()));

      if let Some(after) = &after {
        query.push(("after".to_owned(), after.clone()));
      } else if let Some(before) = &page.before {
        query.push(("before".to_owned(), before.clone()));
      }

      let mut body = self.send(Method::GET, segments, &query, None).await?;
      if let Some(Value::Array(data)) = body.get_mut("data").map(Value::take) {
        items.extend(data);
      }

      let next_cursor =
        body.get("next_cursor").and_then(Value::as_str).map(str::to_owned);
      let is_done = !page.all && items.len() >= page.limit;
      if is_done || next_cursor.is_none() || page.before.is_some() {
        body["data"] = Value::Array(items);
        return Ok(body);
      }

      after = next_cursor;
    }
  }

  pub async fn send(
    &self,
    method: Method,
    segments: &[&str],
    query: &[(String, String)],
    body: Option<Vec<u8>>,
  ) -> Result<Value> {
    let url = self.build_url(segments)?;
    let response = self.send_raw(method, url, query, &HeaderMap::new(), body).await?;

    if response.status.is_success() {
      if response.body.is_empty() {
        return Ok(Value::Null);
      }
      return Ok(serde_json::from_slice(&response.body)?);
    }

    Err(read_api_error(response.status, &response.body))
  }

  pub async fn send_raw(
    &self,
    method: Method,
    url: Url,
    query: &[(String, String)],
    headers: &HeaderMap,
    body: Option<Vec<u8>>,
  ) -> Result<RawResponse> {
    let is_read = method == Method::GET;
    let has_key = headers.contains_key("idempotency-key");
    let idempotency_key =
      (!is_read && !has_key).then(|| uuid::Uuid::new_v4().to_string());
    let carries_key = has_key || idempotency_key.is_some();
    let mut attempt = 0;

    loop {
      let mut request = self
        .http
        .request(method.clone(), url.clone())
        .query(query)
        .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
        .header(USER_AGENT, CLI_USER_AGENT)
        .headers(headers.clone());

      if let Some(key) = &idempotency_key {
        request = request.header("idempotency-key", key);
      }

      if let Some(body) = &body {
        if !headers.contains_key(CONTENT_TYPE) {
          request = request.header(CONTENT_TYPE, "application/json");
        }
        request = request.body(body.clone());
      }

      let started = Instant::now();
      let can_retry = attempt < MAX_RETRIES;
      let received = spinner::show_loading(self.loading.as_deref(), async {
        let response = request.send().await?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.bytes().await?.to_vec();
        Ok::<_, reqwest::Error>((status, headers, body))
      })
      .await;
      let (status, headers, body) = match received {
        Ok(received) => received,
        Err(error)
          if can_retry && (error.is_connect() || is_read && error.is_timeout()) =>
        {
          tokio::time::sleep(get_backoff(attempt)).await;
          attempt += 1;
          continue;
        }
        Err(error) => return Err(error.into()),
      };
      self.log_response(&method, &url, status, &headers, started);

      let is_retryable = status == StatusCode::TOO_MANY_REQUESTS
        || is_read && status.is_server_error()
        || carries_key && is_concurrent_request(status, &body);
      let delay = read_retry_after(&headers).unwrap_or_else(|| get_backoff(attempt));

      if !can_retry || !is_retryable || delay > MAX_RETRY_AFTER {
        return Ok(RawResponse { status, headers, body });
      }

      let notice = (status == StatusCode::TOO_MANY_REQUESTS && self.notify)
        .then(|| show_rate_limit(delay, attempt));
      tokio::time::sleep(delay).await;
      if let Some(notice) = notice {
        notice.finish_and_clear();
      }
      attempt += 1;
    }
  }

  pub async fn upload_with_progress(
    &self,
    url: &str,
    body: Vec<u8>,
    content_type: &str,
    on_progress: impl Fn(u64) + Send + Sync + 'static,
  ) -> Result<()> {
    let total = body.len() as u64;
    let chunks: Vec<Vec<u8>> = body.chunks(UPLOAD_CHUNK).map(<[u8]>::to_vec).collect();
    let mut sent = 0;
    let chunks = chunks.into_iter().map(move |chunk| {
      sent += chunk.len() as u64;
      on_progress(sent);
      Ok::<_, std::io::Error>(chunk)
    });
    let response = self
      .http
      .put(url)
      .header(CONTENT_TYPE, content_type)
      .header(reqwest::header::CONTENT_LENGTH, total)
      .body(reqwest::Body::wrap_stream(futures_util::stream::iter(chunks)))
      .send()
      .await?;

    let status = response.status();
    if status.is_success() {
      return Ok(());
    }

    Err(Error::failed(
      "upload_failed",
      format!("The file upload failed with status {}.", status.as_u16()),
    ))
  }

  pub async fn upload(&self, url: &str, body: Vec<u8>, content_type: &str) -> Result<()> {
    let response = self
      .http
      .put(url)
      .header(CONTENT_TYPE, content_type)
      .body(body)
      .send()
      .await?;

    let status = response.status();
    if status.is_success() {
      return Ok(());
    }

    Err(Error::failed(
      "upload_failed",
      format!("The file upload failed with status {}.", status.as_u16()),
    ))
  }

  pub fn build_url(&self, segments: &[&str]) -> Result<Url> {
    let mut url = self.base_url.clone();
    url
      .path_segments_mut()
      .map_err(|()| Error::usage("invalid_api_url", "The API URL cannot have a path."))?
      .pop_if_empty()
      .extend(segments);

    Ok(url)
  }

  fn log_response(
    &self,
    method: &Method,
    url: &Url,
    status: StatusCode,
    headers: &HeaderMap,
    started: Instant,
  ) {
    if !self.debug {
      return;
    }

    let elapsed = started.elapsed().as_millis();
    let request_id = headers
      .get("x-request-id")
      .and_then(|value| value.to_str().ok())
      .unwrap_or("-");
    anstream::eprintln!("{method} {url} {} {elapsed}ms {request_id}", status.as_u16());
  }
}

pub fn read_api_error(status: StatusCode, body: &[u8]) -> Error {
  let kind = match status {
    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Kind::Auth,
    _ => Kind::Failed,
  };

  let Ok(parsed) = serde_json::from_slice::<ApiErrorBody>(body) else {
    let text = String::from_utf8_lossy(body).trim().to_owned();
    let message = if text.is_empty() {
      format!("The Inboxjot API answered with status {}.", status.as_u16())
    } else {
      text
    };
    return Error::new(kind, "unknown_error", message);
  };

  let mut error = Error::new(kind, parsed.name, parsed.message);
  error.details = parsed.errors;
  error.request_id = parsed.request_id;
  if status == StatusCode::UNAUTHORIZED {
    error = error.explain_lost_login();
  }
  if error.name == "domain_not_verified" {
    error = explain_unverified_domain(error);
  }

  error
}

fn explain_unverified_domain(error: Error) -> Error {
  let message = error.message.clone();
  let message = match message.split_once(": ") {
    Some((prefix, rest)) if prefix.starts_with("Email ") => rest,
    _ => message.as_str(),
  };
  let domain = message.split_whitespace().next().unwrap_or_default().to_owned();
  error
    .with_reason("You can only send from a verified domain.")
    .with_next(
      format!("inboxjot domains verify {domain} --wait"),
      "Check the DNS records again",
    )
    .with_next("inboxjot domains list", "See your domains")
}

pub fn to_query_pairs(
  query: &(impl Serialize + ?Sized),
) -> Result<Vec<(String, String)>> {
  let Value::Object(map) = serde_json::to_value(query)? else {
    return Ok(Vec::new());
  };

  let mut pairs = Vec::new();
  for (key, value) in map {
    match value {
      Value::Null => {}
      Value::Array(items) => {
        pairs.extend(items.iter().map(|item| (key.clone(), get_query_value(item))));
      }
      value => pairs.push((key, get_query_value(&value))),
    }
  }

  Ok(pairs)
}

fn get_query_value(value: &Value) -> String {
  match value {
    Value::String(text) => text.clone(),
    value => value.to_string(),
  }
}

fn is_concurrent_request(status: StatusCode, body: &[u8]) -> bool {
  status == StatusCode::CONFLICT
    && serde_json::from_slice::<ApiErrorName>(body)
      .is_ok_and(|error| error.name == ErrorName::ConcurrentIdempotentRequests)
}

fn show_rate_limit(delay: Duration, attempt: u32) -> ProgressBar {
  let color = has_color();
  let line = Line::new(vec![
    Span::raw("  "),
    Mark::Warning.span(),
    Span::raw(format!(
      " Rate limited. Retrying in {}s",
      delay.as_secs_f64().ceil().max(1.0)
    )),
    Span::styled(format!("  attempt {} of {}", attempt + 2, MAX_RETRIES + 1), DIM),
  ]);
  let bar = ProgressBar::new_spinner().with_style(
    ProgressStyle::with_template("{msg}")
      .unwrap_or_else(|_| ProgressStyle::default_spinner()),
  );
  bar.set_message(line.render(color));
  bar.tick();
  bar
}

fn get_backoff(attempt: u32) -> Duration {
  Duration::from_millis(500 * 2_u64.pow(attempt.min(4)))
}

fn read_retry_after(headers: &HeaderMap) -> Option<Duration> {
  let value = headers.get(RETRY_AFTER)?.to_str().ok()?;
  let seconds = value.trim().parse().ok()?;

  Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{Api, to_query_pairs};

  #[test]
  fn encodes_each_path_segment() {
    let api = Api::new("http://x", String::new(), false).unwrap();
    let url = api.build_url(&["v1", "contacts", "ada+news@acme.dev"]).unwrap();
    assert_eq!(url.as_str(), "http://x/v1/contacts/ada+news@acme.dev");

    let url = api.build_url(&["v1", "a/b c"]).unwrap();
    assert_eq!(url.as_str(), "http://x/v1/a%2Fb%20c");
  }

  #[test]
  fn explains_an_unverified_domain() {
    let body = br#"{"name":"domain_not_verified","status":403,"message":"Email 2: acme.co.uk is not verified","errors":[]}"#;
    let error = super::read_api_error(reqwest::StatusCode::FORBIDDEN, body);
    assert_eq!(
      error.reason.as_deref(),
      Some("You can only send from a verified domain.")
    );
    assert_eq!(error.next[0].command, "inboxjot domains verify acme.co.uk --wait");
    assert_eq!(error.next[1].command, "inboxjot domains list");
  }

  #[test]
  fn flattens_query_values() {
    let pairs =
      to_query_pairs(&json!({ "status": ["sent", "bounced"], "q": "ada", "skip": null }))
        .unwrap();

    assert_eq!(
      pairs,
      [
        ("status".to_owned(), "sent".to_owned()),
        ("status".to_owned(), "bounced".to_owned()),
        ("q".to_owned(), "ada".to_owned()),
      ]
    );
  }
}
