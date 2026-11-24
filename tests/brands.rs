mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn brand() -> Value {
  json!({
    "id": "brd_1",
    "name": "Acme Labs",
    "slug": "acme-labs",
    "logo_url": null,
    "is_default": false,
    "default_topic_id": null,
    "topic_count": 2,
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_brands(harness: &Harness) {
  Mock::given(method("GET"))
    .and(path("/v1/brands"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [brand()], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_brands_as_rows_and_as_json() {
  let harness = Harness::start().await;
  mount_brands(&harness).await;

  let output = harness.inboxjot().args(["brands", "list"]).output().unwrap();
  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Name\tDefault\tTopics\tCreated\nAcme Labs\tno\t2\t2026-11-01T10:00:00Z\n"
  );

  let output = harness.inboxjot().args(["brands", "ls", "--json"]).output().unwrap();
  assert!(output.status.success());
  assert_eq!(
    parse_json(&output.stdout),
    json!({ "data": [brand()], "next_cursor": null, "prev_cursor": null })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_brand() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/brands"))
    .respond_with(ResponseTemplate::new(201).set_body_json(brand()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["brands", "create", "--name", "Acme Labs"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "brd_1\n");
  assert_eq!(
    harness.requests().await[0].body_json::<Value>().unwrap(),
    json!({ "name": "Acme Labs" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_a_brand_name_on_update() {
  let harness = Harness::start().await;
  mount_brands(&harness).await;
  Mock::given(method("PATCH"))
    .and(path("/v1/brands/brd_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(brand()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["brands", "update", "Acme Labs", "--name", "Acme Studio"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let requests = harness.requests().await;
  let patch = requests
    .iter()
    .find(|request| request.method.as_str() == "PATCH")
    .unwrap();
  assert_eq!(patch.body_json::<Value>().unwrap(), json!({ "name": "Acme Studio" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_when_no_brand_has_the_name() {
  let harness = Harness::start().await;
  mount_brands(&harness).await;

  let output = harness
    .inboxjot()
    .args(["brands", "inspect", "acme labs", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "not_found");
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_brand_only_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/brands/brd_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(brand()))
    .mount(&harness.server)
    .await;
  Mock::given(method("DELETE"))
    .and(path("/v1/brands/brd_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .mount(&harness.server)
    .await;
  let is_deleted = |requests: Vec<wiremock::Request>| {
    requests.iter().any(|request| request.method.as_str() == "DELETE")
  };

  let refused = harness
    .inboxjot()
    .args(["brands", "delete", "brd_1", "--json"])
    .output()
    .unwrap();
  assert_eq!(refused.status.code(), Some(2));
  assert_eq!(parse_json(&refused.stderr)["error"]["name"], "confirmation_required");
  assert!(!is_deleted(harness.requests().await));

  let output = harness
    .inboxjot()
    .args(["brands", "rm", "brd_1", "--json", "--yes"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(
    parse_json(&output.stdout),
    json!({ "id": "brd_1", "name": "Acme Labs", "deleted": true })
  );
  assert!(is_deleted(harness.requests().await));
}
