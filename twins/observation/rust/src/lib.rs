//! A disposable observation sink, not a simulation of a cloud logging backend.
//!
//! OTLP/HTTP JSON is retained verbatim behind a run-specific identity boundary.
//! A producer must force-flush its SDK and checkpoint its expected counts before
//! sealing. Receiver acknowledgements alone never establish complete collection.
//! Precedent: https://opentelemetry.io/docs/specs/otlp/ and the Logs SDK ForceFlush.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

/// Exact identities of the producers whose flush checkpoints are required.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Producer {
    pub node: String,
    pub implementation: String,
    pub revision: String,
}

/// One immutable run; a fresh receiver never overwrites another run's journal.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    pub run_id: String,
    pub topology_digest: String,
    pub producers: BTreeMap<String, Producer>,
    /// Edge id to source node; prevents observations claiming another node's edge.
    #[serde(default)]
    pub edges: BTreeMap<String, String>,
    pub journal: PathBuf,
    #[serde(default = "default_limit")]
    pub max_records: u64,
    #[serde(default = "default_bytes")]
    pub max_journal_bytes: u64,
}

fn default_limit() -> u64 {
    100_000
}

fn default_bytes() -> u64 {
    64 * 1024 * 1024
}

/// Counts cover logs and spans independently; transport retries are visible.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    pub logs: u64,
    pub spans: u64,
}

impl Counts {
    fn add(&mut self, other: Self) {
        self.logs += other.logs;
        self.spans += other.spans;
    }

    fn total(self) -> u64 {
        self.logs + self.spans
    }
}

/// Submitted by the runner only after the producer's SDK reports ForceFlush.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Checkpoint {
    pub producer: String,
    pub counts: Counts,
    pub flushed: bool,
    pub dropped: u64,
}

/// Faults are deliberately scoped to the receiver, leaving runner capture alone.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Faults {
    #[serde(default)]
    pub unavailable_requests: u64,
    #[serde(default)]
    pub drop_requests: u64,
    #[serde(default)]
    pub delay_ms: u64,
}

struct Journal {
    config: Config,
    file: File,
    rows: Vec<Value>,
    counts: BTreeMap<String, Counts>,
    checkpoints: BTreeMap<String, Checkpoint>,
    executions: BTreeMap<String, BTreeSet<String>>,
    faults: Faults,
    lost: u64,
    failure: Option<String>,
    sealed: Option<Value>,
    bytes: u64,
}

/// Cloneable fixture handle. All mutations are serialized with journal writes.
#[derive(Clone)]
pub struct Observer(Arc<Mutex<Journal>>);

type Reply = (StatusCode, Json<Value>);

fn error(status: StatusCode, message: impl ToString) -> Reply {
    (status, Json(json!({"error": message.to_string()})))
}

impl Observer {
    pub fn create(config: Config) -> Result<Self, String> {
        if config.run_id.is_empty()
            || !valid_digest(&config.topology_digest)
            || config.producers.is_empty()
            || config.max_records == 0
            || config.max_journal_bytes == 0
        {
            return Err(
                "runId, SHA-256 topologyDigest, producers and positive maxRecords are required"
                    .into(),
            );
        }
        for (id, producer) in &config.producers {
            if id.is_empty()
                || producer.node.is_empty()
                || producer.implementation.is_empty()
                || producer.revision.is_empty()
            {
                return Err("producer identity fields must be nonempty".into());
            }
        }
        if config.edges.iter().any(|(edge, node)| {
            edge.is_empty() || !config.producers.values().any(|p| &p.node == node)
        }) {
            return Err("each edge must name a configured source node".into());
        }
        if let Some(parent) = config.journal.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&config.journal)
            .map_err(|e| e.to_string())?;
        let mut journal = Journal {
            config,
            file,
            rows: vec![],
            counts: BTreeMap::new(),
            checkpoints: BTreeMap::new(),
            executions: BTreeMap::new(),
            faults: Faults::default(),
            lost: 0,
            failure: None,
            sealed: None,
            bytes: 0,
        };
        journal
            .append(json!({"kind": "start", "config": journal.config}))
            .map_err(|e| e.to_string())?;
        Ok(Self(Arc::new(Mutex::new(journal))))
    }

    /// OTLP endpoints may be exposed to applications; admin routes stay private.
    pub fn intake_router(&self) -> Router {
        Router::new()
            .route("/healthz", get(|| async { "ok\n" }))
            .route("/v1/logs", post(logs))
            .route("/v1/traces", post(traces))
            .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
            .with_state(self.clone())
    }

    pub fn admin_router(&self) -> Router {
        Router::new()
            .route("/healthz", get(|| async { "ok\n" }))
            .route("/v1/state", get(inspect))
            .route("/v1/checkpoint", post(checkpoint))
            .route("/v1/faults", post(faults))
            .route("/v1/seal", post(seal))
            .route("/v1/bundle", get(bundle))
            .with_state(self.clone())
    }
}

