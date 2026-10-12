---
id: lawtwin.explanation.lawspec-gaps
kind: explanation
title: LawSpec gap report
---
# LawSpec gap report

Writing the Kafka contract in LawSpec 0.21.0 (ref:lawspec) showed what the
language could not state, or could not generate well, for a dependency
contract of this size. LawSpec branch `lawspec-gaps` closes all but one of
the seventeen, and the contract is now written with them. It needs LawSpec
0.23.0, which has those fixes, typed native code, and fixes for most of the
gaps found since. Each gap below has the example that found it, what
the contract does instead, and what would close it. This page is for the
LawSpec roadmap (ref:lawspec-roadmap); LawTwin does not change LawSpec.

The contract is two units: about 1,100 lines of model and 1,500 lines of
abilities, handlers and laws, with 39 laws over seven abilities.

## How the contract is shaped

### 1. One handler cannot serve several abilities, so shared state needs a layer

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `handler cluster for Broker, Admin, GroupCoordinator with state ...` answers all three from one state, so `Admin` and `GroupCoordinator` no longer need twins of the `Broker` requests.

Kafka's guarantees cross its APIs: a record a producer sends is the record a
consumer polls, and offsets sent by a transaction are the offsets a group
fetches. One reference model has to hold all of it. A spec handler handles
one ability, and its state is its own:

```lawspec fragment
handler reference for Broker with state cluster :: Maybe BrokerState start Nothing is ... end
handler kafkaProducer for Producer with state producerState :: Maybe ProducerState start Nothing is
  send pr is let s = producerSend pr (freshProducer producerState) in ... end
end
```

The contract adds a seventh ability, `Broker`, with one operation per wire
request and the whole model as its state; the six client abilities' handlers
are written over it (ref:DEC-kafka-broker-and-clients). That layer suits the
twin, but every client operation has a `Broker` twin of a different name
(`createTopic` and `createTopicsRequest`), because operation names are
unique in a unit.

**Would close it:** a handler for several abilities with one state,
`handler reference for Producer, Consumer, Admin with state ...`.

### 2. A law never runs on spec handlers alone unless it names them

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `handlers reference is cluster, kafkaProducer, kafkaConsumer, kafkaTransactions, murmurPartitioner end` names the reference composition; every law of an ability it covers also runs under all of it.

An ability law runs once per lawful handler of its own ability. For the
other abilities it uses, it takes "the first lawful one", which is the
native handler. So `Producer: records sent to one partition are read in the
order sent [kafkaProducer]` runs the producer's spec handler with the native
`Consumer` and the native `Broker`, and `lawspec evidence` says the
obligation "relies on the native ... Broker handler". No run checks
`kafkaProducer` over `reference` unless a second, top-level copy of the law
names them with `using`.

**Would close it:** a way for a package to say which spec handlers form its
reference composition, run as its own obligation for every ability law.

### 3. Native handlers of one case cannot share a resource

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `ability Broker with Cluster is ... end`: each case acquires one `Cluster` and makes every native handler with it. The test project's session per thread is gone.

The test project's native handlers must reach one cluster per case, but
each is made with no arguments (`ProducerHandler::default()`). Resources
are given to laws, not to handlers. The test project keeps a session per
thread and starts a new one when a handler is made after an operation has
run (ref:DEC-contract-test-shared-cluster). It works because the generated
tests make every handler before the first operation; nothing promises that.

**Would close it:** handlers that take the law's resources, or a per-case
`session` resource that every native handler of the case receives.

### 4. Ability operations cannot fail with a typed failure

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `produceRequest :: ... -> Int64 fails with KafkaError`; spec handler clauses `raise`, native handlers raise the runtime's `Fail`, and laws say `createTopic t n fails with TopicAlreadyExists`.

```lawspec fragment
ability Broker is
  produceRequest :: TopicPartition -> Maybe ProducerTag -> List Record -> Int64 fails with KafkaError
end
```

is a parse error (`unexpected "fai"`), so every fallible request gives
`Either KafkaError a`, and laws compare with `== Left ProducerFenced`
instead of `fails with ProducerFenced`. Nothing aborts, so a law that
chains requests must match each `Left` by hand.

