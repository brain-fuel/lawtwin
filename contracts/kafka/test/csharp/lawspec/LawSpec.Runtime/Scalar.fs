/// LawSpec's scalar domain on .NET. Exact arithmetic never goes through a
/// native float, and IEEE values are rounded once, from an exact ratio, to
/// nearest with ties to even. Every generated law and dynamic definition uses
/// these operations, so F# and C# agree with each other and with the other
/// targets. ref:DEC-portable-exact-arithmetic ref:ieee-754
/// ref:DEC-explicit-machine-profile
module LawSpec.Runtime.Scalar

open System
open System.Collections.Concurrent
open System.Numerics
open System.Text

let private fail code detail : 'a = Fail.raiseWith code detail

// ---------------------------------------------------------------------------
// Type keys

let private tokenize (text: string) =
    let tokens = ResizeArray<string>()
    let word = StringBuilder()
    let flush () =
        if word.Length > 0 then
            tokens.Add(word.ToString())
            word.Clear() |> ignore
    for c in text do
        match c with
        | '(' | ')' ->
            flush ()
            tokens.Add(string c)
        | ' ' | '\t' -> flush ()
        | c -> word.Append(c) |> ignore
    flush ()
    List.ofSeq tokens

let rec private application parts =
    match parts with
    | [ Named(name, []) ] -> Named(name, [])
    | Named(name, []) :: args when List.contains name [ "List"; "Maybe"; "Optional"; "Nullable" ] ->
        Named(name, [ application args ])
    | Named(name, []) :: args -> Named(name, args)
    | [ t ] -> t
    | _ -> fail "InvalidTypeKey" ""

let rec private parts tokens acc =
    match tokens with
    | [] -> List.rev acc, []
    | ")" :: _ -> List.rev acc, tokens
    | "(" :: rest ->
        let inner, after = parts rest []
        match after with
        | ")" :: next -> parts next (application inner :: acc)
        | _ -> fail "InvalidTypeKey" ""
    | word :: rest when word.StartsWith "$" -> parts rest (Parameter(int (word.Substring 1)) :: acc)
    | word :: rest -> parts rest (Named(word, []) :: acc)

let private typeCache = ConcurrentDictionary<string, TypeRef>()

/// The type a key names: `Either (Int8) (Text)`, `List Int8`, `Maybe (List Text)`.
/// Parentheses keep nested Either and presence arguments apart.
[<CompiledName("TypeOf")>]
let typeOf (key: string) : TypeRef =
    typeCache.GetOrAdd(key, fun key ->
        let found, rest = parts (tokenize key) []
        if not (List.isEmpty rest) then fail "InvalidTypeKey" key
        application found)

/// The key of a type, as typeOf reads it.
[<CompiledName("TypeKey")>]
let rec typeKey (t: TypeRef) =
    match t with
    | Named(name, []) -> name
    | Named(name, args) -> name + String.Join("", args |> List.map (fun a -> " (" + typeKey a + ")"))
    | Parameter i -> sprintf "$%d" i

let private nameOf (t: TypeRef) =
    match t with
    | Named(name, _) -> name
    | Parameter _ -> ""

// ---------------------------------------------------------------------------
// Integers

let private integerTypes =
    set [ "Int8"; "Int16"; "Int32"; "Int64"; "UInt8"; "UInt16"; "UInt32"; "UInt64"
          "IntSize"; "UIntSize"; "UIntPtr"; "Integer"; "BigInt"; "BigUInt" ]

/// Whether a scalar name is one of LawSpec's integer types.
[<CompiledName("IsIntegerType")>]
let isIntegerType name = integerTypes.Contains name

let private pow2 (n: int) = BigInteger.One <<< n

let private signed (width: int) = Some(-(pow2 (width - 1))), Some(pow2 (width - 1) - BigInteger.One)

/// An integer type's inclusive bounds under a machine profile (32 or 64 bits).
[<CompiledName("Bounds")>]
let bounds (name: string) (bits: int) : BigInteger option * BigInteger option =
    if bits <> 32 && bits <> 64 then fail "InvalidMachineBits" (string bits)
    match name with
    | "Integer" | "BigInt" -> None, None
    | "BigUInt" -> Some BigInteger.Zero, None
    | "IntSize" -> signed bits
    | "UIntSize" | "UIntPtr" -> Some BigInteger.Zero, Some(pow2 bits - BigInteger.One)
    | _ when name.StartsWith "UInt" -> Some BigInteger.Zero, Some(pow2 (int (name.Substring 4)) - BigInteger.One)
    | _ when name.StartsWith "Int" -> signed (int (name.Substring 3))
    | _ -> fail "UnknownInteger" name

