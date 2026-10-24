/// The harness plane on .NET: how often a law runs, how long it may take,
/// whether its cases were adequate (cover, classify, labels), targeted
/// search scores, known discrepancies, skips and benchmarks. Statistics go to
/// LAWSPEC_STATS as the same JSON records every target writes.
/// ref:REQ-harness-units ref:DEC-never-pass-vacuously
module LawSpec.Testing.Harness

open System
open System.Collections.Generic
open System.Diagnostics
open System.IO
open System.Security.Cryptography
open System.Text
open System.Text.Json
open System.Text.Json.Nodes
open System.Threading
open System.Threading.Tasks
open LawSpec.Runtime

/// What one generated case observed: cover predicates, classifications,
/// labels and a target score.
type Observation =
    { Covers: Value list
      Classes: (string * Value) list
      Labels: Value list
      Score: Value option }

type private State =
    { Law: string
      Test: string
      mutable Failed: bool
      mutable Cases: int
      mutable Cover: int[]
      Classes: SortedDictionary<string, int>
      Labels: SortedDictionary<string, int>
      mutable Target: bool
      mutable Best: Value option
      mutable Aborted: exn option
      Extra: Dictionary<string, JsonNode> }

let private current = AsyncLocal<State option>()

let private ordinal = StringComparer.Ordinal

/// Fail the harness itself (not the law): a strategy outside its input's
/// refinement, a discard budget spent, a broken cover clause.
[<CompiledName("Abort")>]
let abort (code: string) (detail: string) : 'a =
    let failure = LawSpecException(code, detail)
    match current.Value with
    | Some state when state.Aborted.IsNone -> state.Aborted <- Some(failure :> exn)
    | _ -> ()
    raise failure

/// Mark a failure as the harness's, keeping the failure itself.
[<CompiledName("MarkHarnessFailure")>]
let markHarnessFailure (failure: exn) =
    match current.Value with
    | Some state when state.Aborted.IsNone -> state.Aborted <- Some failure
    | _ -> ()

let private boolean (value: Value) =
    match value with
    | Bool b -> b
    | _ -> abort "InvalidHarnessPredicate" (sprintf "%A" value)

/// Whether a score beats the best so far; a NaN never does.
[<CompiledName("Higher")>]
let higher (score: Value) (best: Value option) =
    match score with
    | Float _ when Scalar.floatClass score = "nan" -> false
    | Float _ ->
        match best with
        | None -> true
        | Some other -> Scalar.floatCompare score other = Some 1
    | Int _ | Ratio _ | Value.Decimal _ ->
        match best with
        | None -> true
        | Some other -> Scalar.compareValues score other = 1
    | _ -> abort "InvalidTargetScore" (sprintf "%A" score)

/// A score as the reports print it.
[<CompiledName("ScoreText")>]
let scoreText (score: Value) =
    match score with
    | Int n -> n.ToString()
    | Ratio(n, d) -> n.ToString() + "/" + d.ToString()
    | Value.Decimal(c, e) -> c.ToString() + "e" + string e
    | Float(w, b) ->
        match Scalar.floatClass score with
        | "nan" -> "NaN"
        | "infinity" -> if (b >>> (w - 1)) = 0UL then "Infinity" else "-Infinity"
        | _ -> (Scalar.toDouble score).ToString("R", Globalization.CultureInfo.InvariantCulture)
    | _ -> sprintf "%A" score

let private observe (state: State) (o: Observation) =
    if List.length o.Covers <> state.Cover.Length then abort "InvalidHarnessCoverObservation" ""
    o.Covers |> List.iteri (fun i holds -> if boolean holds then state.Cover.[i] <- state.Cover.[i] + 1)
    for label, _ in o.Classes do
        if not (state.Classes.ContainsKey label) then state.Classes.[label] <- 0
    for label in o.Classes |> List.filter (snd >> boolean) |> List.map fst |> List.distinct do
        state.Classes.[label] <- state.Classes.[label] + 1
    let labels =
        o.Labels |> List.map (function Text s -> s | v -> abort "InvalidHarnessLabel" (sprintf "%A" v)) |> List.distinct
    for label in labels do
        state.Labels.[label] <- (match state.Labels.TryGetValue label with | true, n -> n | _ -> 0) + 1
    match o.Score with
    | None -> ()
    | Some score ->
        state.Target <- true
        if higher score state.Best then state.Best <- Some score

/// What a case observed.
[<CompiledName("Observation")>]
let observation covers classes labels score = { Covers = covers; Classes = classes; Labels = labels; Score = score }

