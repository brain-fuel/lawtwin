// Scaffolded by LawSpec. User-owned; never overwritten.
//! The native handlers of the lawtwin.kafka contract, for running its laws.
//!
//! A case's handlers are all made with the case's one Cluster. Here the
//! cluster is the generated reference model: the `cluster` spec handler for
//! Broker, Admin and GroupCoordinator, with the murmur2 partitioner, and the
//! client spec handlers written over them. In conformance runs a connection
//! to the Kafka twin or to a real cluster takes its place.
#![allow(unused_variables, unused_imports, non_snake_case)]
use crate::lawspec_abilities::lawtwin_kafka as abilities;
use crate::lawspec_runtime as ls;
use std::sync::Arc;
use abilities::{Consumer as _, Partitioner as _, Producer as _, Transactions as _};

const BROKER: &str = "lawtwin.kafka::ability::Broker";
const PARTITIONER: &str = "lawtwin.kafka::ability::Partitioner";

/// What a Cluster handle holds: the reference model, and a context with it
/// installed for the client spec handlers to call.
pub struct ReferenceCluster {
    model: Arc<abilities::Cluster>,
    partitioner: Arc<abilities::MurmurPartitioner>,
    ctx: ls::Context,
}

fn reference(cluster: &crate::lawspec_data::Cluster) -> &ReferenceCluster {
    cluster.native::<ReferenceCluster>().unwrap_or_else(|error| panic!("{error}"))
}

// LawSpec: (Unit -> lawtwin.kafka::type::Cluster)
pub fn connectCluster(value0: ()) -> crate::lawspec_data::Cluster {
    let mut ctx = ls::Context::default();
    let model = Arc::new(abilities::Cluster::new(&ctx));
    let partitioner = Arc::new(abilities::MurmurPartitioner::new(&ctx));
    ctx.install_handlers(vec![
        (BROKER.to_string(), ls::installed(model.clone() as Arc<dyn abilities::Broker>)),
        (PARTITIONER.to_string(), ls::installed(partitioner.clone() as Arc<dyn abilities::Partitioner>)),
    ]);
    ls::Handle::new(ReferenceCluster { model, partitioner, ctx })
}

// LawSpec: (lawtwin.kafka::type::Cluster -> Unit)
pub fn disconnectCluster(value0: crate::lawspec_data::Cluster) -> () {}

/// The native handler of Broker: createTopicsRequest, createPartitionsRequest, metadataRequest, produceRequest, fetchRequest, listOffsetsRequest, initProducerIdRequest, addPartitionsToTxnRequest, txnOffsetCommitRequest, endTxnRequest, joinGroupRequest, syncGroupRequest, heartbeatRequest, leaveGroupRequest, describeGroupRequest, offsetCommitRequest, offsetFetchRequest.
pub struct BrokerHandler {
    pub resource: crate::lawspec_data::Cluster,
}

impl BrokerHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        Self { resource }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::Broker for BrokerHandler {
    fn createTopicsRequest(&self, value0: std::string::String, value1: i32) -> () {
        <abilities::Cluster as abilities::Broker>::createTopicsRequest(&reference(&self.resource).model, value0, value1)
    }

    fn createPartitionsRequest(&self, value0: std::string::String, value1: i32) -> () {
        <abilities::Cluster as abilities::Broker>::createPartitionsRequest(&reference(&self.resource).model, value0, value1)
    }

    fn metadataRequest(&self, value0: std::string::String) -> std::option::Option<i32> {
        <abilities::Cluster as abilities::Broker>::metadataRequest(&reference(&self.resource).model, value0)
    }

    fn produceRequest(&self, value0: crate::lawspec_data::TopicPartition, value1: std::option::Option<crate::lawspec_data::ProducerTag>, value2: std::vec::Vec<crate::lawspec_data::Record>) -> i64 {
        <abilities::Cluster as abilities::Broker>::produceRequest(&reference(&self.resource).model, value0, value1, value2)
    }