let private checkBounds (value: BigInteger) name bits =
    let lo, hi = bounds name bits
    let below = match lo with Some l -> value < l | None -> false
    let above = match hi with Some h -> value > h | None -> false
    if below || above then fail "IntegerOutOfRange" (sprintf "%s %O" name value)
    Int value

// ---------------------------------------------------------------------------
// Exact numbers

/// A normalized fraction.
[<CompiledName("Ratio")>]
let ratio (n: BigInteger) (d: BigInteger) =
    if d.IsZero then fail "InvalidRational" "zero denominator"
    let g = BigInteger.GreatestCommonDivisor(n, d)
    let s = if d.Sign < 0 then BigInteger.MinusOne else BigInteger.One
    Ratio(s * n / g, BigInteger.Abs d / g)

let private pow10 (n: int) = BigInteger.Pow(BigInteger(10), n)

let private normalized n d =
    match ratio n d with
    | Ratio(a, b) -> a, b
    | _ -> failwith "unreachable"

// IEEE layouts: fraction bits, exponent bits, bias.
let private format width =
    match width with
    | 32 -> 23, 8, 127
    | 64 -> 52, 11, 1023
    | _ -> fail "InvalidIeeeWidth" (string width)

type private Decoded =
    | Finite of sign: int * mantissa: BigInteger * exponent: int
    | Infinity of sign: int
    | NaN

let private decode width (bits: uint64) =
    let p, e, bias = format width
    let sign = int (bits >>> (width - 1)) &&& 1
    let exponent = int ((bits >>> p) &&& ((1UL <<< e) - 1UL))
    let fraction = bits &&& ((1UL <<< p) - 1UL)
    let top = (1 <<< e) - 1
    if exponent = top then (if fraction = 0UL then Infinity sign else NaN)
    elif exponent = 0 then Finite(sign, BigInteger(fraction), 1 - bias - p)
    else Finite(sign, BigInteger((1UL <<< p) + fraction), exponent - bias - p)

/// The exact ratio of an exact number or a finite float.
[<CompiledName("Exact")>]
let exact (value: Value) : BigInteger * BigInteger =
    match value with
    | Int n -> n, BigInteger.One
    | Ratio(n, d) -> n, d
    | Decimal(c, _) when c.IsZero -> BigInteger.Zero, BigInteger.One
    | Decimal(c, e) when e >= 0 -> c * pow10 e, BigInteger.One
    | Decimal(c, e) -> normalized c (pow10 -e)
    | Float(w, b) ->
        match decode w b with
        | Finite(sign, m, e) ->
            let signedM = if sign = 0 then m else -m
            if e >= 0 then signedM <<< e, BigInteger.One else normalized signedM (pow2 -e)
        | _ -> fail "NonFiniteExactConversion" ""
    | _ -> fail "ExactNumberRequired" (sprintf "%A" value)

let private isExact value =
    match value with
    | Int _ | Ratio _ | Decimal _ -> true
    | _ -> false

let rec private factors (n: BigInteger) (factor: BigInteger) count =
    if (n % factor).IsZero then factors (n / factor) factor (count + 1) else n, count

/// A decimal for an exact ratio, when its expansion terminates in base ten.
let private finiteDecimal n d =
    let rn, rd = normalized n d
    let d2, twos = factors rd (BigInteger 2) 0
    let d5, fives = factors d2 (BigInteger 5) 0
    if not d5.IsOne then fail "NonTerminatingDecimal" ""
    let scale = max twos fives
    Value.Decimal(rn * BigInteger.Pow(BigInteger 2, scale - twos) * BigInteger.Pow(BigInteger 5, scale - fives), -scale)

// ---------------------------------------------------------------------------
// IEEE floats

let private floatValue width (bits: uint64) =
    if width = 32 && bits > 0xFFFFFFFFUL then fail "InvalidIeeeBits" ""
    Float(width, bits)

let private zero width sign = Float(width, uint64 sign <<< (width - 1))

let private infinity width sign =
    let p, e, _ = format width
    Float(width, (uint64 sign <<< (width - 1)) ||| (((1UL <<< e) - 1UL) <<< p))

let private nan width =
    let p, _, _ = format width
    match infinity width 0 with
    | Float(_, inf) -> Float(width, inf ||| (1UL <<< (p - 1)))
    | _ -> failwith "unreachable"

let private floorLog2 (n: BigInteger) (d: BigInteger) =
    let guess = int (n.GetBitLength()) - int (d.GetBitLength())
    let below = if guess >= 0 then n < (d <<< guess) else (n <<< -guess) < d
    if below then guess - 1 else guess

let private roundRatio (n: BigInteger) (d: BigInteger) =
    let q = BigInteger.Divide(n, d)
    let r = n - q * d
    if BigInteger 2 * r > d || (BigInteger 2 * r = d && not q.IsEven) then q + BigInteger.One else q

