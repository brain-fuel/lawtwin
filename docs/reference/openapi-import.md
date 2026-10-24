---
id: lawtwin.reference.openapi-import
kind: reference
title: The OpenAPI importer
---
# The OpenAPI importer

`lawtwin import openapi` reads an OpenAPI 3.0 or 3.1 document
(ref:openapi-3-0) (ref:openapi-3-1), in JSON or YAML, and writes a LawSpec
dependency-contract package for the service it describes (ref:lawspec). The
package states the service's types, its operations and their failures, a
reference model of the resources the importer recognises, and laws over
them. It names no application, and nothing about twins
(ref:DEC-dependency-contracts-in-lawspec).

## Command

```sh
lawtwin import openapi <document> --package <name> --out <directory>
        [--version <x.y.z>] [--tag <tag>]... [--path <path>]...
```

| Option | Meaning |
| --- | --- |
| `<document>` | The OpenAPI document. JSON when it starts with `{`, YAML otherwise. |
| `--package` | The package name, lower-case words separated by dots, such as `lawtwin.petstore`. It is the root unit's name. |
| `--out` | The package's directory, such as `contracts/petstore`. |
| `--version` | The package version. The default is `0.1.0`. |
| `--tag` | Keep only operations with this tag. May be repeated. |
| `--path` | Keep only operations at this path or under it. May be repeated. |

Only local references (`#/...`) are followed. Swagger 2.0 is refused.

## What it writes

