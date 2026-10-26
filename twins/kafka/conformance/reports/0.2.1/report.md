# Kafka twin 0.2.1: conformance against apache/kafka:4.3.1

Written by `lawtwin conform kafka`; do not edit. The JSON beside it has every run in full.

**Level 3 (answers)**: Level 2, and every answer of the real dependency is the reference model's, up to renaming, except divergences each accounted for by a ledger decision.

24 runs: 24 pass against the real broker and 24 against the twin; 1050 operations recorded from the real broker; 8 divergence(s); the real broker's histories satisfy every law the kit checks.

| Client | Scenario | Real broker, through the recorder | Twin |
| --- | --- | --- | --- |
| librdkafka | produce-consume | passes | passes |
| librdkafka | group-rebalance | passes; 2 divergence(s) | passes |
| librdkafka | idempotent | passes | passes |
| librdkafka | transactions | passes | passes |
| librdkafka | faults | passes | passes |
| librdkafka | force | passes | passes |
| kafkajs | produce-consume | passes | passes |
| kafkajs | group-rebalance | passes | passes |
| kafkajs | idempotent | passes | passes |
| kafkajs | transactions | passes | passes |
| kafkajs | faults | passes | passes |
| kafkajs | force | passes | passes |
| franz-go | produce-consume | passes | passes |
| franz-go | group-rebalance | passes; 4 divergence(s) | passes |
| franz-go | idempotent | passes | passes |
| franz-go | transactions | passes | passes |
| franz-go | faults | passes | passes |
| franz-go | force | passes | passes |
| java | produce-consume | passes | passes |
| java | group-rebalance | passes; 2 divergence(s) | passes |
| java | idempotent | passes | passes |
| java | transactions | passes | passes |
| java | faults | passes | passes |
| java | force | passes | passes |

## Divergences

| Kind | Count | Accounted for by | What differs |
| --- | --- | --- | --- |
| assignment by the client's assignor | 8 | ref:DEC-kafka-divergence-group-assignment | Kafka's classic protocol gives each member the partitions the leader's assignor chose; the contract's coordinator assigns round robin in member id order |

## What the wire showed beside the answers

Exchanges that are not operations of the contract, written to the history as notes.

| Note | Count | Accounted for by |
| --- | --- | --- |
| rebalance in progress | 16 | ref:DEC-kafka-divergence-rebalance-heartbeat |
| retriable error: AddPartitionsToTxn CONCURRENT_TRANSACTIONS | 8 | ref:DEC-kafka-divergence-coordinator-timing |
| retriable error: InitProducerId COORDINATOR_LOAD_IN_PROGRESS | 1 | ref:DEC-kafka-divergence-coordinator-timing |
| retriable error: InitProducerId NOT_COORDINATOR | 1 | ref:DEC-kafka-divergence-coordinator-timing |

## Levels

0. **unchecked**: No conformance run against the real dependency.
1. **lawful**: The real dependency's histories satisfy every law the kit checks, in every scenario run: the contract describes the real thing.
2. **scenarios**: Level 1, and every client scenario that passes against the twin passes against the real dependency, unchanged.
3. **answers**: Level 2, and every answer of the real dependency is the reference model's, up to renaming, except divergences each accounted for by a ledger decision.
4. **exact**: Level 3 with no divergence at all.
