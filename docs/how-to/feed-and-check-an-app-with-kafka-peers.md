---
id: lawtwin.how-to.feed-and-check-an-app-with-kafka-peers
kind: how-to
title: Feed and check an application with simulated Kafka peers
---
# Feed and check an application with simulated Kafka peers

An application that consumes a topic needs someone producing to it, and
one that produces needs someone checking what it writes. The
[simulated peers](kafka-peers.md) play both, against the Kafka twin or a
real broker.

## Describe the values

Write the type of the topic's values in LawSpec, with its refinements, as
`examples/kafka/orders.lawspec` does:

```lawspec
type Order is
  Order
    id :: (v :: Text where v matches regex "ord-[0-9]{6}")
    amount :: (v :: Int64 where v > 0 && v <= 1000000)
    currency :: Currency
end

type Currency is | EUR | USD | GBP end
```

## Feed a topic the application consumes

```sh
lawtwin up kafka --seed 1 &
lawtwin peer kafka produce --bootstrap localhost:9092 --topic orders \
  --value-type Order --spec examples/kafka/orders.lawspec \
  --count 500 --rate 100 --keys 20 --seed 7 --log orders.jsonl
```

The same seed sends the same records. To test how the application copes
with what real producers do, add faults: `--fault duplicate:0.05` resends
records, `--fault poison:0.01` sends values that are not orders,
`--fault tombstone:0.01` sends records with no value, and with
`--transactional-id feeder`, `--fault abort:0.2` aborts transactions and
`--fault hang` leaves the last one open. `orders.jsonl` says which records
showed which fault, and where each was written.

## Check what the application produces

```sh
lawtwin peer kafka consume --bootstrap localhost:9092 --topic payments --group audit \
  --value-type Order --spec examples/kafka/orders.lawspec --expect 500 --report audit.json
```

It reads committed, as a group member, and exits 1 when what the
application wrote breaks a law: a value that is not an `Order` (with the
path to what is wrong, such as `value.amount`), one key on two
partitions, records out of order, or, reading committed, a record of an
open or aborted transaction. `audit.json` lists each with its partition and
offset.

## Against real Kafka

Point `--bootstrap` at the broker; nothing else changes. The fidelity
crate's tests run both peers against the twin and, with
`make fidelity-test`, against `apache/kafka` in podman.
