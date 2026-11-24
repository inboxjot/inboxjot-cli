mod support;

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{API_KEY, Harness, parse_json};

fn email_event(id: &str, kind: &str) -> String {
  json!({
    "id": id,
    "created_at": "2026-11-09T10:00:00Z",
    "type": kind,
    "data": { "email_id": "eml_1", "to": ["ada@acme.dev"], "from": "hi@acme.com", "subject": "Welcome", "tags": [] }
  })
  .to_string()
}

fn contact_event() -> String {
  json!({
    "id": "evt_c",
    "created_at": "2026-11-09T10:00:00Z",
    "type": "contact.created",
    "data": { "contact_id": "con_1", "email": "ada@acme.dev" }
  })
  .to_string()
}

#[allow(clippy::result_large_err)]
async fn serve_events(sessions: Vec<Vec<String>>) -> (String, JoinHandle<Vec<String>>) {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let url = format!("http://{}", listener.local_addr().unwrap());
  let task = tokio::spawn(async move {
    let mut tokens = Vec::new();
    let count = sessions.len();
    for (index, frames) in sessions.into_iter().enumerate() {
      let (tcp, _) = listener.accept().await.unwrap();
      let mut token = None;
      let callback = |request: &Request, response: Response| {
        assert_eq!(request.uri().path(), "/v1/events/stream");
        token = request
          .headers()
          .get("authorization")
          .map(|value| value.to_str().unwrap().to_owned());
        Ok(response)
      };
      let mut socket = accept_hdr_async(tcp, callback).await.unwrap();
      tokens.extend(token);
      for frame in frames {
        socket.send(Message::text(frame)).await.unwrap();
      }
      if index + 1 == count {
        while socket.next().await.is_some() {}
      }
    }
    tokens
  });

  (url, task)
}

async fn finish(task: JoinHandle<Vec<String>>) -> Vec<String> {
  tokio::time::timeout(Duration::from_secs(5), task)
    .await
    .unwrap()
    .unwrap()
}

