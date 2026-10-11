/// How a harness runs a unit's laws on xUnit: `order random` puts a unit's
/// tests in an order the run's seed chooses (the same seed, the same order),
/// and `parallel` lets them run at once, while other units' tests run one at
/// a time. ref:REQ-harness-units ref:DEC-portable-seeded-generation
namespace LawSpec.Testing

open System
open System.Numerics
open System.Security.Cryptography
open System.Text
open Xunit.v3

module Schedule =
    let private chosen = lazy (BigInteger(RandomNumberGenerator.GetBytes 8 |> Array.append [| 0uy |] |> Array.rev))

    /// The run's seed for ordering: LAWSPEC_SEED, or one chosen once per run.
    [<CompiledName("Seed")>]
    let seed () =
        match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
        | null | "" -> chosen.Value
        | text -> BigInteger.Parse text

    /// Items in the order a seed chooses for a unit: by the hash of the seed,
    /// the unit and each item's position.
    [<CompiledName("Order")>]
    let order (unitName: string) (items: 'a list) =
        let s = seed ()
        eprintfn "order random seed %O (%s): LAWSPEC_SEED=%O replays this order" s unitName s
        items
        |> List.mapi (fun i item ->
            let key = SHA256.HashData(Encoding.UTF8.GetBytes(s.ToString() + "\u0000" + unitName + "\u0000" + string (i + 1)))
            key, i, item)
        |> List.sortWith (fun (a: byte[], i, _) (b, j, _) ->
            match ReadOnlySpan<byte>(a).SequenceCompareTo(ReadOnlySpan<byte>(b)) with
            | 0 -> compare i j
            | c -> c)
        |> List.map (fun (_, _, item) -> item)

/// xUnit's order for a unit with `order random`: its tests in declaration
/// order, then shuffled by the run's seed.
type SeededOrder() =
    interface ITestCaseOrderer with
        member _.OrderTestCases<'T when 'T :> Xunit.Sdk.ITestCase>(testCases: Collections.Generic.IReadOnlyCollection<'T>) =
            let cases = List.ofSeq testCases
            let unitName =
                match cases with
                | c :: _ -> c.TestClassName
                | [] -> ""
            let declared =
                cases |> List.sortBy (fun c -> (if c.TestMethodMetadataToken.HasValue then c.TestMethodMetadataToken.Value else 0), c.TestCaseDisplayName)
            ResizeArray(Schedule.order unitName declared) :> Collections.Generic.IReadOnlyCollection<'T>

/// xUnit v3 orders a class's test methods before each method's cases, so a
/// unit with `order random` orders its methods by the run's seed too: by
/// name, then shuffled.
type SeededMethodOrder() =
    interface ITestMethodOrderer with
        member _.OrderTestMethods<'T when 'T :> Xunit.Sdk.ITestMethod>(testMethods: Collections.Generic.IReadOnlyCollection<'T>) =
            let methods = List.ofSeq testMethods
            let unitName =
                match methods with
                | m :: _ -> m.TestClass.TestClassName
                | [] -> ""
            let declared = methods |> List.sortBy (fun m -> m.MethodName)
            ResizeArray(Schedule.order unitName declared) :> Collections.Generic.IReadOnlyCollection<'T>
