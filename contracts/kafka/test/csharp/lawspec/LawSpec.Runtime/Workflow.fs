/// Workflows on .NET: stages run under their policies (gates, retries with
/// jittered backoff, timeouts, hedges, caches) and a workflow compensates
/// the stages that succeeded when it fails. A workflow runtime owns the
/// policy state, the virtual time and the trace; it is ambient (AsyncLocal),
/// so a test's calls and their concurrent work share it. Without one,
/// workflows share the default runtime on the real clock, and a Clock in
/// scope supplies its view of time. ref:DEC-domain-modeling-primitives
/// ref:DEC-async-native-tasks ref:DEC-portable-seeded-generation
module LawSpec.Runtime.Workflow

open System
open System.Diagnostics
open System.Numerics
open System.Threading
open System.Threading.Tasks
open LawSpec.Runtime.Policy

/// Something that happened in a stage: start, finish, sleep, wait, hedge,
/// cached or compensate, with the attempt (or delay) number.
type Event =
    { Kind: string
      Stage: string
      Number: int64
      Succeeded: bool }

/// Where a runtime's time comes from.
type Clock =
    | RealClock
    | VirtualClock
    | ClockCallbacks of now: (unit -> int64) * sleep: (int64 -> unit) * isVirtual: bool

type private StageRecord =
    { StageKey: string
      mutable Passed: ((string * GateKind) * Gate) list
      mutable Time: int64 }

/// A runtime's policy state: virtual time, the jitter stream, the trace,
/// gate states, the cache, compensation frames and running stages. Every
/// change is under its lock; gate callbacks are pure checked definitions.
[<Sealed>]
type State(seed: BigInteger, time: int64) =
    let gate = obj ()
    let mutable now = time
    let mutable random = Random.seed seed
    let trace = ResizeArray<Event>()
    let mutable states: Map<string * GateKind, Value> = Map.empty
    let mutable cache: Map<string, (Value * Value * int64) list> = Map.empty
    let frames = Collections.Generic.Dictionary<int64, (string * (unit -> Value)) list>()
    let stages = Collections.Generic.Dictionary<int64, StageRecord>()
    let mutable counter = 0L
    member _.Now = lock gate (fun () -> now)
    member _.SetTime(t: int64) = lock gate (fun () -> now <- t)
    member _.Sleep(delay: int64) = lock gate (fun () -> now <- now + max 0L delay)
    member _.Jitter(kind, delay, previous, baseDelay) =
        lock gate (fun () ->
            let value, next = Policy.jitter kind delay previous baseDelay random
            random <- next
            value)
    member _.Event(e: Event) = lock gate (fun () -> trace.Add e)
    member _.Trace = lock gate (fun () -> List.ofSeq trace)
    member _.Cached(key: string, input: Value, at: int64) =
        lock gate (fun () ->
            let live = defaultArg (cache.TryFind key) [] |> List.filter (fun (_, _, until) -> at < until)
            cache <- cache.Add(key, live)
            live |> List.tryPick (fun (k, v, _) -> if Scalar.equal k input then Some v else None))
    member _.Cache(key: string, input: Value, value: Value, at: int64, ttl: int64) =
        lock gate (fun () ->
            let live =
                defaultArg (cache.TryFind key) []
                |> List.filter (fun (k, _, until) -> at < until && not (Scalar.equal k input))
            cache <- cache.Add(key, (input, value, at + ttl) :: live))
    member _.NewFrame() =
        lock gate (fun () ->
            counter <- counter + 1L
            frames.[counter] <- []
            counter)
    member _.Undo(frame: int64, stage: string, undo: unit -> Value) =
        lock gate (fun () ->
            match frames.TryGetValue frame with
            | true, undos -> frames.[frame] <- (stage, undo) :: undos
            | _ -> ())
    member _.TakeFrame(frame: int64) =
        lock gate (fun () ->
            match frames.TryGetValue frame with
            | true, undos ->
                frames.Remove frame |> ignore
                undos
            | _ -> [])
    member _.NewStage(key: string, at: int64) =
        lock gate (fun () ->
            counter <- counter + 1L
            stages.[counter] <- { StageKey = key; Passed = []; Time = at }
            counter)
    /// A gate's decision for a running stage, updating the gate's state.
    member _.Gate(stage: int64, g: Gate, at: int64) =
        lock gate (fun () ->
            let record = stages.[stage]
            let key = record.StageKey, g.Kind
            let before =
                match states.TryFind key with
                | Some s -> s
                | None -> g.Start(Int(BigInteger at))
            match g.Admit before (Int(BigInteger at)) with
            | Data(_, [ after; decision ]) ->
                states <- states.Add(key, after)
                match decision with
                | Data("lawspec.resilience::type::Gate::Admit", []) -> record.Passed <- record.Passed @ [ key, g ]
                | _ -> ()
                record.Time <- at
                decision
            | other -> Fail.raiseWith "InvalidGateStep" (sprintf "%A" other))
    /// A stage ended: each gate it passed records the outcome.
    member _.FinishStage(stage: int64, at: int64, succeeded: bool) =
        lock gate (fun () ->
            match stages.TryGetValue stage with
            | true, record ->
                stages.Remove stage |> ignore
                for key, g in record.Passed do
                    match g.Finish with
                    | Some finish -> states <- states.Add(key, finish states.[key] (Int(BigInteger at)) (Bool succeeded))
                    | None -> ()
            | _ -> ())