/// The float of a width nearest an exact ratio; a zero takes the given sign.
[<CompiledName("FloatFromRatio")>]
let floatFromRatio width (n: BigInteger) (d: BigInteger) zeroSign =
    if d.IsZero then fail "ZeroDenominator" ""
    if n.IsZero then zero width zeroSign else
    let p, _, bias = format width
    let s = if (n.Sign < 0) <> (d.Sign < 0) then 1 else 0
    let an, ad = BigInteger.Abs n, BigInteger.Abs d
    let exponent = max (1 - bias) (floorLog2 an ad)
    let shift = p - exponent
    let mantissa = if shift >= 0 then roundRatio (an <<< shift) ad else roundRatio an (ad <<< -shift)
    let m, e = if mantissa >= pow2 (p + 1) then mantissa >>> 1, exponent + 1 else mantissa, exponent
    if e > bias then infinity width s
    else
        let encoded = if m < pow2 p then 0UL else uint64 (e + bias)
        let fraction = uint64 (m &&& (pow2 p - BigInteger.One))
        Float(width, (uint64 s <<< (width - 1)) ||| (encoded <<< p) ||| fraction)

/// A float's class: "nan", "infinity", "zero" or "finite".
[<CompiledName("FloatClass")>]
let floatClass value =
    match value with
    | Float(w, b) ->
        match decode w b with
        | NaN -> "nan"
        | Infinity _ -> "infinity"
        | Finite(_, m, _) when m.IsZero -> "zero"
        | Finite _ -> "finite"
    | _ -> fail "FloatRequired" ""

let private floatSign value =
    match value with
    | Float(w, b) -> int (b >>> (w - 1)) &&& 1
    | _ -> 0

/// A value as a float of a width: exact numbers round once, floats keep NaN
/// and infinity and otherwise round their exact value.
[<CompiledName("ToFloat")>]
let toFloat value width =
    match value with
    | Float(w, _) when w = width -> value
    | Float(w, b) ->
        match decode w b with
        | NaN -> nan width
        | Infinity s -> infinity width s
        | _ ->
            let n, d = exact value
            floatFromRatio width n d (floatSign value)
    | _ ->
        let n, d = exact value
        floatFromRatio width n d 0

/// A .NET double as a float of a width.
[<CompiledName("FloatFromNative")>]
let floatFromNative width (value: float) =
    toFloat (Float(64, uint64 (BitConverter.DoubleToInt64Bits value))) width

/// A float32 or float64's bits as LawSpec's float of that width.
[<CompiledName("FloatFromSingle")>]
let floatFromSingle (value: float32) =
    Float(32, uint64 (BitConverter.SingleToUInt32Bits value))

[<CompiledName("FloatFromDouble")>]
let floatFromDouble (value: float) =
    Float(64, BitConverter.DoubleToUInt64Bits value)

[<CompiledName("ToSingle")>]
let toSingle value =
    match toFloat value 32 with
    | Float(_, b) -> BitConverter.UInt32BitsToSingle(uint32 b)
    | _ -> failwith "unreachable"

[<CompiledName("ToDouble")>]
let toDouble value =
    match toFloat value 64 with
    | Float(_, b) -> BitConverter.UInt64BitsToDouble b
    | _ -> failwith "unreachable"

let private negateFloat value =
    match value with
    | Float(w, b) -> Float(w, b ^^^ (1UL <<< (w - 1)))
    | _ -> value

let rec private floatOp op width a b =
    let ca, cb = floatClass a, floatClass b
    match op with
    | _ when ca = "nan" || cb = "nan" -> nan width
    | "-" -> floatOp "+" width a (negateFloat b)
    | "+" when ca = "infinity" && cb = "infinity" -> if floatSign a = floatSign b then a else nan width
    | "+" when ca = "infinity" -> a
    | "+" when cb = "infinity" -> b
    | "*" when (ca = "infinity" && cb = "zero") || (ca = "zero" && cb = "infinity") -> nan width
    | "*" when ca = "infinity" || cb = "infinity" -> infinity width (floatSign a ^^^ floatSign b)
    | "/" when ca = "infinity" && cb = "infinity" -> nan width
    | "/" when ca = "zero" && cb = "zero" -> nan width
    | "/" when ca = "infinity" -> infinity width (floatSign a ^^^ floatSign b)
    | "/" when cb = "zero" -> infinity width (floatSign a ^^^ floatSign b)
    | "/" when cb = "infinity" -> zero width (floatSign a ^^^ floatSign b)
    | _ ->
        let an, ad = exact a
        let bn, bd = exact b
        let n, d, zeroSign =
            match op with
            | "+" -> an * bd + bn * ad, ad * bd, floatSign a &&& floatSign b
            | "*" -> an * bn, ad * bd, floatSign a ^^^ floatSign b
            | "/" -> an * bd, ad * bn, floatSign a ^^^ floatSign b
            | _ -> fail "UnknownFloatOperation" op
        floatFromRatio width n d zeroSign

