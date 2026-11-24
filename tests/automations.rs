mod support;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

use support::Harness;

fn automation(status: &str) -> Value {
  json!({
    "id": "aut_1",
    "name": "Welcome",
    "description": null,
    "status": status,
    "trigger_label": "Signed up",
    "entered_count": 10,
    "active_count": 2,
    "completed_count": 8,
    "exited_count": 0,
    "sent_count": 20,
    "turned_on_at": null,
    "created_at": "2026-11-01T10:00:00Z",
    "updated_at": "2026-11-02T10:00:00Z",
    "problems": [{ "step_id": "s1", "message": "Pick a template." }]
  })
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_automations_by_status() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/automations"))
    .and(query_param("status", "on"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "data": [automation("on")], "next_cursor": null, "prev_cursor": null
    })))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["automations", "list", "--status", "on"])
    .output()
    .unwrap();

  assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "ID\tName\tStatus\tTrigger\tActive\tSent\tUpdated\naut_1\tWelcome\ton\tSigned up\t2\t20\t2026-11-02T10:00:00Z\n"
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn shows_the_problems_of_an_automation() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("GET"))
    .and(path("/v1/automations/aut_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(automation("draft")))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["automations", "inspect", "aut_1"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert!(
    String::from_utf8(output.stdout)
      .unwrap()
      .ends_with("Problems\n  Pick a template.\n")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn turns_an_automation_on_and_off() {
  let harness = Harness::start().await;
  harness.save_browser_login(i64::MAX);
  Mock::given(method("POST"))
    .and(path("/v1/automations/aut_1/status"))
    .respond_with(ResponseTemplate::new(200).set_body_json(automation("on")))
    .mount(&harness.server)
    .await;

  let enable = harness
    .inboxjot()
    .args(["automations", "enable", "aut_1"])
    .output()
    .unwrap();
  let disable = harness
    .inboxjot()
    .args(["automations", "disable", "aut_1"])
    .output()
    .unwrap();

  assert!(enable.status.success());
  assert!(disable.status.success());
  assert_eq!(String::from_utf8(enable.stdout).unwrap(), "aut_1\n");
  let bodies: Vec<Value> = harness
    .requests()
    .await
    .iter()
    .map(|request| request.body_json().unwrap())
    .collect();
  assert_eq!(bodies, [json!({ "status": "on" }), json!({ "status": "off" })]);
}
