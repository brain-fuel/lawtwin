/// What typed definitions use: LawSpec's integer semantics on .NET's native
/// integers (checked, failing as the dynamic code does), text by code point,
/// and the sequence and collection helpers, over the native types the
/// generated code already uses (F# lists, options, Map and Set), so a typed
/// definition never copies a value to call another. ref:DEC-typed-native-definitions
/// ref:DEC-portable-exact-arithmetic
module LawSpec.Runtime.Typed

open System
open System.Numerics
open System.Text
open LawSpec

/// An integer outside its type, as the dynamic code reports it.
[<CompiledName("OutOfRange")>]
let outOfRange (name: string) (value: BigInteger) : 'a =
    Fail.raiseWith "IntegerOutOfRange" (sprintf "%s %O" name value)

/// A branch the checker proved unreachable.
[<CompiledName("Unreachable")>]
let unreachable () : 'a =
    Fail.raiseWith "Unreachable" "unreachable branch reached"

/// A failure the typed code raises: the same exception the dynamic code raises.
[<CompiledName("Raise")>]
let raiseFailure (ability: string) (failure: Value) : 'a = raise (RaisedFailure(ability, failure))

// Narrowing an exact result to a bounded type.

[<CompiledName("Int8")>]
let int8 (v: BigInteger) =
    if v < BigInteger(int SByte.MinValue) || v > BigInteger(int SByte.MaxValue) then
        outOfRange "Int8" v
    else
        sbyte v

[<CompiledName("Int16")>]
let int16 (v: BigInteger) =
    if v < BigInteger(int Int16.MinValue) || v > BigInteger(int Int16.MaxValue) then
        outOfRange "Int16" v
    else
        int16 v

[<CompiledName("Int32")>]
let int32 (v: BigInteger) =
    if v < BigInteger(Int32.MinValue) || v > BigInteger(Int32.MaxValue) then
        outOfRange "Int32" v
    else
        int32 v

[<CompiledName("Int64")>]
let int64 (v: BigInteger) =
    if v < BigInteger(Int64.MinValue) || v > BigInteger(Int64.MaxValue) then
        outOfRange "Int64" v
    else
        int64 v

[<CompiledName("UInt8")>]
let uint8 (v: BigInteger) =
    if v.Sign < 0 || v > BigInteger(int Byte.MaxValue) then
        outOfRange "UInt8" v
    else
        byte v

[<CompiledName("UInt16")>]
let uint16 (v: BigInteger) =
    if v.Sign < 0 || v > BigInteger(int UInt16.MaxValue) then
        outOfRange "UInt16" v
    else
        uint16 v

[<CompiledName("UInt32")>]
let uint32 (v: BigInteger) =
    if v.Sign < 0 || v > BigInteger(UInt32.MaxValue) then
        outOfRange "UInt32" v
    else
        uint32 v

[<CompiledName("UInt64")>]
let uint64 (v: BigInteger) =
    if v.Sign < 0 || v > BigInteger(UInt64.MaxValue) then
        outOfRange "UInt64" v
    else
        uint64 v

/// A bounded native integer, exactly.
[<CompiledName("Exact")>]
let exact (x: obj) : BigInteger =
    match x with
    | :? sbyte as v -> BigInteger(int v)
    | :? int16 as v -> BigInteger(int v)
    | :? int as v -> BigInteger v
    | :? int64 as v -> BigInteger v
    | :? byte as v -> BigInteger(int v)
    | :? uint16 as v -> BigInteger(int v)
    | :? uint32 as v -> BigInteger v
    | :? uint64 as v -> BigInteger v
    | :? BigInteger as v -> v
    | _ -> Fail.raiseWith "IntegerRequired" (string x)

// Checked arithmetic on one bounded type: an overflow is the same failure.

[<CompiledName("Add")>]
let inline add (name: string) (a: ^T) (b: ^T) : ^T =
    try
        Checked.(+) a b
    with :? OverflowException ->
        outOfRange name (exact (box a) + exact (box b))

[<CompiledName("Subtract")>]
let inline subtract (name: string) (a: ^T) (b: ^T) : ^T =
    try
        Checked.(-) a b
    with :? OverflowException ->
        outOfRange name (exact (box a) - exact (box b))

[<CompiledName("Multiply")>]
let inline multiply (name: string) (a: ^T) (b: ^T) : ^T =
    try
        Checked.(*) a b
    with :? OverflowException ->
        outOfRange name (exact (box a) * exact (box b))

/// Exact quotient, truncated towards zero; dividing by zero fails.
[<CompiledName("Quotient")>]
let quotient (a: BigInteger) (b: BigInteger) =
    if b.IsZero then
        Fail.raiseWith "InvalidExactOperation" "quot"
    else
        BigInteger.Divide(a, b)

/// Exact remainder, with the dividend's sign; dividing by zero fails.
[<CompiledName("Remainder")>]
let remainder (a: BigInteger) (b: BigInteger) =
    if b.IsZero then
        Fail.raiseWith "InvalidExactOperation" "rem"
    else
        BigInteger.Remainder(a, b)

[<CompiledName("Power")>]
let power (a: BigInteger) (b: BigInteger) =
    if b.Sign < 0 then
        Fail.raiseWith "InvalidExactOperation" "pow"
    elif b > BigInteger(Int32.MaxValue) then
        Fail.raiseWith "InvalidExactOperation" "pow"
    else
        BigInteger.Pow(a, int b)

// Order.

/// The portable order of two native values: text by code point.
[<CompiledName("Compare")>]
let compare (a: 'T) (b: 'T) : int = Portable.compare (box a) (box b)

