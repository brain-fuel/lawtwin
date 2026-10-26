---
id: lawtwin.how-to.record-and-replay-kafka
kind: how-to
title: Record real Kafka traffic and replay it through a twin core
---
# Record real Kafka traffic and replay it through a twin core

This guide puts the recorder between an application and a real Kafka
broker, records a session, and replays it through a twin core to see
whether the twin answers as the broker did.

## Record

Start the recorder in front of the broker, and point the application at
it instead of the broker:

```sh
lawtwin record kafka --upstream localhost:39092 --listen 19092 --admin-port 19644 --out session
KAFKA_BOOTSTRAP=localhost:19092 ./run-the-app
```

The recorder says each divergence from the contract and each broken law
as it finds them. Stop it with Ctrl-C; it writes `session.history.jsonl`,
`session.frames.jsonl` and `session.summary.json`, prints the summary, and
exits 1 when the broker's answers diverged from the contract or a law
broke. While it runs, the [control plane](control-plane.md) works against
it, at the admin port:

```sh
lawtwin --admin http://127.0.0.1:19644 laws
lawtwin --admin http://127.0.0.1:19644 fault duplicate-delivery count=2
```

Start from fresh topics for a trace that replays exactly: records a topic
held before the recording are not in the trace.

## Replay

```sh
lawtwin replay kafka session --core rust
lawtwin replay kafka session --core image:lawtwin/kafka:0.3.0 --report replay.json
```

The core starts fresh, gets the recorded requests in the order the
recording allows, and answers them; the command prints the frames whose
answers differ and exits 1 if any do. A divergence in a group's SyncGroup
or Heartbeat answers is the group protocol
(ref:DEC-kafka-divergence-group-assignment); anything else is the twin
answering differently from the broker.

## Check that a replay catches a broken core

```sh
cargo build --release -p lawtwin-kafka --features mutant-ignore-dedupe --target-dir target/mutant
lawtwin replay kafka session --core target/mutant/release/lawtwin-kafka
```

A session with a retried idempotent batch (the `duplicate-delivery` fault
makes one) diverges at the retry: the broken core writes it again.
`make fidelity-test` does all of this against `apache/kafka` in podman,
with librdkafka.
