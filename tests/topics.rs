mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn topic() -> Value {
  json!({
    "id": "top_1",
    "name": "Product updates",
    "description": "New features, once a month.",
    "default_subscribed": true,
    "visibility": "public",
    "double_opt_in": false,
    "form_enabled": false,
    "sender_name": "Acme",
    "sender_email": null,
    "redirect_url": null,
    "is_default": false,
    "counts": { "subscribed": 40, "pending": 2, "unsubscribed": 3 },
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_topics(harness: &Harness) {
  Mock::given(method("GET"))
    .and(path("/v1/topics"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [topic()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_topics_as_rows_and_as_json() {
  let harness = Harness::start().await;
  mount_topics(&harness).await;

  let output = harness.inboxjot().args(["topics", "list"]).output().unwrap();
  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Name\tVisibility\tDefault\tSubscribed\tCreated\nProduct updates\tpublic\tyes\t40\t2026-11-01T10:00:00Z\n"
  );

  let output = harness.inboxjot().args(["topics", "ls", "--json"]).output().unwrap();
  assert!(output.status.success());
  assert_eq!(
    parse_json(&output.stdout),
    json!({ "data": [topic()], "next_cursor": null, "prev_cursor": null })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_topic_with_the_given_fields() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/topics"))
    .respond_with(ResponseTemplate::new(201).set_body_json(topic()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["topics", "create", "--name", "Product updates"])
    .args(["--default-subscribed", "true", "--visibility", "private"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "top_1\n");
  assert_eq!(
    harness.requests().await[0].body_json::<Value>().unwrap(),
    json!({ "name": "Product updates", "default_subscribed": true, "visibility": "private" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_topic_for_a_brand_by_name() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/brands"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "brd_1", "name": "Acme Labs" }], "next_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/topics"))
    .respond_with(ResponseTemplate::new(201).set_body_json(topic()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["topics", "create", "--name", "Labs news", "--brand", "Acme Labs"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let requests = harness.requests().await;
  let post = requests
    .iter()
    .find(|request| request.method.as_str() == "POST")
    .unwrap();
  assert_eq!(
    post.body_json::<Value>().unwrap(),
    json!({ "name": "Labs news", "brand_id": "brd_1" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_a_topic_name_and_clears_a_field_on_update() {
  let harness = Harness::start().await;
  mount_topics(&harness).await;
  Mock::given(method("PATCH"))
    .and(path("/v1/topics/top_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(topic()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["topics", "update", "Product updates", "--clear", "sender-name"])
    .args(["--form-enabled", "false"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let requests = harness.requests().await;
  let patch = requests
    .iter()
    .find(|request| request.method.as_str() == "PATCH")
    .unwrap();
  assert_eq!(
    patch.body_json::<Value>().unwrap(),
    json!({ "form_enabled": false, "sender_name": null })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_when_no_topic_has_the_name() {
  let harness = Harness::start().await;
  mount_topics(&harness).await;

  let output = harness
    .inboxjot()
    .args(["topics", "inspect", "product updates", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_topic_only_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/topics/top_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(topic()))
    .mount(&harness.server)
    .await;
  Mock::given(method("DELETE"))
    .and(path("/v1/topics/top_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .mount(&harness.server)
    .await;
  let is_deleted = |requests: Vec<wiremock::Request>| {
    requests.iter().any(|request| request.method.as_str() == "DELETE")
  };

  let refused = harness
    .inboxjot()
    .args(["topics", "delete", "top_1", "--json"])
    .output()
    .unwrap();
  assert_eq!(refused.status.code(), Some(2));
  assert_eq!(parse_json(&refused.stderr)["error"]["name"], "confirmation_required");
  assert!(!is_deleted(harness.requests().await));

  let output = harness
    .inboxjot()
    .args(["topics", "rm", "top_1", "--json", "--yes"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(
    parse_json(&output.stdout),
    json!({ "id": "top_1", "name": "Product updates", "deleted": true })
  );
  assert!(is_deleted(harness.requests().await));
}
