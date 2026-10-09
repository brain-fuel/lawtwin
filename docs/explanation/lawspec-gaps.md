---
id: lawtwin.explanation.lawspec-gaps
kind: explanation
title: LawSpec gap report
---
# LawSpec gap report

Writing the Kafka contract in LawSpec 0.21.0 (ref:lawspec) showed what the
language could not state, or could not generate well, for a dependency
contract of this size. Each gap below has the example that found it, what
the contract does instead, and what would close it. This page is for the
LawSpec roadmap (ref:lawspec-roadmap); LawTwin does not change LawSpec.

The contract is two units: about 1,100 lines of model and 1,500 lines of
abilities, handlers and laws, with 39 laws over seven abilities.

## How the contract is shaped

### 1. One handler cannot serve several abilities, so shared state needs a layer

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

The test project's native handlers must reach one cluster per case, but
each is made with no arguments (`ProducerHandler::default()`). Resources
are given to laws, not to handlers. The test project keeps a session per
thread and starts a new one when a handler is made after an operation has
run (ref:DEC-contract-test-shared-cluster). It works because the generated
tests make every handler before the first operation; nothing promises that.

**Would close it:** handlers that take the law's resources, or a per-case
`session` resource that every native handler of the case receives.

### 4. Ability operations cannot fail with a typed failure

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

Offsets are `Int64`, epochs `Int16` and generations `Int32`, as on the wire.
Every increment needs a saturating helper, `next64`, `next32` and `next16`,
because `n + 1` may leave the type. Kafka itself wraps or refuses at these
limits; the model saturates, which no law yet checks.

**Would close it:** wrapping and checked arithmetic as named operations.

## Data

### 11. Bytes cannot be read

`Bytes` has no operations beyond equality, order and `prelude.length`. Kafka
picks a keyed record's partition with murmur2 over the key's octets
(ref:kafka-murmur2); that cannot be written in LawSpec. The `Partitioner`
ability's spec handler always chooses partition 0, which is lawful but says
nothing, and the hash is native code in each test project, checked against
Kafka's own test vectors.

**Would close it:** octet access and folds over `Bytes`, or built-in hashes
with laws, as `lawspec.crypto` has for SHA-3.

### 12. Text cannot be built

There is no text concatenation (`a + b` on `Text` is an error) and no
conversion from numbers. Member ids are therefore supplied by the client, as
KIP-848 has them, rather than made by the coordinator as the classic
protocol does (ref:kafka-classic-group-protocol).

**Would close it:** concatenation and number formatting for `Text`.

### 13. A prelude helper is unknown inside an ability's laws

`prelude.isRight (get x)` in an ability's `laws` section fails with
`unknown helper or wrong arity: prelude.isRight`; the same law at the top
level of the unit passes. The contract defines its own `succeeded`.

**Would close it:** the prelude's helpers in ability laws as everywhere else.

### 14. A literal in a `let` does not take its type from its use

```lawspec fragment
let from = (match g with | Nothing -> 0 | Just group -> ... end) in
Step from (withPosition tp from c)
```

fails with `type mismatch: Integer and Int64`; the literal needs
`(0 :: Int64)`.

## Generation

### 15. Refinements filter; they do not generate

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