fn valid_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

impl Journal {
    fn append(&mut self, value: Value) -> std::io::Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        if bytes.len() as u64 > self.config.max_journal_bytes.saturating_sub(self.bytes) {
            self.failure = Some("journal byte limit reached".into());
            return Err(std::io::Error::other("journal byte limit reached"));
        }
        if let Err(e) = self
            .file
            .write_all(&bytes)
            .and_then(|_| self.file.sync_data())
        {
            self.failure = Some(e.to_string());
            return Err(e);
        }
        self.rows.push(value);
        self.bytes += bytes.len() as u64;
        Ok(())
    }

    fn mutable(&self) -> Result<(), Reply> {
        if self.sealed.is_some() {
            Err(error(StatusCode::CONFLICT, "run is sealed"))
        } else if let Some(failure) = &self.failure {
            Err(error(StatusCode::INSUFFICIENT_STORAGE, failure))
        } else {
            Ok(())
        }
    }
}

fn string_attribute<'a>(attributes: &'a Value, name: &str) -> Option<&'a str> {
    attributes.as_array()?.iter().find(|a| a["key"] == name)?["value"]["stringValue"].as_str()
}

/// Validate every resource and edge before retaining any part of a batch.
fn validate_batch(
    config: &Config,
    signal: &str,
    value: &Value,
) -> Result<BTreeMap<String, Counts>, String> {
    let (resources, scopes, records) = if signal == "logs" {
        ("resourceLogs", "scopeLogs", "logRecords")
    } else {
        ("resourceSpans", "scopeSpans", "spans")
    };
    let batches = value
        .get(resources)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{resources} must be an array"))?;
    let mut counts = BTreeMap::<String, Counts>::new();
    for resource in batches {
        let attributes = &resource["resource"]["attributes"];
        let producer_id = string_attribute(attributes, "lawtwin.producer.id")
            .ok_or("resource needs lawtwin.producer.id")?;
        let producer = config
            .producers
            .get(producer_id)
            .ok_or("unknown producer")?;
        if string_attribute(attributes, "ironframe.execution.id").is_none_or(str::is_empty) {
            return Err("resource needs ironframe.execution.id".into());
        }
        for (key, expected) in [
            ("ironframe.run.id", &config.run_id),
            ("ironframe.topology.digest", &config.topology_digest),
            ("ironframe.node.id", &producer.node),
            ("ironframe.configuration.revision", &producer.revision),
            ("service.version", &producer.implementation),
        ] {
            if string_attribute(attributes, key) != Some(expected.as_str()) {
                return Err(format!(
                    "producer {producer_id}: missing or mismatched {key}"
                ));
            }
        }
        for scope in resource
            .get(scopes)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{scopes} must be an array"))?
        {
            for record in scope
                .get(records)
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{records} must be an array"))?
            {
                if !record.is_object() {
                    return Err("each record must be an object".into());
                }
                if let Some(edge) = string_attribute(&record["attributes"], "ironframe.edge.id") {
                    if config.edges.get(edge) != Some(&producer.node) {
                        return Err(format!("producer {producer_id} cannot claim edge {edge}"));
                    }
                }
                let count = counts.entry(producer_id.to_string()).or_default();
                if signal == "logs" {
                    count.logs += 1;
                } else {
                    count.spans += 1;
                }
            }
        }
    }
    Ok(counts)
}

