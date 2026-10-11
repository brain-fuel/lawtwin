/// LawSpec's portable equality, hash and order over native .NET values. The
/// generated records and unions use it, so a native value orders as its
/// portable value does on every target: text by code point, constructors by
/// their identities, fields left to right. ref:DEC-portable-exact-arithmetic
/// ref:DEC-idiomatic-generated-types
module LawSpec.Runtime.Portable

open System
open System.Collections
open System.Numerics
open System.Text
open LawSpec

let private noOrder (a: obj) = Fail.raiseWith "NoPortableOrder" (if isNull a then "null" else a.GetType().Name)

/// Equality: structural, with floats compared by their bits (so it is
/// reflexive, as .NET collections require).
[<CompiledName("Equal")>]
let rec equal (a: obj) (b: obj) : bool =
    match a, b with
    | null, null -> true
    | null, _ | _, null -> false
    | (:? float as x), (:? float as y) -> BitConverter.DoubleToInt64Bits x = BitConverter.DoubleToInt64Bits y
    | (:? float32 as x), (:? float32 as y) -> BitConverter.SingleToInt32Bits x = BitConverter.SingleToInt32Bits y
    | (:? string as x), (:? string as y) -> String.Equals(x, y, StringComparison.Ordinal)
    | (:? (byte[]) as x), (:? (byte[]) as y) -> ReadOnlySpan<byte>(x).SequenceEqual(ReadOnlySpan<byte>(y))
    | (:? IStructuralEquatable as x), _ when not (a :? string) -> x.Equals(b, StructuralComparisons.StructuralEqualityComparer)
    | _ -> a.Equals b

/// A hash consistent with equal.
[<CompiledName("Hash")>]
let rec hash (a: obj) : int =
    match a with
    | null -> 0
    | :? float as x -> BitConverter.DoubleToInt64Bits(x).GetHashCode()
    | :? float32 as x -> BitConverter.SingleToInt32Bits(x)
    | :? (byte[]) as x ->
        let mutable h = 17
        for b in x do h <- h * 31 + int b
        h
    | :? IStructuralEquatable as x -> x.GetHashCode(StructuralComparisons.StructuralEqualityComparer)
    | _ -> a.GetHashCode()

/// Generated records and unions implement this: their CompareTo is the
/// portable order.
type IPortable =
    inherit IComparable

let private unionCache = Collections.Concurrent.ConcurrentDictionary<Type, (obj -> int) * (obj -> obj[])>()

let private unionParts (t: Type) =
    unionCache.GetOrAdd(t, fun t ->
        let tag = Reflection.FSharpValue.PreComputeUnionTagReader(t)
        let readers =
            Reflection.FSharpType.GetUnionCases(t)
            |> Array.map (fun c -> Reflection.FSharpValue.PreComputeUnionReader(c))
        tag, (fun v -> readers.[tag v] v))

/// Text by code point, for a .NET collection that takes a comparer
/// (StringComparer.Ordinal orders by UTF-16 unit, which differs for
/// characters outside the Basic Multilingual Plane).
[<CompiledName("TextOrder")>]
let textOrder: Collections.Generic.IComparer<string> =
    { new Collections.Generic.IComparer<string> with
        member _.Compare(a, b) = Scalar.compareText a b }

let private orderCache = Collections.Concurrent.ConcurrentDictionary<Type, bool>()

/// Whether F#'s own comparison of a type can disagree with the portable
/// order: it holds text somewhere F# compares (ordinally, by UTF-16 unit)
/// rather than a generated type, which compares portably.
[<CompiledName("Reorders")>]
let rec reorders (t: Type) : bool =
    match orderCache.TryGetValue t with
    | true, known -> known
    | _ ->
        let result =
            if t = typeof<string> then true
            elif typeof<IPortable>.IsAssignableFrom t then false
            elif t.IsArray then reorders (t.GetElementType())
            elif t.IsGenericType then t.GetGenericArguments() |> Array.exists reorders
            else false
        orderCache.[t] <- result
        result

let private setType = typedefof<Set<int>>
let private mapType = typedefof<Map<int, int>>

