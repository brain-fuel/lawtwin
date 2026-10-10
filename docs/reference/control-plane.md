---
id: lawtwin.reference.control-plane
kind: reference
title: The control plane
---
# The control plane

A test reaches what the real dependency will not do on demand through the
twin's control plane (ref:DEC-typed-control-plane): it forces a state, raises
a fault, moves the clock, and reads the state and the history. Every twin
has the same control plane, whatever it stands in for and whatever language
its core is generated in (Rust, Go, Java or Kotlin): a JSON API over HTTP on
a port of its own, and the `lawtwin` command that speaks it
(ref:DEC-control-plane-api). The API is described by an OpenAPI document,
[`control-plane.openapi.yaml`](control-plane.openapi.yaml).

The control plane belongs to LawTwin. The dependency's contract, a LawSpec
package, never mentions it (ref:DEC-three-planes); what a twin may be told
to do is declared beside the package, in its twin file.

## The twin file

Each contract that has a twin has a twin file, `lawtwin.json`, next to its
`lawspec-package.json` (ref:DEC-twin-file). It names the contract, the
ability and the spec handler the twin runs, declares the faults the twin can
show, and says what forcing a state refuses. For each fault it gives:

| Field | Says |
| --- | --- |
| `name` | the name a test raises it by |
| `summary` | what a client sees, in a sentence |
| `params` | each parameter: its name, type (`duration`, `int32`, `topic`, `group` or `api`), whether it is required, and its default |
| `changes` | what changes in the twin while it is active |
| `laws` | for each law the fault touches, whether it still holds and how; every law not named holds |

