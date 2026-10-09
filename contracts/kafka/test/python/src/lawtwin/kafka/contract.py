"""The native handlers of the lawtwin.kafka contract, for running its laws.

Every handler here serves its ability from one shared cluster: the generated
reference model of the Broker ability, with the client spec handlers over it
and Kafka's own murmur2 partitioner. A law's handlers are all made before its
case runs any operation, so a handler made after an operation starts a new
case, and with it a new cluster. A real cluster or the Kafka twin takes the
place of the reference model in conformance runs.
"""
import lawspec_runtime as ls
import lawspec_schema as _schema
from lawspec_abilities.lawtwin.kafka import contract as abilities

_BROKER = "lawtwin.kafka.contract::ability::Broker"
_PARTITIONER = "lawtwin.kafka.contract::ability::Partitioner"

_session = None
_used = False


def _current():
    """The symbols of the current case: a fresh reference cluster and the
    native partitioner, installed for the client spec handlers to call."""
    global _session, _used
    if _session is None or _used:
        symbols = {}
        ls.install_handlers(symbols, {
            _BROKER: abilities.Reference(symbols),
            _PARTITIONER: PartitionerHandler(),
        })
        _session = symbols
        _used = False
    return _session


def murmur2(data: bytes) -> int:
    """Kafka's murmur2 hash, as the Java client's Utils.murmur2."""
    m = 0x5BD1E995
    mask = 0xFFFFFFFF
    length = len(data)
    h = (0x9747B28C ^ length) & mask
    for i in range(0, length - length % 4, 4):
        k = int.from_bytes(data[i:i + 4], "little")
        k = (k * m) & mask
        k ^= k >> 24
        k = (k * m) & mask
        h = (h * m) & mask
        h ^= k
    tail = data[length & ~3:]
    if len(tail) >= 3:
        h ^= tail[2] << 16
    if len(tail) >= 2:
        h ^= tail[1] << 8
    if len(tail) >= 1:
        h ^= tail[0]
        h = (h * m) & mask
    h ^= h >> 13
    h = (h * m) & mask
    h ^= h >> 15
    return h - (1 << 32) if h & 0x80000000 else h


class _Shared:
    """A handler that answers through a spec handler over the shared cluster,
    and marks the case as started when it is first used."""

    def __init__(self, inner):
        self._inner = inner

    def __getattr__(self, name):
        operation = getattr(self._inner, name)

        def call(*args):
            global _used
            _used = True
            return operation(*args)

        return call


class BrokerHandler(_Shared):
    """The Broker: the shared reference cluster."""

    def __init__(self):
        super().__init__(ls.handler(_current(), _BROKER))


class AdminHandler(_Shared):
    """Admin: the client spec handler over the shared cluster."""

    def __init__(self):
        super().__init__(abilities.KafkaAdmin(_current()))


class PartitionerHandler:
    """Partitioner: Kafka's default for keyed records, murmur2 of the key; an
    unkeyed record goes to partition 0."""

    def choosePartition(self, key, n):
        if not isinstance(key, _schema.Just) or n <= 0:
            return 0
        return (murmur2(bytes(key.value)) & 0x7FFFFFFF) % n


class ProducerHandler(_Shared):
    """Producer: the client spec handler over the shared cluster."""

    def __init__(self):
        super().__init__(abilities.KafkaProducer(_current()))


class ConsumerHandler(_Shared):
    """Consumer: the client spec handler over the shared cluster."""

    def __init__(self):
        super().__init__(abilities.KafkaConsumer(_current()))


class GroupCoordinatorHandler(_Shared):
    """GroupCoordinator: the client spec handler over the shared cluster."""

    def __init__(self):
        super().__init__(abilities.KafkaGroups(_current()))


class TransactionsHandler(_Shared):
    """Transactions: the client spec handler over the shared cluster."""

    def __init__(self):
        super().__init__(abilities.KafkaTransactions(_current()))
