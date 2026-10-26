---
id: lawtwin.how-to.check-an-http-core
kind: how-to
title: Check an HTTP core with the conformance kit
---
# Check an HTTP core with the conformance kit

Every core of the HTTP twin, in any language, must pass the same
conformance kit ([HTTP twin cores](../reference/http-core.md) says what it
checks). This guide runs it.

## What you need

Rust 1.85 or later, curl, and podman to check an image. The generated
Python client needs Python 3 and podman; it runs when installed, and is
required with `LAWTWIN_REQUIRE_CLIENTS=1`:

```sh
make openapi-client-http     # generates it from openapi/petstore3.json into target/openapi-client
```

## Check the Rust core

```sh
make core-conformance-http
```

This builds `target/release/lawtwin-http` and checks it. Check the image
instead with:

```sh
make image-http
make core-conformance-http CORE=image:lawtwin/http:0.3.0
```

Without `CORE`, `cargo test -p lawtwin-http-conformance` checks the Rust
core in the test process.

## Check another core

Give the command that runs it. The kit adds the core interface's options
(`--contract`, `--host`, `--port`, `--admin-port`, `--seed`, `--clock` and
`--advertise`):

```sh
make core-conformance-http CORE="twins/http/go/lawtwin-http-go"
make core-conformance-http CORE=image:lawtwin/http-go:0.1.0
```

An image is given its settings in `TWIN_CONFIG`, and must listen on 8080
and 9644. The core must serve the `petstore` and `github` contracts.

## When a step differs from the golden session

The golden test names the first steps a core answered otherwise, with what
was recorded and what came. If the Rust core changed on purpose, record
the golden sessions again and review the difference:

```sh
make golden-http
git diff twins/http/shared/golden
```
