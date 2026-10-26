/// The checked shapes of a program's data: constructors, field types, GADT
/// equations, existential witnesses, index guards and field contracts, all
/// checked by one walk. ref:DEC-typed-core-boundary
/// ref:DEC-gadts-and-index-arithmetic
module LawSpec.Runtime.Schema

open System
open System.Numerics

let private fail code detail : 'a = Fail.raiseWith code detail

/// Bounds and hints that steer the generation of one constructor field from
/// the fields drawn before it. ref:DEC-shrink-within-domain
type FieldHints =
    {
        Bounds: (string * Value) list
        Lengths: (string * Value) list
        Hints: Value list
    }

/// A data constructor; Arguments and WitnessValues are filled in when it is
/// instantiated at a type.
type Constructor =
    {
        Tag: string
        Fields: (string * TypeRef) list
        Predicates: (Schema -> TypeRef list -> Value list -> bool) list
        /// By field position, from 1.
        Generators: Map<int, Schema -> TypeRef list -> Value list -> FieldHints>
        Indices: string list
        Refinements: (int * TypeRef) list
        Existentials: int
        Witnesses: int list
        Arguments: TypeRef list
        WitnessValues: Value list
    }

and DataDefinition =
    {
        Name: string
        Parameters: int
        Constructors: Constructor list
        Handle: bool
    }

and Schema =
    {
        Definitions: Map<string, DataDefinition>
        Arities: Map<string, int>
        Bits: int
        /// The handlers in scope, by ability key (LawSpec.Runtime.Effects).
        Handlers: Map<string, obj>
        /// The innermost handler scope, which owns handler state.
        Scope: obj option
        /// The stateful handlers whose operations are running, innermost first.
        EffectPath: obj list
    }

/// A constructor with only a tag and fields; the generated schema adds the rest.
[<CompiledName("Constructor")>]
let constructor tag fields =
    {
        Tag = tag
        Fields = fields
        Predicates = []
        Generators = Map.empty
        Indices = []
        Refinements = []
        Existentials = 0
        Witnesses = []
        Arguments = []
        WitnessValues = []
    }

/// The type parameters' types, by position.
[<CompiledName("Substitute")>]
let rec substitute (t: TypeRef) (known: Map<int, TypeRef>) =
    match t with
    | Parameter i -> defaultArg (known.TryFind i) t
    | Named(name, args) -> Named(name, args |> List.map (fun a -> substitute a known))

let rec private checkScope (t: TypeRef) scope (schema: Schema) =
    match t with
    | Parameter i ->
        if i < 0 || i >= scope then
            fail "UnboundSchemaParameter" (string i)
    | Named(name, args) ->
        match schema.Arities.TryFind name with
        | Some arity when arity = List.length args -> args |> List.iter (fun a -> checkScope a scope schema)
        | _ -> fail "UnknownTypeOrArity" name

/// Check that a closed type is known, with the right number of arguments.
[<CompiledName("CheckType")>]
let checkType (t: TypeRef) (schema: Schema) = checkScope t 0 schema

let private builtins =
    [ "List", 1; "Maybe", 1; "Either", 2; "Nullable", 1; "Optional", 1 ]

/// A program's schema, from its data definitions and the scalar names.
[<CompiledName("Create")>]
let create (definitions: DataDefinition list) (primitives: string list) (bits: int) =
    if bits <> 32 && bits <> 64 then
        fail "InvalidMachineBits" (string bits)

    let names =
        (definitions |> List.map (fun d -> d.Name))
        @ primitives
        @ (builtins |> List.map fst)

    if List.length names <> List.length (List.distinct names) then
        fail "DuplicateType" ""

    let arities =
        Map.ofList (
            (primitives |> List.map (fun p -> p, 0))
            @ builtins
            @ (definitions |> List.map (fun d -> d.Name, d.Parameters))
        )

    let schema =
        {
            Definitions = definitions |> List.map (fun d -> d.Name, d) |> Map.ofList
            Arities = arities
            Bits = bits
            Handlers = Map.empty
            Scope = None
            EffectPath = []
        }

    let tags =
        definitions
        |> List.collect (fun d -> d.Constructors |> List.map (fun c -> c.Tag))

    if List.length tags <> List.length (List.distinct tags) then
        fail "DuplicateConstructor" ""

    for d in definitions do
        for c in d.Constructors do
            let scope = d.Parameters + c.Existentials

            for _, t in c.Fields do
                checkScope t scope schema

    schema

