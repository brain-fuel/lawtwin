//! Forced states, as a client sees them: a state the contract's operations
//! reach is reached through them and reads back over HTTP; one they do not
//! reach is refused with the law that forbids it, and nothing changes.

use lawtwin_http_conformance::core::TestTwin;
use serde_json::{Value, json};

#[test]
fn a_reachable_state_reads_back_over_http() {
    let twin = TestTwin::start("petstore", 3);
    let doc = json!({ "reset": true, "resources": {
        "Pet": [ { "id": 7, "name": "Rex", "photoUrls": ["https://example.com/rex.png"], "status": "available" }, { "name": "Tom", "photoUrls": [] } ],
        "User": [ { "username": "ada", "firstName": "Ada" } ] } });
    let (status, v) = twin.admin("POST", "/v1/state", &doc.to_string());
    assert_eq!((status, &v["forced"]), (200, &json!(true)), "{v}");
    let client = reqwest::blocking::Client::new();
    let rex: Value = client.get(format!("{}/pet/7", twin.url())).send().unwrap().json().unwrap();
    assert_eq!(rex["name"], "Rex");
    // A pet forced without its key gets the next key the service assigns.
    let tom: Value = client.get(format!("{}/pet/8", twin.url())).send().unwrap().json().unwrap();
    assert_eq!(tom["name"], "Tom");
    let ada = client.get(format!("{}/user/ada", twin.url())).send().unwrap();
    assert_eq!(ada.status(), 200);
    // Each record was reached by operations written to the history as forced.
    assert!(twin.history().iter().any(|e| e["source"] == "force" && e["op"] == "addPet"));
    twin.assert_lawful();
}

#[test]
fn an_unreachable_state_is_refused_with_its_law_and_changes_nothing() {
    let twin = TestTwin::start("github", 3);
    let label = json!({ "parents": { "owner": "o", "repo": "r" }, "body": { "name": "bug", "color": "d73a4a" } });
    let (status, _) = twin.admin("POST", "/v1/state", &json!({ "resources": { "Label": [label] } }).to_string());
    assert_eq!(status, 200);
    let before = twin.admin("GET", "/v1/state", "").1;
    // The same label twice: the second create is refused.
    let (status, v) = twin.admin("POST", "/v1/state", &json!({ "resources": { "Label": [label, label] } }).to_string());
    assert_eq!(status, 422);
    assert_eq!(v["law"], "a created label reads back");
    // A milestone numbered as the service would not number it.
    let milestone = json!({ "parents": { "owner": "o", "repo": "r" }, "record": {
        "url": "", "html_url": "", "labels_url": "", "id": 0, "node_id": "", "number": 40, "state": "open", "title": "v1",
        "description": null, "creator": null, "open_issues": 0, "closed_issues": 0, "created_at": "", "updated_at": "", "closed_at": null, "due_on": null } });
    let (status, v) = twin.admin("POST", "/v1/state", &json!({ "resources": { "Milestone": [milestone] } }).to_string());
    assert_eq!(status, 422, "{v}");
    assert_eq!(v["law"], "a created milestone reads back");
    assert_eq!(twin.admin("GET", "/v1/state", "").1, before, "a refused state changed the twin");
    // Not a forced state at all.
    let (status, _) = twin.admin("POST", "/v1/state", r#"{"topics": []}"#);
    assert_eq!(status, 400);
}
