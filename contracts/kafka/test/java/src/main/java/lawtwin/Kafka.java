// User-owned LawSpec adapter.
//
// The native handlers of the lawtwin.kafka contract, for running its laws.
// A case's handlers are all made with the case's one Cluster. Here the
// cluster is the generated reference model: the `cluster` spec handler for
// Broker, Admin and GroupCoordinator, with the murmur2 partitioner, and the
// client spec handlers written over them. In conformance runs a connection to
// the Kafka twin or to a real cluster takes its place.
package lawtwin;

import java.util.HashMap;
import java.util.Map;
import lawspec.runtime.LawSpecRuntime;
import lawspec.runtime.LawSpecRuntime.Value;

public final class Kafka {
  /** What a Cluster holds: the reference model, and the symbols that install it. */
  public static final class ReferenceCluster {
    final Map<String, Object> symbols = new HashMap<>();
    final lawspec.abilities.lawtwin.Kafka.Cluster model = new lawspec.abilities.lawtwin.Kafka.Cluster(symbols);
    final lawspec.abilities.lawtwin.Kafka.MurmurPartitioner partitioner = new lawspec.abilities.lawtwin.Kafka.MurmurPartitioner(symbols);

    ReferenceCluster() {
      LawSpecRuntime.installHandlers(symbols, Map.of(
          "lawtwin.kafka::ability::Broker", model,
          "lawtwin.kafka::ability::Partitioner", partitioner));
    }
  }

  private static ReferenceCluster reference(java.lang.Object resource) {
    return (ReferenceCluster) resource;
  }

  // (Unit -> lawtwin.kafka::type::Cluster)
  public static java.lang.Object connectCluster(Value value0) {
    return new ReferenceCluster();
  }

  // (lawtwin.kafka::type::Cluster -> Unit)
  public static void disconnectCluster(java.lang.Object value0) {}

  /** The native handler of Broker: createTopicsRequest, createPartitionsRequest, metadataRequest, produceRequest, fetchRequest, listOffsetsRequest, initProducerIdRequest, addPartitionsToTxnRequest, txnOffsetCommitRequest, endTxnRequest, joinGroupRequest, syncGroupRequest, heartbeatRequest, leaveGroupRequest, describeGroupRequest, offsetCommitRequest, offsetFetchRequest. */
  public static final class BrokerHandler implements lawspec.abilities.lawtwin.Kafka.Broker {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.Cluster inner;

    public BrokerHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = reference(resource).model;
    }

    @Override
    public void createTopicsRequest(java.lang.String value0, java.lang.Integer value1) {
      inner.createTopicsRequest(value0, value1);
    }

