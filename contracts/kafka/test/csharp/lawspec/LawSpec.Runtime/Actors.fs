/// Actors and supervision trees on .NET. A handle names a stable mailbox, not
/// a worker: a supervised restart replaces the worker and keeps the committed
/// checkpoint and the messages waiting. Each actor handles one message at a
/// time; a handler that returns commits its next state before the reply, and
/// one that throws commits nothing. Supervisors restart their children by
/// strategy (one for one, one for all, rest for one) and lifetime (permanent,
/// transient, temporary) within a rolling restart budget, and escalate to
/// their own supervisor when it is spent. ref:DEC-actors-otp-supervision
/// ref:erlang-otp-supervisors ref:DEC-fsharp-runtime-from-scratch
module LawSpec.Runtime.Actors

open System
open System.Diagnostics
open System.Threading
open System.Threading.Tasks

/// When a supervisor restarts a child that stopped.
type Lifetime =
    /// Always.
    | Permanent
    /// Only after a crash.
    | Transient
    /// Never.
    | Temporary

/// Which children a supervisor restarts when one fails.
type Strategy =
    | OneForOne
    | OneForAll
    | RestForOne

/// What a tree runs: an actor (its start and its restart from the last
/// checkpoint) or a supervisor of named children.
[<NoEquality; NoComparison>]
type Spec =
    | ActorSpec of start: (unit -> obj) * restart: (obj -> obj) * metadata: obj
    | SupervisorSpec of
        strategy: Strategy *
        restarts: int *
        periodMicros: int64 *
        children: (string * Lifetime * Spec) list *
        metadata: obj

/// What an observer is told about a handle.
[<NoEquality; NoComparison>]
type Event =
    | Crashed of cause: exn
    | Stopped

let private fail code detail : 'a = Fail.raiseWith code detail

type internal Operation =
    | Run of (obj -> obj * obj)
    | Crash of cause: exn * origin: obj
    | Restart
    | Stop

[<NoEquality; NoComparison>]
type internal Entry =
    {
        Reply: TaskCompletionSource<obj> option
        Operation: Operation
        Context: ExecutionContext option
    }

/// A mailbox of events from the handles it monitors, received selectively.
[<Sealed; AllowNullLiteral>]
type Inbox() =
    let events = ResizeArray<obj * Event>()
    let gate = obj ()

    member internal _.Post(handle: obj, event: Event) =
        lock gate (fun () ->
            events.Add((handle, event))
            Monitor.PulseAll gate)

    /// The next event of a handle, waiting at most a number of milliseconds.
    member _.Receive(handle: obj, milliseconds: int) : Event option =
        let deadline =
            Stopwatch.GetTimestamp() + int64 milliseconds * Stopwatch.Frequency / 1000L

        lock gate (fun () ->
            let rec loop () =
                match events |> Seq.tryFindIndex (fun (h, _) -> h.Equals handle) with
                | Some i ->
                    let _, event = events.[i]
                    events.RemoveAt i
                    Some event
                | None ->
                    let left = deadline - Stopwatch.GetTimestamp()

                    if left <= 0L then
                        None
                    else
                        Monitor.Wait(
                            gate,
                            TimeSpan.FromTicks(max 1L (left * TimeSpan.TicksPerSecond / Stopwatch.Frequency))
                        )
                        |> ignore

                        loop ()

            loop ())

[<AllowNullLiteral>]
type internal Node(path: string list, spec: Spec, parent: string list option, lifetime: Lifetime) =
    member val Path = path
    member val Spec = spec
    member val Parent = parent
    member val Lifetime = lifetime
    member val Children: (string * string list) list = [] with get, set
    member val State: obj = null with get, set
    member val Checkpoint: obj option = None with get, set
    /// Run start, or restart from the checkpoint, before the next message.
    member val Init = false with get, set
    member val Worker = 0 with get, set
    /// Which incarnation of the worker runs: a restart makes a new one.
    member val Incarnation = 0 with get, set
    member val Waiting = System.Collections.Generic.Queue<Entry>() with get
    member val Active: Entry option = None with get, set
    member val Running = false with get, set
    member val Accepting = true with get, set
    member val Removed = false with get, set
    member val Observers: Inbox list = [] with get, set
    member val Links: Handle list = [] with get, set
    member val Seen = System.Collections.Generic.HashSet<obj>(HashIdentity.Reference) with get
    member val Restarts: int64 list = [] with get, set
    member val StopNotified = false with get, set

    member _.IsActor =
        match spec with
        | ActorSpec _ -> true
        | _ -> false

