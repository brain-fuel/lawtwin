/// Running a law's property on FsCheck: LawSpec's generation plan is an
/// FsCheck Arbitrary whose shrinker walks the plan's shrink tree, and FsCheck
/// runs the cases, grows the size, counts discards and shrinks the first
/// failure. One seed per run is printed with any failure, and LAWSPEC_SEED
/// replays it through FsCheck's Replay. ref:DEC-dotnet-fscheck
/// ref:DEC-never-pass-vacuously
module LawSpec.Testing.Property

open System
open System.Numerics
open LawSpec.Runtime

/// How many cases to run, how many discards the run may take, and how many
/// shrink steps a failure may take.
type Settings =
    {
        Cases: int
        MaxAttempts: int
        MaxShrinks: int
    }

let private random = System.Random.Shared

/// The run's seed: LAWSPEC_SEED, or a fresh one.
[<CompiledName("RunSeed")>]
let runSeed () =
    match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
    | null
    | "" -> BigInteger(random.NextInt64())
    | text -> BigInteger.Parse text

/// FsCheck's random state for a seed: its low 64 bits, with FsCheck's own gamma.
let private rndOf (seed: BigInteger) =
    let low = BigInteger.Remainder(seed, BigInteger.Pow(BigInteger 2, 64))

    FsCheck.Rnd(
        uint64 (
            if low.Sign < 0 then
                low + BigInteger.Pow(BigInteger 2, 64)
            else
                low
        )
    )

let private render (values: Value list) =
    "(" + String.Join(", ", values |> List.map Recorded.renderPlain) + ")"

/// The largest size a case draws at. FsCheck grows the size linearly from 0
/// to this over the run's cases, and with each discard too, so a domain that
/// small draws rarely meet is reached at larger sizes; past it, cases draw
/// at it.
let private maxSize = 100

/// What FsCheck reports at the end of a run.
type private Outcome() =
    member val Result: FsCheck.TestResult option = None with get, set

    interface FsCheck.IRunner with
        member _.OnStartFixture _ = ()
        member _.OnArguments(_, _, _) = ()
        member _.OnShrink(_, _) = ()
        member this.OnFinished(_, result) = this.Result <- Some result

/// A failure of the harness itself ends the run: FsCheck neither counts it
/// as a counterexample nor shrinks it.
exception private Aborted of exn

/// Check a property over a generator of its inputs.
[<CompiledName("Check")>]
let check (label: string) (settings: Settings) (gen: Gen<Value list>) (property: Value list -> Value) =
    let seed = runSeed ()
    let width = settings.MaxShrinks * 4

    let arbitrary =
        FsCheck.FSharp.Arb.fromGenShrink (
            Gen.toFsCheck gen
            |> FsCheck.FSharp.Gen.scaleSize (min maxSize)
            |> FsCheck.FSharp.Gen.map (Option.map (Tree.limit settings.MaxShrinks width)),
            function
            | Some tree -> tree.Children |> Seq.map Some
            | None -> Seq.empty
        )

    let mutable aborted = None

    let law (drawn: Tree<Value list> option) =
        match drawn, aborted with
        | _, Some _
        | None, _ -> FsCheck.FSharp.Prop.discard ()
        | Some tree, None ->
            try
                match property tree.Value with
                | Bool true -> true
                | Bool false -> raise (LawSpecException("PropertyFalse", ""))
                | other -> raise (LawSpecException("NonBooleanProperty", sprintf "%A" other))
            with :? LawSpecException as e when e.Code = "StrategyOutsideRefinement" || e.Code = "StrategyDiscards" ->
                aborted <- Some e
                FsCheck.FSharp.Prop.discard ()

    let outcome = Outcome()

    let config =
        FsCheck.Config.Quick
            .WithName(label)
            .WithMaxTest(settings.Cases)
            .WithMaxRejected(settings.MaxAttempts)
            .WithStartSize(0)
            .WithEndSize(maxSize)
            .WithQuietOnSuccess(true)
            .WithReplay(Some { Rnd = rndOf seed; Size = None })
            .WithRunner(outcome)

    FsCheck.Check.One(config, FsCheck.FSharp.Prop.forAll arbitrary law)

    match aborted with
    | Some e -> raise e
    | None -> ()

    match outcome.Result with
    | None
    | Some(FsCheck.TestResult.Passed _) -> ()
    | Some(FsCheck.TestResult.Exhausted data) ->
        Harness.abort
            "CannotSatisfy"
            (sprintf
                "%s: no input satisfied the law's domain after %d attempts (%d passing case(s); seed %O)"
                label
                settings.MaxAttempts
                data.NumberOfTests
                seed)
    | Some(FsCheck.TestResult.Failed(data, _, shrunk, result, _, _, _)) ->
        let values =
            match shrunk with
            | (:? (Tree<Value list> option) as drawn) :: _ ->
                drawn |> Option.map (fun t -> render t.Value) |> Option.defaultValue "()"
            | _ -> "()"

        let error =
            match result with
            | FsCheck.Outcome.Failed e when not (isNull e) -> e
            | _ -> LawSpecException("PropertyFalse", "") :> exn

        let message =
            sprintf
                "%s failed after %d passing case(s) and %d shrink(s): counterexample %s; seed %O (LAWSPEC_SEED=%O replays it)\n%s"
                label
                (data.NumberOfTests - 1)
                data.NumberOfShrinks
                values
                seed
                seed
                error.Message

        raise (LawSpecException("PropertyFailed", message, [], error))

