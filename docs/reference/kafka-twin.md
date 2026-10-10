---
id: lawtwin.reference.kafka-twin
kind: reference
title: The Kafka twin
---
# The Kafka twin

The Kafka twin is a Kafka broker that applications reach with their usual
client and configuration (ref:DEC-twins-wire-level-containers). What it
answers is decided by the Kafka contract, `lawtwin.kafka`
([its reference](kafka-contract.md)): every request is mapped onto
operations of the contract's `Broker` ability, run on the model LawSpec
generates from the contract's spec handler `cluster`, and the model's
answers are encoded back (ref:DEC-embed-generated-model). It has a core in
each LawSpec target language; the Rust core is the first
([the core interface](kafka-core.md), ref:DEC-core-per-target). It serves
LawTwin's [control plane](control-plane.md) on its admin port.

## What it answers

The twin speaks the Kafka protocol (ref:kafka-protocol) with the versions
below, which cover what librdkafka, the Java client, franz-go and kafkajs
negotiate (ref:DEC-kafka-wire-versions). A request outside them closes the
connection, as Kafka does.

| Request | Versions | Contract operations | What the wire adds |
| --- | --- | --- | --- |
| ApiVersions | 0–4 | none | this table; a version above 4 is answered at version 0 with UNSUPPORTED_VERSION |
| Metadata | 0–12 | `metadataRequest` for each topic asked for; `createTopicsRequest` for an unknown one when auto-creation is on | broker 1 at the advertised address leads every partition; a leader epoch; a topic id fixed by the name |
| CreateTopics | 2–7 | `createTopicsRequest` (on a copy of the model when `validate_only`) | the default partition count when none is given; the replication factor is not modelled |
| CreatePartitions | 0–3 | `createPartitionsRequest` | |
| Produce | 3–11 | `produceRequest` for each batch | v2 record batches in any compression (none, gzip, snappy, lz4, zstd), checked by CRC32C; the batch's producer id, epoch and base sequence as the producer tag; record timestamps; no response for `acks=0` |
| Fetch | 4–12 | `fetchRequest` for each partition | a long poll of up to the client's wait, at most 1 second, in real time; the records as one uncompressed v2 batch, with an empty batch past transaction markers and aborted records the fetch skipped |
| ListOffsets | 1–7 | `listOffsetsRequest` | `-2` is the earliest offset and `-1` the latest; a time finds the first record at or after it, by the twin's timestamps |
| FindCoordinator | 0–4 | none | broker 1 coordinates every group and transaction |
| JoinGroup | 0–9 | `joinGroupRequest`, and `leaveGroupRequest` for a member that does not rejoin | the classic protocol's rounds, below; member ids |
| SyncGroup | 0–5 | `syncGroupRequest` | the contract's assignment, in the consumer protocol's encoding |
| Heartbeat | 0–4 | `heartbeatRequest` | REBALANCE_IN_PROGRESS while a round waits for the member |
| LeaveGroup | 0–5 | `leaveGroupRequest` | |
| DescribeGroups | 0–5 | `describeGroupRequest` | the group's state and members' client ids |
| ListGroups | 0–4 | none; the groups the model holds | |
| OffsetCommit | 2–8 | `offsetCommitRequest` | |
| OffsetFetch | 1–7 | `offsetFetchRequest` for each partition | |
| InitProducerId | 0–4 | `initProducerIdRequest` | |
| AddPartitionsToTxn | 0–3 | `addPartitionsToTxnRequest` | |
| AddOffsetsToTxn | 0–3 | `txnOffsetCommitRequest` with no offsets | the producer is checked, and fenced, as for TxnOffsetCommit |
| TxnOffsetCommit | 0–3 | `txnOffsetCommitRequest` | |
| EndTxn | 0–3 | `endTxnRequest` | |
| DescribeConfigs | 1–4 | none | a topic's `cleanup.policy`, `message.timestamp.type` and `retention.ms`; nothing else |

A refusal of the contract is answered with Kafka's error code:

