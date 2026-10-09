//! The native handlers of the lawtwin.kafka contract, for running its laws.
//!
//! Every handler here serves its ability from one shared cluster: the
//! generated reference model of the Broker ability, with the client spec
//! handlers over it and Kafka's own murmur2 partitioner. A law's handlers are
//! all made before its case runs any operation, so a handler made after an
//! operation starts a new case, and with it a new cluster. A real cluster or
//! the Kafka twin takes the place of the reference model in conformance runs.
#![allow(unused_variables, unused_imports, non_snake_case)]
use crate::lawspec_abilities::lawtwin_kafka_contract as abilities;
use crate::lawspec_data as data;
use crate::lawspec_runtime as ls;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const BROKER: &str = "lawtwin.kafka.contract::ability::Broker";
const PARTITIONER: &str = "lawtwin.kafka.contract::ability::Partitioner";

thread_local! {
    static SESSION: RefCell<Option<ls::Context>> = const { RefCell::new(None) };
    static USED: Cell<bool> = const { Cell::new(false) };
}

/// The context of the current case: a fresh reference cluster and the native
/// partitioner, installed for the client spec handlers to call.
fn session() -> ls::Context {
    SESSION.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() || USED.with(Cell::get) {
            let mut ctx = ls::Context::default();
            let broker: Arc<dyn abilities::Broker> = Arc::new(abilities::Reference::new(&ctx));
            let partitioner: Arc<dyn abilities::Partitioner> = Arc::new(PartitionerHandler);
            ctx.install_handlers(vec![
                (BROKER.to_string(), ls::installed(broker)),
                (PARTITIONER.to_string(), ls::installed(partitioner)),
            ]);
            *slot = Some(ctx);
            USED.with(|used| used.set(false));
        }
        slot.clone().unwrap()
    })
}

fn used() {
    USED.with(|flag| flag.set(true));
}

fn shared_broker() -> Arc<dyn abilities::Broker> {
    session().handler::<Arc<dyn abilities::Broker>>(BROKER).unwrap()
}

/// Kafka's murmur2 hash, as the Java client's `Utils.murmur2`.
pub fn murmur2(data: &[u8]) -> i32 {
    let length = data.len();
    let m: u32 = 0x5bd1_e995;
    let r = 24;
    let mut h: u32 = 0x9747_b28c ^ (length as u32);
    for chunk in data.chunks_exact(4) {
        let mut k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        k = k.wrapping_mul(m);
        k ^= k >> r;
        k = k.wrapping_mul(m);
        h = h.wrapping_mul(m);
        h ^= k;
    }
    let tail = &data[length & !3..];
    if tail.len() >= 3 {
        h ^= (tail[2] as u32) << 16;
    }
    if tail.len() >= 2 {
        h ^= (tail[1] as u32) << 8;
    }
    if !tail.is_empty() {
        h ^= tail[0] as u32;
        h = h.wrapping_mul(m);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(m);
    h ^= h >> 15;
    h as i32
}

/// The Broker: the shared reference cluster.
pub struct BrokerHandler {
    inner: Arc<dyn abilities::Broker>,
}

impl Default for BrokerHandler {
    fn default() -> Self {
        BrokerHandler { inner: shared_broker() }
    }
}

impl abilities::Broker for BrokerHandler {
    fn createTopicsRequest(&self, value0: String, value1: i32) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.createTopicsRequest(value0, value1)
    }

    fn createPartitionsRequest(&self, value0: String, value1: i32) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.createPartitionsRequest(value0, value1)
    }

    fn metadataRequest(&self, value0: String) -> Option<i32> {
        used();
        self.inner.metadataRequest(value0)
    }

    fn produceRequest(&self, value0: data::TopicPartition, value1: Option<data::ProducerTag>, value2: Vec<data::Record>) -> ls::Either<data::KafkaError, i64> {
        used();
        self.inner.produceRequest(value0, value1, value2)
    }

    fn fetchRequest(&self, value0: data::TopicPartition, value1: i64, value2: i32, value3: data::IsolationLevel) -> ls::Either<data::KafkaError, data::FetchResult> {
        used();
        self.inner.fetchRequest(value0, value1, value2, value3)
    }

    fn listOffsetsRequest(&self, value0: data::TopicPartition, value1: data::OffsetSpec, value2: data::IsolationLevel) -> ls::Either<data::KafkaError, i64> {
        used();
        self.inner.listOffsetsRequest(value0, value1, value2)
    }

    fn initProducerIdRequest(&self, value0: Option<String>) -> ls::Either<data::KafkaError, data::ProducerIdAndEpoch> {
        used();
        self.inner.initProducerIdRequest(value0)
    }

    fn addPartitionsToTxnRequest(&self, value0: String, value1: i64, value2: i16, value3: Vec<data::TopicPartition>) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.addPartitionsToTxnRequest(value0, value1, value2, value3)
    }

    fn txnOffsetCommitRequest(&self, value0: String, value1: i64, value2: i16, value3: String, value4: Vec<data::OffsetEntry>) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.txnOffsetCommitRequest(value0, value1, value2, value3, value4)
    }

    fn endTxnRequest(&self, value0: String, value1: i64, value2: i16, value3: bool) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.endTxnRequest(value0, value1, value2, value3)
    }

    fn joinGroupRequest(&self, value0: String, value1: String, value2: Vec<String>) -> ls::Either<data::KafkaError, i32> {
        used();
        self.inner.joinGroupRequest(value0, value1, value2)
    }

    fn syncGroupRequest(&self, value0: String, value1: String, value2: i32) -> ls::Either<data::KafkaError, Vec<data::TopicPartition>> {
        used();
        self.inner.syncGroupRequest(value0, value1, value2)
    }

    fn heartbeatRequest(&self, value0: String, value1: String, value2: i32) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.heartbeatRequest(value0, value1, value2)
    }

    fn leaveGroupRequest(&self, value0: String, value1: String) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.leaveGroupRequest(value0, value1)
    }

    fn describeGroupRequest(&self, value0: String) -> data::GroupView {
        used();
        self.inner.describeGroupRequest(value0)
    }

    fn offsetCommitRequest(&self, value0: String, value1: String, value2: i32, value3: Vec<data::OffsetEntry>) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.offsetCommitRequest(value0, value1, value2, value3)
    }

    fn offsetFetchRequest(&self, value0: String, value1: data::TopicPartition) -> Option<i64> {
        used();
        self.inner.offsetFetchRequest(value0, value1)
    }
}