**Would close it:** `fails with` on ability operations, with handlers that
`raise`.

## Handler state

### 5. A handler's start value cannot be an empty map

**Closed** by LawSpec 0.23.0 (commit 34e34a8). A start value may call definitions and the prelude: `start model.emptyBroker unitValue`.

```lawspec fragment
handler reference for Broker with state logs :: KeyVal Text (List Bytes) start prelude.keyValOf [] is ...
```

fails `lawspec check` with `unknown value: lawspecCollectionsKeyValOf`. A
definition as the start value fails differently on each target: Python
generation says `a handler's start value calls nothing`, and Rust
generation says `unknown Rust declaration: ex.k::emptyLogs`. There is no
literal for an empty `KeyVal` or `Set`, so a state with one cannot be
written as a constructor either. Every handler here starts from `Nothing`
and reads it as the empty state with a `current` definition.

**Would close it:** literals for empty collections, and start values that
may call checked definitions.

### 6. State is updated only before the result, never in a branch

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `produceRequest by model.produce` makes a clause of a definition that gives `Pair result next`, and a tail `match` or `if` may update the state in its branches.

```lawspec fragment
append t v is
  match prelude.lookup t logs with
  | Nothing -> Left UnknownTopic
  | Just l -> ~logs := prelude.put t (Cons v l) logs; Right 0
  end
end
```

fails with `~logs may appear only on the left of :=`. Each request is
therefore a definition giving a `Step` of its result and next state, and
each clause repeats the same line:

```lawspec fragment
produceRequest tp tag batch is
  let s = model.produce tp tag batch (model.current cluster) in ~cluster := Just (model.stateOf s); model.resultOf s
end
```

Seventeen clauses repeat it. The `Step` form is also what the twin wants to
call.

**Would close it:** a clause written as a transition,
`produceRequest by model.produce`, as `model` declarations already map a
command to a definition giving `Pair result next`.

### 7. No field access or record update

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `b.logs` reads a field and `b with logs = v` replaces one; the fourteen accessors are gone.

`BrokerState` has seven fields. Reading or replacing one is a `match` that
names all seven, so the model has fourteen accessor definitions:

```lawspec fragment
definition withLogs (v :: KeyVal TopicPartition PartitionLog) (b :: BrokerState) :: BrokerState is
  match b with | BrokerState t l p x n g c -> BrokerState t v p x n g c end
end
```

**Would close it:** field access (`b.logs`) and update
(`b with logs = v`) on single-constructor types.

## Totality

### 8. No recursion on integers

**Closed** by LawSpec 0.23.0 (commit 34e34a8). A recursive call may move an integer toward a bound, and the prelude has `range`; the 1024-partition cap stays as a model limit (ref:DEC-kafka-model-scope), no longer a language one.

```lawspec fragment
definition upTo (n :: Natural) :: List Natural is
  if n == 0 then [] else Cons n (upTo (n - 1))
end
```

is rejected: `recursive call has no strict structural descent`. The
prelude has no range. A topic's partitions, 0 to n - 1, are built by
splitting a list of powers of two (`rangeIn [512, 256, ..., 1] 0 n`), which
caps a topic at 1024 partitions (ref:DEC-kafka-model-scope).

**Would close it:** `prelude.range`, or recursion on a natural number that
decreases.

### 9. The prover forgets what it knows about a call's result

**Closed** by LawSpec 0.23.0 (commit 34e34a8). The totality checker keeps what guards say about calls, fields and lengths, knows lengths are not negative, and knows the bounds of `quot` and `rem`.

Each of these is rejected with `integer conversion range could not be
proved`:

```lawspec fragment
definition clip (xs :: List Int32) :: Int32 is
  if prelude.length xs >= 0 && prelude.length xs <= 100 then prelude.Int32 (prelude.length xs) else 100
end

definition clip2 (n :: Int32) :: Int32 is
  let m = dec n in if m >= 1 then prelude.Int32 (m - 1) else 0
end

definition ownerIndex (p :: Int32) (k :: Int32) :: Int32 is
  if k >= 1 && p >= 0 then prelude.Int32 (prelude.rem p k) else 0
end
```

