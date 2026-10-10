# LawTwin

**A Contract Simulator for Systems You Don't Control.**

LawTwin gives you a contract-governed stand-in for any dependency you cannot
reliably provision, reset, isolate, or force into the state your test
requires. A stand-in, a *twin*, speaks the dependency's own wire protocol, so
your application reaches it with its usual client and its usual
configuration. What the twin does is not guessed: it is a reference model
whose laws are written in [LawSpec](https://www.npmjs.com/package/lawspec)
and checked against the twin, against the real dependency, and against the
traffic you record.

LawTwin is part of **Ironframe** by GoForge, *making codebases safe for
autonomous change*. The family is LawSpec and LawTwin; canon and Rice's Tax;
with Rune and Wavelet on the roadmap.

## The three planes

| Plane | Owns | Never contains |
| --- | --- | --- |
| **LawSpec** (business contract) | your application's laws; a dependency is an ability, and its production handler is the real client library | anything about twins, endpoints or environments |
| **LawTwin** (stand-ins) | dependency contracts as LawSpec packages; twin servers; the control plane; conformance, record and replay | business laws; deciding which environment uses what |
| **Ironframe** (wiring) | which dependency is real and which is a twin, per environment; the config service and its cloud sync; environment attestation | contracts; twin internals |

LawSpec knows nothing about LawTwin. Your business laws never mention a
twin. [The three planes](docs/explanation/three-planes.md) explains why.

## What is here

| Path | What it is |
| --- | --- |
| `contracts/kafka/` | The Kafka dependency contract, the LawSpec package `lawtwin.kafka` 0.1.0 |
| `contracts/kafka/test/` | The project that runs the contract's laws on the Rust, Python, Go, Java and Kotlin targets |
| `docs/` | The documentation: explanation, reference and how-to pages |

The Kafka twin server, the control plane, conformance, record and replay,
and the OpenAPI importer follow; [the roadmap](docs/explanation/roadmap.md)
lists what each release covers.

## The Kafka contract

`lawtwin.kafka` states what a Kafka cluster guarantees, as laws over seven
abilities: `Broker` (the wire protocol), `Admin`, `Producer`, `Partitioner`,
`Consumer`, `GroupCoordinator` and `Transactions`. Its spec handler
`cluster` is the reference model the Kafka twin will run, and its handler
set `reference` runs every law over the spec handlers together. [The Kafka
contract](docs/reference/kafka-contract.md) says what it states.

Check and run its laws:

```sh
make contract-check      # compile the package and its laws
make contract-test       # run them on every target (TARGET=rust, for example, for one)
```

The contracts need LawSpec main after the `lawspec-gaps` and
`lawspec-codegen` merges; its npm release is pending, so stage it from a
LawSpec checkout first with `make lawspec-stage LAWSPEC_HOME=<checkout>`.
You also need Node 22 or later, Rust 1.85 or later, and Python 3.13 or
later; Go, a JDK with Maven, and Gradle run the Kafka contract's other
targets. `make help` lists every task.

## Development

LawTwin is checked by [canon](https://github.com/brain-fuel/canon) from its
first commit: `canon.yaml`, the reference registry `canonical_refs.yaml`, the
decision ledger `canonical_decisions.yaml`, and `canonical_vetting/`. Vetting
is deferred to 1.0 (`DEC-vetting-deferred-to-1.0`). Run `make canon` with
`CANON_HOME` naming a canon 0.2.0 checkout built with `stack build`.

Every check runs locally; there is no hosted CI. `make ci` runs them all.

Changes are listed in [CHANGELOG.md](CHANGELOG.md). LawTwin is released
under the [MIT License](LICENSE).
