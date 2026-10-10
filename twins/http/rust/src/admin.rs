//! The control plane's admin API: JSON over HTTP on its own port, as
//! `docs/reference/control-plane.openapi.yaml` describes it, the same for
//! every twin. ref:DEC-control-plane-api

use crate::clock::parse_duration;
use crate::force::{self, Refused};
use crate::history::{self, op_entry};
use crate::laws::{check_history, check_state};
use crate::twin::Twin;
use crate::V;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
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

fn problem(status: StatusCode, message: String) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn clock_json(twin: &Twin) -> Value {
    let core = twin.lock();
    json!({ "mode": core.clock.mode(), "nowMs": core.clock.now_ms() })
}

async fn info(State(twin): Shared) -> Json<Value> {
    let core = twin.lock();
    let host = twin.config.advertised.clone().unwrap_or_else(|| twin.config.listen.replace("0.0.0.0", "localhost"));
    Json(json!({
        "twin": "http",
        "version": env!("CARGO_PKG_VERSION"),
        "contract": {
            "package": core.contract.package,
            "version": core.contract.version,
            "ability": core.map.operations.iter().fold(Vec::<String>::new(), |mut a, o| { if !a.contains(&o.ability) { a.push(o.ability.clone()); } a }).join(", "),
            "handler": "reference",
        },
        "seed": core.seed,
        "clock": { "mode": core.clock.mode(), "nowMs": core.clock.now_ms() },
        "bootstrap": format!("http://{host}"),
        "historyLength": core.history.len(),
        "config": {
            "contract": core.contract.name,
            "contracts": crate::contracts::registry().iter().map(|c| c.name).collect::<Vec<_>>(),
            "source": core.map.source.title,
            "mutant": twin.config.mutant,
        },
    }))
}

/// The model's state in the wire map's terms: each resource's records by
/// key, and each counter.
pub fn state_json(twin: &Twin) -> Value {
    let core = twin.lock();
    let state = core.model.state();
    let mut resources = Map::new();
    let mut counters = Map::new();
    if let V::Data { fields, .. } = &state {
        for (layout, value) in core.map.state.iter().zip(fields) {
            if let Some(r) = core.map.resources.iter().find(|r| r.state_field == layout.field) {
                let key_ty = lawtwin_wire::WireMap::ty(&r.key_type.ty);
                let records: Vec<Value> = match value {
                    V::Map(entries) => entries
                        .iter()
                        .map(|(k, rec)| {
                            json!({
                                "key": core.map.encode(&key_ty, k),
                                "record": core.map.encode(&lawtwin_wire::Ty::Named(r.record.clone()), rec),
                            })
                        })
                        .collect(),
                    _ => vec![],
                };
                resources.insert(r.record.clone(), Value::Array(records));
            } else {
                counters.insert(layout.field.clone(), core.map.encode_untyped(value));
            }
        }
    }
    json!({
        "twin": "http",
        "contract": core.contract.package,
        "seed": core.seed,
        "clock": { "mode": core.clock.mode(), "nowMs": core.clock.now_ms() },
        "faults": core.faults.iter().map(|f| f.to_json()).collect::<Vec<_>>(),
        "historyLength": core.history.len(),
        "resources": resources,
        "counters": counters,
    })
}

async fn inspect(State(twin): Shared) -> Json<Value> {
    Json(state_json(&twin))
}

async fn force_state(State(twin): Shared, body: Bytes) -> Response {
    let doc: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return problem(StatusCode::BAD_REQUEST, format!("not a forced state: {e}")),
    };
    let mut core = twin.lock();
    let reset = doc.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let mut copy = if reset { (core.contract.start)() } else { core.model.boxed() };
    match force::reach(&core.map, copy.as_mut(), &doc) {
        Ok(done) => {
            if reset {
                let seed = core.seed;
                core.reset(seed);
                core.control("force", "reset", json!({ "seed": seed }));
            }
            core.model = copy;
            let n = done.len();
            let at = core.clock.now_ms();
            for (op, args, answer) in done {
                core.record(op_entry(0, at, "force", &op, &args, answer));
            }
            (StatusCode::OK, Json(json!({ "forced": true, "operations": n }))).into_response()
        }
        Err(Refused::Malformed(m)) => problem(StatusCode::BAD_REQUEST, m),
        Err(Refused::Law { law, reason }) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "forced": false, "law": law, "reason": reason }))).into_response(),
    }
}

async fn list_faults(State(twin): Shared) -> Json<Value> {
    let core = twin.lock();
    Json(json!({
        "declared": core.twin_file.faults,
        "active": core.faults.iter().map(|f| f.to_json()).collect::<Vec<_>>(),
        "force": core.twin_file.force,
    }))
}