    @Override
    public void createPartitionsRequest(java.lang.String value0, java.lang.Integer value1) {
      inner.createPartitionsRequest(value0, value1);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Maybe<
            java.lang.Integer> metadataRequest(java.lang.String value0) {
      return inner.metadataRequest(value0);
    }

    @Override
    public java.lang.Long produceRequest(
        lawspec.data.TopicPartition value0,
        lawspec.runtime.LawSpecRuntime.Maybe<lawspec.data.ProducerTag> value1,
        java.util.List<lawspec.data.Record> value2
    ) {
      return inner.produceRequest(value0, value1, value2);
    }

    @Override
    public lawspec.data.FetchResult fetchRequest(
        lawspec.data.TopicPartition value0,
        java.lang.Long value1,
        java.lang.Integer value2,
        lawspec.data.IsolationLevel value3
    ) {
      return inner.fetchRequest(value0, value1, value2, value3);
    }

    @Override
    public java.lang.Long listOffsetsRequest(
        lawspec.data.TopicPartition value0,
        lawspec.data.OffsetSpec value1,
        lawspec.data.IsolationLevel value2
    ) {
      return inner.listOffsetsRequest(value0, value1, value2);
    }

    @Override
    public lawspec.data.ProducerIdAndEpoch initProducerIdRequest(
        lawspec.runtime.LawSpecRuntime.Maybe<java.lang.String> value0
    ) {
      return inner.initProducerIdRequest(value0);
    }

    @Override
    public void addPartitionsToTxnRequest(
        java.lang.String value0,
        java.lang.Long value1,
        java.lang.Short value2,
        java.util.List<lawspec.data.TopicPartition> value3
    ) {
      inner.addPartitionsToTxnRequest(value0, value1, value2, value3);
    }

    @Override
    public void txnOffsetCommitRequest(
        java.lang.String value0,
        java.lang.Long value1,
        java.lang.Short value2,
        java.lang.String value3,
        java.util.List<lawspec.data.OffsetEntry> value4
    ) {
      inner.txnOffsetCommitRequest(value0, value1, value2, value3, value4);
    }

    @Override
    public void endTxnRequest(
        java.lang.String value0,
        java.lang.Long value1,
        java.lang.Short value2,
        java.lang.Boolean value3
    ) {
      inner.endTxnRequest(value0, value1, value2, value3);
    }

    @Override
    public java.lang.Integer joinGroupRequest(
        java.lang.String value0,
        java.lang.String value1,
        java.util.List<java.lang.String> value2
    ) {
      return inner.joinGroupRequest(value0, value1, value2);
    }

    @Override
    public java.util.List<
            lawspec.data.TopicPartition> syncGroupRequest(
        java.lang.String value0,
        java.lang.String value1,
        java.lang.Integer value2
    ) {
      return inner.syncGroupRequest(value0, value1, value2);
    }

    @Override
    public void heartbeatRequest(
        java.lang.String value0,
        java.lang.String value1,
        java.lang.Integer value2
    ) {
      inner.heartbeatRequest(value0, value1, value2);
    }

    @Override
    public void leaveGroupRequest(java.lang.String value0, java.lang.String value1) {
      inner.leaveGroupRequest(value0, value1);
    }

    @Override
    public lawspec.data.GroupView describeGroupRequest(java.lang.String value0) {
      return inner.describeGroupRequest(value0);
    }

    @Override
    public void offsetCommitRequest(
        java.lang.String value0,
        java.lang.String value1,
        java.lang.Integer value2,
        java.util.List<lawspec.data.OffsetEntry> value3
    ) {
      inner.offsetCommitRequest(value0, value1, value2, value3);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Maybe<
            java.lang.Long> offsetFetchRequest(
        java.lang.String value0,
        lawspec.data.TopicPartition value1
    ) {
      return inner.offsetFetchRequest(value0, value1);
    }
  }

  /** The native handler of Admin: createTopic, createPartitions, partitionsFor, listOffsets. */
  public static final class AdminHandler implements lawspec.abilities.lawtwin.Kafka.Admin {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.Cluster inner;

    public AdminHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = reference(resource).model;
    }

    @Override
    public void createTopic(java.lang.String value0, java.lang.Integer value1) {
      inner.createTopic(value0, value1);
    }

    @Override
    public void createPartitions(java.lang.String value0, java.lang.Integer value1) {
      inner.createPartitions(value0, value1);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Maybe<
            java.lang.Integer> partitionsFor(java.lang.String value0) {
      return inner.partitionsFor(value0);
    }

    @Override
    public java.lang.Long listOffsets(
        lawspec.data.TopicPartition value0,
        lawspec.data.OffsetSpec value1,
        lawspec.data.IsolationLevel value2
    ) {
      return inner.listOffsets(value0, value1, value2);
    }
  }

  /** The native handler of Partitioner: choosePartition. */
  public static final class PartitionerHandler implements lawspec.abilities.lawtwin.Kafka.Partitioner {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.MurmurPartitioner inner;

    public PartitionerHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = reference(resource).partitioner;
    }

    @Override
    public java.lang.Integer choosePartition(
        lawspec.runtime.LawSpecRuntime.Maybe<byte[]> value0,
        java.lang.Integer value1
    ) {
      return inner.choosePartition(value0, value1);
    }
  }

  /** The native handler of Producer: send, flush, enableIdempotence, retryLastSend, producerIdentity. */
  public static final class ProducerHandler implements lawspec.abilities.lawtwin.Kafka.Producer {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.KafkaProducer inner;

    public ProducerHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = new lawspec.abilities.lawtwin.Kafka.KafkaProducer(reference(resource).symbols);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            lawspec.data.RecordMetadata> send(lawspec.data.ProducerRecord value0) {
      return inner.send(value0);
    }

