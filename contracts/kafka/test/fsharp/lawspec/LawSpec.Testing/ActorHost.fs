/// The actor runtime as an actor model's system: one permanent child under a
/// one-for-one supervisor whose restart budget is the run's length, so
/// injected crashes cannot exhaust it. ref:DEC-actors-otp-supervision
/// ref:DEC-stateful-models-linearizability
module LawSpec.Testing.ActorHost

open LawSpec.Runtime

/// The host actor models run on.
[<CompiledName("Host")>]
let host: Model.IActorHost =
    { new Model.IActorHost with
        member _.WithActor(restarts, start, restart, body) =
            let spec =
                Actors.supervisor Actors.OneForOne (max 1 restarts) 1_000_000L
                    [ "model", Actors.Permanent, Actors.actor (fun () -> box (start ())) (fun state -> box (restart (unbox<Value> state))) ]
            Actors.withSpec spec (fun supervisor -> body (box (Actors.child supervisor "model")))
        member _.Call(actor, update) =
            Actors.call (unbox<Actors.Handle> actor) (fun state ->
                let reply, next = update (unbox<Value> state)
                box reply, box next)
            |> unbox<Value>
        member _.Crash(actor) = Actors.crash (unbox<Actors.Handle> actor)
        member _.State(actor) = Actors.state (unbox<Actors.Handle> actor) |> unbox<Value> }
