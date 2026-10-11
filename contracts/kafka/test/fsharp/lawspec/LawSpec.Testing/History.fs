/// Scenario histories carry vector clocks through sends, receives and
/// fork/join. Consistency depends on these message edges as well as each
/// caller's own order; real time is required only for linearizability.
/// ref:DEC-sessions-by-construction ref:DEC-stateful-models-linearizability
/// ref:wing-gong-linearizability
module LawSpec.Testing.History

open System.Numerics
open LawSpec.Runtime

/// A completed call: the model command it ran, its arguments and result, its
/// interval on the shared clock, its process and its vector clocks.
type Event =
    { Command: int
      Arguments: Value list
      Result: Value
      Called: int64
      Returned: int64
      Process: string
      CallClock: Map<string, int64>
      ReturnClock: Map<string, int64> }

/// Whether a's return happened before b's call, by their vector clocks.
[<CompiledName("HappenedBefore")>]
let happenedBefore (a: Event) (b: Event) =
    a.ReturnClock |> Map.forall (fun proc count -> defaultArg (b.CallClock.TryFind proc) 0L >= count)

let private before mode (a: Event) (b: Event) =
    if mode = "linearizable" then a.Returned < b.Called else happenedBefore a b

let private mask (indices: int seq) = indices |> Seq.fold (fun bits i -> bits ||| (BigInteger.One <<< i)) BigInteger.Zero

let private bit i = BigInteger.One <<< i

let private search
    (model: Model.Model)
    (context: Model.Context)
    (history: Event[])
    (preds: BigInteger[])
    (members: int list)
    (checkedBits: BigInteger)
    (expected: Value)
    (judge: Value -> bool)
    =
    let full = mask members
    let seen = System.Collections.Generic.HashSet<BigInteger * Value>(HashIdentity.Structural)
    let rec visit (dbits: BigInteger) (state: Value) =
        if dbits = full then judge state
        elif not (seen.Add((dbits, state))) then false
        else
            members
            |> List.exists (fun i ->
                let b = bit i
                let dependencies = preds.[i] &&& full
                let available = (dbits &&& b).IsZero && (dependencies &&& dbits) = dependencies
                if not available then false
                else
                    let e = history.[i]
                    let c = Model.command model e.Command
                    match Model.step c context e.Arguments state with
                    | None -> false
                    | Some(next, wanted) ->
                        let ignore = (checkedBits &&& b).IsZero || c.ReturnsUnit
                        if ignore || Scalar.equal e.Result wanted then visit (dbits ||| b) next else false)
    visit BigInteger.Zero expected

/// Whether a history has an order consistent with the model: real time for
/// linearizability, message order otherwise; causal histories are checked
/// in each process's view. Final is the system's abstract final state, if any.
[<CompiledName("Consistent")>]
let consistent (model: Model.Model) (context: Model.Context) (events: Event list) (expected: Value) (final: Value option) (system: obj) =
    let history = Array.ofList events
    let indices = [ 0 .. history.Length - 1 ]
    let mode = model.Consistency
    let preds =
        [| for i in indices -> mask [ for j in indices do if j <> i && before mode history.[j] history.[i] then yield j ] |]
    let judge compareFinal (state: Value) =
        try
            let equal =
                match compareFinal, final with
                | true, Some actual -> Scalar.equal actual state
                | _ -> true
            equal
            && (Model.checkState (Model.withoutAbstraction model) context system state
                true)
        with _ -> false
    if mode = "causal" then
        let processes = events |> List.map (fun e -> e.Process) |> List.distinct |> List.sort
        match processes with
        | [] -> judge false expected
        | _ ->
            processes
            |> List.forall (fun proc ->
                let own = indices |> List.filter (fun i -> history.[i].Process = proc)
                let visible =
                    own @ [ for j in indices do
                                for i in own do
                                    if j <> i && happenedBefore history.[j] history.[i] then yield j ]
                    |> List.distinct |> List.sort
                search model context history preds visible (mask own) expected (judge false))
    else
        let checkedBits = if mode = "eventual" then BigInteger.Zero else mask indices
        search model context history preds indices checkedBits expected (judge true)
