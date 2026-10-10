package lawspec.runtime

import lawspec.runtime.LawSpecKotlin as Native
import lawspec.runtime.LawSpecRuntime as Runtime
import lawspec.runtime.LawSpecSchema.Codec
import lawspec.runtime.LawSpecSchema.Named
import kotlin.time.toJavaDuration
import kotlin.time.toKotlinDuration

/**
 * Framework-independent bridges for Kotlin-specific native representations.
 *
 * Adapters work with Kotlin's own types, such as kotlin.time.Duration and Set; each codec converts
 * between them and LawSpec's values and checks the result, so a law never sees a native value
 * outside its domain. ref:DEC-native-bindings-typed-identity
 */
object LawSpecKotlinCodecs {
    fun construct(
        schema: LawSpecSchema,
        type: Named,
        tag: String,
        fields: List<Runtime.Value>,
        bits: Int,
        symbols: MutableMap<String, Any> = mutableMapOf(),
    ): Runtime.Value {
        if (type.name() != "List") return schema.construct(type, tag, fields, bits, symbols)
        val values = when {
            tag == "List::Nil" && fields.isEmpty() -> emptyList()
            tag == "List::Cons" && fields.size == 2 -> {
                val tail = fields[1].data() as List<*>
                listOf(fields[0]) + tail.map { it as Runtime.Value }
            }
            else -> throw IllegalArgumentException("invalid List constructor or field count")
        }
        return Runtime.Value(LawSpecSchema.key(type), values)
    }

    fun match(
        schema: LawSpecSchema,
        type: Named,
        value: Runtime.Value,
        bits: Int,
        branch: (String, List<Runtime.Value>) -> Runtime.Value,
    ): Runtime.Value = match(schema, type, value, bits, mutableMapOf(), branch)

    fun match(
        schema: LawSpecSchema,
        type: Named,
        value: Runtime.Value,
        bits: Int,
        symbols: MutableMap<String, Any>,
        branch: (String, List<Runtime.Value>) -> Runtime.Value,
    ): Runtime.Value {
        // Values in generated code are checked where they are built, decoded
        // or drawn, so matching only dispatches.
        val checked = value
        if (type.name() == "List") {
            val values = (checked.data() as List<*>).map { it as Runtime.Value }
            return if (values.isEmpty()) {
                branch("List::Nil", emptyList())
            } else {
                branch(
                    "List::Cons",
                    listOf(values[0], Runtime.Value(checked.type(), values.drop(1))),
                )
            }
        }
        val data = checked.data() as Runtime.Data
        return branch(data.tag(), data.fields())
    }

    /** A Duration natively: a kotlin.time.Duration of whole microseconds. */
    fun duration(schema: LawSpecSchema, bits: Int, symbols: MutableMap<String, Any> = mutableMapOf()): Codec<kotlin.time.Duration> {
        val codec = schema.duration(bits, symbols)
        return schema.codec(codec.type(), bits, symbols, { codec.encode(it.toJavaDuration()) }, { codec.decode(it).toKotlinDuration() })
    }

    /** A Set natively, in canonical order when LawSpec builds it. */
    fun <T> set(schema: LawSpecSchema, bits: Int, element: Codec<T>, symbols: MutableMap<String, Any> = mutableMapOf()): Codec<Set<T>> {
        val codec = schema.set(element, bits, symbols)
        return schema.codec(codec.type(), bits, symbols, { codec.encode(it) }, { codec.decode(it) })
    }

    /** A KeyVal natively, in canonical key order when LawSpec builds it. */
    fun <K, V> keyVal(
        schema: LawSpecSchema, bits: Int, keys: Codec<K>, values: Codec<V>,
        symbols: MutableMap<String, Any> = mutableMapOf(),
    ): Codec<Map<K, V>> {
        val codec = schema.keyVal(keys, values, bits, symbols)
        return schema.codec(codec.type(), bits, symbols, { codec.encode(it) }, { codec.decode(it) })
    }

    /** A Queue, Stack or Deque natively: an ArrayDeque from its first item (a Stack's top is last). */
    fun <T> sequence(
        schema: LawSpecSchema, bits: Int, name: String, element: Codec<T>,
        symbols: MutableMap<String, Any> = mutableMapOf(),
    ): Codec<ArrayDeque<T>> {
        val codec = schema.sequence(name, element, bits, symbols)
        val stack = name == "Stack"
        return schema.codec(
            codec.type(), bits, symbols,
            { deque -> codec.encode(java.util.ArrayDeque(if (stack) deque.reversed() else deque)) },
            { value -> codec.decode(value).toList().let { if (stack) it.reversed() else it }.let { ArrayDeque(it) } },
        )
    }

    fun unit(schema: LawSpecSchema, bits: Int): Codec<Unit> =
      schema.codec(Named("Unit"), bits, { Runtime.absent("Unit") }, { })

    fun nullValue(schema: LawSpecSchema, bits: Int): Codec<Native.Null> =
      schema.codec(Named("Null"), bits, { Runtime.absent("Null") }, { Native.Null })

    fun undefined(schema: LawSpecSchema, bits: Int): Codec<Native.Undefined> =
      schema.codec(
          Named("Undefined"), bits, { Runtime.absent("Undefined") }, { Native.Undefined },
      )

    fun symbol(schema: LawSpecSchema, bits: Int): Codec<Native.Symbol> =
      schema.codec(
          Named("Symbol"),
          bits,
          { Runtime.Value("Symbol", it.token) },
          { Native.Symbol(it.data() as Runtime.SymbolValue) },
      )

    fun codePointText(schema: LawSpecSchema, bits: Int): Codec<IntArray> =
      schema.codec(
          Named("CodePointText"),
          bits,
          { Runtime.sequence("CodePointText", it) },
          { value -> (value.data() as List<*>).map { it as Int }.toIntArray() },
      )

    fun <T> nullable(
        schema: LawSpecSchema,
        bits: Int,
        element: Codec<T>,
        symbols: MutableMap<String, Any> = mutableMapOf(),
    ): Codec<Native.Nullable<T>> {
        val type = Named("Nullable", element.type())
        return schema.codec(
            type,
            bits,
            symbols,
            { value ->
                val payload = when (value) {
                    is Native.Nullable.Null -> Runtime.Presence(false, null)
                    is Native.Nullable.Present -> {
                        Runtime.Presence(
                            true, LawSpecSchema.encodeField(element, value.value, "present"),
                        )
                    }
                }
                Runtime.Value(LawSpecSchema.key(type), payload)
            },
            { value ->
                val payload = value.data() as Runtime.Presence
                if (payload.present()) {
                    Native.Nullable.Present(element.decode(payload.value()))
                } else {
                    Native.Nullable.Null()
                }
            },
        )
    }

    fun <T> optional(
        schema: LawSpecSchema,
        bits: Int,
        element: Codec<T>,
        symbols: MutableMap<String, Any> = mutableMapOf(),
    ): Codec<Native.Optional<T>> {
        val type = Named("Optional", element.type())
        return schema.codec(
            type,
            bits,
            symbols,
            { value ->
                val payload = when (value) {
                    is Native.Optional.Undefined -> Runtime.Presence(false, null)
                    is Native.Optional.Present -> {
                        Runtime.Presence(
                            true, LawSpecSchema.encodeField(element, value.value, "present"),
                        )
                    }
                }
                Runtime.Value(LawSpecSchema.key(type), payload)
            },
            { value ->
                val payload = value.data() as Runtime.Presence
                if (payload.present()) {
                    Native.Optional.Present(element.decode(payload.value()))
                } else {
                    Native.Optional.Undefined()
                }
            },
        )
    }
}