/// One generated case: count it, record what it observed, then run it. After
/// the first failure only the law runs (shrinking does not count cases).
[<CompiledName("Sample")>]
let sample (observation: unit -> Observation) (test: unit -> Value) : Value =
    match current.Value with
    | None -> Fail.raiseWith "HarnessObservationOutsideRun" ""
    | Some state when state.Failed ->
        observation () |> ignore
        test ()
    | Some state ->
        state.Cases <- state.Cases + 1
        try
            observe state (observation ())
            test ()
        with _ ->
            state.Failed <- true
            reraise ()

let private percentage n cases = if cases = 0 then 0.0 else Math.Round(10000.0 * float n / float cases) / 100.0

let private statsDirectory () =
    match Environment.GetEnvironmentVariable "LAWSPEC_STATS" with
    | null | "" -> None
    | directory -> Some directory

let private writeRecord (identity: string) (record: JsonObject) =
    match statsDirectory () with
    | None -> ()
    | Some directory ->
        Directory.CreateDirectory directory |> ignore
        let hash = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes identity))
        let path = Path.Combine(directory, hash + ".json")
        let temporary = path + "." + string Environment.ProcessId + "." + Guid.NewGuid().ToString("N") + ".tmp"
        try
            File.WriteAllText(temporary, record.ToJsonString())
            File.Move(temporary, path, true)
        finally
            if File.Exists temporary then File.Delete temporary

/// The expected-failure context of a `known discrepancy` law.
type private Expected =
    { ExpectedLaw: string
      Checks: ResizeArray<JsonObject>
      mutable ExpectedAborted: exn option }

let private expected = AsyncLocal<Expected option>()

let private commit (record: JsonObject) =
    let law = record.["law"].GetValue<string>()
    match expected.Value with
    | Some e when e.ExpectedLaw = law ->
        e.Checks.Add record
        if record.ContainsKey "failureKind" && record.["failureKind"].GetValue<string>() = "harness" && e.ExpectedAborted.IsNone then
            e.ExpectedAborted <- Some(LawSpecException("HarnessFailed", law) :> exn)
    | _ -> writeRecord (law + "\u0000" + record.["test"].GetValue<string>()) record

let private node (value: obj) : JsonNode =
    match value with
    | :? JsonNode as n -> n
    | :? string as s -> JsonValue.Create(s)
    | :? int as i -> JsonValue.Create(i)
    | :? int64 as i -> JsonValue.Create(i)
    | :? float as f -> JsonValue.Create(f)
    | :? bool as b -> JsonValue.Create(b)
    | null -> null
    | other -> JsonValue.Create(string other)

let private objectOf (fields: (string * obj) list) =
    let o = JsonObject()
    for k, v in fields do
        o.[k] <- node v
    o

type private CoverRow =
    { Label: string
      Required: int
      Observed: float
      Hits: int
      Cases: int
      Met: bool }

let private coverRows (covers: (int * string) list) (state: State) =
    List.zip covers (List.ofArray state.Cover)
    |> List.map (fun ((percent, label), n) ->
        { Label = label
          Required = percent
          Observed = percentage n state.Cases
          Hits = n
          Cases = state.Cases
          Met = state.Cases > 0 && 100 * n >= percent * state.Cases })

let private reportText (state: State) (rows: CoverRow list) =
    let text = StringBuilder()
    let line (s: string) = text.Append(s).Append('\n') |> ignore
    line (sprintf "%s: %d generated case(s)" state.Law state.Cases)
    for row in rows do
        let shortfall = if row.Met then "" else sprintf " (not met; %d/%d cases)" row.Hits row.Cases
        line (sprintf "  cover %d%% \"%s\": %.2f%%%s" row.Required row.Label row.Observed shortfall)
    for KeyValue(label, n) in state.Classes do
        line (sprintf "  %s: %.2f%%" label (percentage n state.Cases))
    for KeyValue(label, n) in state.Labels do
        line (sprintf "  label %s: %.2f%%" label (percentage n state.Cases))
    match state.Target, state.Best with
    | true, Some best -> line ("  best target score: " + scoreText best)
    | _ -> ()
    text.ToString()

