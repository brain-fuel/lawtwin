package lawspec.runtime

/**
 * Native support types whose domains differ from Kotlin's built-in types.
 *
 * Kotlin's null cannot tell an absent value from a present null, so Nullable keeps the two apart,
 * as a law's Maybe and Nullable types require. ref:DEC-algebraic-maybe-either
 */
object LawSpecKotlin {
    sealed interface Nullable<T> {
        class Null<T> : Nullable<T>

        class Present<T>(val value: T) : Nullable<T>
    }

    sealed interface Optional<T> {
        class Undefined<T> : Optional<T>

        class Present<T>(val value: T) : Optional<T>
    }

    object Null

    object Undefined

    /** Equality follows the fixture identity, even after a checked round trip. */
    class Symbol internal constructor(internal val token: LawSpecRuntime.SymbolValue) {
        constructor(description: String) : this(LawSpecRuntime.SymbolValue(description))

        val description: String
            get() = token.description()

        override fun equals(other: Any?): Boolean = other is Symbol && token === other.token

        override fun hashCode(): Int = System.identityHashCode(token)
    }
}
