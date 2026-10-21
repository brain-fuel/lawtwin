//! The reference model the twin runs: the Kafka contract's `cluster` spec
//! handler, through LawSpec's public embedding interface.
//!
//! Each operation calls the generated handler's public operation or
//! `try_` form. Snapshots, restoration and cloning use its public state
//! interface; contract failures remain distinct from defects.
//! ref:DEC-embed-generated-model

use lawspec_example::lawspec_abilities::lawtwin_kafka::{Broker, Cluster};
use lawspec_example::lawspec_definitions::typed;
use lawspec_example::lawspec_runtime as ls;

pub use lawspec_example::lawspec_data as data;

/// Why an operation of the model gave no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The contract refused the request with this error.
    Kafka(data::KafkaError),
    /// The generated code failed; this is a defect, never a refusal.
    Model(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Kafka(e) => write!(f, "{e:?}"),
            Failure::Model(m) => write!(f, "model failure: {m}"),
        }
    }
}

/// The Kafka contract's reference model, with its state.
pub struct ClusterModel {
    handler: Cluster,
}

impl Clone for ClusterModel {
    fn clone(&self) -> Self {
        let handler = Cluster::start();
        handler.set_typed_state(self.handler.typed_state());
        Self { handler }
    }
}

impl Default for ClusterModel {
    fn default() -> Self {
        Self::start()
    }
}

/// Catch defects outside the public typed failure result. A panic must never
/// be relabelled as a Kafka protocol refusal.
fn run<T>(body: impl FnOnce() -> Result<T, data::KafkaError>) -> Result<T, Failure> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => result.map_err(Failure::Kafka),
        Err(payload) => Err(Failure::Model(panic_text(&payload))),
    }
}

fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "a panic in the generated model".into()
    }
}

impl ClusterModel {
    /// The model in its start state: the contract's `model.emptyBroker`.
    pub fn start() -> Self {
        Self {
            handler: Cluster::start(),
        }
    }

    /// The whole state, in the generated native types. It costs a copy of
    /// every record, so `inspect` uses it and requests do not.
    pub fn snapshot(&self) -> data::BrokerState {
        self.handler.state()
    }

    /// The topics and their partition counts, without copying any log.
    pub fn topics(&self) -> std::collections::BTreeMap<String, i32> {
        let typed::BrokerStateNode::BrokerState(topics, ..) = &*self.handler.typed_state().0;
        ls::fast::Public::to_native(topics)
    }

    /// The group ids the model holds, without copying any log.
    pub fn group_ids(&self) -> Vec<String> {
        let typed::BrokerStateNode::BrokerState(_, _, _, _, _, groups, _) =
            &*self.handler.typed_state().0;
        let native: std::collections::BTreeMap<String, data::Group> =
            ls::fast::Public::to_native(groups);
        native.into_keys().collect()
    }
}

impl ClusterModel {
    /// The producer id and epoch a transactional id maps to, without
    /// copying any log.
    pub fn producer_of(&self, transactional_id: &str) -> Option<(i64, i16)> {
        let typed::BrokerStateNode::BrokerState(_, _, producers, ids, ..) =
            &*self.handler.typed_state().0;
        let ids: std::collections::BTreeMap<String, i64> = ls::fast::Public::to_native(ids);
        let pid = *ids.get(transactional_id)?;
        let producers: std::collections::BTreeMap<i64, data::ProducerEntry> =
            ls::fast::Public::to_native(producers);
        producers.get(&pid).map(|p| (pid, p.epoch))
    }
}

#[allow(non_snake_case)]
impl ClusterModel {
    pub fn createTopicsRequest(
        &mut self,
        value0: std::string::String,
        value1: i32,
    ) -> Result<(), Failure> {
        run(|| self.handler.try_createTopicsRequest(value0, value1))
    }

    pub fn createPartitionsRequest(
        &mut self,
        value0: std::string::String,
        value1: i32,
    ) -> Result<(), Failure> {
        run(|| self.handler.try_createPartitionsRequest(value0, value1))
    }

    pub fn metadataRequest(
        &mut self,
        value0: std::string::String,
    ) -> Result<std::option::Option<i32>, Failure> {
        run(|| Ok(self.handler.metadataRequest(value0)))
    }