The twin refuses a fault the file does not declare, and a parameter the
fault does not take. A test of the twin checks that every law the file
names is a law of the contract, and that every declared fault is
implemented. [The Kafka twin](kafka-twin.md#faults) lists the Kafka
twin's faults with their effect on each law.

The HTTP twin, which follows, declares `latency`, `server-error` (a 5xx
status), `rate-limit` (429 with `Retry-After`) and `timeout` the same way.

## Commands

`lawtwin up <twin>` starts a twin. The other commands speak to a running
twin's admin API, at `--admin <url>` or `LAWTWIN_ADMIN`, by default
`http://127.0.0.1:9644`.

| Command | Admin API | Does |
| --- | --- | --- |
| `lawtwin up kafka` | | starts the Kafka twin (see below) |
| `lawtwin inspect [--records]` | `GET /v1/state` | shows the model's state, and with `--records` every log entry |
| `lawtwin force <state.json \| ->` | `POST /v1/state` | forces a state, or refuses it with the law that forbids it (exit 2) |
| `lawtwin fault <name> [<param>=<value>]...` | `POST /v1/faults/{name}` | raises a declared fault |
| `lawtwin fault clear [<name>]` | `DELETE /v1/faults[/{name}]` | clears one fault, or all |
| `lawtwin faults` | `GET /v1/faults` | lists the declared faults, the active ones, and what forcing refuses |
| `lawtwin advance <duration>` | `POST /v1/clock/advance` | moves the twin clock forward |
| `lawtwin history [--since <seq>] [--limit <n>] [--array]` | `GET /v1/history` | prints the history, as JSON lines unless `--array` |
| `lawtwin replay [<file> \| -]` | `POST /v1/history/replay` | replays a history through a fresh model; exit 1 on a divergence or a broken law |
| `lawtwin laws` | `GET /v1/laws` | checks the contract's laws over the state and the history; exit 1 if one does not hold |
| `lawtwin seed [<n>]` | `GET` or `POST /v1/seed` | shows the seed, or starts again from an empty state with a new one |
| `lawtwin reset` | `DELETE /v1/reset` | starts again from an empty state with the same seed |

`GET /v1/twin` describes the running twin: its contract, seed, clock,
bootstrap address and configuration. `GET /healthz` answers `ok`.

Durations are written `250ms`, `5s`, `2m` or `1h`.

### `lawtwin up kafka`

| Option | Default | Says |
| --- | --- | --- |
| `--host <addr>` | `127.0.0.1` | the address both listeners bind |
| `--port <n>` | `9092` | the Kafka port |
| `--admin-port <n>` | `9644` | the admin API's port |
| `--advertise <host:port>` | the Kafka listener's | the address clients are told to connect to |
| `--seed <n>` | `0` | the seed of every random choice |
| `--clock frozen\|wall` | `frozen` | how the twin clock moves |
| `--partitions <n>` | `1` | the partitions of a topic created without a count |
| `--no-auto-create` | | a Metadata request for an unknown topic does not create it |
| `--rebalance-wait <duration>` | `10s` | how long, in real time, a rebalance waits for members to rejoin |
| `--container [--image <image>]` | | runs the twin's image with podman instead |

In a container the same settings come from `TWIN_CONFIG`, a JSON object as
Ironframe passes it (`seed`, `clock`, `advertise`, `partitions`,
`autoCreateTopics`, `rebalanceWait`), or from `LAWTWIN_SEED`,
`LAWTWIN_CLOCK` and `LAWTWIN_ADVERTISE`. Options on the command line win.

## Forcing a state

`force` takes a forced state, a JSON document of what the test needs the
dependency to hold, and reaches it through the contract's own operations
(ref:DEC-force-through-operations). A state reached that way is one the
model can be in; a state that no sequence of operations reaches is refused,
with the law that forbids it, and nothing is applied:

```json
{ "forced": false, "law": "a topic's partitions only grow",
  "reason": "orders has 3 partitions and cannot have 2" }
```

A forced state is applied to a copy of the model. If the model refuses an
operation, or the result breaks an invariant the laws state, the copy is
dropped and the twin is as it was. Otherwise the copy becomes the twin's
state, and each operation is written to the history with the source
`force`, so a forced state replays like any other. [The Kafka
twin](kafka-twin.md#forcing-a-state) gives its forced-state document.

## Faults

A fault is raised by name, with parameters, and lasts until it is cleared,
until it is used up (such as `duplicate-delivery count=2`), or for a time
on the twin clock (`for=5s`). The twin stays lawful except where the twin
file says what changes, and every operation it performs while a fault is
active is still an operation of the contract: a fault changes what reaches
the model, when an answer is sent, or what the client is told, never what
the model answers. Raising, clearing and ending a fault are written to the
history.

## The clock and the seed

The twin clock is frozen by default: it moves only with `advance`
(ref:DEC-twin-clock). Session timeouts and faults raised for a time run on
it, so a test decides when a session expires or a fault ends. With
`--clock wall` the clock also follows real time. Waiting that protects a
client from a stuck connection (a fetch's long poll, a rebalance waiting
for members) runs in real time and changes no answer.

Every random choice (member ids, the cluster id) comes from one seeded
generator. With the same seed, the same clock mode and the same requests in
the same order, a twin writes the same history, byte for byte.

## The history

The history is every operation the twin performed, in order, written as
the call of the contract's ability it is, with the model's answer
(ref:DEC-history-as-ability-calls). As JSON lines:

```json
{"seq":4,"at":1767225600000,"source":"wire","op":"produceRequest","tp":{"topic":"orders","partition":0},"tag":null,"records":[{"key":"k","value":"v"}],"answer":{"ok":0}}
{"seq":5,"at":1767225600000,"source":"wire","op":"endTxnRequest","transactionalId":"tx","producerId":1,"producerEpoch":0,"commit":true,"answer":{"error":"InvalidTxnState"}}
{"seq":6,"at":1767225605000,"source":"fault","control":"fault raised","detail":{"id":1,"name":"consumer-lag","records":1,"delayMs":0,"topic":null}}
```

| Field | Is |
| --- | --- |
| `seq` | the entry's position, from 1 |
| `at` | the twin clock, in milliseconds since the Unix epoch |
| `source` | `wire` (a client's request), `force`, `fault`, `coordinator` (a session or rebalance that timed out) or `replay` |
| `op` and its arguments | the ability's operation, with its arguments by name |
| `answer` | `{"ok": result}`, or `{"error": "<KafkaError>"}` when the contract refused it |
| `control`, `detail` | a control-plane event instead of an operation |

Bytes are written as a string when they are printable UTF-8, and as
`{"base64": "..."}` otherwise. A read that a client repeats while it waits
(a long poll's probes) is written once, with the answer the client got.

## Replay and the laws

`replay` runs a history's operations, in order, through a fresh model and
reports every answer that differs from the recorded one; control entries
are skipped. A history from the twin replays with no divergence; a history
recorded elsewhere, or edited, shows where it parts from the contract.

`laws` checks the contract's laws over the twin's state and history. The
state checks are the invariants the laws state (offsets dense, the stable
offset at or below the high watermark, each partition one owner in a
group). The history checks mirror the laws over sequences of operations: a
batch is written at the log end, an idempotent retry gets its first
write's offset, a read-committed fetch never returns a record of an open or
aborted transaction, no partition has two owners in a generation, no older
generation or epoch is accepted after a newer one, and a committed offset
is fetched as committed. The client tests end every scenario with both.
