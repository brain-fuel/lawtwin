/// LawSpec's portable regex grammar, on Unicode scalar values rather than
/// .NET's regex engine and its character classes. A match covers the whole
/// text, and matching tracks reachable positions, so an ambiguous repeat
/// never backtracks exponentially. ref:DEC-typed-core-boundary
module LawSpec.Runtime.Regex

open System.Collections.Concurrent

/// A compiled pattern.
type Node =
    /// A character set: negated, and items each (negated, ranges).
    | Set of negated: bool * items: (bool * (int * int) list) list
    | Sequence of Node list
    | Alternatives of Node list
    | Repeat of Node * low: int * high: int option

let private fail reason : 'a =
    Fail.raiseWith "NonPortableRegex" reason

let private require condition reason =
    if not condition then
        fail reason

let private escape (cs: int list) =
    match cs with
    | c :: rest when c = int 'd' || c = int 'D' -> (c = int 'D', [ 48, 57 ]), rest
    | c :: rest when c = int 'w' || c = int 'W' -> (c = int 'W', [ 48, 57; 65, 90; 95, 95; 97, 122 ]), rest
    | c :: rest when c = int 's' || c = int 'S' -> (c = int 'S', [ 9, 13; 32, 32 ]), rest
    | c :: rest ->
        let named = [ int 'n', 10; int 't', 9; int 'r', 13; int 'f', 12; int 'v', 11 ]

        match List.tryFind (fun (k, _) -> k = c) named with
        | Some(_, v) -> (false, [ v, v ]), rest
        | None ->
            require ("\\.^$|?*+()[]{}-/" |> Seq.exists (fun x -> int x = c)) "UnsupportedEscape"
            (false, [ c, c ]), rest
    | [] -> fail "TrailingEscape"

let private classItem cs =
    match cs with
    | c :: rest when c = int '\\' -> escape rest
    | c :: _ when c = int '[' -> fail "UnescapedClassOpen"
    | c :: rest -> (false, [ c, c ]), rest
    | [] -> fail "UnclosedClass"

