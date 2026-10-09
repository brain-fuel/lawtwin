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
- The twin: a Rust server that embeds the contract's generated reference
  model behind the Kafka wire protocol (ref:kafka-protocol-crate). It covers
  the requests librdkafka, the Java client, franz-go and kafkajs use:
  ApiVersions, Metadata, Produce, Fetch, ListOffsets, FindCoordinator,
  JoinGroup, SyncGroup, Heartbeat, LeaveGroup, OffsetCommit, OffsetFetch,
  InitProducerId, AddPartitionsToTxn, AddOffsetsToTxn, EndTxn and
  TxnOffsetCommit.
- The control plane: `force`, `fault` (broker unavailable, leader change,
  consumer lag, duplicate delivery, rebalance storm), `advance`, `inspect`
  and `history`.
- Conformance against the `apache/kafka` image (ref:apache-kafka-image),
  then record and replay through a Kafka proxy.

**HTTP and OpenAPI.**

- `lawtwin import openapi <doc>` turns an OpenAPI 3.x document
  (ref:openapi-3-1) into a LawSpec package: types, one operation per
  endpoint, status codes as typed failures, and laws for stateful resources
  where the document allows.
- A stateful HTTP twin, faults (latency, 5xx, 429, timeouts), record and
  replay through an HTTP proxy, and conformance against a real endpoint.

**Packaging.** A Containerfile per twin (ref:containerfile), `podman
compose` for local runs (ref:podman-compose), and OpenTofu modules for AWS
ECS Fargate, GCP Cloud Run and Azure Container Apps (ref:opentofu).

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
