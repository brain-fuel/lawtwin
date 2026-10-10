package lawspec.testing

import io.kotest.property.Arb
import io.kotest.property.RTree
import io.kotest.property.RandomSource
import io.kotest.property.Sample
import io.kotest.property.arbitrary.bind
import io.kotest.property.arbitrary.constant
import io.kotest.property.arbitrary.int
import io.kotest.property.arbitrary.map
import io.kotest.property.filter
import lawspec.runtime.LawSpecRuntime.Data
import lawspec.runtime.LawSpecRuntime.Presence
import lawspec.runtime.LawSpecRuntime.Value
import lawspec.runtime.LawSpecSchema
import lawspec.runtime.LawSpecSchema.Named

/**
 * Native Kotest generation and shrinking for instantiated Core data schemas.
 *
 * Generated tests run in Kotest, the framework Kotlin developers already use, so failures shrink
 * and report as they expect; the arbitraries are built from Kotest's own so its shrinker reduces
 * counterexamples and shrinking stays within the declared domain.
 * ref:DEC-native-property-frameworks ref:kotest ref:DEC-shrink-within-domain
 */
object LawSpecKotlinStrategies {
    fun generator(
        schema: LawSpecSchema,
        type: Named,
        bits: Int,
        budget: Int,
        scalar: (String) -> Arb<Value>,
    ): Arb<Value> {
        require(!schema.hasContracts()) { "constructor contracts require checked generation" }
        require(budget > 0) { "structural node budget must be positive" }
        require(bits == 32 || bits == 64) { "machineBits must be 32 or 64" }
        schema.isScalar(type) // Validate the entire type application before planning.
        return requireNotNull(Builder(schema, bits, scalar).build(type, budget)) {
            "no value of ${LawSpecSchema.key(type)} within structural node budget $budget"
        }
    }

    data class Case(
        val values: List<Value>,
        val symbols: MutableMap<String, Any>,
        val error: RuntimeException? = null,
    ) {
        fun requireValues(): List<Value> {
            error?.let { throw it }
            return values
        }
    }

    fun cases(): Arb<Case> = object : Arb<Case>() {
        override fun edgecase(rs: RandomSource): Case? = null

        override fun sample(rs: RandomSource): Sample<Case> =
            Sample(Case(emptyList(), mutableMapOf()))
    }

    fun filterCases(source: Arb<Case>, attempts: Int, predicate: (Case) -> Boolean): Arb<Case> {
        require(attempts > 0) { "maxAttempts must be positive" }
        return bounded(source, attempts, "property inputs") { it.error != null || predicate(it) }
    }

    data class Checked(val value: Value?, val error: RuntimeException?) {
        fun requireValue(): Value {
            error?.let { throw it }
            return requireNotNull(value)
        }
    }

    fun interface NativeFactory {
        fun create(
            schema: LawSpecSchema,
            type: Named,
            bits: Int,
            symbols: MutableMap<String, Any>,
            arguments: List<Arb<Value>>,
        ): Arb<Value>
    }

    private class NativeFailure(type: Named, cause: RuntimeException) :
        IllegalArgumentException(
            "native generator ${LawSpecSchema.key(type)}: ${cause.message}", cause,
        )

    fun <T> nativeValues(codec: LawSpecSchema.Codec<T>, source: Arb<T>): Arb<Value> =
        source.map { value ->
            try {
                codec.encode(value)
            } catch (error: RuntimeException) {
                throw NativeFailure(codec.type(), error)
            }
        }

    fun <T> nativeArguments(codec: LawSpecSchema.Codec<T>, source: Arb<Value>): Arb<T> =
        source.map { value ->
            try {
                codec.decode(value)
            } catch (error: RuntimeException) {
                throw NativeFailure(codec.type(), error)
            }
        }

    // Retain the native tree, including failures encountered while evaluating a
    // shrink node or enumerating its children. Report those in property callbacks.
    private fun capture(source: Arb<Checked>): Arb<Checked> = object : Arb<Checked>() {
        override fun edgecase(rs: RandomSource): Checked? = null

        fun failure(error: NativeFailure) = RTree({ Checked(null, error) })

        fun tree(source: RTree<Checked>): RTree<Checked> {
            val value = try {
                source.value()
            } catch (error: NativeFailure) {
                return failure(error)
            }
            return RTree({ value }, lazy {
                try {
                    source.children.value.map(::tree)
                } catch (error: NativeFailure) {
                    listOf(failure(error))
                }
            })
        }

        override fun sample(rs: RandomSource): Sample<Checked> {
            val result = try {
                tree(source.sample(rs).shrinks)
            } catch (error: NativeFailure) {
                failure(error)
            }
            return Sample(result.value(), result)
        }
    }