A guard on a parameter, or on a name bound to a parameter, is used; a guard
on a call's result is not, even on the same call; and the prover does not
know that `rem p k` lies between 0 and `k`. The model counts a batch's
records with its own recursive definition rather than `prelude.length`,
moves sequence arithmetic into a definition of its own, and assigns
partitions by walking the subscribers in turn instead of `p mod k`.

**Would close it:** facts about equal calls of pure helpers, the range and
non-negativity of `prelude.length`, and the bounds of `quot` and `rem`.

### 10. Every counter needs a guard against overflow

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `prelude.wrapping`, `prelude.saturating` and `prelude.bounded` put a result into the bounded type the context wants; `next64`, `next32` and `next16` are gone.

Offsets are `Int64`, epochs `Int16` and generations `Int32`, as on the wire.
Every increment needs a saturating helper, `next64`, `next32` and `next16`,
because `n + 1` may leave the type. Kafka itself wraps or refuses at these
limits; the model saturates, which no law yet checks.

**Would close it:** wrapping and checked arithmetic as named operations.

## Data

### 11. Bytes cannot be read

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `prelude.octets` reads a `Bytes` as a list of `UInt8`, and `murmur2` is now written in the contract and checked against Kafka's vectors. On Rust a spec handler cannot yet call it; see below.

`Bytes` has no operations beyond equality, order and `prelude.length`. Kafka
picks a keyed record's partition with murmur2 over the key's octets
(ref:kafka-murmur2); that cannot be written in LawSpec. The `Partitioner`
ability's spec handler always chooses partition 0, which is lawful but says
nothing, and the hash is native code in each test project, checked against
Kafka's own test vectors.

**Would close it:** octet access and folds over `Bytes`, or built-in hashes
with laws, as `lawspec.crypto` has for SHA-3.

### 12. Text cannot be built

**Closed** by LawSpec 0.23.0 (commit 34e34a8). `++` concatenates `Text`, and `textOf` and `join` format it.

There is no text concatenation (`a + b` on `Text` is an error) and no
conversion from numbers. Member ids are therefore supplied by the client, as
KIP-848 has them, rather than made by the coordinator as the classic
protocol does (ref:kafka-classic-group-protocol).

**Would close it:** concatenation and number formatting for `Text`.

### 13. A prelude helper is unknown inside an ability's laws

**Closed** by LawSpec 0.23.0 (commit 34e34a8). Prelude helpers work in an ability's laws and in handler clauses; `succeeded` is gone and laws use `prelude.isRight`.

`prelude.isRight (get x)` in an ability's `laws` section fails with
`unknown helper or wrong arity: prelude.isRight`; the same law at the top
level of the unit passes. The contract defines its own `succeeded`.

**Would close it:** the prelude's helpers in ability laws as everywhere else.

### 14. A literal in a `let` does not take its type from its use

**Closed** by LawSpec 0.23.0 (commit 34e34a8). A numeric literal in a branch takes its type from the other branches.

```lawspec fragment
let from = (match g with | Nothing -> 0 | Just group -> ... end) in
Step from (withPosition tp from c)
```

fails with `type mismatch: Integer and Int64`; the literal needs
`(0 :: Int64)`.

## Generation

### 15. Refinements filter; they do not generate

**Closed** by LawSpec 0.23.0 (commit 34e34a8). A text refined by a regex, directly or through a checked definition such as `model.validTopicName`, is generated from texts the regex matches instead of filtered from any text. The test project binds no generators, and every law passes Python's health checks.

Laws draw topic names as `(t :: Text where model.validTopicName t)`, and
the model's rule ends `name matches regex "[a-zA-Z0-9._-]+"`. Generated
from any text, almost no draw passes, and on Python every such law fails
Hypothesis's `filter_too_much` health check. The same happened to partition
counts, `(n :: Int32 where n >= 1 && n <= 64)`, to batches of one to five
records, and to lists of one to four distinct member ids; Rust's Proptest
was more patient.