/// The portable order of two native values of one type.
[<CompiledName("Compare")>]
let rec compare (a: obj) (b: obj) : int =
    match a, b with
    | null, null -> 0
    | null, _ -> -1
    | _, null -> 1
    | (:? string as x), (:? string as y) -> Scalar.compareText x y
    | _ when a.GetType().IsGenericType && a.GetType().GetGenericTypeDefinition() = setType ->
        compareSequence (portableItems a) (portableItems b)
    | _ when a.GetType().IsGenericType && a.GetType().GetGenericTypeDefinition() = mapType ->
        compareSequence (portableEntries a) (portableEntries b)
    | (:? (byte[]) as x), (:? (byte[]) as y) -> Scalar.compareBytes x y
    | (:? bool as x), (:? bool as y) -> Operators.compare x y
    | (:? BigInteger as x), (:? BigInteger as y) -> sign (BigInteger.Compare(x, y))
    | (:? Rune as x), (:? Rune as y) -> Operators.compare x.Value y.Value
    | (:? float as x), _ -> noOrder x
    | (:? float32 as x), _ -> noOrder x
    | (:? Symbol as x), _ -> noOrder x
    | (:? IPortable as x), _ -> sign (x.CompareTo b)
    | (:? IComparable as x), _ when a.GetType().IsPrimitive -> sign (x.CompareTo b)
    | _ when Reflection.FSharpType.IsUnion(a.GetType()) && not (a :? IEnumerable) ->
        let tag, fields = unionParts (a.GetType())
        let ta, tb = tag a, tag b
        if ta <> tb then Operators.compare ta tb
        else compareSequence (fields a :> IEnumerable) (fields b :> IEnumerable)
    | (:? IEnumerable as xs), (:? IEnumerable as ys) -> compareSequence xs ys
    | (:? IComparable as x), _ -> sign (x.CompareTo b)
    | _ -> noOrder a

/// A Set's items in the portable order.
and private portableItems (s: obj) : IEnumerable =
    let items = (s :?> IEnumerable) |> Seq.cast<obj> |> Array.ofSeq
    items |> Array.sortWith compare :> IEnumerable

/// A Map's entries, each key then its value, in the portable order of the keys.
and private portableEntries (m: obj) : IEnumerable =
    let entries =
        (m :?> IEnumerable) |> Seq.cast<obj>
        |> Seq.map (fun e ->
            let t = e.GetType()
            t.GetProperty("Key").GetValue e, t.GetProperty("Value").GetValue e)
        |> Array.ofSeq
        |> Array.sortWith (fun (k, _) (l, _) -> compare k l)
    entries |> Seq.collect (fun (k, v) -> [ k; v ]) |> Seq.toArray :> IEnumerable

and private compareSequence (xs: IEnumerable) (ys: IEnumerable) =
    let ys = ys.GetEnumerator()
    let xs = xs.GetEnumerator()
    let mutable result = None
    while result.IsNone do
        match xs.MoveNext(), ys.MoveNext() with
        | false, false -> result <- Some 0
        | false, true -> result <- Some -1
        | true, false -> result <- Some 1
        | true, true ->
            match compare xs.Current ys.Current with
            | 0 -> ()
            | c -> result <- Some c
    result.Value

/// Compare two constructors' ranks and then their fields.
[<CompiledName("CompareData")>]
let compareData (rankA: int) (fieldsA: obj list) (rankB: int) (fieldsB: obj list) =
    if rankA <> rankB then Operators.compare rankA rankB
    else
        let rec go xs ys =
            match xs, ys with
            | x :: xs, y :: ys ->
                match compare x y with
                | 0 -> go xs ys
                | c -> c
            | [], [] -> 0
            | [], _ -> -1
            | _, [] -> 1
        go fieldsA fieldsB

/// Whether two constructors' fields are equal.
[<CompiledName("EqualData")>]
let equalData (rankA: int) (fieldsA: obj list) (rankB: int) (fieldsB: obj list) =
    rankA = rankB && List.length fieldsA = List.length fieldsB && List.forall2 equal fieldsA fieldsB

/// A hash of a constructor and its fields.
[<CompiledName("HashData")>]
let hashData (rank: int) (fields: obj list) = fields |> List.fold (fun h f -> h * 31 + hash f) (rank + 7)
