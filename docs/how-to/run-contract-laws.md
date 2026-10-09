---
id: lawtwin.how-to.run-contract-laws
kind: how-to
title: Run a contract's laws
---
# Run a contract's laws

This guide compiles the Kafka contract and runs its laws on the Rust and
Python targets.

## Before you start

You need:

- Node 22 or later, for `npx lawspec@0.21.0` (ref:lawspec);
- Rust 1.85 or later;
- Python 3.13 or later, as `python3.13`, or named with `PYTHON313`.

## Compile the contract

```sh
make contract-check
```

This compiles the package `lawtwin.kafka` with `lawspec package`, then the
test project in `contracts/kafka/test/` with `lawspec check`. It prints the
number of laws, and how each will be checked.

## Run the laws

```sh
make contract-test
```

The first run makes the Python environment in
`contracts/kafka/test/python/.venv`. Then `lawspec test` runs every law on
both targets. To run one target, name it:

```sh
make contract-test TARGET=python
```

Each law runs once for each lawful handler of its ability: the native
handler, and each spec handler. In the test project the native handlers
serve every ability from one shared reference cluster per case
(ref:DEC-contract-test-shared-cluster).

The test project binds generators for `Text` and `Int32` in its
`lawspec.json`: most texts are valid topic names and most integers are
small counts, so the laws' refinements keep most cases. The factories are
`rust/tests/support/kafka_generators.rs` and
`python/tests/kafka_generators.py`.

## After you change the contract

Regenerate the test project, then run the laws again:

```sh
make contract-generate
make contract-test
```

`make contract-generate-check` fails when a generated file is stale. The
native handlers in `contracts/kafka/test/rust/src/lawtwin/kafka/contract.rs`
and `contracts/kafka/test/python/src/lawtwin/kafka/contract.py` are yours:
`generate` never overwrites them. When an ability gains an operation, add it
to both by hand.
