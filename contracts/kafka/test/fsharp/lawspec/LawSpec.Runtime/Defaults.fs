/// The default handlers of the built-in abilities on .NET: the real clock,
/// the run's seeded Random and the operating system's SecureRandom, files,
/// environment variables and ports, logging, and pausing. Values are
/// portable; the generated interfaces convert them where native code meets
/// them. ref:DEC-typed-core-boundary ref:DEC-fsharp-runtime-from-scratch
module LawSpec.Runtime.Defaults

open System
open System.Diagnostics
open System.IO
open System.Numerics
open System.Security.Cryptography
open System.Text.Json

let private fail code detail : 'a = Fail.raiseWith code detail

let private just v = Data("Maybe::Present", [ v ])
let private nothing = Data("Maybe::Absent", [])

// The wall clock at start, then the monotonic clock: a later clock
// correction never moves time backwards.
let private origin = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() * 1000L
let private started = Stopwatch.GetTimestamp()

/// Microseconds since the Unix epoch, never decreasing.
[<CompiledName("NowMicros")>]
let nowMicros () : int64 =
    let elapsed = Stopwatch.GetElapsedTime(started)
    origin + int64 elapsed.Ticks / 10L

/// Sleep at least a number of microseconds.
[<CompiledName("SleepMicros")>]
let sleepMicros (micros: int64) =
    let deadline = Stopwatch.GetTimestamp() + micros * Stopwatch.Frequency / 1_000_000L
    let mutable left = deadline - Stopwatch.GetTimestamp()
    while left > 0L do
        let ms = max 1L ((left * 1000L + Stopwatch.Frequency - 1L) / Stopwatch.Frequency)
        Threading.Thread.Sleep(TimeSpan.FromMilliseconds(float ms))
        left <- deadline - Stopwatch.GetTimestamp()

let private integer (v: Value) =
    match v with
    | Int n -> n
    | _ -> fail "IntegerRequired" (sprintf "%A" v)

let private text (v: Value) =
    match v with
    | Text s -> s
    | _ -> fail "TextRequired" (sprintf "%A" v)

let private mask = (BigInteger.One <<< 64) - BigInteger.One

let private randomStep (s: BigInteger) = (BigInteger 6364136223846793005L * s + BigInteger 1442695040888963407L) &&& mask

let private secureBelow (bound: int64) =
    if bound < 1L then 0L
    else
        let range = UInt64.MaxValue - (UInt64.MaxValue % uint64 bound + 1UL) % uint64 bound
        let mutable draw = 0UL
        let buffer = Array.zeroCreate<byte> 8
        let mutable found = false
        while not found do
            RandomNumberGenerator.Fill(Span<byte>(buffer))
            draw <- BitConverter.ToUInt64(buffer, 0)
            found <- draw <= range
        int64 (draw % uint64 bound)

let private temporaryRoot () =
    let candidates =
        [ "TMPDIR"; "TMP"; "TEMP" ]
        |> List.choose (fun n -> match Environment.GetEnvironmentVariable n with null | "" -> None | p -> Some p)
    match (candidates @ [ Path.GetTempPath() ]) |> List.tryFind Directory.Exists with
    | Some root -> Path.GetFullPath root
    | None -> fail "NoTemporaryDirectory" ""

let private temporary (directory: bool) (prefix: string) =
    if prefix.IndexOfAny([| char 0; '/'; '\\' |]) >= 0 then fail "InvalidTemporaryPrefix" prefix
    let rec attempt n =
        if n = 0 then fail "TemporaryNameExhausted" prefix
        let name = prefix + Convert.ToHexString(RandomNumberGenerator.GetBytes 16).ToLowerInvariant()
        let path = Path.Combine(temporaryRoot (), name)
        if Directory.Exists path || File.Exists path then attempt (n - 1)
        else
            if directory then
                Directory.CreateDirectory(path) |> ignore
                if not (OperatingSystem.IsWindows()) then File.SetUnixFileMode(path, UnixFileMode.UserRead ||| UnixFileMode.UserWrite ||| UnixFileMode.UserExecute)
            else
                use _ = new FileStream(path, FileMode.CreateNew, FileAccess.Write)
                if not (OperatingSystem.IsWindows()) then File.SetUnixFileMode(path, UnixFileMode.UserRead ||| UnixFileMode.UserWrite)
            path
    attempt 100

let private validName (name: string) = name.Length > 0 && name.IndexOfAny([| char 0; '=' |]) < 0

let private restoreEnvironment (saved: string) =
    let decoded =
        try JsonSerializer.Deserialize<Collections.Generic.Dictionary<string, string>>(saved)
        with _ -> fail "InvalidEnvironmentSnapshot" ""
    if isNull decoded || decoded |> Seq.exists (fun kv -> not (validName kv.Key) || isNull kv.Value || kv.Value.Contains(char 0)) then
        fail "InvalidEnvironmentSnapshot" ""
    for entry in Environment.GetEnvironmentVariables() |> Seq.cast<Collections.DictionaryEntry> do
        let name = string entry.Key
        if not (decoded.ContainsKey name) then Environment.SetEnvironmentVariable(name, null)
    for kv in decoded do
        Environment.SetEnvironmentVariable(kv.Key, kv.Value)

