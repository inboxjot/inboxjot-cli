mod support;

use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, ResponseTemplate};

use support::{Harness, parse_json};

const PNG: [u8; 12] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

fn image() -> serde_json::Value {
  json!({
    "id": "img_1",
    "file_name": "logo.png",
    "url": "https://images.example.com/logo.png",
    "content_type": "image/png",
    "size_bytes": 12,
    "width": 1,
    "height": 1,
    "creator_name": null,
    "created_at": "2026-11-01T10:00:00Z"
  })
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_an_image_and_prints_its_url() {
  let harness = Harness::start().await;
  Mock::given(method("POST"))
    .and(path("/v1/images/presign"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({
      "upload_url": format!("{}/upload/key_1", harness.server.uri()),
      "key": "image-uploads/key_1"
    })))
    .mount(&harness.server)
    .await;
  Mock::given(method("PUT"))
    .and(path("/upload/key_1"))
    .and(header("content-type", "image/png"))
    .respond_with(ResponseTemplate::new(200))
    .expect(1)
    .mount(&harness.server)
    .await;
  Mock::given(method("POST"))
    .and(path("/v1/images"))
    .respond_with(ResponseTemplate::new(201).set_body_json(image()))
    .mount(&harness.server)
    .await;
  let file = harness.config_dir.path().join("logo.png");
  std::fs::write(&file, PNG).unwrap();

  let output = harness
    .inboxjot()
    .args(["images", "upload"])
    .arg(&file)
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(
    String::from_utf8(output.stdout).unwrap(),
    "https://images.example.com/logo.png\n"
  );
  assert!(
    String::from_utf8(output.stderr)
      .unwrap()
      .contains("Uploaded logo.png")
  );

  let requests = harness.requests().await;
  let paths: Vec<&str> = requests.iter().map(|request| request.url.path()).collect();
  assert_eq!(paths, ["/v1/images/presign", "/upload/key_1", "/v1/images"]);
  assert_eq!(
    requests[0].body_json::<serde_json::Value>().unwrap(),
    json!({ "content_type": "image/png", "size": 12 })
  );
  assert_eq!(requests[1].body, PNG);
  assert!(requests[1].headers.get("authorization").is_none());
  assert_eq!(
    requests[2].body_json::<serde_json::Value>().unwrap(),
    json!({ "key": "image-uploads/key_1", "file_name": "logo.png" })
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_a_file_that_is_not_an_image() {
  let harness = Harness::start().await;
  let file = harness.config_dir.path().join("notes.png");
  std::fs::write(&file, "hello").unwrap();

  let output = harness
    .inboxjot()
    .args(["images", "upload", "--json"])
    .arg(&file)
    .output()
    .unwrap();

  assert_eq!(output.status.code(), Some(2));
  assert_eq!(parse_json(&output.stderr)["error"]["name"], "unsupported_image");
  assert!(harness.requests().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn prints_the_image_body_with_json() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/images/img_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(image()))
    .mount(&harness.server)
    .await;

  let output = harness
    .inboxjot()
    .args(["images", "inspect", "img_1", "--json"])
    .output()
    .unwrap();

  assert!(output.status.success());
  assert_eq!(parse_json(&output.stdout), image());
}

#[tokio::test(flavor = "multi_thread")]
async fn deletes_an_image_only_with_yes() {
  let harness = Harness::start().await;
  Mock::given(method("GET"))
    .and(path("/v1/images/img_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(image()))
    .mount(&harness.server)
    .await;
  Mock::given(method("DELETE"))
    .and(path("/v1/images/img_1"))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "status": "ok" })))
    .expect(1)
    .mount(&harness.server)
    .await;

  let refused = harness.inboxjot().args(["images", "rm", "img_1"]).output().unwrap();
  let deleted = harness
    .inboxjot()
    .args(["images", "rm", "img_1", "--yes"])
    .output()
    .unwrap();

  assert_eq!(refused.status.code(), Some(2));
  assert!(deleted.status.success());
  assert_eq!(String::from_utf8(deleted.stdout).unwrap(), "img_1\n");
}