    fun checkedGenerator(
        schema: LawSpecSchema,
        type: Named,
        bits: Int,
        budget: Int,
        maxAttempts: Int,
        symbols: MutableMap<String, Any>,
        witnesses: List<Value>,
        scalar: (String) -> Arb<Value>,
    ): Arb<Checked> = checkedGenerator(
        schema, type, bits, budget, maxAttempts, symbols, witnesses, emptyMap(), scalar,
    )

    fun checkedGenerator(
        schema: LawSpecSchema,
        type: Named,
        bits: Int,
        budget: Int,
        maxAttempts: Int,
        symbols: MutableMap<String, Any>,
        witnesses: List<Value>,
        factories: Map<String, NativeFactory>,
        scalar: (String) -> Arb<Value>,
    ): Arb<Checked> {
        require(budget > 0) { "structural node budget must be positive" }
        require(bits == 32 || bits == 64) { "machineBits must be 32 or 64" }
        require(maxAttempts > 0) { "maxAttempts must be positive" }
        schema.isScalar(type)
        val builder = Builder(schema, bits, scalar, symbols, maxAttempts, factories)
        witnesses.forEach { builder.addWitness(type, schema.validate(type, it, bits, symbols)) }
        val source = requireNotNull(builder.build(type, budget)) {
            "no value of ${LawSpecSchema.key(type)} within structural node budget $budget"
        }
        return capture(source.map { value ->
            try {
                Checked(schema.validate(type, value, bits, symbols), null)
            } catch (error: RuntimeException) {
                Checked(null, error)
            }
        })
    }

    private fun nodes(value: Value): Int = 1 + when (val data = value.data()) {
        is Data -> data.fields().sumOf(::nodes)
        is Presence -> if (data.present()) nodes(data.value()) else 0
        is List<*> -> {
            if (value.type().startsWith("List ")) {
                data.sumOf { nodes(it as Value) }
            } else {
                0
            }
        }
        else -> 0
    }

    private fun <T> bounded(
        source: Arb<T>,
        attempts: Int,
        label: String,
        accepted: (T) -> Boolean,
    ): Arb<T> = object : Arb<T>() {
        override fun edgecase(rs: RandomSource): T? = null

        override fun sample(rs: RandomSource): Sample<T> {
            repeat(attempts) {
                val sample = source.sample(rs)
                val tree = sample.shrinks.filter(accepted)
                if (tree != null) return Sample(tree.value(), tree)
            }
            throw IllegalArgumentException(
                "constructor generation exhausted $attempts attempts for $label",
            )
        }
    }

    private const val INDEX_SLACK = 16L
    private const val INDEX_CHOICES = 6

    /**
     * Values whose structural index equals [target]. Each constructor carries its index term then
     * its guards, in prefix notation over field indices (`f<i>`), literals (`c<n>`) and the natural
     * operators. Reachability is a forward fixpoint over levels 0..target+slack, so a child may
     * exceed its parent's index; the target is then solved backwards, and nothing is filtered away.
     */
    fun indexedGenerator(
        schema: LawSpecSchema,
        type: Named,
        bits: Int,
        budget: Int,
        symbols: MutableMap<String, Any>,
        target: Any,
        equations: Map<String, List<String>>,
        scalar: (String) -> Arb<Value>,
    ): Arb<Checked> {
        val k = when (target) {
            is Value -> (target.data() as Number).toLong()
            is Number -> target.toLong()
            else -> throw IllegalArgumentException("index target must be a natural number")
        }
        require(!schema.isScalar(type)) { "indexed generation requires a data type" }
        val limit = maxOf(k, 0L) + INDEX_SLACK
        val indexed = Indexed(schema, bits, budget, limit, equations, Builder(schema, bits, scalar))
        indexed.explore(type)
        // An open target (negative), or one drawn from earlier inputs that breaks their
        // preconditions or names no value, generates from the smallest reachable indices; an index
        // claim rejects a mismatch.
        val levels = if ((type to k) in indexed.reach) {
            listOf(k)
        } else {
            (0..limit).filter { (type to it) in indexed.reach }.take(INDEX_CHOICES)
        }
        require(levels.isNotEmpty()) { "no value of ${LawSpecSchema.key(type)} has an index" }
        val source = if (levels.size == 1) {
            indexed.generate(type, levels[0])
        } else {
            lawspecFlatMap(Arb.int(levels.indices)) { indexed.generate(type, levels[it]) }
        }
        return capture(source.map { value ->
            try {
                Checked(schema.validate(type, value, bits, symbols), null)
            } catch (error: RuntimeException) {
                Checked(null, error)
            }
        })
    }

