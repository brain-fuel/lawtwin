---
id: lawtwin.how-to.run-contract-laws
kind: how-to
title: Run a contract's laws
---
# Run a contract's laws

This guide compiles the contracts and runs their laws: the Kafka contract on
the Rust, Python, Go, Java, Kotlin, F# and C# targets, and each imported OpenAPI
contract on Rust and Python.

## Before you start

You need:

- LawSpec 0.26.0 (ref:lawspec), staged from a built checkout containing
  `24e8706c` with `make lawspec-stage LAWSPEC_HOME=<checkout>`; a published
  package containing the same public embedding additions can instead use
  `make lawspec-stage`;
  `make lawspec-version` prints what the contracts need;
- Node 22 or later;
- Rust 1.85 or later;
- Python 3.13 or later, as `python3.13`, or named with `PYTHON313`;
- for the Kafka contract's other targets, Go 1.25 or later, JDK 25 with
  Maven, Gradle 9.3 (on the path, or named with `GRADLE_HOME`), and the
  .NET 10 SDK for F# and C#.

## Stage LawSpec

The upgrade requires the public embedding additions merged in `24e8706c`.
Until a package containing them is available on npm, stage its CLI from that
LawSpec checkout built with `stack build`:

```sh
make lawspec-stage LAWSPEC_HOME=~/src/lawspec
```

This copies the checkout's npm package into `.lawspec-cli/` and makes it
run the checkout's native compiler, so you need no WebAssembly build. The
checkout is not changed. Every task then runs `node
.lawspec-cli/bin/lawspec.mjs`; set `LAWSPEC` to run LawSpec another way.

## Compile the contracts

```sh
make contract-check
```

This compiles each package with `lawspec package`, then its test project in
`contracts/<name>/test/` with `lawspec check`. It prints the number of laws,
and how each will be checked.

## Run the laws

```sh
make contract-test
```

The first run makes each Python environment in
`contracts/<name>/test/python/.venv`. Then `lawspec test` runs every law on
every target the test project names. To run one target, name it:

```sh
make contract-test TARGET=python
```

Each law runs once for each lawful handler of its ability: the native
handler, and each spec handler. The Kafka contract's native handlers are
made with the case's `Cluster`, which holds the generated reference model
(ref:DEC-contract-test-shared-cluster).

## After you change a contract

Regenerate the test projects, then run the laws again:

```sh
make contract-generate
make contract-test
```

`make contract-generate-check` fails when a generated file is stale. The
Kafka test project's native handlers are yours, and `generate` never
overwrites them:

- `contracts/kafka/test/rust/src/lawtwin/kafka.rs`;
- `contracts/kafka/test/python/src/lawtwin/kafka/__init__.py`;
- `contracts/kafka/test/go/lawtwin/kafka/adapter.go`;
- `contracts/kafka/test/java/src/main/java/lawtwin/Kafka.java`;
- `contracts/kafka/test/kotlin/src/main/kotlin/lawtwin/Kafka.kt`.

When an ability gains an operation, add it to the Rust and Java handlers by
hand; the Python, Go and Kotlin handlers delegate every operation.
