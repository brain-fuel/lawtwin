/// LawSpec's descriptor search, apart from generation and shrinking: the
/// portable seeded climb of a `target` (ten starting points, sixty rounds, at
/// most twenty-four moves a round and four hundred checks), and the failure
/// database that keeps a failing law's inputs (LAWSPEC_FAILURES) and replays
/// them first on the next run. ref:REQ-harness-units
/// ref:DEC-portable-seeded-generation ref:DEC-shrink-within-domain
module LawSpec.Testing.Search

open System
open System.IO
open System.Numerics
open System.Security.Cryptography
open System.Text
open System.Text.Json
open System.Text.Json.Nodes
open LawSpec.Runtime

let private inputFile (law: string) =
    match Environment.GetEnvironmentVariable "LAWSPEC_FAILURES" with
    | null
    | "" -> None
    | directory ->
        let name =
            Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes law)).ToLowerInvariant()

        Some(Path.Combine(directory, "inputs", name + ".json"))

/// Keep a failing case's inputs, encoded on the canonical wire.
[<CompiledName("Remember")>]
let remember (law: string) (descriptors: string list) (values: Value list) =
    match inputFile law with
    | None -> ()
    | Some file ->
        let inputs = JsonArray()

        for descriptor, value in List.zip descriptors values do
            let table, d = Descriptors.fromText descriptor
            inputs.Add(JsonValue.Create(Convert.ToHexString(Descriptors.encode d value table).ToLowerInvariant()))

        let record = JsonObject()
        record.["law"] <- JsonValue.Create(law)
        record.["inputs"] <- inputs
        Directory.CreateDirectory(Path.GetDirectoryName file) |> ignore

        let temporary =
            file
            + "."
            + string Environment.ProcessId
            + "."
            + Guid.NewGuid().ToString("N")
            + ".tmp"

        try
            File.WriteAllText(temporary, record.ToJsonString())
            File.Move(temporary, file, true)
        finally
            if File.Exists temporary then
                File.Delete temporary

let private keep law descriptors values =
    try
        remember law descriptors values
    with e ->
        eprintfn "LawSpec could not save inputs for %s: %s" law e.Message

/// Run a case, keeping its inputs when it fails.
[<CompiledName("Guard")>]
let guard (law: string) (descriptors: string list) (values: Value list) (check: unit -> Value) : Value =
    try
        match check () with
        | Bool false ->
            keep law descriptors values
            Bool false
        | result -> result
    with _ ->
        keep law descriptors values
        reraise ()

/// Replay the kept failing inputs of a law, if any, before its generated
/// cases; a repaired or obsolete one is removed.
[<CompiledName("Replay")>]
let replay (law: string) (descriptors: string list) (check: Value list -> Value option) =
    match inputFile law with
    | None -> ()
    | Some file when not (File.Exists file) -> ()
    | Some file ->
        let values =
            try
                let record = JsonNode.Parse(File.ReadAllText file)

                if record.["law"].GetValue<string>() <> law then
                    None
                else
                    let inputs =
                        record.["inputs"].AsArray()
                        |> Seq.map (fun n -> n.GetValue<string>())
                        |> List.ofSeq

                    Some(
                        List.map2
                            (fun descriptor (hex: string) ->
                                let table, d = Descriptors.fromText descriptor
                                Descriptors.decode d (Convert.FromHexString hex) table)
                            descriptors
                            inputs
                    )
            with _ ->
                None

        match values with
        | None -> eprintfn "%s: discarding incompatible saved inputs" law
        | Some values ->
            eprintfn "%s: replaying the failing inputs kept in .lawspec/failures" law

            match check values with
            | Some(Bool false) -> Fail.raiseWith "ReplayFailed" law
            | _ -> ()

        File.Delete file

// The kept-values store (docs/reference/cli.md, "Kept values"). A run that
// draws a case holding a value of a restricted type records the case's inputs
// in the wire encoding under .lawspec/found, in a file named by a hash of the
// definitions its inputs reach, so a changed restriction makes the old values
// unreachable. A later run replays them as extra cases of the law after
// checking each against the current definitions; one that no longer meets
// them is dropped. A spec is "KEY CAP TAG...": the cap is how many values a
// key keeps (0 keeps none), the tags are the constructors of the restricted
// types.
type private KeptEntry = { Inputs: string list; Used: int }

