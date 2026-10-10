//! The generated model, as the twin binds it: operations, refusals,
//! replay, forced states and the law checks.

use lawtwin_kafka::force::{self, ForceDoc};
use lawtwin_kafka::history::{self, Answer, Op, Outcome, Record, Octets, Tp, ProducerTag, Isolation};
use lawtwin_kafka::laws::{check_history, check_state};
use lawtwin_kafka::model::{ClusterModel, Failure, data::KafkaError};
use lawtwin_kafka::twin::{Config, Core};
use lawtwin_kafka::history::Source;

fn rec(v: &str) -> Record {
    Record { key: None, value: Some(Octets(v.as_bytes().to_vec())), headers: vec![] }
}

fn tp(t: &str, p: i32) -> Tp {
    Tp { topic: t.into(), partition: p }
}

#[test]
fn the_bound_model_answers_and_refuses_as_the_contract_says() {
    let mut m = ClusterModel::start();
    assert_eq!(history::apply(&mut m, &Op::CreateTopicsRequest { name: "orders".into(), partitions: 2 }), Ok(Outcome::Done));
    assert_eq!(
        history::apply(&mut m, &Op::CreateTopicsRequest { name: "orders".into(), partitions: 2 }),
        Err(Failure::Kafka(KafkaError::TopicAlreadyExists))
    );
    assert_eq!(
        history::apply(&mut m, &Op::CreateTopicsRequest { name: "..".into(), partitions: 2 }),
        Err(Failure::Kafka(KafkaError::InvalidTopic))
    );
    let base = history::apply(&mut m, &Op::ProduceRequest { tp: tp("orders", 0), tag: None, records: vec![rec("a"), rec("b")] });
    assert_eq!(base, Ok(Outcome::Number(0)));
    let base = history::apply(&mut m, &Op::ProduceRequest { tp: tp("orders", 0), tag: None, records: vec![rec("c")] });
    assert_eq!(base, Ok(Outcome::Number(2)));
    match history::apply(&mut m, &Op::FetchRequest { tp: tp("orders", 0), offset: 1, max_records: 10, isolation: Isolation::ReadUncommitted }) {
        Ok(Outcome::Fetch(f)) => {
            assert_eq!(f.records.iter().map(|r| r.offset).collect::<Vec<_>>(), vec![1, 2]);
            assert_eq!(f.high_watermark, 3);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        history::apply(&mut m, &Op::ProduceRequest { tp: tp("orders", 7), tag: None, records: vec![rec("x")] }),
        Err(Failure::Kafka(KafkaError::UnknownTopicOrPartition))
    );
    assert!(check_state(&m.snapshot()).is_empty());
}

#[test]
fn an_idempotent_retry_is_written_once_and_replay_agrees() {
    let mut core = Core::new(&Config::default());
    core.call(Source::Wire, Op::CreateTopicsRequest { name: "t".into(), partitions: 1 }).unwrap();
    let pid = match core.call(Source::Wire, Op::InitProducerIdRequest { transactional_id: None }).unwrap() {
        Outcome::Producer { producer_id, .. } => producer_id,
        o => panic!("{o:?}"),
    };
    let tag = ProducerTag { producer_id: pid, producer_epoch: 0, base_sequence: 0 };
    let send = Op::ProduceRequest { tp: tp("t", 0), tag: Some(tag), records: vec![rec("once")] };
    assert_eq!(core.call(Source::Wire, send.clone()), Ok(Outcome::Number(0)));
    assert_eq!(core.call(Source::Wire, send), Ok(Outcome::Number(0)));
    let gap = ProducerTag { producer_id: pid, producer_epoch: 0, base_sequence: 5 };
    assert_eq!(
        core.call(Source::Wire, Op::ProduceRequest { tp: tp("t", 0), tag: Some(gap), records: vec![rec("x")] }),
        Err(Failure::Kafka(KafkaError::OutOfOrderSequenceNumber))
    );
    let (_, divergences) = history::replay(&core.history);
    assert!(divergences.is_empty(), "{divergences:?}");
    assert!(check_history(&core.history).is_empty(), "{:?}", check_history(&core.history));
    // A tampered history is caught by both.
    let mut tampered = core.history.clone();
    if let history::Event::Call { answer, .. } = &mut tampered[3].event {
        *answer = Answer::Ok(Outcome::Number(1));
    }
    assert_eq!(history::replay(&tampered).1.len(), 1);
    assert!(!check_history(&tampered).is_empty());
}

#[test]
fn a_forced_state_is_reached_through_the_contract_or_refused_with_its_law() {
    let mut core = Core::new(&Config::default());
    let doc: ForceDoc = serde_json::from_value(serde_json::json!({
        "topics": [{ "name": "orders", "partitions": 3 }],
        "records": [{ "topic": "orders", "partition": 0, "records": [{ "key": "k", "value": "v1" }, { "value": "v2" }] },
                    { "topic": "orders", "partition": 1, "logEndOffset": 10 }],
        "transactions": [{ "transactionalId": "tx", "records": [{ "topic": "orders", "partition": 2, "value": "open" }], "end": "open" }],
        "groups": [{ "group": "g", "members": [{ "member": "m1", "topics": ["orders"] }, { "member": "m2", "topics": ["orders"] }] }],
        "committed": [{ "group": "g", "topic": "orders", "partition": 1, "offset": 4 }]
    })).unwrap();
    let forced = force::force(&mut core, &doc).expect("a reachable state");
    assert!(forced.operations > 5);
    let b = core.model.snapshot();
    assert_eq!(b.topics["orders"], 3);
    assert!(check_state(&b).is_empty());
    // Impossible states are refused with the law that forbids them, and change nothing.
    let before = core.history.len();
    for (doc, law) in [
        (serde_json::json!({ "topics": [{ "name": "orders", "partitions": 2 }] }), "a topic's partitions only grow"),
        (serde_json::json!({ "topics": [{ "name": "bad name!", "partitions": 1 }] }), "an invalid topic name is refused"),
        (serde_json::json!({ "topics": [{ "name": "big", "partitions": 5000 }] }), "a new topic has the partitions asked for"),
        (serde_json::json!({ "records": [{ "topic": "orders", "partition": 1, "logEndOffset": 3 }] }), "offsets in a partition are dense and increasing"),
        (serde_json::json!({ "records": [{ "topic": "orders", "partition": 0, "offset": 7, "records": [{ "value": "x" }] }] }), "offsets in a partition are dense and increasing"),
        (serde_json::json!({ "records": [{ "topic": "orders", "partition": 9, "records": [{ "value": "x" }] }] }), "a send to a partition the topic lacks is refused"),
        (serde_json::json!({ "groups": [{ "group": "h", "members": [{ "member": "a", "topics": ["orders"] }],
            "assignment": { "a": [{ "topic": "orders", "partition": 0 }, { "topic": "orders", "partition": 0 }] } }] }), "every partition has exactly one owner in a generation"),
        (serde_json::json!({ "groups": [{ "group": "h", "members": [{ "member": "a", "topics": ["orders"] }, { "member": "b", "topics": ["orders"] }],
            "assignment": { "a": [{ "topic": "orders", "partition": 0 }], "b": [{ "topic": "orders", "partition": 1 }, { "topic": "orders", "partition": 2 }] } }] }), "the coordinator assigns every partition exactly once"),
        (serde_json::json!({ "transactions": [{ "transactionalId": "t2", "end": "commit" }] }), "a transaction must begin before it ends"),
    ] {
        let doc: ForceDoc = serde_json::from_value(doc).unwrap();
        let refusal = force::force(&mut core, &doc).expect_err("an impossible state");
        assert_eq!(refusal.law, law, "{}", refusal.reason);
    }
    assert_eq!(core.history.len(), before);
    // The epoch of a transactional id only goes up.
    let up: ForceDoc = serde_json::from_value(serde_json::json!({ "producers": [{ "transactionalId": "tx", "epoch": 4 }] })).unwrap();
    force::force(&mut core, &up).unwrap();
    let down: ForceDoc = serde_json::from_value(serde_json::json!({ "producers": [{ "transactionalId": "tx", "epoch": 2 }] })).unwrap();
    assert_eq!(force::force(&mut core, &down).unwrap_err().law, "a new epoch fences the older producer");
    assert!(history::replay(&core.history).1.is_empty());
    assert!(check_history(&core.history).is_empty(), "{:?}", check_history(&core.history));
}
