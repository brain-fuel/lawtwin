---
id: lawtwin.reference.http-core
kind: reference
title: HTTP twin cores
---
# HTTP twin cores

A twin's core is the server: the model LawSpec generates for one target
language, embedded behind the dependency's wire protocol and LawTwin's
control plane (ref:DEC-core-per-target). The HTTP twin's cores serve any
imported OpenAPI contract over HTTP/1.1. Everything that is not the core is
written once: the [control plane](control-plane.md), each contract's twin
file and wire map, the history format, the `lawtwin` command, the golden
sessions, the conformance kit, and these pages.

| Core | Directory | Deploys as | Status |
| --- | --- | --- | --- |
| Rust | `twins/http/rust/` | the `lawtwin-http` program; the image `lawtwin/http:0.2.1` | passes the conformance kit |
| Go | `twins/http/go/` | a program and an image | next |
| JVM (Java and Kotlin) | `twins/http/jvm/` | a jar and an image | next |
| Python, JavaScript and TypeScript, Haskell | `twins/http/<language>/` | a program or package and an image | later |
| BEAM (Erlang, Elixir, Gleam) | `twins/http/beam/` | an OTP release | later |
| C#, F# and the other targets | `twins/http/<language>/` | a program and an image | later |

## The repository

```text
contracts/<name>/                 an imported contract, its wire map and its twin file
crates/lawtwin-wire/              the wire map read once: types, values, requests, laws
twins/http/shared/golden/         the golden sessions every core must answer
twins/http/conformance/           the core conformance kit
twins/http/rust/                  the Rust core, with its Containerfile
```

## Which contracts a core serves

A core is a registry of compiled contracts, and serves the one it is
started with (ref:DEC-http-contract-registry). Each contract's generated Rust
library is a normal Cargo dependency with a distinct package identity.
`lawtwin import openapi` emits `lawtwin-embed.rs` from its OpenAPI IR; the
adapter calls the reference handler's public `try_` operations and uses
`state` / `set_state` for independent snapshots. No build script reads or
rewrites generated Rust sources or private definition names. One program and
one image serve every registered contract; `lawtwin-http --contracts` lists
them.

To serve a new contract: import it, generate its test project with the pinned
LawSpec compiler, add its package as a dependency in
`twins/http/rust/Cargo.toml` under the alias `<package_with_underscores>_model`,
and register its emitted adapter in `twins/http/rust/src/contracts.rs`.
Rebuild the core. The explicit registry makes the compiler check every adapter
against its public model API. `make openapi-compile-check` compiles fresh
OpenAPI 3.0/3.1 imports and their adapters, including constrained records,
maps and unions.

## The core interface

Every core is started the same way:

```text
<core> --contract <name> [--host <addr>] [--port <n>] [--admin-port <n>]
       [--advertise <host:port>] [--seed <n>] [--clock frozen|wall]
```

| Setting | Default | Environment |
| --- | --- | --- |
| contract | the only one, when the core has one | `LAWTWIN_CONTRACT`, or `contract` in `TWIN_CONFIG` |
| host | `127.0.0.1`; an image uses `0.0.0.0` | |
| API port | `8080` | |
| admin port | `9644` | |
| advertised address | the listener's, with `localhost` for an address that listens on every interface | `LAWTWIN_ADVERTISE`, or `advertise` |
| seed | `0` | `LAWTWIN_SEED`, or `seed` |
| clock | `frozen` | `LAWTWIN_CLOCK`, or `clock` |

The contract is named by its directory (`petstore`) or its package
(`lawtwin.petstore`). The command line wins over the environment. A core
serves the contract as [the HTTP twin](http-twin.md) describes, and the
control plane as
[`control-plane.openapi.yaml`](control-plane.openapi.yaml) describes.

## Determinism

With the same seed, a frozen clock and the same requests in the same
order, every core writes the same history and the same responses, byte for
byte. So that cores agree:

- **Randomness** is SplitMix64 seeded with the seed, as for the Kafka twin
  ([Kafka twin cores](kafka-core.md#determinism)). A core draws only when a
  fault's `probability` is below 100 (one draw, `below(100) < probability`),
  for a latency's `jitter` (`below(jitter + 1)`), and for a required
  documented header with no example.
- **The twin clock** starts at 2026-01-01T00:00:00Z (1767225600000 ms);
  `Date` headers are written from it.
- **Assigned keys** come from the model's counters, not from the seed.
- **The history** has its fields in the order `seq`, `at`, `source`, `op`,
  the arguments in the operation's order, then `answer`. An argument named
  `seq`, `at`, `source`, `op`, `answer`, `control` or `detail` gets a
  trailing underscore.
- **Bodies** are compact JSON with properties in the wire map's order.

## The conformance kit

`make core-conformance-http CORE=<core>` checks a core, driving it only
over HTTP and the admin API (ref:DEC-core-conformance-kit). `CORE` is a
command, given the core interface's options (by default the Rust core,
`target/release/lawtwin-http`), or `image:<image>`, run with podman. Each
test starts a fresh core, for the Petstore and GitHub contracts:

| Test | Checks |
| --- | --- |
| `golden.rs` | the golden session's every step answered as recorded, and the history after it, byte for byte |
| `laws.rs` | the scripted session with four seeds: the history replays through the reference model with no divergence, the laws hold over it and the replayed state, and the core's `/v1/laws` agrees; one changed answer is caught by the replay and the laws; the same seed gives the same history and another seed another |
| `force.rs` | a reachable state reads back over HTTP; an unreachable one is refused with its law and changes nothing; a document that is not a forced state is refused |
| `faults.rs` | every declared fault as reqwest sees it, `applied` included; undeclared faults and parameters refused |
| `clients.rs` | typed reqwest clients of the Petstore and GitHub, curl, and a Python client generated from the Petstore's document by openapi-generator (run when `make openapi-client-http` has installed it, and required with `LAWTWIN_REQUIRE_CLIENTS=1`) |

The Rust core passes all seventeen tests in process, as a program and as the
image.

## The golden sessions

`twins/http/shared/golden/<contract>-42.session.jsonl` is the scripted
session recorded on the Rust core with seed 42, a frozen clock and the
advertised address `lawtwin.golden:8080`: every resource's law sequence,
every other operation, requests that break the document, a path and a
method with no operation, forced states the operations reach and do not,
and every fault, with the clock moved between. Each line is a step: an API
request with its response's status, body and the headers `Content-Type`,
`Location`, `Retry-After`, `Allow` and `Date`, or an admin call with its
JSON answer. `<contract>-42.history.jsonl` is the history after it.
`make golden-http` records them again, from the Rust core, when the twin
changes on purpose.
