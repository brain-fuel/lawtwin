---
id: lawtwin.reference.kafka-record
kind: reference
title: Recording a Kafka broker
---
# Recording a Kafka broker

`lawtwin record kafka` is a Kafka protocol proxy in front of a real broker.
Clients connect to it as to a broker; it forwards each request to the
broker and each response back, keeps every exchange as a frame, and
decodes the frames into the history every twin writes: the contract's
operations with the real broker's answers (ref:DEC-recorded-history). As
it goes, it runs each operation on the reference model and compares the
answers up to renaming (ref:DEC-compare-up-to-renaming), and checks the
contract's laws over the history. It serves LawTwin's
[control plane](control-plane.md), so tests written for a twin run against
the real broker through it (ref:DEC-recorder-control-plane).

```sh
lawtwin record kafka --upstream localhost:39092 --listen 19092 --admin-port 19644 --out session
```

| Option | Default | Says |
| --- | --- | --- |
| `--upstream <host:port>` | | the real broker |
| `--listen <[host:]port>` | `9092` | where clients connect; the host is `127.0.0.1` when not given |
| `--admin-port <[host:]port>` | `9644` | where the control plane listens |
| `--advertise <host:port>` | the listener's | the address clients are told for the broker |
| `--out <prefix>` | | write the trace as it is recorded, as `<prefix>.history.jsonl` and `<prefix>.frames.jsonl` |
| `--clock wall\|frozen` | `wall` | how the recorder's clock moves: faults raised for a time end on it |
| `--quiet` | | do not say each divergence and broken law as it is found |

It runs until it is interrupted, then writes `<prefix>.summary.json` and
prints it: the frames, the operations, every divergence, every broken law
and the notes by kind. It exits 1 when an answer diverged or a law broke.
The program behind it is `lawtwin-kafka-fidelity`, beside `lawtwin`
(`make build` builds both).

## What it changes on the wire

So that a trace replays through any core and every frame decodes, the
recorder offers clients only what the twin answers
(ref:DEC-recorder-negotiates-twin-versions):

