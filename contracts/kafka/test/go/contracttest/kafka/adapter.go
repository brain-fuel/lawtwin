// User-owned LawSpec adapter.
package kafka

// BrokerHandler is the native handler of Broker: createTopicsRequest, createPartitionsRequest, metadataRequest, produceRequest, fetchRequest, listOffsetsRequest, initProducerIdRequest, addPartitionsToTxnRequest, txnOffsetCommitRequest, endTxnRequest, joinGroupRequest, syncGroupRequest, heartbeatRequest, leaveGroupRequest, describeGroupRequest, offsetCommitRequest, offsetFetchRequest.
type BrokerHandler struct {
	Resource any
}

// NewBrokerHandler makes the native handler the generated tests use.
func NewBrokerHandler(resource any) Broker {
	return &BrokerHandler{Resource: resource}
}

func (handler *BrokerHandler) CreateTopicsRequest(value0 string, value1 int32) {
	panic("createTopicsRequest")
}

func (handler *BrokerHandler) CreatePartitionsRequest(value0 string, value1 int32) {
	panic("createPartitionsRequest")
}

func (handler *BrokerHandler) MetadataRequest(value0 string) LawSpecMaybe[int32] {
	panic("metadataRequest")
}

func (handler *BrokerHandler) ProduceRequest(value0 TopicPartition, value1 LawSpecMaybe[ProducerTag], value2 []Record) int64 {
	panic("produceRequest")
}

func (handler *BrokerHandler) FetchRequest(value0 TopicPartition, value1 int64, value2 int32, value3 IsolationLevel) FetchResult {
	panic("fetchRequest")
}

func (handler *BrokerHandler) ListOffsetsRequest(value0 TopicPartition, value1 OffsetSpec, value2 IsolationLevel) int64 {
	panic("listOffsetsRequest")
}

func (handler *BrokerHandler) InitProducerIdRequest(value0 LawSpecMaybe[string]) ProducerIdAndEpoch {
	panic("initProducerIdRequest")
}

func (handler *BrokerHandler) AddPartitionsToTxnRequest(value0 string, value1 int64, value2 int16, value3 []TopicPartition) {
	panic("addPartitionsToTxnRequest")
}

func (handler *BrokerHandler) TxnOffsetCommitRequest(value0 string, value1 int64, value2 int16, value3 string, value4 []OffsetEntry) {
	panic("txnOffsetCommitRequest")
}

func (handler *BrokerHandler) EndTxnRequest(value0 string, value1 int64, value2 int16, value3 bool) {
	panic("endTxnRequest")
}

func (handler *BrokerHandler) JoinGroupRequest(value0 string, value1 string, value2 []string) int32 {
	panic("joinGroupRequest")
}

func (handler *BrokerHandler) SyncGroupRequest(value0 string, value1 string, value2 int32) []TopicPartition {
	panic("syncGroupRequest")
}

func (handler *BrokerHandler) HeartbeatRequest(value0 string, value1 string, value2 int32) {
	panic("heartbeatRequest")
}

func (handler *BrokerHandler) LeaveGroupRequest(value0 string, value1 string) {
	panic("leaveGroupRequest")
}

func (handler *BrokerHandler) DescribeGroupRequest(value0 string) GroupView {
	panic("describeGroupRequest")
}

func (handler *BrokerHandler) OffsetCommitRequest(value0 string, value1 string, value2 int32, value3 []OffsetEntry) {
	panic("offsetCommitRequest")
}

func (handler *BrokerHandler) OffsetFetchRequest(value0 string, value1 TopicPartition) LawSpecMaybe[int64] {
	panic("offsetFetchRequest")
}

// AdminHandler is the native handler of Admin: createTopic, createPartitions, partitionsFor, listOffsets.
type AdminHandler struct {
	Resource any
}

// NewAdminHandler makes the native handler the generated tests use.
func NewAdminHandler(resource any) Admin {
	return &AdminHandler{Resource: resource}
}

func (handler *AdminHandler) CreateTopic(value0 string, value1 int32) {
	panic("createTopic")
}

func (handler *AdminHandler) CreatePartitions(value0 string, value1 int32) {
	panic("createPartitions")
}

func (handler *AdminHandler) PartitionsFor(value0 string) LawSpecMaybe[int32] {
	panic("partitionsFor")
}

func (handler *AdminHandler) ListOffsets(value0 TopicPartition, value1 OffsetSpec, value2 IsolationLevel) int64 {
	panic("listOffsets")
}

// PartitionerHandler is the native handler of Partitioner: choosePartition.
type PartitionerHandler struct {
	Resource any
}

// NewPartitionerHandler makes the native handler the generated tests use.
func NewPartitionerHandler(resource any) Partitioner {
	return &PartitionerHandler{Resource: resource}
}

func (handler *PartitionerHandler) ChoosePartition(value0 LawSpecMaybe[[]byte], value1 int32) int32 {
	panic("choosePartition")
}

// ProducerHandler is the native handler of Producer: send, flush, enableIdempotence, retryLastSend, producerIdentity.
type ProducerHandler struct {
	Resource any
}

// NewProducerHandler makes the native handler the generated tests use.
func NewProducerHandler(resource any) Producer {
	return &ProducerHandler{Resource: resource}
}

func (handler *ProducerHandler) Send(value0 ProducerRecord) LawSpecEither[KafkaError, RecordMetadata] {
	panic("send")
}

