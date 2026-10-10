// User-owned LawSpec adapter.
//
// The native handlers of the lawtwin.kafka contract, for running its laws.
// A case's handlers are all made with the case's one Cluster. Here the
// cluster is the generated reference model: the `cluster` spec handler for
// Broker, Admin and GroupCoordinator, with the murmur2 partitioner, and the
// client spec handlers written over them. In conformance runs a connection to
// the Kafka twin or to a real cluster takes its place.
package lawtwin

import lawspec.abilities.lawtwin.Kafka as Abilities
import lawspec.runtime.LawSpecRuntime

object Kafka {
    /** What a Cluster holds: the reference model, and the symbols that install it. */
    class ReferenceCluster {
        val symbols: MutableMap<String, Any> = mutableMapOf()
        val model = Abilities.Cluster(symbols)
        val partitioner = Abilities.MurmurPartitioner(symbols)

        init {
            LawSpecRuntime.installHandlers(
                symbols,
                mapOf(
                    "lawtwin.kafka::ability::Broker" to model,
                    "lawtwin.kafka::ability::Partitioner" to partitioner,
                ),
            )
        }
    }

    private fun reference(resource: Any): ReferenceCluster = resource as ReferenceCluster

    // (Unit -> lawtwin.kafka::type::Cluster)
    fun connectCluster(value0: kotlin.Unit): kotlin.Any = ReferenceCluster()

    // (lawtwin.kafka::type::Cluster -> Unit)
    fun disconnectCluster(value0: kotlin.Any): kotlin.Unit = Unit

    class BrokerHandler(val resource: Any) : Abilities.Broker by reference(resource).model

    class AdminHandler(val resource: Any) : Abilities.Admin by reference(resource).model

    class GroupCoordinatorHandler(val resource: Any) : Abilities.GroupCoordinator by reference(resource).model

    class PartitionerHandler(val resource: Any) : Abilities.Partitioner by reference(resource).partitioner

    class ProducerHandler(val resource: Any) : Abilities.Producer by Abilities.KafkaProducer(reference(resource).symbols)

    class ConsumerHandler(val resource: Any) : Abilities.Consumer by Abilities.KafkaConsumer(reference(resource).symbols)

    class TransactionsHandler(val resource: Any) : Abilities.Transactions by Abilities.KafkaTransactions(reference(resource).symbols)
}
