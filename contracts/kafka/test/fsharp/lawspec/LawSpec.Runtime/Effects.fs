/// Abilities on .NET: lexically scoped handlers, carried in the schema, and
/// typed failures. A handler is the logical view of an ability's operations;
/// native code sees the ability's generated interface, and a native handler
/// becomes a logical one through its generated bridge. State lives in cells
/// owned by the scope that made the handler, so concurrent steps share it.
/// ref:DEC-typed-core-boundary ref:DEC-fsharp-runtime-from-scratch
module LawSpec.Runtime.Effects

open System
open System.Collections.Generic

/// An operation's clause: the schema in scope and the arguments to a result.
type Clause = Schema.Schema -> Value list -> Value

/// A stateful clause also takes the state, and gives back a pair of the
/// result and the next state.
type StatefulClause = Schema.Schema -> Value list -> Value -> Value

/// Mutable state, read and replaced under a lock.
[<Sealed>]
type Cell(initial: Value) =
    let mutable value = initial
    let gate = obj ()
    member _.Read() = lock gate (fun () -> value)
    member _.Write(next: Value) = lock gate (fun () -> value <- next)

    /// Run an update under the cell's lock.
    member _.Update(update: Value -> 'a * Value) =
        lock gate (fun () ->
            let result, next = update value
            value <- next
            result)

/// A log of the operations a recording handler saw, in order.
[<Sealed>]
type Log() =
    let calls = ResizeArray<string * Value list>()

    member _.Add(operation: string, arguments: Value list) =
        lock calls (fun () -> calls.Add((operation, arguments)))

    member _.Calls = lock calls (fun () -> List.ofSeq calls)

type Handler =
    | Stateless of Map<string, Clause>
    | Stateful of Cell * Map<string, StatefulClause>
    | Recording of Log * Handler

/// A handler scope: it owns the handlers of several abilities made once in
/// it, and is closed when its body ends.
[<Sealed>]
type Scope() =
    let shared = Dictionary<string, Handler>()

    member _.Shared(key: string, make: unit -> Handler) =
        lock shared (fun () ->
            match shared.TryGetValue key with
            | true, h -> h
            | _ ->
                let h = make ()
                shared.[key] <- h
                h)

/// A native interface made from a logical handler keeps it, so the handler
/// is recovered, with its state, when the interface crosses back.
type IOrigin =
    abstract Ability: string
    abstract Symbols: Scalar.SymbolContext
    abstract Handler: Handler
    abstract Schema: Schema.Schema

let private fail code detail : 'a = Fail.raiseWith code detail

/// A handler with no state.
[<CompiledName("Stateless")>]
let stateless (clauses: (string * Clause) list) = Stateless(Map.ofList clauses)

/// A handler with state, from its start.
[<CompiledName("Stateful")>]
let stateful (initial: Value) (clauses: (string * StatefulClause) list) =
    Stateful(Cell initial, Map.ofList clauses)

/// A handler that logs every operation, then runs another.
[<CompiledName("Recording")>]
let recording (inner: Handler) = Recording(Log(), inner)

/// Handlers installed in a schema, overriding outer ones.
[<CompiledName("Install")>]
let install (schema: Schema.Schema) (handlers: (string * Handler) list) =
    { schema with
        Handlers =
            handlers
            |> List.fold (fun table (key, h) -> Map.add key (box h) table) schema.Handlers
    }

/// The handler of an ability in scope.
[<CompiledName("Handler")>]
let handler (schema: Schema.Schema) (ability: string) : Handler =
    match schema.Handlers.TryFind ability with
    | Some h -> unbox h
    | None -> fail "MissingHandler" ability

/// Run a body in a new scope with handlers made by factories. A factory
/// failing leaves no handler behind.
[<CompiledName("WithScope")>]
let withScope
    (schema: Schema.Schema)
    (factories: (string * (Schema.Schema -> Handler)) list)
    (body: Schema.Schema -> 'a)
    : 'a =
    let based =
        { schema with
            Scope = Some(box (Scope()))
        }

    let handlers = factories |> List.map (fun (key, make) -> key, make based)
    body (install based handlers)

/// A handler of several abilities is made once in a scope, so they share its
/// state.
[<CompiledName("Shared")>]
let shared (schema: Schema.Schema) (key: string) (make: unit -> Handler) =
    match schema.Scope with
    | Some(:? Scope as scope) -> scope.Shared(key, make)
    | _ -> make ()

let private clause (clauses: Map<string, 'c>) (operation: string) =
    match clauses.TryFind operation with
    | Some c -> c
    | None -> fail "MissingHandlerOperation" operation

/// Run an operation of a handler.
[<CompiledName("Invoke")>]
let rec invoke (h: Handler) (schema: Schema.Schema) (operation: string) (arguments: Value list) : Value =
    match h with
    | Stateless clauses -> (clause clauses operation) schema arguments
    | Stateful(cell, clauses) ->
        let run = clause clauses operation

        if schema.EffectPath |> List.exists (fun c -> obj.ReferenceEquals(c, cell)) then
            fail "RecursiveHandlerCall" operation

        cell.Update(fun state ->
            match
                run
                    { schema with
                        EffectPath = box cell :: schema.EffectPath
                    }
                    arguments
                    state
            with
            | Data(_, [ result; next ]) -> result, next
            | other -> fail "InvalidHandlerResult" (sprintf "%A" other))
    | Recording(log, inner) ->
        log.Add(operation, arguments)
        invoke inner schema operation arguments

/// Perform an operation of an ability through the handler in scope.
[<CompiledName("Perform")>]
let perform (schema: Schema.Schema) (ability: string) (operation: string) (arguments: Value list) =
    invoke (handler schema ability) schema operation arguments

/// How many times a recording handler saw an operation (with arguments, if
/// given).
[<CompiledName("CountCalls")>]
let countCalls (schema: Schema.Schema) (ability: string) (operation: string) (arguments: Value list option) : Value =
    match handler schema ability with
    | Recording(log, _) ->
        log.Calls
        |> List.filter (fun (name, values) ->
            name = operation
            && (match arguments with
                | None -> true
                | Some args -> Scalar.equal (Value.List values) (Value.List args)))
        |> List.length
        |> bigint
        |> Int
    | _ -> fail "RecordingRequired" ability

/// Raise a typed failure of a Fail ability.
[<CompiledName("RaiseFailure")>]
let raiseFailure (ability: string) (failure: Value) : Value = raise (RaisedFailure(ability, failure))

/// Catch a typed failure: Right of the body's value, or Left of the failure.
[<CompiledName("Attempt")>]
let attempt (ability: string) (body: unit -> Value) (right: Value -> Value) (left: Value -> Value) : Value =
    let outcome =
        try
            Choice1Of2(body ())
        with :? RaisedFailure as f when f.Ability = ability ->
            Choice2Of2 f.Failure

    match outcome with
    | Choice1Of2 v -> right v
    | Choice2Of2 f -> left f

/// A native adapter's Result: its value, or its error raised as the failure.
[<CompiledName("OfResult")>]
let ofResult (ability: string) (convert: 'E -> Value) (result: Result<'T, 'E>) : 'T =
    match result with
    | Ok value -> value
    | Error failure -> raise (RaisedFailure(ability, convert failure))

/// Exceptions of mapped .NET types become typed failures; the others pass.
[<CompiledName("NativeFailures")>]
let nativeFailures (ability: string) (body: unit -> Value) (mappings: (exn -> Value option) list) : Value =
    try
        body ()
    with
    | :? RaisedFailure -> reraise ()
    | :? LawSpecException -> reraise ()
    | e ->
        match mappings |> List.tryPick (fun m -> m e) with
        | Some failure -> raise (RaisedFailure(ability, failure))
        | None -> reraise ()

/// A logical failure crossing to native code: the Result its native form returns.
[<CompiledName("NativeTry")>]
let nativeTry (ability: string) (convert: Value -> 'E) (body: unit -> 'T) : Result<'T, 'E> =
    try
        Ok(body ())
    with :? RaisedFailure as f when f.Ability = ability ->
        Error(convert f.Failure)

/// The handler behind a native interface, if it came from a logical one with
/// the same symbols.
[<CompiledName("Recover")>]
let recover (native: obj) (ability: string) (symbols: Scalar.SymbolContext) (make: unit -> Handler) : Handler =
    match native with
    | :? IOrigin as origin when origin.Ability = ability && obj.ReferenceEquals(origin.Symbols, symbols) ->
        origin.Handler
    | :? IOrigin as origin when origin.Ability = ability -> make ()
    | _ -> make ()

let rec private stateCell (h: Handler) =
    match h with
    | Stateful(cell, _) -> cell
    | Recording(_, inner) -> stateCell inner
    | Stateless _ -> fail "StatelessHandler" ""

/// A spec handler's state, through its native interface.
[<CompiledName("HandlerState")>]
let handlerState (native: obj) : Schema.Schema * Value =
    match native with
    | :? IOrigin as origin -> origin.Schema, (stateCell origin.Handler).Read()
    | _ -> fail "StatelessHandler" "a native handler has no LawSpec state"

/// Replace a spec handler's state, through its native interface.
[<CompiledName("SetHandlerState")>]
let setHandlerState (native: obj) (state: Value) =
    match native with
    | :? IOrigin as origin -> (stateCell origin.Handler).Write state
    | _ -> fail "StatelessHandler" "a native handler has no LawSpec state"

/// A cell native code can use for a handler's state.
[<CompiledName("NewCell")>]
let newCell (initial: Value) = Cell initial

/// A production handler made with a resource, asked for where no case has one.
[<CompiledName("ResourceRequired")>]
let resourceRequired (ability: string) : Handler = fail "ResourceRequired" ability

/// A mapping for nativeFailures: an exception of type 'E becomes a failure.
[<CompiledName("MapException")>]
let mapException<'E when 'E :> exn> (make: 'E -> Value) : exn -> Value option =
    // Bound first, so the function compiles with one argument for C#.
    let mapping (e: exn) =
        match e with
        | :? 'E as caught -> Some(make caught)
        | _ -> None

    mapping