The contract widens batches to 1 to 100 records and derives member ids
from any list of texts. The test project binds generators for `Text` and
`Int32` in `lawspec.json`: three draws in four are a name or a count from
1 to 16, and the fourth is any non-empty text or any integer. A binding is
per type, so every `Text` and every `Int32` the laws draw comes from it; a
consumer of the contract has to bind the same generators to run its laws.

**Would close it:** generation that follows a refinement, from a regex and
from integer bounds, as indexed families are built rather than filtered;
or a generator named on the quantifier, in the contract.

## Generated code

### 16. The reference model is interpreted, and copies its state on every call

**Open.** LawSpec branch `lawspec-codegen` generates typed Rust, Go, Java and Kotlin for checked definitions and spec handlers; it is not yet merged.

In Rust, a spec handler's clause evaluates the model over the dynamic
`ls::Value`, with constructors named by strings
(`ls::construct_data("lawspec.collections::type::Pair::Pair", ...)`), and
starts each call with `self.state.lock().unwrap().clone()`: a deep copy of
the whole cluster. A `List` is a `Vec`, so `Cons` copies, and a `KeyVal` is
a sorted list of entries searched by generated LawSpec code
(`lawspec_lawspecCollectionsKeyValLookupItems_...`), so a lookup is linear.
Every request therefore costs at least the size of the cluster.

For this contract `generate` writes 83,535 lines of
`lawspec_definitions.rs` and 4,417 of `lawspec_abilities.rs`, with every
spec handler copied into each unit that imports it. `lawspec test` runs the
78 law runs in about 250 seconds on Rust and 70 on Python.

The Kafka twin embeds this code (ref:DEC-twins-in-rust). As generated, it
is a correct oracle but not a server: a partition with a million records
would be copied on every produce.

**Would close it:** typed Rust for checked definitions over the generated
data types, persistent or in-place collections for handler state, and
`KeyVal` as a map.

### 17. A unit cannot sit above its package's other units on Python

**Closed** by LawSpec 0.23.0 (commit 34e34a8). A unit named like another unit's package is that package's `__init__.py`, so the contract's facade unit is `lawtwin.kafka`.

A package with units `lawtwin.kafka` and `lawtwin.kafka.model` fails Python
generation with `a Python unit module shadows another unit's package`:
`lawtwin/kafka.py` and `lawtwin/kafka/`. The facade unit the packages guide
recommends cannot be named after the package. The contract's units are
`lawtwin.kafka.model` and `lawtwin.kafka.contract`.

**Would close it:** emitting a unit with sub-units as the package's
`__init__.py`.

## Time

The model has no clock, so group members never time out and transactions
never expire. LawSpec has the `Clock` ability and the virtual clock for
this; the 0.1 model leaves it out (ref:DEC-kafka-model-scope), and the gap,
if any, is not yet known.

## Found while re-expressing the contract

Running the re-expressed contract on the `lawspec-gaps` build (34e34a8) on
five targets found five more, and writing the OpenAPI importer found five
after them. None changes what a contract says.

### 18. On Rust, a failing spec handler clause poisons the handler

**Closed** by LawSpec 0.23.0 (commit 1db7298). A failing clause no longer poisons the handler; all 79 Rust runs of the Kafka contract pass.

A spec handler's clause runs with its context locked. When the clause
fails, the failure unwinds through the lock, and every later call on the
handler panics with `PoisonError`. Any law that expects a failure and then
uses the handler again fails on Rust only:

```lawspec fragment
handler tally for Counter with state total :: Int64 start 0 is
  add by bump
end
-- `a refused add changes nothing`: add (0 - 1) fails, then add 1 panics
```

Twelve of the contract's Rust runs fail this way; Python, Go, Java and
Kotlin pass them.

**Would close it:** release the lock before raising, or recover a poisoned
lock as the clause's failure has already been reported.

### 19. On Rust, a spec handler cannot call `prelude.octets`

**Closed** by LawSpec 0.23.0 (commit e99a0bc): the murmur2 partitioner's Rust runs pass.