let rec private unify (pattern: TypeRef) (actual: TypeRef) (known: Map<int, TypeRef> option) =
    match known with
    | None -> None
    | Some k ->
        match pattern with
        | Parameter i ->
            match k.TryFind i with
            | None -> Some(k.Add(i, actual))
            | Some t when t = actual -> known
            | Some _ -> None
        | Named(name, ps) ->
            match actual with
            | Named(n, xs) when n = name && List.length ps = List.length xs ->
                List.fold2 (fun acc p a -> unify p a acc) known ps xs
            | _ -> None

let private instantiate (c: Constructor) (args: TypeRef list) =
    let known = args |> List.mapi (fun i a -> i, a) |> Map.ofList

    let bound =
        c.Refinements
        |> List.fold (fun acc (i, pattern) -> unify pattern (List.item i args) acc) (Some known)

    bound
    |> Option.map (fun b ->
        { c with
            Arguments = args
            Fields = c.Fields |> List.map (fun (n, t) -> n, substitute t b)
        })

let private builtinConstructors name (args: TypeRef list) =
    match name, args with
    | "Maybe", [ t ] -> Some [ constructor "Maybe::Absent" []; constructor "Maybe::Present" [ "value", t ] ]
    | "Either", [ a; b ] ->
        Some
            [
                constructor "Either::Left" [ "value", a ]
                constructor "Either::Right" [ "value", b ]
            ]
    | _ -> None

/// The constructors of a data type at its arguments, or None for a scalar or
/// a list. A GADT constructor whose equations the arguments refute is absent.
[<CompiledName("Constructors")>]
let constructors (t: TypeRef) (schema: Schema) : Constructor list option =
    checkType t schema

    match t with
    | Named(name, args) ->
        match schema.Definitions.TryFind name with
        | None -> builtinConstructors name args
        | Some d -> Some(d.Constructors |> List.choose (fun c -> instantiate c args))
    | Parameter _ -> None

/// Each instance of a constructor's free existentials over Core's finite
/// witness pool (Bool and Int32).
[<CompiledName("WitnessInstances")>]
let witnessInstances (c: Constructor) =
    let pool = [ Named("Bool", []); Named("Int32", []) ]

    let assignments =
        c.Witnesses
        |> List.fold
            (fun previous i ->
                [
                    for known in previous do
                        for t in pool -> Map.add i t known
                ])
            [ Map.empty ]

    let kept = List.length c.Fields - List.length c.Witnesses

    [
        for known in assignments ->
            { c with
                Fields = c.Fields |> List.take kept |> List.map (fun (n, t) -> n, substitute t known)
                WitnessValues = c.Witnesses |> List.map (fun i -> Text(Scalar.typeKey known.[i]))
            }
    ]

let private findConstructor tag (cs: Constructor list) =
    match cs |> List.filter (fun c -> c.Tag = tag) with
    | [ c ] -> c
    | _ -> fail "ForeignConstructor" tag

let private witnessed (c: Constructor) (values: Value list) (schema: Schema) =
    let texts = values |> List.skip (List.length values - List.length c.Witnesses)

    let known =
        List.zip c.Witnesses texts
        |> List.map (fun (i, v) ->
            match v with
            | Text key -> i, Scalar.typeOf key
            | _ -> fail "InvalidWitness" "")
        |> Map.ofList

    known |> Map.iter (fun _ t -> checkType t schema)
    c.Fields |> List.map (fun (n, t) -> n, substitute t known)

// ---------------------------------------------------------------------------
// Index terms

let private tokens (text: string) =
    text.Split(' ', StringSplitOptions.RemoveEmptyEntries) |> List.ofArray

