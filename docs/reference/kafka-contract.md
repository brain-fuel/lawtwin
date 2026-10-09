---
id: lawtwin.reference.kafka-contract
kind: reference
title: The Kafka contract
---
# The Kafka contract

`lawtwin.kafka` 0.1.0 is a LawSpec package (ref:lawspec) in
`contracts/kafka/`. It states what a Kafka cluster guarantees to its clients
(ref:kafka-protocol). It names no application, and nothing about twins.

| Unit | Holds |
| --- | --- |
| `lawtwin.kafka.model` | the data of a cluster, and one checked definition for each request it answers |
| `lawtwin.kafka.contract` | the abilities, their laws, the reference handler, and the client handlers |

## Abilities

There are two layers (ref:DEC-kafka-broker-and-clients).

**`Broker`** is the cluster as the wire protocol sees it. Each operation is
one request.

| Operation | Request | Gives |
| --- | --- | --- |
| `createTopicsRequest name n` | CreateTopics | nothing, or an error |
| `createPartitionsRequest name total` | CreatePartitions | nothing, or an error |
| `metadataRequest name` | Metadata | the topic's partition count, if it exists |
| `produceRequest tp tag records` | Produce | the batch's base offset |
| `fetchRequest tp offset max isolation` | Fetch | records, the next offset, the high watermark and the last stable offset |
| `listOffsetsRequest tp spec isolation` | ListOffsets | the earliest or latest offset |
| `initProducerIdRequest txid` | InitProducerId | a producer id and epoch |
| `addPartitionsToTxnRequest txid pid epoch tps` | AddPartitionsToTxn | nothing, or an error |
| `txnOffsetCommitRequest txid pid epoch group offsets` | AddOffsetsToTxn and TxnOffsetCommit | nothing, or an error |
| `endTxnRequest txid pid epoch commit` | EndTxn | nothing, or an error |
| `joinGroupRequest group member topics` | JoinGroup | the generation |
| `syncGroupRequest group member generation` | SyncGroup | the member's partitions |
| `heartbeatRequest group member generation` | Heartbeat | nothing, or an error |
| `leaveGroupRequest group member` | LeaveGroup | nothing, or an error |
| `describeGroupRequest group` | DescribeGroups | the generation and every member's partitions |
| `offsetCommitRequest group member generation offsets` | OffsetCommit | nothing, or an error |
| `offsetFetchRequest group tp` | OffsetFetch | the committed offset, if any |

A request that can fail gives `Either KafkaError a`. `KafkaError` names
Kafka's error codes: `UnknownTopicOrPartition`, `TopicAlreadyExists`,
`InvalidTopic`, `InvalidPartitions`, `InvalidRecord`, `InvalidRequest`,
`OffsetOutOfRange`, `OutOfOrderSequenceNumber`, `InvalidProducerEpoch`,
`ProducerFenced`, `UnknownProducerId`, `InvalidProducerIdMapping`,
`InvalidTxnState`, `InvalidGroupId`, `UnknownMemberId`, `IllegalGeneration`
and `IllegalState`.

**The client abilities** are what a client library offers. Each has a spec
handler written over `Broker`.

| Ability | Operations | Spec handler |
| --- | --- | --- |
| `Admin` | `createTopic`, `createPartitions`, `partitionsFor`, `listOffsets` | `kafkaAdmin` |
| `Producer` | `send`, `flush`, `enableIdempotence`, `retryLastSend`, `producerIdentity` | `kafkaProducer`, which keeps the producer id, epoch and a sequence per partition |
| `Partitioner` | `choosePartition` | `firstPartition` |
| `Consumer` | `assign`, `subscribe`, `poll`, `commitSync`, `seek`, `position`, `committedOffset`, `setIsolationLevel`, `close` | `kafkaConsumer`, which keeps the group, generation, assignment and positions |
| `GroupCoordinator` | `joinGroup`, `syncGroup`, `heartbeat`, `leaveGroup`, `describeGroup`, `commitOffsets`, `fetchOffset` | `kafkaGroups` |
| `Transactions` | `initTransactions`, `beginTransaction`, `addPartitionsToTransaction`, `sendInTransaction`, `sendOffsetsToTransaction`, `commitTransaction`, `abortTransaction` | `kafkaTransactions`, which keeps the transactional id, epoch and open transaction |

A `ProducerRecord` names a topic, a `PartitionChoice` (`ToPartition p`, or
`ByKey` for the partitioner to choose) and a `Record`: an optional key, an
optional value, and headers. Keys and values are `Bytes`.

## The reference model

`reference` is the stateful spec handler of `Broker`. Its state,
`BrokerState`, holds:

- **topics**, each with its partition count;
- **a log per partition**: its entries, newest first; its log end offset;
  the first offset of each open transaction in it; its aborted-transaction
  index; and the last five batches of each producer, to recognise retries;
- **producers**: each producer id's epoch, its transactional id, and its
  open transaction with the partitions and offsets it holds;
- **transactional ids**, each mapped to a producer id;
- **groups**: each group's generation, each member's subscription, and the
  assignment;
- **committed offsets**, per group and partition.

