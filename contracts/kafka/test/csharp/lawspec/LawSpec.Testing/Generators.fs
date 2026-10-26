/// Generation from a program's schema: scalars within their refinement
/// bounds, lists within their lengths, and data within a structural budget
/// that counts every node, so recursive types terminate without a hidden cap
/// and deep singletons stay reachable. Indexed families are solved backwards
/// from their target index. ref:DEC-structural-size-budget
/// ref:DEC-shrink-within-domain ref:DEC-indexed-families-as-evidence
module LawSpec.Testing.Generators

open System
open System.Collections.Generic
open System.Numerics
open System.Runtime.CompilerServices
open LawSpec.Runtime
open LawSpec.Runtime.Schema

/// A comparison bound on an input, or on its length.
type Bound =
    | Compare of op: string * value: Value
    | Length of op: string * value: Value

/// A native generator for a data type, given generators for its arguments.
type Factory = Gen<Value> list -> Gen<Value>

/// What generation reads beyond the schema: native generator factories by
/// type name (from lawspec.edn) and the attempts budget.
type Context =
    {
        Schema: Schema
        Symbols: Scalar.SymbolContext
        Factories: Map<string, Factory>
        Attempts: int
    }

let private minimums =
    ConditionalWeakTable<Schema, Dictionary<TypeRef * int, int option>>()

let private exactBound (value: Value) =
    let n, d = Scalar.exact value

    let floor =
        BigInteger.Divide(n, d)
        - (if n.Sign < 0 && not (BigInteger.Remainder(n, d)).IsZero then
               BigInteger.One
           else
               BigInteger.Zero)

    let ceil =
        floor
        + (if (BigInteger.Remainder(n, d)).IsZero then
               BigInteger.Zero
           else
               BigInteger.One)

    floor, ceil

let private lower (lo: BigInteger option) n =
    Some(
        match lo with
        | None -> n
        | Some l -> BigInteger.Max(l, n)
    )

let private upper (hi: BigInteger option) n =
    Some(
        match hi with
        | None -> n
        | Some h -> BigInteger.Min(h, n)
    )

let private limit (lo, hi) (op: string, value: Value) =
    let floor, ceil = exactBound value

    match op with
    | ">" -> lower lo (floor + BigInteger.One), hi
    | ">=" -> lower lo ceil, hi
    | "<" -> lo, upper hi (ceil - BigInteger.One)
    | "<=" -> lo, upper hi floor
    | "==" -> lower lo ceil, upper hi floor
    | _ -> lo, hi

let private lengths bounds =
    bounds
    |> List.choose (function
        | Length(op, v) -> Some(op, v)
        | _ -> None)

let private comparisons bounds =
    bounds
    |> List.choose (function
        | Compare(op, v) -> Some(op, v)
        | _ -> None)

let private lengthRange bounds =
    lengths bounds |> List.fold limit (Some BigInteger.Zero, None)

let private lengthAllowed n bounds =
    let lo, hi = lengthRange bounds
    let n = BigInteger(n: int)

    (match lo with
     | Some l -> n >= l
     | None -> true)
    && (match hi with
        | Some h -> n <= h
        | None -> true)

let private lengthGenerator bounds (size: int) =
    match lengthRange bounds with
    | Some lo, Some hi when lo > hi -> Gen.empty
    | lo, hi ->
        let lo = defaultArg lo BigInteger.Zero

        Gen.integer (Some lo) (upper hi (BigInteger.Max(lo, BigInteger size)))
        |> Gen.map int

