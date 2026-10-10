//! Real clients against the twin, as an application would use them: typed
//! reqwest code for the Petstore and for GitHub's labels and milestones,
//! and curl. Each ends with the core's history replayed and its laws checked.

use lawtwin_http_conformance::core::TestTwin;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Command;

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(rename_all = "camelCase")]
struct Pet {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    name: String,
    photo_urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Label {
    id: i64,
    node_id: String,
    url: String,
    name: String,
    description: Option<String>,
    color: String,
    default: bool,
}

#[derive(Deserialize, Debug)]
struct Milestone {
    number: i64,
    title: String,
    state: String,
    due_on: Option<String>,
}

#[test]
fn a_petstore_client_written_with_reqwest() {
    let twin = TestTwin::start("petstore", 8);
    let c = reqwest::blocking::Client::new();
    let base = twin.url();
    let rex = Pet { id: None, name: "Rex".into(), photo_urls: vec!["https://example.com/rex.png".into()], status: Some("available".into()) };
    let created: Pet = c.post(format!("{base}/pet")).json(&rex).send().unwrap().error_for_status().unwrap().json().unwrap();
    let id = created.id.expect("the service assigns an id");
    assert_eq!(Pet { id: Some(id), ..rex.clone() }, created);
    let read: Pet = c.get(format!("{base}/pet/{id}")).send().unwrap().json().unwrap();
    assert_eq!(read, created);
    let sold = Pet { status: Some("sold".into()), ..created.clone() };
    let replaced: Pet = c.put(format!("{base}/pet")).json(&sold).send().unwrap().json().unwrap();
    assert_eq!(replaced, sold);
    // The document's 400 for a pet without its required name.
    let bad = c.post(format!("{base}/pet")).json(&json!({ "photoUrls": [] })).send().unwrap();
    assert_eq!(bad.status(), 400);
    // An order with a ship date that is not a date-time is refused 422.
    let order = c.post(format!("{base}/store/order")).json(&json!({ "petId": id, "shipDate": "tomorrow" })).send().unwrap();
    assert_eq!(order.status(), 422);
    let order: Value = c.post(format!("{base}/store/order")).json(&json!({ "petId": id, "quantity": 2, "shipDate": "2026-01-02T10:00:00Z", "status": "placed" })).send().unwrap().json().unwrap();
    assert_eq!(order["quantity"], 2);
    assert_eq!(c.delete(format!("{base}/pet/{id}")).send().unwrap().status(), 200);
    assert_eq!(c.get(format!("{base}/pet/{id}")).send().unwrap().status(), 404);
    twin.assert_lawful();
}

#[test]
fn a_github_client_written_with_reqwest() {
    let twin = TestTwin::start("github", 8);
    let c = reqwest::blocking::Client::new();
    let repo = format!("{}/repos/octocat/hello-world", twin.url());
    let r = c.post(format!("{repo}/labels")).json(&json!({ "name": "bug", "color": "d73a4a", "description": "Something is broken" })).send().unwrap();
    assert_eq!(r.status(), 201);
    assert_eq!(r.headers()["location"], format!("{repo}/labels/bug"));
    let bug: Label = r.json().unwrap();
    assert_eq!((bug.name.as_str(), bug.color.as_str()), ("bug", "d73a4a"));
    // The same name again: the document's 422 with its validation-error body.
    let again = c.post(format!("{repo}/labels")).json(&json!({ "name": "bug" })).send().unwrap();
    assert_eq!(again.status(), 422);
    let problem: Value = again.json().unwrap();
    assert!(problem["message"].is_string() && problem["documentation_url"].is_string(), "{problem}");
    let changed: Label = c.patch(format!("{repo}/labels/bug")).json(&json!({ "color": "ffffff" })).send().unwrap().json().unwrap();
    assert_eq!((changed.color.as_str(), changed.description.as_deref()), ("ffffff", Some("Something is broken")));
    let labels: Vec<Label> = c.get(format!("{repo}/labels")).send().unwrap().json().unwrap();
    assert_eq!(labels, vec![changed]);
    let m: Milestone = c.post(format!("{repo}/milestones")).json(&json!({ "title": "v1.0", "due_on": "2026-03-01T00:00:00Z" })).send().unwrap().json().unwrap();
    assert_eq!((m.number, m.state.as_str(), m.due_on.as_deref()), (1, "open", Some("2026-03-01T00:00:00Z")));
    let closed: Milestone = c.patch(format!("{repo}/milestones/1")).json(&json!({ "state": "closed" })).send().unwrap().json().unwrap();
    assert_eq!((closed.title.as_str(), closed.state.as_str()), ("v1.0", "closed"));
    assert_eq!(c.delete(format!("{repo}/milestones/1")).send().unwrap().status(), 204);
    let gone = c.get(format!("{repo}/milestones/1")).send().unwrap();
    assert_eq!(gone.status(), 404);
    assert!(gone.json::<Value>().unwrap()["message"].is_string());
    // Another repository holds no labels.
    let other: Vec<Label> = c.get(format!("{}/repos/octocat/other/labels", twin.url())).send().unwrap().json().unwrap();
    assert!(other.is_empty());
    twin.assert_lawful();
}

fn curl(args: &[&str]) -> (u16, String) {
    let out = Command::new("curl").args(["-s", "-w", "\n%{http_code}"]).args(args).output().expect("curl");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap();
    (code.parse().unwrap(), body.to_string())
}

#[test]
fn curl_against_both_contracts() {
    if Command::new("curl").arg("--version").output().is_err() {
        eprintln!("curl is not installed; skipped");
        return;
    }
    let pets = TestTwin::start("petstore", 9);
    let base = pets.url();
    let (s, body) = curl(&["-X", "POST", &format!("{base}/pet"), "-H", "Content-Type: application/json", "-d", r#"{"id":10,"name":"Rex","photoUrls":[]}"#]);
    assert_eq!((s, body.as_str()), (200, r#"{"id":10,"name":"Rex","photoUrls":[]}"#));
    assert_eq!(curl(&[&format!("{base}/pet/10")]).0, 200);
    assert_eq!(curl(&[&format!("{base}/pet/eleven")]).0, 400);
    assert_eq!(curl(&["-X", "DELETE", &format!("{base}/pet/10")]).0, 200);
    assert_eq!(curl(&[&format!("{base}/pet/10")]).0, 404);
    pets.assert_lawful();
    let gh = TestTwin::start("github", 9);
    let repo = format!("{}/repos/o/r", gh.url());
    let (s, _) = curl(&["-X", "POST", &format!("{repo}/labels"), "-H", "Content-Type: application/json", "-d", r#"{"name":"good first issue"}"#]);
    assert_eq!(s, 201);
    let (s, body) = curl(&[&format!("{repo}/labels/good%20first%20issue")]);
    assert_eq!(s, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["name"], "good first issue");
    assert_eq!(curl(&["-X", "POST", &format!("{repo}/labels"), "-H", "Content-Type: application/json", "-d", "{not json"]).0, 400);
    gh.assert_lawful();
}