/// A workflow runtime: its state, its clock, whether gates and caches
/// apply, and whether it was installed explicitly (an explicit runtime's
/// clock is not replaced by a Clock in scope).
type Runtime =
    { State: State
      Clock: Clock
      Gates: bool
      Explicit: bool }

let private ambient = AsyncLocal<Runtime option>()
let private shared = lazy { State = State(BigInteger.Zero, 0L); Clock = RealClock; Gates = true; Explicit = false }

let private scoped (runtime: Runtime) (body: Runtime -> 'a) : 'a =
    let previous = ambient.Value
    ambient.Value <- Some runtime
    try body runtime
    finally ambient.Value <- previous

let private seedOf () =
    match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
    | null | "" -> BigInteger.Zero
    | s -> (try BigInteger.Parse s with _ -> BigInteger.Zero)

/// Run a body with its own runtime on a clock, from a seed.
[<CompiledName("WithClock")>]
let withClock (clock: Clock) (seed: BigInteger) (body: Runtime -> 'a) : 'a =
    scoped { State = State(seed, 0L); Clock = clock; Gates = true; Explicit = true } body

/// A runtime on a virtual clock that starts at 0 and moves only when told.
[<CompiledName("WithVirtual")>]
let withVirtual (seed: BigInteger) (body: Runtime -> 'a) : 'a = withClock VirtualClock seed body

/// A runtime on the real clock.
[<CompiledName("WithReal")>]
let withReal (seed: BigInteger) (body: Runtime -> 'a) : 'a = withClock RealClock seed body

/// A runtime whose clock is two callbacks (microseconds).
[<CompiledName("WithClockCallbacks")>]
let withClockCallbacks (now: unit -> int64) (sleep: int64 -> unit) (isVirtual: bool) (seed: BigInteger) (body: Runtime -> 'a) : 'a =
    withClock (ClockCallbacks(now, sleep, isVirtual)) seed body

/// A law's runtime: virtual time and no gates or caches, from the run's
/// seed; a Clock in scope (`using virtual clock`) supplies the time.
[<CompiledName("WithTestRuntime")>]
let withTestRuntime (body: unit -> 'a) : 'a =
    scoped { State = State(seedOf (), 0L); Clock = VirtualClock; Gates = false; Explicit = false } (fun _ -> body ())

let rec private isRealClock (handler: Effects.Handler) =
    match handler with
    | Effects.Recording(_, inner) -> isRealClock inner
    | Effects.Stateless clauses ->
        // The default Clock's clauses are closures of the Defaults module.
        match clauses.TryFind "now" with
        | Some clause -> clause.GetType().FullName.StartsWith "LawSpec.Runtime.Defaults"
        | None -> false
    | Effects.Stateful _ -> false

/// The runtime a stage runs under: the ambient one (or the shared default),
/// timed by the Clock in scope unless the runtime was installed explicitly.
[<CompiledName("Current")>]
let current (schema: Schema.Schema) : Runtime =
    let baseRuntime = defaultArg ambient.Value shared.Value
    match baseRuntime.Explicit, schema.Handlers.TryFind "lawspec.time::ability::Clock" with
    | false, Some boxed ->
        let handler: Effects.Handler = unbox boxed
        let now () =
            match Effects.invoke handler schema "now" [] with
            | Data(_, [ Int t ]) -> int64 t
            | other -> Fail.raiseWith "InvalidInstant" (sprintf "%A" other)
        let sleep (delay: int64) =
            Effects.invoke handler schema "sleep" [ Data("lawspec.time::type::Duration::Duration", [ Int(BigInteger delay) ]) ]
            |> ignore
        { baseRuntime with Clock = ClockCallbacks(now, sleep, not (isRealClock handler)) }
    | _ -> baseRuntime

/// The ambient runtime (the shared default outside any).
[<CompiledName("Ambient")>]
let ambientRuntime () = defaultArg ambient.Value shared.Value

let private isVirtual (runtime: Runtime) =
    match runtime.Clock with
    | VirtualClock -> true
    | RealClock -> false
    | ClockCallbacks(_, _, v) -> v

/// The runtime's time, in microseconds.
[<CompiledName("Now")>]
let now (runtime: Runtime) : int64 =
    match runtime.Clock with
    | RealClock -> Defaults.nowMicros ()
    | VirtualClock -> runtime.State.Now
    | ClockCallbacks(read, _, _) -> read ()

/// Let time pass: at once on a virtual clock.
[<CompiledName("Sleep")>]
let sleep (runtime: Runtime) (delay: int64) =
    if delay > 0L then
        match runtime.Clock with
        | RealClock -> Defaults.sleepMicros delay
        | VirtualClock -> runtime.State.Sleep delay
        | ClockCallbacks(_, wait, _) -> wait delay

/// Set a virtual runtime's time.
[<CompiledName("SetTime")>]
let setTime (runtime: Runtime) (time: int64) =
    match runtime.Clock with
    | VirtualClock -> runtime.State.SetTime time
    | _ -> Fail.raiseWith "NotVirtual" "only a virtual runtime's time is set"

/// What happened, in order.
[<CompiledName("Trace")>]
let trace (runtime: Runtime) : Event list = runtime.State.Trace

let private event (runtime: Runtime) (policy: StagePolicy) kind number succeeded =
    runtime.State.Event { Kind = kind; Stage = policy.Stage; Number = number; Succeeded = succeeded }

// ---------------------------------------------------------------------------
// Real-clock timeouts and hedges. ref:DEC-async-native-tasks

let private micros () = Stopwatch.GetTimestamp() * 1_000_000L / Stopwatch.Frequency

/// Run attempts of a body on the real clock: a hedge starts another after
/// the delay, up to most; the first success wins, or the last failure after
/// every attempt; past the deadline the stage is TimedOut. Losing attempts
/// are cancelled through their scope, never awaited.
[<CompiledName("RunAttempts")>]
let runAttempts (timeout: int64 option) (hedge: (int64 * int64) option) (body: unit -> Value) (onHedge: int64 -> unit) : Value =
    Tasks.withScope (fun _ ->
        let start = micros ()
        let deadline =
            match timeout with
            | Some t when t > 0L -> Some(start + t)
            | _ -> None
        let delay, most =
            match hedge with
            | Some(d, m) -> max 0L d, max 1L m
            | None -> 0L, 1L
        let launch (number: int64) =
            if number > 1L then onHedge number
            Tasks.start (fun () ->
                let result = try Choice1Of2(body ()) with e -> Choice2Of2 e
                micros (), result)
        let mutable workers = [ launch 1L ]
        let mutable started = 1L
        let mutable next = start + delay
        let mutable outcome = None
        while outcome.IsNone do
            let current = micros ()
            let timer =
                let hedgeAt = if started < most then Some next else None
                match deadline, hedgeAt with
                | Some d, Some h -> Some(min d h)
                | Some d, None -> Some d
                | None, h -> h
            let waitMs =
                match timer with
                | Some t -> int (max 0L ((t - current + 999L) / 1000L))
                | None -> -1
            let finished = Task.WaitAny((workers |> List.map (fun t -> t :> Task) |> Array.ofList), waitMs)
            if finished >= 0 then
                let worker = List.item finished workers
                let rest = workers |> List.filter (fun w -> not (obj.ReferenceEquals(w, worker)))
                workers <- rest
                let ended, result = worker.Result
                match result with
                | _ when deadline.IsSome && ended > deadline.Value -> outcome <- Some(stageFailure "TimedOut")
                | Choice2Of2 e -> raise e
                | Choice1Of2 value when not (failed value) -> outcome <- Some value
                | Choice1Of2 value when List.isEmpty rest && started >= most -> outcome <- Some value
                | Choice1Of2 _ when List.isEmpty rest ->
                    if deadline.IsNone || micros () < deadline.Value then
                        started <- started + 1L
                        workers <- [ launch started ]
                        next <- micros () + delay
                    else outcome <- Some(stageFailure "TimedOut")
                | Choice1Of2 _ -> ()
            else
                let at = micros ()
                if deadline.IsSome && at >= deadline.Value then outcome <- Some(stageFailure "TimedOut")
                elif started < most && at >= next then
                    started <- started + 1L
                    workers <- workers @ [ launch started ]
                    next <- micros () + delay
        outcome.Value)

// ---------------------------------------------------------------------------
// Stages and workflows

type private Frame = { FrameState: State; Id: int64 }

let private frame = AsyncLocal<Frame option>()

let rec private virtualHedge (hedge: (int64 * int64) option) (body: unit -> Value) (onHedge: int64 -> unit) (started: int64) =
    match hedge with
    | None -> body ()
    | Some(_, most) ->
        let result = body ()
        if failed result && started < most then
            onHedge (started + 1L)
            virtualHedge hedge body onHedge (started + 1L)
        else result

let private timed (runtime: Runtime) (policy: StagePolicy) (body: unit -> Value) =
    let onHedge n = event runtime policy "hedge" n true
    if isVirtual runtime then
        let began = now runtime
        let result = virtualHedge policy.Hedge body onHedge 1L
        match policy.Timeout with
        | Some t when t > 0L && now runtime - began > t -> stageFailure "TimedOut"
        | _ -> result
    elif runtime.Gates && (policy.Timeout.IsSome || policy.Hedge.IsSome) then
        runAttempts policy.Timeout policy.Hedge body onHedge
    else body ()

let private retryError (wraps: bool) (error: Value) =
    match wraps, error with
    | false, _ -> Some error
    | true, Data("lawspec.resilience::type::StageFailure::StepFailed", [ inner ]) -> Some inner
    | true, Data("lawspec.resilience::type::StageFailure::TimedOut", []) -> Some error
    | true, _ -> None

let private retryWait (runtime: Runtime) (retry: Retry) (number: int64) (reason: Value) (previous: int64) =
    match retry.Strategy with
    | Custom decide ->
        decide (Int(BigInteger number)) reason (Data("lawspec.time::type::Duration::Duration", [ Int(BigInteger previous) ]))
        |> retryDecision
    | strategy ->
        let delay = retryDelay strategy number
        let baseDelay = retryDelay strategy 2L
        Some(runtime.State.Jitter(retry.Jitter, delay, previous, baseDelay))

let rec private attempts (runtime: Runtime) (policy: StagePolicy) (body: unit -> Value) (number: int64) (previous: int64) =
    event runtime policy "start" number false
    let result = timed runtime policy body
    let isFailure = failed result
    event runtime policy "finish" number (not isFailure)
    match isFailure, policy.Retry, result with
    | true, Some retry, Data(_, [ error ]) ->
        match retryError policy.Wraps error with
        | None -> result
        | Some reason ->
            let exhausted = retry.Attempts > 0L && number >= retry.Attempts
            let refused =
                match retry.When with
                | Some condition ->
                    match condition reason with
                    | Bool holds -> not holds
                    | other -> Fail.raiseWith "BoolRequired" (sprintf "%A" other)
                | None -> false
            if exhausted || refused then result
            else
                match retryWait runtime retry (number + 1L) reason previous with
                | None -> result
                | Some delay ->
                    event runtime policy "sleep" delay true
                    sleep runtime delay
                    attempts runtime policy body (number + 1L) delay
    | _ -> result

let private gateFailure (kind: GateKind) =
    match kind with
    | Breaker -> "CircuitOpen"
    | Limit -> "RateLimited"
    | Bulkhead -> "Saturated"

let rec private passGate (runtime: Runtime) (stage: int64) (policy: StagePolicy) (g: Gate) (waited: int64) =
    match runtime.State.Gate(stage, g, now runtime) with
    | Data("lawspec.resilience::type::Gate::Admit", []) -> None
    | Data("lawspec.resilience::type::Gate::Reject", []) -> Some(gateFailure g.Kind)
    | _ when g.Wait = NoWait -> Some(gateFailure g.Kind)
    | Data("lawspec.resilience::type::Gate::WaitFor", [ Int d ]) ->
        let delay = int64 d
        match g.Wait with
        | Within most when waited + delay > most -> Some(gateFailure g.Kind)
        | _ ->
            event runtime policy "wait" delay true
            sleep runtime delay
            passGate runtime stage policy g (waited + delay)
    | other -> Fail.raiseWith "InvalidGateDecision" (sprintf "%A" other)

let private passGates runtime stage policy (gates: Gate list) =
    gates |> List.tryPick (fun g -> passGate runtime stage policy g 0L)

let private rememberUndo (runtime: Runtime) (policy: StagePolicy) (result: Value) =
    match frame.Value, policy.Compensate with
    | Some f, Some undo when obj.ReferenceEquals(f.FrameState, runtime.State) ->
        let value =
            match result with
            | Data("Either::Right", [ inner ]) -> inner
            | _ -> result
        runtime.State.Undo(f.Id, policy.Stage, fun () -> undo value)
    | _ -> ()

/// Run a stage of a workflow under its policy. The input is the stage's
/// argument, the key of its cache.
[<CompiledName("RunStage")>]
let runStage (schema: Schema.Schema) (policy: StagePolicy) (body: unit -> Value) (input: Value) : Value =
    let runtime = current schema
    let caching = runtime.Gates && policy.Cache.IsSome
    let cached = if caching then runtime.State.Cached(policy.Key, input, now runtime) else None
    match cached with
    | Some value ->
        event runtime policy "cached" 0L true
        value
    | None ->
        let stage = runtime.State.NewStage(policy.Key, now runtime)
        let gates = if runtime.Gates then policy.Gates else []
        let outcome =
            try
                Choice1Of2(
                    match passGates runtime stage policy gates with
                    | None -> attempts runtime policy body 1L 0L
                    | Some kind -> stageFailure kind
                )
            with e -> Choice2Of2 e
        let succeeded =
            match outcome with
            | Choice1Of2 v -> not (failed v)
            | Choice2Of2 _ -> false
        runtime.State.FinishStage(stage, now runtime, succeeded)
        match outcome with
        | Choice2Of2 e -> raise e
        | Choice1Of2 value ->
            if succeeded then
                rememberUndo runtime policy value
                if caching then runtime.State.Cache(policy.Key, input, value, now runtime, policy.Cache.Value)
            value

/// Run a workflow: when it fails, undo the stages that succeeded, the
/// latest first.
[<CompiledName("RunWorkflow")>]
let runWorkflow (schema: Schema.Schema) (body: unit -> Value) : Value =
    let runtime = current schema
    let id = runtime.State.NewFrame()
    let previous = frame.Value
    frame.Value <- Some { FrameState = runtime.State; Id = id }
    let outcome =
        try
            try Choice1Of2(body ())
            with e -> Choice2Of2 e
        finally
            frame.Value <- previous
    let undos = runtime.State.TakeFrame id
    match outcome with
    | Choice2Of2 e -> raise e
    | Choice1Of2 result ->
        if failed result then
            for stageName, undo in undos do
                runtime.State.Event { Kind = "compensate"; Stage = stageName; Number = 0L; Succeeded = true }
                undo () |> ignore
        result