fn read_lines(output: &[u8]) -> Vec<Value> {
  String::from_utf8_lossy(output)
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_each_event_on_stderr() {
  let harness = Harness::start().await;
  let (url, task) =
    serve_events(vec![vec![email_event("evt_1", "email.delivered")]]).await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .args(["listen", "--timeout", "2s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert!(output.stdout.is_empty());
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("Listening for events"));
  assert!(stderr.contains("email.delivered         eml_1"));
  assert_eq!(finish(task).await, [format!("Bearer {API_KEY}")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_only_the_chosen_events_as_json_lines() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX / 2);
  let frames = vec![
    email_event("evt_1", "email.sent"),
    email_event("evt_2", "email.delivered"),
    contact_event(),
  ];
  let (url, task) = serve_events(vec![frames]).await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .args(["listen", "--events", "email.delivered,contact.created", "--json"])
    .args(["--timeout", "2s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let ids: Vec<Value> = read_lines(&output.stdout)
    .iter()
    .map(|event| event["id"].clone())
    .collect();
  assert_eq!(ids, [json!("evt_2"), json!("evt_c")]);
  finish(task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_json_lines_valid_when_a_subject_has_control_characters() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX / 2);
  let frame = json!({
    "id": "evt_1",
    "created_at": "2026-11-09T10:00:00Z",
    "type": "email.sent",
    "data": { "email_id": "eml_1", "subject": "Hi\u{7f}\u{9b}2J" }
  })
  .to_string();
  let (url, task) = serve_events(vec![vec![frame]]).await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .args(["listen", "--json", "--timeout", "2s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let events = read_lines(&output.stdout);
  assert_eq!(events[0]["data"]["subject"], "Hi\u{7f}\u{9b}2J");
  finish(task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forwards_the_exact_payload_with_a_valid_signature() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX / 2);
  Mock::given(method("POST"))
    .and(path("/webhooks"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&harness.server)
    .await;
  let frame = email_event("evt_1", "email.delivered");
  let (url, task) = serve_events(vec![vec![frame.clone()]]).await;

  let secret = harness
    .inboxjot()
    .args(["listen", "--print-secret"])
    .output()
    .unwrap();
  let secret = String::from_utf8(secret.stdout).unwrap().trim().to_owned();
  let target = format!("{}/webhooks", harness.server.uri().trim_start_matches("http://"));
  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .args(["listen", "--forward-to", &target, "--timeout", "2s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(
    stderr.contains(&format!("Secret whsec_••••••••{}", &secret[secret.len() - 4..]))
  );
  assert!(!stderr.contains(&secret));
  assert!(stderr.contains("Forwarding events to http://"));
  assert!(stderr.contains("→ 200"));
  let requests = harness.requests().await;
  assert_eq!(requests.len(), 1);
  assert_eq!(requests[0].body, frame.as_bytes());
  assert_eq!(requests[0].headers["webhook-id"], "evt_1");
  assert_eq!(requests[0].headers["content-type"], "application/json");
  let event =
    inboxjot::webhooks::verify(&secret, &requests[0].headers, &requests[0].body);
  assert_eq!(event.unwrap().id, "evt_1");
  finish(task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_the_same_signing_secret() {
  let harness = Harness::start().await;

  let first = harness
    .inboxjot()
    .args(["listen", "--print-secret"])
    .output()
    .unwrap();
  let second = harness
    .inboxjot()
    .args(["listen", "--print-secret"])
    .output()
    .unwrap();

  assert!(first.status.success());
  assert_eq!(first.stdout, second.stdout);
  let secret = String::from_utf8(first.stdout).unwrap();
  let key = secret.trim().strip_prefix("whsec_").unwrap();
  assert_eq!(STANDARD.decode(key).unwrap().len(), 24);
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let file = harness.config_dir.path().join("listen-secret");
    let mode = std::fs::metadata(file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_a_secret_file_it_cannot_read() {
  let harness = Harness::start().await;
  let file = harness.config_dir.path().join("listen-secret");
  std::fs::create_dir(&file).unwrap();

  let output = harness
    .inboxjot()
    .args(["listen", "--print-secret", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "io_error");
  assert!(file.is_dir());
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnects_after_the_server_drops_the_connection() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX / 2);
  let sessions = vec![
    vec![email_event("evt_1", "email.sent")],
    vec![email_event("evt_2", "email.delivered")],
  ];
  let (url, task) = serve_events(sessions).await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .args(["listen", "--json", "--timeout", "4s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let ids: Vec<Value> = read_lines(&output.stdout)
    .iter()
    .map(|event| event["id"].clone())
    .collect();
  assert_eq!(ids, [json!("evt_1"), json!("evt_2")]);
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert_eq!(stderr.matches("Lost connection. Retrying in").count(), 1);
  assert_eq!(finish(task).await.len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn tails_email_events_and_skips_contact_events() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX / 2);
  let frames = vec![contact_event(), email_event("evt_1", "email.delivered")];
  let (url, task) = serve_events(vec![frames]).await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .args(["emails", "tail", "--timeout", "2s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "2026-11-09T10:00:00Z\temail.delivered\teml_1\tada@acme.dev\tWelcome\n"
  );
  finish(task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_an_unknown_event_type() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["listen", "--events", "email.sennt"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnects_when_the_server_goes_quiet() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX / 2);
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let url = format!("http://{}", listener.local_addr().unwrap());
  let server = tokio::spawn(async move {
    let (tcp, _) = listener.accept().await.unwrap();
    let _quiet = tokio_tungstenite::accept_async(tcp).await.unwrap();
    let (tcp, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
    socket
      .send(Message::text(email_event("evt_1", "email.delivered")))
      .await
      .unwrap();
    while socket.next().await.is_some() {}
  });

  let output = harness
    .inboxjot()
    .env("INBOXJOT_API_URL", &url)
    .env("__INBOXJOT_STREAM_IDLE_MS", "500")
    .args(["listen", "--timeout", "4s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("Lost connection. Retrying in 1s…"), "{stderr}");
  assert!(stderr.contains("Reconnected"), "{stderr}");
  assert!(stderr.contains("email.delivered         eml_1"), "{stderr}");
  server.abort();
}
