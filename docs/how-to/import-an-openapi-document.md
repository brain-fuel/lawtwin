---
id: lawtwin.how-to.import-an-openapi-document
kind: how-to
title: Import an OpenAPI document
---
# Import an OpenAPI document

This guide turns an OpenAPI 3.0 or 3.1 document into a LawSpec contract
package, runs its laws, and adds what the document cannot say. [The
OpenAPI importer](../reference/openapi-import.md) says what the package
holds.

## Before you start

You need what [Run a contract's laws](run-contract-laws.md) needs for Rust
and Python, with LawSpec staged, and Rust to build `lawtwin`:

```sh
make build
```

This builds `target/release/lawtwin`.

## Import the document

Name the document, the package, and the directory to write:

```sh
target/release/lawtwin import openapi openapi/petstore3.json \
  --package lawtwin.petstore --out contracts/petstore
```

It prints what it made, and a note for each thing it could not express:

```text
lawtwin.petstore: 8 type(s), 19 operation(s) in 3 abilities, 3 resource(s), 7 law(s); 11 operation(s) modelled, 8 left to handwritten.lawspec
note: updatePet: the `default` response is not a typed failure
```

For a large document, keep only what you test against, by tag or by path:

```sh
target/release/lawtwin import openapi api.github.com.json \
  --package lawtwin.github --out contracts/github \
  --path /repos/{owner}/{repo}/labels --path /repos/{owner}/{repo}/milestones
```

## Run its laws

Make the test project's Python environment, compile the package and its
laws, generate the test project, and run the laws:

```sh
c=contracts/petstore
python3.13 -m venv $c/test/python/.venv
$c/test/python/.venv/bin/pip install -q -e "$c/test/python[test]"
node .lawspec-cli/bin/lawspec.mjs package --project $c
node .lawspec-cli/bin/lawspec.mjs check --config $c/test/lawspec.json
node .lawspec-cli/bin/lawspec.mjs generate --config $c/test/lawspec.json
node .lawspec-cli/bin/lawspec.mjs test --config $c/test/lawspec.json
```

To check it with the other contracts in `make ci`, add the directory to
`CONTRACTS` in the Makefile, and the document, package and directory to
`OPENAPI_IMPORTS`.

## Write what the importer could not model

`src/handwritten.lawspec` has a definition for each operation the importer
did not recognise, such as a search. Each fails with `Unmodelled`:

```lawspec fragment
definition findPetsByStatus (status :: model.PetStatus) (current_ :: State) :: Pair (List model.Pet) State fails with ApiError is
  raise Unmodelled
end
```

Replace the body with what the service does, over the model's `State`, and
give back the answer and the next state. Here `withStatus` is a definition
you write beside it, keeping the pets with that status:

```lawspec fragment
definition findPetsByStatus (status :: model.PetStatus) (current_ :: State) :: Pair (List model.Pet) State is
  Pair (withStatus status (prelude.values current_.pets)) current_
end
```

A definition that can no longer fail drops `fails with ApiError`: the
`lawspec-gaps` build refuses one that says it and never raises (see [the
gap report](../explanation/lawspec-gaps.md)). The file is yours: the
importer writes it once and never again.

## Add laws of your own

Write laws the document cannot state in `src/laws.lawspec`, such as that a
search finds only pets with the status asked for. The importer writes this
file once too. Run the laws again after each change.

## Import again after the document changes

Run the same command. The model unit, the facade unit, the wire map and
the native handlers are written again; your two files are kept. When the
document has an operation your hand-written unit lacks, the importer names
it:

```text
lawtwin: contracts/petstore/src/handwritten.lawspec is yours and was kept; add a definition for each of: findPetsByStatus
```

Add a definition for each, then regenerate the test project and run the
laws. `make openapi-import-check` fails when an imported contract in the
repository differs from what the importer writes.
