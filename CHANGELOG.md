# Changelog for `lawtwin`

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added
- The repository, checked by canon 0.2.0 from its first commit: `canon.yaml`, the reference registry, the decision ledger with the owner's decisions of 2026-10-09 (`DEC-three-planes`, `DEC-twins-wire-level-containers`, `DEC-dependency-contracts-in-lawspec`, `DEC-twins-in-rust`, `DEC-fidelity`, `DEC-typed-control-plane`, `DEC-first-release-scope`, `DEC-vetting-deferred-to-1.0`), and exemptions that defer vetting to 1.0
- Documentation: the three planes, the roadmap, and the Kafka contract's reference
- The Kafka dependency contract, the LawSpec package `lawtwin.kafka` 0.1.0, under `contracts/kafka/`: the `Broker` ability with one operation per Kafka request and its stateful spec handler `reference`, the reference model of topics, partition logs, producer ids, epochs and sequences, transactions and their markers, groups, generations, assignments and committed offsets (`DEC-kafka-broker-and-clients`, `DEC-kafka-model-scope`); the client abilities `Admin`, `Producer`, `Partitioner`, `Consumer`, `GroupCoordinator` and `Transactions` with spec handlers written over `Broker`; and 39 laws, among them order within a partition, dense offsets, at-least-once delivery from committed offsets, idempotent retries written once, zombie producers fenced by a new epoch, read-committed isolation, every partition assigned once per generation, and stale generations refused