let rec private charClass cs negated acc =
    match cs with
    | [] -> fail "UnclosedClass"
    | c :: _ when c = int ']' && List.isEmpty acc -> fail "EmptyClass"
    | c :: rest when c = int ']' -> Set(negated, List.rev acc), rest
    | _ ->
        let item, rest = classItem cs

        match item, rest with
        | (false, [ lo, lo' ]), dash :: c :: tail when lo = lo' && dash = int '-' && c <> int ']' ->
            match classItem (c :: tail) with
            | (false, [ hi, hi' ]), next when hi = hi' && hi >= lo ->
                charClass next negated ((false, [ lo, hi ]) :: acc)
            | _ -> fail "InvalidClassRange"
        | _ -> charClass rest negated (item :: acc)

let private count cs =
    let digits = cs |> List.takeWhile (fun c -> c >= int '0' && c <= int '9')
    require (not (List.isEmpty digits)) "MissingRepetitionCount"
    let value = digits |> List.fold (fun n d -> n * 10 + (d - int '0')) 0
    value, List.skip digits.Length cs

let private repeat node low high cs =
    require
        (low <= 1000
         && (match high with
             | None -> true
             | Some h -> h <= 1000 && h >= low))
        "InvalidRepetitionCount"

    match cs with
    | c :: _ -> require (not ("*+?{" |> Seq.exists (fun x -> int x = c))) "RepeatedQuantifier"
    | [] -> ()

    Repeat(node, low, high), cs

let private quantified node cs =
    match cs with
    | c :: rest when c = int '*' -> repeat node 0 None rest
    | c :: rest when c = int '+' -> repeat node 1 None rest
    | c :: rest when c = int '?' -> repeat node 0 (Some 1) rest
    | c :: rest when c = int '{' ->
        let low, after = count rest

        match after with
        | c :: next when c = int '}' -> repeat node low (Some low) next
        | c :: d :: next when c = int ',' && d = int '}' -> repeat node low None next
        | c :: tail when c = int ',' ->
            let high, after = count tail

            match after with
            | d :: next when d = int '}' -> repeat node low (Some high) next
            | _ -> fail "UnclosedRepetition"
        | _ -> fail "InvalidRepetition"
    | _ -> node, cs

let rec private alternatives cs =
    let branch, rest = sequence cs []

    let rec more rest acc =
        match rest with
        | c :: tail when c = int '|' ->
            let next, after = sequence tail []
            more after (next :: acc)
        | _ ->
            match acc with
            | [ only ] -> only, rest
            | _ -> Alternatives(List.rev acc), rest

    more rest [ branch ]

and private sequence cs acc =
    match cs with
    | [] -> Sequence(List.rev acc), []
    | c :: _ when c = int ')' || c = int '|' -> Sequence(List.rev acc), cs
    | _ ->
        let atom, rest = atom cs
        let node, next = quantified atom rest
        sequence next (node :: acc)

and private atom cs =
    match cs with
    | o :: q :: c :: rest when o = int '(' && q = int '?' && c = int ':' -> group rest
    | o :: q :: _ when o = int '(' && q = int '?' -> fail "UnsupportedGroup"
    | o :: rest when o = int '(' -> group rest
    | o :: n :: rest when o = int '[' && n = int '^' -> charClass rest true []
    | o :: rest when o = int '[' -> charClass rest false []
    | d :: rest when d = int '.' -> Set(true, [ false, [ 10, 10 ] ]), rest
    | b :: rest when b = int '\\' ->
        let item, after = escape rest
        Set(false, [ item ]), after
    | c :: rest ->
        require (not ("*+?{^$]}" |> Seq.exists (fun x -> int x = c))) "UnexpectedCharacter"
        Set(false, [ false, [ c, c ] ]), rest
    | [] -> fail "MissingAtom"

and private group cs =
    let node, rest = alternatives cs

    match rest with
    | c :: next when c = int ')' -> node, next
    | _ -> fail "UnclosedGroup"

let private cache = ConcurrentDictionary<string, Node>()

/// Compile a pattern once; compiled patterns are shared.
[<CompiledName("Compile")>]
let compile (pattern: string) =
    cache.GetOrAdd(
        pattern,
        fun pattern ->
            if not (Scalar.isWellFormed pattern) then
                Fail.raiseWith "InvalidUnicode" ""

            let node, rest = alternatives (Scalar.codePoints pattern)
            require (List.isEmpty rest) "UnmatchedClosingParenthesis"
            node
    )

let private inSet (c: int) items =
    items
    |> List.exists (fun (negated, ranges) -> negated <> (ranges |> List.exists (fun (lo, hi) -> c >= lo && c <= hi)))

let rec private reach node (text: int[]) (positions: Set<int>) : Set<int> =
    match node with
    | Set(negated, items) ->
        positions
        |> Seq.filter (fun p -> p < text.Length && inSet text.[p] items <> negated)
        |> Seq.map (fun p -> p + 1)
        |> Set.ofSeq
    | Sequence nodes -> nodes |> List.fold (fun ps n -> reach n text ps) positions
    | Alternatives nodes -> nodes |> List.map (fun n -> reach n text positions) |> Set.unionMany
    | Repeat(inner, low, high) ->
        let mutable required = positions
        let mutable i = 0

        while i < low && not required.IsEmpty do
            required <- reach inner text required
            i <- i + 1

        let mutable seen = required
        let mutable frontier = required
        let mutable remaining = high |> Option.map (fun h -> h - low)

        while not frontier.IsEmpty && remaining <> Some 0 do
            let next = Set.difference (reach inner text frontier) seen
            seen <- Set.union seen next
            frontier <- next
            remaining <- remaining |> Option.map (fun r -> r - 1)

        seen

/// Whether a compiled pattern matches the whole text.
[<CompiledName("Run")>]
let run (node: Node) (text: string) =
    let points = Scalar.codePoints text |> Array.ofList
    reach node points (Set.singleton 0) |> Set.contains points.Length

/// Whether a pattern matches the whole text.
[<CompiledName("Matches")>]
let matches (pattern: string) (text: string) = run (compile pattern) text