/// IEEE arithmetic at the wider of the two widths.
[<CompiledName("FloatBinary")>]
let floatBinary op a b =
    match a, b with
    | Float(wa, _), Float(wb, _) ->
        let w = max wa wb
        floatOp op w (toFloat a w) (toFloat b w)
    | _ -> fail "FloatRequired" op

let private order (a: BigInteger) (b: BigInteger) = compare a b |> sign

let private compareRatios (a: BigInteger, b: BigInteger) (c: BigInteger, d: BigInteger) = order (a * d) (c * b)

/// The order of two floats, or None when one is NaN.
[<CompiledName("FloatCompare")>]
let floatCompare a b : int option =
    match a, b with
    | Float(wa, ba), Float(wb, bb) ->
        match decode wa ba, decode wb bb with
        | NaN, _ | _, NaN -> None
        | Infinity s, Infinity t when s = t -> Some 0
        | Infinity 1, _ -> Some -1
        | _, Infinity 1 -> Some 1
        | Infinity _, _ -> Some 1
        | _, Infinity _ -> Some -1
        | _ -> Some(compareRatios (exact a) (exact b))
    | _ -> fail "FloatRequired" ""

let private toComplex value width =
    match value with
    | Value.Complex(_, r, i) -> Value.Complex(width, toFloat r width, toFloat i width)
    | _ -> Value.Complex(width, toFloat value width, zero width 0)

// ---------------------------------------------------------------------------
// Text

/// Whether a string is well-formed UTF-16: no lone surrogate.
[<CompiledName("IsWellFormed")>]
let isWellFormed (text: string) =
    let mutable i = 0
    let mutable ok = true
    while ok && i < text.Length do
        let c = text.[i]
        if Char.IsHighSurrogate c then
            if i + 1 < text.Length && Char.IsLowSurrogate text.[i + 1] then i <- i + 2 else ok <- false
        elif Char.IsLowSurrogate c then ok <- false
        else i <- i + 1
    ok

/// A text's Unicode scalar values.
[<CompiledName("CodePoints")>]
let codePoints (text: string) : int list =
    [ for rune in text.EnumerateRunes() -> rune.Value ]

let private validUnit kind (n: int) =
    match kind with
    | "Bytes" -> n >= 0 && n <= 255
    | "CodeUnit16" | "Utf16Text" -> n >= 0 && n <= 65535
    | "CodePoint" | "CodePointText" -> n >= 0 && n <= 0x10FFFF
    | _ -> n >= 0 && n <= 0x10FFFF && (n < 0xD800 || n > 0xDFFF)

let private checkedUnit kind n =
    if not (validUnit kind n) then fail "InvalidCharacter" kind
    n

/// The text of Unicode scalar values.
[<CompiledName("TextOf")>]
let textOfPoints (points: int seq) =
    let builder = StringBuilder()
    for p in points do
        builder.Append(Rune(checkedUnit "Text" p).ToString()) |> ignore
    builder.ToString()

/// Compare two well-formed strings by code point, which is UTF-8's byte order
/// and LawSpec's text order (String.CompareOrdinal orders by UTF-16 units).
[<CompiledName("CompareText")>]
let compareText (a: string) (b: string) =
    let fix (c: char) =
        let n = int c
        if n >= 0xD800 then (if n >= 0xE000 then n - 0x800 else n + 0x2000) else n
    let n = min a.Length b.Length
    let mutable i = 0
    let mutable result = 0
    while result = 0 && i < n do
        result <- compare (fix a.[i]) (fix b.[i])
        i <- i + 1
    if result <> 0 then sign result else compare a.Length b.Length

/// Compare two byte arrays as unsigned octets.
[<CompiledName("CompareBytes")>]
let compareBytes (a: byte[]) (b: byte[]) =
    let c = ReadOnlySpan<byte>(a).SequenceCompareTo(ReadOnlySpan<byte>(b))
    sign c

// ---------------------------------------------------------------------------
// Validation and conversion

let private floatType width = if width = 32 then "Float32" else "Float64"