let private keptRecorded =
    System.Collections.Concurrent.ConcurrentDictionary<string, bool>()

let private keptWords (spec: string) = spec.Split ' '

let private keptCap (spec: string) =
    let words = keptWords spec
    if words.Length > 1 then int words.[1] else 0

let private keptFile (spec: string) =
    let directory =
        match Environment.GetEnvironmentVariable "LAWSPEC_FOUND" with
        | null
        | "" ->
            match Environment.GetEnvironmentVariable "LAWSPEC_FAILURES" with
            | null
            | "" -> None
            // The failure database is .lawspec/failures/<target>/<project>.
            | failures -> Some(Path.GetFullPath(Path.Combine(failures, "..", "..", "..", "found")))
        | found -> Some found

    directory
    |> Option.map (fun d -> Path.Combine(d, (keptWords spec).[0] + ".json"))

let private keptRead (file: string) : KeptEntry list =
    try
        let record = JsonNode.Parse(File.ReadAllText file)

        record.["values"].AsArray()
        |> Seq.map (fun n ->
            {
                Inputs = n.["inputs"].AsArray() |> Seq.map (fun i -> i.GetValue<string>()) |> List.ofSeq
                Used = n.["used"].GetValue<int>()
            })
        |> List.ofSeq
    with _ ->
        []

// Merge into what is on disk now, since another target's run may have
// written since this one read: write a temporary file, then rename it.
let private keptUpdate (file: string) (spec: string) (change: KeptEntry list -> KeptEntry list) =
    let entries = change (keptRead file)
    let cap = keptCap spec

    let kept =
        if entries.Length > cap then
            List.skip (entries.Length - cap) entries
        else
            entries

    let directory = Path.GetDirectoryName file

    if not (Directory.Exists directory) then
        Directory.CreateDirectory directory |> ignore
        File.WriteAllText(Path.Combine(directory, ".gitignore"), "*\n")

    let values = JsonArray()

    for entry in kept do
        let inputs = JsonArray()

        for input in entry.Inputs do
            inputs.Add(JsonValue.Create input)

        let node = JsonObject()
        node.["inputs"] <- inputs
        node.["used"] <- JsonValue.Create entry.Used
        values.Add node

    let record = JsonObject()
    record.["key"] <- JsonValue.Create((keptWords spec).[0])
    record.["values"] <- values

    let temporary =
        file
        + "."
        + string Environment.ProcessId
        + "."
        + Guid.NewGuid().ToString("N")
        + ".tmp"

    try
        File.WriteAllText(temporary, record.ToJsonString() + "\n")
        File.Move(temporary, file, true)
    finally
        if File.Exists temporary then
            File.Delete temporary

let rec private keptHolds (tags: Set<string>) (value: Value) =
    match value with
    | Data(tag, fields) -> tags.Contains tag || List.exists (keptHolds tags) fields
    | List items -> List.exists (keptHolds tags) items
    | Presence(_, Some inner) -> keptHolds tags inner
    | _ -> false

/// Keep the case's inputs when it holds a value of a restricted type, once
/// per run.
[<CompiledName("KeptRecord")>]
let keptRecord (spec: string) (descriptors: string list) (values: Value list) =
    if not (keptRecorded.ContainsKey spec) then
        match keptFile spec with
        | Some file when keptCap spec > 0 ->
            let tags = keptWords spec |> Array.skip 2 |> Set.ofArray

            if List.exists (keptHolds tags) values then
                keptRecorded.[spec] <- true

                try
                    let inputs =
                        List.map2
                            (fun descriptor value ->
                                let table, d = Descriptors.fromText descriptor
                                Convert.ToHexString(Descriptors.encode d value table).ToLowerInvariant())
                            descriptors
                            values

                    keptUpdate file spec (fun entries ->
                        let same = entries |> List.tryFind (fun e -> e.Inputs = inputs)

                        (entries |> List.filter (fun e -> e.Inputs <> inputs))
                        @ [ defaultArg same { Inputs = inputs; Used = 0 } ])
                with _ ->
                    ()
        | _ -> keptRecorded.[spec] <- true

