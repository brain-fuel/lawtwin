---
id: lawtwin.reference.index
kind: reference
title: Reference
---
# Reference

These pages state what LawTwin's parts are and do.

- [The Kafka contract](kafka-contract.md): the package `lawtwin.kafka`, its
  abilities, its reference model and its laws.
- [The OpenAPI importer](openapi-import.md): `lawtwin import openapi`, the
  package it writes, how it maps schemas and operations, the resources and
  laws it makes, and what it does not handle yet.
- [The control plane](control-plane.md): the twin file, the commands and
  admin API every twin serves (force, fault, advance, inspect, history,
  replay, laws, seed), the history format and determinism. Its OpenAPI
  document is [`control-plane.openapi.yaml`](control-plane.openapi.yaml).
- [The Kafka twin](kafka-twin.md): the requests and versions it answers,
  the contract operation behind each, consumer groups, its faults and their
  effect on each law, forced states, the clients it is checked with, and
  where it differs from Kafka.
- [Kafka twin cores](kafka-core.md): a core per LawSpec target language, the
  interface every core takes, the rules that make cores agree byte for
  byte, and the conformance kit.
- [The HTTP twin](http-twin.md): how it serves an imported OpenAPI
  contract, the requests it refuses, its responses, faults and forced
  states, its history, and where it differs from a real service.
- [HTTP twin cores](http-core.md): the registry of compiled contracts, the
  core interface, determinism, the conformance kit and golden sessions.
