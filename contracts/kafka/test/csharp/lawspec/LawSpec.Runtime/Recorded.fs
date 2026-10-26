/// Typed text for recorded values. Every target writes a recording the same
/// way, so one recorded file checks every implementation. Identities (symbols
/// and handles) are numbered within one recording. ref:REQ-law-primitives
/// ref:DEC-portable-exact-arithmetic
module LawSpec.Runtime.Recorded

open System
open System.Numerics

let private quote (s: string) =
    "\"" + s.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\""

let private lastPart (tag: string) =
    let parts = tag.Split("::")
    parts.[parts.Length - 1]

let rec private trimDecimal (c: BigInteger) e =
    if c.IsZero then
        BigInteger.Zero, 0
    elif (c % BigInteger 10).IsZero then
        trimDecimal (c / BigInteger 10) (e + 1)
    else
        c, e

let private floatText value =
    match value with
    | Float(w, bits) ->
        let name = "float" + string w

        if Scalar.floatClass value = "nan" then
            name + "NaN"
        else
            name + "Bits(\"" + bits.ToString(if w = 32 then "x8" else "x16") + "\")"
    | _ -> Fail.raiseWith "InvalidRecordedScalar" "float"

let private units name (values: int seq) =
    name + "([" + String.Join(", ", values |> Seq.map string) + "])"

let private scalar value (t: string) =
    match t, value with
    | "Bool", Bool b -> if b then "true" else "false"
    | "Unit", Unit -> "()"
    | "Null", Null -> "null"
    | "Undefined", Undefined -> "undefined"
    | "Text", Text s -> quote s
    | "Char", Int n -> quote (Text.Rune(int n).ToString())
    | ("CodePoint" | "CodeUnit16"), Int n -> n.ToString()
    | "Bytes", Bytes b -> units "bytes" (b |> Seq.map int)
    | "CodePointText", Raw(_, us) -> units "codePoints" us
    | "Utf16Text", Raw(_, us) -> units "utf16" us
    | "Decimal", Decimal(c, e) ->
        let c, e = trimDecimal c e
        c.ToString() + "e" + string e
    | "Rational", Ratio(n, d) -> "rational(" + n.ToString() + ", " + d.ToString() + ")"
    | ("Float32" | "Float64"), Float _ -> floatText value
    | "Complex64", Complex(_, r, i) -> "Complex64(" + floatText r + ", " + floatText i + ")"
    | "Complex128", Complex(_, r, i) -> "Complex128(" + floatText r + ", " + floatText i + ")"
    | _, Int n when Scalar.isIntegerType t -> n.ToString()
    | _ -> Fail.raiseWith "InvalidRecordedScalar" t

type private Ids = Map<string, (obj * int) list>

let private identity kind (value: obj) (ids: Ids) =
    let known = defaultArg (ids.TryFind kind) []

    match
        known
        |> List.tryFind (fun (v, _) -> Object.ReferenceEquals(v, value) || v.Equals value)
    with
    | Some(_, n) -> n, ids
    | None ->
        let n = List.length known + 1
        n, ids.Add(kind, known @ [ value, n ])

let rec private render (value: Value) (t: TypeRef) (schema: Schema.Schema option) (ids: Ids) : string * Ids =
    match t, value with
    | Named("List", [ element ]), List items ->
        let (parts: string list), next =
            sequence items (items |> List.map (fun _ -> element)) schema ids

        "[" + String.Join(", ", parts) + "]", next
    | Named(kind, [ _ ]), Presence(_, None) -> (if kind = "Nullable" then "null" else "undefined"), ids
    | Named(kind, [ inner ]), Presence(_, Some v) ->
        let part, next = render v inner schema ids
        (if kind = "Nullable" then "nullable(" else "optional(") + part + ")", next
    | Named("Symbol", []), Symbol(id, description) ->
        let n, next = identity "symbol" id ids
        "symbol(" + string n + ", " + quote description + ")", next
    | Named(name, []), Handle(_, id) ->
        let n, next = identity ("handle " + name) id ids
        lastPart name + "#" + string n, next
    | _, Data(tag, fields) ->
        let types =
            match t, tag with
            | _, "Maybe::Absent" -> []
            | Named("Maybe", [ inner ]), _ -> [ inner ]
            | Named("Either", [ l; _ ]), "Either::Left" -> [ l ]
            | Named("Either", [ _; r ]), "Either::Right" -> [ r ]
            | _ ->
                match schema with
                | Some s -> Schema.fieldTypes value t s
                | None -> Fail.raiseWith "RecordingNeedsSchema" tag

        let (parts: string list), next = sequence fields types schema ids

        (if List.isEmpty parts then
             lastPart tag
         else
             lastPart tag + "(" + String.Join(", ", parts) + ")"),
        next
    | Named(name, []), _ -> scalar value name, ids
    | _ -> Fail.raiseWith "InvalidRecordedValue" (Scalar.typeKey t)

and private sequence (values: Value list) (types: TypeRef list) schema ids : string list * Ids =
    List.zip values types
    |> List.fold
        (fun (parts, ids) (v, t) ->
            let part, next = render v t schema ids
            parts @ [ part ], next)
        ([], ids)

/// A value's recorded text at its type.
[<CompiledName("Text")>]
let text (value: Value) (t: TypeRef) (schema: Schema.Schema option) = fst (render value t schema Map.empty)

/// The untyped rendering a model's recorded state uses.
[<CompiledName("Render")>]
let rec renderPlain (value: Value) : string =
    match value with
    | Bool true -> "true"
    | Bool false -> "false"
    | Unit -> "()"
    | Int n -> n.ToString()
    | Text s -> quote s
    | List items -> "[" + String.Join(", ", items |> List.map renderPlain) + "]"
    | Data(tag, []) -> lastPart tag
    | Data(tag, fields) -> lastPart tag + "(" + String.Join(", ", fields |> List.map renderPlain) + ")"
    | _ -> sprintf "%A" value
