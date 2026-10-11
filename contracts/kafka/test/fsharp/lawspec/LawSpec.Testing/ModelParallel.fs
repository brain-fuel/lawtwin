/// Concurrent model histories, generated from a valid sequential prefix.
/// Every interleaving of the branches must be allowed by the reference.
/// The calls run on the thread pool at once; a shared counter records each
/// call's interval, and an order consistent with the model (and, for
/// linearizability, with real time) must explain the results.
/// ref:DEC-stateful-models-linearizability ref:herlihy-wing-linearizability
/// ref:wing-gong-linearizability ref:DEC-portable-seeded-generation
module LawSpec.Testing.ModelParallel

open System
open System.Numerics
open System.Threading
open System.Threading.Tasks
open LawSpec.Runtime
open LawSpec.Runtime.Descriptors
open LawSpec.Testing.Model

/// A case: a sequential prefix, then a branch of steps for each thread.
type Case = Run * (int * Value list) list list

/// A call's interval on the shared clock and its outcome.
type Event = int64 * int64 * Result<Value, string>

let private fail code detail : 'a = Fail.raiseWith code detail

let private states (model: Model.Model) (prefix: Run) = simulate model prefix

let private pending (branches: (int * Value list)[][]) (positions: int[]) =
    [ for i in 0 .. branches.Length - 1 do
          let k = positions.[i]
          if k < branches.[i].Length then yield i, branches.[i].[k] ]

let private advance (positions: int[]) i =
    let next = Array.copy positions
    next.[i] <- next.[i] + 1
    next

/// Whether every interleaving of the branches is allowed by the reference.
[<CompiledName("Allowed")>]
let allowed (model: Model.Model) (((startArgs, _) as prefix, branches): Case) =
    match states model prefix with
    | None -> false
    | Some ss ->
        withCase model startArgs (fun context ->
            let bs = branches |> List.map Array.ofList |> Array.ofList
            let seen = System.Collections.Generic.HashSet<int list * Value>(HashIdentity.Structural)
            let rec allOrders (positions: int[]) (state: Value) =
                if not (seen.Add((List.ofArray positions, state))) then true
                else
                    pending bs positions
                    |> List.forall (fun (i, (index, args)) ->
                        match step (command model index) context args state with
                        | None -> false
                        | Some(next, _) -> allOrders (advance positions i) next)
            allOrders (Array.zeroCreate bs.Length) (List.last ss))

let private generateBranch (model: Model.Model) (context: Context) random size count state =
    let rec go random remaining state steps =
        if remaining = 0 || model.Commands.Length = 0 then List.rev steps, random
        else
            let index, r1 = Random.below (BigInteger model.Commands.Length) random
            let c = command model (int index)
            let mutable r = r1
            let args =
                c.Arguments
                |> List.map (fun d ->
                    let v, next = generate d model.Table r size
                    r <- next
                    v)
            match step c context args state with
            | None -> go r (remaining - 1) state steps
            | Some(next, _) -> go r (remaining - 1) next ((int index, args) :: steps)
    go random count state []

let rec private trim (model: Model.Model) ((prefix, branches): Case) : Case =
    if allowed model (prefix, branches) then prefix, branches
    else
        let length = branches |> List.map List.length |> List.max
        let i = branches |> List.findIndex (fun b -> List.length b = length)
        trim model (prefix, List.updateAt i (List.take (length - 1) branches.[i]) branches)

/// A case drawn from a random state: a prefix of up to three steps, then a
/// branch for each thread of one to the branch length steps.
[<CompiledName("Generate")>]
let generateCase (model: Model.Model) (random: Random.Seed) (size: int) (threads: int) (branchLength: int) : Case * Random.Seed =
    let length, r1 = Random.below (BigInteger 4) random
    let ((startArgs, _) as prefix), r2 = generateRun model r1 (int length) size false
    match states model prefix with
    | None -> (prefix, List.replicate threads []), r2
    | Some ss ->
        let branches, r3 =
            withCase model startArgs (fun context ->
                let mutable r = r2
                let branches =
                    [ for _ in 1 .. threads ->
                          let n, next = Random.below (BigInteger branchLength) r
                          let branch, after = generateBranch model context next size (int n + 1) (List.last ss)
                          r <- after
                          branch ]
                branches, r)
        trim model (prefix, branches), r3