let private guard tokens =
    match tokens with
    | ("==" | ">=") :: _ -> true
    | _ -> false

let private indexBinary op (a: BigInteger option) (b: BigInteger option) =
    match a, b with
    | Some a, Some b ->
        match op with
        | "+" -> Some(a + b)
        | "-" when a >= b -> Some(a - b)
        | "*" -> Some(a * b)
        | "div" when b.Sign > 0 -> Some(BigInteger.Divide(a, b))
        | "mod" when b.Sign > 0 -> Some(BigInteger.Remainder(a, b))
        | "^" when b.Sign >= 0 && b <= BigInteger 64 -> Some(BigInteger.Pow(a, int b))
        | _ -> None
    | _ -> None

let rec private evalIndex terms (c: Constructor) (values: Value list) schema =
    match terms with
    | (term: string) :: rest when term.StartsWith "c" -> Some(BigInteger.Parse(term.Substring 1)), rest
    | term :: rest when term.StartsWith "f" || term.StartsWith "p" ->
        let positions, inner =
            match term.Substring(1).Split('.') with
            | [| p |] -> p, 0
            | [| p; i |] -> p, int i
            | _ -> fail "InvalidIndexTerm" term

        let path = positions.Split('/') |> Array.map int |> Array.toList

        if
            inner < 0
            || List.exists ((>) 0) path
            || (term.StartsWith "f" && List.length path <> 1)
        then
            fail "InvalidIndexTerm" term

        Some(indexAtPath c.Fields values path inner schema), rest
    | op :: rest ->
        let a, next = evalIndex rest c values schema
        let b, after = evalIndex next c values schema
        indexBinary op a b, after
    | [] -> fail "InvalidIndexTerm" ""

and private indexAtPath (types: (string * TypeRef) list) (values: Value list) path inner schema =
    match path with
    | position :: rest when position >= 0 && position < List.length types && position < List.length values ->
        let _, t = List.item position types
        let value = List.item position values

        if List.isEmpty rest then
            index value t inner schema
        else
            match t, value, constructors t schema with
            | Named(name, _), Data(tag, fields), Some [ c ] when
                name.StartsWith "lawspec::structural::record::" && c.Tag = tag
                ->
                indexAtPath c.Fields fields rest inner schema
            | _ -> fail "InvalidIndexPath" "index field path must traverse a structural record"
    | _ -> fail "InvalidIndexPath" ""

/// The index at a position of an indexed family's value.
and [<CompiledName("Index")>] index (value: Value) (t: TypeRef) (position: int) (schema: Schema) : BigInteger =
    match value, constructors t schema with
    | Data(tag, values), Some cs ->
        let c0 = findConstructor tag cs

        let c =
            { c0 with
                Fields = witnessed c0 values schema
            }

        let terms = c.Indices |> List.map tokens |> List.filter (guard >> not)

        if position < 0 || position >= List.length terms then
            fail "NoIndex" ""

        match evalIndex (List.item position terms) c values schema with
        | Some v, [] -> v
        | _ -> fail "UndefinedIndex" tag
    | _ -> fail "IndexedTypeRequiresData" (Scalar.typeKey t)

let private checkIndices (c: Constructor) values schema =
    for text in c.Indices do
        match tokens text with
        | (("==" | ">=") as op) :: rest ->
            let a, next = evalIndex rest c values schema
            let b, _ = evalIndex next c values schema

            let holds =
                match a, b with
                | Some a, Some b -> if op = "==" then a = b else a >= b
                | _ -> false

            if not holds then
                fail "RefinementViolation" ("index guard " + text)
        | _ -> ()

// ---------------------------------------------------------------------------
// Validation

let rec private strictOrder values =
    match values with
    | a :: (b :: _ as rest) -> Scalar.compareValues a b = -1 && strictOrder rest
    | _ -> true