/// A law's generation settings.
[<CompiledName("Settings")>]
let settings (cases: int) (attempts: int) (shrinks: int) =
    {
        Cases = cases
        MaxAttempts = attempts
        MaxShrinks = shrinks
    }

/// What generation reads: the schema, the case's symbols and the discard budget.
[<CompiledName("Generation")>]
let generation (schema: Schema.Schema) (symbols: Scalar.SymbolContext) (attempts: int) : Generators.Context =
    {
        Schema = schema
        Symbols = symbols
        Factories = Map.empty
        Attempts = attempts
    }

/// As generation, with the application's generators (bound in lawspec.edn)
/// in place of the built-in ones for their types.
[<CompiledName("GenerationWith")>]
let generationWith
    (schema: Schema.Schema)
    (symbols: Scalar.SymbolContext)
    (attempts: int)
    (factories: (string * Generators.Factory) list)
    : Generators.Context =
    {
        Schema = schema
        Symbols = symbols
        Factories = Map.ofList factories
        Attempts = attempts
    }

/// Each boundary case, in order; the first that fails names its position.
[<CompiledName("Boundaries")>]
let boundaries (label: string) (cases: Value list list) (law: Value list -> Value) =
    cases
    |> List.iteri (fun i values ->
        try
            match law values with
            | Bool true -> ()
            | other -> Fail.raiseWith "ContractFailed" (sprintf "%s boundary %d returned %A" label i other)
        with :? LawSpecException as e ->
            raise (
                LawSpecException(
                    "BoundaryFailed",
                    sprintf "%s boundary %d %s: %s" label i (render values) e.Message,
                    [],
                    e
                )
            ))

/// A strategy's value must lie inside the input's refinement.
[<CompiledName("CheckDrawn")>]
let checkDrawn (strategy: string) (input: string) (value: Value) (holds: bool) =
    if holds then
        value
    else
        Harness.abort
            "StrategyOutsideRefinement"
            (sprintf
                "strategy %s produced %s for %s, outside the input's refinement"
                strategy
                (Recorded.renderPlain value)
                input)

/// A strategy's draw that keeps only values a predicate accepts.
[<CompiledName("SuchThat")>]
let suchThat (strategy: string) (predicate: Value -> bool) (limit: int) (gen: Gen<Value>) =
    Gen.suchThat
        predicate
        limit
        (fun () -> Harness.abort "StrategyDiscards" (sprintf "strategy %s discarded %d values" strategy limit))
        gen
