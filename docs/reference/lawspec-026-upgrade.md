---
id: lawtwin.reference.lawspec-026-upgrade
kind: reference
title: LawSpec 0.26 alignment
---
# LawSpec 0.26 alignment

This increment implements the first cross-project milestone using the merged
LawSpec compiler. The maintained upgrade acceptance matrix passes. The final
landing results and original scoped evidence are recorded separately below;
the larger path to 1.0 and owner review remain open. No package publication
or 1.0 acceptance is implied.

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
compiler additions are merged in LawSpec `24e8706c`. Stage a checkout containing
that commit until a package containing them is released; a matching version label alone does not establish
matching compiler bytes. The staging script rejects a checkout package-version
mismatch and fingerprints the native compiler.

Declared protocol refusals remain distinct from model defects. Snapshot schema
validation does not prove a state reachable; LawTwin's force operation still
constructs states through operations and checks their invariants. Neither native
conformance nor fixture/model agreement proves provider fidelity.

## Verified landing — 21 October 2026

The compiler is staged from LawSpec main `24e8706c`, whose final documentation
commit follows the accepted compiler/artifacts at `8d57bd2b`. Checkout staging
uses the native public JSON interface and records its executable fingerprint;
it is not a new WASM execution claim. The companion's complete 157-step target
CI and 6,416-response native/WASM comparison already pass on those compiler
inputs. The required checkout and staging command are in the README and
[contract guide](../how-to/run-contract-laws.md).

| Gate | Landing result |
| --- | --- |
| Contracts and consumers | All three packages pass mandatory definition admission. Regeneration and a subsequent check preserve application adapters and report zero planned changes. Explicit optional verification layers remain selected. |
| Native laws | All 613 law instances execute successfully with the new compiler build identity and zero reused passing laws: 79 on each of seven Kafka targets, and 14 Petstore plus 16 GitHub on Rust and Python. This is the maintained default 64-bit readable matrix. |
| Importer | All six importer tests pass. Both maintained documents reproduce in scratch directories, and four fresh OpenAPI 3.0/3.1 imports compile with their emitted public Rust adapters. |
| Workspace | The complete workspace test command reports 62 passes. Required real-broker tests also execute separately with `LAWTWIN_REQUIRE_KAFKA=1`; their default no-broker paths are not provider evidence. Release binaries build successfully. |
| Protocols and images | All 13 Kafka and 17 HTTP checks pass against native cores and against freshly built candidate images, with required clients enforced. Kafka's optional throughput benchmark remains ignored in each form. |
| Real Kafka | All 24 scenarios pass on both Apache Kafka 4.3.1 and the twin, with all four clients present, 1,115 observed operations and the eight documented group-assignment divergences: level 3. Strict real-broker peer and record/replay checks pass, including rejection of the deduplication mutant. |
| HTTP providers | Handwritten Petstore application, conformance and mutation/replay tests pass. Swagger Petstore independently reproduces its documented lawful result: 84 requests on each side, 36 divergences, two missing-name findings and no law violations. This is not a claim that Swagger's answers are fully conformant or that a real GitHub provider was checked. |
| Canonical checks | Automated checks pass under the existing owner-deferred exemptions. No human verdict is signed off. |

The real-Kafka run exposed a recorder defect: a Produce v10
`NOT_LEADER_OR_FOLLOWER` response advertised the real broker in its KIP-951
leader hints, allowing a client to reconnect outside the recorder. The recorder
now rewrites Produce and Fetch leader endpoints after fault application, using
the existing protocol codec. Node identities, epochs, racks, errors, offsets,
tagged extensions and raw captured responses are preserved. Three deterministic
checks fail before the repair; all four checks, including legacy byte identity
and lost-acknowledgement fault composition, pass afterward.
ref:kafka-produce-leader-endpoints ref:kafka-fetch-leader-endpoints

A separate replay assertion incorrectly required an answer for a wire-only
coordinator retry that the engine deliberately excludes. Replay accounting now
requires every request to be answered or explicitly counted, bounds omissions
by observed retry notes and retains the deduplication mutation check. The exact
captured 54-frame trace is a permanent regression: 53 answers, one coordinator
retry, no stalled requests, divergences or broken laws. Its bytes are retained
without filtering. ref:DEC-kafka-divergence-coordinator-timing

The first provider run remains failed at level 2, and the subsequent replay
assertion failure is also preserved. The corrected complete third run has its
own reports. Evidence is under `.artifacts/upgrade-026/landing-20261021/`,
including `compiler-stage-receipt.json`, `native-contracts-receipt.json`,
`provider-kafka-3/receipt.json`, `provider-http-1/receipt.json` and the native,
image, importer, workspace and canonical logs. The later recorder repair is in
the host-only Kafka fidelity crate; the images' normal/build dependency graphs
exclude it, and their compiled core/CLI inputs remain unchanged.

## Original working-tree checks — 20 October 2026

The following table preserves the original upgrade cohort, including its
interrupted companion compiler sweeps. The completed landing matrix above
identifies the newer compiler and executed results.

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

Cross-project acceptance against Ironframe's Haskell/WASM port, including CALM
bindings, topology-derived exports and completed-run evidence, remains separate.
Cassandra measurement and
fixture/cluster conformance, cores for every program target, the retained LawSpec
target and demonstration portfolio, advanced verification and final compatibility,
canonical owner review and release checks remain open. No Cassandra performance
budget has been selected. The user-deferred readership exercise and independent
adoption/GitHub Linguist recognition remain explicit external gates.