async fn raise(State(twin): Shared, Path(name): Path<String>, body: Bytes) -> Response {
    let params: Map<String, Value> = if body.iter().all(u8::is_ascii_whitespace) {
        Map::new()
    } else {
        match serde_json::from_slice(&body) {
            Ok(Value::Object(m)) => m,
            _ => return problem(StatusCode::BAD_REQUEST, "a fault's parameters are a JSON object".into()),
        }
    };
    let mut core = twin.lock();
    core.expire();
    match core.raise(&name, &params) {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => problem(StatusCode::BAD_REQUEST, e),
    }
}

async fn clear_all(State(twin): Shared) -> Json<Value> {
    Json(json!({ "cleared": twin.lock().clear(None) }))
}

async fn clear_one(State(twin): Shared, Path(name): Path<String>) -> Response {
    let mut core = twin.lock();
    if core.twin_file.decl(&name).is_none() {
        return problem(StatusCode::NOT_FOUND, format!("{name} is not a fault the twin file declares"));
    }
    Json(json!({ "cleared": core.clear(Some(&name)) })).into_response()
}

async fn clock(State(twin): Shared) -> Json<Value> {
    Json(clock_json(&twin))
}

async fn advance(State(twin): Shared, body: Bytes) -> Response {
    let by = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v.get("by").cloned());
    let ms = match by {
        Some(Value::Number(n)) if n.as_i64().is_some_and(|n| n >= 0) => n.as_i64().unwrap(),
        Some(Value::String(s)) => match parse_duration(&s) {
            Ok(ms) => ms,
            Err(e) => return problem(StatusCode::BAD_REQUEST, e),
        },
        _ => return problem(StatusCode::BAD_REQUEST, "advance takes {\"by\": \"5s\"} or milliseconds".into()),
    };
    let mut core = twin.lock();
    core.clock.advance(ms);
    let now = core.clock.now_ms();
    core.control("control", "clock advanced", json!({ "byMs": ms, "nowMs": now }));
    core.expire();
    Json(json!({ "nowMs": now })).into_response()
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
    let entries: Vec<&Value> = core
        .history
        .iter()
        .filter(|e| e["seq"].as_u64().unwrap_or(0) > since)
        .take(q.limit.unwrap_or(usize::MAX))
        .collect();
    if q.format.as_deref() == Some("jsonl") {
        let mut text = String::new();
        for e in entries {
            text.push_str(&serde_json::to_string(e).unwrap_or_default());
            text.push('\n');
        }
        return ([(header::CONTENT_TYPE, "application/x-ndjson")], text).into_response();
    }
    Json(Value::Array(entries.into_iter().cloned().collect())).into_response()
}

async fn replay_history(State(twin): Shared, body: Bytes) -> Response {
    let (map, start, own) = {
        let core = twin.lock();
        (core.map.clone(), core.contract.start, core.history.clone())
    };
    let entries = if body.iter().all(u8::is_ascii_whitespace) {
        own
    } else {
        match history::parse(&String::from_utf8_lossy(&body)) {
            Ok(e) => e,
            Err(e) => return problem(StatusCode::BAD_REQUEST, e),
        }
    };
    match history::replay(&map, start, &entries) {
        Ok(r) => Json(json!({
            "operations": r.operations,
            "divergences": r.divergences,
            "violations": { "state": check_state(&map, &r.model.state()), "history": check_history(&map, &entries) },
        }))
        .into_response(),
        Err(e) => problem(StatusCode::BAD_REQUEST, e),
    }
}

async fn laws(State(twin): Shared) -> Json<Value> {
    let core = twin.lock();
    let state = check_state(&core.map, &core.model.state());
    let hist = check_history(&core.map, &core.history);
    Json(json!({ "holds": state.is_empty() && hist.is_empty(), "state": state, "history": hist }))
}

async fn seed(State(twin): Shared) -> Json<Value> {
    Json(json!({ "seed": twin.lock().seed }))
}

async fn reseed(State(twin): Shared, body: Bytes) -> Response {
    let Some(seed) = serde_json::from_slice::<Value>(&body).ok().and_then(|v| v["seed"].as_u64()) else {
        return problem(StatusCode::BAD_REQUEST, "reseed takes {\"seed\": <n>}".into());
    };
    twin.lock().reset(seed);
    Json(json!({ "seed": seed, "reset": true })).into_response()
}

async fn reset(State(twin): Shared) -> Json<Value> {
    let mut core = twin.lock();
    let seed = core.seed;
    core.reset(seed);
    Json(json!({ "seed": seed, "reset": true }))
}
