---
id: lawtwin.how-to.run-the-http-twin
kind: how-to
title: Run the HTTP twin and drive it from a test
---
# Run the HTTP twin and drive it from a test

This guide starts the HTTP twin for an imported contract, points an
application at it, and uses the control plane to force a state, raise a
fault and check the contract's laws. [The HTTP twin](../reference/http-twin.md)
says what it answers; [the control plane](../reference/control-plane.md)
lists every command.

## Start it

With podman, from the image:

```sh
make image-http             # builds lawtwin/http:0.2.1
podman run --rm -p 8080:8080 -p 9644:9644 \
  -e TWIN_CONFIG='{"contract": "petstore", "seed": 42}' lawtwin/http:0.2.1
```

With podman compose, from `examples/http/compose.yaml`, which runs the
Petstore twin on 8080 (admin 9644) and the GitHub twin on 8081 (admin
9645):

```sh
podman compose -f examples/http/compose.yaml up -d
```

Or as a program, from a checkout:

```sh
make build
target/release/lawtwin up http --contract petstore --seed 42
```

`lawtwin up http --container` runs the image instead. Point the
application's base URL at `http://localhost:8080` (the Petstore's paths
start at `/pet`, `/store` and `/user`). For an application in the same
compose project, set `advertise` in `TWIN_CONFIG` to the service name, such
as `petstore:8080`, so `Location` headers name it.

## Force a state

Write the records the test needs, and force them:

```sh
cat > state.json <<'JSON'
{ "reset": true,
  "resources": {
    "Pet":  [ { "id": 7, "name": "Rex", "photoUrls": [], "status": "available" } ],
    "User": [ { "username": "ada", "firstName": "Ada" } ] } }
JSON
lawtwin force state.json
curl localhost:8080/pet/7
```

Each record is reached through the contract's own operations. A state they
do not reach is refused with the law that forbids it (exit 2), and nothing
changes. For GitHub, a label's repository is its parents:

```sh
echo '{ "resources": { "Label": [ { "parents": { "owner": "o", "repo": "r" },
        "body": { "name": "bug", "color": "d73a4a" } } ] } }' \
  | lawtwin --admin http://localhost:9645 force -
```

## Raise faults

```sh
lawtwin faults                                         # the declared faults and their effect on each law
lawtwin fault latency delay=300ms
lawtwin fault server-error status=503 count=1 operation=addPet
lawtwin fault server-error applied=true operation=addPet count=1   # runs the create, then answers 500
lawtwin fault rate-limit limit=1 window=10s            # then 429 with Retry-After: 10
lawtwin fault timeout hold=5s count=1
lawtwin fault connection-reset count=1
lawtwin fault slow-body chunk=4 delay=20ms
lawtwin fault clear                                    # clear them all
```

The clock is frozen: a rate-limit window, or a fault raised with
`for=5s`, ends only when the test moves the clock:

```sh
lawtwin advance 10s
```

## Check what happened

```sh
lawtwin inspect        # each resource's records by key, the counters, the active faults
lawtwin history        # every operation, with its arguments and the model's answer
lawtwin laws           # the contract's laws over the history and the state; exit 1 if one fails
lawtwin replay         # the history through a fresh model; exit 1 on a divergence
lawtwin reset          # start again, empty, with the same seed
```

End each test with `lawtwin laws`: it fails when what the application saw
from the twin breaks one of the contract's laws.