| `KafkaError` | Code |
| --- | --- |
| `UnknownTopicOrPartition` | 3 UNKNOWN_TOPIC_OR_PARTITION |
| `TopicAlreadyExists` | 36 TOPIC_ALREADY_EXISTS |
| `InvalidTopic` | 17 INVALID_TOPIC_EXCEPTION |
| `InvalidPartitions` | 37 INVALID_PARTITIONS |
| `InvalidRecord` | 87 INVALID_RECORD |
| `InvalidRequest` | 42 INVALID_REQUEST |
| `OffsetOutOfRange` | 1 OFFSET_OUT_OF_RANGE |
| `OutOfOrderSequenceNumber` | 45 OUT_OF_ORDER_SEQUENCE_NUMBER |
| `InvalidProducerEpoch` | 47 INVALID_PRODUCER_EPOCH |
| `ProducerFenced` | 90 PRODUCER_FENCED |
| `UnknownProducerId` | 59 UNKNOWN_PRODUCER_ID |
| `InvalidProducerIdMapping` | 49 INVALID_PRODUCER_ID_MAPPING |
| `InvalidTxnState`, `IllegalState` | 48 INVALID_TXN_STATE |
| `InvalidGroupId` | 24 INVALID_GROUP_ID |
| `UnknownMemberId` | 25 UNKNOWN_MEMBER_ID |
| `IllegalGeneration` | 22 ILLEGAL_GENERATION |

### What the wire keeps beside the model

