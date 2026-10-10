---
id: lawtwin.how-to.record-and-replay-http
kind: how-to
title: Record real HTTP traffic, replay it through the twin, and check conformance
---
# Record real HTTP traffic, replay it through the twin, and check conformance

A twin is trusted as far as it is checked against the real service
(ref:DEC-fidelity). This guide records traffic in front of a real service,
replays it through the HTTP twin, and runs the contract's laws against
both.

## Record

`lawtwin record http` is a proxy: point clients at it, and it forwards each
request to the real service and writes every exchange to a trace.

```sh
lawtwin record http --contract petstore --upstream http://localhost:8000 \
  --listen 127.0.0.1:8081 --out trace.jsonl
```

| Option | Default | Does |
| --- | --- | --- |
| `--contract <name>` | | the contract the exchanges are read by |
| `--upstream <url>` | | the real service |
| `--listen <host:port>` | `127.0.0.1:8081` | where clients reach the proxy |
| `--out <file>` | `trace.jsonl` | the trace; `-` for standard output |
| `--count <n>` | | stop after this many exchanges |

Each line of the trace is one exchange:

| Field | Is |
| --- | --- |
| `seq` | its position, from 1 |
| `request` | `method`, `target`, `headers` (as `[name, value]` pairs) and `body` (text, or `{"hex": ...}`) |
| `response` | `status`, `headers` and `body`, or `failure` when the service gave no answer (the client then gets 502) |
| `op`, `args` | the call of the contract the request is, its arguments by name |
| `answer` | the response read as the model's answer, `{"ok": ...}` or `{"error": ...}` |
| `problems` | what in the response breaks the document |
| `refused` | instead of `op`, why the request is not a call of the contract |

## Replay

```sh
lawtwin up http --contract petstore &
lawtwin replay http trace.jsonl --contract petstore --target http://localhost:8080
```

| Option | Default | Does |
| --- | --- | --- |
| `--target <url>` | `http://127.0.0.1:8080` | the twin |
| `--admin <url>` | `LAWTWIN_ADMIN`, else `http://127.0.0.1:9644` | its admin API |
| `--no-reset` | | do not reset the twin first |
| `--report <file>` | | write the report as JSON |

The twin is reset, and each recorded request is sent to it. A call is
compared in the contract's terms: its status and the model's answer it
carries; a refused request by its status
(ref:DEC-http-replay-in-contract-terms). A failure's body is the service's
own, so it is not compared. The trace itself is then checked against the
contract: the laws over its calls, and its answers against a fresh model
(`POST /v1/history/replay`). The command exits 1 on any divergence,
violation or answer the model gives differently.

## Conformance

```sh
lawtwin conform http --contract petstore --real http://localhost:8888/api/v3 \
  --twin http://localhost:8080 --twin-admin http://localhost:9644 \
  --seed 1 --rounds 2 --retries --invalid --report conform.json
```

It runs [the client twin](test-an-app-with-the-client-twin.md)'s traffic
against the real service and, with the same seed, against the twin
(reset first), and compares each answer. It takes the client twin's
`--seed`, `--rounds`, `--invalid`, `--retries`, `--duplicates`,
`--timeouts`, `--all-operations` and `--timeout`. The report gives a level:

| Level | Means |
| --- | --- |
| conformant | the laws hold on the real service, it does nothing the document forbids, and the twin answers every request as it does |
| lawful | the laws hold on the real service; it does what the document forbids, or the twin answers some requests differently |
| divergent | the real service breaks a law of the contract (exit 1) |

Start the real service empty for exact answers: keys a service assigns
depend on what it already holds.

## Results

**Against `examples/http/petstore-app`**, a Petstore written by hand: the
client twin's traffic, with retries and broken requests to the operations
it serves, recorded through the proxy, replays through the twin with 0
divergences; replayed through a twin started with `--mutant drop-field` or
`--mutant skip-delete`, it diverges, and the command fails. `conform http`
finds the twin conformant.

**Against the Swagger Petstore 3** (ref:swagger-petstore3-server): the
published image `swaggerapi/petstore3` is built for amd64 only, and its
Java 8 hangs under emulation on arm64, so
`examples/http/petstore3/Containerfile` builds it from source:

```sh
podman build -t lawtwin/petstore3-native examples/http/petstore3
podman run --rm -p 8888:8080 lawtwin/petstore3-native
```

The contract's laws hold on it. With seed 1, two rounds, `--retries` and
`--invalid`, `conform http` sends 84 requests and reports 36 divergences,
level lawful (ref:DEC-petstore3-divergences). Petstore 3:

- fills in values the document does not: `tags: []`, `userStatus: 0`,
  `quantity: 0`, `complete: false`;
- writes a ship date back as `2026-12-11T11:22:33.000+00:00`;
- overwrites a pet whose id is taken, where the model answers 422;
- gives an order sent without an id the id 0;
- accepts a pet without its required `name` (two findings);
- answers 500 for a user it cannot find, and for `POST /pet` without an id;
- answers a missing pet with 404 and the text `Pet not found`, as
  `application/xml`.

A trace recorded through it replays through the twin with 19 divergences
of 32 exchanges, and 3 problems. The twin follows the contract, not
Petstore 3's defaults.