/// Admin: the client spec handler over the shared cluster.
pub struct AdminHandler {
    inner: abilities::KafkaAdmin,
}

impl Default for AdminHandler {
    fn default() -> Self {
        AdminHandler { inner: abilities::KafkaAdmin::new(&session()) }
    }
}

impl abilities::Admin for AdminHandler {
    fn createTopic(&self, value0: String, value1: i32) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.createTopic(value0, value1)
    }

    fn createPartitions(&self, value0: String, value1: i32) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.createPartitions(value0, value1)
    }

    fn partitionsFor(&self, value0: String) -> Option<i32> {
        used();
        self.inner.partitionsFor(value0)
    }

    fn listOffsets(&self, value0: data::TopicPartition, value1: data::OffsetSpec, value2: data::IsolationLevel) -> ls::Either<data::KafkaError, i64> {
        used();
        self.inner.listOffsets(value0, value1, value2)
    }
}

/// Partitioner: Kafka's default for keyed records, murmur2 of the key; an
/// unkeyed record goes to partition 0.
#[derive(Default)]
pub struct PartitionerHandler;

impl abilities::Partitioner for PartitionerHandler {
    fn choosePartition(&self, value0: Option<Vec<u8>>, value1: i32) -> i32 {
        match value0 {
            Some(key) if value1 > 0 => (murmur2(&key) & 0x7fff_ffff) % value1,
            _ => 0,
        }
    }
}

/// Producer: the client spec handler over the shared cluster.
pub struct ProducerHandler {
    inner: abilities::KafkaProducer,
}

impl Default for ProducerHandler {
    fn default() -> Self {
        ProducerHandler { inner: abilities::KafkaProducer::new(&session()) }
    }
}

impl abilities::Producer for ProducerHandler {
    fn send(&self, value0: data::ProducerRecord) -> ls::Either<data::KafkaError, data::RecordMetadata> {
        used();
        self.inner.send(value0)
    }

    fn flush(&self) {
        used();
        self.inner.flush()
    }

    fn enableIdempotence(&self) -> ls::Either<data::KafkaError, data::ProducerIdAndEpoch> {
        used();
        self.inner.enableIdempotence()
    }

    fn retryLastSend(&self) -> ls::Either<data::KafkaError, data::RecordMetadata> {
        used();
        self.inner.retryLastSend()
    }

    fn producerIdentity(&self) -> Option<data::ProducerIdAndEpoch> {
        used();
        self.inner.producerIdentity()
    }
}

/// Consumer: the client spec handler over the shared cluster.
pub struct ConsumerHandler {
    inner: abilities::KafkaConsumer,
}

impl Default for ConsumerHandler {
    fn default() -> Self {
        ConsumerHandler { inner: abilities::KafkaConsumer::new(&session()) }
    }
}

impl abilities::Consumer for ConsumerHandler {
    fn assign(&self, value0: Vec<data::TopicPartition>) {
        used();
        self.inner.assign(value0)
    }

