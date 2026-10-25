---
id: lawtwin.reference.kafka-core
kind: reference
title: Kafka twin cores
---
# Kafka twin cores

A twin's core is the server: the model LawSpec generates for one target
language, embedded behind the dependency's wire protocol and LawTwin's
control plane. Every twin gets a core in every LawSpec target language,
each with its own image or native deployment (ref:DEC-core-per-target).
Everything that is not the core is written once, for every core: the
[control plane](control-plane.md) and its OpenAPI document, the twin file's
fault declarations, the history format, the `lawtwin` command, the golden
wire session, the conformance kit, and these pages.

| Core | Directory | Deploys as | Status |
| --- | --- | --- | --- |
| Rust | `twins/kafka/rust/` | the `lawtwin-kafka` program; the image `lawtwin/kafka:0.2.1` | passes the conformance kit |
| Go | `twins/kafka/go/` | a program and an image | next |
| JVM (Java and Kotlin) | `twins/kafka/jvm/` | a jar and an image | next |
| Python, JavaScript and TypeScript, Haskell | `twins/kafka/<language>/` | a program or package and an image | later |
| BEAM (Erlang, Elixir, Gleam) | `twins/kafka/beam/` | an OTP release, which may run as an Erlang cluster | later |
| C#, F# and the other targets | `twins/kafka/<language>/` | a program and an image | later |

## The repository

```text
contracts/kafka/              the contract (LawSpec) and its twin file, lawtwin.json
twins/kafka/shared/golden/    the golden session every core's codec and conformance tests read
twins/kafka/conformance/      the core conformance kit
twins/kafka/rust/             the Rust core, with its Containerfile
twins/kafka/<language>/       each other core
crates/lawtwin/               the lawtwin command, which drives any core
docs/reference/               the control plane, its OpenAPI document, these pages
```

## The core interface

Every core is started the same way, and the conformance kit, the `lawtwin`
command and Ironframe rely on nothing else:

```text
<core> [--host <addr>] [--port <n>] [--admin-port <n>] [--advertise <host:port>]
       [--seed <n>] [--clock frozen|wall] [--partitions <n>] [--no-auto-create]
       [--rebalance-wait <duration>]
```

| Setting | Default | Environment |
| --- | --- | --- |
| host | `127.0.0.1`; an image uses `0.0.0.0` | |
| Kafka port | `9092` | |
| admin port | `9644` | |
| advertised address | the Kafka listener's, with `localhost` for an address that listens on every interface | `LAWTWIN_ADVERTISE`, or `advertise` in `TWIN_CONFIG` |
| seed | `0` | `LAWTWIN_SEED`, or `seed` |
| clock | `frozen` | `LAWTWIN_CLOCK`, or `clock` |
| partitions of an auto-created topic | `1` | `partitions` |
| auto-created topics | on | `autoCreateTopics` |
| rebalance wait | `10s` | `rebalanceWait` |

`TWIN_CONFIG` is a JSON object, as Ironframe passes a twin's
configuration. The command line wins over the environment.

A core serves the Kafka protocol as [the Kafka twin](kafka-twin.md)
describes it, and the control plane on the admin port as
[`control-plane.openapi.yaml`](control-plane.openapi.yaml) describes it.

## Determinism

With the same seed, a frozen clock and the same requests in the same order,
every core writes the same history and the same responses, byte for byte.
So that cores agree, each makes its choices the same way:

- **Randomness** is SplitMix64, seeded with the seed: add
  `0x9e3779b97f4a7c15` to the state, then mix with `0xbf58476d1ce4e5b9`
  (shift 30) and `0x94d049bb133111eb` (shifts 27 and 31).
- **The cluster id** is `lawtwin-` and the first 22 hex digits of the first
  two draws, each written as 16 lower-case hex digits.
- **A member id** is the client id (`member` when empty), `-`, and the next
  draw as 16 lower-case hex digits.
- **A topic id** is fixed by the topic's name. Over its bytes `b`, `h1` is
  64-bit FNV-1a (start `0xcbf29ce484222325`; `h1 = (h1 xor b) * 0x100000001b3`)
  and `h2` starts at `0x84222325cbf29ce4` with
  `h2 = rotate_left((h2 xor b) * 0x100000001b3, 5)`, all wrapping. The id is
  the UUID whose bytes are `h1` then `h2 or 1`, big-endian.
- **The twin clock** starts at 2026-01-01T00:00:00Z (1767225600000 ms).
- **A record without a timestamp** takes the twin clock's time.
- **The history** is written as `control-plane.md` describes, with fields in
  the order shown there.

## The conformance kit

`make core-conformance CORE=<core>` checks a core, driving it only over the
Kafka protocol and the admin API (ref:DEC-core-conformance-kit). `CORE` is a
command, given the core interface's options (by default the Rust core,
`target/release/lawtwin-kafka`), or `image:<image>`, run with podman. The
kit:

1. runs the six client scenarios with librdkafka, kafkajs, franz-go and the
   Java client, each against a fresh core (the same scenarios run against
   real Kafka in [conformance](kafka-conform.md), each in a namespace a
   scenario reads from `LAWTWIN_NS`);
2. after each, replays the core's history through the reference model
   (the Rust code LawSpec generates from the contract) and fails on any
   answer that differs, checks the contract's laws over the history and the
   replayed state itself, and checks that the core's own `/v1/laws` agrees;
3. checks faults, forced states and the clock as clients see them;
4. runs the scripted session twice with one seed and once with another: the
   same seed must give the same history, byte for byte, and another seed
   other member ids;
5. sends the golden session's requests, byte for byte, and compares every
   response and the history with the golden ones.

The golden session, `twins/kafka/shared/golden/session-42.*`, is the
scripted session recorded on the Rust core with seed 42, a frozen clock and
the advertised address `lawtwin.golden:9092`: every kind of request the twin
answers, in each compression, with their responses as hex
(`session-42.frames.jsonl`) and the history after it
(`session-42.history.jsonl`). Because it fixes the bytes, it fixes every
choice a core makes in encoding its answers. Each core's codec tests also
decode and re-encode its frames. `make golden` records it again, from the
Rust core, when the twin changes on purpose.