    /** A prefix index term: kind is "c", "f" or an operator. */
    private class IndexTerm(val kind: String, val value: Long, val left: IndexTerm?, val right: IndexTerm?) {
        /** Natural index arithmetic; null when an operation has no natural value. */
        fun evaluate(fields: Map<Int, Long>): Long? {
            if (kind == "c") return value
            if (kind == "f") return fields[value.toInt()]
            val x = left!!.evaluate(fields) ?: return null
            val y = right!!.evaluate(fields) ?: return null
            val a = java.math.BigInteger.valueOf(x)
            val b = java.math.BigInteger.valueOf(y)
            val result = when (kind) {
                "+" -> a.add(b)
                "-" -> if (x >= y) a.subtract(b) else null
                "*" -> a.multiply(b)
                "div" -> if (y > 0) a.divide(b) else null
                "mod" -> if (y > 0) a.mod(b) else null
                else -> if (y <= 64) a.pow(y.toInt()) else null
            } ?: return null
            return if (result.bitLength() > 63) null else result.toLong()
        }

        fun fields(into: MutableList<Int>) {
            if (kind == "f") {
                if (value.toInt() !in into) into.add(value.toInt())
            } else if (left != null) {
                left.fields(into)
                right!!.fields(into)
            }
        }

        companion object {
            private val OPERATORS = listOf("+", "-", "*", "div", "mod", "^")

            fun parse(tokens: List<String>, at: IntArray): IndexTerm {
                require(at[0] < tokens.size) { "malformed index term" }
                val token = tokens[at[0]++]
                if (token.startsWith("c") || token.startsWith("f")) {
                    return IndexTerm(token.substring(0, 1), token.substring(1).toLong(), null, null)
                }
                require(token in OPERATORS) { "malformed index term" }
                val left = parse(tokens, at)
                val right = parse(tokens, at)
                return IndexTerm(token, 0, left, right)
            }
        }
    }

    private class IndexGuard(val relation: String, val left: IndexTerm, val right: IndexTerm) {
        fun holds(fields: Map<Int, Long>): Boolean {
            val x = left.evaluate(fields) ?: return false
            val y = right.evaluate(fields) ?: return false
            return if (relation == "==") x == y else x >= y
        }
    }

    private class IndexEquation(val term: IndexTerm, val guards: List<IndexGuard>, val positions: List<Int>)