    pub fn produceRequest(
        &mut self,
        value0: data::TopicPartition,
        value1: std::option::Option<data::ProducerTag>,
        value2: std::vec::Vec<data::Record>,
    ) -> Result<i64, Failure> {
        run(|| self.handler.try_produceRequest(value0, value1, value2))
    }

    pub fn fetchRequest(
        &mut self,
        value0: data::TopicPartition,
        value1: i64,
        value2: i32,
        value3: data::IsolationLevel,
    ) -> Result<data::FetchResult, Failure> {
        run(|| {
            self.handler
                .try_fetchRequest(value0, value1, value2, value3)
        })
    }

    pub fn listOffsetsRequest(
        &mut self,
        value0: data::TopicPartition,
        value1: data::OffsetSpec,
        value2: data::IsolationLevel,
    ) -> Result<i64, Failure> {
        run(|| self.handler.try_listOffsetsRequest(value0, value1, value2))
    }

    pub fn initProducerIdRequest(
        &mut self,
        value0: std::option::Option<std::string::String>,
    ) -> Result<data::ProducerIdAndEpoch, Failure> {
        run(|| self.handler.try_initProducerIdRequest(value0))
    }

    pub fn addPartitionsToTxnRequest(
        &mut self,
        value0: std::string::String,
        value1: i64,
        value2: i16,
        value3: std::vec::Vec<data::TopicPartition>,
    ) -> Result<(), Failure> {
        run(|| {
            self.handler
                .try_addPartitionsToTxnRequest(value0, value1, value2, value3)
        })
    }

    pub fn txnOffsetCommitRequest(
        &mut self,
        value0: std::string::String,
        value1: i64,
        value2: i16,
        value3: std::string::String,
        value4: std::vec::Vec<data::OffsetEntry>,
    ) -> Result<(), Failure> {
        run(|| {
            self.handler
                .try_txnOffsetCommitRequest(value0, value1, value2, value3, value4)
        })
    }

    pub fn endTxnRequest(
        &mut self,
        value0: std::string::String,
        value1: i64,
        value2: i16,
        value3: bool,
    ) -> Result<(), Failure> {
        run(|| {
            self.handler
                .try_endTxnRequest(value0, value1, value2, value3)
        })
    }

    pub fn joinGroupRequest(
        &mut self,
        value0: std::string::String,
        value1: std::string::String,
        value2: std::vec::Vec<std::string::String>,
    ) -> Result<i32, Failure> {
        run(|| self.handler.try_joinGroupRequest(value0, value1, value2))
    }

    pub fn syncGroupRequest(
        &mut self,
        value0: std::string::String,
        value1: std::string::String,
        value2: i32,
    ) -> Result<std::vec::Vec<data::TopicPartition>, Failure> {
        run(|| self.handler.try_syncGroupRequest(value0, value1, value2))
    }

    pub fn heartbeatRequest(
        &mut self,
        value0: std::string::String,
        value1: std::string::String,
        value2: i32,
    ) -> Result<(), Failure> {
        run(|| self.handler.try_heartbeatRequest(value0, value1, value2))
    }

    pub fn leaveGroupRequest(
        &mut self,
        value0: std::string::String,
        value1: std::string::String,
    ) -> Result<(), Failure> {
        run(|| self.handler.try_leaveGroupRequest(value0, value1))
    }

    pub fn describeGroupRequest(
        &mut self,
        value0: std::string::String,
    ) -> Result<data::GroupView, Failure> {
        run(|| Ok(self.handler.describeGroupRequest(value0)))
    }

    pub fn offsetCommitRequest(
        &mut self,
        value0: std::string::String,
        value1: std::string::String,
        value2: i32,
        value3: std::vec::Vec<data::OffsetEntry>,
    ) -> Result<(), Failure> {
        run(|| {
            self.handler
                .try_offsetCommitRequest(value0, value1, value2, value3)
        })
    }

    pub fn offsetFetchRequest(
        &mut self,
        value0: std::string::String,
        value1: data::TopicPartition,
    ) -> Result<std::option::Option<i64>, Failure> {
        run(|| Ok(self.handler.offsetFetchRequest(value0, value1)))
    }
}
