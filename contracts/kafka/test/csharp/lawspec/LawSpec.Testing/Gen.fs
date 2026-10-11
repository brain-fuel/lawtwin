/// LawSpec's property engine for .NET: seeded SplitMix64 generation with
/// integrated shrinking. A generator yields a rose tree, the value with the
/// smaller values it may shrink to, so shrinking a bound input rebuilds the
/// inputs that depend on it and every candidate stays inside the domain.
/// ref:DEC-portable-seeded-generation ref:DEC-shrink-within-domain
/// ref:DEC-fsharp-runtime-from-scratch
namespace LawSpec.Testing

open System.Numerics
open LawSpec.Runtime
open LawSpec.Runtime.Random

/// A value and, lazily, the values it shrinks to, best first.
type Tree<'a> =
    { Value: 'a
      Children: seq<Tree<'a>> }

module Tree =
    [<CompiledName("Leaf")>]
    let leaf value = { Value = value; Children = Seq.empty }

    [<CompiledName("Map")>]
    let rec map (f: 'a -> 'b) (tree: Tree<'a>) =
        { Value = f tree.Value
          Children = tree.Children |> Seq.map (map f) }

    /// Keep only the candidates a predicate accepts, with their own candidates.
    [<CompiledName("Filter")>]
    let rec filter (keep: 'a -> bool) (tree: Tree<'a>) =
        { tree with Children = tree.Children |> Seq.filter (fun t -> keep t.Value) |> Seq.map (filter keep) }

    /// Candidates from a function of a value, unfolded.
    [<CompiledName("Unfold")>]
    let rec unfold (shrink: 'a -> seq<'a>) (value: 'a) =
        { Value = value
          Children = shrink value |> Seq.map (unfold shrink) }

/// A generator: a seed and a size to a tree, or None for an empty domain,
/// which discards the whole tuple of inputs.
type Gen<'a> = Gen of (Seed -> int -> Tree<'a> option)

module Gen =
    [<CompiledName("Run")>]
    let run (Gen g) seed size = g seed size

    [<CompiledName("Constant")>]
    let constant value = Gen(fun _ _ -> Some(Tree.leaf value))

    /// An empty domain: the case is discarded and redrawn.
    [<CompiledName("Empty")>]
    let empty<'a> : Gen<'a> = Gen(fun _ _ -> None)

    [<CompiledName("Map")>]
    let map f (Gen g) = Gen(fun seed size -> g seed size |> Option.map (Tree.map f))

    let rec private bindTree (tree: Tree<'a>) (k: 'a -> Tree<'b> option) : Tree<'b> option =
        k tree.Value
        |> Option.map (fun inner ->
            { Value = inner.Value
              Children =
                Seq.append
                    (tree.Children |> Seq.choose (fun child -> bindTree child k))
                    inner.Children })

    /// A generator that depends on another's value. Shrinking the first value
    /// regenerates the second from the same seed.
    [<CompiledName("Bind")>]
    let bind (Gen g) (k: 'a -> Gen<'b>) =
        Gen(fun seed size ->
            let s1, s2 = split seed
            g s1 size |> Option.bind (fun tree -> bindTree tree (fun a -> run (k a) s2 size)))

    [<CompiledName("Sized")>]
    let sized (f: int -> Gen<'a>) = Gen(fun seed size -> run (f size) seed size)

    [<CompiledName("Resize")>]
    let resize size (Gen g) = Gen(fun seed _ -> g seed size)

    let private towards (origin: BigInteger) (value: BigInteger) =
        if value = origin then Seq.empty
        else
            seq {
                yield origin
                let mutable distance = (value - origin) / BigInteger 2
                while not distance.IsZero do
                    yield value - distance
                    distance <- distance / BigInteger 2
                if BigInteger.Abs(value - origin) > BigInteger.One then
                    yield (if value > origin then value - BigInteger.One else value + BigInteger.One)
            }
            |> Seq.distinct

    /// An integer in an inclusive range, shrinking towards the value nearest
    /// zero. An open end is limited by the size, exponentially, so large
    /// magnitudes appear as the size grows.
    [<CompiledName("Integer")>]
    let integer (lo: BigInteger option) (hi: BigInteger option) =
        Gen(fun seed size ->
            let reach = BigInteger.Pow(BigInteger 2, min 64 (2 + size / 2))
            let lo' = match lo, hi with
                      | Some l, _ -> l
                      | None, Some h -> BigInteger.Min(BigInteger.Zero, h) - reach
                      | None, None -> -reach
            let hi' = match hi, lo with
                      | Some h, _ -> h
                      | None, Some l -> BigInteger.Max(BigInteger.Zero, l) + reach
                      | None, None -> reach
            if lo' > hi' then None
            else
                let n, _ = below (hi' - lo' + BigInteger.One) seed
                let value = lo' + n
                let origin = BigInteger.Min(BigInteger.Max(BigInteger.Zero, lo'), hi')
                Some(Tree.unfold (towards origin) value))

    [<CompiledName("Range")>]
    let range (lo: int) (hi: int) = integer (Some(BigInteger lo)) (Some(BigInteger hi)) |> map int

    /// One of several generators, shrinking towards the earlier ones.
    [<CompiledName("Oneof")>]
    let oneof (gens: Gen<'a> list) =
        match gens with
        | [] -> empty
        | _ ->
            let arr = Array.ofList gens
            bind (range 0 (arr.Length - 1)) (fun i -> arr.[i])

    /// A weighted choice, shrinking towards the earlier ones.
    [<CompiledName("Frequency")>]
    let frequency (choices: (int * Gen<'a>) list) =
        let total = choices |> List.sumBy fst
        if total <= 0 then empty
        else
            bind (range 0 (total - 1)) (fun n ->
                let rec pick n choices =
                    match choices with
                    | [ (_, g) ] -> g
                    | (w, g) :: rest -> if n < w then g else pick (n - w) rest
                    | [] -> empty
                pick n choices)

    let rec private sequenceTrees (trees: Tree<'a> list) : Tree<'a list> =
        let values = trees |> List.map (fun t -> t.Value)
        let removals =
            seq {
                let n = List.length trees
                if n > 0 then
                    yield []
                    if n > 1 then yield List.take (n / 2) trees
                    for i in 0 .. n - 1 do
                        yield List.removeAt i trees
            }
            |> Seq.map sequenceTrees
        let replacements =
            seq {
                for i, t in List.indexed trees do
                    for child in t.Children do
                        yield sequenceTrees (List.updateAt i child trees)
            }
        { Value = values
          Children = Seq.append removals replacements }

    let rec private productTrees (trees: Tree<'a> list) : Tree<'a list> =
        { Value = trees |> List.map (fun t -> t.Value)
          Children =
            seq {
                for i, t in List.indexed trees do
                    for child in t.Children do
                        yield productTrees (List.updateAt i child trees)
            } }

    let private draws (gens: Gen<'a> list) seed size =
        let rec go gens seed acc =
            match gens with
            | [] -> Some(List.rev acc)
            | g :: rest ->
                let s1, s2 = split seed
                match run g s1 size with
                | None -> None
                | Some t -> go rest s2 (t :: acc)
        go gens seed []

    /// A fixed number of values, each shrinking in place.
    [<CompiledName("Vector")>]
    let vector count (g: Gen<'a>) =
        Gen(fun seed size -> draws (List.replicate count g) seed size |> Option.map productTrees)

    /// Values of several generators, in order.
    [<CompiledName("Tuple")>]
    let tuple (gens: Gen<'a> list) = Gen(fun seed size -> draws gens seed size |> Option.map productTrees)

    /// A list of a length the size limits, shrinking by removing items and by
    /// shrinking them.
    [<CompiledName("List")>]
    let list (g: Gen<'a>) =
        Gen(fun seed size ->
            let s1, s2 = split seed
            let n, _ = below (BigInteger(size + 1)) s1
            draws (List.replicate (int n) g) s2 size |> Option.map sequenceTrees)

    /// Keep values a predicate accepts, redrawing up to a limit; candidates
    /// are filtered too.
    [<CompiledName("Constrain")>]
    let constrain (limit: int) (predicate: 'a -> bool) (Gen g) =
        Gen(fun seed size ->
            let rec attempt seed n =
                if n <= 0 then None
                else
                    let s1, s2 = split seed
                    match g s1 size with
                    | Some tree when predicate tree.Value -> Some(Tree.filter predicate tree)
                    | _ -> attempt s2 (n - 1)
            attempt seed limit)

    /// A dependent input's domain: an impossible value discards the tuple, so
    /// an earlier input is redrawn rather than retried in vain.
    [<CompiledName("Refine")>]
    let refine (predicate: 'a -> bool) (g: Gen<'a>) =
        bind g (fun value -> if predicate value then constant value else empty)

    /// Draw until a predicate holds, failing the harness after a limit.
    [<CompiledName("SuchThat")>]
    let suchThat (predicate: 'a -> bool) (limit: int) (onExhausted: unit -> unit) (Gen g) =
        Gen(fun seed size ->
            let rec attempt seed n =
                if n <= 0 then
                    onExhausted ()
                    None
                else
                    let s1, s2 = split seed
                    match g s1 size with
                    | Some tree when predicate tree.Value -> Some(Tree.filter predicate tree)
                    | _ -> attempt s2 (n - 1)
            attempt seed (limit + 1))

    /// A weighted mix of fixed values and a generator.
    [<CompiledName("WithExamples")>]
    let withExamples (examples: 'a list) (g: Gen<'a>) =
        match examples with
        | [] -> g
        | _ -> frequency [ 1, oneof (examples |> List.map constant); 9, g ]

    [<CompiledName("Apply")>]
    let apply (f: Gen<'a -> 'b>) (x: Gen<'a>) = bind f (fun f -> map f x)
