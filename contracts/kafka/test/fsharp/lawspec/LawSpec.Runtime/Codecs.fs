/// Conversions between portable values and the native .NET types adapters
/// use. The generated code composes these for each adapter's signature, and
/// generates one for each data type, so no reflection is involved.
/// ref:DEC-typed-core-boundary ref:DEC-native-bindings-typed-identity
namespace LawSpec.Runtime

open System
open System.Numerics
open System.Text
open LawSpec

/// To and from a native type.
type Codec<'T> =
    { ToNative: Value -> 'T
      FromNative: 'T -> Value }

module Codec =
    let private wrong (expected: string) (value: Value) : 'a =
        Fail.raiseWith "InvalidNativeValue" (sprintf "expected %s, got %A" expected value)

    let private integer name (value: Value) =
        match value with
        | Int n -> n
        | _ -> wrong name value

    let inline private bounded name (convert: BigInteger -> 'T) (back: 'T -> BigInteger) : Codec<'T> =
        { ToNative = fun v -> convert (integer name v)
          FromNative = fun n -> Int(back n) }

    [<CompiledName("Int8")>]
    let int8 = bounded "Int8" sbyte BigInteger
    [<CompiledName("Int16")>]
    let int16 = bounded "Int16" int16 BigInteger
    [<CompiledName("Int32")>]
    let int32 = bounded "Int32" int32 (fun (n: int) -> BigInteger n)
    [<CompiledName("Int64")>]
    let int64 = bounded "Int64" int64 (fun (n: int64) -> BigInteger n)
    [<CompiledName("UInt8")>]
    let uint8 = bounded "UInt8" byte BigInteger
    [<CompiledName("UInt16")>]
    let uint16 = bounded "UInt16" uint16 BigInteger
    [<CompiledName("UInt32")>]
    let uint32 = bounded "UInt32" uint32 (fun (n: uint32) -> BigInteger n)
    [<CompiledName("UInt64")>]
    let uint64 = bounded "UInt64" uint64 (fun (n: uint64) -> BigInteger n)
    [<CompiledName("BigInteger")>]
    let bigint: Codec<BigInteger> = { ToNative = integer "Integer"; FromNative = Int }

    [<CompiledName("Bool")>]
    let bool: Codec<bool> =
        { ToNative = (function Bool b -> b | v -> wrong "Bool" v)
          FromNative = Bool }

    [<CompiledName("Unit")>]
    let unit: Codec<unit> =
        { ToNative = (function Unit -> () | v -> wrong "Unit" v)
          FromNative = fun () -> Unit }

    [<CompiledName("Null")>]
    let nullValue: Codec<Null> =
        { ToNative = (function Value.Null -> Null.Null | v -> wrong "Null" v)
          FromNative = fun _ -> Value.Null }

    [<CompiledName("Undefined")>]
    let undefinedValue: Codec<Undefined> =
        { ToNative = (function Value.Undefined -> Undefined.Undefined | v -> wrong "Undefined" v)
          FromNative = fun _ -> Value.Undefined }

    [<CompiledName("Text")>]
    let text: Codec<string> =
        { ToNative = (function Text s -> s | v -> wrong "Text" v)
          FromNative = fun s ->
              if isNull s then Fail.raiseWith "InvalidNativeValue" "a null string"
              Text s }

    /// Bytes cross as a copy, so neither side can change the other's.
    [<CompiledName("Bytes")>]
    let bytes: Codec<byte[]> =
        { ToNative = (function Bytes b -> Array.copy b | v -> wrong "Bytes" v)
          FromNative = fun b ->
              if isNull b then Fail.raiseWith "InvalidNativeValue" "a null byte array"
              Bytes(Array.copy b) }

    [<CompiledName("Char")>]
    let rune: Codec<Rune> =
        { ToNative = fun v -> Rune(int (integer "Char" v))
          FromNative = fun r -> Int(BigInteger r.Value) }

    [<CompiledName("CodePoint")>]
    let codePoint: Codec<int> = bounded "CodePoint" int (fun (n: int) -> BigInteger n)

    [<CompiledName("CodeUnit16")>]
    let codeUnit16: Codec<char> = bounded "CodeUnit16" (fun n -> char (int n)) (fun (c: char) -> BigInteger(int c))

    [<CompiledName("CodePointText")>]
    let codePointText: Codec<int list> =
        { ToNative = (function Raw(_, units) -> units | v -> wrong "CodePointText" v)
          FromNative = fun units -> Raw("CodePointText", units) }

    [<CompiledName("Utf16Text")>]
    let utf16Text: Codec<string> =
        { ToNative = (function Raw(_, units) -> String(units |> List.map char |> Array.ofList) | v -> wrong "Utf16Text" v)
          FromNative = fun s -> Raw("Utf16Text", s |> Seq.map int |> List.ofSeq) }

    [<CompiledName("Float32")>]
    let float32: Codec<float32> =
        { ToNative = (function Float(32, b) -> BitConverter.UInt32BitsToSingle(Operators.uint32 b) | v -> wrong "Float32" v)
          FromNative = Scalar.floatFromSingle }

    [<CompiledName("Float64")>]
    let float64: Codec<float> =
        { ToNative = (function Float(64, b) -> BitConverter.UInt64BitsToDouble b | v -> wrong "Float64" v)
          FromNative = Scalar.floatFromDouble }

    [<CompiledName("Complex64")>]
    let complex64: Codec<Complex64> =
        { ToNative = (function Complex(_, r, i) -> { Real = float32.ToNative r; Imaginary = float32.ToNative i } | v -> wrong "Complex64" v)
          FromNative = fun c -> Value.Complex(32, float32.FromNative c.Real, float32.FromNative c.Imaginary) }

    [<CompiledName("Complex128")>]
    let complex128: Codec<Complex> =
        { ToNative = (function Complex(_, r, i) -> Complex(float64.ToNative r, float64.ToNative i) | v -> wrong "Complex128" v)
          FromNative = fun c -> Value.Complex(64, float64.FromNative c.Real, float64.FromNative c.Imaginary) }

    [<CompiledName("Rational")>]
    let rational: Codec<Rational> =
        { ToNative = (function Ratio(n, d) -> Rational.Create(n, d) | v -> wrong "Rational" v)
          FromNative = fun r -> Scalar.ratio r.Numerator r.Denominator }

    [<CompiledName("Decimal")>]
    let exactDecimal: Codec<ExactDecimal> =
        { ToNative = (function Decimal(c, e) -> { Coefficient = c; Exponent = e } | v -> wrong "Decimal" v)
          FromNative = fun d -> Value.Decimal(d.Coefficient, d.Exponent) }

    [<CompiledName("Symbol")>]
    let symbol: Codec<Symbol> =
        { ToNative = (function Value.Symbol(id, d) -> Symbol(id, d) | v -> wrong "Symbol" v)
          FromNative = fun s ->
              if isNull s then Fail.raiseWith "InvalidNativeValue" "a null symbol"
              Value.Symbol(s.Identity, s.Description) }

    [<CompiledName("List")>]
    let list (item: Codec<'T>) : Codec<'T list> =
        { ToNative = (function List items -> items |> List.map item.ToNative | v -> wrong "List" v)
          FromNative = fun items -> List(items |> List.map item.FromNative) }

    [<CompiledName("Maybe")>]
    let option (item: Codec<'T>) : Codec<'T option> =
        { ToNative =
            (function
            | Data("Maybe::Absent", []) -> None
            | Data("Maybe::Present", [ v ]) -> Some(item.ToNative v)
            | v -> wrong "Maybe" v)
          FromNative =
            (function
            | None -> Data("Maybe::Absent", [])
            | Some v -> Data("Maybe::Present", [ item.FromNative v ])) }

    [<CompiledName("Either")>]
    let either (left: Codec<'L>) (right: Codec<'R>) : Codec<Either<'L, 'R>> =
        { ToNative =
            (function
            | Data("Either::Left", [ v ]) -> Either.Left(left.ToNative v)
            | Data("Either::Right", [ v ]) -> Either.Right(right.ToNative v)
            | v -> wrong "Either" v)
          FromNative =
            (function
            | Either.Left v -> Data("Either::Left", [ left.FromNative v ])
            | Either.Right v -> Data("Either::Right", [ right.FromNative v ])) }

    [<CompiledName("Nullable")>]
    let nullable (item: Codec<'T>) : Codec<Nullable<'T>> =
        { ToNative =
            (function
            | Presence(_, None) -> Nullable.Null
            | Presence(_, Some v) -> Nullable.Present(item.ToNative v)
            | v -> wrong "Nullable" v)
          FromNative =
            (function
            | Nullable.Null -> Presence("Nullable", None)
            | Nullable.Present v -> Presence("Nullable", Some(item.FromNative v))) }

    [<CompiledName("Optional")>]
    let optional (item: Codec<'T>) : Codec<Optional<'T>> =
        { ToNative =
            (function
            | Presence(_, None) -> Optional.Undefined
            | Presence(_, Some v) -> Optional.Present(item.ToNative v)
            | v -> wrong "Optional" v)
          FromNative =
            (function
            | Optional.Undefined -> Presence("Optional", None)
            | Optional.Present v -> Presence("Optional", Some(item.FromNative v))) }

    /// A handle's identity crosses unchanged.
    [<CompiledName("Handle")>]
    let handle<'T when 'T: not struct> (name: string) : Codec<'T> =
        { ToNative = (function Handle(_, identity) -> unbox<'T> identity | v -> wrong name v)
          FromNative = fun native ->
              if isNull (box native) then Fail.raiseWith "HandleRequiresIdentity" name
              Handle(name, box native) }

    /// The value itself, for a type with no native form.
    [<CompiledName("Portable")>]
    let portable: Codec<Value> = { ToNative = id; FromNative = id }

    /// A Set as F#'s persistent Set: canonical (sorted, distinct) in the
    /// portable order whatever F#'s own order of the items.
    [<CompiledName("Set")>]
    let set (item: Codec<'T>) : Codec<Set<'T>> =
        { ToNative =
            (function
            | Data(_, [ List items ]) -> items |> List.map item.ToNative |> Set.ofList
            | v -> wrong "Set" v)
          FromNative = fun s ->
              let items = s |> Seq.map item.FromNative |> List.ofSeq |> List.sortWith Scalar.compareValues
              Data("lawspec.collections::type::Set::SetItems", [ List items ]) }

    /// A KeyVal as F#'s persistent Map, its entries in the portable order of
    /// their keys.
    [<CompiledName("KeyVal")>]
    let map (key: Codec<'K>) (value: Codec<'V>) : Codec<Map<'K, 'V>> =
        { ToNative =
            (function
            | Data(_, [ List entries ]) ->
                entries
                |> List.map (function
                    | Data(_, [ k; v ]) -> key.ToNative k, value.ToNative v
                    | e -> wrong "Entry" e)
                |> Map.ofList
            | v -> wrong "KeyVal" v)
          FromNative = fun m ->
              let entries =
                  m |> Seq.map (fun (KeyValue(k, v)) -> key.FromNative k, value.FromNative v)
                  |> List.ofSeq |> List.sortWith (fun (a, _) (b, _) -> Scalar.compareValues a b)
                  |> List.map (fun (k, v) -> Data("lawspec.collections::type::Entry::Entry", [ k; v ]))
              Data("lawspec.collections::type::KeyVal::KeyValEntries", [ List entries ]) }

    /// A Duration as a TimeSpan of whole microseconds.
    [<CompiledName("Duration")>]
    let duration: Codec<TimeSpan> =
        { ToNative =
            (function
            | Data(_, [ Int micros ]) -> TimeSpan.FromTicks(Operators.int64 micros * 10L)
            | v -> wrong "Duration" v)
          FromNative = fun span ->
              if span.Ticks % 10L <> 0L then Fail.raiseWith "InvalidNativeValue" "a Duration is a whole number of microseconds"
              Data("lawspec.time::type::Duration::Duration", [ Int(BigInteger(span.Ticks / 10L)) ]) }
