---
id: lawtwin.reference.kafka-peers
kind: reference
title: Simulated Kafka peers
---
# Simulated Kafka peers

The peers play the other parties on a topic (ref:DEC-simulated-peers): an
external producer feeding a topic the application consumes, and an
external consumer, a member of a group, reading what the application
produces and checking it. Both are real clients (librdkafka,
ref:librdkafka), so they work against [the Kafka twin](kafka-twin.md) and
against real Kafka alike. Each prints a JSON report, or writes it to
`--report <file>`.

## `lawtwin peer kafka produce`

```sh
lawtwin peer kafka produce --bootstrap localhost:9092 --topic orders \
  --value-type Order --spec examples/kafka/orders.lawspec \
  --count 1000 --rate 200 --keys 50 --seed 7 \
  --transactional-id feeder --txn-size 20 --fault abort:0.1 --fault poison:0.01
```

| Option | Default | Says |
| --- | --- | --- |
| `--bootstrap <host:port>` | `127.0.0.1:9092` | the broker or twin |
| `--topic <topic>` | | the topic; it must be a valid topic name |
| `--count <n>` | 10 | how many records |
| `--duration <d>` | | run this long instead of sending a count |
| `--rate <n>` | as fast as accepted | records a second |
| `--seed <n>` | 0 | the seed of every choice: keys, values, faults |
| `--keys <n>` | 8 | keys `key-0` to `key-<n-1>`, drawn from the seed |
| `--no-keys` | | records without keys |
| `--header <name>=<value>` | | a header every record carries; may be repeated |
| `--no-seq-header` | | leave out the `lawtwin-seq` header, which numbers the records from 0 |
| `--partition <n>` | the key's | one partition for every record |
| `--value-type <type>` with `--spec <path>` | | values of a LawSpec type, read from a `.lawspec` file or every one under a directory |
| `--format json\|lawspec` | `json` | how values are written (below) |
| `--transactional-id <id>` | | send in transactions of `--txn-size` records (10) |
| `--idempotent` | | an idempotent producer |
| `--fault <fault>[:<p>]` | | a fault, with the chance it shows on each record or transaction (1 when not given); may be repeated |
| `--late-by <d>` | `1h` | how far in the past a late record's timestamp is |
| `--log <file>` | | write every record sent, one JSON line each |

Without a value type, a value is the text `value-<n>-<8 hex digits>`.

| Fault | Shows |
| --- | --- |
| `duplicate` | a record sent twice, as a producer without idempotence resends one whose acknowledgement it lost |
| `poison` | a value that is not of the value type: eight bytes that are not UTF-8 |
| `tombstone` | a record with no value |
| `abort` | a transaction aborted instead of committed (needs `--transactional-id`) |
| `late` | a timestamp `--late-by` in the past |
| `hang` | the last transaction left open when the peer stops, as a producer that dies mid-transaction leaves it (needs `--transactional-id`) |

The records are valid under [the Kafka contract](kafka-contract.md): the
topic name is valid, every header has a name, and each is sent in a batch
of its own size. The report gives the records, how many were sent and
written, how many failed, the transactions committed, aborted and left
open, and the faults shown; the log gives each record's key, value, faults,
transaction and its outcome, and its partition and offset once
acknowledged. The command exits 1 if a record was not written.

## `lawtwin peer kafka consume`

```sh
lawtwin peer kafka consume --bootstrap localhost:9092 --topic payments --group audit \
  --value-type Order --spec examples/kafka/orders.lawspec --expect 500 --report audit.json
```

| Option | Default | Says |
| --- | --- | --- |
| `--bootstrap <host:port>` | `127.0.0.1:9092` | the broker or twin |
| `--topic <topic>` | | the topic |
| `--group <group>` | `lawtwin-peer` | the group it joins |
| `--isolation read_committed\|read_uncommitted` | `read_committed` | what it reads |
| `--value-type <type>` with `--spec <path>` | | check each value against a LawSpec type |
| `--format json\|lawspec` | `json` | how values are read |
| `--expect <n>` | | stop after this many records |
| `--duration <d>` | `60s` | stop after this long |
| `--idle <d>` | `3s` | stop after no record for this long, once any expected count is read |
| `--from earliest\|latest` | `earliest` | where a group with no committed offset starts |
| `--no-commit` | | do not commit the offsets read |
| `--log <file>` | | write every record read, one JSON line each |

It checks what it read against the contract's consumer-side laws and the
value type:

| Law | Broken when |
| --- | --- |
| a poll reads each partition in offset order | an offset is read after a later one, within one assignment; reading uncommitted, a record below the last one read was never read |
| records with one key go to one partition | a key is read from two partitions |
| read-committed fetches never see open or aborted transactions | reading committed, a record of an open or aborted transaction is read |
| committed records are read committed together and in order | reading committed, a committed record below the last one read was never read |
| every value is a `<type>` | a value does not decode as the type, or breaks a refinement; the detail gives the path, such as `value.lines[0].sku` |

A record read again after a rebalance is a redelivery, which at-least-once
delivery allows; the report counts redeliveries. To tell committed records
from those of open or aborted transactions, the peer reads each partition
it read from again over the wire, from the first offset it read, both read
committed and read uncommitted. The report gives the records, the
redeliveries, the invalid values, each partition's first and last offset,
every violation with its partition and offset, and the client errors it
recovered from. The command exits 1 when a law is broken.

## Value types

A peer reads a value type from LawSpec sources itself
(ref:DEC-peer-value-types):

- `type` declarations, products and sums with named fields, whose fields
  may be refined, as `amount :: (v :: Int64 where v > 0)`;
- `refinement` declarations without parameters, as
  `refinement Sku is (s :: Text where validSku s) end`;
- `definition`s whose body is one expression, which a refinement may call.

A predicate may use literals, the value and its fields, arithmetic,
comparisons, `&&`, `||`, `!`, `if`, `matches regex`, `prelude.length`,
`prelude.isEmpty`, `prelude.isPresent` and those definitions. Generated
values stay within the bounds a predicate's comparisons give (a value's
range, a text's or list's length, a regex), and a value is kept only when
its whole predicate holds. The type may be any type expression, such as
`List Order`. `examples/kafka/orders.lawspec` is an example.

| Type | JSON | LawSpec's encoding |
| --- | --- | --- |
| a product | an object of its fields | its fields in order |
| a sum | a constructor without fields as its name; with fields, `{"Name": {fields}}` | the constructor's position, then its fields |
| `Maybe a` | the value, or `null` | `0`, or `1` and the value |
| `Either a b` | `{"Left": v}` or `{"Right": v}` | `0` or `1`, then the value |
| `List a` | an array | the count, then each item |
| integers | a number | zigzag, then LEB128 |
| `Text` | a string | the UTF-8 length, then the UTF-8 |
| `Bytes` | base64 text | the length, then the bytes |
