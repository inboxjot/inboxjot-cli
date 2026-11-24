use std::future::Future;
use std::io::IsTerminal;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use clap::builder::PossibleValue;
use futures_util::StreamExt;
use http::HeaderValue;
use http::header::{AUTHORIZATION, USER_AGENT};
use inboxjot::WebhookEventType;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as SocketError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use crate::api::{CLI_USER_AGENT, read_api_error};
use crate::context::Context;
use crate::error::{Error, Kind, Result};
use crate::output::get_path;

pub const EVENT_TYPES: [EventType; 13] = [
  EventType(WebhookEventType::EmailSent),
  EventType(WebhookEventType::EmailDelivered),
  EventType(WebhookEventType::EmailOpened),
  EventType(WebhookEventType::EmailClicked),
  EventType(WebhookEventType::EmailBounced),
  EventType(WebhookEventType::EmailComplained),
  EventType(WebhookEventType::EmailDeliveryDelayed),
  EventType(WebhookEventType::EmailRejected),
  EventType(WebhookEventType::EmailFailed),
  EventType(WebhookEventType::EmailUnsubscribed),
  EventType(WebhookEventType::ContactCreated),
  EventType(WebhookEventType::ContactUpdated),
  EventType(WebhookEventType::ContactDeleted),
];

const FIRST_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
const GIVE_UP_AFTER: Duration = Duration::from_secs(5 * 60);
const IDLE_TIMEOUT: Duration = Duration::from_secs(70);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventType(pub WebhookEventType);

impl EventType {
  pub fn is_email(self) -> bool {
    !matches!(
      self.0,
      WebhookEventType::ContactCreated
        | WebhookEventType::ContactUpdated
        | WebhookEventType::ContactDeleted
    )
  }
}

impl ValueEnum for EventType {
  fn value_variants<'a>() -> &'a [Self] {
    &EVENT_TYPES
  }

  fn to_possible_value(&self) -> Option<PossibleValue> {
    let name = serde_json::to_value(self.0).ok()?;
    Some(PossibleValue::new(name.as_str()?.to_owned()))
  }
}

pub struct Event {
  pub body: String,
  pub payload: Value,
}

impl Event {
  pub fn get_type(&self) -> &str {
    self.get_text("type")
  }

  pub fn get_kind(&self) -> EventType {
    let kind = get_path(&self.payload, "type")
      .and_then(|value| WebhookEventType::deserialize(value).ok())
      .unwrap_or(WebhookEventType::Unknown);
    EventType(kind)
  }

  pub fn get_text(&self, path: &str) -> &str {
    get_path(&self.payload, path)
      .and_then(Value::as_str)
      .unwrap_or_default()
  }

  pub fn get_subject_id(&self) -> &str {
    let email_id = self.get_text("data.email_id");
    if email_id.is_empty() { self.get_text("data.contact_id") } else { email_id }
  }

  pub fn format_clock(&self) -> String {
    let timestamp = self
      .get_text("created_at")
      .parse()
      .unwrap_or_else(|_| Timestamp::now());
    timestamp
      .to_zoned(TimeZone::system())
      .strftime("%H:%M:%S")
      .to_string()
  }
}

pub enum Item {
  Event(Event),
  Lost { retry_in: Duration },
  Reconnected,
}

struct Retry {
  started: Instant,
  delay: Duration,
}

pub struct EventStream<'a> {
  context: &'a Context,
  socket: Socket,
  idle_timeout: Duration,
  retry: Option<Retry>,
}

impl<'a> EventStream<'a> {
  pub async fn connect(context: &'a Context) -> Result<Self> {
    let socket = open_socket(context).await?;
    let idle_timeout = std::env::var("__INBOXJOT_STREAM_IDLE_MS")
      .ok()
      .and_then(|value| value.parse().ok())
      .map_or(IDLE_TIMEOUT, Duration::from_millis);
    Ok(Self { context, socket, idle_timeout, retry: None })
  }