The model holds the cluster's state; the wire keeps only what Kafka's
protocol needs and the contract does not state: each record's timestamp
(the contract's records carry none), each group member's client id,
protocols and session timeout, member ids handed out with
MEMBER_ID_REQUIRED, leader epochs, and the active faults. None of it
changes what the model answers.

### Consumer groups

The contract's coordinator assigns partitions itself, as in KIP-848
(ref:kip-848): a member joins with its topics, and the coordinator gives
each partition of a topic to its subscribers in turn, in member id order.
Clients speak the classic protocol (ref:kafka-classic-group-protocol), in
which every member rejoins before a new generation starts and a leader
proposes the assignment. The twin runs the classic protocol's rounds over
the contract's coordinator (ref:DEC-classic-groups-over-coordinator):

1. A new member, or one whose subscription changed, starts a round. The
   other members' heartbeats answer REBALANCE_IN_PROGRESS, and they rejoin.
   Commits at the current generation still succeed while the round waits.
2. When every member has rejoined, or after the rebalance wait (10 seconds
   of real time by default), the round completes: a member that did not
   rejoin leaves (`leaveGroupRequest`, source `coordinator`), and each one
   that did joins (`joinGroupRequest`). Every waiting member gets the new
   generation; the leader, the first member in id order, gets every
   member's metadata.
3. A sync answers with the contract's assignment, whatever the leader
   proposed. A member that rejoins unchanged into a stable group learns the
   current generation at once.

A member whose session times out on the twin clock leaves the group
(`leaveGroupRequest`, source `coordinator`). On the frozen clock that
happens only when a test advances it past the session timeout.

## Faults

The twin file `contracts/kafka/lawtwin.json` declares the faults
(ref:DEC-twin-file). Every law not named under a fault holds while it is
active.

| Fault | Parameters | What the client sees | Effect on the laws |
| --- | --- | --- | --- |
| `broker-unavailable` | `for` | every connection closes as soon as a request arrives, and new ones close at once | Nothing reaches the model. *A flushed record is in the log* holds later: a flush completes once the broker is back. |
| `leader-change` | `topic`, `partition` | the next produce, fetch or list-offsets request for each partition is answered NOT_LEADER_OR_FOLLOWER; Metadata shows a new leader epoch | The refused request never reaches the model, so *an idempotent retry is written once* and *an idempotent producer's retried send is written once* hold: the retry is the first write. |
| `consumer-lag` | `records` (1), `delay` (0ms), `topic`, `for` | a fetch returns at most `records` records per partition, after `delay` | *A batch reads back whole and in order*, *a committed transaction is read whole and in order* and *committed records are read committed together and in order* hold across fetches rather than in one; *a poll reads each partition in offset order* holds. |
| `duplicate-delivery` | `count` (1), `topic` | the next `count` produce requests are written, then answered REQUEST_TIMED_OUT, so the producer resends | *An idempotent retry is written once* and *an idempotent producer's retried send is written once* hold, and the fault exercises them. *Records sent to one partition are read in the order sent* changes for a producer without idempotence: its record is written twice, so a later send's offset is past both copies; order is kept. |
| `rebalance-storm` | `rounds` (3), `group` | every member is told to rejoin, `rounds` times; each round is a new generation | The coordinator takes the round's leader out and back in (`leaveGroupRequest`, `joinGroupRequest`, source `fault`). *Every partition has exactly one owner in a generation*, *the coordinator assigns every partition exactly once* and *leaving starts a new generation* hold in every generation; *a commit from an older generation is refused* holds; *a restarted consumer resumes at its committed offset* holds, and records after it may come again. |
| `request-latency` | `delay` (required), `api`, `for` | each response, or each of one request type, is held back by `delay` | All hold: only when an answer is sent changes. |

## Forcing a state

A forced state is a JSON object; each part is optional, and they are
reached in this order:

| Part | Reached by |
| --- | --- |
| `reset: true` | starting from an empty cluster |
| `topics: [{name, partitions}]` | `createTopicsRequest`, or `createPartitionsRequest` to grow one |
| `records: [{topic, partition, records: [{key, value, headers}]}]`, or `logEndOffset` instead of `records`, and `offset` to say where they must start | `produceRequest` |
| `producers: [{transactionalId, epoch}]` | `initProducerIdRequest` until the epoch is reached |
| `transactions: [{transactionalId, records, group, offsets, end: open\|commit\|abort}]` | `initProducerIdRequest`, `addPartitionsToTxnRequest`, tagged `produceRequest`s, `txnOffsetCommitRequest`, `endTxnRequest` |
| `groups: [{group, members: [{member, topics}], assignment}]` | `joinGroupRequest` for each member; `assignment`, if given, must be the coordinator's |
| `committed: [{group, topic, partition, offset}]` | `offsetCommitRequest` |

A forced member is a member like any other: it leaves when its session
times out on the twin clock, or when a round finds it did not rejoin. A
state no sequence of operations reaches is refused with the law that
forbids it:

| Law | Refuses |
| --- | --- |
| an invalid topic name is refused | a name that is empty, longer than 249, `.` or `..`, or has a character other than ASCII letters, digits, `.`, `_` and `-` |
| a new topic has the partitions asked for | fewer than 1 or more than 1024 partitions |
| a topic's partitions only grow | fewer partitions than the topic has |
| a send to a partition the topic lacks is refused | records or offsets for a partition that does not exist |
| offsets in a partition are dense and increasing | records at an offset other than the log end, or a log end below the current one |
| a new epoch fences the older producer | an epoch below the current one, or above 32767 |
| every partition has exactly one owner in a generation | an assignment with a partition owned twice, or not at all |
| the coordinator assigns every partition exactly once | an assignment other than the coordinator's |
| a transaction must begin before it ends | a transaction that ends with nothing in it |

## Clients

The core conformance kit (ref:DEC-core-conformance-kit) runs six scenarios
with each client against a fresh twin, and then checks the contract's laws
over the history and replays it through the reference model:

| Client | Version | Produce and consume | Group rebalance | Idempotent producer | Transactions, read committed | Faults | Forced state |
| --- | --- | --- | --- | --- | --- | --- | --- |
| librdkafka (ref:librdkafka), through the rdkafka crate (ref:rdkafka-crate) | 2.12.1 | passes | passes | passes | passes | passes | passes |
| kafkajs (ref:kafkajs) | 2.2.4 | passes | passes | passes | passes | passes | passes |
| franz-go (ref:franz-go) | 1.22.1 | passes | passes, with its default cooperative-sticky balancer | passes | passes | passes | passes |
| The Java client (ref:kafka-java-client) | 4.3.1 | passes | passes | passes | passes | passes | passes |

The scenarios are: producing 30 keyed records and reading them back in
order with a group; two members splitting four partitions and one taking
them back when the other leaves; an idempotent producer writing a lost
acknowledgement once and a plain one twice; committed, aborted and
committed transactions read committed and uncommitted, then a zombie
fenced by a second instance; each declared fault observed by the client;
and a consumer resuming at a forced committed offset after an impossible
state is refused.

## Throughput

The Rust core answers librdkafka at about two million records a second
produced, in batches, and half a million a second read back by a group:
a million 100-byte records to eight partitions in 0.44 s, read in 1.8 s, on
an Apple M-series laptop (`cargo test --release -p lawtwin-kafka-conformance
--test throughput -- --ignored --nocapture`). Each batch is one operation of
the model; a fetch costs what it returns, not the size of the log.

## Differences from Kafka

The twin is the contract's model, which is one broker with one replica
(ref:DEC-kafka-model-scope). Beyond that:

- Topics cannot be deleted, logs are never truncated or compacted, and
  timestamps are always the producer's (CreateTime).
- A transaction has no timeout: it stays open until it ends, or until its
  transactional id is initialised again.
- Fetch sessions are not kept (each fetch is a full fetch), and the
  KIP-848 consumer protocol and transactions version 2 are not offered;
  clients fall back to the classic protocols.
- Fetched batches carry no producer ids and no aborted-transaction list:
  the model has already left out what a read-committed consumer must not
  see.
- A group's assignment is the contract's, whatever assignor the clients
  name; cooperative clients see it as revocations and rejoin.
- One advertised address serves every client; a compose project picks
  `localhost:9092` or the service name.
- No authentication, TLS or quotas.

Conformance against the `apache/kafka` image (ref:apache-kafka-image)
records each divergence in the ledger as it is found.
