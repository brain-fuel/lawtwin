//! Receiver boundary tests: completion needs independent producer accounting;
//! faults, stale identities and partial exports cannot manufacture success.
use axum::{Router, body::Body, http::Request};
use http_body_util::BodyExt;
use lawtwin_observation::{Config, Observer};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use tower::ServiceExt;

static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    observer: Observer,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "lawtwin-observation-{}-{}.jsonl",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let config: Config = serde_json::from_value(json!({
            "runId":"run-1", "topologyDigest": format!("sha256:{}", "a".repeat(64)),
            "producers":{"worker":{"node":"worker","implementation":"build-1","revision":"rev-1"}},
            "edges":{"send":"worker"}, "journal":path
        }))
        .unwrap();
        Self {
            observer: Observer::create(config).unwrap(),
            path,
        }
    }
    async fn admin(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        call(self.observer.admin_router(), method, path, body).await
    }
    async fn logs(&self, body: Value) -> (u16, Value) {
        call(self.observer.intake_router(), "POST", "/v1/logs", body).await
    }
    async fn checkpoint(&self, logs: u64, flushed: bool, dropped: u64) {
        assert_eq!(self.admin("POST", "/v1/checkpoint", json!({"producer":"worker","counts":{"logs":logs,"spans":0},"flushed":flushed,"dropped":dropped})).await.0, 200);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn call(router: Router, method: &str, path: &str, body: Value) -> (u16, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn batch() -> Value {
    let attrs = [
        ("lawtwin.producer.id", "worker".to_string()),
        ("ironframe.execution.id", "execution-1".to_string()),
        ("ironframe.run.id", "run-1".to_string()),
        (
            "ironframe.topology.digest",
            format!("sha256:{}", "a".repeat(64)),
        ),
        ("ironframe.node.id", "worker".to_string()),
        ("ironframe.configuration.revision", "rev-1".to_string()),
        ("service.version", "build-1".to_string()),
    ]
    .into_iter()
    .map(|(k, v)| json!({"key":k,"value":{"stringValue":v}}))
    .collect::<Vec<_>>();
    json!({"resourceLogs":[{"resource":{"attributes":attrs},"scopeLogs":[{"logRecords":[{"body":{"stringValue":"accepted"},"attributes":[{"key":"ironframe.edge.id","value":{"stringValue":"send"}}]}]}]}]})
}

#[tokio::test]
async fn exports_then_seals_only_after_matching_flush() {
    let f = Fixture::new();
    assert_eq!(f.admin("GET", "/v1/bundle", Value::Null).await.0, 409);
    assert_eq!(f.logs(batch()).await.0, 200);
    f.checkpoint(1, true, 0).await;
    let sealed = f.admin("POST", "/v1/seal", json!({})).await.1;
    assert_eq!(sealed["status"], "complete");
    assert_eq!(sealed["counts"]["worker"]["logs"], 1);
    assert_eq!(f.admin("GET", "/v1/bundle", Value::Null).await.1, sealed);
    assert_eq!(f.logs(batch()).await.0, 409);
    assert_eq!(f.admin("POST", "/v1/faults", json!({})).await.0, 409);
    assert!(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .contains("\"kind\":\"seal\"")
    );
}

#[tokio::test]
async fn rejects_cross_run_and_cross_edge_batches_atomically() {
    let f = Fixture::new();
    let mut wrong = batch();
    wrong["resourceLogs"][0]["resource"]["attributes"][2]["value"]["stringValue"] =
        json!("other-run");
    assert_eq!(f.logs(wrong).await.0, 400);
    let mut wrong = batch();
    wrong["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"][0]["value"]["stringValue"] =
        json!("not-an-edge");
    assert_eq!(f.logs(wrong).await.0, 400);
    assert_eq!(
        f.admin("GET", "/v1/state", Value::Null).await.1["counts"],
        json!({})
    );
}

#[tokio::test]
async fn loss_remains_incomplete_after_fault_is_cleared() {
    let f = Fixture::new();
    assert_eq!(
        f.admin("POST", "/v1/faults", json!({"dropRequests":1}))
            .await
            .0,
        200
    );
    let reply = f.logs(batch()).await;
    assert_eq!(reply.1["partialSuccess"]["rejectedLogRecords"], "1");
    f.admin("POST", "/v1/faults", json!({})).await;
    f.checkpoint(1, true, 0).await;
    assert_eq!(
        f.admin("POST", "/v1/seal", json!({})).await.1["status"],
        "incomplete"
    );
}

#[tokio::test]
async fn unavailable_can_recover_with_successful_retry() {
    let f = Fixture::new();
    f.admin("POST", "/v1/faults", json!({"unavailableRequests":1}))
        .await;
    assert_eq!(f.logs(batch()).await.0, 503);
    assert_eq!(f.logs(batch()).await.0, 200);
    f.checkpoint(1, true, 0).await;
    assert_eq!(
        f.admin("POST", "/v1/seal", json!({})).await.1["status"],
        "complete"
    );
}

#[tokio::test]
async fn missing_flush_mismatch_and_producer_drops_are_incomplete() {
    for mode in 0..4 {
        let f = Fixture::new();
        f.logs(batch()).await;
        match mode {
            0 => {}
            1 => f.checkpoint(2, true, 0).await,
            2 => f.checkpoint(1, false, 0).await,
            _ => f.checkpoint(1, true, 1).await,
        }
        assert_eq!(
            f.admin("POST", "/v1/seal", json!({})).await.1["status"],
            "incomplete"
        );
    }
}

#[tokio::test]
async fn checkpoints_are_final_and_existing_journals_are_never_overwritten() {
    let f = Fixture::new();
    f.checkpoint(0, true, 0).await;
    assert_eq!(f.logs(batch()).await.0, 409);
    assert_eq!(
        f.admin(
            "POST",
            "/v1/checkpoint",
            json!({"producer":"worker","counts":{"logs":0,"spans":0},"flushed":true,"dropped":0})
        )
        .await
        .0,
        409
    );
    let header: Value = serde_json::from_str(
        std::fs::read_to_string(&f.path)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let config: Config = serde_json::from_value(header["config"].clone()).unwrap();
    assert!(Observer::create(config).is_err());
}

#[tokio::test]
async fn logs_and_spans_have_independent_flush_counts() {
    let f = Fixture::new();
    let logs = batch();
    let resource = logs["resourceLogs"][0]["resource"].clone();
    let traces = json!({"resourceSpans":[{"resource":resource,"scopeSpans":[{"spans":[{"traceId":"a".repeat(32),"spanId":"b".repeat(16),"name":"send"}]}]}]});
    assert_eq!(f.logs(logs).await.0, 200);
    assert_eq!(
        call(f.observer.intake_router(), "POST", "/v1/traces", traces)
            .await
            .0,
        200
    );
    assert_eq!(
        f.admin(
            "POST",
            "/v1/checkpoint",
            json!({"producer":"worker","counts":{"logs":1,"spans":1},"flushed":true,"dropped":0})
        )
        .await
        .0,
        200
    );
    let sealed = f.admin("POST", "/v1/seal", json!({})).await.1;
    assert_eq!(sealed["status"], "complete");
    assert_eq!(sealed["executions"]["worker"], json!(["execution-1"]));
}

#[tokio::test]
async fn a_bad_resource_after_a_good_one_does_not_partially_append() {
    let f = Fixture::new();
    let mut request = batch();
    let mut bad = request["resourceLogs"][0].clone();
    bad["resource"]["attributes"][0]["value"]["stringValue"] = json!("unknown-producer");
    request["resourceLogs"].as_array_mut().unwrap().push(bad);
    assert_eq!(f.logs(request).await.0, 400);
    assert_eq!(
        f.admin("GET", "/v1/state", Value::Null).await.1["counts"],
        json!({})
    );
}