let private report (state: State) (rows: CoverRow list) =
    let cover = JsonArray()
    for row in rows do
        cover.Add(objectOf [ "label", box row.Label; "required", box row.Required; "observed", box row.Observed
                             "hits", box row.Hits; "cases", box row.Cases; "met", box row.Met ])
    let classes = JsonObject()
    for KeyValue(k, v) in state.Classes do classes.[k] <- JsonValue.Create(v)
    let labels = JsonObject()
    for KeyValue(k, v) in state.Labels do labels.[k] <- JsonValue.Create(v)
    let o =
        objectOf [ "law", box state.Law; "test", box state.Test; "attempts", box 1; "cases", box state.Cases
                   "cover", box cover; "classes", box classes; "labels", box labels ]
    if state.Target then
        o.["target"] <- objectOf [ "best", (match state.Best with Some b -> box (scoreText b) | None -> null) ]
    for KeyValue(k, v) in state.Extra do
        o.[k] <- v.DeepClone()
    o

/// Options of a controlled run.
type Options =
    { Repeat: int
      Retries: int
      Timeout: int option
      Resources: bool
      Observed: bool }

[<CompiledName("Defaults")>]
let defaults = { Repeat = 1; Retries = 0; Timeout = None; Resources = false; Observed = true }

/// A controlled run's options.
[<CompiledName("Options")>]
let options (repeat: int) (retries: int) (timeout: int option) (resources: bool) (observed: bool) =
    { Repeat = repeat; Retries = retries; Timeout = timeout; Resources = resources; Observed = observed }

let private execute (law: string) (options: Options) (test: unit -> unit) : exn option =
    match options.Timeout with
    | None ->
        try
            test ()
            None
        with e -> Some e
    | Some ms ->
        let state = current.Value
        let work = Task.Run(fun () ->
            current.Value <- state
            test ())
        let finished = try work.Wait(ms) with _ -> true
        if not finished then
            let failure = LawSpecException("TestTimeout", sprintf "%s did not finish within %d ms" law ms)
            markHarnessFailure failure
            Some(failure :> exn)
        elif work.IsFaulted then
            Some(match work.Exception.InnerExceptions.Count with
                 | 1 -> work.Exception.InnerExceptions.[0]
                 | _ -> work.Exception :> exn)
        else None

/// Run a test under the harness: each repeat is a complete check with its
/// own counters, and a retry starts the repeats again. Coverage is never
/// pooled across runs.
[<CompiledName("Run")>]
let run (law: string) (name: string) (covers: (int * string) list) (options: Options) (test: unit -> unit) =
    let previous = current.Value
    try
        let rec attempts (attempt: int) (history: JsonObject list) =
            let rec repeats (repetition: int) (runs: JsonObject list) =
                let state =
                    { Law = law; Test = name; Failed = false; Cases = 0; Cover = Array.zeroCreate covers.Length
                      Classes = SortedDictionary(ordinal); Labels = SortedDictionary(ordinal); Target = false
                      Best = None; Aborted = None; Extra = Dictionary() }
                current.Value <- Some state
                let failure = execute law options test
                let rows = coverRows covers state
                let unmet = rows |> List.filter (fun r -> not r.Met)
                let reportObject = if options.Observed then report state rows else objectOf [ "law", box law; "test", box name ]
                let text = if options.Observed then reportText state rows else ""
                if options.Observed then Console.Out.Write text
                let outcome, retryable =
                    match failure, state.Aborted, unmet with
                    | None, Some cause, _ -> Some cause, false
                    | None, None, _ :: _ ->
                        Some(LawSpecException("HarnessFailed", law + ": UnmetCover\n" + text) :> exn), false
                    | Some e, Some _, _ -> Some e, false
                    | Some e, None, _ ->
                        (if options.Observed && not (List.isEmpty covers) then
                             Some(LawSpecException("PropertyFailed", e.Message + "\n" + text, [], e) :> exn)
                         else Some e), true
                    | None, None, [] -> None, true
                let run = reportObject.DeepClone().AsObject()
                run.Remove "law" |> ignore
                run.Remove "test" |> ignore
                run.Remove "attempts" |> ignore
                run.["attempt"] <- JsonValue.Create(attempt)
                run.["repetition"] <- JsonValue.Create(repetition)
                run.["outcome"] <- JsonValue.Create(if outcome.IsNone then "passed" else "failed")
                let runs = run :: runs
                match outcome with
                | None when repetition < options.Repeat -> repeats (repetition + 1) runs
                | _ -> outcome, retryable, reportObject, runs
            let outcome, retryable, reportObject, runs = repeats 1 history
            match outcome with
            | Some _ when retryable && attempt <= options.Retries -> attempts (attempt + 1) runs
            | _ ->
                let status =
                    match outcome with
                    | None when attempt > 1 -> "flaky"
                    | None -> "passed"
                    | Some _ -> "failed"
                reportObject.["outcome"] <- JsonValue.Create(status)
                reportObject.["attempts"] <- JsonValue.Create(attempt)
                if not retryable then reportObject.["failureKind"] <- JsonValue.Create("harness")
                if options.Repeat <> 1 || options.Retries <> 0 then
                    reportObject.["repeat"] <- JsonValue.Create(options.Repeat)
                    let array = JsonArray()
                    for r in List.rev runs do array.Add(r)
                    reportObject.["runs"] <- array
                match outcome with
                | None ->
                    commit reportObject
                    if status = "flaky" then printfn "%s is flaky: it failed, then passed on attempt %d" law attempt
                | Some e ->
                    (try commit reportObject with r -> eprintfn "LawSpec statistics failed for %s: %s" law r.Message)
                    raise e
        attempts 1 []
    finally
        current.Value <- previous