// Text, by code point.

[<CompiledName("TextLength")>]
let textLength (s: string) : BigInteger =
    let mutable n = 0

    for _ in s.EnumerateRunes() do
        n <- n + 1

    BigInteger n

let private clamp (value: BigInteger) (lo: int) (hi: int) =
    if value < BigInteger lo then lo
    elif value > BigInteger hi then hi
    else int value

let private window (from: BigInteger) (until: BigInteger) (count: int) =
    let i = clamp from 0 count
    let j = clamp until i count
    i, j

[<CompiledName("SliceText")>]
let sliceText (s: string) (from: BigInteger) (until: BigInteger) =
    let runes = s.EnumerateRunes() |> Seq.toArray
    let i, j = window from until runes.Length
    let b = StringBuilder()

    for k in i .. j - 1 do
        b.Append(runes.[k].ToString()) |> ignore

    b.ToString()

[<CompiledName("SliceBytes")>]
let sliceBytes (s: byte[]) (from: BigInteger) (until: BigInteger) =
    let i, j = window from until s.Length
    Array.sub s i (j - i)

[<CompiledName("SliceList")>]
let sliceList (xs: 'a list) (from: BigInteger) (until: BigInteger) =
    let i, j = window from until (List.length xs)
    xs |> List.skip i |> List.take (j - i)

[<CompiledName("AtList")>]
let atList (xs: 'a list) (i: BigInteger) : 'a option =
    if i.Sign < 0 || i >= BigInteger(List.length xs) then
        None
    else
        Some(List.item (int i) xs)

[<CompiledName("AtBytes")>]
let atBytes (xs: byte[]) (i: BigInteger) : byte option =
    if i.Sign < 0 || i >= BigInteger xs.Length then
        None
    else
        Some xs.[int i]

/// lo, lo + 1, ..., hi - 1.
[<CompiledName("Range")>]
let range (lo: BigInteger) (hi: BigInteger) : BigInteger list =
    [
        let mutable n = lo

        while n < hi do
            yield n
            n <- n + BigInteger.One
    ]

[<CompiledName("Utf8")>]
let utf8 (s: string) = Encoding.UTF8.GetBytes s

[<CompiledName("TextOf")>]
let textOf (n: BigInteger) =
    n.ToString(Globalization.CultureInfo.InvariantCulture)

[<CompiledName("Join")>]
let join (separator: string) (parts: string list) = String.Join(separator, parts)

[<CompiledName("StartsWith")>]
let startsWith (text: string) (prefix: string) =
    text.StartsWith(prefix, StringComparison.Ordinal)

[<CompiledName("EndsWith")>]
let endsWith (text: string) (suffix: string) =
    text.EndsWith(suffix, StringComparison.Ordinal)

[<CompiledName("Contains")>]
let contains (text: string) (part: string) =
    part = "" || text.Contains(part, StringComparison.Ordinal)

// Collections.

[<CompiledName("KeyValOf")>]
let keyValOf (key: 'e -> 'k) (value: 'e -> 'v) (entries: 'e list) : Map<'k, 'v> =
    entries |> List.fold (fun m e -> Map.add (key e) (value e) m) Map.empty

// F#'s Map and Set order their keys by F#'s comparison, which for text is
// by UTF-16 unit; LawSpec orders text by code point. The two differ only for
// text (characters outside the Basic Multilingual Plane sort after U+E000 to
// U+FFFF by code point, before them by UTF-16 unit), so a key type holding
// text is re-sorted whenever its order is observed.

/// A Set's items in the portable order.
[<CompiledName("Items")>]
let items (s: Set<'a>) : 'a list =
    let xs = Set.toList s

    if Portable.reorders typeof<'a> then
        List.sortWith compare xs
    else
        xs

let private ordered (m: Map<'k, 'v>) : ('k * 'v) list =
    let xs = Map.toList m

    if Portable.reorders typeof<'k> then
        List.sortWith (fun (a, _) (b, _) -> compare a b) xs
    else
        xs

[<CompiledName("Keys")>]
let keys (m: Map<'k, 'v>) : 'k list = ordered m |> List.map fst

[<CompiledName("Values")>]
let values (m: Map<'k, 'v>) : 'v list = ordered m |> List.map snd

[<CompiledName("Entries")>]
let entries (entry: 'k -> 'v -> 'e) (m: Map<'k, 'v>) : 'e list =
    ordered m |> List.map (fun (k, v) -> entry k v)

// Failures across the typed boundary.

/// attempt: Right of the body's value, or Left of the failure raised with
/// this ability, converted to its native type.
[<CompiledName("Attempt")>]
let attempt (ability: string) (failure: Value -> 'e) (body: unit -> 'a) : Either<'e, 'a> =
    try
        Either.Right(body ())
    with :? RaisedFailure as f when f.Ability = ability ->
        Either.Left(failure f.Failure)

/// A typed handler's operation as its native Result: Ok of the body's value,
/// or Error of the failure raised with this ability, converted to its native
/// type.
[<CompiledName("TryResult")>]
let tryResult (ability: string) (failure: Value -> 'e) (body: unit -> 'a) : Result<'a, 'e> =
    try
        Ok(body ())
    with :? RaisedFailure as f when f.Ability = ability ->
        Error(failure f.Failure)

/// An operation's Result, its error raised as the failure the dynamic code raises.
[<CompiledName("Performed")>]
let performed (ability: string) (failure: 'e -> Value) (result: Result<'a, 'e>) : 'a =
    match result with
    | Ok value -> value
    | Error error -> raise (RaisedFailure(ability, failure error))
