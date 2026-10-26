/// Supervision evidence: the generated check of a program with supervisors
/// exercises real trees of the runtime's actors, every restart strategy and
/// lifetime, restart budgets and escalation, links, committed checkpoints
/// and stable mailboxes. ref:DEC-actors-otp-supervision ref:erlang-otp-supervisors
module LawSpec.Testing.Supervision

open System
open System.Threading
open LawSpec.Runtime
open LawSpec.Runtime.Actors

let private failure (label: string) (expected: obj) (actual: obj) : 'a =
    Fail.raiseWith "SupervisionFailed" (sprintf "%s: expected %A, actual %A" label expected actual)

let private expect (label: string) (expected: 'a) (actual: 'a) =
    if expected <> actual then
        failure label (box expected) (box actual)

let private counter () =
    actor (fun () -> box 0) (fun _ -> box 0)

let private counters names =
    names |> List.map (fun n -> n, Permanent, counter ())

let private tree strategy limit children body =
    withSpec (supervisor strategy limit 10_000_000L children) body

let private childrenOf sup names = names |> List.map (child sup)
let private value (h: Handle) = unbox<int>(state h)
let private states handles = handles |> List.map value

let private bump (h: Handle) =
    unbox<int>(call h (fun s -> let n = unbox<int> s + 1 in box n, box n))

let private failCall (h: Handle) =
    match
        (try
            Choice1Of2(call h (fun _ -> raise (InvalidOperationException "supervision probe")))
         with e ->
             Choice2Of2 e)
    with
    | Choice2Of2(:? LawSpecException as e) when
        e.Code = "ActorCrashed" && (e.InnerException :? InvalidOperationException)
        ->
        ()
    | other -> failure "failing handler" (box "ActorCrashed") (box other)

let private stopped (h: Handle) =
    match
        (try
            Choice1Of2(state h)
         with e ->
             Choice2Of2 e)
    with
    | Choice2Of2(:? LawSpecException as e) when e.Code = "ActorStopped" -> ()
    | other -> failure "stopped actor" (box "ActorStopped") (box other)

let private crashed (inbox: Inbox) (h: Handle) =
    match receiveEvent inbox h 2000 with
    | Some(Crashed _) -> ()
    | other -> failure "crash monitor" (box "Crashed") (box other)

let private waitFor (label: string) (signal: ManualResetEventSlim) =
    if not (signal.Wait 2000) then
        failure label (box "signal") (box "timeout")

let private strategies () =
    for strategy, expected in [ OneForOne, [ 1; 0; 1 ]; OneForAll, [ 0; 0; 0 ]; RestForOne, [ 1; 0; 0 ] ] do
        tree strategy 5 (counters [ "a"; "b"; "c" ]) (fun sup ->
            let actors = childrenOf sup [ "a"; "b"; "c" ]
            let before = actors |> List.map worker

            for a in actors do
                bump a |> ignore

            expect "three children" 3 (List.length (children sup))
            failCall actors.[1]
            expect (sprintf "%A" strategy) expected (states actors)
            let after = actors |> List.map worker

            expect
                (sprintf "%A replaced workers" strategy)
                (expected |> List.map (fun v -> v = 0))
                (List.map2 (<>) before after))

let private lifetimes () =
    tree
        OneForOne
        5
        [
            "p", Permanent, counter ()
            "t", Transient, counter ()
            "x", Temporary, counter ()
        ]
        (fun sup ->
            let p, t, x = child sup "p", child sup "t", child sup "x"
            bump p |> ignore
            stop p
            expect "permanent normal stop" 0 (value p)
            bump t |> ignore
            failCall t
            expect "transient crash" 0 (value t)
            stop t
            stopped t
            failCall x
            stopped x
            expect "remaining children" [ "p" ] (children sup |> List.map fst))

let private limits () =
    tree OneForOne 2 (counters [ "a"; "b" ]) (fun sup ->
        let a, b = child sup "a", child sup "b"
        let inbox = Inbox()
        monitor sup inbox

        for _ in 1..3 do
            failCall a

        stopped b
        crashed inbox sup
        expect "one limit notification" true (receiveEvent inbox sup 0 |> Option.isNone))

