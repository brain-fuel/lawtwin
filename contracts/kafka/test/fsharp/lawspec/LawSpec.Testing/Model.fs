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

                    if not (admits c indices) then
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

/// A run drawn from a random state, of at most a length, at a size; crashes
/// are injected (one step in eight) only when asked, and only for actors.
[<CompiledName("Generate")>]
let generateRun (model: Model) (random: Random.Seed) (length: int) (size: int) (crashes: bool) : Run * Random.Seed =
    let startArgs, r1 = arguments model model.StartArguments random size

    withCase model startArgs (fun context ->
        let started =
            try
                Some(initial model context)
            with _ ->
                None

        match started with
        | None -> (startArgs, []), r1
        | Some state ->
            let rec go random remaining indices state steps =
                if remaining = 0 then
                    List.rev steps, random
                else
                    let allowed =
                        model.Commands
                        |> Array.indexed
                        |> Array.filter (fun (_, c) -> admits c indices)
                        |> Array.map fst

                    if allowed.Length = 0 then
                        List.rev steps, random
                    else
                        let choice, r1 = Random.below (BigInteger allowed.Length) random

                        let index, r2 =
                            if crashes && model.Actor then
                                let crash, r = Random.below (BigInteger 8) r1

                                (if crash.IsZero then
                                     model.Commands.Length
                                 else
                                     allowed.[int choice]),
                                r
                            else
                                allowed.[int choice], r1

                        let c = command model index
                        let args, r3 = arguments model c.Arguments r2 size

                        match step c context args state with
                        | None -> go r3 (remaining - 1) indices state steps
                        | Some(next, _) -> go r3 (remaining - 1) (shifted c indices) next ((index, args) :: steps)

            let steps, last = go r1 length model.StartIndices state []
            (startArgs, steps), last)

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
            for v in shrink d arg model.Table -> List.updateAt i v args
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

    let rec go n random =
        if n < options.Cases then
            let length, r1 = Random.below (BigInteger(options.MaxLength + 1)) random
            let run, r2 = generateRun model r1 (int length) (1 + n % 8) true

            match execute model run with
            | Ok() -> go (n + 1) r2
            | Error failure ->
                let smaller, found = shrinkRun model run failure options.MaxShrinks

                Fail.raiseWith
                    "ModelFailed"
                    (sprintf
                        "model %s: seed %d (LAWSPEC_SEED=%d replays it), case %d: %s; at step %d: %s"
                        model.Name
                        (seedValue start)
                        (seedValue start)
                        n
                        (describe model smaller)
                        found.Step
                        found.Reason)

    go 0 start

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
