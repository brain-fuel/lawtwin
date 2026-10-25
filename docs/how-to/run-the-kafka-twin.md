---
id: lawtwin.how-to.run-the-kafka-twin
kind: how-to
title: Run the Kafka twin and drive it from a test
---
# Run the Kafka twin and drive it from a test

This guide starts the Kafka twin, points an application at it, and uses
the control plane to force a state, raise a fault and check the contract's
laws. [The Kafka twin](../reference/kafka-twin.md) says what it answers;
[the control plane](../reference/control-plane.md) lists every command.

## Start it

With podman, from the image:

```sh
make image                  # builds lawtwin/kafka:0.2.1
podman run --rm -p 9092:9092 -p 9644:9644 lawtwin/kafka:0.2.1
```

With podman compose, from the example in `examples/kafka/compose.yaml`:

```sh
podman compose -f examples/kafka/compose.yaml up -d
```

Or as a program, from a checkout:

```sh
make build
target/release/lawtwin up kafka --seed 42
```

The twin listens for Kafka on 9092 and for the control plane on 9644. Point
the application's `bootstrap.servers` at `localhost:9092`. An application in
the same compose project reaches it by the service name: set `advertise` in
`TWIN_CONFIG` to `kafka:9092`.

## Force a state

Write the state the test needs, and force it:

```sh
cat > state.json <<'JSON'
{
  "topics": [{ "name": "orders", "partitions": 3 }],
  "records": [{ "topic": "orders", "partition": 0,
                "records": [{ "key": "o-1", "value": "placed" }, { "key": "o-1", "value": "paid" }] }],
  "committed": [{ "group": "billing", "topic": "orders", "partition": 0, "offset": 1 }]
}
JSON
lawtwin force state.json
```

A consumer in the group `billing` now starts at offset 1 of `orders-0`. A
state the contract does not allow is refused, and nothing changes:

```sh
echo '{ "topics": [{ "name": "orders", "partitions": 2 }] }' | lawtwin force -
# lawtwin: refused: the law `a topic's partitions only grow` forbids this state: orders has 3 partitions and cannot have 2
```

## Raise a fault

```sh
lawtwin faults                                   # what the twin file declares
lawtwin fault duplicate-delivery count=2         # the next two acknowledgements are lost
lawtwin fault broker-unavailable for=30s         # until the twin clock passes 30s
lawtwin advance 30s                              # ... which it does now
lawtwin fault rebalance-storm rounds=3 group=billing
lawtwin fault clear
```

From a test in any language, call the admin API directly:

```sh
curl -X POST localhost:9644/v1/faults/consumer-lag -d '{"records": 1, "delay": "200ms"}'
curl -X POST localhost:9644/v1/clock/advance -d '{"by": "10s"}'
```

## Check what happened

```sh
lawtwin inspect                  # topics, offsets, transactions, groups, committed offsets
lawtwin history > run.jsonl      # every operation, as the contract's ability calls
lawtwin laws                     # exit 1 if a law does not hold over the state or the history
lawtwin replay run.jsonl         # replay through a fresh model; exit 1 on a divergence
```

Run a test twice with the same seed (`lawtwin seed 42` starts again from an
empty state) and the same requests, and the two histories are the same,
byte for byte.
