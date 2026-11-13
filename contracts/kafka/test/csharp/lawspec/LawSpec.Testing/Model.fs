/// Portable stateful model runs over checked logical callbacks. A fresh
/// context owns each generation, replay and shrink attempt, so no handler
/// state or workflow state is shared between attempts. The draws, the
/// rejected reference steps, the crash injection of actor models and the
/// shrink order are those of every other runtime, so one seed replays one
/// run on every target. ref:DEC-stateful-models-linearizability
/// ref:DEC-portable-seeded-generation ref:DEC-shrink-within-domain
module LawSpec.Testing.Model

open System
open System.Numerics
open LawSpec.Runtime
open LawSpec.Runtime.Descriptors

/// What a callback runs with: the schema, with the model's production
/// handlers in scope, and the case's symbols.
type CallbackContext = Schema.Schema * Scalar.SymbolContext

/// A checked entry of the program: the case's context and its arguments.
type Callback = CallbackContext -> Value list -> Value

/// A checked entry as a callback: the context's schema and symbols, then
/// its arguments in order. Generated models name each entry with the call
/// of its arity.
let call0 (entry: Schema.Schema -> Scalar.SymbolContext -> Value) : Callback =
    fun (schema, symbols) _ -> entry schema symbols

let call1 (entry: Schema.Schema -> Scalar.SymbolContext -> Value -> Value) : Callback =
    fun (schema, symbols) arguments -> entry schema symbols (Runtime.item 0 arguments)

let call2 (entry: Schema.Schema -> Scalar.SymbolContext -> Value -> Value -> Value) : Callback =
    fun (schema, symbols) arguments -> entry schema symbols (Runtime.item 0 arguments) (Runtime.item 1 arguments)

let call3 (entry: Schema.Schema -> Scalar.SymbolContext -> Value -> Value -> Value -> Value) : Callback =
    fun (schema, symbols) arguments ->
        entry schema symbols (Runtime.item 0 arguments) (Runtime.item 1 arguments) (Runtime.item 2 arguments)

let call4 (entry: Schema.Schema -> Scalar.SymbolContext -> Value -> Value -> Value -> Value -> Value) : Callback =
    fun (schema, symbols) arguments ->
        entry
            schema
            symbols
            (Runtime.item 0 arguments)
            (Runtime.item 1 arguments)
            (Runtime.item 2 arguments)
            (Runtime.item 3 arguments)

/// Runs a body in a fresh case context: generated code makes a symbol
/// context, takes the program's schema and installs production handlers
/// (Effects.withScope) around the body.
type ContextFactory = (CallbackContext -> obj) -> obj

