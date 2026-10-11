// User-owned LawSpec adapter.
//
// The native handlers of the lawtwin.kafka contract, for running its laws.
// A case's handlers are all made with the case's one Cluster. Here the
// cluster is the generated reference model: the `cluster` spec handler for
// Broker, Admin and GroupCoordinator, with the murmur2 partitioner, and the
// client spec handlers written over them. In conformance runs a connection to
// the Kafka twin or to a real cluster takes its place.
namespace Lawtwin;

using System;
using Microsoft.FSharp.Collections;
using LawSpec.Runtime;
using LawSpec.Abilities.LawtwinKafka;
using LawSpec.Handlers.LawtwinKafka;

public static class Kafka
{
    /// <summary>What a Cluster holds: the reference model, the partitioner, and the
    /// schema that installs them for the client spec handlers to call.</summary>
    public sealed record ReferenceCluster(
        Cluster Model, MurmurPartitioner Partitioner, Schema.Schema Schema, Scalar.SymbolContext Symbols);

    private static ReferenceCluster Reference(object resource) => (ReferenceCluster)resource;

    /// <summary>(Unit -> lawtwin.kafka::type::Cluster)</summary>
    public static object ConnectCluster()
    {
        var symbols = new Scalar.SymbolContext();
        var program = LawSpec.Generated.Program.Schema;
        var model = new Cluster(program, symbols);
        var partitioner = new MurmurPartitioner(program, symbols);
        var schema = Effects.Install(program, ListModule.OfArray(new[]
        {
            Tuple.Create("lawtwin.kafka::ability::Broker",
                LawSpec.Generated.Abilities.LawtwinKafkaBrokerFromNative(program, symbols, model)),
            Tuple.Create("lawtwin.kafka::ability::Partitioner",
                LawSpec.Generated.Abilities.LawtwinKafkaPartitionerFromNative(program, symbols, partitioner)),
        }));
        return new ReferenceCluster(model, partitioner, schema, symbols);
    }

    /// <summary>(lawtwin.kafka::type::Cluster -> Unit)</summary>
    public static void DisconnectCluster(object value0) { }

    /// <summary>Broker answers through the reference model.</summary>
    public static IBroker BrokerHandler(object resource) => Reference(resource).Model;

    /// <summary>Admin answers through the reference model.</summary>
    public static IAdmin AdminHandler(object resource) => Reference(resource).Model;

    /// <summary>Kafka's murmur2 partitioner, as the contract states it.</summary>
    public static IPartitioner PartitionerHandler(object resource) => Reference(resource).Partitioner;

    /// <summary>The client spec handler over the cluster.</summary>
    public static IProducer ProducerHandler(object resource)
    {
        var cluster = Reference(resource);
        return new KafkaProducer(cluster.Schema, cluster.Symbols);
    }

    /// <summary>The client spec handler over the cluster.</summary>
    public static IConsumer ConsumerHandler(object resource)
    {
        var cluster = Reference(resource);
        return new KafkaConsumer(cluster.Schema, cluster.Symbols);
    }

    /// <summary>GroupCoordinator answers through the reference model.</summary>
    public static IGroupCoordinator GroupCoordinatorHandler(object resource) => Reference(resource).Model;

    /// <summary>The client spec handler over the cluster.</summary>
    public static ITransactions TransactionsHandler(object resource)
    {
        var cluster = Reference(resource);
        return new KafkaTransactions(cluster.Schema, cluster.Symbols);
    }
}
