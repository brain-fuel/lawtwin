//! Every fault the twin file declares, as a real client (reqwest) sees it.

use lawtwin_http_conformance::core::TestTwin;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn pet() -> Value {
    json!({ "id": 1, "name": "Rex", "photoUrls": [] })
}

fn client(timeout_ms: u64) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder().timeout(Duration::from_millis(timeout_ms)).build().unwrap()
}

fn raise(twin: &TestTwin, name: &str, params: Value) {
    let (status, v) = twin.admin("POST", &format!("/v1/faults/{name}"), &params.to_string());
    assert_eq!(status, 201, "{name}: {v}");
}

#[test]
fn latency_holds_answers_back() {
    let twin = TestTwin::start("petstore", 1);
    raise(&twin, "latency", json!({ "delay": "300ms" }));
    let t = Instant::now();
    let r = client(5000).get(format!("{}/pet/1", twin.url())).send().unwrap();
    assert_eq!(r.status(), 404);
    assert!(t.elapsed() >= Duration::from_millis(300));
}

#[test]
fn server_error_answers_5xx_before_or_after_the_model() {
    let twin = TestTwin::start("petstore", 1);
    let c = client(5000);
    raise(&twin, "server-error", json!({ "status": 503, "count": 1, "operation": "addPet" }));
    let r = c.post(format!("{}/pet", twin.url())).json(&pet()).send().unwrap();
    assert_eq!(r.status(), 503);
    assert_eq!(c.get(format!("{}/pet/1", twin.url())).send().unwrap().status(), 404, "a refused create wrote nothing");
    raise(&twin, "server-error", json!({ "count": 1, "operation": "addPet", "applied": true }));
    let r = c.post(format!("{}/pet", twin.url())).json(&pet()).send().unwrap();
    assert_eq!(r.status(), 500);
    assert_eq!(c.get(format!("{}/pet/1", twin.url())).send().unwrap().status(), 200, "an applied create took effect");
    // The client's retry finds the key taken.
    assert_eq!(c.post(format!("{}/pet", twin.url())).json(&pet()).send().unwrap().status(), 422);
    twin.assert_lawful();
}

#[test]
fn rate_limit_answers_429_with_retry_after_until_the_window_ends() {
    let twin = TestTwin::start("github", 1);
    let c = client(5000);
    raise(&twin, "rate-limit", json!({ "limit": 2, "window": "30s" }));
    let url = format!("{}/repos/o/r/labels", twin.url());
    assert_eq!(c.get(&url).send().unwrap().status(), 200);
    assert_eq!(c.get(&url).send().unwrap().status(), 200);
    let r = c.get(&url).send().unwrap();
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()["retry-after"], "30");
    twin.admin("POST", "/v1/clock/advance", r#"{"by":"20s"}"#);
    assert_eq!(c.get(&url).send().unwrap().headers()["retry-after"], "10");
    twin.admin("POST", "/v1/clock/advance", r#"{"by":"10s"}"#);
    assert_eq!(c.get(&url).send().unwrap().status(), 200);
}

#[test]
fn timeout_sends_nothing_and_the_client_gives_up() {
    let twin = TestTwin::start("petstore", 1);
    raise(&twin, "timeout", json!({ "hold": "5s", "count": 1 }));
    let t = Instant::now();
    let e = client(400).get(format!("{}/pet/1", twin.url())).send().unwrap_err();
    assert!(e.is_timeout(), "{e}");
    assert!(t.elapsed() < Duration::from_secs(4));
    assert_eq!(client(2000).get(format!("{}/pet/1", twin.url())).send().unwrap().status(), 404, "the fault was used up");
}

#[test]
fn connection_reset_breaks_the_connection() {
    let twin = TestTwin::start("petstore", 1);
    raise(&twin, "connection-reset", json!({ "count": 1, "operation": "addPet", "applied": true }));
    let c = client(2000);
    assert!(c.post(format!("{}/pet", twin.url())).json(&pet()).send().is_err());
    assert_eq!(c.get(format!("{}/pet/1", twin.url())).send().unwrap().status(), 200, "the reset came after the create");
}

#[test]
fn slow_body_trickles_and_arrives_whole() {
    let twin = TestTwin::start("petstore", 1);
    let c = client(10_000);
    c.post(format!("{}/pet", twin.url())).json(&pet()).send().unwrap();
    raise(&twin, "slow-body", json!({ "chunk": 4, "delay": "20ms", "operation": "getPetById" }));
    let t = Instant::now();
    let v: Value = c.get(format!("{}/pet/1", twin.url())).send().unwrap().json().unwrap();
    assert_eq!(v, json!({ "id": 1, "name": "Rex", "photoUrls": [] }));
    assert!(t.elapsed() >= Duration::from_millis(100), "{:?}", t.elapsed());
}

#[test]
fn undeclared_faults_and_parameters_are_refused() {
    let twin = TestTwin::start("petstore", 1);
    assert_eq!(twin.admin("POST", "/v1/faults/broker-unavailable", "{}").0, 400);
    assert_eq!(twin.admin("POST", "/v1/faults/latency", "{}").0, 400, "delay is required");
    assert_eq!(twin.admin("POST", "/v1/faults/server-error", r#"{"status":404}"#).0, 400);
    assert_eq!(twin.admin("POST", "/v1/faults/latency", r#"{"delay":"1s","operation":"noSuchOperation"}"#).0, 400);
    let (_, faults) = twin.admin("GET", "/v1/faults", "");
    let declared: Vec<&str> = faults["declared"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(declared, ["latency", "server-error", "rate-limit", "timeout", "connection-reset", "slow-body"]);
}
