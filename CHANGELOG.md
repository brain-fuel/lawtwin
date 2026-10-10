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
- `lawtwin import openapi`, in a new Cargo workspace (`crates/lawtwin`): it reads an OpenAPI 3.0 or 3.1 document, in JSON or YAML, and writes a LawSpec dependency-contract package. Objects become records, enums sums, `oneOf` and `anyOf` sums of their alternatives, `allOf` merged records, arrays `List`, maps `KeyVal Text`, and nullable or optional values `Maybe`; bounds, lengths, patterns, formats and item counts become named refinements and `valid<Type>` definitions the model checks (`DEC-openapi-constraints-as-validation`). Each tag is an ability, each path and method an operation that fails with `ApiError`, a constructor per documented failing status. Resources (create, read, list, replace, update and delete by key, under parent paths) get a stateful reference model and laws: a created record reads back, a deleted one is not found, the list holds a created one, an update changes only the fields it gives, and a replaced one reads back (`DEC-openapi-resources`). What it cannot model goes to a hand-written unit, and hand-written laws to a laws unit, both written once. It also writes the test project, with native handlers on Rust and Python, and `lawtwin-openapi.json`, the wire map. `--tag` and `--path` select operations
- `contracts/petstore` (`lawtwin.petstore`), imported from the Swagger Petstore (OpenAPI 3.0), and `contracts/github` (`lawtwin.github`), imported from GitHub's label and milestone endpoints; both compile with `lawspec check` and their laws pass on Rust and Python. The documents are vendored in `openapi/` with their licences
- `make build`, `make test`, `make openapi-import` and `make openapi-import-check`; `make ci` runs the importer's tests and checks the imported contracts are current
- Documentation: the OpenAPI importer's reference, and how to import an OpenAPI document; the gap report lists five more LawSpec gaps the importer found
- `make lawspec-stage` and `tools/stage-lawspec.sh`, which stage the LawSpec CLI from a source checkout and run its native compiler, for LawSpec builds not yet on npm; `make lawspec-version` names the LawSpec the contracts need

### Changed
- The Kafka contract is re-expressed for LawSpec main after the `lawspec-gaps` merge, which closes 16 of the 17 gaps it found: one spec handler `cluster` answers `Broker`, `Admin` and `GroupCoordinator` by the model's checked definitions (`produceRequest by model.produce`), and the handler set `reference` adds the client handlers; operations `fail with KafkaError`; the model uses field access and record update, `prelude.range`, and saturating arithmetic; the facade unit is `lawtwin.kafka`; the murmur2 partitioner is written in the contract and checked against Kafka's vectors; every ability is declared `with Cluster`. `FetchedRecord`'s field `record` is now `entry`, which Java allows
- The Kafka test project binds no generators: LawSpec now generates topic names from the model's regex
- The gap report marks each closed gap, and lists five more found on the `lawspec-gaps` build