let private perturb (random: Random.Seed) =
    let choice, next = Random.below (BigInteger 4) random
    if choice.IsZero then ()
    elif choice.IsOne then Thread.Yield() |> ignore
    else Thread.Sleep 1
    next

let private raisedText (e: exn) = "raised " + e.GetType().Name + ": " + e.Message

let private prefixRun (model: Model.Model) (context: Context) (steps: (int * Value list) list) (system: obj) (expected: Value) =
    let rec go steps expected =
        checkState model context system expected
        match steps with
        | [] -> expected
        | (i, args) :: rest ->
            let c = command model i
            match step c context args expected with
            | None -> fail "ModelPrefixInvalid" c.Name
            | Some(next, wanted) ->
                let result, _ = invoke model context c args system
                if c.ReturnsUnit || Scalar.equal result wanted then go rest next
                else fail "ModelPrefixReturned" (sprintf "%s returned %s, expected %s" c.Name (render result) (render wanted))
    go steps expected

/// Search the orders consistent with each branch's order (and, for
/// linearizability, real time) for one the reference explains.
let rec private findOrder (model: Model.Model) (context: Context) (branches: (int * Value list) list list) (history: Event list list) (expected: Value) (finish: Value -> bool) =
    let bs = branches |> List.map Array.ofList |> Array.ofList
    let hs = history |> List.map Array.ofList |> Array.ofList
    let linear = model.Consistency = "linearizable"
    let seen = System.Collections.Generic.HashSet<int list * Value>(HashIdentity.Structural)
    let rec visit (positions: int[]) (state: Value) =
        if not (seen.Add((List.ofArray positions, state))) then false
        else
            match pending bs positions with
            | [] -> finish state
            | next ->
                next
                |> List.exists (fun (i, (index, args)) ->
                    let called, _, result = hs.[i].[positions.[i]]
                    let blocked =
                        linear && (next |> List.exists (fun (j, _) ->
                            let _, returned, _ = hs.[j].[positions.[j]]
                            j <> i && returned < called))
                    if blocked then false
                    else
                        let c = command model index
                        match step c context args state with
                        | None -> false
                        | Some(next, wanted) ->
                            let ignore = model.Consistency = "eventual" || c.ReturnsUnit
                            let value = match result with Ok v -> v | Error _ -> Unit
                            if ignore || Scalar.equal value wanted then visit (advance positions i) next else false)
    visit (Array.zeroCreate bs.Length) expected

let private causal (model: Model.Model) (context: Context) branches (history: Event list list) expected system =
    List.zip branches history
    |> List.forall (fun (branch, events) ->
        try
            let last =
                List.zip branch events
                |> List.fold (fun state ((i, args), (_, _, result)) ->
                    let c = command model i
                    match step c context args state with
                    | Some(next, wanted) ->
                        let value = match result with Ok v -> v | Error _ -> Unit
                        if c.ReturnsUnit || Scalar.equal value wanted then next else failwith "inconsistent"
                    | None -> failwith "inconsistent") expected
            checkState (withoutAbstraction model) context system last
            true
        with _ -> false)

let rec private foldKeys (model: Model.Model) context (groups: ((int * Value list) * Event) list list list) state (finish: Value -> bool) =
    match groups with
    | [] -> finish state
    | parts :: rest ->
        let branches = parts |> List.map (List.map fst)
        let history = parts |> List.map (List.map snd)
        findOrder model context branches history state (fun next -> foldKeys model context rest next finish)