/// Check that a value belongs to a scalar or built-in container type under a
/// machine profile, and return it.
[<CompiledName("Validate")>]
let rec validate (value: Value) (t: TypeRef) (bits: int) : Value =
    match t, value with
    | Named("Bool", []), Bool _ -> value
    | Named("Text", []), Text s -> (if not (isWellFormed s) then fail "InvalidUnicode" ""); value
    | Named("Bytes", []), Bytes _ -> value
    | Named(("Char" | "CodePoint" | "CodeUnit16") as kind, []), Int n ->
        if n < BigInteger.Zero || n > BigInteger(0x10FFFF) then fail "InvalidCharacter" kind
        checkedUnit kind (int n) |> ignore
        value
    | Named(("CodePointText" | "Utf16Text") as kind, []), Raw(k, units) when k = kind ->
        units |> List.iter (checkedUnit kind >> ignore)
        value
    | Named("Decimal", []), Decimal _ -> value
    | Named("Rational", []), Ratio(n, d) -> ratio n d
    | Named("Float32", []), Float(32, _) -> value
    | Named("Float64", []), Float(64, _) -> value
    | Named("Complex64", []), Value.Complex(32, r, i) ->
        Value.Complex(32, validate r (Named("Float32", [])) bits, validate i (Named("Float32", [])) bits)
    | Named("Complex128", []), Value.Complex(64, r, i) ->
        Value.Complex(64, validate r (Named("Float64", [])) bits, validate i (Named("Float64", [])) bits)
    | Named("Symbol", []), Symbol _ -> value
    | Named("Unit", []), Unit -> value
    | Named("Null", []), Null -> value
    | Named("Undefined", []), Undefined -> value
    | Named(name, []), Int n when isIntegerType name -> checkBounds n name bits
    | Named("List", [ element ]), List items -> List(items |> List.map (fun v -> validate v element bits))
    | Named("Maybe", [ _ ]), Data("Maybe::Nothing", []) -> value
    | Named("Maybe", [ inner ]), Data("Maybe::Just", [ v ]) -> Data("Maybe::Just", [ validate v inner bits ])
    | Named("Either", [ l; _ ]), Data("Either::Left", [ v ]) -> Data("Either::Left", [ validate v l bits ])
    | Named("Either", [ _; r ]), Data("Either::Right", [ v ]) -> Data("Either::Right", [ validate v r bits ])
    | Named(("Nullable" | "Optional") as kind, [ inner ]), Presence(k, present) when k = kind ->
        Presence(kind, present |> Option.map (fun v -> validate v inner bits))
    | _ -> fail "InvalidValue" (typeKey t)

/// An explicit conversion (`x :: T`): exact numbers convert exactly or fail,
/// floats round once, and anything else is validated.
[<CompiledName("Convert")>]
let rec convert (value: Value) (t: TypeRef) (bits: int) : Value =
    match t with
    | Named("Rational", []) -> exact value ||> ratio
    | Named("Decimal", []) -> exact value ||> finiteDecimal
    | Named("Float32", []) -> toFloat value 32
    | Named("Float64", []) -> toFloat value 64
    | Named("Complex64", []) -> toComplex value 32
    | Named("Complex128", []) -> toComplex value 64
    | Named(name, []) when isIntegerType name ->
        let n, d = exact value
        if not (n % d).IsZero then fail "FractionalConversion" name
        checkBounds (n / d) name bits
    | Named("Nullable", [ _ ]) when value = Null -> Presence("Nullable", None)
    | Named("Optional", [ _ ]) when value = Undefined -> Presence("Optional", None)
    | Named(("Nullable" | "Optional") as kind, [ inner ]) ->
        match value with
        | Presence(k, present) when k = kind -> Presence(kind, present |> Option.map (fun v -> convert v inner bits))
        | _ -> validate value t bits
    | _ -> validate value t bits

// ---------------------------------------------------------------------------
// Equality, order and arithmetic

/// LawSpec equality: structural, exact across exact numbers, and false for a
/// NaN, even inside a container.
[<CompiledName("Equal")>]
let rec equal (a: Value) (b: Value) : bool =
    match a, b with
    | Float _, Float _ -> floatCompare a b = Some 0
    | Value.Complex(_, r1, i1), Value.Complex(_, r2, i2) -> equal r1 r2 && equal i1 i2
    | Value.Complex(w, r, i), Float _ -> equal r b && equal i (zero w 0)
    | Float _, Complex _ -> equal b a
    | Symbol(x, _), Symbol(y, _) -> Object.ReferenceEquals(x, y) || x.Equals y
    | Presence(k1, Some x), Presence(k2, Some y) -> k1 = k2 && equal x y
    | Presence(k1, None), Presence(k2, None) -> k1 = k2
    | Data("lawspec.collections::type::EqSet::EqSetItems", [ List x ]), Data("lawspec.collections::type::EqSet::EqSetItems", [ List y ]) -> sameMembers x y
    | Data(t1, f1), Data(t2, f2) -> t1 = t2 && equalLists f1 f2
    | List x, List y -> equalLists x y
    | _ when isExact a && isExact b -> compareRatios (exact a) (exact b) = 0
    | Bytes x, Bytes y -> compareBytes x y = 0
    | Raw(k1, u1), Raw(k2, u2) -> k1 = k2 && u1 = u2
    | Handle(n1, i1), Handle(n2, i2) -> n1 = n2 && (Object.ReferenceEquals(i1, i2) || i1.Equals i2)
    | _ -> a = b

