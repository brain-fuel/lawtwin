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
    | null | "" -> None
    | directory ->
        let name = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes law)).ToLowerInvariant()
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
        let temporary = file + "." + string Environment.ProcessId + "." + Guid.NewGuid().ToString("N") + ".tmp"
        try
            File.WriteAllText(temporary, record.ToJsonString())
            File.Move(temporary, file, true)
        finally
            if File.Exists temporary then File.Delete temporary

let private keep law descriptors values =
    try remember law descriptors values
    with e -> eprintfn "LawSpec could not save inputs for %s: %s" law e.Message

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
                if record.["law"].GetValue<string>() <> law then None
                else
                    let inputs = record.["inputs"].AsArray() |> Seq.map (fun n -> n.GetValue<string>()) |> List.ofSeq
                    Some(List.map2 (fun descriptor (hex: string) ->
                        let table, d = Descriptors.fromText descriptor
                        Descriptors.decode d (Convert.FromHexString hex) table) descriptors inputs)
            with _ -> None
        match values with
        | None -> eprintfn "%s: discarding incompatible saved inputs" law
        | Some values ->
            eprintfn "%s: replaying the failing inputs kept in .lawspec/failures" law
            match check values with
            | Some(Bool false) -> Fail.raiseWith "ReplayFailed" law
            | _ -> ()
        File.Delete file

let private bounds (d: Descriptors.Descriptor) =
    let number = function
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
    values |> List.fold (fun acc v -> if List.contains v acc then acc else acc @ [ v ]) []

/// Each move changes one integer leaf, within its wire bounds.
let rec private moves (d: Descriptors.Descriptor) (value: Value) table : Value list =
    match Descriptors.resolve d table, value with
    | (Descriptors.Forms [ Descriptors.Atom "int"; _; _; _ ] as resolved), Int n ->
        let lo, hi = bounds resolved
        let two = BigInteger 2
        [ n + BigInteger.One; n - BigInteger.One; n * two; BigInteger.Divide(n, two)
          n + BigInteger.Divide(hi - n, two); n - BigInteger.Divide(n - lo, two) ]
        |> List.filter (fun x -> x <> n && x >= lo && x <= hi)
        |> distinct
        |> List.map Int
    | Descriptors.Forms [ Descriptors.Atom "list"; item ], List items ->
        fieldMoves (items |> List.map (fun _ -> item)) items table |> List.map List
    | Descriptors.Forms [ Descriptors.Atom "maybe"; item ], Data("Maybe::Just" as tag, [ v ]) ->
        moves item v table |> List.map (fun c -> Data(tag, [ c ]))
    | Descriptors.Forms [ Descriptors.Atom "either"; left; right ], Data(tag, [ v ]) ->
        moves (if tag = "Either::Left" then left else right) v table |> List.map (fun c -> Data(tag, [ c ]))
    | Descriptors.Forms(Descriptors.Atom "data" :: _ :: constructors), Data(tag, values) ->
        let types =
            constructors
            |> List.pick (function
                | Descriptors.Forms(Descriptors.Atom "ctor" :: name :: ds) when
                    (match name with Descriptors.Quoted s | Descriptors.Atom s -> s = tag | _ -> false) -> Some ds
                | _ -> None)
        fieldMoves types values table |> List.map (fun cs -> Data(tag, cs))
    | _ -> []

and private fieldMoves ds values table =
    match ds, values with
    | d :: ds, v :: vs ->
        (moves d v table |> List.map (fun c -> c :: vs)) @ (fieldMoves ds vs table |> List.map (fun cs -> v :: cs))
    | _ -> []

let rec private inputMoves (tables: (Map<string, Descriptors.Descriptor> * Descriptors.Descriptor) list) (values: Value list) =
    match tables, values with
    | (table, d) :: tables, v :: vs ->
        (moves d v table |> List.map (fun c -> c :: vs)) @ (inputMoves tables vs |> List.map (fun tail -> v :: tail))
    | _ -> []

type internal Climb =
    { Tried: int
      Accepted: int
      Best: Value option
      Values: Value list option }

exception internal SearchFailed of exn * Climb * Value list

/// Climb towards the highest score of a law's `target`, from the run's seed.
/// The check gives None for inputs outside the law's domain, or the score.
[<CompiledName("Climb")>]
let climb (law: string) (descriptors: string list) (check: Value list -> Value option) =
    let seed =
        match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
        | null | "" -> BigInteger.Zero
        | text -> BigInteger.Parse text
    let tables = descriptors |> List.map Descriptors.fromText
    let generate (s: BigInteger) =
        let mutable state = Random.seed s
        [ for table, d in tables ->
              let v, next = Descriptors.generate d table state 8
              state <- next
              v ]
    let attempt (values: Value list) (state: Climb) =
        if state.Tried >= 400 then false, state
        else
            let next = { state with Tried = state.Tried + 1 }
            let outcome = try Choice1Of2(check values) with e -> Choice2Of2 e
            match outcome with
            | Choice2Of2 e -> raise (SearchFailed(e, next, values))
            | Choice1Of2 None -> false, next
            | Choice1Of2(Some score) ->
                if Harness.higher score state.Best then
                    true, { next with Accepted = next.Accepted + 1; Best = Some score; Values = Some values }
                else false, { next with Accepted = next.Accepted + 1 }
    let publish outcome (state: Climb) =
        let best = state.Best |> Option.map Harness.scoreText
        printfn "%s: targeted search tried %d case(s)%s" law state.Tried (match best with Some b -> "; best score " + b | None -> "")
        let search = JsonObject()
        search.["seed"] <- JsonValue.Create(seed.ToString())
        search.["tried"] <- JsonValue.Create(state.Tried)
        search.["accepted"] <- JsonValue.Create(state.Accepted)
        search.["best"] <- (match best with Some b -> JsonValue.Create(b) :> JsonNode | None -> null)
        Harness.record [ "search", (search :> JsonNode); "outcome", (JsonValue.Create(outcome: string) :> JsonNode) ]
    try
        let mutable state = { Tried = 0; Accepted = 0; Best = None; Values = None }
        for k in 0 .. 9 do
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
            if not rose then state <- snd (attempt (generate (seed + BigInteger 1000 + BigInteger step)) state)
            step <- step + 1
        publish "passed" state
    with SearchFailed(e, state, values) ->
        keep law descriptors values
        (try publish "failed" state with r -> eprintfn "LawSpec search statistics failed for %s: %s" law r.Message)
        let inputs =
            List.map2 (fun (table, d) v -> Convert.ToHexString(Descriptors.encode d v table)) tables values
        raise (LawSpecException("TargetFailed", sprintf "%s (seed %O, inputs %s): %s" law seed (String.Join(", ", inputs)) e.Message, [], e))