/// Record extra fields (a search's report) in the running test's record.
[<CompiledName("Record")>]
let record (fields: (string * JsonNode) list) =
    match current.Value with
    | Some state -> for k, v in fields do state.Extra.[k] <- v
    | None -> ()

/// A skipped law, recorded and reported.
[<CompiledName("Skip")>]
let skip (law: string) (reason: string) =
    eprintfn "SKIPPED %s: %s" law reason
    commit (objectOf [ "law", box law; "test", box (law + " skipped"); "outcome", box "skipped"; "reason", box reason ])

/// An acknowledged discrepancy must reproduce. An unexpected pass requires
/// human review of the specification and implementation.
[<CompiledName("KnownDiscrepancy")>]
let knownDiscrepancy (law: string) (name: string) (reason: string) (checks: (unit -> unit) list) =
    let previous = expected.Value
    let context = { ExpectedLaw = law; Checks = ResizeArray(); ExpectedAborted = None }
    expected.Value <- Some context
    let failure =
        try
            try
                for check in checks do check ()
                None
            with e -> Some e
        finally
            expected.Value <- previous
    let reports = JsonArray()
    for c in context.Checks do reports.Add(c)
    let record outcome extra =
        let o = objectOf ([ "law", box law; "test", box name; "reason", box reason; "outcome", box outcome ] @ extra)
        o.["checks"] <- reports
        commit o
    match failure, context.ExpectedAborted with
    | None, None ->
        record "known-discrepancy-passed" []
        eprintfn "%s is marked known discrepancy (%s), but it passes; review the specification and implementation before removing `known discrepancy` from its harness" law reason
        Fail.raiseWith "KnownDiscrepancyPassed" (law + " is marked known discrepancy (" + reason + "), but it passes; review the specification and implementation before removing `known discrepancy` from its harness")
    | Some e, None ->
        record "known-discrepancy" [ "failure", box e.Message ]
        printfn "%s has a known discrepancy (%s): %s" law reason e.Message
    | Some e, Some _ ->
        record "failed" [ "failureKind", box "harness"; "failure", box e.Message ]
        raise e
    | None, Some cause ->
        record "failed" [ "failureKind", box "harness" ]
        raise cause

/// Measure a body: at least three iterations, about 200 ms, at most 100,000.
[<CompiledName("Benchmark")>]
let benchmark (unitName: string) (name: string) (body: unit -> unit) =
    let started = Stopwatch.GetTimestamp()
    let ticks = Stopwatch.Frequency
    let toNs (t: int64) = t * 1_000_000_000L / ticks
    let mutable count = 0L
    let mutable total = 0L
    let mutable fastest = Int64.MaxValue
    let mutable go = true
    while go && count < 100000L do
        let before = Stopwatch.GetTimestamp()
        if count >= 3L && toNs (before - started) >= 200_000_000L then go <- false
        else
            body ()
            let elapsed = toNs (Stopwatch.GetTimestamp() - before)
            fastest <- min fastest elapsed
            total <- total + elapsed
            count <- count + 1L
    let mean = total / count
    writeRecord ("benchmark\u0000" + unitName + "\u0000" + name)
        (objectOf [ "benchmark", box name; "unit", box unitName; "iterations", box count; "mean_ns", box mean; "min_ns", box fastest ])
    printfn "benchmark %s: %d iteration(s), mean %.2f us, fastest %.2f us" name count (float mean / 1000.0) (float fastest / 1000.0)

/// Record that a parallel unit ran.
[<CompiledName("RecordParallel")>]
let recordParallel (unitName: string) (fields: (string * obj) list) =
    writeRecord ("parallel\u0000" + unitName) (objectOf (("parallel", box unitName) :: fields))
