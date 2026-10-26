---
id: lawtwin.how-to.check-a-kafka-core
kind: how-to
title: Check a Kafka core with the conformance kit
---
# Check a Kafka core with the conformance kit

Every core of the Kafka twin, in any language, must pass the same
conformance kit ([Kafka twin cores](../reference/kafka-core.md) says what it
checks). This guide runs it.

## What you need

Rust 1.85 or later, with CMake to build librdkafka; Node 22 or later for
kafkajs; Go 1.22 or later for franz-go; a JDK 17 or later with Maven for the
Java client; and podman to check an image. The kit fails when one of them is
missing, so that no client is skipped by accident.

## Check the Rust core

```sh
make core-conformance
```

This builds `target/release/lawtwin-kafka` and checks it. Check the image
instead with:

```sh
make image
make core-conformance CORE=image:lawtwin/kafka:0.3.0
```

## Check another core

Build the core, then name its program. The kit starts it once for each
scenario with the core interface's options (`--host`, `--port`,
`--admin-port`, `--seed`, `--clock frozen`, and `--advertise` for the
golden session), so the program must take them:

```sh
make core-conformance CORE=twins/kafka/go/lawtwin-kafka-go
make core-conformance CORE=image:lawtwin/kafka-go:0.1.0
```

A core that answers a golden frame differently fails with the frame, its
request type and version, and both responses in hex. A core that writes a
different history fails with the first line that differs.

## Change the twin on purpose

When a change to the twin changes its answers (a new request version, a
new field), record the golden session again from the Rust core and check
the difference into the repository with the change:

```sh
make golden
git diff twins/kafka/shared/golden/
```

Every other core then has to give the new bytes too.