A definition that calls `prelude.octets` passes as a top-level law, but
when a spec handler's clause calls it, Rust's evaluator stops with
`unknown helper: octets`. The six Rust runs of `murmurPartitioner`, and of
the producer laws that use it, fail this way.

**Would close it:** the sequence helpers in the Rust evaluator.

### 20. A package's units do not bring their collections helpers

**Closed** by LawSpec 0.23.0 (commit 66a7e2f).

A test project that imports `lawtwin.kafka` as a package fails `lawspec
check` with `unknown core declaration:
contracttest.kafka::lawspecCollectionsSaturatingInt64`. LawSpec builds a
program's collections unit from the sources it is given, not from the
packages they import, so bounded arithmetic the model uses is declared only
when a source of the project names it. The test project's suite holds a
definition that calls `prelude.saturating` for that reason.

**Would close it:** building the collections unit from every unit of the
program, packaged ones included.

### 21. On Java, a field cannot be named `record`

**Closed** by LawSpec 0.23.0 (commit d40ea24): Java record components may be named `record`, `sealed`, `permits` or `var`.

`FetchedRecord`'s field `record` fails Java generation with `invalid Java
data identifier: record`; `record` is a contextual keyword in Java. The
field is now `entry`.

**Would close it:** escaping or renaming Java's reserved and contextual
words, as the other targets do.

### 22. The branch's npm package is not rebuilt

The `lawspec-gaps` checkout's `npm/core.wasm` is from before the branch, so
`npx` cannot run it. `tools/stage-lawspec.sh` stages the checkout's CLI with
its native compiler instead; see [Run a contract's
laws](../how-to/run-contract-laws.md).

## Found while writing the OpenAPI importer

### 23. A regex field contract is not proved through a record update

**Closed** by LawSpec 0.23.0 (commit 1db7298).

```lawspec fragment
type Pet is
  Pet
    id :: Maybe Int64
    name :: (v :: Text where v matches regex "[a-z]+")
end

