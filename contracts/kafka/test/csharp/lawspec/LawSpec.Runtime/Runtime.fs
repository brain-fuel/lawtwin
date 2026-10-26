/// Services the generated definitions and laws share: contracts, equations,
/// helpers, recordings and concurrent groups. Values stay portable until a
/// native boundary. ref:DEC-typed-core-boundary
module LawSpec.Runtime.Runtime

open System
open System.IO
open System.Threading.Tasks

/// A portable boolean as a .NET one.
[<CompiledName("Truth")>]
let truth (value: Value) =
    match value with
    | Bool b -> b
    | _ -> Fail.raiseWith "BoolRequired" (sprintf "%A" value)

[<CompiledName("Not")>]
let not (value: Value) = Bool(Operators.not (truth value))

/// The item of a list of values at a position.
[<CompiledName("Item")>]
let item (index: int) (values: Value list) = List.item index values

/// The item of a list at a position.
[<CompiledName("Nth")>]
let nth (index: int) (items: 'a list) = List.item index items

[<CompiledName("Some")>]
let some (value: 'a) = Some value

/// A data value's field at a position (a list's head and tail).
[<CompiledName("Field")>]
let field (value: Value) (index: int) =
    match value, index with
    | Data(_, fields), _ -> List.item index fields
    | List(head :: _), 0 -> head
    | List(_ :: tail), 1 -> List tail
    | _ -> Fail.raiseWith "ExpectedData" (sprintf "%A" value)

/// Whether a value was built by a constructor.
[<CompiledName("HasTag")>]
let hasTag (tag: string) (value: Value) =
    match value with
    | Data(t, _) -> t = tag
    | List [] -> tag = "List::Nil"
    | List _ -> tag = "List::Cons"
    | _ -> false

/// A match no branch covers.
[<CompiledName("NoMatch")>]
let noMatch (value: Value) : Value =
    Fail.raiseWith "MatchFailure" (sprintf "%A" value)

/// Whether every element of a list satisfies a predicate.
[<CompiledName("AllElements")>]
let allElements (value: Value) (predicate: Value -> Value) =
    match value with
    | List items -> Bool(items |> List.forall (predicate >> truth))
    | _ -> Fail.raiseWith "ListRequired" ""

/// Fold a finite search once, preserving incomplete inner outcomes.
[<CompiledName("SearchElements")>]
let searchElements (universal: bool) (value: Value) (predicate: Value -> Value) =
    let decisive = if universal then -1I else 1I

    let rec search result items =
        match items with
        | [] -> Int result
        | item :: rest ->
            match predicate item with
            | Int code when code = decisive -> Int code
            | Int code when code = 0I -> search 0I rest
            | Int code when code = -decisive -> search result rest
            | _ -> Fail.raiseWith "InvalidFiniteSearchOutcome" ""

    match value with
    | List items -> search -decisive items
    | _ -> Fail.raiseWith "ListRequired" ""

/// Whether every payload of a container satisfies its parameter's predicate.
[<CompiledName("AllPayloads")>]
let allPayloads (value: Value) (t: TypeRef) (predicates: (Value -> Value) list) (schema: Schema.Schema) =
    Bool(Schema.allPayloads value t (predicates |> List.map (fun p -> p >> truth)) schema)

/// Nullable or Optional, absent.
[<CompiledName("Absent")>]
let absent (kind: string) = Presence(kind, None)

/// Nullable or Optional, present.
[<CompiledName("Present")>]
let present (kind: string) (value: Value) = Presence(kind, Some value)

/// Fail a contract unless it holds.
[<CompiledName("Require")>]
let require (holds: Value) (context: string) : unit =
    match holds with
    | Bool true -> ()
    | Bool false -> Fail.raiseWith "ContractFailed" context
    | _ -> Fail.raiseWith "NonBooleanContract" context

/// Where two unequal values first differ: the path to the item or field,
/// and both values there.
let private difference (actual: Value) (expected: Value) =
    let rec walk (a: Value) (b: Value) (path: string list) =
        match a, b with
        | List xs, List ys ->
            let firstDiff =
                Seq.zip xs ys
                |> Seq.tryFindIndex (fun (x, y) -> Recorded.renderPlain x <> Recorded.renderPlain y)

            match firstDiff with
            | Some i -> walk (List.item i xs) (List.item i ys) (path @ [ sprintf "item %d" (i + 1) ])
            | None when List.length xs = List.length ys -> None
            | None -> Some(path @ [ "length" ], string (List.length xs), string (List.length ys))
        | Data(tx, fx), Data(ty, fy) when tx = ty && List.length fx = List.length fy ->
            let name = tx.Substring(tx.LastIndexOf("::") + 2)

            match
                List.zip fx fy
                |> List.tryFindIndex (fun (x, y) -> Recorded.renderPlain x <> Recorded.renderPlain y)
            with
            | Some i -> walk (List.item i fx) (List.item i fy) (path @ [ sprintf "field %d of %s" (i + 1) name ])
            | None -> None
        | _ when List.isEmpty path -> None
        | _ -> Some(path, Recorded.renderPlain a, Recorded.renderPlain b)

    match walk actual expected [] with
    | Some(path, shownActual, shownExpected) ->
        sprintf " | first difference at %s: expected %s, actual %s" (String.Join(", ", path)) shownExpected shownActual
    | None -> ""

/// An equation of a law: both sides are kept for the report.
[<CompiledName("AssertEqual")>]
let assertEqual (actual: Value) (expected: Value) (label: string) : Value =
    if Scalar.equal actual expected then
        Bool true
    else
        Fail.raiseWith
            "EquationFailed"
            (sprintf
                "%s: actual %s, expected %s%s"
                label
                (Recorded.renderPlain actual)
                (Recorded.renderPlain expected)
                (difference actual expected))

/// Name the definition or law a LawSpec failure happened in.
[<CompiledName("Contextual")>]
let contextual (identity: string) (body: unit -> 'a) : 'a =
    try
        body ()
    with :? LawSpecException as e ->
        raise (e.Within identity)

let private recordedRoot () =
    match Environment.GetEnvironmentVariable "LAWSPEC_RECORDED" with
    | null
    | "" ->
        let start = Directory.GetCurrentDirectory()

        let rec find (folder: string) =
            if
                File.Exists(Path.Combine(folder, "lawspec.edn"))
                || Directory.Exists(Path.Combine(folder, "recorded"))
            then
                Path.Combine(folder, "recorded")
            else
                match Directory.GetParent folder with
                | null -> Path.Combine(start, "recorded")
                | parent -> find parent.FullName

        find start
    | given -> given

/// Compare a value's text with its recording, or record it under
/// LAWSPEC_UPDATE_RECORDED=1 (lawspec test --update-recorded).
[<CompiledName("RecordedText")>]
let recordedText (key: string) (text: string) : Value =
    let path = Path.Combine(Array.append [| recordedRoot () |] (key.Split '/'))

    if Environment.GetEnvironmentVariable "LAWSPEC_UPDATE_RECORDED" = "1" then
        Directory.CreateDirectory(Path.GetDirectoryName path) |> ignore
        File.WriteAllText(path, text + "\n")
        Bool true
    elif File.Exists path then
        let stored = File.ReadAllText path

        let stored =
            if stored.EndsWith "\n" then
                stored.Substring(0, stored.Length - 1)
            else
                stored

        if stored = text then
            Bool true
        else
            Fail.raiseWith
                "RecordingDiffers"
                ("recorded/"
                 + key
                 + " differs: expected "
                 + stored
                 + ", actual "
                 + text
                 + " (lawspec test --update-recorded records the new value)")
    else
        Fail.raiseWith
            "RecordingMissing"
            ("no recording recorded/"
             + key
             + "; run lawspec test --update-recorded to record "
             + text)

/// A recorded value, at its type.
[<CompiledName("Recorded")>]
let recorded (key: Value) (value: Value) (t: TypeRef) (schema: Schema.Schema) : Value =
    match key with
    | Text k -> recordedText k (Recorded.text value t (Some schema))
    | _ -> Fail.raiseWith "TextRequired" "recorded"

/// A built-in helper by name.
[<CompiledName("Helper")>]
let helper (name: string) (values: Value list) (types: string list) (bits: int) : Value =
    match name, values with
    | "incompleteSearch", [ Text reason ] -> failwith reason
    | "unreachable", _ -> Fail.raiseWith "Unreachable" ""
    | "regexMatches", [ Text pattern; Text text ] -> Bool(Regex.matches pattern text)
    | "recorded", [ Text key; value ] when List.length types < 2 -> recordedText key (Recorded.renderPlain value)
    | "recorded", [ key; value ] -> recorded key value (Scalar.typeOf (List.item 1 types)) (Schema.create [] [] bits)
    | _ -> Scalar.helper name values types bits

let private unwrap (e: exn) =
    match e with
    | :? AggregateException as a when a.InnerExceptions.Count = 1 -> a.InnerExceptions.[0]
    | _ -> e

/// Run steps concurrently, each on the thread pool, and return their values
/// in source order. Every step finishes before the first failure in source
/// order is raised.
[<CompiledName("Concurrently")>]
let concurrently (steps: (unit -> Value) list) : Value list =
    let tasks =
        steps |> List.map (fun step -> Task.Run(fun () -> step ())) |> Array.ofList

    try
        Task.WaitAll(tasks |> Array.map (fun t -> t :> Task))
    with _ ->
        ()

    tasks
    |> Array.map (fun t ->
        if t.IsFaulted then
            raise (unwrap t.Exception)
        elif t.IsCanceled then
            Fail.raiseWith "ConcurrentStepCancelled" ""
        else
            t.Result)
    |> List.ofArray

/// Await an async adapter's task where it is called.
[<CompiledName("Await")>]
let await (task: Task<'a>) : 'a =
    try
        task.GetAwaiter().GetResult()
    with e ->
        raise (unwrap e)

/// Await a task with no result (a C# adapter's Task).
[<CompiledName("AwaitDone")>]
let awaitDone (task: Task) : unit =
    try
        task.GetAwaiter().GetResult()
    with e ->
        raise (unwrap e)

/// Discard a value computed for its checks.
[<CompiledName("Ignore")>]
let ignore (_: Value) = ()

/// An integer value from its digits.
[<CompiledName("Integer")>]
let integer (digits: string) = Int(Numerics.BigInteger.Parse digits)
