/// Cancellation scopes: a scope owns the work started beneath it, including
/// nested scopes, and closing it cancels that work. .NET cannot kill a
/// running task, so cancellation is cooperative: a scope's token is the one
/// its work observes, and a closed scope never waits for work that ignores
/// it. ref:DEC-async-native-tasks ref:DEC-typed-core-boundary
module LawSpec.Runtime.Tasks

open System
open System.Threading
open System.Threading.Tasks

/// A cancellation scope and the work it owns.
[<Sealed; AllowNullLiteral>]
type Scope(parent: Scope, parentToken: CancellationToken) =
    let source =
        if isNull (box parent) then
            new CancellationTokenSource()
        else
            CancellationTokenSource.CreateLinkedTokenSource(parentToken)

    let work = ResizeArray<Task>()
    member _.Parent = parent
    member _.Token = source.Token
    member _.IsClosed = source.IsCancellationRequested
    /// Register work started in this scope.
    member _.Own(task: Task) = lock work (fun () -> work.Add task)
    member _.Work = lock work (fun () -> List.ofSeq work)

    /// Cancel everything the scope owns.
    member _.Close() =
        try
            source.Cancel()
        with _ ->
            ()

let private current = AsyncLocal<Scope>()

/// The innermost scope, or null outside any.
[<CompiledName("Current")>]
let currentScope () = current.Value

/// The token of the innermost scope (none outside any).
[<CompiledName("Token")>]
let token () =
    match current.Value with
    | null -> CancellationToken.None
    | scope -> scope.Token

/// Run a body in a new scope, closing it (and cancelling its work) after.
[<CompiledName("WithScope")>]
let withScope (body: Scope -> 'a) : 'a =
    let previous = current.Value

    let scope =
        Scope(
            previous,
            (match previous with
             | null -> CancellationToken.None
             | p -> p.Token)
        )

    current.Value <- scope

    try
        body scope
    finally
        current.Value <- previous
        scope.Close()

/// Start work in the current scope; it sees the caller's ambient context
/// (handlers, workflow runtime) and the scope's token.
[<CompiledName("Start")>]
let start (body: unit -> 'a) : Task<'a> =
    let scope = current.Value

    let task =
        Task.Run(
            (fun () -> body ()),
            (match scope with
             | null -> CancellationToken.None
             | s -> s.Token)
        )

    match scope with
    | null -> ()
    | s -> s.Own task

    task

/// Fail with OperationCanceledException if the current scope was closed.
[<CompiledName("Check")>]
let check () = token().ThrowIfCancellationRequested()