    fn subscribe(&self, value0: String, value1: String, value2: Vec<String>) -> ls::Either<data::KafkaError, Vec<data::TopicPartition>> {
        used();
        self.inner.subscribe(value0, value1, value2)
    }

    fn poll(&self, value0: i32) -> ls::Either<data::KafkaError, Vec<data::ConsumerRecord>> {
        used();
        self.inner.poll(value0)
    }

    fn commitSync(&self) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.commitSync()
    }

    fn seek(&self, value0: data::TopicPartition, value1: i64) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.seek(value0, value1)
    }

    fn position(&self, value0: data::TopicPartition) -> ls::Either<data::KafkaError, i64> {
        used();
        self.inner.position(value0)
    }

    fn committedOffset(&self, value0: data::TopicPartition) -> Option<i64> {
        used();
        self.inner.committedOffset(value0)
    }

    fn setIsolationLevel(&self, value0: data::IsolationLevel) {
        used();
        self.inner.setIsolationLevel(value0)
    }

    fn close(&self) {
        used();
        self.inner.close()
    }
}

/// GroupCoordinator: the client spec handler over the shared cluster.
pub struct GroupCoordinatorHandler {
    inner: abilities::KafkaGroups,
}

impl Default for GroupCoordinatorHandler {
    fn default() -> Self {
        GroupCoordinatorHandler { inner: abilities::KafkaGroups::new(&session()) }
    }
}

impl abilities::GroupCoordinator for GroupCoordinatorHandler {
    fn joinGroup(&self, value0: String, value1: String, value2: Vec<String>) -> ls::Either<data::KafkaError, i32> {
        used();
        self.inner.joinGroup(value0, value1, value2)
    }

    fn syncGroup(&self, value0: String, value1: String, value2: i32) -> ls::Either<data::KafkaError, Vec<data::TopicPartition>> {
        used();
        self.inner.syncGroup(value0, value1, value2)
    }

    fn heartbeat(&self, value0: String, value1: String, value2: i32) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.heartbeat(value0, value1, value2)
    }

    fn leaveGroup(&self, value0: String, value1: String) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.leaveGroup(value0, value1)
    }

    fn describeGroup(&self, value0: String) -> data::GroupView {
        used();
        self.inner.describeGroup(value0)
    }

    fn commitOffsets(&self, value0: String, value1: String, value2: i32, value3: Vec<data::OffsetEntry>) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.commitOffsets(value0, value1, value2, value3)
    }

    fn fetchOffset(&self, value0: String, value1: data::TopicPartition) -> Option<i64> {
        used();
        self.inner.fetchOffset(value0, value1)
    }
}

/// Transactions: the client spec handler over the shared cluster.
pub struct TransactionsHandler {
    inner: abilities::KafkaTransactions,
}

impl Default for TransactionsHandler {
    fn default() -> Self {
        TransactionsHandler { inner: abilities::KafkaTransactions::new(&session()) }
    }
}

impl abilities::Transactions for TransactionsHandler {
    fn initTransactions(&self, value0: String) -> ls::Either<data::KafkaError, data::ProducerIdAndEpoch> {
        used();
        self.inner.initTransactions(value0)
    }

    fn beginTransaction(&self) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.beginTransaction()
    }

    fn addPartitionsToTransaction(&self, value0: Vec<data::TopicPartition>) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.addPartitionsToTransaction(value0)
    }

    fn sendInTransaction(&self, value0: data::ProducerRecord) -> ls::Either<data::KafkaError, data::RecordMetadata> {
        used();
        self.inner.sendInTransaction(value0)
    }

    fn sendOffsetsToTransaction(&self, value0: String, value1: Vec<data::OffsetEntry>) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.sendOffsetsToTransaction(value0, value1)
    }

    fn commitTransaction(&self) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.commitTransaction()
    }

    fn abortTransaction(&self) -> ls::Either<data::KafkaError, ()> {
        used();
        self.inner.abortTransaction()
    }
}

#[cfg(test)]
mod tests {
    use super::murmur2;

    /// The values the Java client's own murmur2 test checks.
    #[test]
    fn murmur2_matches_kafka() {
        assert_eq!(murmur2(b"21"), -973932308);
        assert_eq!(murmur2(b"foobar"), -790332482);
        assert_eq!(murmur2(b"a-little-bit-long-string"), -985981536);
        assert_eq!(murmur2(b"a-little-bit-longer-string"), -1486304829);
        assert_eq!(murmur2(b"lkjh234lh9fiuh90y23oiuhsafujhadof229phr9h19h89h8"), -58897971);
        assert_eq!(murmur2(b"abc"), 479470107);
    }
}
