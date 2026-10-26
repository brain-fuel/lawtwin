/// LawSpec's portable values, as the generated laws and the dynamic
/// definitions see them. Native F# and C# code never handles these directly:
/// generated codecs convert at the boundary to records, unions and the .NET
/// types the adapters use. ref:DEC-fsharp-runtime-from-scratch
/// ref:DEC-typed-core-boundary
namespace LawSpec.Runtime

open System
open System.Numerics

/// A value in LawSpec's portable domain. Exact numbers stay exact, IEEE
/// values keep their bits (so a NaN payload and a negative zero survive), and
/// data is a constructor tag with its fields in declaration order.
[<StructuralEquality; NoComparison>]
type Value =
    | Bool of bool
    | Int of BigInteger
    /// A normalized fraction: positive denominator, lowest terms.
    | Ratio of numerator: BigInteger * denominator: BigInteger
    /// coefficient * 10 ^ exponent, with the scale it was written with.
    | Decimal of coefficient: BigInteger * exponent: int
    | Float of width: int * bits: uint64
    | Complex of width: int * real: Value * imaginary: Value
    /// Unicode text; it never holds a lone surrogate.
    | Text of string
    | Bytes of byte[]
    /// CodePointText and Utf16Text: their units, without validation as text.
    | Raw of kind: string * units: int list
    | Unit
    | Null
    | Undefined
    /// Two symbols are equal when they share an identity.
    | Symbol of identity: obj * description: string
    | List of Value list
    | Data of tag: string * fields: Value list
    /// Nullable or Optional: absent, or present with a value.
    | Presence of kind: string * value: Value option
    /// A handle's identity crosses the native boundary unchanged.
    | Handle of name: string * identity: obj

/// A type as the runtime checks it: a name applied to arguments, or a data
/// declaration's parameter by position.
[<StructuralEquality; StructuralComparison>]
type TypeRef =
    | Named of name: string * arguments: TypeRef list
    | Parameter of index: int

/// A broken contract, an unmet law or a malformed value. The code names the
/// kind of failure (StrategyOutsideRefinement, EquationFailed, ...), so test
/// output names it the same way on every .NET target.
type LawSpecException(code: string, detail: string, context: string list, inner: exn) =
    inherit
        Exception(
            (match context with
             | [] -> ""
             | _ -> String.Join(": ", List.rev context) + ": ")
            + code
            + (if String.IsNullOrEmpty detail then "" else " " + detail),
            inner
        )

    new(code, detail) = LawSpecException(code, detail, [], null)
    member _.Code = code
    member _.Detail = detail
    member _.Context = context

    /// The same failure, inside one more definition or law.
    member this.Within(identity: string) =
        LawSpecException(code, detail, identity :: context, this.InnerException)

/// A failure an operation raised with `raise`, carrying its portable value
/// until an `attempt` catches it. ref:DEC-typed-native-definitions
type RaisedFailure(ability: string, failure: Value) =
    inherit Exception("LawSpec failure raised by " + ability + ": " + sprintf "%A" failure)
    member _.Ability = ability
    member _.Failure = failure

/// Helpers for raising LawSpec's failures.
module Fail =
    /// Raise a LawSpec failure with a code and detail.
    [<CompiledName("RaiseWith")>]
    let inline raiseWith code detail : 'a = raise (LawSpecException(code, detail))

    /// Fail unless the condition holds.
    [<CompiledName("Require")>]
    let inline require condition code detail =
        if not condition then
            raiseWith code detail