and private sameMembers xs ys =
    (xs |> List.forall (fun value -> ys |> List.exists (equal value))) &&
    (ys |> List.forall (fun value -> xs |> List.exists (equal value)))

and private equalLists xs ys =
    match xs, ys with
    | [], [] -> true
    | x :: xs, y :: ys -> equal x y && equalLists xs ys
    | _ -> false

[<CompiledName("DistinctMembers")>]
let distinctMembers (items: Value list) =
    let rec loop previous values =
        match values with
        | [] -> true
        | value :: rest -> not (previous |> List.exists (equal value)) && loop (value :: previous) rest
    loop [] items

/// LawSpec's portable total order of keys. Floats, symbols and distinct
/// handles have no portable order.
[<CompiledName("Compare")>]
let rec compareValues (a: Value) (b: Value) : int =
    match a, b with
    | Bool x, Bool y -> compare x y |> sign
    | Text x, Text y -> compareText x y
    | Bytes x, Bytes y -> compareBytes x y
    | Raw(k1, x), Raw(k2, y) when k1 = k2 -> compare x y |> sign
    | Presence(k1, None), Presence(k2, None) when k1 = k2 -> 0
    | Presence(k1, None), Presence(k2, Some _) when k1 = k2 -> -1
    | Presence(k1, Some _), Presence(k2, None) when k1 = k2 -> 1
    | Presence(k1, Some x), Presence(k2, Some y) when k1 = k2 -> compareValues x y
    | Data("Maybe::Nothing", []), Data("Maybe::Just", _) -> -1
    | Data("Maybe::Just", _), Data("Maybe::Nothing", []) -> 1
    | Data(t1, f1), Data(t2, f2) when t1 = t2 -> compareLists f1 f2
    | Data(t1, _), Data(t2, _) -> compareText t1 t2
    | List x, List y -> compareLists x y
    | Handle(n1, i1), Handle(n2, i2) when n1 = n2 && Object.ReferenceEquals(i1, i2) -> 0
    | Unit, Unit | Null, Null | Undefined, Undefined -> 0
    | _ when isExact a && isExact b -> compareRatios (exact a) (exact b)
    | _ -> fail "NoPortableOrder" ""

and private compareLists xs ys =
    match xs, ys with
    | [], [] -> 0
    | [], _ -> -1
    | _, [] -> 1
    | x :: xs, y :: ys ->
        match compareValues x y with
        | 0 -> compareLists xs ys
        | c -> c

/// A value's negation.
[<CompiledName("Negate")>]
let rec negate value =
    match value with
    | Int n -> Int(-n)
    | Ratio(n, d) -> ratio (-n) d
    | Decimal(c, e) -> Value.Decimal(-c, e)
    | Float _ -> negateFloat value
    | Value.Complex(w, r, i) -> Value.Complex(w, negate r, negate i)
    | _ -> fail "NumberRequired" "negate"

let private inexactWidth (key: string) =
    match key with
    | "Float32" | "Complex64" -> 32
    | "Float64" | "Complex128" -> 64
    | _ -> 0

let private isComplexType (key: string) = key = "Complex64" || key = "Complex128"

let private relation op (ordering: int option) =
    match ordering with
    | None -> false
    | Some n ->
        match op with
        | "<" -> n < 0
        | "<=" -> n <= 0
        | ">" -> n > 0
        | ">=" -> n >= 0
        | _ -> fail "UnknownRelation" op

let private exactBinary op a b (ta: string) (tb: string) =
    match op, a, b with
    | "pow", Int x, Int y when y.Sign >= 0 -> Int(BigInteger.Pow(x, int y))
    | "quot", Int x, Int y when not y.IsZero -> Int(BigInteger.Divide(x, y))
    | "rem", Int x, Int y when not y.IsZero -> Int(BigInteger.Remainder(x, y))
    | _ ->
        let an, ad = exact a
        let bn, bd = exact b
        let n, d =
            match op with
            | "+" -> an * bd + bn * ad, ad * bd
            | "-" -> an * bd - bn * ad, ad * bd
            | "*" -> an * bn, ad * bd
            | "/" -> an * bd, ad * bn
            | _ -> fail "InvalidExactOperation" op
        let r = ratio n d
        if op = "/" || ta = "Rational" || tb = "Rational" then r
        elif ta = "Decimal" || tb = "Decimal" then finiteDecimal n d
        else
            match r with
            | Ratio(i, _) -> Int i
            | _ -> r

