// User-owned LawSpec adapter.
//
// The native handlers of the lawtwin.kafka contract, for running its laws.
// A case's handlers are all made with the case's one Cluster. Here the
// cluster is the generated reference model: the `cluster` spec handler for
// Broker, Admin and GroupCoordinator, with the murmur2 partitioner, and the
// client spec handlers written over them. In conformance runs a connection to
// the Kafka twin or to a real cluster takes its place.
module Lawtwin.Kafka

open LawSpec.Runtime
open LawSpec.Abilities.LawtwinKafka

/// What a Cluster holds: the reference model, the partitioner, and the
/// schema that installs them for the client spec handlers to call.
type ReferenceCluster =
    { Model: obj
      Partitioner: obj
      Schema: Schema.Schema
      Symbols: Scalar.SymbolContext }

// The spec handler classes compile after this adapter, so they are made by
// name; each implements the ability interfaces declared before it.
let private make (name: string) (schema: Schema.Schema) (symbols: Scalar.SymbolContext) : obj =
    let handler = typeof<IBroker>.Assembly.GetType("LawSpec.Handlers.LawtwinKafka." + name, true)
    System.Activator.CreateInstance(handler, [| box schema; box symbols |])

let private reference (resource: LawSpec.Data.Cluster) = resource :?> ReferenceCluster

/// (Unit -> lawtwin.kafka::type::Cluster)
let connectCluster (value0: unit) : LawSpec.Data.Cluster =
    let symbols = Scalar.SymbolContext()
    let program = LawSpec.Generated.Program.schema
    let model = make "Cluster" program symbols
    let partitioner = make "MurmurPartitioner" program symbols
    let schema =
        Effects.install program
            [ "lawtwin.kafka::ability::Broker",
              LawSpec.Generated.Abilities.lawtwinKafkaBrokerFromNative program symbols (model :?> IBroker)
              "lawtwin.kafka::ability::Partitioner",
              LawSpec.Generated.Abilities.lawtwinKafkaPartitionerFromNative program symbols (partitioner :?> IPartitioner) ]
    box { Model = model; Partitioner = partitioner; Schema = schema; Symbols = symbols }

/// (lawtwin.kafka::type::Cluster -> Unit)
let disconnectCluster (value0: LawSpec.Data.Cluster) : unit = ()

/// Broker answers through the reference model.
let brokerHandler (resource: LawSpec.Data.Cluster) : IBroker = (reference resource).Model :?> IBroker

/// Admin answers through the reference model.
let adminHandler (resource: LawSpec.Data.Cluster) : IAdmin = (reference resource).Model :?> IAdmin

/// Kafka's murmur2 partitioner, as the contract states it.
let partitionerHandler (resource: LawSpec.Data.Cluster) : IPartitioner =
    (reference resource).Partitioner :?> IPartitioner

/// The client spec handler over the cluster.
let producerHandler (resource: LawSpec.Data.Cluster) : IProducer =
    let cluster = reference resource
    make "KafkaProducer" cluster.Schema cluster.Symbols :?> IProducer

/// The client spec handler over the cluster.
let consumerHandler (resource: LawSpec.Data.Cluster) : IConsumer =
    let cluster = reference resource
    make "KafkaConsumer" cluster.Schema cluster.Symbols :?> IConsumer

/// GroupCoordinator answers through the reference model.
let groupCoordinatorHandler (resource: LawSpec.Data.Cluster) : IGroupCoordinator =
    (reference resource).Model :?> IGroupCoordinator

/// The client spec handler over the cluster.
let transactionsHandler (resource: LawSpec.Data.Cluster) : ITransactions =
    let cluster = reference resource
    make "KafkaTransactions" cluster.Schema cluster.Symbols :?> ITransactions