/// Run the law on the cases earlier runs kept, after checking each against
/// the current definitions; a case that no longer meets them is dropped.
/// `check` answers None for a case outside the law's own refinements.
[<CompiledName("KeptReplay")>]
let keptReplay
    (law: string)
    (spec: string)
    (descriptors: string list)
    (schema: Schema.Schema)
    (references: TypeRef list)
    (check: Value list -> Value option)
    =
    match keptFile spec with
    | Some file when keptCap spec > 0 ->
        let all = keptRead file

        let considered =
            if all.Length > keptCap spec then
                List.skip (all.Length - keptCap spec) all
            else
                all

        let mutable dropped = []
        let mutable used = []

        try
            for entry in considered do
                let values =
                    try
                        if entry.Inputs.Length <> descriptors.Length then
                            None
                        else
                            Some(
                                List.map3
                                    (fun descriptor reference (hex: string) ->
                                        let table, d = Descriptors.fromText descriptor

                                        Schema.validate
                                            (Descriptors.decode d (Convert.FromHexString hex) table)
                                            reference
                                            schema)
                                    descriptors
                                    references
                                    entry.Inputs
                            )
                    with _ ->
                        None

                match values with
                | None -> dropped <- entry.Inputs :: dropped
                | Some values ->
                    match check values with
                    | None -> dropped <- entry.Inputs :: dropped
                    | Some _ -> used <- entry.Inputs :: used
        finally
            if not dropped.IsEmpty || not used.IsEmpty then
                keptUpdate file spec (fun entries ->
                    entries
                    |> List.filter (fun e -> not (List.contains e.Inputs dropped))
                    |> List.map (fun e ->
                        if List.contains e.Inputs used then
                            { e with Used = e.Used + 1 }
                        else
                            e))

        if not used.IsEmpty then
            eprintfn "%s: ran %d kept value(s) found by earlier runs in .lawspec/found" law used.Length
    | _ -> ()

let private bounds (d: Descriptors.Descriptor) =
    let number =
        function
        | Descriptors.Number n -> Some n
        | _ -> None

    match d with
    | Descriptors.Forms [ _; _; lo; hi ] ->
        match number lo, number hi with
        | None, None -> BigInteger(-1000000), BigInteger 1000000
        | None, Some h -> BigInteger.Min(BigInteger(-1000000), h - BigInteger 2000000), h
        | Some l, None -> l, BigInteger.Max(BigInteger 1000000, l + BigInteger 2000000)
        | Some l, Some h -> l, h
    | _ -> BigInteger.Zero, BigInteger.Zero