let private complexBinary op a b =
    match a, b with
    | Value.Complex(w, x, y), Value.Complex(_, u, v) ->
        let f = floatBinary
        let r, i =
            match op with
            | "+" | "-" -> f op x u, f op y v
            | "*" -> f "-" (f "*" x u) (f "*" y v), f "+" (f "*" x v) (f "*" y u)
            | "/" ->
                let den = f "+" (f "*" u u) (f "*" v v)
                f "/" (f "+" (f "*" x u) (f "*" y v)) den, f "/" (f "-" (f "*" y u) (f "*" x v)) den
            | _ -> fail "UnknownComplexOperation" op
        Value.Complex(w, r, i)
    | _ -> fail "ComplexRequired" op

/// A binary operator on two values of the given types (by key).
[<CompiledName("Binary")>]
let binary (op: string) (a: Value) (b: Value) (ta: string) (tb: string) : Value =
    match op with
    | "==" -> Bool(equal a b)
    | "!=" -> Bool(not (equal a b))
    | "<" | "<=" | ">" | ">=" ->
        let ordering =
            if inexactWidth ta > 0 || inexactWidth tb > 0 then floatCompare a b
            else Some(compareValues a b)
        Bool(relation op ordering)
    | _ ->
        if (inexactWidth ta > 0) <> (inexactWidth tb > 0) then fail "ExactInexactMixing" op
        match max (inexactWidth ta) (inexactWidth tb) with
        | 0 -> exactBinary op a b ta tb
        | width when isComplexType ta || isComplexType tb -> complexBinary op (toComplex a width) (toComplex b width)
        | _ -> floatBinary op a b

// ---------------------------------------------------------------------------
// Helpers

let private sequenceItems value (kind: string) =
    match value with
    | List items -> items
    | Text s when kind = "Text" -> codePoints s |> List.map (BigInteger >> Int)
    | Text s -> Encoding.UTF8.GetBytes s |> Seq.map (int >> BigInteger >> Int) |> List.ofSeq
    | Bytes b -> b |> Seq.map (int >> BigInteger >> Int) |> List.ofSeq
    | _ -> fail "SequenceRequired" kind

let private sequenceLike value (kind: string) (items: Value list) =
    // Only text and bytes are made of integers; a list keeps its items.
    let ints () = items |> List.map (function Int n -> int n | _ -> fail "SequenceRequired" kind)
    match value with
    | List _ -> List items
    | Text _ when kind = "Text" -> Text(textOfPoints (ints ()))
    | _ -> Bytes(ints () |> List.map byte |> Array.ofList)

let private bool value =
    match value with
    | Bool b -> b
    | _ -> fail "BoolRequired" ""

let private integer value =
    match value with
    | Int n -> n
    | _ -> fail "IntegerRequired" ""

let private text value =
    match value with
    | Text s -> s
    | _ -> fail "TextRequired" ""

let private clamp (n: BigInteger) (lo: BigInteger) (hi: BigInteger) = if n < lo then lo elif n > hi then hi else n

