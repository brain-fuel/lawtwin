---
id: lawtwin.explanation.three-planes
kind: explanation
title: The three planes
---
# The three planes

A test often needs a dependency it does not control: a Kafka cluster, a
payment API, a mail server. The real one is hard to provision, slow to
reset, shared with other tests, and will not fail on demand. A stand-in
solves that, but a stand-in nobody checks drifts from the real thing, and a
test that passes against it proves little.

LawTwin keeps three concerns apart, so that each can be checked on its own
(ref:DEC-three-planes).

| Plane | Owns | Never contains |
| --- | --- | --- |
| **LawSpec**, the business contract | the application's laws; a dependency is an ability, and its production handler is the real client library | anything about twins, endpoints or environments |
| **LawTwin**, the stand-ins | dependency contracts as LawSpec packages; twin servers; the control plane; conformance, record and replay | business laws; deciding which environment uses what |
| **Ironframe**, the wiring | which dependency is real and which is a twin, per environment; the config service and its cloud sync; environment attestation | contracts; twin internals |

## LawSpec: the business contract

An application's spec says what the application does. When it calls Kafka,
the spec names an ability, and its laws hold for every lawful handler of it.
In production that handler is the real client library. The spec never says
"twin", and LawSpec never imports, names or calls LawTwin.

The one link is neutral. `lawspec evidence` can read an environment
attestation that any tool writes: the environment, each dependency, the
endpoint it reached, and whether that endpoint was `real` or a `stand-in`
with a declared conformance level. Evidence then reports which obligations
ran against stand-ins.

## LawTwin: the stand-ins

A dependency's behaviour is a LawSpec package of its own, kept here
(ref:DEC-dependency-contracts-in-lawspec). The Kafka contract,
`lawtwin.kafka`, states what a cluster guarantees, as laws over abilities,
and gives a stateful spec handler that is the reference model. Nothing in it
names a business unit.

A twin is a container that speaks the dependency's wire protocol
(ref:DEC-twins-wire-level-containers), written in Rust around the code
LawSpec generates from the reference model (ref:DEC-twins-in-rust). The
model the laws check is the model the twin runs.

Tests reach what the real dependency will not do through a typed control
plane (ref:DEC-typed-control-plane): force a state the contract allows, draw
a fault from a declared set, advance the clock, inspect the state and the
history. A forced state that breaks an invariant is refused.

Trust in a twin comes from four sources, and nothing else
(ref:DEC-fidelity):

- **Conformance.** The same laws run against the real dependency and against
  the twin.
- **Record and replay.** Real traffic is recorded, checked against the
  contract, and replayed through the twin.
- **Consumer contracts.** A consumer writes its expectations as laws against
  the dependency contract, run against both the twin and the provider.
- **Evidence.** What ran against a stand-in is reported as such.

## Ironframe: the wiring

Ironframe decides, per environment and per dependency, whether the
application reaches the real thing or a twin, starts the twins, serves the
connection settings, and writes the attestation. It holds no contracts and
knows nothing of a twin's internals.

## Why three

Each plane changes for its own reasons. Business laws change with the
business; a dependency contract changes when the dependency does, or when
conformance finds a divergence; the wiring changes with the environments.
Kept apart, a business law is never weakened to suit a stand-in, a stand-in
is never shaped by one application, and an environment can swap a twin for
the real thing without touching either.