let private distinct (values: 'a list) =
    values
    |> List.fold (fun acc v -> if List.contains v acc then acc else acc @ [ v ]) []

/// Each move changes one integer leaf, within its wire bounds.
let rec private moves (d: Descriptors.Descriptor) (value: Value) table : Value list =
    match Descriptors.resolve d table, value with
    | (Descriptors.Forms [ Descriptors.Atom "int"; _; _; _ ] as resolved), Int n ->
        let lo, hi = bounds resolved
        let two = BigInteger 2

        [
            n + BigInteger.One
            n - BigInteger.One
            n * two
            BigInteger.Divide(n, two)
            n + BigInteger.Divide(hi - n, two)
            n - BigInteger.Divide(n - lo, two)
        ]
        |> List.filter (fun x -> x <> n && x >= lo && x <= hi)
        |> distinct
        |> List.map Int
    | Descriptors.Forms [ Descriptors.Atom "list"; item ], List items ->
        fieldMoves (items |> List.map (fun _ -> item)) items table |> List.map List
    | Descriptors.Forms [ Descriptors.Atom "maybe"; item ], Data("Maybe::Present" as tag, [ v ]) ->
        moves item v table |> List.map (fun c -> Data(tag, [ c ]))
    | Descriptors.Forms [ Descriptors.Atom "either"; left; right ], Data(tag, [ v ]) ->
        moves (if tag = "Either::Left" then left else right) v table
        |> List.map (fun c -> Data(tag, [ c ]))
    | Descriptors.Forms(Descriptors.Atom "data" :: _ :: constructors), Data(tag, values) ->
        let types =
            constructors
            |> List.pick (function
                | Descriptors.Forms(Descriptors.Atom "ctor" :: name :: ds) when
                    (match name with
                     | Descriptors.Quoted s
                     | Descriptors.Atom s -> s = tag
                     | _ -> false)
                    ->
                    Some ds
                | _ -> None)

        fieldMoves types values table |> List.map (fun cs -> Data(tag, cs))
    | _ -> []

and private fieldMoves ds values table =
    match ds, values with
    | d :: ds, v :: vs ->
        (moves d v table |> List.map (fun c -> c :: vs))
        @ (fieldMoves ds vs table |> List.map (fun cs -> v :: cs))
    | _ -> []

let rec private inputMoves
    (tables: (Map<string, Descriptors.Descriptor> * Descriptors.Descriptor) list)
    (values: Value list)
    =
    match tables, values with
    | (table, d) :: tables, v :: vs ->
        (moves d v table |> List.map (fun c -> c :: vs))
        @ (inputMoves tables vs |> List.map (fun tail -> v :: tail))
    | _ -> []

type internal Climb =
    {
        Tried: int
        Accepted: int
        Best: Value option
        Values: Value list option
    }

exception internal SearchFailed of exn * Climb * Value list

/// Climb towards the highest score of a law's `target`, from the run's seed.
/// The check gives None for inputs outside the law's domain, or the score.
[<CompiledName("Climb")>]
let climb (law: string) (descriptors: string list) (check: Value list -> Value option) =
    let seed =
        match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
        | null
        | "" -> BigInteger.Zero
        | text -> BigInteger.Parse text

    let tables = descriptors |> List.map Descriptors.fromText

    let generate (s: BigInteger) =
        let mutable state = Random.seed s

        [
            for table, d in tables ->
                let v, next = Descriptors.generate d table state 8
                state <- next
                v
        ]

    let attempt (values: Value list) (state: Climb) =
        if state.Tried >= 400 then
            false, state
        else
            let next = { state with Tried = state.Tried + 1 }

            let outcome =
                try
                    Choice1Of2(check values)
                with e ->
                    Choice2Of2 e

            match outcome with
            | Choice2Of2 e -> raise (SearchFailed(e, next, values))
            | Choice1Of2 None -> false, next
            | Choice1Of2(Some score) ->
                if Harness.higher score state.Best then
                    true,
                    { next with
                        Accepted = next.Accepted + 1
                        Best = Some score
                        Values = Some values
                    }
                else
                    false,
                    { next with
                        Accepted = next.Accepted + 1
                    }

    let publish outcome (state: Climb) =
        let best = state.Best |> Option.map Harness.scoreText

        printfn
            "%s: targeted search tried %d case(s)%s"
            law
            state.Tried
            (match best with
             | Some b -> "; best score " + b
             | None -> "")

        let search = JsonObject()
        search.["seed"] <- JsonValue.Create(seed.ToString())
        search.["tried"] <- JsonValue.Create(state.Tried)
        search.["accepted"] <- JsonValue.Create(state.Accepted)

        search.["best"] <-
            (match best with
             | Some b -> JsonValue.Create(b) :> JsonNode
             | None -> null)

        Harness.record
            [
                "search", (search :> JsonNode)
                "outcome", (JsonValue.Create(outcome: string) :> JsonNode)
            ]

    try
        let mutable state =
            {
                Tried = 0
                Accepted = 0
                Best = None
                Values = None
            }

        for k in 0..9 do
            state <- snd (attempt (generate (seed + BigInteger k)) state)

        let mutable step = 0

        while step < 60 && state.Tried < 400 do
            let candidates =
                match state.Values with
                | None -> []
                | Some values -> inputMoves tables values |> List.truncate 24

            let mutable rose = false

            for candidate in candidates do
                let rising, next = attempt candidate state
                rose <- rose || rising
                state <- next

            if not rose then
                state <- snd (attempt (generate (seed + BigInteger 1000 + BigInteger step)) state)

            step <- step + 1

        publish "passed" state
    with SearchFailed(e, state, values) ->
        keep law descriptors values

        (try
            publish "failed" state
         with r ->
             eprintfn "LawSpec search statistics failed for %s: %s" law r.Message)

        let inputs =
            List.map2 (fun (table, d) v -> Convert.ToHexString(Descriptors.encode d v table)) tables values

        raise (
            LawSpecException(
                "TargetFailed",
                sprintf "%s (seed %O, inputs %s): %s" law seed (String.Join(", ", inputs)) e.Message,
                [],
                e
            )
        )