  pub async fn next(&mut self) -> Result<Item> {
    loop {
      if let Some(retry) = &mut self.retry {
        tokio::time::sleep(retry.delay).await;
        match open_socket(self.context).await {
          Ok(socket) => {
            self.socket = socket;
            self.retry = None;
            return Ok(Item::Reconnected);
          }
          Err(error) if error.kind != Kind::Failed => return Err(error),
          Err(_) if retry.started.elapsed() >= GIVE_UP_AFTER => {
            return Err(
              Error::failed("network_error", "Lost the connection to the event stream.")
                .with_next("inboxjot doctor", "Check the setup and the connection"),
            );
          }
          Err(_) => {
            retry.delay = (retry.delay * 2).min(MAX_RETRY_DELAY);
            return Ok(Item::Lost { retry_in: retry.delay });
          }
        }
      }

      match tokio::time::timeout(self.idle_timeout, self.socket.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
          let body = text.to_string();
          if let Ok(payload) = serde_json::from_str(&body) {
            return Ok(Item::Event(Event { body, payload }));
          }
        }
        Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => {
          self.retry = Some(Retry { started: Instant::now(), delay: FIRST_RETRY_DELAY });
          return Ok(Item::Lost { retry_in: FIRST_RETRY_DELAY });
        }
        Ok(Some(Ok(_))) => {}
      }
    }
  }
}

pub async fn run_until_stopped(
  work: impl Future<Output = Result<()>>,
  timeout: Option<Duration>,
) -> Result<()> {
  let deadline = async {
    match timeout {
      Some(timeout) => tokio::time::sleep(timeout).await,
      None => std::future::pending().await,
    }
  };

  tokio::select! {
    result = work => result,
    _ = tokio::signal::ctrl_c() => {
      if std::io::stderr().is_terminal() {
        eprintln!();
      }
      Ok(())
    }
    () = deadline => Ok(()),
  }
}

async fn open_socket(context: &Context) -> Result<Socket> {
  let api = context.api().await?;
  let mut url = api.build_url(&["v1", "events", "stream"])?;
  let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
  url.set_scheme(scheme).map_err(|()| {
    Error::usage("invalid_api_url", format!("The API URL {url} is not valid."))
  })?;

  let mut request = url.as_str().into_client_request().map_err(get_socket_error)?;
  let token = HeaderValue::from_str(&format!("Bearer {}", api.token()))
    .map_err(|_| Error::auth("invalid_token", "The saved login is not valid."))?;
  request.headers_mut().insert(AUTHORIZATION, token);
  request
    .headers_mut()
    .insert(USER_AGENT, HeaderValue::from_static(CLI_USER_AGENT));

  let (socket, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect_async(request))
    .await
    .map_err(|_| {
      Error::failed("network_error", "The event stream did not answer in time.")
        .with_next("inboxjot doctor", "Check the setup and the connection")
    })?
    .map_err(get_socket_error)?;
  Ok(socket)
}

fn get_socket_error(error: SocketError) -> Error {
  let SocketError::Http(response) = error else {
    return Error::failed(
      "network_error",
      format!("Could not connect to the event stream: {error}"),
    )
    .with_next("inboxjot doctor", "Check the setup and the connection");
  };

  let status = response.status();
  if status == http::StatusCode::SERVICE_UNAVAILABLE {
    return Error::failed(
      "too_many_streams",
      "Too many event streams are open. Close one and try again.",
    );
  }

  let error = read_api_error(status, response.body().as_deref().unwrap_or_default());
  if error.kind == Kind::Auth {
    return error.with_next("inboxjot login", "Log in again");
  }

  error
}

#[cfg(test)]
mod tests {
  use inboxjot::WebhookEventType;
  use serde_json::json;

  use super::{Event, EventType};

  #[test]
  fn reads_the_email_or_contact_id() {
    let email = Event {
      body: String::new(),
      payload: json!({ "type": "email.sent", "data": { "email_id": "eml_1" } }),
    };
    let contact = Event {
      body: String::new(),
      payload: json!({ "type": "contact.created", "data": { "contact_id": "con_1" } }),
    };

    assert_eq!(email.get_type(), "email.sent");
    assert_eq!(email.get_kind(), EventType(WebhookEventType::EmailSent));
    assert!(email.get_kind().is_email());
    assert!(!contact.get_kind().is_email());
    assert_eq!(email.get_subject_id(), "eml_1");
    assert_eq!(contact.get_subject_id(), "con_1");
  }
}