async fn logs(
    State(observer): State<Observer>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Reply {
    intake(observer, headers, value, "logs").await
}

async fn traces(
    State(observer): State<Observer>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Reply {
    intake(observer, headers, value, "traces").await
}

async fn intake(observer: Observer, headers: HeaderMap, value: Value, signal: &str) -> Reply {
    if headers
        .get("content-encoding")
        .is_some_and(|v| v != "identity")
    {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "only uncompressed OTLP/HTTP JSON is supported; use a Collector bridge for protobuf/gRPC",
        );
    }
    let delay = observer.0.lock().unwrap().faults.delay_ms;
    tokio::time::sleep(Duration::from_millis(delay)).await;
    let mut journal = observer.0.lock().unwrap();
    if let Err(reply) = journal.mutable() {
        return reply;
    }
    let counts = match validate_batch(&journal.config, signal, &value) {
        Ok(counts) => counts,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    if counts.keys().any(|id| journal.checkpoints.contains_key(id)) {
        return error(
            StatusCode::CONFLICT,
            "producer already checkpointed; no exports may follow its flush checkpoint",
        );
    }
    if journal.faults.unavailable_requests > 0 {
        journal.faults.unavailable_requests -= 1;
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "declared receiver-unavailable fault",
        );
    }
    let incoming: u64 = counts.values().map(|c| c.total()).sum();
    let retained: u64 = journal.counts.values().map(|c| c.total()).sum();
    if incoming > journal.config.max_records.saturating_sub(retained) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "receiver record limit reached",
        );
    }
    let dropping = journal.faults.drop_requests > 0;
    if dropping {
        journal.faults.drop_requests -= 1;
    }
    let resource_key = if signal == "logs" {
        "resourceLogs"
    } else {
        "resourceSpans"
    };
    let executions: Vec<(String, String)> = value[resource_key]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| {
            let attrs = &resource["resource"]["attributes"];
            (
                string_attribute(attrs, "lawtwin.producer.id")
                    .unwrap()
                    .to_string(),
                string_attribute(attrs, "ironframe.execution.id")
                    .unwrap()
                    .to_string(),
            )
        })
        .collect();
    let row = json!({"kind": if dropping { "declared-loss" } else { "export" }, "sequence": journal.rows.len(), "signal": signal, "counts": counts, "request": if dropping { Value::Null } else { value }});
    if let Err(e) = journal.append(row) {
        return error(StatusCode::INSUFFICIENT_STORAGE, e);
    }
    if dropping {
        journal.lost += incoming;
        let rejected = if signal == "logs" {
            "rejectedLogRecords"
        } else {
            "rejectedSpans"
        };
        return (
            StatusCode::OK,
            Json(
                json!({"partialSuccess": {rejected: incoming.to_string(), "errorMessage": "declared receiver drop fault"}}),
            ),
        );
    }
    for (id, count) in counts {
        journal.counts.entry(id).or_default().add(count);
    }
    for (id, execution) in executions {
        journal.executions.entry(id).or_default().insert(execution);
    }
    (StatusCode::OK, Json(json!({})))
}

async fn inspect(State(observer): State<Observer>) -> Reply {
    let journal = observer.0.lock().unwrap();
    (
        StatusCode::OK,
        Json(
            json!({"runId": journal.config.run_id, "counts": journal.counts, "checkpoints": journal.checkpoints, "lost": journal.lost, "failure": journal.failure, "sealed": journal.sealed.is_some(), "faults": journal.faults}),
        ),
    )
}

