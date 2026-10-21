---
id: lawtwin.reference.lawspec-026-upgrade
kind: reference
title: LawSpec 0.26 alignment
---
# LawSpec 0.26 alignment

This working-tree increment implements the first cross-project milestone.
The scoped LawTwin acceptance checks pass; broader release acceptance
remains incomplete, and the larger path to 1.0 remains open. No new
release, provider certification or 1.0 acceptance is implied.

The compiler pin is 0.26.0, configuration version 2, language profile
`1.0-rc.2`. The Kafka, Petstore and GitHub sources use canonical records,
explicit payloads, phrase names and named endings. Existing dependency package
versions stay 0.1.0. The Kafka package still has 79 instantiated law runs;
Petstore has 14 and GitHub 16. Mandatory definition validation runs independently
of optional law verification. Each maintained conformance project explicitly
selects verification layers, including property execution: configuration
version 2 otherwise selects no optional law checks.

Kafka now calls the generated `Cluster` handler's public operations and `try_`
forms. `typed_state` / `set_typed_state` preserve independent trial models while
sharing persistent data; `state` supplies native inspection. The HTTP importer
emits `lawtwin-embed.rs` from its OpenAPI IR, using public generated native types,
`try_` calls and native snapshots. Distinct Cargo package identities let the
core link its registered models as normal libraries. Neither twin has a build
script that scrapes generated handler bodies or rewrites generated module paths.
The HTTP registry is explicit; [HTTP cores](http-core.md) describes registration.

A fresh constrained OpenAPI 3.1 model uses LawSpec's dynamic Rust path. The
companion LawSpec changes provide that path with public `start`, `try_`, `state`
and schema-checked `set_state` methods, sharing the existing failure-catching
implementation with typed handlers. Dynamic snapshots copy state. These local
compiler changes must be staged from the companion checkout until a package
containing them is released; a matching version label alone does not establish
matching compiler bytes. The staging script rejects a checkout package-version
mismatch and fingerprints the native compiler.

Declared protocol refusals remain distinct from model defects. Snapshot schema
validation does not prove a state reachable; LawTwin's force operation still
constructs states through operations and checks their invariants. Neither native
conformance nor fixture/model agreement proves provider fidelity.

## Checks

Local logs are under `.artifacts/upgrade-026/`; companion compiler logs are
under LawSpec's `.artifacts/integration-1/`. The stopped broader sweeps are
recorded in `interrupted-sweeps.json`; their unfinished portions are not passes.
Earlier failed or no-layer runs are
retained and are not acceptance evidence. In particular, the first configuration
upgrade omitted verification layers; the final configuration and regression test
prevent that command from succeeding without native law execution.

| Gate | Current result |
| --- | --- |
| Contract admission | Kafka, Petstore and GitHub compile with mandatory definition checks and explicitly selected optional layers. |
| Fresh OpenAPI imports | Four documents compile, including 3.0 JSON/YAML and constrained 3.1 records, maps, singleton enums and unions; all four emitted Rust adapters compile against public model APIs. |
| Importer checks | Six Rust importer tests pass, including maintained configuration layer selection and handwritten-file preservation. Fresh importer-owned outputs reproduce without modifying maintained files. |
| Public state/failure behavior | HTTP tests cover refusal recovery, snapshot isolation and malformed-call defects; Kafka covers refusal recovery and independent cloned trial models. |
| Companion dynamic Rust API | Both integer widths and both output layouts pass native restoration, refusal and concurrent-update checks, and reject the wrong-arithmetic mutation. |
| Native contract targets | Pass: 613 law instances actually executed across the seven Kafka targets and Rust/Python for Petstore and GitHub. The generator check reports zero planned changes. |
| Protocol conformance | Final rebuilt-core runs pass: 13 Kafka checks and 17 HTTP checks, including clients, faults, force, determinism, golden histories and replay. One optional Kafka throughput benchmark remains ignored; it is separate from conformance. |
| Independent providers | The final freshly rebuilt Kafka fidelity executable compares 24 scenarios against `apache/kafka:4.3.1` and the twin: all 24 pass on each, with 1,104 observed operations and eight documented group-assignment divergences. The result reaches level 3. Real-broker record/replay rejects the deduplication mutant. HTTP application tests independently check the handwritten Petstore and its mutations. |
| Canonical checks | Pass on a Git snapshot with identical captured source bytes and modes. The live uncommitted tree cannot supply HEAD history for new files. Owner vetting remains deferred to 1.0 under the existing decision; no pending verdict is signed off. |
| Packed SDK | The unpacked local npm archive passes the dynamic embedding regression on both widths and layouts, including the arithmetic mutation. Archive SHA-256: `8e7dc20a91c044f595183f28b0a32fb53e77888d447cce091ad8e6108606d724`. No package is published. |
| Companion compiler | The full compiler suite passes 2,253 examples with zero failures. WASM rebuild, source/API integrity, 122 generated-file checks and the 232-module compiler boundary check pass. The unfiltered SDK sweep was stopped with 333 passed, five opt-in skips and one cancelled example-export test: it is incomplete. The broad native/WASM sweep was also stopped after 68 frontend fixtures at both widths and all 17 targets in the 32-bit noncompact profile. A separate completed focused run compares 303 full responses across all 17 targets, both widths and both layouts, including the changed dynamic embedding model and invalid-input controls. |

Reproduce with the companion source checkout:

```sh
make lawspec-stage LAWSPEC_HOME=../lawspec
make contract-check
make contract-generate contract-test GRADLE_HOME=../lawspec/.tools/gradle-9.3.0
make openapi-import-check openapi-compile-check
make test core-conformance core-conformance-http
make fidelity-test
```

The optional Gradle path above uses the companion checkout's existing toolchain;
an installed Gradle can be used instead. The generated OpenAPI Python client
is required for full HTTP client coverage (`make openapi-client-http`);
`LAWTWIN_REQUIRE_CLIENTS=1` makes its absence a failure.

## Remaining programme

CALM 1.2 identity resolution and environment decorators, topology-derived exports
and completed-run evidence remain the next milestone. Cassandra measurement and
fixture/cluster conformance, cores for every program target, the retained LawSpec
target and demonstration portfolio, advanced verification and final compatibility,
canonical owner review and release checks remain open. No Cassandra performance
budget has been selected. The user-deferred readership exercise and independent
adoption/GitHub Linguist recognition remain explicit external gates.
