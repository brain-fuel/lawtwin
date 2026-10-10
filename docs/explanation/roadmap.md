---
id: lawtwin.explanation.roadmap
kind: explanation
title: Roadmap
---
# Roadmap

Each release adds dependencies, or deepens how far a twin can be trusted.
What a release covers is decided in the ledger (ref:DEC-first-release-scope).

## 0.1: Kafka and HTTP

**Kafka.**

- The dependency contract `lawtwin.kafka`: done in this repository.
- The twin's Rust core: done. It embeds the contract's generated reference
  model behind the Kafka wire protocol (ref:kafka-protocol-crate), with the
  requests and versions librdkafka, the Java client, franz-go and kafkajs
  negotiate: ApiVersions, Metadata, Produce, Fetch, ListOffsets,
  FindCoordinator, JoinGroup, SyncGroup, Heartbeat, LeaveGroup,
  DescribeGroups, ListGroups, OffsetCommit, OffsetFetch, InitProducerId,
  AddPartitionsToTxn, AddOffsetsToTxn, EndTxn, TxnOffsetCommit,
  CreateTopics, CreatePartitions and DescribeConfigs. Its image is 6 MB.
- The control plane: done, shared by every twin and every core: `force`,
  `fault` (broker unavailable, leader change, consumer lag, duplicate
  delivery, rebalance storm, request latency), `advance`, `inspect`,
  `history`, `replay`, `laws` and `seed`, over an admin API described by an
  OpenAPI document.
- The core conformance kit: done. All four clients pass every scenario on
  the Rust core, as a program and as an image.
- Conformance against the `apache/kafka` image (ref:apache-kafka-image),
  then record and replay through a Kafka proxy.

**Cores in every target language.** Each twin gets a core in every LawSpec
target language, each embedding the model LawSpec generates for it, each
with its own image or native deployment, and each checked by the same
conformance kit, golden session included (ref:DEC-core-per-target):

1. Rust: done for Kafka and HTTP.
2. Go, and the JVM (Java and Kotlin).
3. Python, JavaScript and TypeScript, and Haskell.
4. The BEAM (Erlang, Elixir and Gleam), deployed as an OTP release or an
   Erlang cluster as well as an image; C#, F# and the other targets.

**HTTP and OpenAPI.**

- `lawtwin import openapi <doc>` turns an OpenAPI 3.x document
  (ref:openapi-3-1) into a LawSpec package: types, one operation per
  endpoint, status codes as typed failures, and laws for stateful resources
  where the document allows. Done in this repository, with the Petstore
  and GitHub contracts imported from their documents.
- The HTTP twin's Rust core: done. One core serves any imported contract,
  with the shared control plane, faults (latency, 5xx, 429 with
  `Retry-After`, timeouts, connection resets, slow bodies), forced states,
  and a conformance kit with golden sessions. Its image is 5 MB.
- The OpenAPI client twin, `lawtwin peer http call`: done.
- Record and replay through an HTTP proxy, and `lawtwin conform http`
  against a real endpoint: done, against a hand-written Petstore and the
  Swagger Petstore 3.

**Packaging.** A Containerfile per core (ref:containerfile): done for the
Kafka twin's Rust core, `lawtwin/kafka:0.1.0`, and the HTTP twin's,
`lawtwin/http:0.1.0`. `podman compose` for local runs (ref:podman-compose):
done, in `examples/kafka/` and `examples/http/`. OpenTofu modules for
AWS ECS Fargate, GCP Cloud Run and Azure Container Apps (ref:opentofu).

## Later tiers

Each tier is a dependency contract, a twin, its faults and its conformance
suite. The order follows how often a dependency blocks a test.

1. **SQL**, starting with Postgres: transactions and isolation levels,
   constraints, sequences.
2. **Redis and Valkey**: keys with expiry, data structures, pub/sub,
   streams.
3. **S3**: buckets, objects, versions, multipart uploads, read-after-write.
4. **Queues**: SQS (visibility timeouts, dead-letter queues, FIFO groups) and
   RabbitMQ (exchanges, routing, acknowledgements).
5. **Cassandra**: tunable consistency, lightweight transactions.
6. **Document and key-value stores**: MongoDB and DynamoDB, with conditional
   writes and streams.
7. **Search**: Elasticsearch and OpenSearch, with refresh and near-real-time
   visibility.
8. **gRPC**: services from their protobuf definitions, as OpenAPI services
   are from theirs.
9. **SMTP**: delivery, bounces, and the mail a test expects to see.
10. **OAuth and OIDC**: authorization servers, tokens, expiry and rotation.
11. **Networks and load balancers**: partitions, latency, retries and
    connection draining between services.

## Before 1.0

Vetting: every comment, page, decision and reference is signed off
(ref:DEC-vetting-deferred-to-1.0).

What LawSpec cannot yet state for these contracts is collected in the
[LawSpec gap report](lawspec-gaps.md), for the LawSpec roadmap
(ref:lawspec-roadmap).
