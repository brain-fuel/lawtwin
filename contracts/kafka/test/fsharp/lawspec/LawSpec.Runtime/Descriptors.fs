/// Type descriptors and the portable generator over them: models, scenarios
/// and channels draw, shrink, render and encode their values exactly as every
/// other runtime does, so one seed replays one run on every target, and the
/// wire bytes are the same. ref:DEC-portable-seeded-generation
/// ref:DEC-distribution-canonical-wire ref:DEC-shrink-within-domain
module LawSpec.Runtime.Descriptors

open System
open System.IO
open System.Numerics
open System.Text

/// A descriptor form: an atom, an integer, a quoted string, the hole `_`, or
/// a parenthesized list of forms.
[<StructuralEquality; NoComparison>]
type Descriptor =
    | Atom of string
    | Number of BigInteger
    | Quoted of string
    | Hole
    | Forms of Descriptor list

let private fail code : 'a = Fail.raiseWith code ""

let private require condition code = if not condition then fail code

/// Read every form of a descriptor text.
[<CompiledName("Read")>]
let read (text: string) : Descriptor list =
    let points = Scalar.codePoints text |> Array.ofList
    let mutable i = 0
    let isSpace c = c = int ' ' || c = int '\n' || c = int '\r' || c = int '\t'
    let skip () = while i < points.Length && isSpace points.[i] do i <- i + 1
    let rec forms () =
        let acc = ResizeArray()
        skip ()
        while i < points.Length && points.[i] <> int ')' do
            acc.Add(item ())
            skip ()
        List.ofSeq acc
    and item () =
        if points.[i] = int '(' then
            i <- i + 1
            let children = forms ()
            require (i < points.Length && points.[i] = int ')') "UnclosedDescriptor"
            i <- i + 1
            Forms children
        elif points.[i] = int '"' then
            i <- i + 1
            let chars = ResizeArray()
            let mutable closed = false
            while not closed do
                require (i < points.Length) "UnclosedDescriptorString"
                let c = points.[i]
                if c = int '\\' && i + 1 < points.Length then
                    chars.Add points.[i + 1]
                    i <- i + 2
                elif c = int '"' then
                    closed <- true
                    i <- i + 1
                else
                    chars.Add c
                    i <- i + 1
            Quoted(Scalar.textOfPoints chars)
        else
            let start = i
            while i < points.Length && not (isSpace points.[i]) && points.[i] <> int '(' && points.[i] <> int ')' do
                i <- i + 1
            require (i > start) "EmptyDescriptorAtom"
            let word = Scalar.textOfPoints points.[start .. i - 1]
            if word = "_" then Hole
            else
                match BigInteger.TryParse(word, Globalization.NumberStyles.AllowLeadingSign, Globalization.CultureInfo.InvariantCulture) with
                | true, n -> Number n
                | _ -> Atom word
    let result = forms ()
    require (i = points.Length) "TrailingDescriptorInput"
    result

let private unquote d =
    match d with
    | Quoted s -> s
    | Atom s -> s
    | _ -> fail "InvalidDescriptorName"

/// The named data descriptors of a text, and its last form.
[<CompiledName("FromText")>]
let fromText (text: string) : Map<string, Descriptor> * Descriptor =
    let forms = read text
    require (not (List.isEmpty forms)) "EmptyDescriptor"
    let table =
        forms
        |> List.choose (function
            | Forms(Atom "data" :: name :: _) as d -> Some(unquote name, d)
            | _ -> None)
        |> Map.ofList
    table, List.last forms

/// A reference resolved through the table.
[<CompiledName("Resolve")>]
let resolve (d: Descriptor) (table: Map<string, Descriptor>) =
    match d with
    | Forms [ Atom "ref"; name ] -> table.[unquote name]
    | _ -> d

let private bound d =
    match d with
    | Number n -> Some n
    | Hole -> None
    | _ -> fail "InvalidDescriptorBound"

let private generationBounds lo hi =
    match lo, hi with
    | None, None -> BigInteger(-1000000), BigInteger 1000000
    | None, Some h -> BigInteger.Min(BigInteger(-1000000), h - BigInteger 2000000), h
    | Some l, None -> l, BigInteger.Max(BigInteger 1000000, l + BigInteger 2000000)
    | Some l, Some h -> l, h

