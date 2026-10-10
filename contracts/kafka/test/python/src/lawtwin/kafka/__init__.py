"""The native handlers of the lawtwin.kafka contract, for running its laws.

A case's handlers are all made with the case's one Cluster. Here the cluster
is the generated reference model: the `cluster` spec handler for Broker,
Admin and GroupCoordinator, with the murmur2 partitioner, and the client spec
handlers written over them. In conformance runs a connection to the Kafka
twin or to a real cluster takes its place.
"""
import lawspec_runtime as ls
from lawspec_abilities.lawtwin import kafka as abilities

_BROKER = "lawtwin.kafka::ability::Broker"
_PARTITIONER = "lawtwin.kafka::ability::Partitioner"


class ReferenceCluster:
    """What a Cluster holds: the reference model, and the symbols that
    install it for the client spec handlers to call."""

    def __init__(self) -> None:
        self.symbols = {}
        self.model = abilities.Cluster(self.symbols)
        self.partitioner = abilities.MurmurPartitioner(self.symbols)
        ls.install_handlers(self.symbols, {_BROKER: self.model, _PARTITIONER: self.partitioner})


def connectCluster(value0):
    return ReferenceCluster()


def disconnectCluster(value0) -> None:
    return None


class _Delegating:
    """A native handler that answers each operation through another handler."""

    def __init__(self, resource, inner) -> None:
        self.resource = resource
        self._inner = inner

    def __getattr__(self, name):
        return getattr(self._inner, name)


class BrokerHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, resource.model)


class AdminHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, resource.model)


class GroupCoordinatorHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, resource.model)


class PartitionerHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, resource.partitioner)


class ProducerHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, abilities.KafkaProducer(resource.symbols))


class ConsumerHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, abilities.KafkaConsumer(resource.symbols))


class TransactionsHandler(_Delegating):
    def __init__(self, resource) -> None:
        super().__init__(resource, abilities.KafkaTransactions(resource.symbols))
