//! The twin file of an imported contract, `lawtwin.json`: the faults the
//! HTTP twin can show for it, what each does to each of the contract's laws,
//! and what forcing a state refuses (ref:DEC-twin-file). The importer writes
//! it once; it is the author's from then on, like the hand-written units.

use crate::api::Api;
use serde_json::{Map, Value, json};

fn param(name: &str, ty: &str, required: bool, default: Value, summary: &str) -> Value {
    let mut p = json!({ "name": name, "type": ty, "required": required });
    if !default.is_null() {
        p["default"] = default;
    }
    p["summary"] = json!(summary);
    p
}

fn common(applied: bool) -> Vec<Value> {
    let mut out = vec![
        param("operation", "operation", false, Value::Null, "Only requests for this operation of the contract, by its name; without it, every operation."),
        param("count", "int32", false, Value::Null, "How many requests it answers before it ends by itself; without it, until it is cleared."),
        param("probability", "int32", false, json!(100), "The chance, in percent, that a matching request is affected, drawn from the seeded generator."),
    ];
    if applied {
        out.push(param(
            "applied",
            "boolean",
            false,
            json!(false),
            "Whether the request reaches the model first: true runs the operation and then answers as the fault says, so the client does not learn that it took effect.",
        ));
    }
    out.push(param("for", "duration", false, Value::Null, "How long, on the twin clock; without it, until it is cleared or used up."));
    out
}

/// What a fault that answers in the model's place does to each law, by the law's kind.
fn refusing_effect(kind: &str, what: &str) -> String {
    match kind {
        "createdReadsBack" | "replacedReadsBack" => format!(
            "Holds for every read that reaches the model. A read {what} is not the record, so a client sees the law broken for it; a write {what} before it reaches the model wrote nothing, and claims nothing. With applied=true the write took effect: the record reads back once the fault lets reads through."
        ),
        "deletedIsNotFound" => format!(
            "Holds when the delete reaches the model: with applied=true the record is gone though the client was told otherwise. A delete {what} before it reaches the model left the record, so a read still finds it; a read {what} is not NotFound."
        ),
        "listHoldsCreated" => format!("Holds for every list that reaches the model; a list {what} holds nothing."),
        _ => format!(
            "Holds for every update and read that reach the model; an update {what} before it reaches the model changed nothing, and a read {what} gives no record to compare."
        ),
    }
}

/// The twin file for a contract.
pub fn twin_file(package: &str, version: &str, api: &Api, laws: &[(String, &'static str, String)]) -> String {
    let abilities = api.abilities.join(", ");
    let effects = |what: &str| -> Value {
        let mut m = Map::new();
        for (_, kind, title) in laws {
            m.insert(title.clone(), json!(refusing_effect(kind, what)));
        }
        Value::Object(m)
    };
    let mut refusals = Map::new();
    for (record, kind, title) in laws {
        if *kind == "createdReadsBack" {
            refusals.insert(
                title.clone(),
                json!(format!(
                    "a {record} the contract's create, then its update or replace, would not read back as given: a field the service sets itself, a key the service assigns differently, a key already held, or a value that breaks the document's constraints"
                )),
            );
        }
    }
    let server_error_params: Vec<Value> = std::iter::once(param("status", "status", false, json!(500), "The status, 500 to 599."))
        .chain(common(true))
        .collect();
    let timeout_params: Vec<Value> = std::iter::once(param(
        "hold",
        "duration",
        false,
        json!("30s"),
        "How long, in real time, the connection is held before it is closed.",
    ))
    .chain(common(true))
    .collect();
    let file = json!({
        "lawtwin": 1,
        "twin": "http",
        "contract": { "package": package, "version": version, "ability": abilities, "handler": "reference" },
        "about": "The HTTP twin's control plane for this contract: the faults it can be told to show, and what each does to the contract's laws. The contract itself (src/) never mentions them. A law not named under a fault holds while the fault is active, for every request the model answers. Written once by `lawtwin import openapi`; yours from then on.",
        "faults": [
            {
                "name": "latency",
                "summary": "Responses are slow: each matching response is held back before it is sent.",
                "params": [
                    param("delay", "duration", true, Value::Null, "How long, in real time, each response is held back."),
                    param("jitter", "duration", false, json!("0ms"), "Up to this much more, drawn from the seeded generator."),
                    param("operation", "operation", false, Value::Null, "Only requests for this operation; without it, every operation."),
                    param("for", "duration", false, Value::Null, "How long, on the twin clock; without it, until it is cleared."),
                ],
                "changes": "Only when an answer is sent. Every request reaches the model as usual.",
                "laws": {}
            },
            {
                "name": "server-error",
                "summary": "The service fails: matching requests are answered with a 5xx status and the body the document gives that status, if any.",
                "params": server_error_params,
                "changes": "With applied=false the request never reaches the model. With applied=true the model runs it, and the client is told it failed.",
                "laws": effects("answered with a 5xx status")
            },
            {
                "name": "rate-limit",
                "summary": "The service limits its rate: past `limit` requests in a window of the twin clock, requests are answered 429 Too Many Requests with Retry-After, the seconds left in the window.",
                "params": [
                    param("limit", "int32", false, json!(0), "How many requests a window lets through; 0 refuses every one."),
                    param("window", "duration", false, json!("1s"), "The window, on the twin clock; it starts when the fault is raised, and the next one when it ends."),
                    param("operation", "operation", false, Value::Null, "Only requests for this operation; without it, every operation, counted together."),
                    param("for", "duration", false, Value::Null, "How long, on the twin clock; without it, until it is cleared."),
                ],
                "changes": "A refused request never reaches the model. On the frozen clock a window ends only when a test advances the clock past it.",
                "laws": effects("refused with 429")
            },
            {
                "name": "timeout",
                "summary": "The service hangs: matching requests get no response; the connection is held open, then closed with nothing sent.",
                "params": timeout_params,
                "changes": "With applied=false the request never reaches the model. With applied=true the model runs it, and the client never hears so.",
                "laws": effects("that times out")
            },
            {
                "name": "connection-reset",
                "summary": "The connection breaks: matching requests are answered by resetting the connection, with nothing sent.",
                "params": common(true),
                "changes": "With applied=false the request never reaches the model. With applied=true the model runs it, and the connection is reset before the answer.",
                "laws": effects("whose connection is reset")
            },
            {
                "name": "slow-body",
                "summary": "Bodies trickle: each matching response's body is sent a few bytes at a time, with a pause between.",
                "params": [
                    param("chunk", "int32", false, json!(16), "How many bytes each write sends."),
                    param("delay", "duration", false, json!("100ms"), "How long, in real time, between writes."),
                    param("operation", "operation", false, Value::Null, "Only requests for this operation; without it, every operation."),
                    param("for", "duration", false, Value::Null, "How long, on the twin clock; without it, until it is cleared."),
                ],
                "changes": "Only how the answer is sent: its headers at once, its body slowly. Every request reaches the model as usual.",
                "laws": {}
            }
        ],
        "force": {
            "about": "A forced state names records of each resource. Each is reached through the contract's own operations: a create, then an update or replace for what the create does not set; it must then read back as given. A state the operations do not reach is refused with the law that forbids it, and nothing is applied.",
            "refusals": refusals
        }
    });
    serde_json::to_string_pretty(&file).unwrap() + "\n"
}
