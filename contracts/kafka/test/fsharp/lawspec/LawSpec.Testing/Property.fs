/// Running a law's property: draw cases from a seed, run each, and shrink the
/// first failure to a small counterexample. One seed per run is printed with
/// any failure, and LAWSPEC_SEED replays it. ref:DEC-portable-seeded-generation
/// ref:DEC-never-pass-vacuously
module LawSpec.Testing.Property

open System
open System.Numerics
open LawSpec.Runtime

/// How many cases to run, how many discards a case may take, and how many
/// shrink steps a failure may take.
type Settings =
    { Cases: int
      MaxAttempts: int
      MaxShrinks: int }

let private random = System.Random.Shared

/// The run's seed: LAWSPEC_SEED, or a fresh one.
[<CompiledName("RunSeed")>]
let runSeed () =
    match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
    | null | "" -> BigInteger(random.NextInt64())
    | text -> BigInteger.Parse text

let private render (values: Value list) =
    "(" + String.Join(", ", values |> List.map Recorded.renderPlain) + ")"

let private failureOf (property: Value list -> Value) (values: Value list) : exn option =
    try
        match property values with
        | Bool true -> None
        | Bool false -> Some(LawSpecException("PropertyFalse", "") :> exn)
        | other -> Some(LawSpecException("NonBooleanProperty", sprintf "%A" other) :> exn)
    with
    | :? LawSpecException as e when e.Code = "StrategyOutsideRefinement" || e.Code = "StrategyDiscards" -> raise e
    | e -> Some e

/// Check a property over a generator of its inputs.
[<CompiledName("Check")>]
let check (label: string) (settings: Settings) (gen: Gen<Value list>) (property: Value list -> Value) =
    let seed = runSeed ()
    let mutable state = Random.seed seed
    let mutable passed = 0
    let mutable failure = None
    while failure.IsNone && passed < settings.Cases do
        let size = min 99 passed
        let mutable drawn = None
        let mutable discards = 0
        while drawn.IsNone do
            let s1, s2 = Random.split state
            state <- s2
            match Gen.run gen s1 size with
            | Some tree -> drawn <- Some tree
            | None ->
                discards <- discards + 1
                if discards > settings.MaxAttempts then
                    Harness.abort "CannotSatisfy"
                        (sprintf "%s: no input satisfied the law's domain after %d attempts (seed %O)" label settings.MaxAttempts seed)
        let tree = drawn.Value
        match failureOf property tree.Value with
        | None -> passed <- passed + 1
        | Some error -> failure <- Some(tree, error)
    match failure with
    | None -> ()
    | Some(tree, error) ->
        let mutable best = tree
        let mutable bestError = error
        let mutable steps = 0
        let mutable searching = true
        while searching && steps < settings.MaxShrinks do
            let next =
                best.Children
                |> Seq.truncate (settings.MaxShrinks * 4)
                |> Seq.tryPick (fun child -> failureOf property child.Value |> Option.map (fun e -> child, e))
            match next with
            | Some(child, e) ->
                best <- child
                bestError <- e
                steps <- steps + 1
            | None -> searching <- false
        let message =
            sprintf "%s failed after %d passing case(s) and %d shrink(s): counterexample %s; seed %O (LAWSPEC_SEED=%O replays it)\n%s"
                label passed steps (render best.Value) seed seed bestError.Message
        raise (LawSpecException("PropertyFailed", message, [], bestError))

/// A law's generation settings.
[<CompiledName("Settings")>]
let settings (cases: int) (attempts: int) (shrinks: int) = { Cases = cases; MaxAttempts = attempts; MaxShrinks = shrinks }

/// What generation reads: the schema, the case's symbols and the discard budget.
[<CompiledName("Generation")>]
let generation (schema: Schema.Schema) (symbols: Scalar.SymbolContext) (attempts: int) : Generators.Context =
    { Schema = schema; Symbols = symbols; Factories = Map.empty; Attempts = attempts }

/// As generation, with the application's generators (bound in lawspec.json)
/// in place of the built-in ones for their types.
[<CompiledName("GenerationWith")>]
let generationWith (schema: Schema.Schema) (symbols: Scalar.SymbolContext) (attempts: int) (factories: (string * Generators.Factory) list) : Generators.Context =
    { Schema = schema; Symbols = symbols; Factories = Map.ofList factories; Attempts = attempts }

/// Each boundary case, in order; the first that fails names its position.
[<CompiledName("Boundaries")>]
let boundaries (label: string) (cases: Value list list) (law: Value list -> Value) =
    cases
    |> List.iteri (fun i values ->
        try
            match law values with
            | Bool true -> ()
            | other -> Fail.raiseWith "ContractFailed" (sprintf "%s boundary %d returned %A" label i other)
        with
        | :? LawSpecException as e ->
            raise (LawSpecException("BoundaryFailed", sprintf "%s boundary %d %s: %s" label i (render values) e.Message, [], e)))

/// A strategy's value must lie inside the input's refinement.
[<CompiledName("CheckDrawn")>]
let checkDrawn (strategy: string) (input: string) (value: Value) (holds: bool) =
    if holds then value
    else Harness.abort "StrategyOutsideRefinement" (sprintf "strategy %s produced %s for %s, outside the input's refinement" strategy (Recorded.renderPlain value) input)

/// A strategy's draw that keeps only values a predicate accepts.
[<CompiledName("SuchThat")>]
let suchThat (strategy: string) (predicate: Value -> bool) (limit: int) (gen: Gen<Value>) =
    Gen.suchThat predicate limit (fun () -> Harness.abort "StrategyDiscards" (sprintf "strategy %s discarded %d values" strategy limit)) gen