let rec private mentionsData d =
    match d with
    | Forms(Atom "ref" :: _) -> true
    | Forms(Atom "data" :: _) -> true
    | Forms(_ :: fields) -> List.exists mentionsData fields
    | _ -> false

let private constructorsOf d =
    match d with
    | Forms(Atom "data" :: _ :: cs) -> cs
    | _ -> fail "InvalidDataDescriptor"

let private baseConstructors cs =
    let found = cs |> List.filter (function Forms(Atom "ctor" :: _ :: fields) -> not (List.exists mentionsData fields) | _ -> false)
    if List.isEmpty found then cs else found

let private below (n: BigInteger) state = Random.below n state

let private drawN count draw state =
    let mutable s = state
    let values =
        [ for _ in 1 .. count ->
              let v, next = draw s
              s <- next
              v ]
    values, s

let private drawN' items draw state =
    let mutable s = state
    let values =
        [ for item in items ->
              let v, next = draw item s
              s <- next
              v ]
    values, s

/// A value drawn from a descriptor at a size, and the next state. Every draw,
/// including each boundary choice, follows the portable algorithm.
[<CompiledName("Generate")>]
let rec generate (d: Descriptor) (table: Map<string, Descriptor>) (state: Random.Seed) (size: int) : Value * Random.Seed =
    match resolve d table with
    | Forms [ Atom "int"; _; lo; hi ] ->
        let lo, hi = generationBounds (bound lo) (bound hi)
        let choice, s1 = below (BigInteger 10) state
        if choice < BigInteger 2 then
            let i, s2 = below (BigInteger 4) s1
            let options = [ lo; hi; BigInteger.Min(BigInteger.Max(BigInteger.Zero, lo), hi); BigInteger.Min(BigInteger.Max(BigInteger.One, lo), hi) ]
            Int(List.item (int i) options), s2
        else
            let n, s2 = below (hi - lo + BigInteger.One) s1
            Int(lo + n), s2
    | Forms [ Atom "bool" ] ->
        let n, next = below (BigInteger 2) state
        Bool n.IsOne, next
    | Forms [ Atom "text" ] ->
        let count, s1 = below (BigInteger(size + 1)) state
        let chars, s2 = drawN (int count) (fun s -> let n, s0 = below (BigInteger 95) s in int n + 32, s0) s1
        Text(String(chars |> List.map char |> Array.ofList)), s2
    | Forms [ Atom "unit" ] -> Unit, state
    | Forms [ Atom "list"; item ] ->
        let count, s1 = below (BigInteger(size + 1)) state
        let items, s2 = drawN (int count) (fun s -> generate item table s size) s1
        List items, s2
    | Forms [ Atom "maybe"; item ] ->
        let choice, s1 = below (BigInteger 4) state
        if choice.IsZero then Data("Maybe::Absent", []), s1
        else
            let v, s2 = generate item table s1 size
            Data("Maybe::Present", [ v ]), s2
    | Forms [ Atom "either"; left; right ] ->
        let choice, s1 = below (BigInteger 2) state
        let tag, inner = if choice.IsZero then "Either::Left", left else "Either::Right", right
        let v, s2 = generate inner table s1 size
        Data(tag, [ v ]), s2
    | Forms(Atom "data" :: _ :: cs) ->
        let choices = if size = 0 then baseConstructors cs else cs
        let i, s1 = below (BigInteger(List.length choices)) state
        match List.item (int i) choices with
        | Forms(Atom "ctor" :: tag :: fields) ->
            let values, s2 = drawN' fields (fun field s -> generate field table s (max (size - 1) 0)) s1
            Data(unquote tag, values), s2
        | _ -> fail "InvalidConstructorDescriptor"
    | other -> Fail.raiseWith "UnsupportedGenerationDescriptor" (sprintf "%A" other)

