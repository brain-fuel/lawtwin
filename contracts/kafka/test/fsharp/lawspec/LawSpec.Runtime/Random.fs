/// The portable SplitMix64 sequence. Its state is passed explicitly, so
/// concurrent callers never draw from one another's stream, and the same seed
/// replays the same run on every target. ref:splitmix
/// ref:DEC-portable-seeded-generation
module LawSpec.Runtime.Random

open System.Numerics

/// A SplitMix64 state.
[<Struct>]
type Seed = Seed of uint64

/// The state for a seed (any integer, reduced to 64 bits).
[<CompiledName("SeedOf")>]
let seed (n: BigInteger) =
    Seed(uint64 (n &&& BigInteger(System.UInt64.MaxValue)))

/// The next output and state.
[<CompiledName("Next")>]
let next (Seed state) =
    let s = state + 0x9e3779b97f4a7c15UL
    let a = (s ^^^ (s >>> 30)) * 0xbf58476d1ce4e5b9UL
    let b = (a ^^^ (a >>> 27)) * 0x94d049bb133111ebUL
    b ^^^ (b >>> 31), Seed s

/// A draw below a bound (0 for a bound that is not positive), as every
/// runtime reduces it: the output modulo the bound.
[<CompiledName("Below")>]
let below (bound: BigInteger) (state: Seed) =
    if bound.Sign <= 0 then
        BigInteger.Zero, state
    else
        let n, next' = next state
        BigInteger.Remainder(BigInteger n, bound), next'

/// Two independent states from one, for generators that branch.
[<CompiledName("Split")>]
let split (state: Seed) =
    let a, s1 = next state
    let b, _ = next s1
    Seed a, Seed b
