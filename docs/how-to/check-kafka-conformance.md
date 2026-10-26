---
id: lawtwin.how-to.check-kafka-conformance
kind: how-to
title: Check the Kafka twin's conformance against real Kafka
---
# Check the Kafka twin's conformance against real Kafka

A twin is trusted as far as it is checked against the real thing
(ref:DEC-fidelity). This guide runs the core conformance kit's client
scenarios against a real Kafka broker and reads the report.

## Against apache/kafka in podman

With podman running, node, go, mvn and java installed:

```sh
make conform-kafka
```

It builds `lawtwin` and `lawtwin-kafka-fidelity`, starts
`apache/kafka:4.3.1` on port 39092, runs every client and scenario against
it and against the Rust core, writes the report to
`twins/kafka/conformance/reports/0.1.0/`, and stops the broker. It takes
about ten minutes. `KAFKA_IMAGE`, `KAFKA_PORT` and `RELEASE` choose
another image, port or release:

```sh
make conform-kafka KAFKA_IMAGE=docker.io/apache/kafka:4.1.0 RELEASE=0.1.1
```

## Against a broker of your own

The scenarios create topics, groups and transactional ids with a prefix of
their own, so they share a broker without meeting anything on it. The
broker needs transactions and its internal topics on as many replicas as it
has brokers; the recorder speaks for one broker.

```sh
lawtwin conform kafka --real kafka.test:9092 --real-name "our staging Kafka" \
  --client librdkafka --client java --out conformance --frames
```

`--no-twin` leaves out the runs against the twin; `--twin image:lawtwin/kafka:0.3.0`
checks the image instead of the Rust core in process.

## Read the report

`report.md` gives the level, a table of every client and scenario on each
side, each kind of divergence with the decision that accounts for it, and
what the wire showed beside the answers. Look at, in order:

1. **A broken law against the real broker.** The contract says something
   real Kafka does not do: the contract is wrong, or the decoder misreads
   the wire. Find the entry by its `seq` in
   `traces/<client>/<scenario>.history.jsonl`.
2. **A scenario that fails against the real broker and passes against the
   twin.** The twin lets an application do what Kafka does not.
3. **A divergence or note with no decision.** A difference nobody has
   decided about: decide whether the twin follows Kafka or the contract,
   and write the decision in `canonical_decisions.yaml`. Classify its kind
   in `twins/kafka/fidelity/src/conform.rs` once it is decided.

A trace kept with `--frames` replays through any core
([replay](kafka-replay.md)):

```sh
lawtwin replay kafka conformance/traces/java/transactions --core rust
```