let private checkCollection (name: string) (fields: Value list) =
    match name, fields with
    | "lawspec.collections::type::EqSet", [ List items ] ->
        if not (Scalar.distinctMembers items) then
            fail "RefinementViolation" "Eq-only set: duplicate equal elements"
    | "lawspec.collections::type::Set", [ List items ] ->
        if not (strictOrder items) then
            fail "RefinementViolation" name
    | "lawspec.collections::type::KeyVal", [ List entries ] ->
        let keys =
            entries
            |> List.map (function
                | Data(_, [ k; _ ]) -> k
                | _ -> fail "InvalidValue" name)

        if not (strictOrder keys) then
            fail "RefinementViolation" name
    | _ -> ()

let private checkPredicates (c: Constructor) args values schema =
    for predicate in c.Predicates do
        if not (predicate schema args values) then
            fail "RefinementViolation" c.Tag

/// Check a value against a type: shape, field contracts, index guards and the
/// canonical order of collections. Returns the value.
[<CompiledName("Validate")>]
let rec validate (value: Value) (t: TypeRef) (schema: Schema) : Value =
    checkType t schema
    walk value t schema true

and private walk value t schema deep =
    match t, value with
    | Named("List", [ element ]), List items -> List(items |> List.map (fun v -> walk v element schema deep))
    | Named(("Optional" | "Nullable") as kind, [ inner ]), Presence(k, present) when k = kind ->
        Presence(kind, present |> Option.map (fun v -> walk v inner schema deep))
    | Named(name, _), _ ->
        match schema.Definitions.TryFind name with
        | Some d when d.Handle ->
            match value with
            | Handle(n, _) when n = name -> value
            | _ -> fail "InvalidHandle" name
        | _ ->
            match constructors t schema with
            | None -> Scalar.validate value t schema.Bits
            | Some cs -> walkData value t cs schema deep
    | Parameter _, _ -> fail "UnboundSchemaParameter" ""

and private walkData value t cs schema deep =
    match value with
    | Data(tag, fields) ->
        let c = findConstructor tag cs

        if List.length fields <> List.length c.Fields then
            fail "WrongFieldCount" tag

        let typed = witnessed c fields schema

        let converted =
            if deep then
                List.map2 (fun f (_, ft) -> walk f ft schema deep) fields typed
            else
                fields

        let args =
            match t with
            | Named(_, a) -> a
            | _ -> []

        checkPredicates c args converted schema
        checkIndices { c with Fields = typed } converted schema

        checkCollection
            (match t with
             | Named(n, _) -> n
             | _ -> "")
            converted

        Data(tag, converted)
    | _ -> fail "ExpectedData" (Scalar.typeKey t)

/// Build a value from checked fields: the fields are not walked again, but
/// the constructor's contracts are checked.
[<CompiledName("Construct")>]
let construct (tag: string) (fields: Value list) (t: TypeRef) (schema: Schema) : Value =
    match tag, fields with
    | "List::Nil", [] -> List []
    | "List::Cons", [ head; List tail ] -> List(head :: tail)
    | _ ->
        checkType t schema

        match constructors t schema with
        | Some cs -> walkData (Data(tag, fields)) t cs schema false
        | None -> fail "ExpectedData" (Scalar.typeKey t)

