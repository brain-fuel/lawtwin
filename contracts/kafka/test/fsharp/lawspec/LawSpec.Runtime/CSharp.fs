/// The C# compatibility layer. C# programs run over this F# runtime and over
/// the program's generated F# model (its data types, codecs and ability
/// interfaces); where an F# idiom is awkward from C#, this layer gives the C#
/// form: functions from delegates (F# functions are curried FSharpFunc
/// values), F# lists, options and tuples from ordinary arguments, the unit
/// value, portable Value constructors, and the Maybe helpers C# adapters use.
/// Generated C# calls only these and the runtime's PascalCase compiled names.
/// ref:DEC-csharp-over-fsharp
namespace LawSpec.CSharp

open System
open System.Collections.Generic
open System.Numerics
open System.Runtime.CompilerServices
open Microsoft.FSharp.Core
open LawSpec.Runtime

/// F# functions, lists and tuples for C# (F#'s unit value is default(Unit)).
[<AbstractClass; Sealed>]
type Fn =
    static member F0(f: Func<'R>) : FSharpFunc<Unit, 'R> = FuncConvert.FromFunc f
    static member F1(f: Func<'A, 'R>) : FSharpFunc<'A, 'R> = FuncConvert.FromFunc f
    /// A curried F# function of two arguments.
    static member F2(f: Func<'A, 'B, 'R>) : FSharpFunc<'A, FSharpFunc<'B, 'R>> = fun a b -> f.Invoke(a, b)
    static member F3(f: Func<'A, 'B, 'C, 'R>) : FSharpFunc<'A, FSharpFunc<'B, FSharpFunc<'C, 'R>>> = fun a b c -> f.Invoke(a, b, c)
    static member F4(f: Func<'A, 'B, 'C, 'D, 'R>) : FSharpFunc<'A, FSharpFunc<'B, FSharpFunc<'C, FSharpFunc<'D, 'R>>>> =
        fun a b c d -> f.Invoke(a, b, c, d)
    static member F5(f: Func<'A, 'B, 'C, 'D, 'E, 'R>) : FSharpFunc<'A, FSharpFunc<'B, FSharpFunc<'C, FSharpFunc<'D, FSharpFunc<'E, 'R>>>>> =
        fun a b c d e -> f.Invoke(a, b, c, d, e)
    static member F6(f: Func<'A, 'B, 'C, 'D, 'E, 'G, 'R>) : FSharpFunc<'A, FSharpFunc<'B, FSharpFunc<'C, FSharpFunc<'D, FSharpFunc<'E, FSharpFunc<'G, 'R>>>>>> =
        fun a b c d e g -> f.Invoke(a, b, c, d, e, g)

    /// An F# function applied to its arguments, one at a time.
    static member Invoke(f: FSharpFunc<Unit, 'R>) : 'R = f (Unchecked.defaultof<Unit>)
    static member Invoke(f: FSharpFunc<'A, 'R>, a: 'A) : 'R = f a
    static member Invoke(f: FSharpFunc<'A, FSharpFunc<'B, 'R>>, a: 'A, b: 'B) : 'R = f a b
    static member Invoke(f: FSharpFunc<'A, FSharpFunc<'B, FSharpFunc<'C, 'R>>>, a: 'A, b: 'B, c: 'C) : 'R = f a b c

    /// An F# list of the arguments.
    static member List<'T>([<ParamArray>] items: 'T[]) : 'T list = List.ofArray items
    /// An F# list of a sequence.
    static member ListOf<'T>(items: IEnumerable<'T>) : 'T list = List.ofSeq items
    /// An F# tuple.
    static member Tuple(a: 'A, b: 'B) : 'A * 'B = (a, b)
    static member Tuple(a: 'A, b: 'B, c: 'C) : 'A * 'B * 'C = (a, b, c)
    /// The value of a block of statements (generated code binds one this way).
    static member Run(body: Func<'T>) : 'T = body.Invoke()
    /// An F# option holding value.
    static member Some(value: 'T) : 'T option = Some value
    /// The F# option holding nothing.
    static member None<'T>() : 'T option = None

/// Portable values for C#: Value's cases as methods.
[<AbstractClass; Sealed>]
type V =
    static member Bool(b: bool) = Value.Bool b
    static member Int(n: BigInteger) = Value.Int n
    static member Int(n: int64) = Value.Int(BigInteger n)
    static member Ratio(numerator: BigInteger, denominator: BigInteger) = Value.Ratio(numerator, denominator)
    static member Decimal(coefficient: BigInteger, exponent: int) = Value.Decimal(coefficient, exponent)
    static member Float(width: int, bits: uint64) = Value.Float(width, bits)
    static member Complex(width: int, real: Value, imaginary: Value) = Value.Complex(width, real, imaginary)
    static member Text(s: string) = Value.Text s
    static member Bytes(b: byte[]) = Value.Bytes b
    static member Raw(kind: string, units: int list) = Value.Raw(kind, units)
    static member Unit = Value.Unit
    static member Null = Value.Null
    static member Undefined = Value.Undefined
    static member Symbol(identity: obj, description: string) = Value.Symbol(identity, description)
    static member List(items: Value list) = Value.List items
    static member Data(tag: string, fields: Value list) = Value.Data(tag, fields)
    static member Presence(kind: string, value: Value option) = Value.Presence(kind, value)
    static member Handle(name: string, identity: obj) = Value.Handle(name, identity)

/// F# options from C#: whether one holds a value, and what it holds.
[<Extension; AbstractClass; Sealed>]
type Maybe =
    [<Extension>]
    static member IsSome(option: 'T option) = Option.isSome option
    [<Extension>]
    static member IsNone(option: 'T option) = Option.isNone option
    /// The value, or fallback when there is none.
    [<Extension>]
    static member GetOrElse(option: 'T option, fallback: 'T) = defaultArg option fallback
    /// One function of the value, or the other of nothing.
    [<Extension>]
    static member Match(option: 'T option, some: Func<'T, 'R>, none: Func<'R>) =
        match option with
        | Some v -> some.Invoke v
        | None -> none.Invoke()
    /// An option of a nullable reference.
    static member OfObj(value: 'T) : 'T option when 'T: null = Option.ofObj value
    static member Of(value: 'T) : 'T option = Some value
    static member Nothing<'T>() : 'T option = None

/// F# results from C#: a typed failure an adapter returns.
[<AbstractClass; Sealed>]
type Outcome =
    static member Ok<'T, 'E>(value: 'T) : Result<'T, 'E> = Ok value
    static member Error<'T, 'E>(error: 'E) : Result<'T, 'E> = Error error