let private byKey (model: Model.Model) context (branches: (int * Value list) list list) (history: Event list list) expected finish =
    let n = List.length branches
    let groups =
        List.zip branches history
        |> List.indexed
        |> List.fold (fun (acc: Map<string, ((int * Value list) * Event) list list>) (i, (branch, events)) ->
            List.zip branch events
            |> List.fold (fun (groups: Map<string, ((int * Value list) * Event) list list>) (((index, args) as s), e) ->
                let keyIndex = (command model index).Key.Value
                let key = render (List.item keyIndex args)
                let parts = defaultArg (groups.TryFind key) (List.replicate n [])
                groups.Add(key, List.updateAt i (parts.[i] @ [ s, e ]) parts)) acc) Map.empty
    let ordered =
        groups |> Map.toList |> List.sortWith (fun (a, _) (b, _) -> Scalar.compareText a b) |> List.map snd
    foldKeys model context ordered expected finish

/// Whether a parallel history has an order consistent with the model.
[<CompiledName("Consistent")>]
let consistent (model: Model.Model) (context: Context) (branches: (int * Value list) list list) (history: Event list list) (expected: Value) (final: Value option) (system: obj) =
    let finish (state: Value) =
        try
            let equal = match final with None -> true | Some actual -> Scalar.equal actual state
            equal && (checkState (withoutAbstraction model) context system state; true)
        with _ -> false
    match model.Consistency with
    | "causal" -> causal model context branches history expected system
    | consistency ->
        let canPartition =
            consistency = "linearizable" && model.PerKey
            && (branches |> List.concat |> List.forall (fun (i, _) -> (command model i).Key.IsSome))
        if canPartition then byKey model context branches history expected finish
        else findOrder model context branches history expected finish

/// Run a case once, its branches at the same time, with scheduling
/// perturbed from a shake value; Ok when an order explains the results.
[<CompiledName("Execute")>]
let execute (model: Model.Model) (((startArgs, steps), branches): Case) (shake: uint64) : Result<unit, string> =
    try
        withSystem model startArgs (List.length steps) (fun context system expected ->
            let finalPrefix = prefixRun model context steps system expected
            let clock = ref 0L
            let gate = obj ()
            let tick () = lock gate (fun () -> clock.Value <- clock.Value + 1L; clock.Value)
            let runBranch (i: int) (branch: (int * Value list) list) =
                let mutable random =
                    Random.seed (BigInteger shake ^^^ (BigInteger(i + 1) * BigInteger 0x9e3779b97f4a7c15UL))
                [ for index, args in branch ->
                      let c = command model index
                      random <- perturb random
                      let called = tick ()
                      let result =
                          try
                              let value, _ = invoke model context c args system
                              Ok value
                          with e -> Error(c.Name + ": " + raisedText e)
                      let returned = tick ()
                      random <- perturb random
                      (called, returned, result) ]
            let tasks = branches |> List.mapi (fun i b -> Task.Run(fun () -> runBranch i b)) |> Array.ofList
            (try Task.WaitAll(tasks |> Array.map (fun t -> t :> Task)) with _ -> ())
            let history =
                tasks
                |> Array.map (fun t ->
                    if t.IsFaulted then [ 0L, 0L, Error(raisedText t.Exception.InnerException) ] else t.Result)
                |> List.ofArray
            let errors = [ for events in history do for (_, _, r) in events do match r with Error e -> yield e | Ok _ -> () ]
            match errors with
            | e :: _ -> Error e
            | [] ->
                let final = model.Abstract |> Option.map (fun a -> callback a context [ systemState model system ])
                if consistent model context branches history finalPrefix final system then Ok()
                else
                    let shown =
                        history
                        |> List.mapi (fun i events ->
                            string (char (int 'A' + i)) + ": "
                            + String.Join(", ", [ for (a, b, r) in events -> sprintf "[%d,%d] %s" a b (match r with Ok v -> render v | Error e -> e) ]))
                    Error("no consistent order: " + String.Join("; ", shown)))
    with e -> Error(raisedText e)