/// What an actor model needs from the actor runtime (LawSpec.Runtime.Actors):
/// a supervised actor holding the system's state, restarted after a crash.
type IActorHost =
    /// Start one permanent child under a one-for-one supervisor that allows
    /// the given number of restarts, run the body with the child, then stop.
    /// start makes the first state; restart makes a state from the state at
    /// the crash.
    abstract WithActor: restarts: int * start: (unit -> Value) * restart: (Value -> Value) * body: (obj -> 'r) -> 'r
    /// Run an update on the actor's state in the actor: its reply and next state.
    abstract Call: actor: obj * update: (Value -> Value * Value) -> Value
    /// Crash the actor, waiting until its supervisor has restarted it.
    abstract Crash: actor: obj -> unit
    /// The actor's current state.
    abstract State: actor: obj -> Value

type StepKind =
    | Command
    | Crash

/// A command of the model (or the injected crash of an actor model).
type Step =
    {
        Name: string
        Arguments: Descriptor list
        StatePosition: int
        ReturnsUnit: bool
        Needs: (string * BigInteger) list
        Shifts: (string * BigInteger) list
        Key: int option
        Restart: bool
        Kind: StepKind
        Run: Callback
        Reference: Callback
        When: Callback option
    }

type Model =
    {
        Name: string
        Shared: bool
        Actor: bool
        Table: Map<string, Descriptor>
        StartArguments: Descriptor list
        StartIndices: BigInteger list
        StartRun: Callback
        StartModel: Callback
        Commands: Step[]
        Steps: Step[]
        RestartStep: Step option
        Abstract: Callback option
        Invariants: (string * Callback) list
        PerKey: bool
        Consistency: string
        Context: ContextFactory
        Actors: IActorHost option
    }

/// A case of the run: the model's context, its start arguments and the model.
type Context =
    {
        Value: CallbackContext
        Start: Value list
        Model: Model
    }

/// A run: start arguments, and each step's index and arguments.
type Run = Value list * (int * Value list) list

/// Why a run failed, and at which step (0 before the first).
type Failure = { Step: int; Reason: string }

let private fail code detail : 'a = Fail.raiseWith code detail

let private require condition code =
    if not condition then
        fail code ""

let private noCallback: Callback = fun _ _ -> fail "ModelMissingCallback" ""

let private atom d =
    match d with
    | Atom a -> a
    | Quoted q -> q
    | Number n -> n.ToString()
    | _ -> fail "InvalidModelSpec" (sprintf "%A" d)

let private fields (forms: Descriptor list) =
    forms
    |> List.choose (function
        | Forms(Atom k :: vs) -> Some(k, vs)
        | _ -> None)
    |> Map.ofList

let private one key (f: Map<string, Descriptor list>) =
    match f.TryFind key with
    | Some [ v ] -> v
    | _ -> fail "InvalidModelSpec" key

let private flag key (forms: Descriptor list) =
    forms |> List.contains (Forms [ Atom key; Atom "true" ])

let private form key (forms: Descriptor list) =
    forms
    |> List.pick (function
        | Forms(Atom k :: rest) when k = key -> Some rest
        | _ -> None)

let private pairs (ds: Descriptor list) =
    ds
    |> List.map (function
        | Forms [ Atom k; Number n ] -> k, n
        | d -> fail "InvalidModelSpec" (sprintf "%A" d))

let private readCommand (forms: Descriptor list) (run: Callback, reference: Callback, ``when``: Callback option) =
    match forms with
    | Atom "command" :: name :: rest ->
        let f = fields rest

        {
            Name = atom name
            Arguments = defaultArg (f.TryFind "arguments") []
            StatePosition =
                (match one "state" f with
                 | Number n -> int n
                 | d -> fail "InvalidModelSpec" (sprintf "%A" d))
            ReturnsUnit = atom (one "unit" f) = "true"
            Needs = pairs (defaultArg (f.TryFind "needs") [])
            Shifts = pairs (defaultArg (f.TryFind "shifts") [])
            Key =
                (match one "key" f with
                 | Number n -> Some(int n)
                 | _ -> None)
            Restart = atom (one "restart" f) = "true"
            Kind = Command
            Run = run
            Reference = reference
            When = ``when``
        }
    | _ -> fail "InvalidModelSpec" "command"

let private defaultContext: ContextFactory =
    fun body -> body (Schema.create [] [] 64, Scalar.SymbolContext())

/// A model from its spec text (LawSpec.MachineSpec), its start entries (the
/// system's and the reference's), each command's run, reference and optional
/// precondition, its optional abstraction and its invariants.
[<CompiledName("Create")>]
let create
    (spec: string)
    (startRun: Callback, startModel: Callback)
    (callbacks: (Callback * Callback * Callback option) list)
    (abstractRun: Callback option)
    (invariants: Callback list)
    : Model =
    let forms = read spec

    let name, sharing =
        match List.head forms with
        | Forms [ Atom "machine"; name; sharing ] -> atom name, atom sharing
        | _ -> fail "InvalidModelSpec" "machine"

    let start = fields (form "start" forms)

    let commandForms =
        forms
        |> List.choose (function
            | Forms(Atom "command" :: _ as f) -> Some f
            | _ -> None)

    let kinds = form "invariants" forms |> List.map atom
    require (List.length commandForms = List.length callbacks) "ModelCommandCallbacks"
    require (List.length kinds = List.length invariants) "ModelInvariantCallbacks"
    let all = List.map2 readCommand commandForms callbacks
    let restarts = all |> List.filter (fun c -> c.Restart)
    require (List.length restarts <= 1) "ModelRestartCallbacks"
    let commands = all |> List.filter (fun c -> not c.Restart) |> Array.ofList
    let actor = flag "actor" forms
    let consistency = atom (List.head (form "consistency" forms))
    require (sharing = "linear" || sharing = "shared") "ModelInvalidSharing"
    require (List.contains consistency [ "linearizable"; "sequential"; "causal"; "eventual" ]) "ModelInvalidConsistency"
    require (consistency <> "eventual" || abstractRun.IsSome) "ModelEventualRequiresAbstraction"
    require (not actor || sharing = "shared") "ModelActorMustBeShared"
    require (actor || List.isEmpty restarts) "ModelRestartRequiresActor"

    let crash =
        {
            Name = "crash"
            Arguments = []
            StatePosition = 0
            ReturnsUnit = true
            Needs = []
            Shifts = []
            Key = None
            Restart = false
            Kind = Crash
            Run = noCallback
            Reference = noCallback
            When = None
        }

    {
        Name = name
        Shared = (sharing = "shared")
        Actor = actor
        Table =
            forms
            |> List.choose (function
                | Forms(Atom "data" :: n :: _) as d -> Some(atom n, d)
                | _ -> None)
            |> Map.ofList
        StartArguments = defaultArg (start.TryFind "arguments") []
        StartIndices =
            defaultArg (start.TryFind "indices") []
            |> List.map (function
                | Number n -> n
                | d -> fail "InvalidModelSpec" (sprintf "%A" d))
        StartRun = startRun
        StartModel = startModel
        Commands = commands
        Steps = Array.append commands (if actor then [| crash |] else [||])
        RestartStep = List.tryHead restarts
        Abstract = abstractRun
        Invariants = List.zip kinds invariants
        PerKey = flag "perkey" forms
        Consistency = consistency
        Context = defaultContext
        Actors = None
    }

/// The model with the context factory generated code supplies.
[<CompiledName("WithContext")>]
let withContextFactory (factory: ContextFactory) (model: Model) = { model with Context = factory }

/// The model with the actor runtime an actor model runs its system on.
[<CompiledName("WithActors")>]
let withActors (host: IActorHost) (model: Model) = { model with Actors = Some host }

/// A context factory: fresh symbols, the program's schema, and the given
/// handler factories installed in a scope of their own around each body.
[<CompiledName("Context")>]
let context (schema: Schema.Schema) (factories: (string * (Schema.Schema -> Effects.Handler)) list) : ContextFactory =
    // Bound first, so the function compiles with two arguments for C#.
    let factory body =
        let symbols = Scalar.SymbolContext()

        if List.isEmpty factories then
            body (schema, symbols)
        else
            Effects.withScope schema factories (fun scoped -> body (scoped, symbols))

    factory

/// The step at an index (commands, then the injected crash).
[<CompiledName("Command")>]
let command (model: Model) (index: int) =
    if index < 0 || index >= model.Steps.Length then
        fail "ModelInvalidStep" (string index)

    model.Steps.[index]

let private zipShort a b = Seq.zip a b |> List.ofSeq

/// Whether a command's index preconditions admit the current indices.
[<CompiledName("Admits")>]
let admits (step: Step) (indices: BigInteger list) =
    zipShort step.Needs indices
    |> List.forall (fun ((kind, n), i) -> if kind = "atleast" then i >= n else i = n)

/// The indices after a command.
[<CompiledName("Shifted")>]
let shifted (step: Step) (indices: BigInteger list) =
    if List.isEmpty step.Shifts then
        indices
    else
        zipShort step.Shifts indices
        |> List.map (fun ((kind, n), i) -> if kind = "by" then i + n else n)

/// Run a body in a fresh case context.
[<CompiledName("WithCaseContext")>]
let withCase (model: Model) (startArgs: Value list) (body: Context -> 'a) : 'a =
    model.Context(fun value ->
        box (
            body
                {
                    Value = value
                    Start = startArgs
                    Model = model
                }
        ))
    |> unbox<'a>

[<CompiledName("Callback")>]
let callback (f: Callback) (context: Context) (arguments: Value list) = f context.Value arguments

[<CompiledName("Initial")>]
let initial (model: Model) (context: Context) =
    callback model.StartModel context context.Start

/// The reference's step: the next model state and the reply, or None when
/// the reference rejects it. An invalid reference removes a candidate; it
/// never turns the implementation's exception into a pass.
[<CompiledName("Step")>]
let step (s: Step) (context: Context) (args: Value list) (state: Value) : (Value * Value) option =
    try
        match s.Kind with
        | Crash ->
            let next =
                match context.Model.RestartStep with
                | None -> initial context.Model context
                | Some r -> callback r.Reference context [ state ]

            Some(next, Unit)
        | Command ->
            let allowed =
                match s.When with
                | None -> true
                | Some w -> callback w context [ state ] = Bool true

            if not allowed then
                None
            else
                let out = callback s.Reference context (args @ [ state ])

                if s.ReturnsUnit then
                    Some(out, Unit)
                else
                    match out with
                    | Data(_, [ reply; next ]) -> Some(next, reply)
                    | _ -> None
    with _ ->
        None

// Bounds are instantiated from the abstract run, before drawing any values.
let rec private modelBound indices expression =
    match expression with
    | Number n -> n
    | Forms [ Atom "index"; Number k ] -> List.item (int k) indices
    | Forms [ Atom op; a; b ] ->
        let x, y = modelBound indices a, modelBound indices b

        match op with
        | "+" -> x + y
        | "-" -> x - y
        | "*" -> x * y
        | _ -> fail "UnsupportedModelArgumentBound" op
    | _ -> fail "UnsupportedModelArgumentBound" ""

let private modelBase descriptor =
    match descriptor with
    | Forms(Atom "dependent" :: baseType :: _) -> baseType
    | d -> d

let private modelArgument indices descriptor =
    match descriptor with
    | Forms(Atom "dependent" :: Forms [ Atom "int"; name; lo; hi ] :: conditions) ->
        let bound =
            function
            | Number n -> Some n
            | _ -> None

        let tighten (low, high) condition =
            match condition with
            | Forms [ Atom op; expression ] ->
                let n = modelBound indices expression
                let l = n + (if op = ">" then BigInteger.One else BigInteger.Zero)
                let h = n - (if op = "<" then BigInteger.One else BigInteger.Zero)

                let low =
                    if List.contains op [ ">"; ">="; "==" ] then
                        Some(Option.fold max l low)
                    else
                        low

                let high =
                    if List.contains op [ "<"; "<="; "==" ] then
                        Some(Option.fold min h high)
                    else
                        high

                low, high
            | _ -> fail "UnsupportedModelArgumentBound" ""

        let low, high = List.fold tighten (bound lo, bound hi) conditions

        Forms
            [
                Atom "int"
                name
                Option.fold (fun _ n -> Number n) Hole low
                Option.fold (fun _ n -> Number n) Hole high
            ]
    | d -> d

let private modelArguments indices (c: Step) =
    List.map (modelArgument indices) c.Arguments

let private emptyModelArguments ds =
    ds
    |> List.exists (function
        | Forms [ Atom "int"; _; Number lo; Number hi ] -> lo > hi
        | Forms [ Atom "oneof"; _ ] -> true
        | _ -> false)

let private validModelArguments ds args =
    List.forall2
        (fun d value ->
            match d, value with
            | Forms [ Atom "int"; _; lo; hi ], Int n ->
                (match lo with
                 | Number l -> n >= l
                 | _ -> true)
                && (match hi with
                    | Number h -> n <= h
                    | _ -> true)
            | _ -> true)
        ds
        args

/// The model states a run passes through, or None when the reference
/// rejects it.
[<CompiledName("Simulate")>]
let simulate (model: Model) ((startArgs, steps): Run) : Value list option =
    withCase model startArgs (fun context ->
        try
            let rec go steps indices state states =
                match steps with
                | [] -> Some(List.rev (state :: states))
                | (i, args) :: rest ->
                    let c = command model i

                    if
                        not (admits c indices)
                        || not (validModelArguments (modelArguments indices c) args)
                    then
                        None
                    else
                        match step c context args state with
                        | None -> None
                        | Some(next, _) -> go rest (shifted c indices) next (state :: states)

            go steps model.StartIndices (initial model context) []
        with _ ->
            None)

let private arguments (model: Model) (ds: Descriptor list) (random: Random.Seed) (size: int) =
    let mutable r = random

    let values =
        ds
        |> List.map (fun d ->
            let v, next = generate d model.Table r size
            r <- next
            v)

    values, r

// Generation honours the reference. A step is drawn among the commands
// enabled at the model state: typestate admits them, their precondition
// holds, and some arguments are inside the reference's refinements. When a
// command's argument tuples are few (finiteSteps), each is tried against the
// reference and one valid tuple is chosen, as QuickCheck state machines
// generate from a precondition. Otherwise arguments are drawn and checked
// again, up to stepAttempts times, and every draw the reference rejects is
// counted. A run where the reference rejected more draws than it accepted
// shows little, so its check is incomplete, never passed.
// ref:DEC-honest-model-steps

/// The most argument tuples of one step that are enumerated instead of drawn.
let private finiteSteps = 64

/// How many times a step's arguments are drawn before the step is given up.
let private stepAttempts = 8

/// The commands enabled at a state, each with its valid argument tuples, or
/// None to draw them.
let private enabledSteps (model: Model) (context: Context) (state: Value) (indices: BigInteger list) =
    let holds (c: Step) =
        match c.When with
        | None -> true
        | Some guard ->
            try
                callback guard context [ state ] = Bool true
            with _ ->
                false

    [
        for i, c in Array.indexed model.Commands do
            if admits c indices then
                let descriptors = modelArguments indices c

                if not (emptyModelArguments descriptors) && holds c then
                    let domains = descriptors |> List.map (fun d -> finite d model.Table finiteSteps)

                    let tuples =
                        if List.forall Option.isSome domains then
                            product (List.choose id domains) finiteSteps
                        else
                            None

                    match tuples with
                    | None -> yield i, None
                    | Some tuples ->
                        // A tuple outside the reference is not a step here.
                        let valid = tuples |> List.filter (fun t -> (step c context t state).IsSome)

                        if not (List.isEmpty valid) then
                            yield i, Some valid
    ]

/// One drawn step: no command enabled (the run stops), every draw rejected
/// (the run goes on from the same state), or a step and the model state
/// after it.
type Next =
    | NoneEnabled
    | NoStep
    | Taken of step: (int * Value list) * state: Value

/// The next step of a run at a state and indices, how many of its drawn
/// arguments the reference rejected, and the next random state.
[<CompiledName("NextStep")>]
let nextStep
    (model: Model)
    (context: Context)
    (random: Random.Seed)
    (state: Value)
    (indices: BigInteger list)
    (size: int)
    (crashes: bool)
    : Next * int * Random.Seed =
    match enabledSteps model context state indices with
    | [] -> NoneEnabled, 0, random
    | enabled ->
        let choice, r1 = Random.below (BigInteger(List.length enabled)) random
        let index, tuples = List.item (int choice) enabled

        let taken c (i, args) =
            match step c context args state with
            | Some(next, _) -> Taken((i, args), next)
            | None -> NoStep

        // One step in eight of an actor's run is a crash.
        let crash, r2 =
            if crashes && model.Actor then
                let n, r = Random.below (BigInteger 8) r1
                n.IsZero, r
            else
                false, r1

        if crash then
            taken (command model model.Commands.Length) (model.Commands.Length, []), 0, r2
        else
            let c = command model index

            match tuples with
            | Some [ only ] -> taken c (index, only), 0, r2
            | Some tuples ->
                let k, r3 = Random.below (BigInteger(List.length tuples)) r2
                taken c (index, List.item (int k) tuples), 0, r3
            | None ->
                let rec attempt n random rejected =
                    if n = stepAttempts then
                        NoStep, rejected, random
                    else
                        let args, r = arguments model (modelArguments indices c) random size

                        match taken c (index, args) with
                        | NoStep -> attempt (n + 1) r (rejected + 1)
                        | found -> found, rejected, r

                attempt 0 r2 0

/// A run drawn from a random state, of at most a length, at a size; crashes
/// are injected (one step in eight) only when asked, and only for actors.
/// Also how many of its drawn steps the reference rejected.
[<CompiledName("Generate")>]
let generateRun
    (model: Model)
    (random: Random.Seed)
    (length: int)
    (size: int)
    (crashes: bool)
    : Run * int * Random.Seed =
    let startArgs, r1 = arguments model model.StartArguments random size

    withCase model startArgs (fun context ->
        let started =
            try
                Some(initial model context)
            with _ ->
                None

        match started with
        | None -> (startArgs, []), 0, r1
        | Some state ->
            let rec go random remaining indices state steps rejected =
                if remaining = 0 then
                    List.rev steps, rejected, random
                else
                    match nextStep model context random state indices size crashes with
                    | NoneEnabled, drawn, r -> List.rev steps, rejected + drawn, r
                    | NoStep, drawn, r -> go r (remaining - 1) indices state steps (rejected + drawn)
                    | Taken((index, _) as s, next), drawn, r ->
                        let indices = shifted (command model index) indices
                        go r (remaining - 1) indices next (s :: steps) (rejected + drawn)

            let steps, rejected, last = go r1 length model.StartIndices state [] 0
            (startArgs, steps), rejected, last)

/// The message of a run whose reference rejected more of its drawn steps
/// than it accepted.
[<CompiledName("IncompleteRun")>]
let incompleteRun (model: Model) (run: string) (accepted: int) (rejected: int) =
    sprintf
        "incomplete model generation: model %s's %s drew %d steps its reference rejected and %d it accepted; fewer than half of its steps are valid, so it shows little of the system"
        model.Name
        run
        rejected
        accepted

/// How a failing run's rejected draws read after its failure.
[<CompiledName("RejectedNote")>]
let rejectedNote (rejected: int) =
    if rejected = 0 then
        ""
    else
        sprintf " (the reference rejected %d drawn steps of this run)" rejected

/// One line of JSON per model check, appended to the file
/// LAWSPEC_MODEL_REPORT names: how many runs and steps it checked, and how
/// many drawn steps the reference rejected.
[<CompiledName("ReportModel")>]
let reportModel (model: Model) (check: string) (runs: int) (steps: int) (rejected: int) (outcome: string) =
    match Environment.GetEnvironmentVariable "LAWSPEC_MODEL_REPORT" with
    | null
    | "" -> ()
    | path ->
        let line =
            sprintf
                "{\"model\":\"%s\",\"check\":\"%s\",\"runs\":%d,\"steps\":%d,\"rejected\":%d,\"outcome\":\"%s\"}\n"
                model.Name
                check
                runs
                steps
                rejected
                outcome

        try
            IO.File.AppendAllText(path, line)
        with _ ->
            // The report is a side channel; the check's own result stands.
            ()

let private hostOf (model: Model) =
    match model.Actors with
    | Some host -> host
    | None -> fail "ModelActorRuntimeMissing" model.Name

/// Run a body with the system started (an actor model's under a
/// supervisor), its context and the reference's initial state.
[<CompiledName("WithSystem")>]
let withSystem (model: Model) (startArgs: Value list) (restarts: int) (body: Context -> obj -> Value -> 'a) : 'a =
    withCase model startArgs (fun context ->
        let start () =
            callback model.StartRun context startArgs

        if not model.Actor then
            body context (box (start ())) (initial model context)
        else
            let restart state =
                match model.RestartStep with
                | None -> start ()
                | Some r -> callback r.Run context [ state ]

            (hostOf model).WithActor(restarts, start, restart, fun actor -> body context actor (initial model context)))

let private insertState (s: Step) (args: Value list) (state: Value) =
    let before, after = List.splitAt s.StatePosition args
    before @ [ state ] @ after

let private pair (out: Value) =
    match out with
    | Data(_, [ reply; next ]) -> reply, next
    | _ -> fail "ModelInvalidReply" (sprintf "%A" out)

/// Run a step on the system: its reply and the system after it.
[<CompiledName("Invoke")>]
let invoke (model: Model) (context: Context) (s: Step) (args: Value list) (system: obj) : Value * obj =
    match s.Kind with
    | Crash ->
        (hostOf model).Crash system
        Unit, system
    | Command when model.Actor ->
        let reply =
            (hostOf model)
                .Call(
                    system,
                    fun state ->
                        let out = callback s.Run context (insertState s args state)
                        if s.ReturnsUnit then Unit, out else pair out
                )

        reply, system
    | Command ->
        let state = unbox<Value> system
        let out = callback s.Run context (insertState s args state)

        match model.Shared, s.ReturnsUnit with
        | true, _ -> out, system
        | false, true ->
            match out with
            | Data(_, [ next ]) -> Unit, box next
            | _ -> fail "ModelInvalidReply" (sprintf "%A" out)
        | false, false ->
            let reply, next = pair out
            reply, box next

exception private ModelFailure of string

/// The system's state, as its abstraction and state invariants see it.
[<CompiledName("SystemState")>]
let systemState (model: Model) (system: obj) =
    if model.Actor then
        (hostOf model).State system
    else
        unbox<Value> system

/// Check the system against the reference state: its abstraction, then the
/// invariants on the model state and on the system state.
[<CompiledName("CheckState")>]
let checkState (model: Model) (context: Context) (system: obj) (expected: Value) =
    let state = systemState model system

    match model.Abstract with
    | None -> ()
    | Some a ->
        let actual = callback a context [ state ]

        if not (Scalar.equal actual expected) then
            raise (ModelFailure(sprintf "state %s, expected %s" (render actual) (render expected)))

    for kind, invariant in model.Invariants do
        let value = if kind = "model" then expected else state

        match callback invariant context [ value ] with
        | Bool true -> ()
        | Bool false -> raise (ModelFailure("invariant on the " + kind + " failed"))
        | other -> raise (ModelFailure(sprintf "invariant on the %s gave %s" kind (render other)))

/// A model without its abstraction (for the checks of each order).
[<CompiledName("WithoutAbstraction")>]
let withoutAbstraction (model: Model) = { model with Abstract = None }

let private raised (e: exn) =
    match e with
    | ModelFailure reason -> reason
    | _ -> "raised " + e.GetType().Name + ": " + e.Message

/// Run a run on the system, checking each reply and state.
[<CompiledName("Execute")>]
let execute (model: Model) ((startArgs, steps): Run) : Result<unit, Failure> =
    try
        withSystem model startArgs (List.length steps) (fun context system expected ->
            let rec go steps indices system expected n =
                try
                    checkState model context system expected

                    match steps with
                    | [] -> Ok()
                    | (i, args) :: rest ->
                        let c = command model i

                        if not (admits c indices) then
                            Error
                                {
                                    Step = n + 1
                                    Reason = "invalid step"
                                }
                        else
                            match step c context args expected with
                            | None ->
                                Error
                                    {
                                        Step = n + 1
                                        Reason = "invalid step"
                                    }
                            | Some(next, wanted) ->
                                let outcome =
                                    try
                                        Ok(invoke model context c args system)
                                    with e ->
                                        Error e

                                match outcome with
                                | Error e -> Error { Step = n + 1; Reason = raised e }
                                | Ok(result, after) ->
                                    if c.ReturnsUnit || Scalar.equal result wanted then
                                        go rest (shifted c indices) after next (n + 1)
                                    else
                                        Error
                                            {
                                                Step = n + 1
                                                Reason =
                                                    sprintf "returned %s, expected %s" (render result) (render wanted)
                                            }
                with e ->
                    Error { Step = n; Reason = raised e }

            go steps model.StartIndices system expected 0)
    with e ->
        Error { Step = 0; Reason = raised e }

let rec private halves n =
    if n = 0 then [] else n :: halves (n / 2)

let private removeRange (values: 'a list) start size =
    let before, rest = List.splitAt start values
    before @ List.skip (min size (List.length rest)) rest

let private smallerArgs (model: Model) (ds: Descriptor list) (args: Value list) =
    [
        for i, (d, arg) in List.indexed (List.zip ds args) do
            for v in shrink (modelBase d) arg model.Table -> List.updateAt i v args
    ]

/// Shrink candidates in the portable order: remove chunks of steps, then
/// shrink each step's arguments, then the start arguments.
[<CompiledName("Candidates")>]
let candidates (model: Model) ((startArgs, steps): Run) : Run list =
    let n = List.length steps

    let drops =
        [
            for size in halves (n / 2) do
                for start in 0..size .. n - 1 -> startArgs, removeRange steps start size
        ]

    let args =
        [
            for k, (i, values) in List.indexed steps do
                for smaller in smallerArgs model (command model i).Arguments values ->
                    startArgs, List.updateAt k (i, smaller) steps
        ]

    let starts =
        [
            for smaller in smallerArgs model model.StartArguments startArgs -> smaller, steps
        ]

    drops @ args @ starts

/// The smallest failing run within a budget of attempts.
[<CompiledName("Shrink")>]
let rec shrinkRun (model: Model) (run: Run) (failure: Failure) (budget: int) : Run * Failure =
    let rec next candidates budget =
        match candidates with
        | _ when budget = 0 -> run, failure
        | [] -> run, failure
        | candidate :: rest ->
            let result =
                match simulate model candidate with
                | None -> Ok()
                | Some _ -> execute model candidate

            match result with
            | Ok() -> next rest (budget - 1)
            | Error found -> shrinkRun model candidate found (budget - 1)

    next (candidates model run) budget

let private describeStep (name: string) (args: Value list) =
    name + "(" + String.Join(", ", args |> List.map render) + ")"

/// A run as text: start(...); command(...); ...
[<CompiledName("Describe")>]
let describe (model: Model) ((startArgs, steps): Run) =
    String.Join(
        "; ",
        describeStep "start" startArgs
        :: [ for i, args in steps -> describeStep (command model i).Name args ]
    )

/// The run's seed: given, LAWSPEC_SEED, or 0.
[<CompiledName("Seed")>]
let seed (given: BigInteger option) =
    match given with
    | Some n -> Random.seed n
    | None ->
        match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
        | null
        | "" -> Random.seed BigInteger.Zero
        | text -> Random.seed (BigInteger.Parse text)

/// How many cases, how long a run may be, how many shrink attempts, and
/// the seed (otherwise LAWSPEC_SEED, or 0).
type Options =
    {
        Cases: int
        MaxLength: int
        MaxShrinks: int
        Seed: BigInteger option
    }

[<CompiledName("Defaults")>]
let defaults =
    {
        Cases = 100
        MaxLength = 20
        MaxShrinks = 2000
        Seed = None
    }

let private seedValue (Random.Seed s) = s

/// Check a model's sequential runs, with options.
[<CompiledName("CheckWith")>]
let checkWith (options: Options) (model: Model) =
    require (options.Cases > 0) "ModelRequiresCases"
    require (options.MaxLength >= 0) "ModelInvalidLength"
    require (options.MaxShrinks >= 0) "ModelInvalidShrinks"
    let start = seed options.Seed

    let rec go n random steps rejected =
        if n < options.Cases then
            let length, r1 = Random.below (BigInteger(options.MaxLength + 1)) random
            let run, drawn, r2 = generateRun model r1 (int length) (1 + n % 8) true
            let accepted = List.length (snd run)
            let steps = steps + accepted
            let rejected = rejected + drawn

            if drawn > accepted then
                reportModel model "sequential" (n + 1) steps rejected "incomplete"

                Fail.raiseWith
                    "IncompleteModelGeneration"
                    (incompleteRun model (sprintf "run %d of %d" (n + 1) options.Cases) accepted drawn)

            match execute model run with
            | Ok() -> go (n + 1) r2 steps rejected
            | Error failure ->
                reportModel model "sequential" (n + 1) steps rejected "failed"
                let smaller, found = shrinkRun model run failure options.MaxShrinks

                Fail.raiseWith
                    "ModelFailed"
                    (sprintf
                        "model %s: seed %d (LAWSPEC_SEED=%d replays it), case %d: %s; at step %d: %s%s"
                        model.Name
                        (seedValue start)
                        (seedValue start)
                        n
                        (describe model smaller)
                        found.Step
                        found.Reason
                        (rejectedNote drawn))
        else
            reportModel model "sequential" options.Cases steps rejected "passed"

    go 0 start 0 0

/// Check a model's sequential runs.
[<CompiledName("Check")>]
let check (model: Model) = checkWith defaults model

/// A context factory whose handler factories are made with each body's fresh
/// symbols, as generated code builds them.
[<CompiledName("ContextWith")>]
let contextWith
    (schema: Schema.Schema)
    (factories: Scalar.SymbolContext -> (string * (Schema.Schema -> Effects.Handler)) list)
    : ContextFactory =
    let factory body =
        let symbols = Scalar.SymbolContext()

        match factories symbols with
        | [] -> body (schema, symbols)
        | made -> Effects.withScope schema made (fun scoped -> body (scoped, symbols))

    factory