/// Branch on a value's constructor.
[<CompiledName("MatchValue")>]
let matchValue (value: Value) (branches: Map<string, Value list -> 'a>) : 'a =
    match value with
    | List [] -> branches.["List::Nil"][]
    | List(head :: tail) ->
        branches.["List::Cons"][head
                                List tail]
    | Data(tag, fields) ->
        match branches.TryFind tag with
        | Some branch -> branch fields
        | None -> fail "MatchFailure" tag
    | _ -> fail "ExpectedData" ""

/// The field types of a checked data value (a recording uses them to tell Text
/// from Bytes and Char from an integer).
[<CompiledName("FieldTypes")>]
let fieldTypes (value: Value) (t: TypeRef) (schema: Schema) =
    match value, constructors t schema with
    | Data(tag, values), Some cs ->
        let c = findConstructor tag cs
        witnessed c values schema |> List.map snd
    | _ -> fail "ExpectedData" (Scalar.typeKey t)

// ---------------------------------------------------------------------------
// Payloads

type private Recipe =
    | NoPayload
    | Predicate of (Value -> bool)
    | Container of string * Recipe list

let rec private recipe (t: TypeRef) (args: Recipe list) =
    match t with
    | Parameter i -> if i < List.length args then List.item i args else NoPayload
    | Named(name, fields) ->
        let children = fields |> List.map (fun f -> recipe f args)

        if
            children
            |> List.forall (function
                | NoPayload -> true
                | _ -> false)
        then
            NoPayload
        else
            Container(name, children)

let rec private payload (r: Recipe) (value: Value) (schema: Schema) =
    match r, value with
    | NoPayload, _ -> true
    | Predicate p, _ -> p value
    | Container("List", [ inner ]), List items -> items |> List.forall (fun v -> payload inner v schema)
    | Container(_, [ _ ]), Presence(_, None) -> true
    | Container(_, [ inner ]), Presence(_, Some v) -> payload inner v schema
    | Container("Maybe", _), Data("Maybe::Absent", []) -> true
    | Container("Maybe", [ inner ]), Data("Maybe::Present", [ v ]) -> payload inner v schema
    | Container("Either", [ inner; _ ]), Data("Either::Left", [ v ]) -> payload inner v schema
    | Container("Either", [ _; inner ]), Data("Either::Right", [ v ]) -> payload inner v schema
    | Container(name, args), Data(tag, values) ->
        let c = findConstructor tag schema.Definitions.[name].Constructors
        List.forall2 (fun (_, ft) v -> payload (recipe ft args) v schema) c.Fields values
    | _ -> fail "InvalidPayload" ""

/// Whether every payload of a container (each value at a type parameter)
/// satisfies that parameter's predicate.
[<CompiledName("AllPayloads")>]
let allPayloads (value: Value) (t: TypeRef) (predicates: (Value -> bool) list) (schema: Schema) =
    match t with
    | Named(name, args) ->
        if List.length args <> List.length predicates then
            fail "PayloadArityMismatch" name

        validate value t schema |> ignore
        payload (Container(name, predicates |> List.map Predicate)) value schema
    | Parameter _ -> fail "UnboundSchemaParameter" ""

// ---------------------------------------------------------------------------
// Builders, as the generated schema module writes a program's data

/// A field's name and type (by key).
[<CompiledName("Field")>]
let field (name: string) (key: string) = name, Scalar.typeOf key

/// A data definition.
[<CompiledName("Definition")>]
let definition (name: string) (parameters: int) (constructors: Constructor list) (handle: bool) =
    {
        Name = name
        Parameters = parameters
        Constructors = constructors
        Handle = handle
    }

[<CompiledName("WithPredicates")>]
let withPredicates predicates (c: Constructor) = { c with Predicates = predicates }

[<CompiledName("WithGenerators")>]
let withGenerators (generators: (int * (Schema -> TypeRef list -> Value list -> FieldHints)) list) (c: Constructor) =
    { c with
        Generators = Map.ofList generators
    }

[<CompiledName("WithIndices")>]
let withIndices indices (c: Constructor) = { c with Indices = indices }

[<CompiledName("WithRefinements")>]
let withRefinements refinements (c: Constructor) = { c with Refinements = refinements }

[<CompiledName("WithExistentials")>]
let withExistentials count (c: Constructor) = { c with Existentials = count }

[<CompiledName("WithWitnesses")>]
let withWitnesses witnesses (c: Constructor) = { c with Witnesses = witnesses }

/// A refinement of a parameter, by position, to a type (by key).
[<CompiledName("Refinement")>]
let refinement (index: int) (key: string) = index, Scalar.typeOf key

/// A constructor contract's known parameter types, by position.
[<CompiledName("Known")>]
let known (types: TypeRef list) =
    types |> List.mapi (fun i t -> i, t) |> Map.ofList

/// A field's generation hints.
[<CompiledName("Hints")>]
let hints bounds lengths values =
    {
        Bounds = bounds
        Lengths = lengths
        Hints = values
    }
