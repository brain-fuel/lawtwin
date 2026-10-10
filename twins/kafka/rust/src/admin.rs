//! The control plane's admin API: JSON over HTTP on its own port, as
//! `docs/reference/control-plane.openapi.yaml` describes it.

use crate::clock::parse_duration;
use crate::faults;
use crate::force::{self, ForceDoc};
use crate::history::{Entry, Event, replay};
use crate::laws::{check_history, check_state};
use crate::model::data;
use crate::twin::{Twin, fault_name};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::sync::Arc;

type Shared = State<Arc<Twin>>;

/// The admin API's routes.
pub fn router(twin: Arc<Twin>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/twin", get(info))
        .route("/v1/state", get(inspect).post(force_state))
        .route("/v1/faults", get(list_faults).delete(clear_all))
        .route("/v1/faults/{name}", post(raise).delete(clear_one))
        .route("/v1/clock", get(clock))
        .route("/v1/clock/advance", post(advance))
        .route("/v1/history", get(history))
        .route("/v1/history/replay", post(replay_history))
        .route("/v1/laws", get(laws))
        .route("/v1/seed", get(seed).post(reseed))
        .route("/v1/reset", delete(reset))
        .with_state(twin)
}

fn problem(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

async fn info(State(twin): Shared) -> Json<Value> {
    let core = twin.lock();
    let (host, port) = twin.advertised();
    Json(json!({
        "twin": "kafka",
        "version": env!("CARGO_PKG_VERSION"),
        "contract": { "package": "lawtwin.kafka", "version": "0.1.0", "ability": "Broker", "handler": "cluster" },
        "seed": core.seed,
        "clock": { "mode": core.clock.mode(), "nowMs": core.clock.now_ms() },
        "bootstrap": format!("{host}:{port}"),
        "clusterId": core.cluster_id,
        "historyLength": core.history.len(),
        "config": twin.config,
    }))
}

fn tp_json(tp: &data::TopicPartition) -> Value {
    json!({ "topic": tp.topic, "partition": tp.partition })
}

fn record_json(r: &data::Record) -> Value {
    serde_json::to_value(crate::history::record_of(r)).unwrap_or_default()
}

/// The twin's state, as `inspect` shows it.
pub fn inspect_state(twin: &Twin, with_records: bool) -> Value {
    let core = twin.lock();
    let b = core.model.snapshot();
    let topics: Vec<Value> = b
        .topics
        .iter()
        .map(|(name, n)| {
            let partitions: Vec<Value> = (0..*n)
                .map(|p| {
                    let tp = data::TopicPartition { topic: name.clone(), partition: p };
                    let log = b.logs.get(&tp);
                    let end = log.map(|l| l.logEndOffset).unwrap_or(0);
                    let lso = log.map(|l| l.openTxns.values().copied().fold(l.logEndOffset, i64::min)).unwrap_or(0);
                    let mut v = json!({
                        "partition": p,
                        "highWatermark": end,
                        "lastStableOffset": lso,
                        "leaderEpoch": core.leader_epoch(name, p),
                        "openTransactions": log.map(|l| l.openTxns.iter().map(|(w, f)| json!({ "producerId": w, "firstOffset": f })).collect::<Vec<_>>()).unwrap_or_default(),
                        "abortedTransactions": log.map(|l| l.abortedTxns.iter().map(|a| json!({ "producerId": a.abortedWriter, "firstOffset": a.firstOffset, "lastOffset": a.lastOffset })).collect::<Vec<_>>()).unwrap_or_default(),
                    });
                    if with_records {
                        let entries: Vec<Value> = log
                            .map(|l| {
                                l.entries
                                    .iter()
                                    .rev()
                                    .map(|e| match e {
                                        data::LogEntry::DataEntry { entryOffset, entryRecord, writer, transactional } => {
                                            let mut r = record_json(entryRecord);
                                            r["offset"] = json!(entryOffset);
                                            r["producerId"] = json!(writer);
                                            r["transactional"] = json!(transactional);
                                            r
                                        }
                                        data::LogEntry::ControlEntry { markerOffset, markerWriter, commits } => json!({
                                            "offset": markerOffset, "marker": if *commits { "commit" } else { "abort" }, "producerId": markerWriter,
                                        }),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        v["entries"] = Value::Array(entries);
                    }
                    v
                })
                .collect();
            json!({ "name": name, "partitions": partitions })
        })
        .collect();
    let producers: Vec<Value> = b
        .producers
        .iter()
        .map(|(pid, p)| {
            let txn = match &p.txn {
                data::TxnState::NoTxn => Value::Null,
                data::TxnState::Ongoing { txnPartitions, txnOffsets } => json!({
                    "partitions": txnPartitions.iter().map(tp_json).collect::<Vec<_>>(),
                    "offsets": txnOffsets.iter().map(|o| json!({ "group": o.pendingGroup, "topic": o.pendingEntry.topicPartition.topic, "partition": o.pendingEntry.topicPartition.partition, "offset": o.pendingEntry.offset })).collect::<Vec<_>>(),
                }),
            };
            json!({ "producerId": pid, "epoch": p.epoch, "transactionalId": p.transactionalId, "transaction": txn })
        })
        .collect();
    let groups: Vec<Value> = b
        .groups
        .iter()
        .map(|(g, group)| {
            let wire = core.groups.get(g);
            json!({
                "group": g,
                "generation": group.groupGeneration,
                "rebalancing": wire.is_some_and(|w| w.round.is_some()),
                "protocol": wire.and_then(|w| w.protocol_name.clone()),
                "members": group.subscriptions.iter().map(|(m, topics)| json!({
                    "member": m,
                    "topics": topics,
                    "assigned": group.assignment.get(m).map(|a| a.iter().map(tp_json).collect::<Vec<_>>()).unwrap_or_default(),
                    "clientId": wire.and_then(|w| w.members.get(m)).map(|w| w.client_id.clone()),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let committed: Vec<Value> = b
        .committed
        .iter()
        .map(|(k, o)| json!({ "group": k.groupId, "topic": k.groupPartition.topic, "partition": k.groupPartition.partition, "offset": o }))
        .collect();
    json!({
        "twin": "kafka",
        "seed": core.seed,
        "clock": { "mode": core.clock.mode(), "nowMs": core.clock.now_ms() },
        "topics": topics,
        "producers": producers,
        "transactionalIds": b.transactionalIds,
        "nextProducerId": b.nextProducerId,
        "groups": groups,
        "committed": committed,
        "faults": core.faults,
        "historyLength": core.history.len(),
    })
}

#[derive(Deserialize)]
struct InspectQuery {
    #[serde(default)]
    records: bool,
}

async fn inspect(State(twin): Shared, Query(q): Query<InspectQuery>) -> Json<Value> {
    let twin2 = twin.clone();
    let v = tokio::task::spawn_blocking(move || inspect_state(&twin2, q.records)).await.unwrap_or_default();
    Json(v)
}

async fn force_state(State(twin): Shared, body: String) -> Response {
    let doc: ForceDoc = match serde_json::from_str(&body) {
        Ok(d) => d,
        Err(e) => return problem(StatusCode::BAD_REQUEST, json!({ "error": format!("not a forced state: {e}") })),
    };
    let result = {
        let mut core = twin.lock();
        force::force(&mut core, &doc)
    };
    twin.data.notify_waiters();
    match result {
        Ok(f) => problem(StatusCode::OK, json!({ "forced": true, "operations": f.operations })),
        Err(r) => problem(StatusCode::UNPROCESSABLE_ENTITY, json!({ "forced": false, "law": r.law, "reason": r.reason })),
    }
}

async fn list_faults(State(twin): Shared) -> Json<Value> {
    let core = twin.lock();
    let file = faults::declared();
    Json(json!({ "declared": file.faults, "active": core.faults, "force": file.force }))
}

async fn raise(State(twin): Shared, Path(name): Path<String>, body: String) -> Response {
    let params: Map<String, Value> = if body.trim().is_empty() {
        Map::new()
    } else {
        match serde_json::from_str(&body) {
            Ok(Value::Object(m)) => m,
            _ => return problem(StatusCode::BAD_REQUEST, json!({ "error": "the parameters are a JSON object" })),
        }
    };
    match faults::parse(&name, &params) {
        Err(e) => problem(StatusCode::BAD_REQUEST, json!({ "error": e })),
        Ok((fault, lasting)) => {
            let active = twin.lock().raise(fault, lasting);
            problem(StatusCode::CREATED, serde_json::to_value(active).unwrap_or_default())
        }
    }
}

async fn clear_one(State(twin): Shared, Path(name): Path<String>) -> Response {
    if !faults::declared().faults.iter().any(|f| f.name == name) {
        return problem(StatusCode::NOT_FOUND, json!({ "error": format!("{name} is not a declared fault") }));
    }
    let n = twin.lock().clear(Some(&name));
    problem(StatusCode::OK, json!({ "cleared": n }))
}

async fn clear_all(State(twin): Shared) -> Json<Value> {
    let n = twin.lock().clear(None);
    Json(json!({ "cleared": n }))
}

async fn clock(State(twin): Shared) -> Json<Value> {
    let core = twin.lock();
    Json(json!({ "mode": core.clock.mode(), "nowMs": core.clock.now_ms() }))
}

#[derive(Deserialize)]
struct Advance {
    by: Value,
}

async fn advance(State(twin): Shared, Json(a): Json<Advance>) -> Response {
    let ms = match &a.by {
        Value::String(s) => parse_duration(s),
        Value::Number(n) => n.as_i64().filter(|n| *n >= 0).ok_or_else(|| "by must be a duration".to_string()),
        _ => Err("by must be a duration such as 5s".to_string()),
    };
    let ms = match ms {
        Ok(ms) => ms,
        Err(e) => return problem(StatusCode::BAD_REQUEST, json!({ "error": e })),
    };
    let now = {
        let mut core = twin.lock();
        core.clock.advance(ms);
        let now = core.clock.now_ms();
        core.control(crate::history::Source::Coordinator, "clock advanced", json!({ "byMs": ms, "nowMs": now }));
        core.expire_faults();
        crate::group::expire_sessions(&mut core);
        now
    };
    problem(StatusCode::OK, json!({ "nowMs": now }))
}

#[derive(Deserialize)]
struct HistoryQuery {
    since: Option<u64>,
    limit: Option<usize>,
    format: Option<String>,
}

async fn history(State(twin): Shared, Query(q): Query<HistoryQuery>) -> Response {
    let core = twin.lock();
    let since = q.since.unwrap_or(0);
    let entries: Vec<&Entry> =
        core.history.iter().filter(|e| e.seq > since).take(q.limit.unwrap_or(usize::MAX)).collect();
    if q.format.as_deref() == Some("jsonl") {
        let mut text = String::new();
        for e in entries {
            text.push_str(&serde_json::to_string(e).unwrap_or_default());
            text.push('\n');
        }
        return ([("content-type", "application/x-ndjson")], text).into_response();
    }
    Json(json!(entries)).into_response()
}

/// Reads a history given as a JSON array or as JSON lines.
pub fn parse_history(text: &str) -> Result<Vec<Entry>, String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        return serde_json::from_str(trimmed).map_err(|e| e.to_string());
    }
    trimmed
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("line {}: {e}", i + 1)))
        .collect()
}

async fn replay_history(State(twin): Shared, body: String) -> Response {
    let entries = if body.trim().is_empty() {
        twin.lock().history.clone()
    } else {
        match parse_history(&body) {
            Ok(e) => e,
            Err(e) => return problem(StatusCode::BAD_REQUEST, json!({ "error": format!("not a history: {e}") })),
        }
    };
    let result = tokio::task::spawn_blocking(move || {
        let operations = entries.iter().filter(|e| matches!(e.event, Event::Call { .. })).count();
        let (model, divergences) = replay(&entries);
        let state = check_state(&model.snapshot());
        let history = check_history(&entries);
        json!({ "operations": operations, "divergences": divergences, "violations": { "state": state, "history": history } })
    })
    .await
    .unwrap_or_default();
    problem(StatusCode::OK, result)
}

async fn laws(State(twin): Shared) -> Json<Value> {
    let (snapshot, entries) = {
        let core = twin.lock();
        (core.model.snapshot(), core.history.clone())
    };
    let v = tokio::task::spawn_blocking(move || {
        let state = check_state(&snapshot);
        let history = check_history(&entries);
        json!({ "holds": state.is_empty() && history.is_empty(), "state": state, "history": history })
    })
    .await
    .unwrap_or_default();
    Json(v)
}

async fn seed(State(twin): Shared) -> Json<Value> {
    Json(json!({ "seed": twin.lock().seed }))
}

#[derive(Deserialize)]
struct Seed {
    seed: u64,
}

async fn reseed(State(twin): Shared, Json(s): Json<Seed>) -> Json<Value> {
    twin.reset(s.seed);
    Json(json!({ "seed": s.seed, "reset": true }))
}

async fn reset(State(twin): Shared) -> Json<Value> {
    let seed = twin.lock().seed;
    twin.reset(seed);
    Json(json!({ "seed": seed, "reset": true }))
}

/// The active faults' names, for logs.
pub fn active_names(twin: &Twin) -> Vec<&'static str> {
    twin.lock().faults.iter().map(|a| fault_name(&a.fault)).collect()
}