definition withId (body :: Pet) (key :: Int64) :: Pet is body with id = Just key end
```

fails with `constructor field refinement could not be proved`, though
`name` is copied unchanged. The same holds for an explicit `match` and
rebuild, and for a guard `if body.name matches regex "[a-z]+" then ...`.
Length and numeric bounds on fields are proved. The importer therefore
keeps every constraint out of its types, as validation
(ref:DEC-openapi-constraints-as-validation).

**Would close it:** the facts of a field's refinement, regexes included,
for the fields a record update or a rebuild copies.

### 24. A definition that says `fails with` but never raises breaks `by`

**Closed** by LawSpec 0.23.0 (commit 1db7298).

A clause `addPet by model.modelAddPet`, where `modelAddPet` says `fails
with ApiError` and has no `raise`, fails with `core: referenceAddPet: it
uses lawspec::ability::Fail(ApiError) (through modelAddPet), but its row is
empty`. The importer writes `fails with` only on definitions that raise.

**Would close it:** a failing row taken from the definition's signature,
whether or not its body raises.

### 25. A refinement in a data type cannot call a definition

**Closed** by LawSpec 0.23.0 (commit 1db7298).

`name :: (v :: Text where validName v)` in a `type` fails with `unknown
value: validName`, where a law's input may say the same. The importer
writes predicates inline.

**Would close it:** checked definitions in field refinements, as in
inputs.

### 26. On Rust, a handler named like its ability does not compile

**Closed** by LawSpec 0.23.0 (commit 1db7298): the handler's struct is `<Name>Spec`.

A spec handler `counter` for an ability `Counter` becomes a Rust struct
`Counter`, beside the trait `Counter`: `the name Counter is defined
multiple times`. The importer names its handler `reference`, and its
abilities `<Tag>Api`.

**Would close it:** distinct Rust names for a spec handler's struct and
the ability's trait.

### 27. Laws filter records whose fields a regex constrains

**Closed** by LawSpec 0.23.0 (commit 1db7298): boundary witnesses for a regex field come from the regex.

A record input with a regex-constrained field, `(body :: Pet where
model.validPet body)`, is drawn from any record and filtered: Proptest
gives up with `Too many local rejects`, and Hypothesis fails its
`filter_too_much` health check. A text input refined by the same regex is
drawn from it. The importer's laws draw each constrained top-level field
as a refined input and put it into the record.

**Would close it:** generating a record's fields from the regexes a
refinement of the record states.

## Found while embedding the model in the Kafka twin

The Kafka twin's Rust core runs the code LawSpec branch `lawspec-lawtwin`
(e99a0bc) generates for the contract, through the embedding API: the spec
handler `Cluster::start()` and its `Broker` trait (ref:DEC-embed-generated-model).
At about a million operations a second the generated model is fast enough
to be the broker. Four things made the twin go around the API rather than
through it.

### 28. A failing operation of an embedded handler panics

`produceRequest` says `fails with KafkaError`, but the generated trait method
returns `i64`: a refusal panics with the raised value (`ls::Failure`), which
a law catches and an application must too. The twin calls the typed
definitions itself and catches the panic around each call, giving the
`KafkaError` back as a value.

**Would close it:** for an operation that `fails with E`, an embedding
method that returns `Result<T, E>` (an `error` return in Go, a checked
exception on the JVM), beside the trait method laws use.

### 29. An embedded handler's state cannot be read or replaced

`Cluster` keeps its state in a private field, so an application can neither
show it (the control plane's `inspect`) nor set it (`force`, a reset, a
snapshot to roll back to). The twin keeps the typed state itself.

**Would close it:** `state()` and `with_state(s)` (or `snapshot` and
`restore`) on a handler with state, in the native types, and cheap access
to the typed state for an application that keeps it.

### 30. The typed definitions an embedding needs have generated names

The handler's clauses call `typed::clusterProduceRequest_41`,
`typed::clusterStart_42` and so on: names numbered by the generator, which
change when the contract does. The twin's build script reads them from the
generated `impl Broker for Cluster` rather than naming them.

**Would close it:** with gap 28 and 29, nothing further; otherwise stable
names for a spec handler's clause and start functions.

### 31. A generated crate warns in every crate that depends on it

The contract's Rust project compiles with hundreds of `unused import`
warnings (`pub(crate) use self::evaluate_38 as evaluate_39`), which Cargo
shows in every build of a crate that depends on it by path, as the twin
does.

**Would close it:** `#![allow(unused_imports)]` on the generated modules,
as their other allowances are.

### 32. Generation is not the same from one run to the next

On branch `lawspec-lawtwin` (e99a0bc), `lawspec generate` of one project
gives different code from one run to the next: which definitions take the
typed native path changes. Regenerating the Kafka test project's Go target,
with nothing changed, rewrote 9 files (about 13,000 lines of
`lawspec_typed.go` and `lawspec_definitions.go`); its Python target and the
Petstore project's flip the same way. Both versions pass every law, but
`make contract-generate-check` fails on a tree that is current, depending
on the compiler's cache.

**Would close it:** a typed-path choice that depends only on the program,
not on what the cache holds.

## Found running the laws on F# and C#

### 33. On F# and C#, a law with several non-empty texts draws no input

**Closed** by LawSpec 0.23.0 (commit c0f87ce). The .NET harness grows the size with passed cases and with discards, as QuickCheck does, and draws a `Text where x != ""` from its refinement as a minimum length of one; all 79 Kafka runs pass on F# and on C#.

At LawSpec commit f37e577, before 0.23.0, four Kafka laws stop on F# and C#
with `CannotSatisfy` after 10,000 draws, under every handler, the spec
handlers too: `a member of an older generation cannot commit`, `a commit
from an older generation is refused`, `a heartbeat from an older
generation asks the member to rejoin` and `a restarted consumer resumes at
its committed offset`, 8 runs of 79. Each draws a valid topic name and two
or three `Text where x != ""`, one of them also unlike another. The .NET
harness draws the first case at size 0 and keeps that size while it
discards, and at size 0 a text is empty unless a witness is chosen, one
draw in ten, so these domains are almost never met. The same laws pass on
the other five targets.

**Would close it:** growing the size as draws are discarded, or drawing a
`Text where x != ""` from its refinement, as topic names are.