    private class Indexed(
        val schema: LawSpecSchema,
        val bits: Int,
        val budget: Int,
        val limit: Long,
        val equations: Map<String, List<String>>,
        val builder: Builder,
    ) {
        val parsed = mutableMapOf<String, IndexEquation>()
        val families = mutableListOf<Named>()
        val reach = mutableSetOf<Pair<Named, Long>>()
        val solutions = mutableMapOf<Triple<Named, String, Long>, List<Map<Int, Long>>>()
        val cache = mutableMapOf<Pair<Named, Long>, Arb<Value>>()

        fun equation(tag: String): IndexEquation = parsed.getOrPut(tag) {
            val texts = requireNotNull(equations[tag]) { "missing index equation for $tag" }
            require(texts.isNotEmpty()) { "missing index equation for $tag" }
            val tokens = texts[0].split(" ")
            val at = intArrayOf(0)
            val term = IndexTerm.parse(tokens, at)
            require(at[0] == tokens.size) { "malformed index term" }
            val positions = mutableListOf<Int>()
            term.fields(positions)
            val guards = texts.drop(1).map { text ->
                val parts = text.split(" ")
                require(parts[0] == "==" || parts[0] == ">=") { "malformed index guard" }
                val position = intArrayOf(1)
                val left = IndexTerm.parse(parts, position)
                val right = IndexTerm.parse(parts, position)
                left.fields(positions)
                right.fields(positions)
                IndexGuard(parts[0], left, right)
            }
            IndexEquation(term, guards, positions)
        }

        fun plainFields(type: Named, tag: String): Boolean {
            val fields = schema.fields(type, tag)
            val positions = equation(tag).positions
            return fields.indices.all {
                it in positions || builder.build(fields[it].type() as Named, budget) != null
            }
        }

        // Every guard-satisfying assignment of reachable indices to the index fields.
        fun assignments(type: Named, tag: String): List<Pair<Long, Map<Int, Long>>> {
            val found = equation(tag)
            val fields = schema.fields(type, tag)
            val results = mutableListOf<Pair<Long, Map<Int, Long>>>()
            fun extend(at: Int, current: MutableMap<Int, Long>) {
                if (at == found.positions.size) {
                    if (!found.guards.all { it.holds(current) }) return
                    val value = found.term.evaluate(current) ?: return
                    if (value <= limit) results.add(value to current.toMap())
                    return
                }
                val position = found.positions[at]
                val fieldType = fields[position].type() as Named
                for (value in 0..limit) {
                    if ((fieldType to value) in reach) {
                        current[position] = value
                        extend(at + 1, current)
                    }
                }
                current.remove(position)
            }
            extend(0, mutableMapOf())
            return results
        }

        fun explore(root: Named) {
            val pending = mutableListOf(root)
            while (pending.isNotEmpty()) {
                val type = pending.removeAt(pending.size - 1)
                if (type in families) continue
                families.add(type)
                for (tag in schema.constructors(type)) {
                    val fields = schema.fields(type, tag)
                    equation(tag).positions.forEach { pending.add(fields[it].type() as Named) }
                }
            }
            do {
                val next = mutableSetOf<Pair<Named, Long>>()
                for (type in families) {
                    for (tag in schema.constructors(type)) {
                        if (!plainFields(type, tag)) continue
                        assignments(type, tag).forEach { next.add(type to it.first) }
                    }
                }
            } while (reach.addAll(next))
        }

        fun solve(type: Named, tag: String, k: Long): List<Map<Int, Long>> =
            solutions.getOrPut(Triple(type, tag, k)) {
                assignments(type, tag).filter { it.first == k }.map { it.second }
            }

        // Assignments are chosen by index, which shrinks toward the first.
        fun generate(type: Named, k: Long): Arb<Value> {
            cache[type to k]?.let { return it }
            val variants = schema.constructors(type)
                .filter { plainFields(type, it) && solve(type, it, k).isNotEmpty() }
                .map { tag ->
                    val fields = schema.fields(type, tag)
                    val choices = solve(type, tag, k)
                    lawspecFlatMap(Arb.int(choices.indices)) { choice ->
                        val targets = choices[choice]
                        val children = fields.mapIndexed { index, field ->
                            val target = targets[index]
                            if (target != null) {
                                generate(field.type() as Named, target)
                            } else {
                                requireNotNull(builder.build(field.type() as Named, budget))
                            }
                        }
                        children.fold(Arb.constant(emptyList<Value>())) { prior, child ->
                            Arb.bind(prior, child) { values, value -> values + value }
                        }.map { schema.construct(type, tag, it, bits) }
                    }
                }
            require(variants.isNotEmpty()) {
                "no value of ${LawSpecSchema.key(type)} has index $k"
            }
            val result = if (variants.size == 1) {
                variants[0]
            } else {
                lawspecFlatMap(Arb.int(variants.indices)) { variants[it] }
            }
            cache[type to k] = result
            return result
        }
    }

    private data class Request(val type: Named, val budget: Int)

