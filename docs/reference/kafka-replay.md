---
id: lawtwin.reference.kafka-replay
kind: reference
title: Replaying a Kafka trace through a core
---
# Replaying a Kafka trace through a core

`lawtwin replay kafka` sends a [recorded trace](kafka-record.md#the-trace)'s
requests to a fresh twin core over the Kafka protocol, decodes the core's
responses as it decoded the broker's, and reports every frame whose
answers differ (ref:DEC-replay-over-the-wire). It checks the core's wire
layer as well as its model.

```sh
lawtwin replay kafka session --core rust
lawtwin replay kafka session.frames.jsonl --core image:lawtwin/kafka:0.3.0 --report replay.json
```

| Option | Says |
| --- | --- |
| `<trace>` | the trace's prefix, or one of its files |
| `--core <core>` | the core: `rust` for the Rust core in the replaying process, a command given [the core interface](kafka-core.md#the-core-interface)'s options, or `image:<image>`, run with podman |
| `--report <file>` | write the report there; otherwise it is printed |

`lawtwin replay <trace> --core <core>`, without `kafka`, is the same
command; without `--core`, `lawtwin replay` replays a history through a
running twin's admin API ([the control plane](control-plane.md)).

## How it replays

- The core starts with seed 0 and a frozen clock. Topics the broker already
  held when the recording started are forced on it first.
- Each recorded connection gets a connection of its own. A request is sent
  once every response that came back before it in the recording has come
  back in the replay; a request that waits more than 30 seconds is sent
  anyway and reported as stalled.
- Requests carry the core's identifiers: member ids, generations, producer
  ids and epochs, learned from the core's answers as they come, are
  written into the requests, and record batches get their checksums again
  (ref:DEC-compare-up-to-renaming).

## How it compares

Both sides' responses are decoded into the history
([how requests become operations](kafka-record.md#how-requests-become-operations)),
the recorded side renamed to the core's identifiers, and compared frame by
frame:

- operations and answers must be equal; two refusals are equal when Kafka
  answers them with the same code;
- a fetch compares the records both sides returned up to the shorter
  answer, and the high watermark and last stable offset exactly
  (ref:DEC-fetch-sizing);
- a Metadata answer is compared where both sides know the topic, and the
  topics each side created are compared over the whole trace
  (ref:DEC-kafka-divergence-auto-create);
- a group's JoinGroup rounds are compared as a whole, since the sides may
  choose another leader to answer with the members.

Then the core's own history is checked as
[the conformance kit](kafka-core.md#the-conformance-kit) checks it: replayed
through the reference model, and the laws checked over it and the state.

## The report

| Field | Is |
| --- | --- |
| `trace`, `core` | what was replayed through what |
| `requests`, `answered` | requests sent, and answered by the core |
| `stalled` | frames sent before the frames they waited for came back |
| `divergences` | each frame whose answers differ: `n`, `api`, and the `recorded` and `replayed` operations with their answers |
| `violations` | the core's answers the reference model gives differently, and laws broken over its history or state |

It exits 0 with no divergence, violation or stalled request, and 1
otherwise.

## A broken core

`cargo build -p lawtwin-kafka --features mutant-ignore-dedupe` builds a Rust
core that forgets a batch's producer tag, so it writes an idempotent retry
again. A trace with a retried batch replays through it with a divergence
at the retry's Produce frame. It is for tests; never deploy it.
