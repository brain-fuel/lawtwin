/// The .NET types LawSpec values take at the native boundary, where a .NET
/// type has no exact counterpart: exact fractions and decimals, single
/// precision complex numbers, the two kinds of presence, and symbols. Every
/// other LawSpec type maps to a .NET type directly (sbyte ... uint64, bigint,
/// string, byte[], list, option, Rune). ref:DEC-fsharp-runtime-from-scratch
namespace LawSpec

open System.Numerics

/// An exact fraction in lowest terms, with a positive denominator.
[<CustomEquality; CustomComparison>]
type Rational =
    private
        {
            N: BigInteger
            D: BigInteger
        }

    member this.Numerator = this.N
    member this.Denominator = this.D

    static member Create(numerator: BigInteger, denominator: BigInteger) =
        if denominator.IsZero then
            invalidArg (nameof denominator) "a rational's denominator is not zero"

        let g = BigInteger.GreatestCommonDivisor(numerator, denominator)

        let s =
            if denominator.Sign < 0 then
                BigInteger.MinusOne
            else
                BigInteger.One

        {
            N = s * numerator / g
            D = BigInteger.Abs denominator / g
        }

    override this.Equals other =
        match other with
        | :? Rational as r -> this.N = r.N && this.D = r.D
        | _ -> false

    override this.GetHashCode() = hash (this.N, this.D)
    override this.ToString() = sprintf "%O/%O" this.N this.D

    interface System.IComparable with
        member this.CompareTo other =
            match other with
            | :? Rational as r -> compare (this.N * r.D) (r.N * this.D)
            | _ -> invalidArg "other" "a rational compares with a rational"

/// coefficient * 10 ^ exponent, exactly; 1.50 keeps its two places.
[<Struct; CustomEquality; NoComparison>]
type ExactDecimal =
    {
        Coefficient: BigInteger
        Exponent: int
    }

    override this.Equals other =
        match other with
        | :? ExactDecimal as d -> this.Coefficient = d.Coefficient && this.Exponent = d.Exponent
        | _ -> false

    override this.GetHashCode() = hash (this.Coefficient, this.Exponent)

    override this.ToString() =
        sprintf "%Oe%d" this.Coefficient this.Exponent

    /// Both decimals at the smaller exponent.
    static member private Aligned(a: ExactDecimal, b: ExactDecimal) =
        let e = min a.Exponent b.Exponent

        let scale (d: ExactDecimal) =
            d.Coefficient * BigInteger.Pow(BigInteger 10, d.Exponent - e)

        scale a, scale b, e

    /// Exact sum, at the finer of the two scales.
    static member (+)(a: ExactDecimal, b: ExactDecimal) =
        let x, y, e = ExactDecimal.Aligned(a, b)
        { Coefficient = x + y; Exponent = e }

    /// Exact difference, at the finer of the two scales.
    static member (-)(a: ExactDecimal, b: ExactDecimal) =
        let x, y, e = ExactDecimal.Aligned(a, b)
        { Coefficient = x - y; Exponent = e }

    /// Exact product.
    static member (*)(a: ExactDecimal, b: ExactDecimal) =
        {
            Coefficient = a.Coefficient * b.Coefficient
            Exponent = a.Exponent + b.Exponent
        }

/// A complex number of two single precision floats (Complex128 is
/// System.Numerics.Complex).
[<Struct>]
type Complex64 = { Real: float32; Imaginary: float32 }

/// A value that may be null, distinct from an absent Optional.
[<RequireQualifiedAccess>]
type Nullable<'T> =
    | Null
    | Present of 'T

/// A value that may be undefined, distinct from a null Nullable.
[<RequireQualifiedAccess>]
type Optional<'T> =
    | Undefined
    | Present of 'T

/// Either of two values; Left conventionally the failure.
[<RequireQualifiedAccess>]
type Either<'L, 'R> =
    | Left of left: 'L
    | Right of right: 'R

/// LawSpec's Null value.
type Null = | Null

/// LawSpec's Undefined value.
type Undefined = | Undefined

/// A symbol: equal only to itself.
[<Sealed; AllowNullLiteral>]
type Symbol(identity: obj, description: string) =
    new(description) = Symbol(obj (), description)
    member _.Identity = identity
    member _.Description = description

    override _.Equals other =
        match other with
        | :? Symbol as s -> obj.ReferenceEquals(identity, s.Identity)
        | _ -> false

    override _.GetHashCode() =
        System.Runtime.CompilerServices.RuntimeHelpers.GetHashCode identity

    override _.ToString() = "Symbol(" + description + ")"
