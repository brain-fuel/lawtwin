/// LawSpec's generation plans as FsCheck generators. FsCheck draws every
/// random choice and gives the size; LawSpec builds, for each value, the rose
/// tree of the smaller values it may shrink to, so shrinking a bound input
/// rebuilds the inputs that depend on it and every candidate stays inside the
/// domain. Property turns the tree into an FsCheck Arbitrary whose shrinker
/// walks it. ref:DEC-dotnet-fscheck ref:DEC-shrink-within-domain
/// ref:DEC-planned-generation
namespace LawSpec.Testing

open System
open System.Numerics
open FsCheck

/// A value and, lazily, the values it shrinks to, best first.
type Tree<'a> = { Value: 'a; Children: seq<Tree<'a>> }

module Tree =
    [<CompiledName("Leaf")>]
    let leaf value = { Value = value; Children = Seq.empty }

    [<CompiledName("Map")>]
    let rec map (f: 'a -> 'b) (tree: Tree<'a>) =
        {
            Value = f tree.Value
            Children = tree.Children |> Seq.map (map f)
        }

    /// Keep only the candidates a predicate accepts, with their own candidates.
    [<CompiledName("Filter")>]
    let rec filter (keep: 'a -> bool) (tree: Tree<'a>) =
        { tree with
            Children = tree.Children |> Seq.filter (fun t -> keep t.Value) |> Seq.map (filter keep)
        }

    /// Candidates from a function of a value, unfolded.
    [<CompiledName("Unfold")>]
    let rec unfold (shrink: 'a -> seq<'a>) (value: 'a) =
        {
            Value = value
            Children = shrink value |> Seq.map (unfold shrink)
        }

    /// At most a number of shrink steps, each trying at most a number of
    /// candidates.
    [<CompiledName("Limit")>]
    let rec limit (steps: int) (width: int) (tree: Tree<'a>) =
        { tree with
            Children =
                if steps <= 0 then
                    Seq.empty
                else
                    tree.Children |> Seq.truncate width |> Seq.map (limit (steps - 1) width)
        }

/// A generator: FsCheck's generator of a value's shrink tree, or None for an
/// empty domain, which discards the whole tuple of inputs.
type Gen<'a> = Gen of FsCheck.Gen<Tree<'a> option>

module Gen =
    module Native = FsCheck.FSharp.Gen

    /// FsCheck's generator of the trees.
    [<CompiledName("ToFsCheck")>]
    let toFsCheck (Gen g) = g

    [<CompiledName("Constant")>]
    let constant value =
        Gen(Native.constant (Some(Tree.leaf value)))

    /// An empty domain: the case is discarded and redrawn.
    [<CompiledName("Empty")>]
    let empty<'a> : Gen<'a> = Gen(Native.constant None)

    [<CompiledName("Map")>]
    let map f (Gen g) =
        Gen(Native.map (Option.map (Tree.map f)) g)

    let rec private bindTree (tree: Tree<'a>) (k: 'a -> Tree<'b> option) : Tree<'b> option =
        k tree.Value
        |> Option.map (fun inner ->
            {
                Value = inner.Value
                Children = Seq.append (tree.Children |> Seq.choose (fun child -> bindTree child k)) inner.Children
            })

    /// A fresh FsCheck random state, drawn from the current one.
    let private fork =
        Native.choose64 (Int64.MinValue, Int64.MaxValue)
        |> Native.map (fun s -> Rnd(uint64 s))

    let private at (Gen g) (rnd: Rnd) (size: int) =
        (Native.sampleWithSeed rnd size 1 g).[0]

    /// A generator that depends on another's value. Shrinking the first value
    /// regenerates the second from the same random state and size.
    [<CompiledName("Bind")>]
    let bind (Gen g) (k: 'a -> Gen<'b>) =
        Gen(
            Native.sized (fun size ->
                g
                |> Native.bind (fun drawn ->
                    fork
                    |> Native.map (fun rnd ->
                        drawn |> Option.bind (fun tree -> bindTree tree (fun a -> at (k a) rnd size)))))
        )

    /// Values drawn in turn, each by its step given the values drawn before
    /// it: a law's inputs, each knowing the earlier ones. It shrinks as the
    /// nested binds it stands for do.
    [<CompiledName("Dependent")>]
    let dependent (steps: ('a list -> Gen<'a>) list) : Gen<'a list> =
        let rec go (previous: 'a list) (remaining: ('a list -> Gen<'a>) list) =
            match remaining with
            | [] -> constant previous
            | step :: rest -> bind (step previous) (fun drawn -> go (previous @ [ drawn ]) rest)

        go [] steps

    [<CompiledName("Sized")>]
    let sized (f: int -> Gen<'a>) =
        Gen(Native.sized (fun size -> let (Gen g) = f size in g))

    [<CompiledName("Resize")>]
    let resize size (Gen g) = Gen(Native.resize size g)

    let private int64Range = BigInteger Int64.MaxValue + BigInteger.One

    /// A uniform BigInteger in [0, n), from FsCheck's random state.
    let private below (n: BigInteger) =
        if n <= int64Range then
            Native.choose64 (0L, int64 (n - BigInteger.One)) |> Native.map BigInteger
        else
            // Enough 64-bit words that the remainder's bias is negligible.
            let words = int (n.GetBitLength() / 64L) + 2

            Native.listOfLength words (Native.choose64 (Int64.MinValue, Int64.MaxValue))
            |> Native.map (fun ws ->
                let total =
                    ws
                    |> List.fold (fun acc (w: int64) -> (acc <<< 64) + BigInteger(uint64 w)) BigInteger.Zero

                BigInteger.Remainder(total, n))

    let private towards (origin: BigInteger) (value: BigInteger) =
        if value = origin then
            Seq.empty
        else
            seq {
                yield origin
                let mutable distance = (value - origin) / BigInteger 2

                while not distance.IsZero do
                    yield value - distance
                    distance <- distance / BigInteger 2

                if BigInteger.Abs(value - origin) > BigInteger.One then
                    yield
                        (if value > origin then
                             value - BigInteger.One
                         else
                             value + BigInteger.One)
            }
            |> Seq.distinct

    /// An integer in an inclusive range, shrinking towards the value nearest
    /// zero. An open end is limited by the size, exponentially, so large
    /// magnitudes appear as the size grows.
    [<CompiledName("Integer")>]
    let integer (lo: BigInteger option) (hi: BigInteger option) =
        Gen(
            Native.sized (fun size ->
                let reach = BigInteger.Pow(BigInteger 2, min 64 (2 + size / 2))

                let lo' =
                    match lo, hi with
                    | Some l, _ -> l
                    | None, Some h -> BigInteger.Min(BigInteger.Zero, h) - reach
                    | None, None -> -reach

                let hi' =
                    match hi, lo with
                    | Some h, _ -> h
                    | None, Some l -> BigInteger.Max(BigInteger.Zero, l) + reach
                    | None, None -> reach

                if lo' > hi' then
                    Native.constant None
                else
                    let origin = BigInteger.Min(BigInteger.Max(BigInteger.Zero, lo'), hi')

                    below (hi' - lo' + BigInteger.One)
                    |> Native.map (fun n -> Some(Tree.unfold (towards origin) (lo' + n))))
        )

    [<CompiledName("Range")>]
    let range (lo: int) (hi: int) =
        integer (Some(BigInteger lo)) (Some(BigInteger hi)) |> map int

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

        if total <= 0 then
            empty
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

                    if n > 1 then
                        yield List.take (n / 2) trees

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

        {
            Value = values
            Children = Seq.append removals replacements
        }

    let rec private productTrees (trees: Tree<'a> list) : Tree<'a list> =
        {
            Value = trees |> List.map (fun t -> t.Value)
            Children =
                seq {
                    for i, t in List.indexed trees do
                        for child in t.Children do
                            yield productTrees (List.updateAt i child trees)
                }
        }

    /// Each generator's tree in order, or None when any domain is empty.
    let private draws (gens: Gen<'a> list) =
        gens
        |> List.map toFsCheck
        |> Native.sequenceToList
        |> Native.map (fun drawn ->
            if List.forall Option.isSome drawn then
                Some(List.map Option.get drawn)
            else
                None)

    /// A fixed number of values, each shrinking in place.
    [<CompiledName("Vector")>]
    let vector count (g: Gen<'a>) =
        Gen(draws (List.replicate count g) |> Native.map (Option.map productTrees))

    /// Values of several generators, in order.
    [<CompiledName("Tuple")>]
    let tuple (gens: Gen<'a> list) =
        Gen(draws gens |> Native.map (Option.map productTrees))

    /// A list of a length the size limits, shrinking by removing items and by
    /// shrinking them.
    [<CompiledName("List")>]
    let list (g: Gen<'a>) =
        Gen(
            Native.sized (fun size ->
                below (BigInteger(size + 1))
                |> Native.bind (fun n -> draws (List.replicate (int n) g) |> Native.map (Option.map sequenceTrees)))
        )

    /// Redraw until a predicate accepts the value, at most a number of times;
    /// the accepted tree's candidates are filtered too.
    let private retry (predicate: 'a -> bool) (limit: int) (exhausted: unit -> Tree<'a> option) (Gen g) =
        let rec attempt n =
            g
            |> Native.bind (fun drawn ->
                match drawn with
                | Some tree when predicate tree.Value -> Native.constant (Some(Tree.filter predicate tree))
                | _ when n <= 1 -> Native.constant () |> Native.map exhausted
                | _ -> attempt (n - 1))

        if limit <= 0 then
            Gen(Native.constant () |> Native.map exhausted)
        else
            Gen(attempt limit)

    /// Keep values a predicate accepts, redrawing up to a limit; candidates
    /// are filtered too.
    [<CompiledName("Constrain")>]
    let constrain (limit: int) (predicate: 'a -> bool) (g: Gen<'a>) =
        retry predicate limit (fun () -> None) g

    /// A dependent input's domain: an impossible value discards the tuple, so
    /// an earlier input is redrawn rather than retried in vain.
    [<CompiledName("Refine")>]
    let refine (predicate: 'a -> bool) (g: Gen<'a>) =
        bind g (fun value -> if predicate value then constant value else empty)

    /// Draw until a predicate holds, failing the harness after a limit.
    [<CompiledName("SuchThat")>]
    let suchThat (predicate: 'a -> bool) (limit: int) (onExhausted: unit -> unit) (g: Gen<'a>) =
        retry
            predicate
            (limit + 1)
            (fun () ->
                onExhausted ()
                None)
            g

    /// A weighted mix of fixed values and a generator.
    [<CompiledName("WithExamples")>]
    let withExamples (examples: 'a list) (g: Gen<'a>) =
        match examples with
        | [] -> g
        | _ -> frequency [ 1, oneof (examples |> List.map constant); 9, g ]

    [<CompiledName("Apply")>]
    let apply (f: Gen<'a -> 'b>) (x: Gen<'a>) = bind f (fun f -> map f x)

    /// An application's FsCheck Arbitrary, keeping its own shrinker.
    [<CompiledName("OfArbitrary")>]
    let ofArbitrary (arbitrary: Arbitrary<'a>) =
        let shrink = FsCheck.FSharp.Arb.toShrink arbitrary

        Gen(
            FsCheck.FSharp.Arb.toGen arbitrary
            |> Native.map (fun value -> Some(Tree.unfold shrink value))
        )