/// A stable handle on an actor or a supervisor of a tree.
and [<CustomEquality; NoComparison>] Handle =
    internal
        {
            Tree: Tree
            Path: string list
        }

    override this.Equals other =
        match other with
        | :? Handle as h -> obj.ReferenceEquals(this.Tree, h.Tree) && this.Path = h.Path
        | _ -> false

    override this.GetHashCode() =
        hash this.Path ^^^ Runtime.CompilerServices.RuntimeHelpers.GetHashCode this.Tree

    override this.ToString() = "actor " + String.Join("/", this.Path)

/// The tree of one started spec: its nodes, under one lock. No application
/// code runs under the lock.
and [<Sealed; AllowNullLiteral>] Tree internal (context: ExecutionContext) =
    let gate = obj ()

    let nodes =
        System.Collections.Generic.Dictionary<string list, Node>(HashIdentity.Structural)

    member internal _.Gate = gate
    member internal _.Nodes = nodes
    member internal _.Context = context

let private current = new ThreadLocal<Node>()

let private now () =
    Stopwatch.GetTimestamp() * 1_000_000L / Stopwatch.Frequency

let private stopped () : exn =
    LawSpecException("ActorStopped", "the actor has stopped") :> exn

let private crashed (cause: exn) : exn =
    LawSpecException("ActorCrashed", cause.Message, [], cause) :> exn

let private reply (entry: Entry) (result: Choice<obj, exn>) =
    match entry.Reply with
    | Some r ->
        match result with
        | Choice1Of2 v -> r.TrySetResult v |> ignore
        | Choice2Of2 e -> r.TrySetException e |> ignore
    | None -> ()

let private node (tree: Tree) path = tree.Nodes.[path]

let rec private descendants (tree: Tree) path : string list list =
    path
    :: ((node tree path).Children
        |> List.collect (fun (_, child) -> descendants tree child))

let private notify (tree: Tree) (n: Node) (event: Event) =
    for inbox in n.Observers do
        inbox.Post(box { Tree = tree; Path = n.Path }, event)

let private eligible (n: Node) = not n.Removed

/// Mark a subtree gone: every waiting caller is told the actor stopped.
let private halt (tree: Tree) path =
    for p in List.rev (descendants tree path) do
        let n = node tree p

        if not n.Removed then
            n.Active |> Option.iter (fun e -> reply e (Choice2Of2(stopped ())))

            for e in n.Waiting do
                reply e (Choice2Of2(stopped ()))

            n.Waiting.Clear()

            if not n.StopNotified then
                notify tree n Stopped

            n.Removed <- true
            n.Accepting <- false
            n.Active <- None
            n.StopNotified <- true
            n.Worker <- n.Worker + 1

let rec private schedule (tree: Tree) (n: Node) =
    if not n.Running && not n.Removed && n.IsActor && (n.Init || n.Waiting.Count > 0) then
        n.Running <- true
        let generation = n.Worker
        ThreadPool.QueueUserWorkItem(fun _ -> pump tree n generation) |> ignore