| File | Written | Holds |
| --- | --- | --- |
| `lawspec.edn` | every import | the package's name, version and sources |
| `src/model.lawspec` | every import | unit `<name>.model`: the types, the failures, the constraints, the state, and a definition for each operation the importer models |
| `src/api.lawspec` | every import | unit `<name>`: the `Service` resource, one ability per tag with its laws, and the handler `reference` |
| `src/handwritten.lawspec` | once | unit `<name>.handwritten`: a definition for each operation the importer cannot model, failing with `Unmodelled` until you write it |
| `src/laws.lawspec` | once | unit `<name>.laws`: your own laws about the service |
| `lawtwin-openapi.json` | every import | the wire map: each operation's method, path, parameters, media types and failing statuses, each type's wire names, and each resource |
| `lawtwin.json` | once | the twin file: the HTTP twin's faults and their effect on each of the package's laws, and what forcing a state refuses ([the HTTP twin](http-twin.md#faults)) |
| `test/` | once, except the native handlers | the test project that runs the laws on Rust and Python |

A file written once is yours: the importer never writes it again. When a
new import has an operation that `src/handwritten.lawspec` lacks, the
importer names it. The hand-written unit writes the model's types
qualified, as `model.Pet`, so a new import does not break it.

LawSpec never reads the wire map. The HTTP twin, the client twin, record
and replay, and conformance runs use it to put each request and response
into the package's names. Besides each operation, type and resource, it
gives:

| Where | Field | Is |
| --- | --- | --- |
| a record's field | `required` | whether the property must be present; a required `Maybe` field is nullable, and written `null` when it holds nothing |
| a parameter or field | `constraints` | its checks: `minimum`, `maximum`, `minLength`, `maxLength`, `regex` (with its `origin`, such as `format date-time`), `finite`, `minItems`, `maxItems`, `oneOf` |
| an operation's `response` | `headers` | each documented header: its name, whether it is required, its schema and its example |
| an operation's failure | `body` | the documented body's media type and schema, with its references written out |
| a resource | `keyType`, `parents`, `counterField`, `replaceByBodyKey` | the type it is kept under (with its fields, for a key record), the collection's path parameters, the State field with the next key, and whether its replace takes the key from the body |
| a resource | `laws` | each generated law's title, by kind: `createdReadsBack`, `deletedIsNotFound`, `listHoldsCreated`, `updateChangesOnlyGivenFields`, `replacedReadsBack` |
| the map | `state` | the model's `State`, field by field, in its constructor's order |

## Types

| Schema | LawSpec |
| --- | --- |
| `type: object` with `properties` | a record: one constructor, a field per property |
| a required property | a field of the property's type |
| an optional property | `Maybe` of it |
| `nullable: true` (3.0), `type: [T, "null"]` (3.1), a `null` alternative of `oneOf` or `anyOf`, or `null` in an `enum` | `Maybe` of it |
| a string `enum`, or a string `const` | a sum with a constructor per value |
| `oneOf` or `anyOf` of strings | a sum with a constructor per value |
| `oneOf` or `anyOf` otherwise | a sum with a constructor per alternative, holding it as `payload` |
| `allOf` | one record with the properties of every part |
| `type: array` | `List` of its items |
| `additionalProperties` with a schema, and no properties | `KeyVal Text` of it |
| a free-form object, or a value of several types | `Text`, holding the JSON |
| `type: string` | `Text` |
| `format: binary`, and an `application/octet-stream` body | `Bytes` |
| `type: integer` with `format: int32` | `Int32` |
| `type: integer` otherwise | `Int64` |
| `type: number` | `Float64` |
| `type: boolean` | `Bool` |
| no content, or `type: null` | `Unit` |

A schema under `components/schemas` is named after its key, in upper camel
case: `nullable-simple-user` is `NullableSimpleUser`. An inline object, enum
or union is named after where it is met: `IssuesCreateLabelBody` is the
body of `issues/create-label`, and `PetStatus` is the field `status` of
`Pet`. An inline enum with the values of an enum already made is that
enum. A name LawSpec or a failure already uses gets a suffix, `Schema` or
`Object`.

Field and parameter names are lower camel case: `node_id` is `nodeId`. A
name that is a keyword of LawSpec or of a target language gets a trailing
underscore: `default` is `default_`. The model's comments name each field's
wire name, and the wire map holds them all.

## Constraints

A schema's constraints do not change its type; they are kept as
validation (ref:DEC-openapi-constraints-as-validation).

| Constraint | Check on a value `v` |
| --- | --- |
| `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum` (both forms) | `v >= n`, `v <= n`, `v > n`, `v < n` |
| `minLength`, `maxLength` | `prelude.length v >= n`, `prelude.length v <= n` |
| `pattern` | `v matches regex "..."` |
| `format` `date-time`, `date`, `time`, `uuid`, `email`, `uri`, `url`, `ipv4`, `hostname`, `byte` | `v matches regex "..."`, a regex for the format |
| `type: number` | `prelude.isFinite v`: JSON has no NaN or infinity |
| `minItems`, `maxItems` | `prelude.length v >= n`, `prelude.length v <= n` |
| an integer `enum` | `v` equals one of the values |

A `pattern` is an ECMAScript regex that may match anywhere in the text; a
LawSpec regex is portable and matches the whole text. The importer drops
`^` and `$`, and pads an unanchored end with `.*`. A pattern that uses
what the portable dialect leaves out (lookaround, backreferences, word
boundaries, Unicode classes, lazy quantifiers, flags, an anchor inside the
pattern) is not checked, and the importer says so.

The checks become:

- **named refinements** in the model unit, one for each constrained
  property a law draws, such as `refinement OrderShipDate is (v :: Text
  where v matches regex "...") end`. Laws draw inputs from them, so the
  values they send are valid;
- **validation definitions**, `valid<Type>`, for each request body with a
  constraint anywhere in it. The model checks each request's parameters
  and body against them, and refuses one that breaks a constraint with
  `UnprocessableEntity` when the operation documents 422, otherwise with
  `BadRequest`.

## Operations and abilities

Each operation is an operation of the ability named after its first tag,
`<Tag>Api`; operations with no tag are in `Api`. The operation's name is
its `operationId` in lower camel case (`issues/create-label` is
`issuesCreateLabel`), or the method and path when it has none.

Its arguments are its path parameters in the order the path names them,
then its query, header and cookie parameters, then its request body. An
optional parameter or body is `Maybe`. Its result is the type of its
success response: `200`, else `201`, else another `2xx`, else `2XX`; `Unit`
when the response has no content. A body or response is read from
`application/json`, else any `+json` type, else a form, text or binary
type, else the first.

Every operation `fails with ApiError`. `ApiError` has a constructor for each
failing status the document names anywhere (`400` `BadRequest`, `404`
`NotFound`, `409` `Conflict`, `422` `UnprocessableEntity`, `429`
`TooManyRequests`, `503` `ServiceUnavailable`, and the rest; `4XX`
`ClientError`, `5XX` `ServerError`; an unnamed status `Status<code>`), and
always `BadRequest`, `NotFound` and `Unmodelled`, which the model needs. A
`default` response is not a typed failure; the importer says so.

Every ability is declared `with Service`: each case's native handlers are
made with its one `Service`, so what one operation changes the next one
sees, as with the Kafka contract's `Cluster`
(ref:DEC-contract-test-shared-cluster).

## Resources

The importer recognises a resource from a `GET` on an item path, a path
ending in a parameter, such as `/pet/{petId}`, whose response is a record
(ref:DEC-openapi-resources). The path before the last parameter is the
collection, such as `/pet`.

| Role | Operation | Model |
| --- | --- | --- |
| read | `GET` on the item | the record kept under the key, or `NotFound` |
| create | `POST` on the collection with a record body, answering the record or nothing | keeps the record built from the body under its key |
| list | `GET` on the collection answering a list of the record | the records kept under the parents, in key order |
| replace | `PUT` on the item; or `PUT` or `PATCH` on the collection whose body is the record and carries the key | replaces a kept record, or `NotFound` |
| update | `PATCH` on the item with a record body | changes the fields the body gives, or `NotFound` |
| delete | `DELETE` on the item | removes the record, or `NotFound` |

**The key.** A record's field holds the key when it is named like the
item path's parameter: the same name (`username`), the record's name then
the field (`petId` and `id`, `milestone_number` and `number`), or any name
ending in `id` for `id`. A create takes the key:

- from a required field of the body, such as a label's `name`;
- from an optional field of the body when given, and otherwise from a
  counter the service keeps, when the key is an integer, such as a pet's
  `id`;
- from an optional field of the body, which the create must then give,
  when the key is text, such as a user's `username`;
- from the counter alone, when the body has no such field and the record
  has it, such as a milestone's `number`.

A key already taken fails with the first of `Conflict`,
`UnprocessableEntity` and `BadRequest` the create documents, and otherwise
is overwritten.

**Parents.** The collection's own path parameters, such as `owner` and
`repo` in `/repos/{owner}/{repo}/labels`, are part of the key: the
resource is kept under a key record, `LabelKey owner repo name`, and a
list gives the records under one parent.

**Building a record.** A create or replace whose body is the record keeps
the body, with the key in its field. A body of another type is copied field
by field: a field of the same name and type is copied, an optional one is
unwrapped or wrapped, and every other field is empty (`""`, `0`, `false`,
`Absent`, `[]`, or an enum's first value).

**An update** changes each field of the record that the body names. An
absent field leaves it, and so does a `null` one, as the two read the same.

The state, `State`, holds a `KeyVal` for each resource, and a counter for
each whose keys the service assigns. Each recognised operation is answered
by a checked definition of the model unit, `model<Operation>`, written
`addPet by model.modelAddPet`. Every other operation, such as a search or a
login, is answered by the hand-written unit.

## Laws

For each resource with a create the importer can follow to a key, it
writes these laws, in the ability of the operations they use:

| Law | States |
| --- | --- |
| a created *record* reads back | a read after a create gives what the create answered |
| a deleted *record* is not found | a read after a delete fails with `NotFound` |
| the list holds a created *record* | a list after a create holds what it answered |
| an update changes only the fields it gives to a *record* | after an update, each field the body gives has its value, and every other field is unchanged |
| a replaced *record* reads back as replaced | a read after a replace gives what the replace answered |

A law draws the body from its type, then sets each constrained top-level
field from its named refinement, so the body it sends is valid. A create
that fails claims nothing, so the laws also hold against a service that
already holds data. Every law runs on every lawful handler: the reference
model, the HTTP twin, and the real service.

Laws the document cannot state, about filters, pagination or how one
resource changes another, belong in `src/laws.lawspec`.

## The test project

`test/` runs the package's laws on Rust and Python. Its native handlers,
`test/rust/src/<name>.rs` and `test/python/src/<name>/__init__.py`, answer
through the reference model held by the case's `Service`; the importer
writes them on every import. `lawspec generate` writes the rest.

## What it handles, and what it does not yet

On the vendored documents (`openapi/`, with their licences in
`openapi/NOTICE`):

| Document | Operations | Types | Resources | Laws | Modelled |
| --- | --- | --- | --- | --- | --- |
| Swagger Petstore, OpenAPI 3.0 (ref:swagger-petstore) | 19 in 3 abilities | 8 | 3 | 7 | 11 |
| GitHub labels and milestones (ref:github-rest-api-description) | 10 in 1 ability | 12 | 2 | 8 | 10 |

Both packages compile with `lawspec check`, and their laws pass on Rust and
Python. The whole GitHub description, 1,232 operations, imports in under
a second into 2,108 types, 145 resources and 87 laws. The `lawspec-gaps` build does not finish
compiling that package within 30 minutes, so select what you test against
with `--tag` or `--path`.

Not handled yet:

- references to other files or URLs;
- `discriminator` and `mapping`: a `oneOf` is a sum, but the wire map does
  not yet say how to tell its alternatives apart;
- `not`, `if`, `then`, `else`, `dependentSchemas`, `patternProperties`,
  `prefixItems`, `unevaluatedProperties`, `uniqueItems` and `multipleOf`;
- telling an absent property from a `null` one;
- a typed body for a failure: each failure is a constructor with no payload;
- `default` responses, links, callbacks, webhooks and security schemes;
- headers of responses, and cookies beyond reading them as parameters;
- constraints of nested and list fields in law inputs: a law sets only the
  body's top-level constrained fields, so the model may refuse other draws,
  and those cases claim nothing;
- query parameters that filter or page a list: the model's list gives
  every record under the parents;
- recognising a resource from anything but a `GET` on an item path, or a
  key that is not a scalar.