func (handler *ProducerHandler) Flush() {
	panic("flush")
}

func (handler *ProducerHandler) EnableIdempotence() LawSpecEither[KafkaError, ProducerIdAndEpoch] {
	panic("enableIdempotence")
}

func (handler *ProducerHandler) RetryLastSend() LawSpecEither[KafkaError, RecordMetadata] {
	panic("retryLastSend")
}

func (handler *ProducerHandler) ProducerIdentity() LawSpecMaybe[ProducerIdAndEpoch] {
	panic("producerIdentity")
}

// ConsumerHandler is the native handler of Consumer: assign, subscribe, poll, commitSync, seek, position, committedOffset, setIsolationLevel, close.
type ConsumerHandler struct {
	Resource any
}

// NewConsumerHandler makes the native handler the generated tests use.
func NewConsumerHandler(resource any) Consumer {
	return &ConsumerHandler{Resource: resource}
}

func (handler *ConsumerHandler) Assign(value0 []TopicPartition) {
	panic("assign")
}

func (handler *ConsumerHandler) Subscribe(value0 string, value1 string, value2 []string) LawSpecEither[KafkaError, []TopicPartition] {
	panic("subscribe")
}

func (handler *ConsumerHandler) Poll(value0 int32) LawSpecEither[KafkaError, []ConsumerRecord] {
	panic("poll")
}

func (handler *ConsumerHandler) CommitSync() LawSpecEither[KafkaError, LawSpecUnit] {
	panic("commitSync")
}

func (handler *ConsumerHandler) Seek(value0 TopicPartition, value1 int64) LawSpecEither[KafkaError, LawSpecUnit] {
	panic("seek")
}

func (handler *ConsumerHandler) Position(value0 TopicPartition) LawSpecEither[KafkaError, int64] {
	panic("position")
}

func (handler *ConsumerHandler) CommittedOffset(value0 TopicPartition) LawSpecMaybe[int64] {
	panic("committedOffset")
}

func (handler *ConsumerHandler) SetIsolationLevel(value0 IsolationLevel) {
	panic("setIsolationLevel")
}

func (handler *ConsumerHandler) Close() {
	panic("close")
}

// GroupCoordinatorHandler is the native handler of GroupCoordinator: joinGroup, syncGroup, heartbeat, leaveGroup, describeGroup, commitOffsets, fetchOffset.
type GroupCoordinatorHandler struct {
	Resource any
}

// NewGroupCoordinatorHandler makes the native handler the generated tests use.
func NewGroupCoordinatorHandler(resource any) GroupCoordinator {
	return &GroupCoordinatorHandler{Resource: resource}
}

func (handler *GroupCoordinatorHandler) JoinGroup(value0 string, value1 string, value2 []string) int32 {
	panic("joinGroup")
}

func (handler *GroupCoordinatorHandler) SyncGroup(value0 string, value1 string, value2 int32) []TopicPartition {
	panic("syncGroup")
}

func (handler *GroupCoordinatorHandler) Heartbeat(value0 string, value1 string, value2 int32) {
	panic("heartbeat")
}

func (handler *GroupCoordinatorHandler) LeaveGroup(value0 string, value1 string) {
	panic("leaveGroup")
}

func (handler *GroupCoordinatorHandler) DescribeGroup(value0 string) GroupView {
	panic("describeGroup")
}

func (handler *GroupCoordinatorHandler) CommitOffsets(value0 string, value1 string, value2 int32, value3 []OffsetEntry) {
	panic("commitOffsets")
}

func (handler *GroupCoordinatorHandler) FetchOffset(value0 string, value1 TopicPartition) LawSpecMaybe[int64] {
	panic("fetchOffset")
}

// TransactionsHandler is the native handler of Transactions: initTransactions, beginTransaction, addPartitionsToTransaction, sendInTransaction, sendOffsetsToTransaction, commitTransaction, abortTransaction.
type TransactionsHandler struct {
	Resource any
}

// NewTransactionsHandler makes the native handler the generated tests use.
func NewTransactionsHandler(resource any) Transactions {
	return &TransactionsHandler{Resource: resource}
}

func (handler *TransactionsHandler) InitTransactions(value0 string) LawSpecEither[KafkaError, ProducerIdAndEpoch] {
	panic("initTransactions")
}

func (handler *TransactionsHandler) BeginTransaction() LawSpecEither[KafkaError, LawSpecUnit] {
	panic("beginTransaction")
}

func (handler *TransactionsHandler) AddPartitionsToTransaction(value0 []TopicPartition) LawSpecEither[KafkaError, LawSpecUnit] {
	panic("addPartitionsToTransaction")
}

func (handler *TransactionsHandler) SendInTransaction(value0 ProducerRecord) LawSpecEither[KafkaError, RecordMetadata] {
	panic("sendInTransaction")
}

func (handler *TransactionsHandler) SendOffsetsToTransaction(value0 string, value1 []OffsetEntry) LawSpecEither[KafkaError, LawSpecUnit] {
	panic("sendOffsetsToTransaction")
}

func (handler *TransactionsHandler) CommitTransaction() LawSpecEither[KafkaError, LawSpecUnit] {
	panic("commitTransaction")
}

func (handler *TransactionsHandler) AbortTransaction() LawSpecEither[KafkaError, LawSpecUnit] {
	panic("abortTransaction")
}