    fn fetchRequest(&self, value0: crate::lawspec_data::TopicPartition, value1: i64, value2: i32, value3: crate::lawspec_data::IsolationLevel) -> crate::lawspec_data::FetchResult {
        <abilities::Cluster as abilities::Broker>::fetchRequest(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn listOffsetsRequest(&self, value0: crate::lawspec_data::TopicPartition, value1: crate::lawspec_data::OffsetSpec, value2: crate::lawspec_data::IsolationLevel) -> i64 {
        <abilities::Cluster as abilities::Broker>::listOffsetsRequest(&reference(&self.resource).model, value0, value1, value2)
    }

    fn initProducerIdRequest(&self, value0: std::option::Option<std::string::String>) -> crate::lawspec_data::ProducerIdAndEpoch {
        <abilities::Cluster as abilities::Broker>::initProducerIdRequest(&reference(&self.resource).model, value0)
    }

    fn addPartitionsToTxnRequest(&self, value0: std::string::String, value1: i64, value2: i16, value3: std::vec::Vec<crate::lawspec_data::TopicPartition>) -> () {
        <abilities::Cluster as abilities::Broker>::addPartitionsToTxnRequest(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn txnOffsetCommitRequest(&self, value0: std::string::String, value1: i64, value2: i16, value3: std::string::String, value4: std::vec::Vec<crate::lawspec_data::OffsetEntry>) -> () {
        <abilities::Cluster as abilities::Broker>::txnOffsetCommitRequest(&reference(&self.resource).model, value0, value1, value2, value3, value4)
    }

    fn endTxnRequest(&self, value0: std::string::String, value1: i64, value2: i16, value3: bool) -> () {
        <abilities::Cluster as abilities::Broker>::endTxnRequest(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn joinGroupRequest(&self, value0: std::string::String, value1: std::string::String, value2: std::vec::Vec<std::string::String>) -> i32 {
        <abilities::Cluster as abilities::Broker>::joinGroupRequest(&reference(&self.resource).model, value0, value1, value2)
    }

    fn syncGroupRequest(&self, value0: std::string::String, value1: std::string::String, value2: i32) -> std::vec::Vec<crate::lawspec_data::TopicPartition> {
        <abilities::Cluster as abilities::Broker>::syncGroupRequest(&reference(&self.resource).model, value0, value1, value2)
    }

    fn heartbeatRequest(&self, value0: std::string::String, value1: std::string::String, value2: i32) -> () {
        <abilities::Cluster as abilities::Broker>::heartbeatRequest(&reference(&self.resource).model, value0, value1, value2)
    }

    fn leaveGroupRequest(&self, value0: std::string::String, value1: std::string::String) -> () {
        <abilities::Cluster as abilities::Broker>::leaveGroupRequest(&reference(&self.resource).model, value0, value1)
    }

    fn describeGroupRequest(&self, value0: std::string::String) -> crate::lawspec_data::GroupView {
        <abilities::Cluster as abilities::Broker>::describeGroupRequest(&reference(&self.resource).model, value0)
    }

    fn offsetCommitRequest(&self, value0: std::string::String, value1: std::string::String, value2: i32, value3: std::vec::Vec<crate::lawspec_data::OffsetEntry>) -> () {
        <abilities::Cluster as abilities::Broker>::offsetCommitRequest(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn offsetFetchRequest(&self, value0: std::string::String, value1: crate::lawspec_data::TopicPartition) -> std::option::Option<i64> {
        <abilities::Cluster as abilities::Broker>::offsetFetchRequest(&reference(&self.resource).model, value0, value1)
    }
}

/// The native handler of Admin: createTopic, createPartitions, partitionsFor, listOffsets.
pub struct AdminHandler {
    pub resource: crate::lawspec_data::Cluster,
}

impl AdminHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        Self { resource }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::Admin for AdminHandler {
    fn createTopic(&self, value0: std::string::String, value1: i32) -> () {
        <abilities::Cluster as abilities::Admin>::createTopic(&reference(&self.resource).model, value0, value1)
    }

    fn createPartitions(&self, value0: std::string::String, value1: i32) -> () {
        <abilities::Cluster as abilities::Admin>::createPartitions(&reference(&self.resource).model, value0, value1)
    }

    fn partitionsFor(&self, value0: std::string::String) -> std::option::Option<i32> {
        <abilities::Cluster as abilities::Admin>::partitionsFor(&reference(&self.resource).model, value0)
    }

    fn listOffsets(&self, value0: crate::lawspec_data::TopicPartition, value1: crate::lawspec_data::OffsetSpec, value2: crate::lawspec_data::IsolationLevel) -> i64 {
        <abilities::Cluster as abilities::Admin>::listOffsets(&reference(&self.resource).model, value0, value1, value2)
    }
}

/// The native handler of Partitioner: choosePartition.
pub struct PartitionerHandler {
    pub resource: crate::lawspec_data::Cluster,
}

impl PartitionerHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        Self { resource }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::Partitioner for PartitionerHandler {
    fn choosePartition(&self, value0: std::option::Option<std::vec::Vec<u8>>, value1: i32) -> i32 {
        reference(&self.resource).partitioner.choosePartition(value0, value1)
    }
}

/// The native handler of Producer: send, flush, enableIdempotence, retryLastSend, producerIdentity.
pub struct ProducerHandler {
    pub resource: crate::lawspec_data::Cluster,
    inner: abilities::KafkaProducer,
}

impl ProducerHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        let inner = abilities::KafkaProducer::new(&reference(&resource).ctx);
        Self { resource, inner }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::Producer for ProducerHandler {
    fn send(&self, value0: crate::lawspec_data::ProducerRecord) -> ls::Either<crate::lawspec_data::KafkaError, crate::lawspec_data::RecordMetadata> {
        self.inner.send(value0)
    }

    fn flush(&self) -> () {
        self.inner.flush()
    }

    fn enableIdempotence(&self) -> ls::Either<crate::lawspec_data::KafkaError, crate::lawspec_data::ProducerIdAndEpoch> {
        self.inner.enableIdempotence()
    }

    fn retryLastSend(&self) -> ls::Either<crate::lawspec_data::KafkaError, crate::lawspec_data::RecordMetadata> {
        self.inner.retryLastSend()
    }

    fn producerIdentity(&self) -> std::option::Option<crate::lawspec_data::ProducerIdAndEpoch> {
        self.inner.producerIdentity()
    }
}

/// The native handler of Consumer: assign, subscribe, poll, commitSync, seek, position, committedOffset, setIsolationLevel, close.
pub struct ConsumerHandler {
    pub resource: crate::lawspec_data::Cluster,
    inner: abilities::KafkaConsumer,
}

impl ConsumerHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        let inner = abilities::KafkaConsumer::new(&reference(&resource).ctx);
        Self { resource, inner }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::Consumer for ConsumerHandler {
    fn assign(&self, value0: std::vec::Vec<crate::lawspec_data::TopicPartition>) -> () {
        self.inner.assign(value0)
    }

    fn subscribe(&self, value0: std::string::String, value1: std::string::String, value2: std::vec::Vec<std::string::String>) -> ls::Either<crate::lawspec_data::KafkaError, std::vec::Vec<crate::lawspec_data::TopicPartition>> {
        self.inner.subscribe(value0, value1, value2)
    }

    fn poll(&self, value0: i32) -> ls::Either<crate::lawspec_data::KafkaError, std::vec::Vec<crate::lawspec_data::ConsumerRecord>> {
        self.inner.poll(value0)
    }

    fn commitSync(&self) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.commitSync()
    }

    fn seek(&self, value0: crate::lawspec_data::TopicPartition, value1: i64) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.seek(value0, value1)
    }

    fn position(&self, value0: crate::lawspec_data::TopicPartition) -> ls::Either<crate::lawspec_data::KafkaError, i64> {
        self.inner.position(value0)
    }

    fn committedOffset(&self, value0: crate::lawspec_data::TopicPartition) -> std::option::Option<i64> {
        self.inner.committedOffset(value0)
    }

    fn setIsolationLevel(&self, value0: crate::lawspec_data::IsolationLevel) -> () {
        self.inner.setIsolationLevel(value0)
    }

    fn close(&self) -> () {
        self.inner.close()
    }
}

/// The native handler of GroupCoordinator: joinGroup, syncGroup, heartbeat, leaveGroup, describeGroup, commitOffsets, fetchOffset.
pub struct GroupCoordinatorHandler {
    pub resource: crate::lawspec_data::Cluster,
}

impl GroupCoordinatorHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        Self { resource }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::GroupCoordinator for GroupCoordinatorHandler {
    fn joinGroup(&self, value0: std::string::String, value1: std::string::String, value2: std::vec::Vec<std::string::String>) -> i32 {
        <abilities::Cluster as abilities::GroupCoordinator>::joinGroup(&reference(&self.resource).model, value0, value1, value2)
    }

    fn syncGroup(&self, value0: std::string::String, value1: std::string::String, value2: i32) -> std::vec::Vec<crate::lawspec_data::TopicPartition> {
        <abilities::Cluster as abilities::GroupCoordinator>::syncGroup(&reference(&self.resource).model, value0, value1, value2)
    }

    fn heartbeat(&self, value0: std::string::String, value1: std::string::String, value2: i32) -> () {
        <abilities::Cluster as abilities::GroupCoordinator>::heartbeat(&reference(&self.resource).model, value0, value1, value2)
    }

    fn leaveGroup(&self, value0: std::string::String, value1: std::string::String) -> () {
        <abilities::Cluster as abilities::GroupCoordinator>::leaveGroup(&reference(&self.resource).model, value0, value1)
    }

    fn describeGroup(&self, value0: std::string::String) -> crate::lawspec_data::GroupView {
        <abilities::Cluster as abilities::GroupCoordinator>::describeGroup(&reference(&self.resource).model, value0)
    }

    fn commitOffsets(&self, value0: std::string::String, value1: std::string::String, value2: i32, value3: std::vec::Vec<crate::lawspec_data::OffsetEntry>) -> () {
        <abilities::Cluster as abilities::GroupCoordinator>::commitOffsets(&reference(&self.resource).model, value0, value1, value2, value3)
    }

    fn fetchOffset(&self, value0: std::string::String, value1: crate::lawspec_data::TopicPartition) -> std::option::Option<i64> {
        <abilities::Cluster as abilities::GroupCoordinator>::fetchOffset(&reference(&self.resource).model, value0, value1)
    }
}

/// The native handler of Transactions: initTransactions, beginTransaction, addPartitionsToTransaction, sendInTransaction, sendOffsetsToTransaction, commitTransaction, abortTransaction.
pub struct TransactionsHandler {
    pub resource: crate::lawspec_data::Cluster,
    inner: abilities::KafkaTransactions,
}

impl TransactionsHandler {
    pub fn new(resource: crate::lawspec_data::Cluster) -> Self {
        let inner = abilities::KafkaTransactions::new(&reference(&resource).ctx);
        Self { resource, inner }
    }
}

impl crate::lawspec_abilities::lawtwin_kafka::Transactions for TransactionsHandler {
    fn initTransactions(&self, value0: std::string::String) -> ls::Either<crate::lawspec_data::KafkaError, crate::lawspec_data::ProducerIdAndEpoch> {
        self.inner.initTransactions(value0)
    }

    fn beginTransaction(&self) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.beginTransaction()
    }

    fn addPartitionsToTransaction(&self, value0: std::vec::Vec<crate::lawspec_data::TopicPartition>) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.addPartitionsToTransaction(value0)
    }

    fn sendInTransaction(&self, value0: crate::lawspec_data::ProducerRecord) -> ls::Either<crate::lawspec_data::KafkaError, crate::lawspec_data::RecordMetadata> {
        self.inner.sendInTransaction(value0)
    }

    fn sendOffsetsToTransaction(&self, value0: std::string::String, value1: std::vec::Vec<crate::lawspec_data::OffsetEntry>) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.sendOffsetsToTransaction(value0, value1)
    }

    fn commitTransaction(&self) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.commitTransaction()
    }

    fn abortTransaction(&self) -> ls::Either<crate::lawspec_data::KafkaError, ()> {
        self.inner.abortTransaction()
    }
}
