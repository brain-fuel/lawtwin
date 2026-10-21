//! The declared fault set, read from the contract's twin file
//! (`contracts/kafka/lawtwin.json`), and the faults active in a twin.
//!
//! A fault is raised by name with parameters; both are checked against the
//! declaration, so a test can only ask for what the twin file says, and the
//! file says what each fault does to each law. ref:DEC-twin-file

use crate::clock::parse_duration;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The twin file, as written next to the contract.
pub const TWIN_FILE: &str = include_str!("../../../../contracts/kafka/lawtwin.json");

/// A parameter a fault declares.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamDecl {
    /// Its name.
    pub name: String,
    /// Its type: duration, int32, topic, group or api.
    #[serde(rename = "type")]
    pub ty: String,
    /// Whether it must be given.
    pub required: bool,
    /// Its value when not given.
    #[serde(default)]
    pub default: Option<Value>,
    /// What it means.
    pub summary: String,
}

/// A fault the twin file declares.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaultDecl {
    /// Its name.
    pub name: String,
    /// What it does, in a sentence.
    pub summary: String,
    /// Its parameters.
    pub params: Vec<ParamDecl>,
    /// What changes while it is active.
    pub changes: String,
    /// Its effect on each law it touches; every other law holds.
    pub laws: std::collections::BTreeMap<String, String>,
}

/// The twin file's content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TwinFile {
    /// The declared faults.
    pub faults: Vec<FaultDecl>,
    /// What forcing a state refuses, by law.
    pub force: Value,
}

/// The twin file, parsed.
pub fn declared() -> TwinFile {
    serde_json::from_str(TWIN_FILE).expect("contracts/kafka/lawtwin.json is valid")
}

/// The Kafka request types a fault may name.
pub const APIS: &[&str] = &[
    "Produce",
    "Fetch",
    "ListOffsets",
    "Metadata",
    "OffsetCommit",
    "OffsetFetch",
    "FindCoordinator",
    "JoinGroup",
    "Heartbeat",
    "LeaveGroup",
    "SyncGroup",
    "DescribeGroups",
    "ListGroups",
    "ApiVersions",
    "CreateTopics",
    "InitProducerId",
    "AddPartitionsToTxn",
    "AddOffsetsToTxn",
    "EndTxn",
    "TxnOffsetCommit",
    "DescribeConfigs",
    "CreatePartitions",
];

/// A fault, as active in the twin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "name",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Fault {
    /// The broker cannot be reached.
    BrokerUnavailable,
    /// The next request for each matching partition is refused NOT_LEADER_OR_FOLLOWER.
    LeaderChange {
        /// Only this topic.
        topic: Option<String>,
        /// Only this partition.
        partition: Option<i32>,
    },
    /// Fetches return at most `records` records per partition, after `delay_ms`.
    ConsumerLag {
        /// The most records per partition and fetch.
        records: i32,
        /// The delay before each fetch response, in milliseconds.
        delay_ms: i64,
        /// Only this topic.
        topic: Option<String>,
    },
    /// The next `count` produce requests are written, then answered REQUEST_TIMED_OUT.
    DuplicateDelivery {
        /// How many acknowledgements are still to be lost.
        count: i32,
        /// Only this topic.
        topic: Option<String>,
    },
    /// Groups rebalance `rounds` more times.
    RebalanceStorm {
        /// Rounds still to come.
        rounds: i32,
        /// Only this group.
        group: Option<String>,
    },
    /// Responses are held back by `delay_ms`.
    RequestLatency {
        /// The delay, in milliseconds.
        delay_ms: i64,
        /// Only this request type.
        api: Option<String>,
    },
}

/// A fault the twin is showing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Active {
    /// Its id, from 1.
    pub id: u64,
    /// The fault and its parameters.
    #[serde(flatten)]
    pub fault: Fault,
    /// When it ends, on the twin clock, if it was raised for a time.
    #[serde(rename = "untilMs", skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<i64>,
}

fn get<'a>(params: &'a Map<String, Value>, name: &str) -> Option<&'a Value> {
    params.get(name).filter(|v| !v.is_null())
}

fn int(params: &Map<String, Value>, name: &str, default: i64) -> Result<i64, String> {
    match get(params, name) {
        None => Ok(default),
        Some(Value::Number(n)) => n
            .as_i64()
            .ok_or_else(|| format!("{name} must be a whole number")),
        Some(Value::String(s)) => s
            .parse()
            .map_err(|_| format!("{name} must be a whole number, not {s}")),
        Some(v) => Err(format!("{name} must be a whole number, not {v}")),
    }
}

fn text(params: &Map<String, Value>, name: &str) -> Result<Option<String>, String> {
    match get(params, name) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Number(n)) => Ok(Some(n.to_string())),
        Some(v) => Err(format!("{name} must be text, not {v}")),
    }
}