A log entry is a data record or a transaction marker. A marker takes an
offset, as in Kafka, and no consumer ever receives it.

The model is one broker with one replica (ref:DEC-kafka-model-scope):

- The high watermark is the log end offset.
- The last stable offset is the first offset of the earliest open
  transaction in the partition, or the high watermark when none is open.
- The log start offset is zero; nothing is deleted.
- A group's coordinator assigns partitions, as in KIP-848 (ref:kip-848): the
  partitions of each topic go to its subscribers in turn, in member id order.
- A member leaves only by LeaveGroup; there is no session timeout yet.
- A topic has 1 to 1024 partitions. A name is 1 to 249 ASCII letters,
  digits, dots, underscores and dashes, and not
  `.` or `..`.

## Laws

Every law binds every lawful handler of its ability: the reference model,
the Kafka twin, and a real cluster. A law that needs a new topic creates it,
and claims nothing if the topic already exists. So the laws also hold on a
cluster that already holds data.

### Broker

- **Offsets in a partition are dense and increasing.** A batch produced after
  another starts where the other ends.
- **A batch reads back whole and in order.** Fetching from a batch's base
  offset gives its records, in the order produced.
- **An idempotent retry is written once.** A batch sent again with the same
  producer id, epoch and sequence gives the first write's offset, and adds
  nothing to the log (ref:kip-98).
- **A gap in sequence numbers is refused.** A producer's first batch must
  start at sequence 0; a later one where the last ended.
- **A new epoch fences the older producer.** Initialising a transactional id
  again bumps its epoch. Requests with the old epoch fail: transactional
  ones with `ProducerFenced`, produce with `InvalidProducerEpoch`.
- **Read-committed fetches never see open or aborted transactions.** While a
  transaction is open, its records are past the last stable offset. After
  it aborts, read-committed fetches skip them for ever. Read-uncommitted
  fetches see them.
- **A committed transaction is read whole and in order.**
- **Offsets sent in a transaction are committed only with it.** After an
  abort, the group's committed offset is unchanged; after a commit, it is
  the offset sent.
- **The coordinator assigns every partition exactly once.** After members
  join, each partition of the topic they subscribe to has one owner.
- **A member of an older generation cannot commit.** A new member starts a
  new generation. The old generation's commits and heartbeats fail with
  `IllegalGeneration`.
- **The stable offset never passes the high watermark.** And it is no later
  than the first offset of an open transaction.

### Admin

- **A new topic has the partitions asked for.**
- **A topic is created once.** Creating it again fails with
  `TopicAlreadyExists`.
- **A topic's partitions only grow.** Asking for the same count fails with
  `InvalidPartitions`; asking for more succeeds.
- **A new topic starts at offset zero.**
- **An invalid topic name is refused.** `""` and `..` fail with
  `InvalidTopic`.

### Partitioner

- **A chosen partition exists.** It is at least 0 and less than the count.
- **A key always goes to the same partition.**

The spec handler `firstPartition` always chooses 0. Kafka's own partitioner
hashes the key with murmur2 (ref:kafka-murmur2); LawSpec cannot read the
octets of `Bytes`, so that hash lives in the native handler.

### Producer

- **Records sent to one partition are read in the order sent.** The later
  send gets the next offset. A consumer reading from the first record gets
  both, first to last.
- **A flushed record is in the log.** After `flush`, the latest offset is
  past the record.
- **An idempotent producer's retried send is written once.**
- **Records with one key go to one partition.**
- **A send to a partition the topic lacks is refused.**

### Consumer

- **A restarted consumer resumes at its committed offset.** A consumer polls,
  commits and closes, then subscribes again. Its position is the committed
  offset, and every record it polls is at or after it. A record may come
  again; none is lost. This is at-least-once delivery.
- **A poll reads each partition in offset order.**
- **Seek sets the position.**
- **Only an assigned partition has a position.** Asking for another fails
  with `IllegalState`.
- **A read-committed consumer never sees aborted records.** It sees the
  records written outside the transaction.

### GroupCoordinator

- **Every partition has exactly one owner in a generation.**
- **A sync gives a member the partitions the group view shows.**
- **A commit from an older generation is refused.** A commit at the current
  generation succeeds.
- **A heartbeat from an older generation asks the member to rejoin.** When
  it rejoins with the same subscription, it learns the new generation and
  starts no other.
- **Leaving starts a new generation.** The member is then unknown.
- **A committed offset can be fetched.**

### Transactions

- **A newer instance fences a zombie.** Another instance initialises the
  same transactional id; the first one's next send fails with
  `ProducerFenced`.
- **Aborted records are never read committed.**
- **Committed records are read committed together and in order.** Before the
  commit, none is visible.
- **Offsets sent in an aborted transaction are not committed.**
- **A transaction must begin before it ends.** Committing without one fails
  with `IllegalState`.

## Running the laws

`contracts/kafka/test/` runs every law on Rust and Python; see [Run a
contract's laws](../how-to/run-contract-laws.md). Each law runs once per
lawful handler of its ability, 78 runs in all. The native handlers there
serve every ability from one shared reference cluster per case, with Kafka's
murmur2 partitioner (ref:DEC-contract-test-shared-cluster).