    @Override
    public void flush() {
      inner.flush();
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, lawspec.data.ProducerIdAndEpoch> enableIdempotence() {
      return inner.enableIdempotence();
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, lawspec.data.RecordMetadata> retryLastSend() {
      return inner.retryLastSend();
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Maybe<
            lawspec.data.ProducerIdAndEpoch> producerIdentity() {
      return inner.producerIdentity();
    }
  }

  /** The native handler of Consumer: assign, subscribe, poll, commitSync, seek, position, committedOffset, setIsolationLevel, close. */
  public static final class ConsumerHandler implements lawspec.abilities.lawtwin.Kafka.Consumer {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.KafkaConsumer inner;

    public ConsumerHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = new lawspec.abilities.lawtwin.Kafka.KafkaConsumer(reference(resource).symbols);
    }

    @Override
    public void assign(java.util.List<lawspec.data.TopicPartition> value0) {
      inner.assign(value0);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            java.util.List<
                    lawspec.data.TopicPartition>> subscribe(
        java.lang.String value0,
        java.lang.String value1,
        java.util.List<java.lang.String> value2
    ) {
      return inner.subscribe(value0, value1, value2);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            java.util.List<lawspec.data.ConsumerRecord>> poll(java.lang.Integer value0) {
      return inner.poll(value0);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, lawspec.runtime.LawSpecRuntime.Value> commitSync() {
      return inner.commitSync();
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            lawspec.runtime.LawSpecRuntime.Value> seek(
        lawspec.data.TopicPartition value0,
        java.lang.Long value1
    ) {
      return inner.seek(value0, value1);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, java.lang.Long> position(lawspec.data.TopicPartition value0) {
      return inner.position(value0);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Maybe<
            java.lang.Long> committedOffset(lawspec.data.TopicPartition value0) {
      return inner.committedOffset(value0);
    }

    @Override
    public void setIsolationLevel(lawspec.data.IsolationLevel value0) {
      inner.setIsolationLevel(value0);
    }

    @Override
    public void close() {
      inner.close();
    }
  }

  /** The native handler of GroupCoordinator: joinGroup, syncGroup, heartbeat, leaveGroup, describeGroup, commitOffsets, fetchOffset. */
  public static final class GroupCoordinatorHandler implements lawspec.abilities.lawtwin.Kafka.GroupCoordinator {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.Cluster inner;

    public GroupCoordinatorHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = reference(resource).model;
    }

    @Override
    public java.lang.Integer joinGroup(
        java.lang.String value0,
        java.lang.String value1,
        java.util.List<java.lang.String> value2
    ) {
      return inner.joinGroup(value0, value1, value2);
    }

    @Override
    public java.util.List<
            lawspec.data.TopicPartition> syncGroup(
        java.lang.String value0,
        java.lang.String value1,
        java.lang.Integer value2
    ) {
      return inner.syncGroup(value0, value1, value2);
    }

    @Override
    public void heartbeat(
        java.lang.String value0,
        java.lang.String value1,
        java.lang.Integer value2
    ) {
      inner.heartbeat(value0, value1, value2);
    }

    @Override
    public void leaveGroup(java.lang.String value0, java.lang.String value1) {
      inner.leaveGroup(value0, value1);
    }

    @Override
    public lawspec.data.GroupView describeGroup(java.lang.String value0) {
      return inner.describeGroup(value0);
    }

    @Override
    public void commitOffsets(
        java.lang.String value0,
        java.lang.String value1,
        java.lang.Integer value2,
        java.util.List<lawspec.data.OffsetEntry> value3
    ) {
      inner.commitOffsets(value0, value1, value2, value3);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Maybe<
            java.lang.Long> fetchOffset(
        java.lang.String value0,
        lawspec.data.TopicPartition value1
    ) {
      return inner.fetchOffset(value0, value1);
    }
  }

  /** The native handler of Transactions: initTransactions, beginTransaction, addPartitionsToTransaction, sendInTransaction, sendOffsetsToTransaction, commitTransaction, abortTransaction. */
  public static final class TransactionsHandler implements lawspec.abilities.lawtwin.Kafka.Transactions {
    public final java.lang.Object resource;
    private final lawspec.abilities.lawtwin.Kafka.KafkaTransactions inner;

    public TransactionsHandler(java.lang.Object resource) {
      this.resource = resource;
      this.inner = new lawspec.abilities.lawtwin.Kafka.KafkaTransactions(reference(resource).symbols);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            lawspec.data.ProducerIdAndEpoch> initTransactions(java.lang.String value0) {
      return inner.initTransactions(value0);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, lawspec.runtime.LawSpecRuntime.Value> beginTransaction() {
      return inner.beginTransaction();
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            lawspec.runtime.LawSpecRuntime.Value> addPartitionsToTransaction(
        java.util.List<lawspec.data.TopicPartition> value0
    ) {
      return inner.addPartitionsToTransaction(value0);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            lawspec.data.RecordMetadata> sendInTransaction(lawspec.data.ProducerRecord value0) {
      return inner.sendInTransaction(value0);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError,
            lawspec.runtime.LawSpecRuntime.Value> sendOffsetsToTransaction(
        java.lang.String value0,
        java.util.List<lawspec.data.OffsetEntry> value1
    ) {
      return inner.sendOffsetsToTransaction(value0, value1);
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, lawspec.runtime.LawSpecRuntime.Value> commitTransaction() {
      return inner.commitTransaction();
    }

    @Override
    public lawspec.runtime.LawSpecRuntime.Either<
            lawspec.data.KafkaError, lawspec.runtime.LawSpecRuntime.Value> abortTransaction() {
      return inner.abortTransaction();
    }
  }
}
