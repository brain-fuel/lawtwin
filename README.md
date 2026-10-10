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
| `contracts/kafka/` | The Kafka dependency contract, the LawSpec package `lawtwin.kafka` 0.1.0, and its twin file `lawtwin.json`, which declares the twin's faults |
| `contracts/kafka/test/` | The project that runs the contract's laws on the Rust, Python, Go, Java and Kotlin targets |
| `twins/kafka/rust/` | The Kafka twin's Rust core: the contract's generated model behind the Kafka wire protocol, with the control plane; its image is `lawtwin/kafka:0.1.0` |
| `twins/kafka/conformance/` | The core conformance kit every Kafka core must pass, with librdkafka, kafkajs, franz-go and the Java client |
| `twins/kafka/shared/` | The golden wire session every core answers byte for byte |
| `crates/lawtwin/` | The `lawtwin` command: `lawtwin import openapi`, `lawtwin up`, and the control-plane commands |
| `examples/kafka/` | Running the Kafka twin with podman compose |
| `contracts/petstore/lawtwin.json`, `contracts/github/lawtwin.json` | The twin files of the imported contracts: the HTTP twin's faults and their effect on each law |
| `crates/lawtwin-wire/` | The wire map of an imported contract, read once for the HTTP cores, the client twin and conformance runs |
| `twins/http/rust/` | The HTTP twin's Rust core: any imported contract's generated model behind HTTP/1.1, with the control plane; its image is `lawtwin/http:0.1.0` |
| `twins/http/conformance/`, `twins/http/shared/` | The core conformance kit every HTTP core must pass, and its golden sessions |
| `examples/http/` | Running the HTTP twin with podman compose; a hand-written Petstore; the Swagger Petstore 3 built for this machine |
| `contracts/petstore/`, `contracts/github/` | Contracts imported from the Swagger Petstore and from GitHub's label and milestone endpoints, with their test projects |
| `openapi/` | The OpenAPI documents they are imported from, with their licences in `openapi/NOTICE` |
| `docs/` | The documentation: explanation, reference and how-to pages |

Conformance against the real Kafka, and record and replay for Kafka,
follow; [the roadmap](docs/explanation/roadmap.md) lists what each release
covers.

## The Kafka twin

The Kafka twin is a broker your application reaches with its usual client.
Every request is answered by the Kafka contract's reference model, the code
LawSpec generates from it, and a test drives it through the control plane:

```sh
make image                                         # lawtwin/kafka:0.1.0, about 6 MB
podman run --rm -p 9092:9092 -p 9644:9644 lawtwin/kafka:0.1.0
lawtwin force state.json                           # a state, or the law that forbids it
lawtwin fault duplicate-delivery count=2           # a declared fault
lawtwin advance 30s                                # the twin clock
lawtwin history > run.jsonl && lawtwin laws        # what happened, checked against the laws
```

librdkafka, kafkajs, franz-go and the Java client each produce, consume,
rebalance a group, use the idempotent producer and transactions read
committed, and see every fault and forced state, with the contract's laws
checked over each history. The twin will have a core in every LawSpec target
language; each must pass the same conformance kit (`make core-conformance`).
[The Kafka twin](docs/reference/kafka-twin.md),
[the control plane](docs/reference/control-plane.md) and [Run the Kafka
twin](docs/how-to/run-the-kafka-twin.md) say more.

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

The contracts need LawSpec branch `lawspec-lawtwin` at e99a0bc; its npm
release is pending, so stage it from a checkout of that branch first with
`make lawspec-stage LAWSPEC_HOME=<checkout>`. You also need Node 22 or
later, Rust 1.85 or later with CMake, and Python 3.13 or later; Go, a JDK
with Maven, and Gradle run the Kafka contract's other targets and the
conformance kit's clients, and podman builds the image. `make help` lists
every task.

## The HTTP twin

The HTTP twin serves any imported OpenAPI contract: requests are routed and
read by the contract's wire map, refused as the document says, answered by
the contract's generated reference model, and shaped as the document
describes. The same control plane forces states, raises faults (latency,
5xx, 429 with Retry-After, timeouts, resets, slow bodies) and checks the
laws:

```sh
make image-http                                    # lawtwin/http:0.1.0, under 5 MB
podman run --rm -p 8080:8080 -p 9644:9644 -e TWIN_CONFIG='{"contract":"petstore"}' lawtwin/http:0.1.0
lawtwin fault rate-limit limit=10 window=1s        # a declared fault
lawtwin peer http call --contract petstore --target http://localhost:3000 --invalid --retries
lawtwin record http --contract petstore --upstream http://real:8080 --listen 127.0.0.1:8081
lawtwin replay http trace.jsonl --contract petstore # through the twin, divergences reported
```

`lawtwin peer http call` is the simulated client of your own API, and
`lawtwin conform http` runs the contract's laws against a real service and
the twin. [The HTTP twin](docs/reference/http-twin.md), [HTTP twin
cores](docs/reference/http-core.md) and [Run the HTTP
twin](docs/how-to/run-the-http-twin.md) say more.

## The OpenAPI importer

`lawtwin import openapi` turns an OpenAPI 3.0 or 3.1 document into a
contract package: its schemas as types, an ability per tag with an
operation per path and method, a typed failure per documented failing
status, a reference model of the resources it recognises, and laws over
them. [The OpenAPI importer](docs/reference/openapi-import.md) says what it
writes; [Import an OpenAPI document](docs/how-to/import-an-openapi-document.md)
shows how.

```sh
make build
target/release/lawtwin import openapi openapi/petstore3.json --package lawtwin.petstore --out contracts/petstore
```

## Development

LawTwin is checked by [canon](https://github.com/brain-fuel/canon) from its
first commit: `canon.yaml`, the reference registry `canonical_refs.yaml`, the
decision ledger `canonical_decisions.yaml`, and `canonical_vetting/`. Vetting
is deferred to 1.0 (`DEC-vetting-deferred-to-1.0`). Run `make canon` with
`CANON_HOME` naming a canon 0.2.0 checkout built with `stack build`.

Every check runs locally; there is no hosted CI. `make ci` runs them all.

Changes are listed in [CHANGELOG.md](CHANGELOG.md). LawTwin is released
under the [MIT License](LICENSE).