fn duration(params: &Map<String, Value>, name: &str) -> Result<Option<i64>, String> {
    match get(params, name) {
        None => Ok(None),
        Some(Value::String(s)) => parse_duration(s).map(Some),
        Some(Value::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{name} must be a duration")),
        Some(v) => Err(format!(
            "{name} must be a duration such as 500ms or 5s, not {v}"
        )),
    }
}

/// Checks a request for a fault against the twin file and gives the fault,
/// and how long it lasts on the twin clock.
pub fn parse(name: &str, params: &Map<String, Value>) -> Result<(Fault, Option<i64>), String> {
    let file = declared();
    let decl = file.faults.iter().find(|f| f.name == name).ok_or_else(|| {
        format!(
            "{name} is not a declared fault; the twin file declares {}",
            file.faults
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    for key in params.keys() {
        if !decl.params.iter().any(|p| &p.name == key) {
            return Err(format!(
                "{name} has no parameter {key}; it takes {}",
                decl.params
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    for p in &decl.params {
        if p.required && get(params, &p.name).is_none() {
            return Err(format!("{name} needs {}", p.name));
        }
    }
    let lasting = duration(params, "for")?;
    let fault = match name {
        "broker-unavailable" => Fault::BrokerUnavailable,
        "leader-change" => Fault::LeaderChange {
            topic: text(params, "topic")?,
            partition: get(params, "partition")
                .map(|_| int(params, "partition", 0).map(|p| p as i32))
                .transpose()?,
        },
        "consumer-lag" => {
            let records = int(params, "records", 1)?;
            if !(0..=i32::MAX as i64).contains(&records) {
                return Err("records must be 0 or more".into());
            }
            Fault::ConsumerLag {
                records: records as i32,
                delay_ms: duration(params, "delay")?.unwrap_or(0),
                topic: text(params, "topic")?,
            }
        }
        "duplicate-delivery" => {
            let count = int(params, "count", 1)?;
            if count < 1 {
                return Err("count must be 1 or more".into());
            }
            Fault::DuplicateDelivery {
                count: count.min(i32::MAX as i64) as i32,
                topic: text(params, "topic")?,
            }
        }
        "rebalance-storm" => {
            let rounds = int(params, "rounds", 3)?;
            if rounds < 1 {
                return Err("rounds must be 1 or more".into());
            }
            Fault::RebalanceStorm {
                rounds: rounds.min(1000) as i32,
                group: text(params, "group")?,
            }
        }
        "request-latency" => {
            let api = text(params, "api")?;
            if let Some(a) = &api {
                if !APIS.contains(&a.as_str()) {
                    return Err(format!("{a} is not a request type the twin answers"));
                }
            }
            Fault::RequestLatency {
                delay_ms: duration(params, "delay")?.unwrap_or(0),
                api,
            }
        }
        _ => {
            return Err(format!(
                "{name} is declared but this twin does not implement it"
            ));
        }
    };
    Ok((fault, lasting))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn params(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn every_declared_fault_is_implemented() {
        for f in declared().faults {
            let mut p = Map::new();
            for d in &f.params {
                if d.required {
                    p.insert(d.name.clone(), json!("1s"));
                }
            }
            parse(&f.name, &p).unwrap_or_else(|e| panic!("{}: {e}", f.name));
        }
    }

    #[test]
    fn every_law_the_twin_file_names_is_a_law_of_the_contract() {
        let contract = include_str!("../../../../contracts/kafka/src/contract.lawspec");
        let file = declared();
        let mut names: Vec<String> = file
            .faults
            .iter()
            .flat_map(|f| f.laws.keys().cloned())
            .collect();
        names.extend(file.force["refusals"].as_object().unwrap().keys().cloned());
        for law in names {
            assert!(
                contract.contains(&format!("law \"|{law}|")),
                "{law} is not a law of the contract"
            );
        }
    }

    #[test]
    fn undeclared_faults_and_parameters_are_refused() {
        assert!(parse("disk-full", &Map::new()).is_err());
        assert!(parse("consumer-lag", &params(json!({"speed": 3}))).is_err());
        assert!(parse("request-latency", &Map::new()).is_err());
        assert!(
            parse(
                "request-latency",
                &params(json!({"delay": "1s", "api": "Gossip"}))
            )
            .is_err()
        );
        let (f, until) =
            parse("consumer-lag", &params(json!({"records": 2, "for": "5s"}))).unwrap();
        assert_eq!(
            f,
            Fault::ConsumerLag {
                records: 2,
                delay_ms: 0,
                topic: None
            }
        );
        assert_eq!(until, Some(5000));
    }
}
