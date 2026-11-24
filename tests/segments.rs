mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn segment() -> Value {
  json!({
    "id": "seg_1",
    "name": "Engaged customers",
    "description": "Opened in the last month",
    "type": "dynamic",
    "conditions": {
      "match": "all",
      "rules": [{ "field": "email.opened", "operator": "within", "value": "30d" }]
    },
    "contact_count": 12,
    "counted_at": null,
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_lookup(harness: &Harness) {
  Mock::given(method("GET"))
    .and(path("/v1/segments"))
    .and(query_param("query", "Engaged customers"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [
        { "id": "seg_2", "name": "Engaged customers (old)" },
        { "id": "seg_1", "name": "Engaged customers" }
      ],
      "next_cursor": null
    })))
    .mount(&harness.server)
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_segments_with_filters() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/segments"))
    .and(query_param("type", "dynamic"))
    .and(query_param("query", "eng"))
    .and(query_param("per_page", "50"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [segment()], "total": 1, "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "list", "--type", "dynamic", "--query", "eng"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Name\tType\tContacts\tCreated\nEngaged customers\tdynamic\t12\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_dynamic_segment_from_a_conditions_file() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/segments"))
    .respond_with(ResponseTemplate::new(201).set_body_json(segment()))
    .mount(&harness.server)
    .await;
  let conditions = segment()["conditions"].clone();
  let file = harness.config_dir.path().join("rules.json");
  std::fs::write(&file, conditions.to_string()).unwrap();

  let output = harness
    .inboxjot()
    .args(["segments", "create", "--name", "Engaged customers", "--conditions"])
    .arg(format!("@{}", file.display()))
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "seg_1\n");
  assert_eq!(
    harness.requests().await[0].body_json::<Value>().unwrap(),
    json!({ "name": "Engaged customers", "type": "dynamic", "conditions": conditions })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_conditions_that_are_not_valid() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["segments", "create", "--name", "Broken", "--conditions", "{", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "invalid_conditions");
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn resolves_an_exact_segment_name_before_inspecting_it() {
  let harness = Harness::start().await;
  mount_lookup(&harness).await;
  Mock::given(method("GET"))
    .and(path("/v1/segments/seg_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(segment()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "inspect", "Engaged customers", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), segment());
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_when_no_segment_has_the_name() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/segments"))
    .and(query_param("query", "Engaged"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "seg_1", "name": "Engaged customers" }], "next_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "inspect", "Engaged", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let error = parse_json(&output.stderr);
  assert_eq!(error["error"]["name"], "not_found");
  assert_eq!(error["error"]["next"], "inboxjot segments list");
}

#[tokio::test(flavor = "multi_thread")]
async fn clears_the_description_on_update() {
  let harness = Harness::start().await;
  Mock::given(method("PATCH"))
    .and(path("/v1/segments/seg_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(segment()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "update", "seg_1", "--name", "VIP", "--clear", "description"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    harness.requests().await[0].body_json::<Value>().unwrap(),
    json!({ "name": "VIP", "description": null })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn adds_members_read_from_stdin() {
  let harness = Harness::start().await;
  mount_lookup(&harness).await;
  Mock::given(method("GET"))
    .and(path("/v1/contacts/jane@example.com"))
    .respond_with(
      ResponseTemplate::new(200)
        .set_body_json(json!({ "id": "con_2", "email": "jane@example.com" })),
    )
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/segments/seg_1/members"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "added": 2 })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "members", "add", "Engaged customers", "-"])
    .write_stdin("con_1\n\njane@example.com\n")
    .output()
    .unwrap();

  assert!(output.status.success());
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Added 2 contacts to Engaged customers")
  );
  let requests = harness.requests().await;
  let post = requests
    .iter()
    .find(|request| request.method.as_str() == "POST")
    .unwrap();
  assert_eq!(
    post.body_json::<Value>().unwrap(),
    json!({ "contact_ids": ["con_1", "con_2"] })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn removes_members_and_prints_the_count_with_json() {
  let harness = Harness::start().await;
  Mock::given(method("DELETE"))
    .and(path("/v1/segments/seg_1/members"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "removed": 1 })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "members", "remove", "seg_1", "con_1", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), json!({ "removed": 1 }));
  assert_eq!(
    harness.requests().await[0].body_json::<Value>().unwrap(),
    json!({ "contact_ids": ["con_1"] })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_members_with_a_query() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/segments/seg_1/members"))
    .and(query_param("query", "jane"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{
        "id": "con_1", "email": "jane@example.com", "first_name": "Jane",
        "last_name": null, "status": "subscribed", "joined_at": "2026-11-02T10:00:00Z"
      }],
      "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["segments", "members", "ls", "seg_1", "--query", "jane"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Email\tFirst name\tLast name\tStatus\tJoined\njane@example.com\tJane\t\tsubscribed\t2026-11-02T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_segment_only_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/segments/seg_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(segment()))
    .mount(&harness.server)
    .await;
  Mock::given(method("DELETE"))
    .and(path("/v1/segments/seg_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .mount(&harness.server)
    .await;
  let is_deleted = |requests: Vec<wiremock::Request>| {
    requests.iter().any(|request| request.method.as_str() == "DELETE")
  };

  let refused = harness.inboxjot().args(["segments", "rm", "seg_1"]).output().unwrap();
  assert_eq!(refused.status.code(), Some(2));
  assert!(!is_deleted(harness.requests().await));

  let output = harness
    .inboxjot()
    .args(["segments", "delete", "seg_1", "-y"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "seg_1\n");
  assert!(is_deleted(harness.requests().await));
}