async fn checkpoint(State(observer): State<Observer>, Json(checkpoint): Json<Checkpoint>) -> Reply {
    let mut journal = observer.0.lock().unwrap();
    if let Err(reply) = journal.mutable() {
        return reply;
    }
    if !journal.config.producers.contains_key(&checkpoint.producer) {
        return error(StatusCode::BAD_REQUEST, "unknown producer");
    }
    if journal.checkpoints.contains_key(&checkpoint.producer) {
        return error(StatusCode::CONFLICT, "checkpoint is immutable");
    }
    if let Err(e) = journal.append(json!({"kind": "checkpoint", "checkpoint": checkpoint})) {
        return error(StatusCode::INSUFFICIENT_STORAGE, e);
    }
    journal
        .checkpoints
        .insert(checkpoint.producer.clone(), checkpoint);
    (StatusCode::OK, Json(json!({})))
}

async fn faults(State(observer): State<Observer>, Json(faults): Json<Faults>) -> Reply {
    if faults.delay_ms > 30_000 {
        return error(StatusCode::BAD_REQUEST, "delayMs must be at most 30000");
    }
    let mut journal = observer.0.lock().unwrap();
    if let Err(reply) = journal.mutable() {
        return reply;
    }
    if let Err(e) = journal.append(json!({"kind": "faults", "faults": faults})) {
        return error(StatusCode::INSUFFICIENT_STORAGE, e);
    }
    journal.faults = faults;
    (StatusCode::OK, Json(json!({})))
}

/// Seal permanently, even on failure; callers can export diagnostic evidence.
async fn seal(State(observer): State<Observer>) -> Reply {
    let mut journal = observer.0.lock().unwrap();
    if let Some(bundle) = &journal.sealed {
        return (StatusCode::OK, Json(bundle.clone()));
    }
    let mut reasons = vec![];
    if journal.lost > 0 {
        reasons.push("receiver lost observations".to_string());
    }
    if let Some(failure) = &journal.failure {
        reasons.push(format!("journal failure: {failure}"));
    }
    for id in journal.config.producers.keys() {
        match journal.checkpoints.get(id) {
            None => reasons.push(format!("{id}: no flush checkpoint")),
            Some(checkpoint) => {
                if !checkpoint.flushed || checkpoint.dropped > 0 {
                    reasons.push(format!(
                        "{id}: flush failed or producer dropped observations"
                    ));
                }
                if checkpoint.counts != journal.counts.get(id).copied().unwrap_or_default() {
                    reasons.push(format!("{id}: expected and retained counts differ"));
                }
            }
        }
    }
    let seal_row = json!({"kind": "seal", "status": if reasons.is_empty() { "complete" } else { "incomplete" }, "reasons": reasons});
    if let Err(e) = journal.append(seal_row) {
        reasons.push(format!("seal could not be persisted: {e}"));
    }
    let bytes = journal
        .rows
        .iter()
        .map(|row| serde_json::to_string(row).unwrap() + "\n")
        .collect::<String>();
    let bundle = json!({
        "format": "lawtwin-observation", "version": 1,
        "runId": journal.config.run_id, "topologyDigest": journal.config.topology_digest,
        "status": if reasons.is_empty() { "complete" } else { "incomplete" }, "reasons": reasons,
        "producers": journal.config.producers, "counts": journal.counts, "checkpoints": journal.checkpoints,
        "executions": journal.executions,
        "lost": journal.lost, "journalDigest": format!("sha256:{:x}", Sha256::digest(bytes.as_bytes())),
        "journal": journal.rows,
        "trust": "producer-declared flush/counts; receiver persistence; no proof of business outcomes or cloud logging fidelity"
    });
    journal.sealed = Some(bundle.clone());
    (StatusCode::OK, Json(bundle))
}

async fn bundle(State(observer): State<Observer>) -> Reply {
    let journal = observer.0.lock().unwrap();
    match &journal.sealed {
        Some(bundle) => (StatusCode::OK, Json(bundle.clone())),
        None => error(
            StatusCode::CONFLICT,
            "seal before exporting; a running collection is not complete evidence",
        ),
    }
}