    private class Builder(
        val schema: LawSpecSchema,
        val bits: Int,
        val scalar: (String) -> Arb<Value>,
        val symbols: MutableMap<String, Any>? = null,
        val maxAttempts: Int = 100,
        val factories: Map<String, NativeFactory> = emptyMap(),
    ) {
        val witnesses = mutableMapOf<Named, MutableList<Value>>()
        val cache = mutableMapOf<Request, Arb<Value>?>()

        fun addWitness(type: Named, value: Value) {
            witnesses.getOrPut(type) { mutableListOf() }.add(value)
            when {
                schema.isScalar(type) -> Unit
                type.name() == "List" -> {
                    (value.data() as List<*>).forEach {
                        addWitness(type.arguments()[0] as Named, it as Value)
                    }
                }
                type.name() in listOf("Nullable", "Optional") -> {
                    val presence = value.data() as Presence
                    if (presence.present()) {
                        addWitness(type.arguments()[0] as Named, presence.value())
                    }
                }
                else -> {
                    val data = value.data() as Data
                    schema.fieldsOf(type, data.tag(), data.fields()).forEachIndexed { index, field ->
                        addWitness(field.type() as Named, data.fields()[index])
                    }
                }
            }
        }

        fun minimum(type: Named, limit: Int): Int? =
          (1..limit).firstOrNull { build(type, it) != null }

        fun allocation(fields: List<LawSpecSchema.Field>, available: Int): List<Int>? {
            var remaining = available
            val minima = fields.map { field ->
                val cost = minimum(field.type() as Named, remaining) ?: return null
                remaining -= cost
                cost
            }
            return minima.mapIndexed { index, cost ->
                cost + remaining / minima.size + if (index < remaining % minima.size) 1 else 0
            }
        }

        fun build(type: Named, available: Int): Arb<Value>? {
            if (available < 1) return null
            val request = Request(type, available)
            if (cache.containsKey(request)) return cache[request]
            factories[type.name()]?.let { factory ->
                val arguments = type.arguments().map {
                    build(it as Named, available) ?: object : Arb<Value>() {
                        // A phantom parameter can be uninhabited. Fail only if
                        // its factory actually asks this arbitrary for a value.
                        override fun edgecase(rs: RandomSource): Value? = null
                        override fun sample(rs: RandomSource): Sample<Value> =
                            error("no native generator argument for ${LawSpecSchema.key(it)} within node budget $available")
                    }
                }
                val context = requireNotNull(symbols)
                val result = factory.create(schema, type, bits, context, arguments).map { value ->
                    try {
                        schema.validate(type, value, bits, context)
                    } catch (error: RuntimeException) {
                        throw NativeFailure(type, error)
                    }
                }
                cache[request] = result
                return result
            }
            var result = when {
                schema.isScalar(type) -> scalar(type.name()).map { schema.validate(type, it, bits) }
                type.name() == "List" -> list(type, available)
                type.name() in listOf("Nullable", "Optional") -> presence(type, available)
                else -> {
                    // A field-only existential takes each type of the witness pool.
                    choice(schema.constructors(type).flatMap { tag ->
                      schema.instances(type, tag).mapNotNull { instance ->
                        val fields = instance.fields()
                        val witnessValues = instance.keys().map { LawSpecSchema.witnessText(it) }
                        val costs = allocation(fields, available - 1) ?: return@mapNotNull null
                        val children = fields.mapIndexed { index, field ->
                            requireNotNull(build(field.type() as Named, costs[index]))
                        }
                        val product =
                            children.fold(Arb.constant(emptyList<Value>())) { prior, child ->
                                Arb.bind(prior, child) { values, value -> values + value }
                            }
                        product.map { it + witnessValues }.map {
                            val value = if (symbols == null) {
                                schema.construct(type, tag, it, bits)
                            } else {
                                Value(LawSpecSchema.key(type), Data(tag, it))
                            }
                            // Generated collections are canonicalised rather than filtered.
                            if (LawSpecSchema.canonicalCollection(type.name())) LawSpecSchema.canonical(value) else value
                        }
                      }
                    })
                }
            }
            if (result != null && symbols != null) {
                val seeds = witnesses[type].orEmpty().filter { nodes(it) <= available }
                if (seeds.isNotEmpty()) {
                    val sample = Arb.int(seeds.indices).map { seeds[it] }
                    result = lawspecChoice(result, sample)
                }
                result = bounded(result, maxAttempts, LawSpecSchema.key(type)) { value ->
                    try {
                        schema.check(type, value, bits, symbols) is LawSpecSchema.Accepted
                    } catch (error: RuntimeException) {
                        // Keep evaluator errors for the outer checked result to report.
                        true
                    }
                }
            }
            cache[request] = result
            return result
        }

        fun list(type: Named, available: Int): Arb<Value> {
            val element = type.arguments()[0] as Named
            val remaining = available - 1
            val cost = minimum(element, remaining)
            val maximum = if (cost == null) 0 else remaining / cost
            // Plan before sampling: callbacks only select immutable native arbitraries.
            val sizes = (0..maximum).map { length ->
                if (length == 0) {
                    Arb.constant(Value(LawSpecSchema.key(type), emptyList<Value>()))
                } else {
                    lawspecList(requireNotNull(build(element, remaining / length)), length..length)
                        .map { Value(LawSpecSchema.key(type), it) }
                }
            }
            return lawspecFlatMap(Arb.int(0..maximum)) { sizes[it] }
        }

        fun presence(type: Named, available: Int): Arb<Value> {
            val absent = Arb.constant(Value(LawSpecSchema.key(type), Presence(false, null)))
            val child = build(type.arguments()[0] as Named, available - 1) ?: return absent
            val present = child.map { Value(LawSpecSchema.key(type), Presence(true, it)) }
            return requireNotNull(choice(listOf(absent, present)))
        }

        fun choice(alternatives: List<Arb<Value>>): Arb<Value>? = when (alternatives.size) {
            0 -> null
            1 -> alternatives[0]
            else -> lawspecFlatMap(Arb.int(alternatives.indices)) { alternatives[it] }
        }
    }
}
