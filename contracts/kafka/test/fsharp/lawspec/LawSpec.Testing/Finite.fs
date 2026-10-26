/// Ordered finite-cover data and xUnit's individually selectable boundary
/// rows. Discovery carries only an integer index; every execution makes its
/// portable values in that attempt's fresh symbol context.
/// ref:DEC-planned-generation ref:DEC-dotnet-fscheck
module LawSpec.Testing.Finite

open System
open System.Numerics
open LawSpec.Runtime
open Xunit

/// An ordered input domain, materialized only after a case is selected.
type Domain =
    private
        {
            Count: BigInteger
            At: BigInteger -> Scalar.SymbolContext -> Value
        }

/// A completed prefix's multiplicity, or ordered choices and their suffixes.
type Cover =
    private
    | Complete of int
    | Alternatives of (Domain * Cover) list

[<CompiledName("Finish")>]
let finish count =
    if count < 0 then
        invalidArg "count" "A finite cover cannot have negative multiplicity"

    Complete count

[<CompiledName("Values")>]
let values (factories: (Scalar.SymbolContext -> Value) list) =
    let choices = List.toArray factories

    {
        Count = BigInteger choices.Length
        At = fun index symbols -> choices[int index] symbols
    }

[<CompiledName("Range")>]
let range (_typeName: string) (first: BigInteger) (last: BigInteger) =
    {
        Count = max BigInteger.Zero (last - first + BigInteger.One)
        At = fun index _ -> Value.Int(first + index)
    }

[<CompiledName("Branch")>]
let branch domain suffix = domain, suffix

[<CompiledName("Choices")>]
let choices branches = Alternatives branches

/// The number of planned observations, including repeated completed prefixes.
[<CompiledName("Count")>]
let rec count cover =
    match cover with
    | Complete n -> BigInteger n
    | Alternatives branches -> branches |> List.sumBy (fun (domain, suffix) -> domain.Count * count suffix)

/// One-based selection preserves the planner's depth-first order without
/// constructing the preceding cases or overflowing a machine-width endpoint.
[<CompiledName("At")>]
let at cover (symbols: Scalar.SymbolContext) (index: int) =
    if index < 1 || BigInteger index > count cover then
        invalidArg "index" "A finite case index is outside its cover"

    let rec select cover offset =
        match cover with
        | Complete _ -> []
        | Alternatives branches ->
            let rec choose remaining offset =
                match remaining with
                | [] -> invalidOp "A finite cover's count disagrees with its branches"
                | (domain, suffix) :: rest ->
                    let suffixCount = count suffix
                    let branchCount = domain.Count * suffixCount

                    if offset >= branchCount then
                        choose rest (offset - branchCount)
                    else
                        let item = domain.At (offset / suffixCount) symbols
                        item :: select suffix (offset % suffixCount)

            choose branches offset

    select cover (BigInteger(index - 1))

/// A serializable native row with a stable display name, selectable with
/// xUnit's --filter-display-name. No values cross discovery.
[<CompiledName("Rows")>]
let rows (label: string) (source: string) (total: int) =
    seq {
        for index in 1..total do
            let row = TheoryDataRow<int>(index)

            row.TestDisplayName <-
                label
                + " boundary "
                + string index
                + (if source = "" then "" else " — " + source)

            yield row
    }

/// One boundary observation, with its stable index and concrete input values
/// in any failure. The caller supplies its ordinary shared law checker.
[<CompiledName("Check")>]
let check (label: string) (index: int) (values: Value list) (law: Value list -> Value) =
    try
        match law values with
        | Bool true -> ()
        | other -> Fail.raiseWith "ContractFailed" (sprintf "%s boundary %d returned %A" label index other)
    with :? LawSpecException as e ->
        let shown = "(" + String.Join(", ", values |> List.map Recorded.renderPlain) + ")"
        raise (LawSpecException("BoundaryFailed", sprintf "%s boundary %d %s: %s" label index shown e.Message, [], e))

/// A known-discrepancy law remains one harness control, walking the same cover
/// instead of embedding a copy of every row in its generated source.
[<CompiledName("CheckAll")>]
let checkAll label cover symbols law =
    let total = count cover

    if total > BigInteger Int32.MaxValue then
        invalidOp "A finite cover has too many native rows"

    for index in 1 .. int total do
        check label index (at cover symbols index) law