- its ApiVersions answer is the broker's, cut to the requests and versions
  of [the Kafka twin](kafka-twin.md#what-it-answers), with no feature
  levels, so clients use the classic group protocol and transactions
  version 1;
- it sends every fetch without a fetch session (KIP-227, ref:kip-227);
- Metadata and FindCoordinator answers name the recorder, at its
  advertised address, as the broker, so every connection passes through it.
  It speaks for one broker.

The frames keep the requests as the broker got them.

## The trace

A trace is two files with one prefix:

| File | Holds |
| --- | --- |
| `<prefix>.history.jsonl` | the history, as [the control plane](control-plane.md#the-history) writes it |
| `<prefix>.frames.jsonl` | one frame per line |
| `<prefix>.summary.json` | written when the recorder stops |

A frame is:

| Field | Is |
| --- | --- |
| `n` | its place in the order the requests were sent, from 1 |
| `conn` | the client connection, from 1; 0 for the recorder's own, for a forced state |
| `api`, `key`, `version` | the request type, its API key and version |
| `request` | the request as the broker got it, header and body, in hex |
| `response` | the broker's response, header and body, in hex; absent for a produce with `acks=0` |
| `delivered` | what the client got instead, when a fault changed the answer |
| `sentUs`, `answeredUs` | when the request was sent and the response came, in microseconds from the start |
| `source` | `wire`, or `force` for a forced state |

## How requests become operations

The decoder mirrors how [the Kafka twin](kafka-twin.md) maps requests onto
the contract's `Broker` operations, so a recorded history and a twin's say
the same thing in the same words:

| Request | Written as |
| --- | --- |
| Metadata | `metadataRequest` for each topic in name order; a topic the request names and the broker creates, as the twin writes an auto-creation; a topic the broker already held, as its creation with the source `observed`; internal topics are left out |
| CreateTopics, CreatePartitions | `createTopicsRequest`, `createPartitionsRequest` |
| Produce | `produceRequest` for each batch, with its producer tag and records |
| Fetch | `fetchRequest` for each partition, with the records a consumer reads: markers, records below the fetch offset and, read committed, records of aborted transactions are left out; its maximum is the number of records returned (ref:DEC-fetch-sizing) |
| ListOffsets | `listOffsetsRequest` for the earliest and the latest offset; a lookup by time is not an operation |
| JoinGroup | when the leader's answer names the round's members: `leaveGroupRequest` (source `coordinator`) for a member that did not rejoin, and `joinGroupRequest` for each member; an unchanged member's rejoin, `joinGroupRequest` |
| SyncGroup, Heartbeat, LeaveGroup, DescribeGroups | `syncGroupRequest`, `heartbeatRequest`, `leaveGroupRequest` for a member the coordinator counts, `describeGroupRequest` |
| OffsetCommit, OffsetFetch | `offsetCommitRequest`, `offsetFetchRequest` for each partition |
| InitProducerId, AddPartitionsToTxn, AddOffsetsToTxn, TxnOffsetCommit, EndTxn | `initProducerIdRequest`, `addPartitionsToTxnRequest`, `txnOffsetCommitRequest` (with no offsets for AddOffsetsToTxn), `endTxnRequest` |
| ApiVersions, FindCoordinator, ListGroups, DescribeConfigs | nothing |

A Kafka error code the contract states is its `KafkaError`; Kafka's
INVALID_PRODUCER_EPOCH on the requests and versions that spell a fenced
producer so is `ProducerFenced` (ref:DEC-fenced-producer-spelling). Other
codes are written by Kafka's name, such as `REBALANCE_IN_PROGRESS`.

What the wire shows that no operation states is a note in the history,
which replay and the laws skip:

| Note | When |
| --- | --- |
| `retriable error` | an error a client retries, such as COORDINATOR_LOAD_IN_PROGRESS or CONCURRENT_TRANSACTIONS (ref:DEC-retriable-errors-not-answers) |
| `rebalance in progress` | a heartbeat answered REBALANCE_IN_PROGRESS (ref:DEC-kafka-divergence-rebalance-heartbeat) |
| `stale generation` | a JoinGroup answer with a generation older than the group's |
| `unacknowledged produce` | a produce with `acks=0` |
| `unmodelled request`, `undecodable frame`, `undecodable batch` | a request the contract does not model, or one that does not decode |
| `fault applied` (source `fault`) | the recorder applied a fault to the exchange |

## Its control plane

The recorder serves the [control plane](control-plane.md) over a shadow
twin: its history is the recorded one, its state the reference model run
over it, its faults and clock the recorder's.
`GET /v1/recording` gives the summary so far.

| Command | Against the real broker |
| --- | --- |
| `inspect`, `history`, `laws`, `replay`, `faults` | as on a twin, over the recording |
| `fault <name>` | applied on the wire, below |
| `advance <duration>` | moves the recorder's clock, ending faults raised for a time |
| `force <state>` | checked on a copy of the reference model and refused with the law that forbids it, as on a twin; otherwise reached by sending the broker the contract's operations, recorded with the source `force`. Only topics, records and the committed offsets of an empty group can be reached on a real broker; producers, transactions and groups are refused (501) |

| Fault | On the wire |
| --- | --- |
| `broker-unavailable` | connections close as a request arrives, and new ones at once |
| `leader-change` | a produce, fetch or list-offsets request for only the partitions it names is answered NOT_LEADER_OR_FOLLOWER by the recorder |
| `consumer-lag` | fetch answers are cut to `records` records per partition and held back by `delay`; the history has the fetch the client got |
| `duplicate-delivery` | an acknowledgement of a written batch is replaced by REQUEST_TIMED_OUT, so the producer resends |
| `rebalance-storm` | the leader's heartbeat is answered REBALANCE_IN_PROGRESS, and the group rebalances for real, `rounds` times |
| `request-latency` | answers are held back by `delay` |
