---
id: lawtwin.reference.observation-receiver
kind: reference
title: Disposable observation receiver
---
# Disposable observation receiver

`lawtwin-observation` collects OTLP/HTTP JSON logs and spans for one isolated run.
It is a test observation fixture, not a claim of CloudWatch, Azure Monitor,
Google Cloud Logging or general OpenTelemetry Collector fidelity. It adds no
solver or generated dependency model. Application assertions and independent
protocol/provider histories retain their own meaning.

Build with `make build-observation`; run it directly, or with `lawtwin observe`:

```sh
target/release/lawtwin-observation --config receiver.json \
  --listen 127.0.0.1:4318 --admin-listen 127.0.0.1:9646
```

Example configuration, with a fresh journal path:

```json
{
  "runId": "notification-test-1",
  "topologyDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "journal": "artifacts/notification-test-1/observations.jsonl",
  "producers": {
    "worker": {"node":"worker","implementation":"build-1","revision":"revision-1"}
  },
  "edges": {"worker-email":"worker"},
  "maxRecords": 100000,
  "maxJournalBytes": 67108864
}
```

Intake serves `POST /v1/logs` and `POST /v1/traces`, with uncompressed JSON up to
4 MiB per request. gRPC, protobuf, compression and metrics are not supported.
Use an actual OpenTelemetry Collector bridge when the SDK does not export JSON.
Supported behavior follows the primary [OTLP specification](https://opentelemetry.io/docs/specs/otlp/).
Unknown payload fields are retained; this is not full protobuf-schema validation.

Every resource needs string attributes `lawtwin.producer.id`, `ironframe.run.id`,
`ironframe.execution.id`, `ironframe.topology.digest`, `ironframe.node.id`,
`ironframe.configuration.revision` and `service.version`. All configured identities
must match. Record attributes may include `ironframe.edge.id`; it must belong to
the producer's node. Request batches are validated before any part is retained.
Keep notification ids and ordinary trace/span ids in the application's telemetry.

The separate private administration listener serves:

| Endpoint | Behavior |
| --- | --- |
| `GET /healthz` | Readiness, also available on intake. |
| `GET /v1/state` | Retained counts, checkpoints, faults and loss. |
| `POST /v1/checkpoint` | Immutable producer counts, flush outcome and drop count. |
| `POST /v1/faults` | Replace declared receiver faults. |
| `POST /v1/seal` | Permanently seal, complete or explicitly incomplete. |
| `GET /v1/bundle` | Export the sealed journal and its SHA-256 identity. |

Checkpoint example:

```json
{"producer":"worker","counts":{"logs":42,"spans":12},"flushed":true,"dropped":0}
```

The producer must actually perform SDK ForceFlush and independently account for
intended observations. SDK [ForceFlush](https://opentelemetry.io/docs/specs/otel/logs/sdk/)
and receiver HTTP acknowledgement are different operations. Checkpointing from
the receiver's own counts would hide producer-side loss. No exports are accepted
after a producer checkpoints. SDK/bridge sampling and dropped records must be
included in the stated completeness boundary.

Fault fields are `unavailableRequests` (503, allowing a successful retry),
`dropRequests` (200 with OTLP partial-success rejected counts), and `delayMs`
(real transport delay, at most 30000). Posting `{}` clears active faults but does
not erase past loss. Each change is journaled. Duplicate successful exports are
retained and count independently; the fixture does not invent deduplication.

The journal is created exclusively, and each retained batch is flushed to disk
before success is acknowledged. Existing journals are never overwritten or
implicitly reset. Missing checkpoints, failed flushes, drops, differing expected
counts or persistence failures yield `incomplete`. A byte limit also bounds empty
batches and control entries. Sealing is permanent and idempotent; a running
collection cannot be exported as complete evidence.

Export the bundle before stopping the receiver. The raw journal also survives
failure or termination; an interrupted unsealed journal is incomplete evidence.
The bundle includes resource/execution identities, checkpoints, counts, journal
entries, digest, status and reasons. It establishes only receiver persistence and
producer-declared accounting, not the truth of an application's business log.
Arrival order is not a total causal order across services; preserve trace links
and compare independent provider outcomes.

`make test-observation` exercises lifecycle and negative controls without running
the full Kafka/HTTP contract harnesses. `make image-observation` builds a local
0.3.0 image without publishing it. Administration has no authentication;
keep it on the runner's private network. Normal protocol twins still own their
existing histories, deterministic models, resets, replay and conformance suites.
