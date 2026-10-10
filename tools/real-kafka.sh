#!/usr/bin/env bash
# Start or stop a real Kafka broker in podman, for `make conform-kafka` and
# `make fidelity-test`: the apache/kafka image as one KRaft node (broker and
# controller), with transactions, its internal topics on one replica, and no
# initial rebalance delay. It listens on the same port inside the container
# and out, so that the broker reaches its own advertised address, which
# Kafka's transaction verification needs.
#
#   tools/real-kafka.sh up | down
#   KAFKA_IMAGE (docker.io/apache/kafka:4.3.1), KAFKA_PORT (39092),
#   KAFKA_NAME (lawtwin-real-kafka), PODMAN (podman)
set -euo pipefail
image="${KAFKA_IMAGE:-docker.io/apache/kafka:4.3.1}"
port="${KAFKA_PORT:-39092}"
name="${KAFKA_NAME:-lawtwin-real-kafka}"
podman="${PODMAN:-podman}"
case "${1:-}" in
  up)
    "$podman" rm -f "$name" >/dev/null 2>&1 || true
    "$podman" run -d --name "$name" -p "127.0.0.1:$port:$port" \
      -e KAFKA_HEAP_OPTS="-Xmx768m -Xms256m" \
      -e KAFKA_NODE_ID=1 \
      -e KAFKA_PROCESS_ROLES=broker,controller \
      -e KAFKA_LISTENERS="PLAINTEXT://:$port,CONTROLLER://:9093" \
      -e KAFKA_ADVERTISED_LISTENERS="PLAINTEXT://localhost:$port" \
      -e KAFKA_CONTROLLER_LISTENER_NAMES=CONTROLLER \
      -e KAFKA_LISTENER_SECURITY_PROTOCOL_MAP=CONTROLLER:PLAINTEXT,PLAINTEXT:PLAINTEXT \
      -e KAFKA_CONTROLLER_QUORUM_VOTERS=1@localhost:9093 \
      -e KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR=1 \
      -e KAFKA_TRANSACTION_STATE_LOG_REPLICATION_FACTOR=1 \
      -e KAFKA_TRANSACTION_STATE_LOG_MIN_ISR=1 \
      -e KAFKA_GROUP_INITIAL_REBALANCE_DELAY_MS=0 \
      "$image" >/dev/null
    for _ in $(seq 1 120); do
      if "$podman" logs "$name" 2>&1 | grep -q "Kafka Server started"; then
        echo "$image is up on 127.0.0.1:$port as $name"
        exit 0
      fi
      sleep 1
    done
    echo "$image did not start; podman logs $name says:" >&2
    "$podman" logs "$name" 2>&1 | tail -20 >&2
    exit 1
    ;;
  down)
    "$podman" rm -f "$name" >/dev/null 2>&1 || true
    ;;
  *)
    echo "usage: tools/real-kafka.sh up | down" >&2
    exit 2
    ;;
esac
