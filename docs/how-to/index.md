---
id: lawtwin.how-to.index
kind: how-to
title: How-to guides
---
# How-to guides

These guides show how to do one task.

- [Run a contract's laws](run-contract-laws.md): compile the dependency
  contracts and run their laws.
- [Import an OpenAPI document](import-an-openapi-document.md): make a
  contract package from an OpenAPI document, run its laws, and add what the
  document cannot say.
- [Run the Kafka twin and drive it from a test](run-the-kafka-twin.md): start
  the twin with podman, compose or as a program, force a state, raise
  faults, and check the laws over what happened.
- [Check a Kafka core with the conformance kit](check-a-kafka-core.md): run
  the kit against the Rust core, its image or another core, and record the
  golden session again.
- [Check the Kafka twin's conformance against real Kafka](check-kafka-conformance.md):
  run the kit's scenarios against `apache/kafka` or a broker of your own,
  and read the report.
- [Record real Kafka traffic and replay it through a twin core](record-and-replay-kafka.md):
  the recording proxy, replay, and a broken core caught.
- [Feed and check an application with simulated Kafka peers](feed-and-check-an-app-with-kafka-peers.md):
  produce seeded, typed records with faults, and check what an application
  writes.
- [Run the HTTP twin and drive it from a test](run-the-http-twin.md): start
  it for a contract, force records, raise faults, check the laws.
- [Test an application's API with the client twin](test-an-app-with-the-client-twin.md):
  call it with seeded, broken and faulty traffic and check its answers
  against the contract's laws.
- [Record real HTTP traffic, replay it through the twin, and check conformance](record-and-replay-http.md):
  the recording proxy, replay, and `lawtwin conform http`.
- [Check an HTTP core with the conformance kit](check-an-http-core.md): run
  the kit against a core or its image, and record the golden sessions
  again.
