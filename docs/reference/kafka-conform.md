---
id: lawtwin.reference.kafka-conform
kind: reference
title: Conformance of the Kafka twin against real Kafka
---
# Conformance of the Kafka twin against real Kafka

`lawtwin conform kafka` runs the
[core conformance kit](kafka-core.md#the-conformance-kit)'s client
scenarios against a real broker, through [the recorder](kafka-record.md),
and the kit's checks over what the recorder decoded: the contract's laws
over the history and the state, and the reference model run over the
history, whose answers must be the broker's up to renaming
(ref:DEC-compare-up-to-renaming). The same scenarios run against a twin
core beside them. It writes a report with a level for the twin release
(ref:DEC-conformance-levels), and each divergence it finds is a ledger
decision (ref:DEC-fidelity).

```sh
make conform-kafka
lawtwin conform kafka --real localhost:39092 --real-name apache/kafka:4.3.1 --out report
```

| Option | Default | Says |
| --- | --- | --- |
| `--real <host:port>` | | the real broker |
| `--real-name <name>` | `a Kafka broker` | what it is, for the report |
| `--client <name>` | all four | `librdkafka`, `kafkajs`, `franz-go` or `java`; may be repeated |
| `--scenario <name>` | all six | `produce-consume`, `group-rebalance`, `idempotent`, `transactions`, `faults` or `force`; may be repeated |
| `--out <directory>` | `conformance-kafka-<release>` | where the report and traces go |
| `--twin <core>` | the Rust core, in process | the core run beside: a command or `image:<image>` |
| `--no-twin` | | run against the real broker only |
| `--release <version>` | the tool's | the twin release the report is for |
| `--frames` | | keep each run's frames beside its history, so its trace replays |
| `--clients <directory>` | the kit's | where the client programs are |

A client whose toolchain (node, go, mvn and java) is missing is skipped,
and the report says so. It exits 1 when a law does not hold over a real
history, or the twin fails its own checks.

`make conform-kafka` starts `apache/kafka` (ref:apache-kafka-image)
version 4.3.1 in podman with `tools/real-kafka.sh` (one KRaft node with
transactions, `KAFKA_PORT`, 39092 by default), runs the command with
`--frames` for the release `RELEASE` (0.3.0), and stops the broker. The
broker listens on the same port inside its container and out, so it
reaches its own advertised address, which Kafka's transaction
verification (KIP-890, ref:kip-890) needs.

## Each run

For each client and scenario:

1. a fresh recorder starts in front of the broker, with a frozen clock;
2. the client runs the scenario against it, with `LAWTWIN_NS` set to a
   namespace of its own (`conform-<run>-<client>-<scenario>-`), which
   every topic, group and transactional id it uses begins with, so the
   scenarios share one broker without meeting;
3. the scenario's faults, forced states and clock reach the broker
   through [the recorder's control plane](kafka-record.md#its-control-plane);
4. the run passes when the client's own checks pass; the recorder's
   divergences and the laws broken over its history and state are the
   run's;
5. the same scenario runs against a fresh twin core, and the core's
   history is replayed through the reference model and its laws checked.

## The levels

A report's level is the highest whose conditions every run meets:

| Level | Name | Means |
| --- | --- | --- |
| 0 | unchecked | no run against the real dependency |
| 1 | lawful | the real dependency's histories satisfy every law the kit checks: the contract describes the real thing |
| 2 | scenarios | also, every client scenario that passes against the twin passes against the real dependency, unchanged |
| 3 | answers | also, every answer of the real dependency is the reference model's up to renaming, except divergences, and notes the wire showed, each accounted for by a ledger decision |
| 4 | exact | level 3 with no divergence at all |

## The report

`<out>/report.json`, `<out>/report.md`, and for each run
`<out>/traces/<client>/<scenario>.history.jsonl` (and `.frames.jsonl` with
`--frames`), a trace that [replays](kafka-replay.md) through any core.

| Field | Is |
| --- | --- |
| `twin`, `release`, `contract` | what the report is for |
| `real` | the real dependency, its bootstrap, and what its ApiVersions answer says of it |
| `twinCore` | the core run beside |
| `level`, `levels` | the level reached, and every level |
| `summary` | runs, passes on each side, whether each side's histories are lawful, operations recorded, divergences |
| `clients` | the clients with their pinned versions, and why one was skipped |
| `divergenceKinds` | each kind of divergence: its count, the decision that accounts for it, and up to three examples |
| `wireNotes` | each kind of note the recorder wrote, by request and error, with the decision that accounts for it |
| `runs` | each run against the real broker and the twin: passed, the client's output, operations, broken laws, divergences, notes and time |

A divergence is one operation whose answer from the broker is not the
model's: its `seq`, the operation with its arguments, the broker's answer
and the model's. Its kind is named from the operation and the two answers;
the kinds a decision accounts for are:

| Kind | Decision |
| --- | --- |
| assignment by the client's assignor | ref:DEC-kafka-divergence-group-assignment |
| group view under the client's assignor | ref:DEC-kafka-divergence-group-assignment |

| Note | Decision |
| --- | --- |
| retriable error | ref:DEC-kafka-divergence-coordinator-timing |
| rebalance in progress | ref:DEC-kafka-divergence-rebalance-heartbeat |

## Kafka twin 0.1.0

The report of release 0.1.0 is
`twins/kafka/conformance/reports/0.1.0/report.md`, against
`apache/kafka:4.3.1`. It reaches **level 3, answers**:

- all 24 runs (four clients, six scenarios) pass against the real broker
  and against the Rust core;
- the real broker's histories, 1053 operations, satisfy every law the kit
  checks;
- 8 divergences, all a group's assignment, where Kafka's classic protocol
  lets the client's assignor choose (ref:DEC-kafka-divergence-group-assignment);
- 27 notes: 16 heartbeats answered REBALANCE_IN_PROGRESS
  (ref:DEC-kafka-divergence-rebalance-heartbeat), and 11 retriable errors
  while coordinators loaded, transactions completed or a new topic's leader
  was elected (ref:DEC-kafka-divergence-coordinator-timing).

Conformance found one more divergence, which the twin now follows: Kafka
answers a fenced producer INVALID_PRODUCER_EPOCH on the request versions
librdkafka and kafkajs send (ref:DEC-fenced-producer-spelling). Kafka also
creates a topic a Metadata request names a moment later than the twin
does (ref:DEC-kafka-divergence-auto-create).

The recorded traces replay through the Rust core with no divergence,
except the group rebalances, which differ only in the group protocol; the
fidelity crate's test `committed_traces` checks it with no broker.
