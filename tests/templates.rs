mod support;

use serde_json::json;
use wiremock::matchers::{header_exists, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn template() -> serde_json::Value {
  json!({
    "id": "tpl_1",
    "name": "Welcome",
    "from": "Acme <hello@acme.dev>",
    "reply_to": null,
    "subject": "Welcome to Acme",
    "preheader": "Get started",
    "html": "<html><body>Hi</body></html>",
    "text": "Hi",
    "published_version_id": null,
    "has_unpublished_changes": true,
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_template(harness: &Harness) {
  Mock::given(method("GET"))
    .and(path("/v1/templates/tpl_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(template()))
    .mount(&harness.server)
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_templates_with_a_query() {
  let harness = Harness::start().await;
  let page = json!({
    "data": [{
      "id": "tpl_1", "name": "Welcome", "from": "", "subject": "Welcome to Acme",
      "preheader": "", "stats": { "sent": 12, "opened": 3, "clicked": 1 },
      "used_in": { "count": 0, "first": null },
      "created_at": "2026-11-01T10:00:00Z", "updated_at": "2026-11-01T10:00:00Z"
    }],
    "next_cursor": null, "prev_cursor": null
  });
  Mock::given(method("GET"))
    .and(path("/v1/templates"))
    .and(query_param("query", "wel"))
    .respond_with(ResponseTemplate::new(200).set_body_json(&page))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["templates", "ls", "--query", "wel"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ID\tName\tSubject\tSent\tUpdated\ntpl_1\tWelcome\tWelcome to Acme\t12\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_template_from_an_html_file() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/templates"))
    .and(header_exists("idempotency-key"))
    .respond_with(ResponseTemplate::new(201).set_body_json(template()))
    .mount(&harness.server)
    .await;
  let html = harness.config_dir.path().join("welcome.html");
  std::fs::write(&html, "<p>Hi</p>").unwrap();

  let output = harness
    .inboxjot()
    .args(["templates", "create", "--name", "Welcome", "--subject", "Hello"])
    .arg("--html-file")
    .arg(&html)
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "tpl_1\n");
  let requests = harness.requests().await;
  assert_eq!(
    requests[0].body_json::<serde_json::Value>().unwrap(),
    json!({ "name": "Welcome", "subject": "Hello", "html": "<p>Hi</p>" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_only_the_given_fields() {
  let harness = Harness::start().await;
  Mock::given(method("PATCH"))
    .and(path("/v1/templates/tpl_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(template()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["templates", "update", "tpl_1", "--subject", "New"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let requests = harness.requests().await;
  assert_eq!(
    requests[0].body_json::<serde_json::Value>().unwrap(),
    json!({ "subject": "New" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_update_without_changes() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["templates", "update", "tpl_1"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn previews_the_draft_html_and_text() {
  let harness = Harness::start().await;
  mount_template(&harness).await;

  let html = harness
    .inboxjot()
    .args(["templates", "preview", "tpl_1"])
    .output()
    .unwrap();
  let text = harness
    .inboxjot()
    .args(["templates", "preview", "tpl_1", "--text"])
    .output()
    .unwrap();

  assert!(html.status.success());
  assert_eq!(String::from_utf8(html.stdout).unwrap(), "<html><body>Hi</body></html>\n");
  assert_eq!(String::from_utf8(text.stdout).unwrap(), "Hi\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_to_publish_without_yes() {
  let harness = Harness::start().await;
  mount_template(&harness).await;

  let output = harness
    .inboxjot()
    .args(["templates", "publish", "tpl_1", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(error["error"]["next"], "inboxjot templates publish tpl_1 --json --yes");
  assert!(
    harness
      .requests()
      .await
      .iter()
      .all(|request| request.method.as_str() == "GET")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn publishes_with_yes() {
  let harness = Harness::start().await;
  mount_template(&harness).await;
  Mock::given(method("POST"))
    .and(path("/v1/templates/tpl_1/publish"))
    .respond_with(ResponseTemplate::new(200).set_body_json(template()))
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["templates", "publish", "tpl_1", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "tpl_1\n");
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Published Welcome")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_and_restores_versions() {
  let harness = Harness::start().await;
  let page = json!({
    "data": [{ "id": "ver_1", "reason": "manual", "name": "Launch", "editors": [], "created_at": "2026-11-01T10:00:00Z" }],
    "total": 1, "next_cursor": null, "prev_cursor": null
  });
  Mock::given(method("GET"))
    .and(path("/v1/templates/tpl_1/versions"))
    .respond_with(ResponseTemplate::new(200).set_body_json(&page))
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/templates/tpl_1/versions/ver_1/restore"))
    .respond_with(ResponseTemplate::new(200).set_body_json(template()))
    .expect(1)
    .mount(&harness.server)
    .await;

  let listed = harness
    .inboxjot()
    .args(["templates", "versions", "list", "tpl_1", "--json"])
    .output()
    .unwrap();
  let restored = harness
    .inboxjot()
    .args(["templates", "versions", "restore", "tpl_1", "ver_1"])
    .output()
    .unwrap();

  assert_eq!(parse_json(&listed.stdout), page);
  assert!(restored.status.success());
  assert_eq!(String::from_utf8(restored.stdout).unwrap(), "tpl_1\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_template_with_yes() {
  let harness = Harness::start().await;
  mount_template(&harness).await;
  Mock::given(method("DELETE"))
    .and(path("/v1/templates/tpl_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .expect(1)
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["templates", "rm", "tpl_1", "--yes"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "tpl_1\n");
}
