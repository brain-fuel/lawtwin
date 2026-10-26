/// Workflow policies as values: retry strategies and their delays, jitter
/// drawn from an explicit SplitMix64 state, the gates a stage passes
/// (breakers, rate limits, bulkheads) and the logical failures a stage gives.
/// Durations are whole microseconds. The policy's random stream is separate
/// from the Random ability's. ref:DEC-domain-modeling-primitives ref:splitmix
/// ref:DEC-typed-core-boundary
module LawSpec.Runtime.Policy

open System.Numerics

/// How long to wait before each retry.
type Strategy =
    | Immediate
    | Fixed of delay: int64
    | Linear of delay: int64 * step: int64
    | Exponential of delay: int64 * factor: int64 * cap: int64 option
    | Fibonacci of delay: int64
    /// A checked definition decides: attempt (Int), error, previous delay
    /// (a Duration) to a RetryDecision.
    | Custom of decide: (Value -> Value -> Value -> Value)

/// How a delay is randomized.
type Jitter =
    | NoJitter
    | FullJitter
    | EqualJitter
    | DecorrelatedJitter

/// Attempts count the first try; zero attempts retries without a bound.
type Retry =
    {
        Strategy: Strategy
        Attempts: int64
        Jitter: Jitter
        /// Whether an error is worth retrying (a portable Bool).
        When: (Value -> Value) option
    }

type GateKind =
    | Breaker
    | Limit
    | Bulkhead

/// How long a stage may wait at a gate that answers WaitFor.
type Wait =
    | NoWait
    | Forever
    | Within of int64

/// A gate: its state starts at a time, admits at a time (giving the next
/// state and a Gate decision), and may record how an admitted call ended.
/// Times are portable Ints, the outcome a portable Bool.
type Gate =
    {
        Kind: GateKind
        Start: Value -> Value
        Admit: Value -> Value -> Value
        Finish: (Value -> Value -> Value -> Value) option
        Wait: Wait
    }

/// A stage's policies, as a definition with a policy declares them.
type StagePolicy =
    {
        Key: string
        Stage: string
        Retry: Retry option
        Timeout: int64 option
        Gates: Gate list
        Cache: int64 option
        Compensate: (Value -> Value) option
        Wraps: bool
        /// A further attempt starts after the delay, up to most in all.
        Hedge: (int64 * int64) option
    }

let private fail code detail : 'a = Fail.raiseWith code detail

let rec private power (b: int64) (n: int64) : int64 =
    if n = 0L then 1L
    elif n % 2L = 0L then power (b * b) (n / 2L)
    else b * power (b * b) (n / 2L)

let rec private fibonacci (n: int64) (a: int64) (b: int64) =
    if n <= 1L then a else fibonacci (n - 1L) b (a + b)

/// The delay before an attempt (from the second) of a strategy.
[<CompiledName("RetryDelay")>]
let retryDelay (strategy: Strategy) (attempt: int64) : int64 =
    if attempt < 2L then
        fail "InvalidRetryAttempt" (string attempt)

    match strategy with
    | Immediate -> 0L
    | Fixed delay -> delay
    | Linear(delay, step) -> delay + step * (attempt - 2L)
    | Exponential(delay, factor, cap) ->
        let value = delay * power factor (attempt - 2L)

        match cap with
        | Some c -> min value c
        | None -> value
    | Fibonacci delay -> delay * fibonacci (attempt - 1L) 1L 1L
    | Custom _ -> fail "CustomRetryDelay" "a custom strategy decides its own delays"

let private below (bound: int64) state =
    let n, next = Random.below (BigInteger bound) state
    int64 n, next

/// A jittered delay and the next random state.
[<CompiledName("Jittered")>]
let jitter
    (kind: Jitter)
    (delay: int64)
    (previous: int64)
    (baseDelay: int64)
    (state: Random.Seed)
    : int64 * Random.Seed =
    match kind with
    | NoJitter -> delay, state
    | FullJitter -> below (delay + 1L) state
    | EqualJitter ->
        let half = delay / 2L
        let r, next = below (delay - half + 1L) state
        half + r, next
    | DecorrelatedJitter ->
        let high = max baseDelay (previous * 3L)
        let r, next = below (high - baseDelay + 1L) state
        min delay (baseDelay + r), next