/// A supervisor restarts a group of its children: the failed actor starts
/// again from its checkpoint before its next message; every other actor
/// restarts after the messages it had already accepted.
and private restartGroup (tree: Tree) parent failed (origin: string list) =
    let supervisor = node tree parent

    let live =
        supervisor.Children
        |> List.map snd
        |> List.filter (fun c -> not (node tree c).Removed)

    let group =
        match supervisor.Spec with
        | SupervisorSpec(OneForOne, _, _, _, _) -> [ failed ]
        | SupervisorSpec(OneForAll, _, _, _, _) -> live
        | SupervisorSpec(RestForOne, _, _, _, _) -> live |> List.skipWhile (fun c -> c <> failed)
        | ActorSpec _ -> [ failed ]

    for member' in group do
        if (node tree member').Lifetime = Temporary then
            halt tree member'
        else
            for p in descendants tree member' do
                let n = node tree p

                if not n.Removed then
                    n.Accepting <- true
                    n.Incarnation <- n.Incarnation + 1

                    if n.IsActor then
                        if p = origin then
                            n.Init <- true
                        else
                            n.Waiting.Enqueue
                                {
                                    Reply = None
                                    Operation = Restart
                                    Context = None
                                }

                        schedule tree n
                    else
                        n.Restarts <- []

/// Decide what a stop (normal) or crash (abnormal) means: a restart within the
/// parent's budget, an escalation, or the end of the subtree.
and private decide (tree: Tree) path (abnormal: bool) (origin: string list) =
    let n = node tree path

    let restart =
        eligible n && (n.Lifetime = Permanent || (n.Lifetime = Transient && abnormal))

    match n.Parent with
    | Some parent when restart ->
        let supervisor = node tree parent

        match supervisor.Spec with
        | SupervisorSpec(_, limit, period, _, _) ->
            let at = now ()
            let recent = supervisor.Restarts |> List.filter (fun t -> at - t <= period)

            if List.length recent < limit && eligible supervisor then
                supervisor.Restarts <- recent @ [ at ]
                restartGroup tree parent path origin
            else
                decide tree parent true origin
        | ActorSpec _ -> halt tree path
    | _ ->
        if not n.IsActor && abnormal then
            notify tree n (Crashed(LawSpecException("RestartLimit", String.Join("/", path)) :> exn))
            n.StopNotified <- true

        halt tree path

/// An actor failed: its caller learns why, its observers and linked actors
/// are told, and its supervisor decides.
and private failed (tree: Tree) (n: Node) (entry: Entry option) (cause: exn) (origin: obj) =
    match entry with
    | Some({ Operation = Crash _ } as e) -> reply e (Choice1Of2(box ()))
    | Some e -> reply e (Choice2Of2(crashed cause))
    | None -> ()

    n.Active <- None
    n.Seen.Add origin |> ignore
    notify tree n (Crashed cause)
    n.StopNotified <- true
    let links = n.Links
    decide tree n.Path true n.Path

    if not n.Removed then
        n.StopNotified <- false

    links

and private propagate (links: Handle list) (cause: exn) (origin: obj) =
    for other in links do
        let t = other.Tree

        lock t.Gate (fun () ->
            match t.Nodes.TryGetValue other.Path with
            | true, n when n.Accepting && not n.Removed && not (n.Seen.Contains origin) ->
                n.Seen.Add origin |> ignore

                n.Waiting.Enqueue
                    {
                        Reply = None
                        Operation = Crash(cause, origin)
                        Context = None
                    }

                schedule t n
            | _ -> ())

and private pump (tree: Tree) (n: Node) (generation: int) =
    let mutable go = true

    while go do
        let origin = obj ()

        let step =
            lock tree.Gate (fun () ->
                if n.Worker <> generation || n.Removed then
                    if n.Worker = generation then
                        n.Running <- false

                    None
                elif n.Init then
                    Some(Choice1Of2 n.Checkpoint)
                elif n.Waiting.Count > 0 then
                    let e = n.Waiting.Dequeue()
                    n.Active <- Some e
                    Some(Choice2Of2 e)
                else
                    n.Running <- false
                    None)

        match step with
        | None -> go <- false
        | Some(Choice1Of2 checkpoint) ->
            let start, restart =
                match n.Spec with
                | ActorSpec(s, r, _) -> s, r
                | SupervisorSpec _ -> failwith "unreachable"

            let outcome =
                try
                    let mutable value = null

                    ExecutionContext.Run(
                        tree.Context.CreateCopy(),
                        (fun _ ->
                            value <-
                                (match checkpoint with
                                 | Some c -> restart c
                                 | None -> start ())),
                        null
                    )

                    Choice1Of2 value
                with e ->
                    Choice2Of2 e

            let links =
                lock tree.Gate (fun () ->
                    if n.Worker <> generation then
                        []
                    else
                        match outcome with
                        | Choice1Of2 v ->
                            n.State <- v
                            n.Checkpoint <- Some v
                            n.Init <- false
                            []
                        | Choice2Of2 e -> failed tree n None e origin)

            match outcome with
            | Choice2Of2 e when not (List.isEmpty links) -> propagate links e origin
            | _ -> ()
        | Some(Choice2Of2 entry) ->
            let state = lock tree.Gate (fun () -> n.State)

            let run (body: unit -> obj * obj) =
                current.Value <- n

                try
                    try
                        let mutable result = (null, null)

                        match entry.Context with
                        | Some c -> ExecutionContext.Run(c.CreateCopy(), (fun _ -> result <- body ()), null)
                        | None -> ExecutionContext.Run(tree.Context.CreateCopy(), (fun _ -> result <- body ()), null)

                        Choice1Of2 result
                    with e ->
                        Choice2Of2 e
                finally
                    current.Value <- null

            match entry.Operation with
            | Stop ->
                lock tree.Gate (fun () ->
                    if n.Worker = generation then
                        n.Active <- None
                        reply entry (Choice1Of2(box ()))
                        decide tree n.Path false n.Path)
            | Crash(cause, origin) ->
                let links =
                    lock tree.Gate (fun () ->
                        if n.Worker = generation then
                            failed tree n (Some entry) cause origin
                        else
                            [])

                propagate links cause origin
            | Run handler ->
                let outcome = run (fun () -> handler state)

                let links =
                    lock tree.Gate (fun () ->
                        if n.Worker <> generation then
                            []
                        else
                            match outcome with
                            | Choice1Of2(answer, next) ->
                                n.State <- next
                                n.Checkpoint <- Some next
                                n.Active <- None
                                reply entry (Choice1Of2 answer)
                                []
                            | Choice2Of2 e -> failed tree n (Some entry) e origin)

                match outcome with
                | Choice2Of2 e when not (List.isEmpty links) -> propagate links e origin
                | _ -> ()
            | Restart ->
                let restart =
                    match n.Spec with
                    | ActorSpec(_, r, _) -> r
                    | _ -> id

                let outcome = run (fun () -> box (), restart state)

                let links =
                    lock tree.Gate (fun () ->
                        if n.Worker <> generation then
                            []
                        else
                            match outcome with
                            | Choice1Of2(_, next) ->
                                n.State <- next
                                n.Checkpoint <- Some next
                                n.Active <- None
                                []
                            | Choice2Of2 e -> failed tree n (Some entry) e origin)

                match outcome with
                | Choice2Of2 e when not (List.isEmpty links) -> propagate links e origin
                | _ -> ()

// ---------------------------------------------------------------------------
// Specs

/// An actor: its state's start, and its state after a crash from the last
/// committed one.
[<CompiledName("Actor")>]
let actor (start: unit -> obj) (restart: obj -> obj) : Spec = ActorSpec(start, restart, null)

/// An actor whose state after a crash is a fresh start.
[<CompiledName("ActorOf")>]
let actorOf (start: unit -> obj) : Spec =
    ActorSpec(start, (fun _ -> start ()), null)

/// A supervisor: its strategy, at most a number of restarts within a period
/// (microseconds), and its children in start order, each named and with its
/// lifetime.
[<CompiledName("Supervisor")>]
let supervisor
    (strategy: Strategy)
    (restarts: int)
    (periodMicros: int64)
    (children: (string * Lifetime * Spec) list)
    : Spec =
    if restarts < 0 || periodMicros <= 0L then
        fail "InvalidSupervisor" "a supervisor needs restarts >= 0 and a positive period"

    let names = children |> List.map (fun (n, _, _) -> n)

    if List.length names <> List.length (List.distinct names) then
        fail "InvalidSupervisor" "duplicate child names"

    SupervisorSpec(strategy, restarts, periodMicros, children, null)

/// A spec with metadata a generated API reads back from its handle.
[<CompiledName("WithMetadata")>]
let withMetadata (metadata: obj) (spec: Spec) : Spec =
    match spec with
    | ActorSpec(s, r, _) -> ActorSpec(s, r, metadata)
    | SupervisorSpec(st, n, p, c, _) -> SupervisorSpec(st, n, p, c, metadata)

// ---------------------------------------------------------------------------
// Starting and stopping trees

let rec private allocate (tree: Tree) path (spec: Spec) parent lifetime =
    let n = Node(path, spec, parent, lifetime)
    tree.Nodes.[path] <- n

    match spec with
    | SupervisorSpec(_, _, _, children, _) ->
        n.Children <- children |> List.map (fun (name, _, _) -> name, path @ [ name ])

        for name, life, child in children do
            allocate tree (path @ [ name ]) child (Some path) life
    | ActorSpec _ -> n.Init <- true

/// Stop a handle: an actor finishes the messages it accepted first; a
/// supervisor stops its whole subtree. Stopping a stopped handle does nothing.
[<CompiledName("Stop")>]
let stop (handle: Handle) =
    let tree = handle.Tree

    let wait =
        lock tree.Gate (fun () ->
            let n = node tree handle.Path

            if n.Removed then
                None
            elif n.IsActor then
                n.Accepting <- n.Parent.IsSome && n.Lifetime = Permanent

                let r =
                    TaskCompletionSource<obj>(TaskCreationOptions.RunContinuationsAsynchronously)

                n.Waiting.Enqueue
                    {
                        Reply = Some r
                        Operation = Stop
                        Context = None
                    }

                schedule tree n
                Some r.Task
            else
                match n.Parent with
                | Some _ when n.Lifetime = Permanent -> decide tree handle.Path false handle.Path
                | _ -> halt tree handle.Path

                None)

    match wait with
    | Some task ->
        (try
            task.Wait()
         with _ ->
             ())
    | None -> ()

/// Start a spec. Its actors start in order; a start that fails stops the
/// tree and fails here. An actor alone runs as the temporary child of a
/// supervisor with no restarts; its handle is the actor's.
[<CompiledName("Start")>]
let start (spec: Spec) : Handle =
    let tree = Tree(ExecutionContext.Capture())

    let root, public' =
        match spec with
        | ActorSpec _ -> supervisor OneForOne 0 1L [ "actor", Temporary, spec ], [ "actor" ]
        | SupervisorSpec _ -> spec, []

    allocate tree [] root None Permanent

    let actors =
        descendants tree [] |> List.map (node tree) |> List.filter (fun n -> n.IsActor)

    for n in actors do
        match n.Spec with
        | ActorSpec(s, _, _) ->
            let value =
                try
                    let mutable v = null
                    ExecutionContext.Run(tree.Context.CreateCopy(), (fun _ -> v <- s ()), null)
                    v
                with e ->
                    lock tree.Gate (fun () -> halt tree [])
                    raise (LawSpecException("ActorStartFailed", e.Message, [], e))

            lock tree.Gate (fun () ->
                n.State <- value
                n.Checkpoint <- Some value
                n.Init <- false)
        | SupervisorSpec _ -> ()

    { Tree = tree; Path = public' }

/// Run a body with a started spec, stopping its tree afterwards.
[<CompiledName("WithSpec")>]
let withSpec (spec: Spec) (body: Handle -> 'a) : 'a =
    let handle = start spec

    try
        body handle
    finally
        stop { handle with Path = [] }

// ---------------------------------------------------------------------------
// Messages

let private enqueue (handle: Handle) (operation: Operation) (wait: bool) : Task<obj> =
    let tree = handle.Tree

    lock tree.Gate (fun () ->
        let n = node tree handle.Path

        if obj.ReferenceEquals(current.Value, n) && wait then
            fail "ActorSelfCall" "an actor cannot call itself"

        if not n.IsActor || not n.Accepting || n.Removed then
            Task.FromException<obj>(stopped ())
        else
            let r =
                TaskCompletionSource<obj>(TaskCreationOptions.RunContinuationsAsynchronously)

            n.Waiting.Enqueue
                {
                    Reply = Some r
                    Operation = operation
                    Context = Some(ExecutionContext.Capture())
                }

            schedule tree n
            r.Task)

let private await (task: Task<obj>) =
    try
        task.GetAwaiter().GetResult()
    with :? LawSpecException ->
        reraise ()

/// Send a handler and wait for its reply. The handler gets the state and
/// gives the reply and the next state.
[<CompiledName("Call")>]
let call (handle: Handle) (handler: obj -> obj * obj) : obj =
    await (enqueue handle (Run handler) true)

/// Send a handler without waiting for it.
[<CompiledName("Tell")>]
let tell (handle: Handle) (handler: obj -> obj * obj) =
    let task = enqueue handle (Run handler) false

    if task.IsFaulted then
        await task |> ignore

/// The actor's committed state.
[<CompiledName("State")>]
let state (handle: Handle) : obj = call handle (fun s -> s, s)

/// Crash the actor in order with its messages, as a failing handler would.
[<CompiledName("Crash")>]
let crash (handle: Handle) =
    enqueue handle (Crash(LawSpecException("CrashedOnPurpose", "") :> exn, obj ())) true
    |> await
    |> ignore

/// Kill the actor's worker now, as an external failure: a handler it is
/// running commits nothing and its caller learns the actor crashed; the
/// messages waiting run after its supervisor restarts it.
[<CompiledName("Kill")>]
let kill (handle: Handle) =
    let tree = handle.Tree
    let origin = obj ()

    let links, cause =
        lock tree.Gate (fun () ->
            let n = node tree handle.Path

            if n.Removed || not n.IsActor then
                [], null
            else
                let cause = LawSpecException("ActorKilled", "the actor's worker was killed") :> exn
                n.Worker <- n.Worker + 1
                n.Incarnation <- n.Incarnation + 1
                n.Running <- false
                let links = failed tree n n.Active cause origin
                schedule tree n
                links, cause)

    if not (isNull cause) then
        propagate links cause origin

/// A supervisor's child by name (stable across restarts).
[<CompiledName("Child")>]
let child (handle: Handle) (name: string) : Handle =
    lock handle.Tree.Gate (fun () ->
        match (node handle.Tree handle.Path).Children |> List.tryFind (fun (n, _) -> n = name) with
        | Some(_, path) -> { handle with Path = path }
        | None -> fail "UnknownChild" name)

/// A supervisor's children that have not stopped for good, in order.
[<CompiledName("Children")>]
let children (handle: Handle) : (string * Handle) list =
    lock handle.Tree.Gate (fun () ->
        (node handle.Tree handle.Path).Children
        |> List.filter (fun (_, p) -> not (node handle.Tree p).Removed)
        |> List.map (fun (n, p) -> n, { handle with Path = p }))

/// Restarts a supervisor has made in its current period.
[<CompiledName("RestartCount")>]
let restartCount (handle: Handle) : int =
    lock handle.Tree.Gate (fun () -> List.length (node handle.Tree handle.Path).Restarts)

/// The worker now behind a handle (it changes when the worker is replaced),
/// or None when the handle has stopped.
[<CompiledName("Worker")>]
let worker (handle: Handle) : int option =
    lock handle.Tree.Gate (fun () ->
        let n = node handle.Tree handle.Path
        if n.Removed then None else Some n.Incarnation)

/// The metadata of the spec a handle was started from.
[<CompiledName("Metadata")>]
let metadata (handle: Handle) : obj =
    lock handle.Tree.Gate (fun () ->
        match (node handle.Tree handle.Path).Spec with
        | ActorSpec(_, _, m)
        | SupervisorSpec(_, _, _, _, m) -> m)

/// Tell an inbox about a handle's crashes and its stop.
[<CompiledName("Monitor")>]
let monitor (handle: Handle) (inbox: Inbox) =
    lock handle.Tree.Gate (fun () ->
        let n = node handle.Tree handle.Path
        n.Observers <- inbox :: n.Observers)

/// Link two actors: a crash of either crashes the other.
[<CompiledName("Link")>]
let link (left: Handle) (right: Handle) =
    lock left.Tree.Gate (fun () -> let n = node left.Tree left.Path in n.Links <- right :: n.Links)
    lock right.Tree.Gate (fun () -> let n = node right.Tree right.Path in n.Links <- left :: n.Links)

/// The next event of a handle in an inbox, waiting at most a number of
/// milliseconds.
[<CompiledName("ReceiveEvent")>]
let receiveEvent (inbox: Inbox) (handle: Handle) (milliseconds: int) : Event option =
    inbox.Receive(box handle, milliseconds)