/// A crypto ability's default handler, set by LawSpec.Runtime.Crypto when a
/// program uses lawspec.crypto.
let mutable cryptoHandler: string -> Effects.Handler = fun key -> fail "UnknownDefaultHandler" key

/// The default handler of a built-in ability, by its key.
[<CompiledName("Handler")>]
let handler (key: string) : Effects.Handler =
    match key with
    | k when k.StartsWith "lawspec.crypto::ability::" -> cryptoHandler k
    | "lawspec.time::ability::Clock" ->
        Effects.stateless
            [ "now", (fun _ _ -> Data("lawspec.time::type::Instant::Instant", [ Int(BigInteger(nowMicros ())) ]))
              "sleep",
              (fun _ args ->
                  match args with
                  | [ Data(_, [ Int micros ]) ] ->
                      sleepMicros (int64 micros)
                      Unit
                  | _ -> fail "InvalidDuration" "") ]
    | "lawspec.randomness::ability::Random" ->
        let seed =
            match Environment.GetEnvironmentVariable "LAWSPEC_SEED" with
            | null | "" -> BigInteger.Zero
            | s -> (try BigInteger.Parse s with _ -> BigInteger.Zero)
        let cell = Effects.Cell(Int(seed &&& mask))
        Effects.stateless
            [ "randomBelow",
              (fun _ args ->
                  let bound = integer (List.head args)
                  cell.Update(fun state ->
                      let first = randomStep (integer state)
                      let second = randomStep first
                      let bits = ((first >>> 32) <<< 32) ||| (second >>> 32)
                      let draw = if bound.Sign > 0 then BigInteger.Remainder(bits, bound) else BigInteger.Zero
                      Int draw, Int second)) ]
    | "lawspec.randomness::ability::SecureRandom" ->
        Effects.stateless
            [ "secureBytes", (fun _ args -> Bytes(RandomNumberGenerator.GetBytes(max 0 (int (integer (List.head args))))))
              "secureBelow", (fun _ args -> Int(BigInteger(secureBelow (int64 (integer (List.head args))))))
              "secureToken", (fun _ _ -> Text(Convert.ToHexString(RandomNumberGenerator.GetBytes 32).ToLowerInvariant())) ]
    | "lawspec.host::ability::FileSystem" ->
        Effects.stateless
            [ "readBytes", (fun _ args -> let p = text (List.head args) in if File.Exists p then just (Bytes(File.ReadAllBytes p)) else nothing)
              "writeBytes",
              (fun _ args ->
                  match args with
                  | [ Text p; Bytes b ] ->
                      File.WriteAllBytes(p, b)
                      Unit
                  | _ -> fail "InvalidArguments" "writeBytes")
              "pathExists", (fun _ args -> let p = text (List.head args) in Bool(File.Exists p || Directory.Exists p))
              "removePath",
              (fun _ args ->
                  let p = text (List.head args)
                  if Directory.Exists p then Directory.Delete(p, true)
                  elif File.Exists p then File.Delete p
                  Unit)
              "temporaryDirectory", (fun _ args -> Text(temporary true (text (List.head args))))
              "temporaryFile", (fun _ args -> Text(temporary false (text (List.head args)))) ]
    | "lawspec.host::ability::Environment" ->
        Effects.stateless
            [ "environmentVariable",
              (fun _ args ->
                  let name = text (List.head args)
                  if not (validName name) then nothing
                  else
                      match Environment.GetEnvironmentVariable name with
                      | null -> nothing
                      | value -> just (Text value))
              "environmentSnapshot",
              (fun _ _ ->
                  let all = Collections.Generic.SortedDictionary<string, string>(Portable.textOrder)
                  for entry in Environment.GetEnvironmentVariables() |> Seq.cast<Collections.DictionaryEntry> do
                      all.[string entry.Key] <- string entry.Value
                  Text(JsonSerializer.Serialize all))
              "restoreEnvironment",
              (fun _ args ->
                  restoreEnvironment (text (List.head args))
                  Unit) ]
    | "lawspec.host::ability::Ports" ->
        Effects.stateless
            [ "freePort",
              (fun _ _ ->
                  let listener = new Net.Sockets.TcpListener(Net.IPAddress.Loopback, 0)
                  listener.Start()
                  try Int(BigInteger((listener.LocalEndpoint :?> Net.IPEndPoint).Port))
                  finally listener.Stop()) ]
    | "lawspec.logging::ability::Log" ->
        Effects.stateless
            [ "logMessage",
              (fun _ args ->
                  match args with
                  | [ Data(level, []); Text message ] ->
                      let name = level.Substring(level.LastIndexOf("::") + 2)
                      let line = sprintf "[%s] %s" (name.ToLowerInvariant()) message
                      if name = "Warning" || name = "Error" then eprintfn "%s" line else printfn "%s" line
                      Unit
                  | _ -> fail "InvalidArguments" "logMessage") ]
    | "lawspec.logging::ability::Trace" ->
        Effects.stateless
            [ "traceEvent",
              (fun _ args ->
                  Trace.WriteLine(text (List.head args), "lawspec")
                  Unit) ]
    | "lawspec.concurrent::ability::Async" ->
        Effects.stateless
            [ "pause",
              (fun _ _ ->
                  Threading.Thread.Yield() |> ignore
                  Unit) ]
    | _ -> fail "UnknownDefaultHandler" key