/// A custom strategy's RetryDecision: Some delay to wait, None to stop.
[<CompiledName("RetryDecision")>]
let retryDecision (decision: Value) : int64 option =
    match decision with
    | Data("lawspec.time::type::RetryDecision::RetryAfter", [ Data(_, [ Int delay ]) ]) -> Some(int64 delay)
    | Data("lawspec.time::type::RetryDecision::Stop", []) -> None
    | other -> fail "InvalidRetryDecision" (sprintf "%A" other)

/// Whether a stage's result is a failure (a Left).
[<CompiledName("Failed")>]
let failed (result: Value) =
    match result with
    | Data("Either::Left", [ _ ]) -> true
    | _ -> false

/// A stage's own failure: TimedOut, RateLimited, CircuitOpen or Saturated.
[<CompiledName("StageFailure")>]
let stageFailure (kind: string) : Value =
    match kind with
    | "TimedOut"
    | "RateLimited"
    | "CircuitOpen"
    | "Saturated" -> Data("Either::Left", [ Data("lawspec.resilience::type::StageFailure::" + kind, []) ])
    | _ -> fail "UnknownStageFailure" kind

// ---------------------------------------------------------------------------
// Builders, as generated code writes a policy

[<CompiledName("Immediate")>]
let immediate () = Immediate

[<CompiledName("Fixed")>]
let fixedDelay (delay: int64) = Fixed delay

[<CompiledName("Linear")>]
let linear (delay: int64) (step: int64) = Linear(delay, step)

[<CompiledName("Exponential")>]
let exponential (delay: int64) (factor: int64) (cap: int64 option) = Exponential(delay, factor, cap)

[<CompiledName("Fibonacci")>]
let fibonacciDelay (delay: int64) = Fibonacci delay

[<CompiledName("Custom")>]
let custom (decide: Value -> Value -> Value -> Value) = Custom decide

/// A jitter by name: none, full, equal or decorrelated.
[<CompiledName("JitterOf")>]
let jitterOf (name: string) =
    match name with
    | "none" -> NoJitter
    | "full" -> FullJitter
    | "equal" -> EqualJitter
    | "decorrelated" -> DecorrelatedJitter
    | _ -> fail "UnknownJitter" name

[<CompiledName("Retry")>]
let retry (strategy: Strategy) (attempts: int64) (jitter: Jitter) (condition: (Value -> Value) option) =
    {
        Strategy = strategy
        Attempts = attempts
        Jitter = jitter
        When = condition
    }

[<CompiledName("NoWait")>]
let noWait () = NoWait

[<CompiledName("Forever")>]
let forever () = Forever

[<CompiledName("Within")>]
let within (micros: int64) = Within micros

[<CompiledName("Breaker")>]
let breaker (start: Value -> Value) (admit: Value -> Value -> Value) (finish: Value -> Value -> Value -> Value) =
    {
        Kind = Breaker
        Start = start
        Admit = admit
        Finish = Some finish
        Wait = NoWait
    }

[<CompiledName("Limit")>]
let limit (start: Value -> Value) (admit: Value -> Value -> Value) (wait: Wait) =
    {
        Kind = Limit
        Start = start
        Admit = admit
        Finish = None
        Wait = wait
    }

[<CompiledName("Bulkhead")>]
let bulkhead
    (start: Value -> Value)
    (admit: Value -> Value -> Value)
    (finish: Value -> Value -> Value -> Value)
    (wait: Wait)
    =
    {
        Kind = Bulkhead
        Start = start
        Admit = admit
        Finish = Some finish
        Wait = wait
    }

/// A stage's policy.
[<CompiledName("Stage")>]
let stage
    (key: string)
    (stageName: string)
    (retry: Retry option)
    (timeout: int64 option)
    (gates: Gate list)
    (cache: int64 option)
    (compensate: (Value -> Value) option)
    (wraps: bool)
    (hedge: (int64 * int64) option)
    =
    {
        Key = key
        Stage = stageName
        Retry = retry
        Timeout = timeout
        Gates = gates
        Cache = cache
        Compensate = compensate
        Wraps = wraps
        Hedge = hedge
    }
