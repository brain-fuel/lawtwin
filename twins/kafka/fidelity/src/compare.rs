//! Comparing a recorded history with the contract: each operation is run on
//! the reference model in turn, and the broker's answer compared with the
//! model's.
//!
//! A real broker chooses some values the contract leaves to it: producer
//! ids, a transactional producer's first epoch, and generation numbers. The
//! comparison is up to a renaming of those (ref:DEC-compare-up-to-renaming):
//! when the broker hands one out, the model's choice is noted beside it,
//! and later operations that carry the broker's value are run with the
//! model's. Everything else must be equal.

use lawtwin_kafka::history::{Answer, Entry, Event, Op, Outcome, apply};
use serde::{Deserialize, Serialize};
use lawtwin_kafka::model::ClusterModel;
use serde_json::Value;
use std::collections::HashMap;

/// A renaming from a broker's identifiers to a model's.
#[derive(Clone, Debug, Default)]
pub struct Renaming {
    /// Each producer id the broker gave: the model's, and how far the
    /// broker's epochs are ahead of the model's.
    pub producers: HashMap<i64, (i64, i16)>,
    /// For each group, each generation the broker gave: the model's.
    pub generations: HashMap<String, HashMap<i32, i32>>,
}

impl Renaming {
    fn producer(&self, pid: i64, epoch: i16) -> (i64, i16) {
        match self.producers.get(&pid) {
            Some((model, ahead)) => (*model, epoch.saturating_sub(*ahead)),
            None => (pid, epoch),
        }
    }

    fn generation(&self, group: &str, g: i32) -> i32 {
        self.generations.get(group).and_then(|m| m.get(&g)).copied().unwrap_or(g)
    }

    /// The operation with the model's identifiers.
    pub fn op(&self, op: &Op) -> Op {
        let mut op = op.clone();
        match &mut op {
            Op::ProduceRequest { tag: Some(t), .. } => {
                (t.producer_id, t.producer_epoch) = self.producer(t.producer_id, t.producer_epoch);
            }
            Op::AddPartitionsToTxnRequest { producer_id, producer_epoch, .. }
            | Op::TxnOffsetCommitRequest { producer_id, producer_epoch, .. }
            | Op::EndTxnRequest { producer_id, producer_epoch, .. } => {
                (*producer_id, *producer_epoch) = self.producer(*producer_id, *producer_epoch);
            }
            Op::SyncGroupRequest { group, generation, .. }
            | Op::HeartbeatRequest { group, generation, .. }
            | Op::OffsetCommitRequest { group, generation, .. } => {
                *generation = self.generation(group, *generation);
            }
            _ => {}
        }
        op
    }

    /// Notes what the broker and the model each handed out.
    pub fn learn(&mut self, op: &Op, recorded: &Answer, model: &Answer) {
        match (op, recorded, model) {
            (
                Op::InitProducerIdRequest { .. },
                Answer::Ok(Outcome::Producer { producer_id: r, producer_epoch: re }),
                Answer::Ok(Outcome::Producer { producer_id: m, producer_epoch: me }),
            ) => {
                self.producers.insert(*r, (*m, re.saturating_sub(*me)));
            }
            (Op::JoinGroupRequest { group, .. }, Answer::Ok(Outcome::Number(r)), Answer::Ok(Outcome::Number(m))) => {
                self.generations.entry(group.clone()).or_default().insert(*r as i32, *m as i32);
            }
            _ => {}
        }
    }
}

/// Two answers, the broker's and the model's, compared up to the renaming.
pub fn same(op: &Op, recorded: &Answer, model: &Answer, renaming: &Renaming) -> bool {
    match (op, recorded, model) {
        (_, Answer::Error(a), Answer::Error(b)) => {
            a == b || crate::decode::contract_code(a).is_some_and(|x| crate::decode::contract_code(b) == Some(x))
        }
        (Op::InitProducerIdRequest { .. }, Answer::Ok(Outcome::Producer { .. }), Answer::Ok(Outcome::Producer { .. })) => true,
        (Op::JoinGroupRequest { .. }, Answer::Ok(Outcome::Number(_)), Answer::Ok(Outcome::Number(_))) => true,
        (Op::DescribeGroupRequest { group }, Answer::Ok(Outcome::Group { generation: a, members: ma }), Answer::Ok(Outcome::Group { generation: b, members: mb })) => {
            ma == mb && (renaming.generation(group, *a) == *b || *a == 0)
        }
        _ => recorded == model,
    }
}

/// An operation whose answer from the broker is not the model's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// The history entry.
    pub seq: u64,
    /// The operation and its arguments, as recorded.
    pub op: Value,
    /// The broker's answer.
    pub recorded: Value,
    /// The reference model's answer.
    pub model: Value,
}

/// One step of a comparison: the operation run on the model, and whether
/// its answer is the broker's.
pub struct Checker {
    /// The reference model, after every operation so far.
    pub model: ClusterModel,
    /// The renaming so far.
    pub renaming: Renaming,
}

impl Default for Checker {
    fn default() -> Self {
        Self { model: ClusterModel::start(), renaming: Renaming::default() }
    }
}

impl Checker {
    /// Runs an operation and compares the model's answer with the broker's.
    pub fn step(&mut self, seq: u64, op: &Op, recorded: &Answer) -> Option<Divergence> {
        step(&mut self.model, &mut self.renaming, seq, op, recorded)
    }
}

/// Runs an operation on a model, with the broker's identifiers renamed, and
/// compares the model's answer with the broker's.
pub fn step(model: &mut ClusterModel, renaming: &mut Renaming, seq: u64, op: &Op, recorded: &Answer) -> Option<Divergence> {
    let renamed = renaming.op(op);
    let answer = Answer::of(&apply(model, &renamed));
    renaming.learn(op, recorded, &answer);
    if same(op, recorded, &answer, renaming) {
        return None;
    }
    Some(Divergence {
        seq,
        op: serde_json::to_value(op).unwrap_or(Value::Null),
        recorded: serde_json::to_value(recorded).unwrap_or(Value::Null),
        model: serde_json::to_value(&answer).unwrap_or(Value::Null),
    })
}

/// Compares a whole history with the contract: the model after it and every
/// divergence.
pub fn compare(entries: &[Entry]) -> (Checker, Vec<Divergence>) {
    let mut checker = Checker::default();
    let mut out = vec![];
    for e in entries {
        if let Event::Call { op, answer } = &e.event {
            if let Some(d) = checker.step(e.seq, op, answer) {
                out.push(d);
            }
        }
    }
    (checker, out)
}
