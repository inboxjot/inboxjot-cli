mod support;

use serde_json::{Value, json};
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, Request, ResponseTemplate};

use support::{Harness, parse_json};

fn email(status: &str) -> Value {
  json!({
    "id": "eml_1",
    "from": "Acme <hi@acme.com>",
    "to": ["ada@acme.dev"],
    "cc": [],
    "bcc": [],
    "subject": "Welcome",
    "tags": [],
    "status": status,
    "source": "api",
    "scheduled_at": null,
    "sent_at": "2026-11-01T10:00:01Z",
    "created_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_send(harness: &Harness) {
  Mock::given(method("POST"))
    .and(path("/v1/emails"))
    .and(header_exists("idempotency-key"))
    .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "id": "eml_1" })))
    .mount(&harness.server)
    .await;
}

fn get_body(request: &Request) -> Value {
  request.body_json().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_every_field_and_prints_the_id_when_piped() {
  let harness = Harness::start().await;
  mount_send(&harness).await;
  let file = harness.config_dir.path().join("invoice.txt");
  std::fs::write(&file, "hello").unwrap();

  let output = harness
    .inboxjot()
    .args(["send", "--from", "Acme <hi@acme.com>", "--to", "ada@acme.dev,bo@acme.dev"])
    .args(["--cc", "cy@acme.dev", "--subject", "Welcome", "--html", "<p>Hi</p>"])
    .args(["--text", "Hi", "--tag", "plan=pro", "--header", "X-Entity-Ref: 42"])
    .args(["--scheduled-at", "2026-11-20T09:00:00Z", "--attach"])
    .arg(&file)
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "eml_1\n");
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Scheduled eml_1 for")
  );
  let requests = harness.requests().await;
  assert_eq!(
    get_body(&requests[0]),
    json!({
      "from": "Acme <hi@acme.com>",
      "to": ["ada@acme.dev", "bo@acme.dev"],
      "cc": ["cy@acme.dev"],
      "subject": "Welcome",
      "html": "<p>Hi</p>",
      "text": "Hi",
      "headers": { "X-Entity-Ref": "42" },
      "tags": [{ "name": "plan", "value": "pro" }],
      "scheduled_at": "2026-11-20T09:00:00Z",
      "attachments": [{ "filename": "invoice.txt", "content": "aGVsbG8=" }]
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_a_template_with_its_variables() {
  let harness = Harness::start().await;
  mount_send(&harness).await;

  let output = harness
    .inboxjot()
    .args(["emails", "send", "--to", "ada@acme.dev", "--template", "tpl_1"])
    .args(["--data", r#"{"name":"Ada"}"#])
    .output()
    .unwrap();

  assert!(output.status.success());
  let requests = harness.requests().await;
  assert_eq!(
    get_body(&requests[0]),
    json!({ "to": ["ada@acme.dev"], "template": { "id": "tpl_1", "variables": { "name": "Ada" } } })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_open_tracking_only_when_the_flag_is_given() {
  let harness = Harness::start().await;
  mount_send(&harness).await;

  for extra in [&["--open-tracking", "false"][..], &[]] {
    let output = harness
      .inboxjot()
      .args(["send", "--to", "ada@acme.dev", "--text", "Hi"])
      .args(extra)
      .output()
      .unwrap();
    assert!(output.status.success());
  }

  let requests = harness.requests().await;
  assert_eq!(
    get_body(&requests[0]),
    json!({ "to": ["ada@acme.dev"], "text": "Hi", "open_tracking": false })
  );
  assert_eq!(get_body(&requests[1]), json!({ "to": ["ada@acme.dev"], "text": "Hi" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_body_and_sends_nothing_on_a_dry_run() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .env_remove("INBOXJOT_API_KEY")
    .args(["send", "--to", "ada@acme.dev", "--text", "Hi", "--dry-run"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), json!({ "to": ["ada@acme.dev"], "text": "Hi" }));
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_send_without_content() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["send", "--to", "ada@acme.dev", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "missing_content");
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_api_body_with_json() {
  let harness = Harness::start().await;
  mount_send(&harness).await;

  let output = harness
    .inboxjot()
    .args(["send", "--to", "ada@acme.dev", "--text", "Hi", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), json!({ "id": "eml_1" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn waits_until_the_email_is_delivered() {
  let harness = Harness::start().await;
  mount_send(&harness).await;
  Mock::given(method("GET"))
    .and(path("/v1/emails/eml_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(email("delivered")))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["send", "--to", "ada@acme.dev", "--text", "Hi", "--wait", "--timeout", "5s"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "eml_1\n");
  assert!(String::from_utf8(output.stderr).unwrap().contains("Delivered eml_1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_inspect_wait_when_the_email_bounced() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/emails/eml_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(email("bounced")))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["emails", "inspect", "eml_1", "--wait", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stdout), email("bounced"));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "delivery_failed");
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_a_batch_in_chunks_and_reports_a_partial_failure() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/emails/batch"))
    .respond_with(|request: &Request| {
      let emails = request.body_json::<Vec<Value>>().unwrap();
      if emails.len() < 100 {
        return ResponseTemplate::new(422).set_body_json(json!({
          "name": "validation_error", "status": 422, "message": "The from address is not verified.", "errors": []
        }));
      }
      let ids: Vec<Value> =
        (0..emails.len()).map(|index| json!({ "id": format!("eml_{index}") })).collect();
      ResponseTemplate::new(202).set_body_json(json!({ "data": ids }))
    })
    .mount(&harness.server)
    .await;
  let lines: Vec<String> = (0..150)
    .map(|index| {
      json!({ "to": [format!("u{index}@acme.dev")], "text": "Hi" }).to_string()
    })
    .collect();
  let file = harness.config_dir.path().join("emails.jsonl");
  std::fs::write(&file, lines.join("\n")).unwrap();

  let output = harness
    .inboxjot()
    .args(["emails", "batch"])
    .arg(&file)
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let stdout = String::from_utf8(output.stdout).unwrap();
  assert_eq!(stdout.lines().count(), 100);
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Sent 100 of 150 emails. 50 failed.")
  );
  let sizes: Vec<usize> = harness
    .requests()
    .await
    .iter()
    .map(|request| request.body_json::<Vec<Value>>().unwrap().len())
    .collect();
  assert_eq!(sizes, [100, 50]);
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_emails_with_filters() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/emails"))
    .and(query_param("status", "delivered"))
    .and(query_param("query", "ada"))
    .and(query_param("source", "api"))
    .and(query_param("sent_after", "2026-11-01T00:00:00Z"))
    .and(query_param("per_page", "10"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [email("delivered")], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["emails", "ls", "--status", "delivered", "--query", "ada", "--source", "api"])
    .args(["--since", "2026-11-01T00:00:00Z", "-n", "10"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ID\tTo\tSubject\tStatus\tCreated\neml_1\tada@acme.dev\tWelcome\tdelivered\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn reschedules_an_email() {
  let harness = Harness::start().await;
  Mock::given(method("PATCH"))
    .and(path("/v1/emails/eml_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "id": "eml_1", "scheduled_at": "2026-11-20T09:00:00Z"
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["emails", "reschedule", "eml_1", "--at", "2026-11-20T09:00:00Z"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "eml_1\n");
  assert_eq!(
    get_body(&harness.requests().await[0]),
    json!({ "scheduled_at": "2026-11-20T09:00:00Z" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_cancel_without_yes_when_it_cannot_prompt() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["emails", "cancel", "eml_1", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(error["error"]["next"], "inboxjot emails cancel eml_1 --json --yes");
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancels_an_email_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/emails/eml_1/cancel"))
    .respond_with(
      ResponseTemplate::new(200)
        .set_body_json(json!({ "id": "eml_1", "status": "canceled" })),
    )
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["emails", "cancel", "eml_1", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "eml_1\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_each_attachment_into_the_folder() {
  let harness = Harness::start().await;
  let download_url = format!("{}/files/invoice.pdf", harness.server.uri());
  Mock::given(method("GET"))
    .and(path("/v1/emails/eml_1/attachments"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [
        { "id": "att_1", "filename": "invoice.pdf", "content_type": "application/pdf", "size": 3, "content_id": null, "content_disposition": "attachment", "deleted_at": null, "download_url": download_url, "expires_at": "2026-11-01T10:10:00Z" },
        { "id": "att_2", "filename": "old.png", "content_type": "image/png", "size": 9, "content_id": null, "content_disposition": "attachment", "deleted_at": "2026-11-01T00:00:00Z", "download_url": null, "expires_at": null }
      ]
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/files/invoice.pdf"))
    .respond_with(ResponseTemplate::new(200).set_body_bytes(b"pdf".to_vec()))
    .mount(&harness.server)
    .await;
  let folder = harness.config_dir.path().join("files");

  let output = harness
    .inboxjot()
    .args(["emails", "attachments", "download", "eml_1", "--output"])
    .arg(&folder)
    .output()
    .unwrap();

  assert!(output.status.success());
  let saved = folder.join("invoice.pdf");
  assert_eq!(std::fs::read(&saved).unwrap(), b"pdf");
  assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{}\n", saved.display()));
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("old.png was deleted")
  );
}
