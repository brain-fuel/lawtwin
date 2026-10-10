// User-owned LawSpec adapter.
//
// The native handlers of the lawtwin.kafka contract, for running its laws.
// A case's handlers are all made with the case's one Cluster. Here the
// cluster is the generated reference model: the `cluster` spec handler for
// Broker, Admin and GroupCoordinator, with the murmur2 partitioner, and the
// client spec handlers written over them. In conformance runs a connection to
// the Kafka twin or to a real cluster takes its place.
package kafka

// ReferenceCluster is what a Cluster holds: the reference model, and the
// symbols that install it for the client spec handlers to call.
type ReferenceCluster struct {
	symbols     map[string]*LawSpecSymbol
	model       *Cluster
	partitioner *MurmurPartitioner
}

func reference(resource any) *ReferenceCluster {
	return resource.(*ReferenceCluster)
}

// ConnectCluster implements connectCluster :: (Unit -> lawtwin.kafka::type::Cluster).
func ConnectCluster(value0 LawSpecValue) any {
	symbols := map[string]*LawSpecSymbol{}
	model := NewCluster(symbols)
	partitioner := NewMurmurPartitioner(symbols)
	lsInstallHandlers(symbols, map[string]any{
		"lawtwin.kafka::ability::Broker":      Broker(model),
		"lawtwin.kafka::ability::Partitioner": Partitioner(partitioner),
	})
	return &ReferenceCluster{symbols: symbols, model: model, partitioner: partitioner}
}

// DisconnectCluster implements disconnectCluster :: (lawtwin.kafka::type::Cluster -> Unit).
func DisconnectCluster(value0 any) {}

// BrokerHandler answers through the reference model.
type BrokerHandler struct {
	Resource any
	*Cluster
}

// NewBrokerHandler makes the native handler the generated tests use.
func NewBrokerHandler(resource any) Broker {
	return &BrokerHandler{Resource: resource, Cluster: reference(resource).model}
}

// AdminHandler answers through the reference model.
type AdminHandler struct {
	Resource any
	*Cluster
}

// NewAdminHandler makes the native handler the generated tests use.
func NewAdminHandler(resource any) Admin {
	return &AdminHandler{Resource: resource, Cluster: reference(resource).model}
}

// GroupCoordinatorHandler answers through the reference model.
type GroupCoordinatorHandler struct {
	Resource any
	*Cluster
}

// NewGroupCoordinatorHandler makes the native handler the generated tests use.
func NewGroupCoordinatorHandler(resource any) GroupCoordinator {
	return &GroupCoordinatorHandler{Resource: resource, Cluster: reference(resource).model}
}

// PartitionerHandler is Kafka's murmur2 partitioner, as the contract states it.
type PartitionerHandler struct {
	Resource any
	*MurmurPartitioner
}

// NewPartitionerHandler makes the native handler the generated tests use.
func NewPartitionerHandler(resource any) Partitioner {
	return &PartitionerHandler{Resource: resource, MurmurPartitioner: reference(resource).partitioner}
}

// ProducerHandler is the client spec handler over the cluster.
type ProducerHandler struct {
	Resource any
	*KafkaProducer
}

// NewProducerHandler makes the native handler the generated tests use.
func NewProducerHandler(resource any) Producer {
	return &ProducerHandler{Resource: resource, KafkaProducer: NewKafkaProducer(reference(resource).symbols)}
}

// ConsumerHandler is the client spec handler over the cluster.
type ConsumerHandler struct {
	Resource any
	*KafkaConsumer
}

// NewConsumerHandler makes the native handler the generated tests use.
func NewConsumerHandler(resource any) Consumer {
	return &ConsumerHandler{Resource: resource, KafkaConsumer: NewKafkaConsumer(reference(resource).symbols)}
}

// TransactionsHandler is the client spec handler over the cluster.
type TransactionsHandler struct {
	Resource any
	*KafkaTransactions
}

// NewTransactionsHandler makes the native handler the generated tests use.
func NewTransactionsHandler(resource any) Transactions {
	return &TransactionsHandler{Resource: resource, KafkaTransactions: NewKafkaTransactions(reference(resource).symbols)}
}