/// A built-in helper by name, with its arguments and their type keys.
[<CompiledName("Helper")>]
let helper (name: string) (values: Value list) (types: string list) (bits: int) : Value =
    match name, values with
    | "isNaN", [ f ] -> Bool(floatClass f = "nan")
    | "isInfinite", [ f ] -> Bool(floatClass f = "infinity")
    | "isFinite", [ f ] -> Bool(let c = floatClass f in c = "finite" || c = "zero")
    | "isNegativeZero", [ f ] -> Bool(floatClass f = "zero" && floatSign f = 1)
    | "length", [ List items ] -> Int(BigInteger(List.length items))
    | "length", [ Bytes b ] -> Int(BigInteger b.Length)
    | "length", [ Text s ] -> Int(BigInteger(List.length (codePoints s)))
    | "length", [ Raw(_, units) ] -> Int(BigInteger(List.length units))
    | ("real"), [ Value.Complex(_, r, _) ] -> r
    | ("imag" | "imaginary"), [ Value.Complex(_, _, i) ] -> i
    | "conjugate", [ Value.Complex(w, r, i) ] -> Value.Complex(w, r, negate i)
    | "abs", [ v ] when isExact v -> if fst (exact v) < BigInteger.Zero then negate v else v
    | "abs", [ Float(w, b) ] -> Float(w, b &&& ((1UL <<< (w - 1)) - 1UL))
    | "round", [ v; Int places ] ->
        let n, d = exact v
        let places = int places
        let sn, sd = if places >= 0 then n * pow10 places, d else n, d * pow10 -places
        let q = roundRatio (BigInteger.Abs sn) sd
        Value.Decimal((if sn.Sign < 0 then -q else q), -places)
    | "startsWith", [ Text t; Text p ] -> Bool(t.StartsWith(p, StringComparison.Ordinal))
    | "endsWith", [ Text t; Text s ] -> Bool(t.EndsWith(s, StringComparison.Ordinal))
    | "textContains", [ Text t; Text p ] -> Bool(p = "" || t.Contains(p, StringComparison.Ordinal))
    | "checked", [ _ ] -> Bool true
    | "select", [ Bool c; a; b ] -> if c then a else b
    | "compare", [ a; b ] ->
        let tag =
            match compareValues a b with
            | -1 -> "Less"
            | 0 -> "Equal"
            | _ -> "Greater"
        Data("lawspec.collections::type::Ordering::" + tag, [])
    | "isPresent", [ Presence(_, p) ] -> Bool(Option.isSome p)
    | "presentValue", [ Presence(_, Some v) ] -> v
    | "presentValue", [ Presence(_, None) ] -> fail "AbsentPresenceValue" ""
    | "negate", [ v ] -> negate v
    | "append", [ List a; List b ] -> List(a @ b)
    | "append", [ Text a; Text b ] -> Text(a + b)
    | "append", [ Bytes a; Bytes b ] -> Bytes(Array.append a b)
    | "slice", [ x; Int from; Int until ] ->
        let kind = List.head types
        let items = sequenceItems x kind
        let count = BigInteger(List.length items)
        let i = clamp from BigInteger.Zero count
        let j = clamp until i count
        sequenceLike x kind (items |> List.skip (int i) |> List.take (int (j - i)))
    | "at", [ x; Int i ] ->
        let items = sequenceItems x (List.head types)
        if i.Sign >= 0 && i < BigInteger(List.length items) then Data("Maybe::Just", [ List.item (int i) items ])
        else Data("Maybe::Nothing", [])
    | "range", [ Int lo; Int hi ] ->
        List [ let mutable n = lo
               while n < hi do
                   yield Int n
                   n <- n + BigInteger.One ]
    | "octets", [ Bytes b ] -> List(b |> Seq.map (int >> BigInteger >> Int) |> List.ofSeq)
    | "bytesOf", [ List items ] -> Bytes(items |> List.map (integer >> int >> byte) |> Array.ofList)
    | "utf8", [ Text t ] -> Bytes(Encoding.UTF8.GetBytes t)
    | "textOf", [ Int n ] -> Text(n.ToString())
    | "join", [ Text separator; List parts ] -> Text(String.Join(separator, parts |> List.map text))
    | ("quot" | "rem" | "pow"), [ a; b ] -> binary name a b (List.item 0 types) (List.item 1 types)
    | _, [ v ] -> convert v (typeOf name) bits
    | _ -> fail "UnknownHelper" name

// ---------------------------------------------------------------------------
// Symbols

/// A fresh symbol with a description.
[<CompiledName("NewSymbol")>]
let newSymbol (description: string) = Symbol(obj (), description)

/// Literal symbols share identities within one context (a law's run).
type SymbolContext() =
    let symbols = ConcurrentDictionary<string, obj>()
    member _.Identity(id: string) = symbols.GetOrAdd(id, fun _ -> obj ())

/// A literal symbol, interned in its context.
[<CompiledName("Symbol")>]
let symbol (context: SymbolContext) (id: string) (description: string) = Symbol(context.Identity id, description)

// ---------------------------------------------------------------------------
// Literals, as the generated code writes them

[<CompiledName("Integer")>]
let integerLiteral (digits: string) = Int(BigInteger.Parse(digits, Globalization.CultureInfo.InvariantCulture))

[<CompiledName("DecimalOf")>]
let decimalOf (coefficient: string) (exponent: int) =
    Value.Decimal(BigInteger.Parse(coefficient, Globalization.CultureInfo.InvariantCulture), exponent)

[<CompiledName("RationalOf")>]
let rationalOf (numerator: string) (denominator: string) =
    ratio (BigInteger.Parse(numerator, Globalization.CultureInfo.InvariantCulture))
        (BigInteger.Parse(denominator, Globalization.CultureInfo.InvariantCulture))

[<CompiledName("FloatBits")>]
let floatBits (width: int) (hex: string) =
    floatValue width (UInt64.Parse(hex, Globalization.NumberStyles.HexNumber))

[<CompiledName("TextUnits")>]
let textUnits (points: int list) = Text(textOfPoints points)

[<CompiledName("BytesOf")>]
let bytesOf (units: int list) = Bytes(units |> List.map (checkedUnit "Bytes" >> byte) |> Array.ofList)

[<CompiledName("RawOf")>]
let rawOf (kind: string) (units: int list) = Raw(kind, units |> List.map (checkedUnit kind))