/// The smallest value of a descriptor.
[<CompiledName("Minimal")>]
let rec minimal (d: Descriptor) (table: Map<string, Descriptor>) : Value =
    match resolve d table with
    | Forms [ Atom "int"; _; lo; hi ] ->
        let lo, hi = generationBounds (bound lo) (bound hi)
        Int(BigInteger.Min(BigInteger.Max(BigInteger.Zero, lo), hi))
    | Forms [ Atom "bool" ] -> Bool false
    | Forms [ Atom "text" ] -> Text ""
    | Forms [ Atom "unit" ] -> Unit
    | Forms [ Atom "list"; _ ] -> List []
    | Forms [ Atom "maybe"; _ ] -> Data("Maybe::Absent", [])
    | Forms [ Atom "either"; left; _ ] -> Data("Either::Left", [ minimal left table ])
    | Forms(Atom "data" :: _ :: cs) ->
        match baseConstructors cs with
        | Forms(Atom "ctor" :: tag :: fields) :: _ -> Data(unquote tag, fields |> List.map (fun f -> minimal f table))
        | _ -> fail "InvalidDataDescriptor"
    | other -> Fail.raiseWith "UnsupportedGenerationDescriptor" (sprintf "%A" other)

let private shrinkSequence (values: 'a list) =
    match values with
    | [] -> []
    | _ ->
        [ []; List.take (List.length values / 2) values ]
        @ [ for i in 0 .. List.length values - 1 -> List.removeAt i values ]

let private replaceShrinks (values: Value list) (shrink: Value -> Value list) =
    [ for i, v in List.indexed values do
          for c in shrink v -> List.updateAt i c values ]

let private constructorOf tag cs =
    match cs |> List.filter (function Forms(Atom "ctor" :: t :: _) -> unquote t = tag | _ -> false) with
    | [ c ] -> c
    | _ -> Fail.raiseWith "ForeignWireConstructor" tag

/// The candidates a value shrinks to, in the portable order, without
/// duplicates or the value itself.
[<CompiledName("Shrink")>]
let rec shrink (d: Descriptor) (value: Value) (table: Map<string, Descriptor>) : Value list =
    let candidates = shrinkResolved (resolve d table) value table
    let seen = System.Collections.Generic.HashSet<Value>(HashIdentity.Structural)
    seen.Add value |> ignore
    candidates |> List.filter (fun c -> seen.Add c)

and private shrinkResolved d value table =
    match d, value with
    | Forms [ Atom "int"; _; _; _ ], Int v ->
        match minimal d table with
        | Int target when v = target -> []
        | Int target ->
            let half = BigInteger.Divide(v - target, BigInteger 2)
            [ Int target; Int(v - half); Int(v - (if v > target then BigInteger.One else BigInteger.MinusOne)) ]
        | _ -> []
    | Forms [ Atom "bool" ], Bool true -> [ Bool false ]
    | Forms [ Atom "bool" ], Bool false -> []
    | Forms [ Atom "text" ], Text s -> shrinkSequence (Scalar.codePoints s) |> List.map (Scalar.textOfPoints >> Text)
    | Forms [ Atom "list"; item ], List items ->
        (shrinkSequence items |> List.map List) @ (replaceShrinks items (fun v -> shrink item v table) |> List.map List)
    | Forms [ Atom "maybe"; item ], Data("Maybe::Present", [ v ]) ->
        Data("Maybe::Absent", []) :: (shrink item v table |> List.map (fun c -> Data("Maybe::Present", [ c ])))
    | Forms [ Atom "maybe"; _ ], Data("Maybe::Absent", []) -> []
    | Forms [ Atom "either"; left; right ], Data(tag, [ v ]) ->
        let inner = if tag = "Either::Left" then left else right
        shrink inner v table |> List.map (fun c -> Data(tag, [ c ]))
    | Forms(Atom "data" :: name :: cs), Data(tag, fields) ->
        let fieldDescriptors =
            match constructorOf tag cs with
            | Forms(Atom "ctor" :: _ :: ds) -> ds
            | _ -> []
        let recursive = Forms [ Atom "ref"; name ]
        [ minimal d table ]
        @ [ for f, fd in List.zip fields fieldDescriptors do if fd = recursive then yield f ]
        @ [ for i, fd in List.indexed fieldDescriptors do
                for c in shrink fd (List.item i fields) table -> Data(tag, List.updateAt i c fields) ]
    | Forms [ Atom "unit" ], Unit -> []
    | _ -> fail "InvalidShrinkValue"

let private quote (s: string) = "\"" + s.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\""

/// A value as the portable runtimes print it.
[<CompiledName("Render")>]
let rec render (value: Value) : string =
    match value with
    | Bool true -> "true"
    | Bool false -> "false"
    | Unit -> "()"
    | Int n -> n.ToString()
    | Text s -> quote s
    | List items -> "[" + String.Join(", ", items |> List.map render) + "]"
    | Data(tag, fields) ->
        let parts = tag.Split("::")
        let name = parts.[parts.Length - 1]
        if List.isEmpty fields then name else name + "(" + String.Join(", ", fields |> List.map render) + ")"
    | other -> sprintf "%A" other

// ---------------------------------------------------------------------------
// The canonical wire: zigzag varints, length-prefixed bytes, constructor
// indices. ref:DEC-distribution-canonical-wire

/// A non-negative integer as LEB128.
[<CompiledName("Varint")>]
let varint (n: BigInteger) : byte[] =
    if n.Sign < 0 then fail "NegativeVarint"
    let bytes = ResizeArray<byte>()
    let mutable rest = n
    let mutable go = true
    while go do
        let low = int (rest &&& BigInteger 127)
        rest <- rest >>> 7
        if rest.IsZero then
            bytes.Add(byte low)
            go <- false
        else bytes.Add(byte (low ||| 128))
    bytes.ToArray()

/// A LEB128 integer at a position, and the position after it.
[<CompiledName("ReadVarint")>]
let readVarint (bytes: byte[]) (start: int) : BigInteger * int =
    let mutable n = BigInteger.Zero
    let mutable shift = 0
    let mutable i = start
    let mutable go = true
    while go do
        if i >= bytes.Length then fail "TruncatedWireValue"
        let b = bytes.[i]
        n <- n ||| (BigInteger(int b &&& 127) <<< shift)
        shift <- shift + 7
        i <- i + 1
        if b < 128uy then go <- false
    n, i

let private checkInteger (v: BigInteger) lo hi =
    let ok = (match bound lo with Some l -> v >= l | None -> true) && (match bound hi with Some h -> v <= h | None -> true)
    if not ok then fail "WireIntegerOutOfRange"

let rec private put (d: Descriptor) (value: Value) (table: Map<string, Descriptor>) (out: MemoryStream) =
    let write (bytes: byte[]) = out.Write(bytes, 0, bytes.Length)
    match resolve d table, value with
    | Forms [ Atom "int"; _; lo; hi ], Int v ->
        checkInteger v lo hi
        write (varint (if v.Sign >= 0 then v * BigInteger 2 else -v * BigInteger 2 - BigInteger.One))
    | Forms [ Atom "bool" ], Bool b -> out.WriteByte(if b then 1uy else 0uy)
    | Forms [ Atom "unit" ], Unit -> ()
    | Forms [ Atom("text" | "end") ], Text s ->
        let bytes = Encoding.UTF8.GetBytes s
        write (varint (BigInteger bytes.Length))
        write bytes
    | Forms [ Atom("bytes" | "end") ], Bytes b ->
        write (varint (BigInteger b.Length))
        write b
    | Forms [ Atom "list"; item ], List items ->
        write (varint (BigInteger(List.length items)))
        for v in items do put item v table out
    | Forms [ Atom "maybe"; _ ], Data("Maybe::Absent", []) -> out.WriteByte 0uy
    | Forms [ Atom "maybe"; item ], Data("Maybe::Present", [ v ]) ->
        out.WriteByte 1uy
        put item v table out
    | Forms [ Atom "either"; left; _ ], Data("Either::Left", [ v ]) ->
        out.WriteByte 0uy
        put left v table out
    | Forms [ Atom "either"; _; right ], Data("Either::Right", [ v ]) ->
        out.WriteByte 1uy
        put right v table out
    | Forms(Atom "data" :: _ :: cs), Data(tag, fields) ->
        let c = constructorOf tag cs
        let ds = match c with Forms(Atom "ctor" :: _ :: ds) -> ds | _ -> []
        require (List.length ds = List.length fields) "WireFieldCount"
        let index = cs |> List.findIndex (fun other -> other = c)
        write (varint (BigInteger index))
        for fd, v in List.zip ds fields do put fd v table out
    | other, _ -> Fail.raiseWith "InvalidWireValue" (sprintf "%A" other)

/// A value's canonical wire bytes.
[<CompiledName("Encode")>]
let encode (d: Descriptor) (value: Value) (table: Map<string, Descriptor>) : byte[] =
    use out = new MemoryStream()
    put d value table out
    out.ToArray()

let rec private get (d: Descriptor) (bytes: byte[]) (i: int) (table: Map<string, Descriptor>) : Value * int =
    match resolve d table with
    | Forms [ Atom "int"; _; lo; hi ] ->
        let z, next = readVarint bytes i
        let v = if z.IsEven then z / BigInteger 2 else -(z + BigInteger.One) / BigInteger 2
        checkInteger v lo hi
        Int v, next
    | Forms [ Atom "bool" ] ->
        require (i < bytes.Length && bytes.[i] < 2uy) "InvalidWireValue"
        Bool(bytes.[i] = 1uy), i + 1
    | Forms [ Atom "unit" ] -> Unit, i
    | Forms [ Atom kind ] when kind = "text" || kind = "bytes" || kind = "end" ->
        let n, start = readVarint bytes i
        let n = int n
        require (start + n <= bytes.Length) "TruncatedWireValue"
        let slice = bytes.[start .. start + n - 1]
        if kind = "bytes" then Bytes slice, start + n
        else
            let text =
                try UTF8Encoding(false, true).GetString slice
                with _ -> fail "WireInvalidUtf8"
            Text text, start + n
    | Forms [ Atom "list"; item ] ->
        let n, start = readVarint bytes i
        let mutable position = start
        let items =
            [ for _ in 1 .. int n ->
                  let v, next = get item bytes position table
                  position <- next
                  v ]
        List items, position
    | Forms [ Atom "maybe"; item ] ->
        require (i < bytes.Length && bytes.[i] < 2uy) "InvalidWireValue"
        if bytes.[i] = 0uy then Data("Maybe::Absent", []), i + 1
        else
            let v, next = get item bytes (i + 1) table
            Data("Maybe::Present", [ v ]), next
    | Forms [ Atom "either"; left; right ] ->
        require (i < bytes.Length && bytes.[i] < 2uy) "InvalidWireValue"
        let tag, inner = if bytes.[i] = 0uy then "Either::Left", left else "Either::Right", right
        let v, next = get inner bytes (i + 1) table
        Data(tag, [ v ]), next
    | Forms(Atom "data" :: _ :: cs) ->
        let index, start = readVarint bytes i
        require (index < BigInteger(List.length cs)) "InvalidWireConstructor"
        match List.item (int index) cs with
        | Forms(Atom "ctor" :: tag :: ds) ->
            let mutable position = start
            let fields =
                [ for fd in ds ->
                      let v, next = get fd bytes position table
                      position <- next
                      v ]
            Data(unquote tag, fields), position
        | _ -> fail "InvalidWireConstructor"
    | other -> Fail.raiseWith "InvalidWireValue" (sprintf "%A" other)

/// A value from its wire bytes, and the position after it.
[<CompiledName("DecodePrefix")>]
let decodePrefix (d: Descriptor) (bytes: byte[]) (start: int) (table: Map<string, Descriptor>) = get d bytes start table

/// A value from exactly its wire bytes.
[<CompiledName("Decode")>]
let decode (d: Descriptor) (bytes: byte[]) (table: Map<string, Descriptor>) : Value =
    let v, next = get d bytes 0 table
    require (next = bytes.Length) "TrailingWireBytes"
    v

/// The values a descriptor text draws from a seed (the generation suite's
/// conformance vectors).
[<CompiledName("Generated")>]
let generated (text: string) (seed: BigInteger) (size: int) (count: int) : string list =
    let table, d = fromText text
    let mutable state = Random.seed seed
    [ for _ in 1 .. max 0 count ->
          let v, next = generate d table state size
          state <- next
          render v ]

/// The shrink candidates of the first value a descriptor text draws.
[<CompiledName("Shrunk")>]
let shrunk (text: string) (seed: BigInteger) (size: int) : string list =
    let table, d = fromText text
    let v, _ = generate d table (Random.seed seed) size
    shrink d v table |> List.map render