let private escalation () =
    let inner = supervisor OneForOne 1 10_000_000L (counters [ "a"; "b" ])

    tree OneForOne 5 [ "inner", Permanent, inner; "c", Permanent, counter () ] (fun sup ->
        let innerHandle, c = child sup "inner", child sup "c"
        let a, b = child innerHandle "a", child innerHandle "b"
        let old = worker innerHandle
        bump b |> ignore
        bump c |> ignore
        failCall a
        failCall a
        expect "escalated children" [ 0; 0; 1 ] (states [ a; b; c ])
        expect "replaced inner supervisor" true (old <> worker innerHandle)
        expect "outer restart count" 1 (restartCount sup)
        expect "fresh inner budget" 0 (restartCount innerHandle))

let private links () =
    withSpec (counter ()) (fun a ->
        withSpec (counter ()) (fun b ->
            link a b
            let inbox = Inbox()
            monitor b inbox
            failCall a
            crashed inbox b
            stopped b))

    tree OneForOne 10 (counters [ "a"; "b"; "c" ]) (fun sup ->
        let actors = childrenOf sup [ "a"; "b"; "c" ]
        let a, b, c = actors.[0], actors.[1], actors.[2]
        link a b
        link b c
        link c a
        let inbox = Inbox()

        for x in actors do
            bump x |> ignore
            monitor x inbox

        failCall a

        for x in actors do
            crashed inbox x

        let deadline = DateTime.UtcNow.AddSeconds 2.0

        while restartCount sup < 3 && DateTime.UtcNow < deadline do
            Thread.Sleep 1

        expect "link cycle" [ 0; 0; 0 ] (states actors)
        expect "one restart per linked actor" 3 (restartCount sup))

let private checkpointAndMailbox () =
    let spec = actor (fun () -> box 0) id

    tree OneForOne 5 [ "a", Permanent, spec ] (fun sup ->
        let a = child sup "a"
        bump a |> ignore
        failCall a
        expect "committed checkpoint" 1 (value a)
        let entered = new ManualResetEventSlim()
        let finish = new ManualResetEventSlim()
        let inbox = Inbox()
        monitor a inbox
        let w = worker a

        try
            tell a (fun s ->
                entered.Set()
                finish.Wait()
                box (), box (unbox<int> s + 10))

            waitFor "mailbox handler started" entered
            tell a (fun s -> box (), box (unbox<int> s + 2))
            kill a
            crashed inbox a
            expect "queued tell after an external kill" 3 (value a)
            expect "stable mailbox handle" a (child sup "a")
            expect "replaced killed worker" true (w <> worker a)
        finally
            finish.Set())

let private queuedSiblingRestart strategy =
    let restored = ref None

    let sibling =
        actor (fun () -> box 0) (fun s ->
            restored.Value <- Some(unbox<int> s)
            box 0)

    tree strategy 5 [ "a", Permanent, counter (); "b", Permanent, sibling ] (fun sup ->
        let a, b = child sup "a", child sup "b"
        let entered = new ManualResetEventSlim()
        let finish = new ManualResetEventSlim()
        let queued = ref None

        try
            tell b (fun s ->
                entered.Set()
                finish.Wait()
                box (), box (unbox<int> s + 1))

            waitFor "sibling started" entered

            tell b (fun s ->
                queued.Value <- Some(unbox<int> s)
                box (), box (unbox<int> s + 3))

            tell a (fun _ -> raise (InvalidOperationException "supervision probe"))
            let deadline = DateTime.UtcNow.AddSeconds 2.0

            while restartCount sup = 0 do
                if DateTime.UtcNow > deadline then
                    failure "restart admitted" (box true) (box false)

                Thread.Sleep 1
            // This tell follows the restart; the two accepted earlier belong
            // to the old checkpoint, and their order survives the restart.
            tell b (fun s -> box (), box (unbox<int> s + 10))
            finish.Set()
            expect (sprintf "%A post-restart tell" strategy) 10 (value b)
            expect (sprintf "%A queued checkpoint" strategy) (Some 1) queued.Value
            expect (sprintf "%A restart checkpoint" strategy) (Some 4) restored.Value
        finally
            finish.Set())

/// The supervision check: fails with SupervisionFailed naming what broke.
[<CompiledName("Check")>]
let check () =
    withSpec (counter ()) (fun a ->
        expect "initial increment" 1 (bump a)
        failCall a
        stopped a)

    strategies ()
    lifetimes ()
    limits ()
    escalation ()
    links ()
    checkpointAndMailbox ()

    for strategy in [ OneForAll; RestForOne ] do
        queuedSiblingRestart strategy