/// Shrink candidates: remove a prefix step, remove a branch step, then
/// shrink a branch step's arguments.
[<CompiledName("Candidates")>]
let candidates (model: Model.Model) ((((startArgs, steps) as prefix), branches): Case) : Case list =
    let prefixes = [ for i in 0 .. List.length steps - 1 -> (startArgs, List.removeAt i steps), branches ]
    let drops =
        [ for i, branch in List.indexed branches do
              for k in 0 .. List.length branch - 1 -> prefix, List.updateAt i (List.removeAt k branch) branches ]
    let smaller =
        [ for i, branch in List.indexed branches do
              for k, (index, values) in List.indexed branch do
                  for a, (d, v) in List.indexed (List.zip (command model index).Arguments values) do
                      for value in shrink d v model.Table ->
                          prefix, List.updateAt i (List.updateAt k (index, List.updateAt a value values) branch) branches ]
    prefixes @ drops @ smaller

let rec private repeats model case count (shake: uint64) =
    if count = 0 then Ok()
    else
        match execute model case shake with
        | Ok() -> repeats model case (count - 1) (shake + 1UL)
        | error -> error

/// The smallest case still failing within a budget of attempts.
[<CompiledName("Shrink")>]
let rec shrinkCase (model: Model.Model) (case: Case) (failure: string) (count: int) (budget: int) (shake: uint64) : Case * string =
    let rec next candidates budget =
        match candidates with
        | _ when budget = 0 -> case, failure
        | [] -> case, failure
        | candidate :: rest ->
            let found = if not (allowed model candidate) then Ok() else repeats model candidate count shake
            match found with
            | Ok() -> next rest (budget - 1)
            | Error reason -> shrinkCase model candidate reason count (budget - 1) shake
    next (candidates model case) budget

/// A case as text: its prefix, then each branch, at the same time.
[<CompiledName("Describe")>]
let describeCase (model: Model.Model) ((prefix, branches): Case) =
    let parts =
        branches
        |> List.mapi (fun i b ->
            let shown =
                match b with
                | [] -> "nothing"
                | _ -> (describe model ([], b)).Substring("start(); ".Length)
            string (char (int 'A' + i)) + ": " + shown)
    let joined =
        match parts with
        | [] -> "nothing"
        | [ only ] -> only
        | _ -> String.Join(", ", List.take (parts.Length - 1) parts) + " and " + List.last parts
    describe model prefix + ", then " + joined + " at the same time"

/// How many cases, how many runs of each, the shrink budget, the threads
/// and the longest branch.
type Options =
    { Cases: int
      Repeats: int
      MaxShrinks: int
      Threads: int
      BranchLength: int
      Seed: BigInteger option }

[<CompiledName("Defaults")>]
let defaults = { Cases = 50; Repeats = 10; MaxShrinks = 300; Threads = 3; BranchLength = 5; Seed = None }

let private seedValue (Random.Seed s) = s

/// Check a shared model's concurrent histories, with options.
[<CompiledName("CheckWith")>]
let checkWith (options: Options) (model: Model.Model) =
    if not model.Shared then fail "ModelNotShared" model.Name
    if options.Cases <= 0 || options.Repeats <= 0 || options.Threads <= 0 || options.BranchLength <= 0 || options.MaxShrinks < 0 then
        fail "ModelInvalidOptions" model.Name
    let start = Model.seed options.Seed
    let rec go n random =
        if n < options.Cases then
            let case, r1 = generateCase model random (1 + n % 8) options.Threads options.BranchLength
            let shake, r2 = Random.next r1
            match repeats model case options.Repeats shake with
            | Ok() -> go (n + 1) r2
            | Error failure ->
                let smaller, found = shrinkCase model case failure (max 2 (options.Repeats / 2)) options.MaxShrinks shake
                Fail.raiseWith "ModelInconsistent"
                    (sprintf "model %s is not %s: seed %d (LAWSPEC_SEED=%d replays it), case %d, shake %d: %s; %s"
                        model.Name model.Consistency (seedValue start) (seedValue start) n shake (describeCase model smaller) found)
    go 0 (Random.Seed(seedValue start ^^^ 0x5bd1e995UL))

/// Check a shared model's concurrent histories.
[<CompiledName("Check")>]
let check (model: Model.Model) = checkWith defaults model
