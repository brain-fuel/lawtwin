---
id: lawtwin.reference.http-twin
kind: reference
title: The HTTP twin
---
# The HTTP twin

The HTTP twin is an API server that applications reach with their usual
HTTP client and base URL (ref:DEC-twins-wire-level-containers). It serves
any service imported with `lawtwin import openapi`
([the importer](openapi-import.md)): what it answers is decided by that
contract's reference model, as LawSpec generates it, and the contract's
wire map, `lawtwin-openapi.json`, says how each request and answer looks
on the wire (ref:DEC-embed-generated-model). One core serves any compiled
contract, chosen when it starts ([HTTP twin cores](http-core.md),
ref:DEC-http-contract-registry). It serves LawTwin's
[control plane](control-plane.md) on its admin port.

## What it answers

A request is routed by its method and path to an operation of the wire
map. A literal path segment wins over a parameter, so `/pet/findByStatus`
is not `/pet/{petId}`. The request's path, query, header and cookie
parameters and its body are read as the operation's arguments, each as a
value of its type, and the operation runs on the model.

A request that is not a call of the contract is refused before it reaches
the model (ref:DEC-http-refusals):

| Request | Answer |
| --- | --- |
| no operation at the path | 404 |
| an operation at the path, but not with the method | 405, with `Allow` listing the methods |
| a body that is not JSON | 400 |
| a missing required parameter or field, a value of the wrong type, an unknown enum value | 422 when the operation documents 422 and not 400; otherwise 400 |
| a value that breaks a constraint (a pattern, a format, a bound) | the model's refusal: `UnprocessableEntity` or `BadRequest`, as the importer wrote it |

A refusal's body is the one the document gives that status, built from
the schema in the wire map: every required property, and `message`,
`status`, `code` and `error` when the schema has them. When the document
gives no body, it is `{"message": "<why>"}`. A refused request is written
to the history as a `request refused` control entry.

The model's answer becomes the response:

| Model's answer | Response |
| --- | --- |
| a value | the operation's documented success status (`201` for GitHub's creates, `204` for its deletes); the value as JSON, written by the wire map, with `Content-Type` the documented media type; no body for `Unit` |
| `ApiError` constructor | the status it stands for in the wire map, with the documented body as above |
| `Unmodelled` | 501: the operation is in `src/handwritten.lawspec` and not written yet |
| a defect of the generated code | 500, and a `model defect` control entry |

An optional property the model holds nothing for is left out; a required
nullable one is written `null`. Every response has a `Date` header from the
twin clock and a `Content-Length`. A create whose operation documents a
`Location` header gets the created record's item URL, at the advertised
address; another documented header that is required gets its documented
example, or a value drawn from the seed. Connections are kept alive unless
the client asks to close them.

## Faults

Each contract's twin file, `contracts/<name>/lawtwin.json`, declares the
faults (ref:DEC-twin-file). The importer writes it once, with the effect of
each fault on each of the contract's laws; it is the author's from then on.
A fault answers in the model's place, either before the request reaches the
model or, with `applied=true`, after it; it never changes what the model
answers (ref:DEC-http-faults). Every law not named under a fault holds
while it is active.

| Fault | Parameters | What the client sees | Effect on the laws |
| --- | --- | --- | --- |
| `latency` | `delay` (required), `jitter` (0ms), `operation`, `for` | each response held back by `delay`, plus up to `jitter` drawn from the seed | All hold: only when an answer is sent changes. |
| `server-error` | `status` (500, 500 to 599), `operation`, `count`, `probability` (100), `applied` (false), `for` | the status, with the body the document gives it | A read the fault answers is not the record, so *a created ... reads back* and *the list holds a created ...* fail for it; a write refused before the model wrote nothing; with `applied=true` the write took effect, and a client's retry finds its key taken. |
| `rate-limit` | `limit` (0), `window` (1s), `operation`, `for` | past `limit` requests in a window of the twin clock, 429 with `Retry-After`, the whole seconds left in the window | As for `server-error`, for each refused request. On the frozen clock a window ends only when a test advances the clock. |
| `timeout` | `hold` (30s), `operation`, `count`, `probability` (100), `applied` (false), `for` | no response; the connection is held for `hold` of real time, then closed | As for `server-error`. |
| `connection-reset` | `operation`, `count`, `probability` (100), `applied` (false), `for` | the connection reset, with nothing sent | As for `server-error`. |
| `slow-body` | `chunk` (16), `delay` (100ms), `operation`, `for` | the headers at once, the body `chunk` bytes at a time, `delay` apart | All hold. |

`operation` is an operation of the contract by its name, such as `addPet`;
without it a fault touches every request. `count` ends a fault after it has
answered that many requests; `for` ends it on the twin clock. Raising,
answering, clearing and ending a fault are written to the history. The
twin file names each law's effect in full; `lawtwin faults` prints it.

## Forcing a state

A forced state names records of each resource, by the record's name
(ref:DEC-force-through-operations):