let private sequenceOf (element: Gen<'a>) bounds =
    match lengths bounds with
    | [] -> Gen.list element
    | _ -> Gen.sized (fun size -> Gen.bind (lengthGenerator bounds size) (fun count -> Gen.vector count element))

let private unicodeScalar =
    Gen.oneof
        [
            Gen.integer (Some BigInteger.Zero) (Some(BigInteger 0xD7FF))
            Gen.integer (Some(BigInteger 0xE000)) (Some(BigInteger 0x10FFFF))
        ]

let private ieee width =
    Gen.integer (Some BigInteger.Zero) (Some((BigInteger.One <<< width) - BigInteger.One))
    |> Gen.map (fun bits -> Float(width, uint64 bits))

let private scalar (context: Context) (t: TypeRef) (bounds: Bound list) : Gen<Value> =
    match t with
    | Named("Bool", []) -> Gen.oneof [ Gen.constant (Bool false); Gen.constant (Bool true) ]
    | Named("Unit", []) -> Gen.constant Unit
    | Named("Null", []) -> Gen.constant Null
    | Named("Undefined", []) -> Gen.constant Undefined
    | Named("Rational", []) ->
        Gen.bind (Gen.integer None None) (fun n ->
            Gen.integer (Some BigInteger.One) None |> Gen.map (fun d -> Scalar.ratio n d))
    | Named("Decimal", []) ->
        Gen.bind (Gen.integer None None) (fun c ->
            Gen.sized (fun size -> Gen.range -size size |> Gen.map (fun e -> Value.Decimal(c, e))))
    | Named("Float32", []) -> ieee 32
    | Named("Float64", []) -> ieee 64
    | Named(("Complex64" | "Complex128") as name, []) ->
        let w = if name = "Complex64" then 32 else 64
        Gen.bind (ieee w) (fun r -> ieee w |> Gen.map (fun i -> Value.Complex(w, r, i)))
    | Named("Char", []) -> unicodeScalar |> Gen.map Int
    | Named("CodePoint", []) -> Gen.integer (Some BigInteger.Zero) (Some(BigInteger 0x10FFFF)) |> Gen.map Int
    | Named("CodeUnit16", []) -> Gen.integer (Some BigInteger.Zero) (Some(BigInteger 0xFFFF)) |> Gen.map Int
    | Named("Text", []) ->
        sequenceOf unicodeScalar bounds
        |> Gen.map (List.map int >> Scalar.textOfPoints >> Text)
    | Named("Bytes", []) ->
        sequenceOf (Gen.integer (Some BigInteger.Zero) (Some(BigInteger 255))) bounds
        |> Gen.map (fun units -> Bytes(units |> List.map byte |> Array.ofList))
    | Named(("CodePointText" | "Utf16Text") as name, []) ->
        let top = if name = "CodePointText" then 0x10FFFF else 0xFFFF

        sequenceOf (Gen.integer (Some BigInteger.Zero) (Some(BigInteger top))) bounds
        |> Gen.map (fun units -> Raw(name, units |> List.map int))
    | Named("Symbol", []) ->
        Gen.integer (Some BigInteger.Zero) None
        |> Gen.map (fun n ->
            let key = n.ToString()
            Value.Symbol(context.Symbols.Identity("generated:" + key), key))
    | Named(name, []) when Scalar.isIntegerType name ->
        let lo, hi =
            comparisons bounds |> List.fold limit (Scalar.bounds name context.Schema.Bits)

        match lo, hi with
        | Some l, Some h when l > h -> Gen.empty
        | _ -> Gen.integer lo hi |> Gen.map Int
    | _ -> Fail.raiseWith "NoGenerator" (Scalar.typeKey t)

/// Whether a value is a member of its type, contracts included.
[<CompiledName("Accepted")>]
let accepted (value: Value) (t: TypeRef) (schema: Schema) =
    try
        validate value t schema |> ignore
        true
    with :? LawSpecException as e when e.Code = "RefinementViolation" ->
        false

let rec private valueNodes (value: Value) =
    match value with
    | Data(_, fields) -> 1 + (fields |> List.sumBy valueNodes)
    | Presence(_, Some v) -> 1 + valueNodes v
    | List items -> 1 + (items |> List.sumBy (fun v -> 1 + valueNodes v))
    | _ -> 1

/// Sets and maps are canonical: sorted and without duplicate keys, also
/// after every shrink.
let private canonical (t: TypeRef) (value: Value) =
    let distinctSorted key items =
        items
        |> List.sortWith (fun a b -> Scalar.compareValues (key a) (key b))
        |> List.fold
            (fun acc item ->
                match acc with
                | last :: _ when Scalar.compareValues (key last) (key item) = 0 -> acc
                | _ -> item :: acc)
            []
        |> List.rev

    match t, value with
    | Named("lawspec.collections::type::EqSet", _), Data(tag, [ List items ]) ->
        let distinct =
            items
            |> List.fold
                (fun previous item ->
                    if List.exists (Scalar.equal item) previous then
                        previous
                    else
                        item :: previous)
                []
            |> List.rev

        Data(tag, [ List distinct ])
    | Named("lawspec.collections::type::Set", _), Data(tag, [ List items ]) ->
        Data(tag, [ List(distinctSorted id items) ])
    | Named("lawspec.collections::type::KeyVal", _), Data(tag, [ List entries ]) ->
        Data(
            tag,
            [
                List(
                    distinctSorted
                        (function
                        | Data(_, [ k; _ ]) -> k
                        | v -> v)
                        entries
                )
            ]
        )
    | _ -> value

let private factory (context: Context) (t: TypeRef) =
    match t with
    | Named(name, _) -> context.Factories.TryFind name
    | _ -> None

let rec private inhabited (context: Context) (t: TypeRef) budget =
    if budget < 1 then
        false
    else
        match t with
        | Named(("List" | "Optional" | "Nullable"), _) -> true
        | _ when (factory context t).IsSome -> true
        | _ ->
            match constructors t context.Schema with
            | None -> true
            | Some cs ->
                cs
                |> List.collect witnessInstances
                |> List.exists (fun (c: Constructor) ->
                    Option.isSome (allocation context c.Fields (budget - 1 - List.length c.WitnessValues)))

/// The least structural cost of a type, if it is at most the limit.
and [<CompiledName("Minimum")>] minimum (context: Context) (t: TypeRef) (limit: int) : int option =
    if limit < 1 then
        None
    else
        let cache = minimums.GetValue(context.Schema, fun _ -> Dictionary())
        let key = t, limit

        match
            lock cache (fun () ->
                match cache.TryGetValue key with
                | true, v -> Some v
                | _ -> None)
        with
        | Some cost -> cost
        | None ->
            let cost = seq { 1..limit } |> Seq.tryFind (fun c -> inhabited context t c)
            lock cache (fun () -> cache.[key] <- cost)
            cost

and private costs (context: Context) (fields: (string * TypeRef) list) (budget: int) =
    match fields with
    | [] -> if budget >= 0 then Some [] else None
    | _ when budget < 1 -> None
    | (_, t) :: rest ->
        minimum context t budget
        |> Option.bind (fun cost -> costs context rest (budget - cost) |> Option.map (fun more -> cost :: more))

/// Each field's budget: its minimum, plus an even share of what is left.
and [<CompiledName("Allocation")>] allocation (context: Context) (fields: (string * TypeRef) list) (budget: int) =
    costs context fields budget
    |> Option.map (fun minimums ->
        match minimums with
        | [] -> []
        | _ ->
            let extra = budget - List.sum minimums
            let count = List.length minimums

            minimums
            |> List.mapi (fun i cost -> cost + extra / count + (if i < extra % count then 1 else 0)))

let private withWitnesses (native: Gen<Value>) (witnesses: Value list) = Gen.withExamples witnesses native

let rec private build (context: Context) (t: TypeRef) (budget: int) (bounds: Bound list) : Gen<Value> =
    match factory context t with
    | Some f -> custom context t budget bounds f
    | None -> buildNative context t budget bounds

and private custom (context: Context) (t: TypeRef) (budget: int) (bounds: Bound list) (f: Factory) =
    let args =
        match t with
        | Named(_, a) -> a
        | _ -> []

    let children =
        args
        |> List.map (fun child ->
            let available = max 1 (budget - 1)

            match minimum context child available with
            | None -> Gen.empty
            | Some _ -> build context child available [])

    let checkedGen = f children |> Gen.map (fun v -> validate v t context.Schema)

    match bounds with
    | [] -> checkedGen
    | _ ->
        checkedGen
        |> Gen.refine (fun v ->
            bounds
            |> List.forall (function
                | Length(op, b) ->
                    Scalar.binary
                        op
                        (Scalar.helper "length" [ v ] [ Scalar.typeKey t ] context.Schema.Bits)
                        b
                        "Integer"
                        "Integer"
                        =
                        Bool true
                | Compare(op, b) -> Scalar.binary op v b (Scalar.typeKey t) (Scalar.typeKey t) = Bool true))

and private buildNative (context: Context) (t: TypeRef) (budget: int) (bounds: Bound list) =
    match t with
    | Named("List", [ element ]) ->
        let lengthBounds = lengths bounds

        let limitCost =
            if List.isEmpty lengthBounds then
                budget - 1
            else
                max 64 (budget - 1)

        match minimum context element limitCost with
        | None -> Gen.constant (List []) |> Gen.refine (fun _ -> lengthAllowed 0 bounds)
        | Some cost ->
            Gen.bind (lengthGenerator bounds ((budget - 1) / cost)) (fun count ->
                if count = 0 then
                    Gen.constant (List [])
                else
                    Gen.vector count (build context element (max cost ((budget - 1) / count)) [])
                    |> Gen.map List)
    | Named(("Optional" | "Nullable") as kind, [ inner ]) ->
        let absent = Gen.constant (Presence(kind, None))

        match minimum context inner (budget - 1) with
        | None -> absent
        | Some _ ->
            Gen.oneof
                [
                    absent
                    build context inner (budget - 1) [] |> Gen.map (fun v -> Presence(kind, Some v))
                ]
    | _ ->
        match constructors t context.Schema with
        | None -> scalar context t bounds
        | Some raw ->
            let choices =
                [
                    for c0 in raw do
                        for c in witnessInstances c0 do
                            match allocation context c.Fields (budget - 1 - List.length c.WitnessValues) with
                            | Some shares -> yield constructorGen context c shares
                            | None -> ()
                ]

            Gen.oneof choices
            |> Gen.map (canonical t)
            |> Gen.constrain context.Attempts (fun v -> accepted v t context.Schema)

and private constructorGen (context: Context) (c: Constructor) (shares: int list) =
    let rec fields (remaining: ((string * TypeRef) * int) list) (values: Value list) =
        match remaining with
        | [] -> Gen.constant (Data(c.Tag, values @ c.WitnessValues))
        | ((_, ft), share) :: rest ->
            let position = List.length values + 1

            let hints =
                match c.Generators.TryFind position with
                | Some constraints -> Some(constraints context.Schema c.Arguments values)
                | None -> None

            let bounds, witnesses =
                match hints with
                | Some h -> (h.Bounds |> List.map Compare) @ (h.Lengths |> List.map Length), h.Hints
                | None -> [], []

            let native = build context ft share bounds

            let g =
                if (factory context ft).IsSome then
                    native
                else
                    withWitnesses native witnesses

            Gen.bind g (fun v -> fields rest (values @ [ v ]))

    fields (List.zip c.Fields shares) []

// ---------------------------------------------------------------------------
// Indexed families

type private Term =
    | Constant of BigInteger
    | Field of int
    | Op of string * Term * Term

let private tokens (text: string) =
    text.Split(' ', StringSplitOptions.RemoveEmptyEntries) |> List.ofArray

let rec private parse tokens =
    match tokens with
    | (t: string) :: rest when t.StartsWith "c" -> Constant(BigInteger.Parse(t.Substring 1)), rest
    | t :: rest when t.StartsWith "f" -> Field(int (t.Substring 1)), rest
    | op :: rest ->
        let a, next = parse rest
        let b, after = parse next
        Op(op, a, b), after
    | [] -> Fail.raiseWith "InvalidIndexTerm" ""

let rec private termFields term =
    match term with
    | Constant _ -> []
    | Field i -> [ i ]
    | Op(_, a, b) -> termFields a @ termFields b

let rec private evaluate term (assigned: Map<int, BigInteger>) =
    match term with
    | Constant n -> Some n
    | Field i -> assigned.TryFind i
    | Op(op, a, b) ->
        match evaluate a assigned, evaluate b assigned with
        | Some x, Some y ->
            match op with
            | "+" -> Some(x + y)
            | "-" when x >= y -> Some(x - y)
            | "*" -> Some(x * y)
            | "div" when y.Sign > 0 -> Some(BigInteger.Divide(x, y))
            | "mod" when y.Sign > 0 -> Some(BigInteger.Remainder(x, y))
            | "^" when y.Sign >= 0 && y <= BigInteger 64 -> Some(BigInteger.Pow(x, int y))
            | _ -> None
        | _ -> None

type private Equation =
    {
        Term: Term
        Guards: (string * Term * Term) list
        Positions: int list
    }

let private equation (texts: string list) =
    match texts with
    | text :: guards ->
        let term, _ = parse (tokens text)

        let parsed =
            guards
            |> List.map (fun g ->
                match tokens g with
                | op :: rest ->
                    let a, next = parse rest
                    let b, _ = parse next
                    op, a, b
                | [] -> Fail.raiseWith "InvalidIndexGuard" g)

        let positions =
            (termFields term
             @ (parsed |> List.collect (fun (_, a, b) -> termFields a @ termFields b)))
            |> List.distinct
            |> List.sort

        {
            Term = term
            Guards = parsed
            Positions = positions
        }
    | [] -> Fail.raiseWith "InvalidIndexTerm" ""

let private holds (relation, a, b) assigned =
    match evaluate a assigned, evaluate b assigned with
    | Some x, Some y -> if relation = "==" then x = y else x >= y
    | _ -> false

/// An indexed family's value whose index is the target (or, for a negative
/// target, any of its smallest reachable indices).
[<CompiledName("Indexed")>]
let indexed
    (context: Context)
    (t: TypeRef)
    (target: BigInteger)
    (equations: Map<string, string list>)
    (budget: int)
    : Gen<Value> =
    let parsed = equations |> Map.map (fun _ texts -> equation texts)
    let positions (c: Constructor) = parsed.[c.Tag].Positions

    let rec families pending (found: Map<TypeRef, Constructor list>) =
        match pending with
        | [] -> found
        | t :: rest when found.ContainsKey t -> families rest found
        | t :: rest ->
            let cs =
                match constructors t context.Schema with
                | Some cs -> cs |> List.collect witnessInstances
                | None -> Fail.raiseWith "IndexedTypeRequiresData" (Scalar.typeKey t)

            let children =
                [
                    for c in cs do
                        for i in positions c -> snd (List.item i c.Fields)
                ]

            families (children @ rest) (found.Add(t, cs))

    let all = families [ t ] Map.empty

    let plain =
        [
            for KeyValue(_, cs) in all do
                for c in cs do
                    for i, (_, ft) in List.indexed c.Fields do
                        if not (List.contains i (positions c)) then
                            yield ft, minimum context ft 64
        ]
        |> Map.ofList

    let limitIndex = int (BigInteger.Max(BigInteger.Zero, target)) + 16

    let assignments (c: Constructor) (costs: Map<TypeRef * int, int>) =
        let eq = parsed.[c.Tag]
        let fields = c.Fields

        let ok =
            fields
            |> List.indexed
            |> List.forall (fun (i, (_, ft)) -> List.contains i eq.Positions || (plain.[ft]).IsSome)

        if not ok then
            []
        else
            let choices =
                [
                    for i in eq.Positions ->
                        let _, ft = List.item i fields

                        i,
                        (costs
                         |> Map.toList
                         |> List.choose (fun ((t0, k), _) -> if t0 = ft then Some k else None)
                         |> List.sort)
                ]

            let rec assign choices (assigned: Map<int, BigInteger>) =
                let ready =
                    eq.Guards
                    |> List.filter (fun (_, a, b) -> (termFields a @ termFields b) |> List.forall assigned.ContainsKey)

                if not (ready |> List.forall (fun g -> holds g assigned)) then
                    []
                else
                    match choices with
                    | [] -> [ assigned ]
                    | (i, values) :: rest ->
                        values
                        |> List.collect (fun v -> assign rest (assigned.Add(i, BigInteger(v: int))))

            [
                for assigned in assign choices Map.empty do
                    match evaluate eq.Term assigned with
                    | Some value when value.Sign >= 0 && value <= BigInteger limitIndex ->
                        let required =
                            fields
                            |> List.mapi (fun i (_, ft) ->
                                match assigned.TryFind i with
                                | Some k -> costs.[(ft, int k)]
                                | None -> (plain.[ft]).Value)

                        yield int value, assigned, required
                    | _ -> ()
            ]

    let rec reach (costs: Map<TypeRef * int, int>) =
        let next =
            all
            |> Map.fold
                (fun acc ft cs ->
                    cs
                    |> List.fold
                        (fun table c ->
                            assignments c costs
                            |> List.fold
                                (fun table (k, _, fields) ->
                                    let cost = 1 + List.length c.WitnessValues + List.sum fields

                                    match Map.tryFind (ft, k) table with
                                    | Some existing when existing <= cost -> table
                                    | _ -> Map.add (ft, k) cost table)
                                table)
                        acc)
                costs

        if next = costs then costs else reach next

    let costs = reach Map.empty

    let solutions =
        all
        |> Map.fold
            (fun table ft cs ->
                cs
                |> List.fold
                    (fun acc c ->
                        assignments c costs
                        |> List.fold
                            (fun acc (k, targets, fields) ->
                                let existing = defaultArg (Map.tryFind (ft, k) acc) []
                                Map.add (ft, k) (existing @ [ c, targets, fields ]) acc)
                            acc)
                    table)
            Map.empty

    let levels =
        if target.Sign < 0 then
            costs
            |> Map.toList
            |> List.choose (fun ((t0, k), _) -> if t0 = t then Some k else None)
            |> List.sort
            |> List.truncate 6
        else
            [ int target ] |> List.filter (fun k -> costs.ContainsKey(t, k))

    let rec generate ft k share =
        let choices =
            defaultArg (solutions.TryFind(ft, k)) []
            |> List.filter (fun (c, _, fieldCosts) -> 1 + List.length c.WitnessValues + List.sum fieldCosts <= share)

        Gen.bind
            (Gen.oneof (choices |> List.map Gen.constant))
            (fun (c: Constructor, targets: Map<int, BigInteger>, fieldCosts: int list) ->
                let count = List.length fieldCosts
                let extra = share - 1 - List.length c.WitnessValues - List.sum fieldCosts

                let children =
                    List.zip c.Fields fieldCosts
                    |> List.mapi (fun i ((_, childType), cost) ->
                        let childShare =
                            cost
                            + (if count > 0 then
                                   extra / count + (if i < extra % count then 1 else 0)
                               else
                                   0)

                        match targets.TryFind i with
                        | Some index -> generate childType (int index) childShare
                        | None -> build context childType childShare [])

                Gen.tuple children
                |> Gen.map (fun values -> Data(c.Tag, values @ c.WitnessValues)))
        |> Gen.constrain context.Attempts (fun v -> accepted v ft context.Schema)

    match levels with
    | [] when target.Sign < 0 -> Fail.raiseWith "NoIndexedGenerator" (Scalar.typeKey t)
    | [] -> Gen.empty
    | _ -> Gen.bind (Gen.oneof (levels |> List.map Gen.constant)) (fun k -> generate t k (max budget costs.[(t, k)]))

/// A bound on an input.
[<CompiledName("CompareBound")>]
let compareBound (op: string) (value: Value) = Compare(op, value)

/// A bound on an input's length.
[<CompiledName("LengthBound")>]
let lengthBound (op: string) (value: Value) = Length(op, value)

/// No index to aim at.
[<CompiledName("NoIndex")>]
let noIndex () : (BigInteger * Map<string, string list>) option = None

/// An indexed family's target index and each constructor's index terms.
[<CompiledName("Index")>]
let index (target: Value) (tags: string list) (terms: string list list) =
    match target with
    | Int n -> Some(n, Map.ofList (List.zip tags terms))
    | _ -> Fail.raiseWith "IntegerRequired" "index target"

/// The generator of an input of a type: native factories first, then the
/// type's structure within a budget that grows with the size.
[<CompiledName("Generator")>]
let generator
    (context: Context)
    (t: TypeRef)
    (bounds: Bound list)
    (witnesses: Value list)
    (index: (BigInteger * Map<string, string list>) option)
    =
    Gen.sized (fun size ->
        match factory context t with
        | Some f -> custom context t (size + 1) bounds f
        | None ->
            let limit = List.max (64 :: (witnesses |> List.map valueNodes))

            match minimum context t limit with
            | None -> Fail.raiseWith "NoStructuralGenerator" (Scalar.typeKey t)
            | Some least ->
                let budget = max least (size + 1)

                match index with
                | Some(target, equations) -> indexed context t target equations budget
                | None -> withWitnesses (build context t budget bounds) witnesses)
