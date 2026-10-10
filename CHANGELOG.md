# Changelog for `lawtwin`

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added
- The repository, checked by canon 0.2.0 from its first commit: `canon.yaml`, the reference registry, the decision ledger with the owner's decisions of 2026-10-09 (`DEC-three-planes`, `DEC-twins-wire-level-containers`, `DEC-dependency-contracts-in-lawspec`, `DEC-twins-in-rust`, `DEC-fidelity`, `DEC-typed-control-plane`, `DEC-first-release-scope`, `DEC-vetting-deferred-to-1.0`), and exemptions that defer vetting to 1.0
- Documentation: the three planes, the roadmap, the Kafka contract's reference, how to run a contract's laws, and the LawSpec gap report
- The Kafka dependency contract, the LawSpec package `lawtwin.kafka` 0.1.0, under `contracts/kafka/`: the `Broker` ability with one operation per Kafka request and its stateful spec handler `reference`, the reference model of topics, partition logs, producer ids, epochs and sequences, transactions and their markers, groups, generations, assignments and committed offsets (`DEC-kafka-broker-and-clients`, `DEC-kafka-model-scope`); the client abilities `Admin`, `Producer`, `Partitioner`, `Consumer`, `GroupCoordinator` and `Transactions` with spec handlers written over `Broker`; and 39 laws, among them order within a partition, dense offsets, at-least-once delivery from committed offsets, idempotent retries written once, zombie producers fenced by a new epoch, read-committed isolation, every partition assigned once per generation, and stale generations refused
- `contracts/kafka/test/`, the project that runs the contract's laws on Rust, Python, Go, Java and Kotlin, with native handlers made with each case's `Cluster`, which holds the generated reference model (`DEC-contract-test-shared-cluster`)
- `make lawspec-stage` and `tools/stage-lawspec.sh`, which stage the LawSpec CLI from a source checkout and run its native compiler, for LawSpec builds not yet on npm; `make lawspec-version` names the LawSpec the contracts need

### Changed
- The Kafka contract is re-expressed for LawSpec main after the `lawspec-gaps` merge, which closes 16 of the 17 gaps it found: one spec handler `cluster` answers `Broker`, `Admin` and `GroupCoordinator` by the model's checked definitions (`produceRequest by model.produce`), and the handler set `reference` adds the client handlers; operations `fail with KafkaError`; the model uses field access and record update, `prelude.range`, and saturating arithmetic; the facade unit is `lawtwin.kafka`; the murmur2 partitioner is written in the contract and checked against Kafka's vectors; every ability is declared `with Cluster`. `FetchedRecord`'s field `record` is now `entry`, which Java allows
- The Kafka test project binds no generators: LawSpec now generates topic names from the model's regex
- The gap report marks each closed gap, and lists five more found on the `lawspec-gaps` build
