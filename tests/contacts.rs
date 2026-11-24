mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

fn contact() -> Value {
  json!({
    "id": "con_1",
    "email": "jane@example.com",
    "first_name": "Jane",
    "last_name": "Doe",
    "status": "subscribed",
    "language": "en",
    "properties": { "plan": "pro" },
    "segments": [{ "id": "seg_1", "name": "Engaged" }],
    "topics": [{ "id": "top_1", "name": "News", "status": "subscribed", "subscribed": true }],
    "brands": [{
      "id": "brd_1",
      "name": "Acme",
      "status": "subscribed",
      "paused_until": null,
      "unsubscribed_at": null,
      "unsubscribe_source": null
    }],
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-01T10:00:00Z"
  })
}

async fn mount_contact(harness: &Harness) {
  Mock::given(method("GET"))
    .and(path("/v1/contacts/jane@example.com"))
    .respond_with(ResponseTemplate::new(200).set_body_json(contact()))
    .mount(&harness.server)
    .await;
}

fn has_method(requests: &[wiremock::Request], name: &str) -> bool {
  requests.iter().any(|request| request.method.as_str() == name)
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_contacts_with_filters_and_a_segment_name() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/segments"))
    .and(query_param("query", "Engaged"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "seg_1", "name": "Engaged" }], "next_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/contacts"))
    .and(query_param("query", "jane"))
    .and(query_param("status", "not_subscribed"))
    .and(query_param("segment_id", "seg_1"))
    .and(query_param("per_page", "10"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [contact()], "total": 1, "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "ls", "--query", "jane", "--status", "not-subscribed"])
    .args(["--segment", "Engaged", "-n", "10"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Email\tFirst name\tLast name\tStatus\tAdded\njane@example.com\tJane\tDoe\tsubscribed\t2026-11-01T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_contact_body_with_json() {
  let harness = Harness::start().await;
  mount_contact(&harness).await;

  let output = harness
    .inboxjot()
    .args(["contacts", "inspect", "jane@example.com", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), contact());
}

#[tokio::test(flavor = "multi_thread")]
async fn creates_a_contact_with_typed_properties() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts"))
    .respond_with(ResponseTemplate::new(201).set_body_json(contact()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "create", "--email", "jane@example.com", "--first-name", "Jane"])
    .args(["--property", "plan=pro", "--property", "seats=3", "--subscribed", "false"])
    .args(["--segment", "seg_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "con_1\n");
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Saved jane@example.com")
  );
  let requests = harness.requests().await;
  assert_eq!(
    requests[0].body_json::<Value>().unwrap(),
    json!({
      "email": "jane@example.com",
      "first_name": "Jane",
      "properties": { "plan": "pro", "seats": 3 },
      "subscribed": false,
      "segment_ids": ["seg_1"]
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_only_the_given_fields_and_clears_one() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/topics"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "top_1", "name": "News" }], "next_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("PATCH"))
    .and(path("/v1/contacts/jane@example.com"))
    .respond_with(ResponseTemplate::new(200).set_body_json(contact()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "update", "jane@example.com", "--clear", "last-name"])
    .args(["--notes", "VIP", "--unsubscribe-from", "News"])
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
    json!({
      "last_name": null,
      "notes": "VIP",
      "topics": [{ "id": "top_1", "status": "unsubscribed" }]
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn unsubscribes_a_contact_from_a_brand_by_name() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/brands"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [{ "id": "brd_1", "name": "Acme Labs" }], "next_cursor": null
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("PATCH"))
    .and(path("/v1/contacts/jane@example.com"))
    .respond_with(ResponseTemplate::new(200).set_body_json(contact()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "update", "jane@example.com"])
    .args(["--unsubscribe-from-brand", "Acme Labs", "--subscribe-to-brand", "brd_2"])
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
    json!({
      "brands": [
        { "id": "brd_2", "status": "subscribed" },
        { "id": "brd_1", "status": "unsubscribed" }
      ]
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_an_update_without_options() {
  let harness = Harness::start().await;

  let output = harness
    .inboxjot()
    .args(["contacts", "update", "jane@example.com", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "missing_input");
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_a_contact_only_with_yes() {
  let harness = Harness::start().await;
  mount_contact(&harness).await;
  Mock::given(method("DELETE"))
    .and(path("/v1/contacts/con_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .mount(&harness.server)
    .await;

  let refused = harness
    .inboxjot()
    .args(["contacts", "rm", "jane@example.com"])
    .output()
    .unwrap();
  assert_eq!(refused.status.code(), Some(2));
  assert!(String::from_utf8(refused.stderr).unwrap().contains("--yes"));
  assert!(!has_method(&harness.requests().await, "DELETE"));

  let output = harness
    .inboxjot()
    .args(["contacts", "delete", "jane@example.com", "--yes"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "con_1\n");
  assert!(has_method(&harness.requests().await, "DELETE"));
}

#[tokio::test(flavor = "multi_thread")]
async fn erases_an_address_only_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/contacts/erase"))
    .respond_with(ResponseTemplate::new(202).set_body_json(json!({ "status": "queued" })))
    .mount(&harness.server)
    .await;

  let refused = harness
    .inboxjot()
    .args(["contacts", "erase", "jane@example.com", "--json"])
    .output()
    .unwrap();
  assert_eq!(refused.status.code(), Some(2));
  let error = parse_json(&refused.stderr);
  assert_eq!(error["error"]["name"], "confirmation_required");
  assert_eq!(
    error["error"]["next"],
    "inboxjot contacts erase jane@example.com --json --yes"
  );
  assert!(harness.requests().await.is_empty());

  let output = harness
    .inboxjot()
    .args(["contacts", "erase", "jane@example.com", "--json", "--yes"])
    .output()
    .unwrap();
  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), json!({ "status": "queued" }));
  assert_eq!(
    harness.requests().await[0].body_json::<Value>().unwrap(),
    json!({ "email": "jane@example.com" })
  );
}

const CSV: &str =
  "Email,First Name,Plan,Color\njane@example.com,Jane,pro,blue\nbad,Bo,free,red\n";

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_mapping_of_a_dry_run_without_network() {
  let harness = Harness::start().await;
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("people.csv");
  std::fs::write(&file, CSV).unwrap();

  let output = harness
    .inboxjot()
    .args(["contacts", "import"])
    .arg(&file)
    .arg("--dry-run")
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "Column\tField\nEmail\temail\nFirst Name\tfirst_name\nPlan\tproperty plan\nColor\tproperty color\n"
  );
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn imports_a_csv_and_prints_the_counts() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  let uri = harness.server.uri();
  let imports = "/v1/contact-imports";
  Mock::given(method("GET"))
    .and(path("/v1/contact-properties"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "key": "plan" }])))
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path(format!("{imports}/presign")))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "upload_url": format!("{uri}/upload/people"), "key": "imports/people.csv"
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("PUT"))
    .and(path("/upload/people"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path(imports))
    .respond_with(
      ResponseTemplate::new(201)
        .set_body_json(json!({ "id": "imp_1", "status": "queued" })),
    )
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path(format!("{imports}/imp_1")))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "id": "imp_1", "status": "completed", "total_rows": 2, "processed_rows": 2,
      "created_count": 1, "updated_count": 0, "skipped_count": 1,
      "errors": [{ "row": 2, "email": "bad", "reason": "Not a valid email address" }]
    })))
    .mount(&harness.server)
    .await;
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("people.csv");
  std::fs::write(&file, CSV).unwrap();

  let output = harness
    .inboxjot()
    .args(["contacts", "import"])
    .arg(&file)
    .args(["--segment", "seg_1"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(String::from_utf8(output.stdout).unwrap(), "imp_1\n");
  let stderr = String::from_utf8(output.stderr).unwrap();
  assert!(stderr.contains("Imported 1 contact\n"));
  assert!(stderr.contains("1 new · 0 updated · 1 skipped"));
  assert!(stderr.contains("Row 2 (bad): Not a valid email address"));

  let requests = harness.requests().await;
  let upload = requests
    .iter()
    .find(|request| request.method.as_str() == "PUT")
    .unwrap();
  assert_eq!(upload.body, CSV.as_bytes());
  assert_eq!(upload.headers["content-type"], "text/csv");
  let created = requests
    .iter()
    .find(|request| request.method.as_str() == "POST" && request.url.path() == imports)
    .unwrap();
  assert_eq!(
    created.body_json::<Value>().unwrap(),
    json!({
      "file_key": "imports/people.csv",
      "file_name": "people.csv",
      "total_rows": 2,
      "mapping": { "Email": "email", "First Name": "first_name", "Plan": "plan", "Color": null },
      "segment_id": "seg_1"
    })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn exports_contacts_to_a_file() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  let uri = harness.server.uri();
  Mock::given(method("POST"))
    .and(path("/v1/exports"))
    .respond_with(
      ResponseTemplate::new(201)
        .set_body_json(json!({ "id": "exp_1", "status": "queued" })),
    )
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/v1/exports/exp_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "id": "exp_1", "status": "completed", "row_count": 1,
      "download_url": format!("{uri}/download/contacts.csv")
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("GET"))
    .and(path("/download/contacts.csv"))
    .respond_with(ResponseTemplate::new(200).set_body_string("email\njane@example.com\n"))
    .mount(&harness.server)
    .await;
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("out.csv");

  let output = harness
    .inboxjot()
    .args(["contacts", "export", "--segment", "seg_1", "--output"])
    .arg(&file)
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{}\n", file.display()));
  assert_eq!(std::fs::read_to_string(&file).unwrap(), "email\njane@example.com\n");
  let body: Value = harness.requests().await[0].body_json().unwrap();
  assert_eq!(body, json!({ "kind": "contacts", "filters": { "segment_id": "seg_1" } }));
}

#[tokio::test(flavor = "multi_thread")]
async fn stops_on_an_unknown_export_status() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/exports"))
    .respond_with(
      ResponseTemplate::new(201)
        .set_body_json(json!({ "id": "exp_1", "status": "archived" })),
    )
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "export", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "unknown_status");
  assert_eq!(harness.requests().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn fails_when_the_export_has_no_id() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/exports"))
    .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "status": "queued" })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "export", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "invalid_json");
  assert_eq!(harness.requests().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stops_waiting_for_an_export_at_the_timeout() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/exports"))
    .respond_with(
      ResponseTemplate::new(201)
        .set_body_json(json!({ "id": "exp_1", "status": "exporting" })),
    )
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["contacts", "export", "--timeout", "0s", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "timeout");
}
