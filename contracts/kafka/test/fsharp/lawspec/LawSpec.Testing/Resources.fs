/// Resources a law's cases use: acquired before a case and released after
/// it, even when it fails, or shared across a unit's cases (`share R per
/// unit`): acquired once, reset before each use, and released when the run
/// ends. A resource that is not concurrent is used by one case at a time.
/// ref:REQ-law-primitives ref:REQ-harness-units
module LawSpec.Testing.Resources

open System
open System.Collections.Concurrent
open System.Threading
open LawSpec.Runtime

let private cleanupFailure (detail: string) (inner: exn) =
    let failure =
        LawSpecException("TestCleanupFailed", detail + ": " + inner.Message, [], inner)

    Harness.markHarnessFailure failure
    failure

/// A resource for one case: the body's outcome stands unless releasing the
/// resource fails, which fails the case.
[<CompiledName("WithResource")>]
let withResource (acquire: unit -> Value) (release: Value -> Value) (body: Value -> Value) : Value =
    let value = acquire ()

    let outcome =
        try
            Choice1Of2(body value)
        with e ->
            Choice2Of2 e

    let cleanup =
        try
            release value |> ignore
            None
        with e ->
            Some e

    match outcome, cleanup with
    | Choice1Of2 result, None -> result
    | Choice2Of2 e, None -> raise e
    | Choice1Of2 _, Some c -> raise (cleanupFailure "a resource's release failed" c)
    | Choice2Of2 e, Some c ->
        raise (cleanupFailure ("a resource's release failed after the case failed (" + e.Message + ")") c)

type private Shared(acquire: unit -> Value, reset: Value -> Value, release: Value -> Value, concurrent: bool) =
    let gate = new SemaphoreSlim(1, 1)

    let users =
        new SemaphoreSlim((if concurrent then Int32.MaxValue else 1), (if concurrent then Int32.MaxValue else 1))

    let mutable value: Value option = None
    let mutable leases = 0

    member _.Checkout() =
        users.Wait()
        gate.Wait()

        try
            try
                match value with
                | None -> value <- Some(acquire ())
                | Some v when leases = 0 -> reset v |> ignore
                | Some _ -> ()

                leases <- leases + 1
                value.Value
            with _ ->
                users.Release() |> ignore
                reraise ()
        finally
            gate.Release() |> ignore

    member _.Checkin() =
        gate.Wait()
        leases <- leases - 1
        gate.Release() |> ignore
        users.Release() |> ignore

    member _.Release() =
        match value with
        | Some v ->
            value <- None
            release v |> ignore
        | None -> ()

let private shared = ConcurrentDictionary<string, Shared>()

let private released =
    lazy
        AppDomain.CurrentDomain.ProcessExit.Add(fun _ ->
            for KeyValue(key, entry) in shared do
                try
                    entry.Release()
                with e ->
                    eprintfn "LawSpec: releasing the shared resource %s failed: %s" key e.Message)

/// A resource shared by a unit's cases.
[<CompiledName("WithShared")>]
let withShared
    (key: string)
    (acquire: unit -> Value)
    (reset: Value -> Value)
    (release: Value -> Value)
    (concurrent: bool)
    (body: Value -> Value)
    : Value =
    released.Force()

    let entry =
        shared.GetOrAdd(key, fun _ -> Shared(acquire, reset, release, concurrent))

    let value = entry.Checkout()

    try
        body value
    finally
        entry.Checkin()