```json
{ "reset": true,
  "resources": {
    "Pet": [ { "id": 7, "name": "Rex", "photoUrls": [], "status": "available" },
             { "name": "Tom", "photoUrls": [] } ],
    "Label": [ { "parents": { "owner": "o", "repo": "r" },
                 "body": { "name": "bug", "color": "d73a4a" } } ] } }
```

| Item | Reached by |
| --- | --- |
| a record, or `{"parents": ..., "record": ...}` | the resource's create, with a body built from the record, then its update or replace for what the create does not set; it must then read back as given. A record without its key gets the key the service assigns. |
| `{"parents": ..., "body": ...}` | the resource's create, with that body as it is; the record is what the create answers |

`parents` gives the collection's path parameters, such as a repository's
`owner` and `repo`. Everything runs on a copy of the model, written to the
history with the source `force`. A state the operations do not reach is
refused with the law that forbids it, and nothing changes:

```json
{ "forced": false, "law": "a created label reads back",
  "reason": "issuesCreateLabel refused it with {\"error\":\"UnprocessableEntity\"}" }
```

The GitHub contract refuses a label forced twice (the second create finds
the name taken) and a milestone forced with a number the service would not
give it (`number` is assigned from 1). A document with other parts, such
as Kafka's `topics`, is not a forced state: 400.

## The history

Each operation is written as the call it is, with its arguments by the
contract's parameter names, as JSON on the wire, and the model's answer
(ref:DEC-history-as-ability-calls):

```json
{"seq":1,"at":1767225600000,"source":"wire","op":"addPet","body":{"name":"Rex","photoUrls":["a"],"status":"available"},"answer":{"ok":{"id":1,"name":"Rex","photoUrls":["a"],"status":"available"}}}
{"seq":3,"at":1767225600000,"source":"wire","op":"getPetById","petId":2,"answer":{"error":"NotFound"}}
{"seq":4,"at":1767225600000,"source":"wire","control":"request refused","detail":{"op":"addPet","status":400,"reason":"body.name: is required"}}
```

An absent optional argument is `null`. An answer is `{"ok": value}`,
`{"error": "<ApiError>"}` or `{"defect": "..."}`. The control entries are
`request refused`, `fault raised`, `fault answered` (with the operation and
whether it was applied), `fault cleared`, `fault ended`, `clock advanced`,
`reset` and `model defect`. `lawtwin replay` runs a history through a fresh
model; `lawtwin laws` checks the contract's laws over it: a read after a
create, replace or update gives what it answered, a read after a delete is
`NotFound`, a list holds what was created under its parents, an update
changes only the fields it gives. The state check is that every record is
kept under its own key.

## A mutated twin

`--mutant <name>` (or `LAWTWIN_MUTANT`) makes the core part from its
contract on purpose, for showing that conformance, the client twin and
replay catch a twin that does: `drop-field` leaves the last property out of
every success body, and `skip-delete` answers a delete without running it.
It is for tests; never deploy it.

## Clients

The conformance kit drives the twin with typed reqwest code (ref:reqwest)
for the Petstore and for GitHub's labels and milestones, with curl
(ref:curl), with a Python client generated from the Petstore's document by
openapi-generator (ref:openapi-generator), installed by `make openapi-client-http`, and
with the client twin (`lawtwin peer http call`), each
against a fresh twin, then replays the history through the reference model
and checks the laws ([HTTP twin cores](http-core.md)).

## The image

`make image-http` builds `lawtwin/http:0.3.0` (also tagged
`docker.io/lawtwin/http:0.3.0`):
the static core and the `lawtwin` command on `scratch`, from
`twins/http/rust/Containerfile` (`make image-http`). The API listens on
8080 and the admin API on 9644. It is configured by `TWIN_CONFIG`, a JSON
object with `contract`, `seed`, `clock` and `advertise`, or by
`LAWTWIN_CONTRACT`, `LAWTWIN_SEED`, `LAWTWIN_CLOCK` and `LAWTWIN_ADVERTISE`.
`examples/http/compose.yaml` runs the Petstore and GitHub twins side by
side from the one image.

## Differences from a real service

The twin is the contract's model, and the model is what the document says
(ref:DEC-openapi-resources). Beyond that:

- An operation the importer could not model answers 501 until it is
  written in the contract's `src/handwritten.lawspec`. On the Petstore that
  is eight of nineteen: `findPetsByStatus`, `findPetsByTags`,
  `updatePetWithForm`, `uploadFile`, `getInventory`,
  `createUsersWithListInput`, `loginUser` and `logoutUser`.
- Fields the service sets itself and the document does not describe are
  empty: a GitHub label has `id` 0 and `url` `""`.
- Query parameters that filter or page a list are read and checked, and
  change nothing: a list gives every record under its parents.
- Failure bodies are not typed in the contract; the twin builds one that
  keeps the documented schema.
- No TLS, no authentication, no CORS.

Divergences from a real service are recorded in the ledger as they are
found (ref:DEC-petstore3-divergences).
