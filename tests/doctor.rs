mod support;

use std::net::TcpListener;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

async fn mount_domains(harness: &Harness, domains: Value) {
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": domains, "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;
}

fn find_check<'a>(report: &'a Value, name: &str) -> &'a Value {
  report["checks"]
    .as_array()
    .unwrap()
    .iter()
    .find(|check| check["name"] == name)
    .unwrap_or_else(|| panic!("no {name} check in {report}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_every_check_as_json() {
  let harness = Harness::start().await;
  mount_domains(&harness, json!([{ "name": "acme.com", "status": "verified" }])).await;
  let listener = TcpListener::bind("127.0.0.1:0").unwrap();
  let port = listener.local_addr().unwrap().port();

  let output = harness
    .inboxjot()
    .env("INBOXJOT_SMTP_HOST", format!("127.0.0.1:{port}"))
    .args(["doctor", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  let report = parse_json(&output.stdout);
  assert_eq!(find_check(&report, "credential")["status"], "pass");
  assert_eq!(find_check(&report, "api")["status"], "pass");
  assert_eq!(find_check(&report, "clock")["status"], "pass");
  assert_eq!(find_check(&report, "domains")["status"], "pass");
  assert_eq!(find_check(&report, "domains")["message"], "acme.com");
  assert_eq!(find_check(&report, "smtp")["status"], "pass");
  drop(listener);
}

#[tokio::test(flavor = "multi_thread")]
async fn exits_with_one_when_no_domain_is_verified() {
  let harness = Harness::start().await;
  mount_domains(
    &harness,
    json!([
      { "name": "acme.com", "status": "pending" },
      { "name": "acme.dev", "status": "failed" }
    ]),
  )
  .await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_SMTP_HOST", "127.0.0.1")
    .args(["doctor", "--json"])
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(1));
  let report = parse_json(&output.stdout);
  let domains = find_check(&report, "domains");
  assert_eq!(domains["status"], "fail");
  assert_eq!(domains["fix"], "inboxjot domains add <domain>");
}

#[tokio::test(flavor = "multi_thread")]
async fn warns_when_the_key_cannot_read_domains() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/domains"))
    .respond_with(ResponseTemplate::new(403).set_body_json(json!({
      "name": "forbidden", "status": 403, "message": "Forbidden", "errors": [], "request_id": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .env("INBOXJOT_SMTP_HOST", "127.0.0.1")
    .args(["doctor", "--json"])
    .output()
    .unwrap();

  let report = parse_json(&output.stdout);
  assert_eq!(find_check(&report, "api")["status"], "warn");
  assert!(
    report["checks"]
      .as_array()
      .unwrap()
      .iter()
      .all(|check| check["name"] != "domains")
  );
}
