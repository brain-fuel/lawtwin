---
id: lawtwin.how-to.test-an-app-with-the-client-twin
kind: how-to
title: Test an application's API with the client twin
---
# Test an application's API with the client twin

The client twin is a simulated peer: it calls your application's API as
the contract's clients would, with traffic drawn from a seed, and checks
every answer against the contract's laws (ref:DEC-openapi-client-twin).
Use it when your application serves an API that an imported contract
describes.

## Run it

```sh
lawtwin peer http call --contract petstore --target http://localhost:8000
```

| Option | Default | Does |
| --- | --- | --- |
| `--contract <name>` | | the contract: its name (`petstore`), package or directory |
| `--target <url>` | | the application's base URL, such as `http://localhost:8000/api/v3` |
| `--seed <n>` | `0` | the seed every value is drawn from; the same seed sends the same traffic |
| `--rounds <n>` | `3` | how many times each resource's sequence runs |
| `--invalid` | | also send requests that break the document: a body that is not JSON, a required field left out, a field of the wrong type, a value that breaks its format, a path parameter of the wrong type |
| `--retries` | | also send each successful create again, as a client that lost the answer does |
| `--duplicates` | | also send each create twice at the same time |
| `--timeouts` | | also hang up on a create before its answer, then look for what it did |
| `--all-operations` | | also call every operation that is not part of a resource |
| `--timeout <duration>` | `10s` | how long to wait for each answer |
| `--report <file>` | | write the report as JSON |

For each resource of the contract it walks the sequences the laws are
about: create, read, list, update, replace, delete, read. Every answer is
read as the model's (`{"ok": ...}` or `{"error": ...}`) and the contract's
laws are checked over all of them, with the check a twin runs over its own
history. It reports three things:

| Kind | Is |
| --- | --- |
| violation | a law of the contract broken, with its title |
| finding | something the contract forbids that is not a law: a request that breaks the document accepted, or answered 5xx; a create retried or duplicated that fails with 5xx; a request with no answer |
| problem | an answer that breaks the document: an undocumented status, a body not of its type, a missing `Content-Type` or required header |

It exits 0 when there is none of them, and 1 otherwise. An operation the
application answers 501 is taken as not served, and not sent broken
requests.

## Example

`examples/http/petstore-app/app.py` is a small Petstore written by hand,
with bugs that `--bug` switches on:

```sh
python3 examples/http/petstore-app/app.py --port 8000 &
lawtwin peer http call --contract petstore --target http://localhost:8000 \
  --seed 6 --invalid --retries --duplicates --timeouts
```

```text
lawtwin peer http call: lawtwin.petstore against http://localhost:8000: 127 requests, 7 laws checked over 49 calls; 0 violation(s), 0 finding(s), 0 problem(s)
```

With `--bug forget-delete`, a delete answers 200 and keeps the record:

```text
lawtwin peer http call: lawtwin.petstore against http://localhost:8000: 16 requests, 7 laws checked over 16 calls; 3 violation(s), 0 finding(s), 0 problem(s)
  violation: `a deleted pet is not found`: getPetById [66428520] answered {"ok":{...}} after a delete, not NotFound
```

`--bug lose-field` breaks *a created pet reads back* or *a replaced pet
reads back as replaced*; `--bug crash-invalid` gives findings: `addPet
failed with 500 on a request that breaks the document (the body is not
JSON), rather than refusing it`.

The client twin checks the HTTP twin the same way; a twin started with
`--mutant skip-delete` breaks *a deleted pet is not found*.
