/**
 * Kotest combinators that keep native shrink trees, which LawSpec's generated arbitraries rely on
 * so that Kotest's shrinker, not a second one, reduces counterexamples.
 * ref:DEC-native-property-frameworks ref:kotest ref:DEC-shrink-within-domain
 */
package lawspec.testing

import io.kotest.property.Arb
import io.kotest.property.RTree
import io.kotest.property.RandomSource
import io.kotest.property.Sample
import io.kotest.property.arbitrary.list
import io.kotest.property.arbitrary.merge

/** Kotest's merge preserves branch samples; choice in 5.9 keeps only values. */
fun <A> lawspecChoice(first: Arb<A>, second: Arb<A>): Arb<A> = first.merge(second)

/** Composes Kotest's list shrink tree with the native trees of its elements. */
fun <A> lawspecList(element: Arb<A>, range: IntRange): Arb<List<A>> {
    // Kotest 5.9's list generator keeps structural shrinking but discards element
    // samples. Retain those samples as the list payload and compose both trees.
    val sampledElements = object : Arb<Sample<A>>() {
        // Arb.edgecase returns only a value, so it cannot preserve a shrink tree.
        // LawSpec emits element boundary cases separately from random properties.
        override fun edgecase(rs: RandomSource): Sample<A>? = null

        override fun sample(rs: RandomSource): Sample<Sample<A>> = Sample(element.sample(rs))
    }
    val lists = Arb.list(sampledElements, range)
    return object : Arb<List<A>>() {
        override fun edgecase(rs: RandomSource): List<A>? =
          lists.edgecase(rs)?.map { it.value }

        override fun sample(rs: RandomSource): Sample<List<A>> {
            val tree = composeListTree(lists.sample(rs).shrinks)
            return Sample(tree.value(), tree)
        }
    }
}

private fun <A> composeListTree(source: RTree<List<Sample<A>>>): RTree<List<A>> =
  RTree(
      { source.value().map { it.value } },
      lazy {
          source.children.value.map { composeListTree(it) } +
            source.value().flatMap { sample ->
                sample.shrinks.children.value.map { child ->
                    composeListTree(replaceSample(source, sample, Sample(child.value(), child)))
                }
            }
      },
  )

private fun <A> replaceSample(
    source: RTree<List<Sample<A>>>,
    original: Sample<A>,
    replacement: Sample<A>,
): RTree<List<Sample<A>>> =
  RTree(
      { source.value().map { if (it === original) replacement else it } },
      lazy { source.children.value.map { replaceSample(it, original, replacement) } },
  )

/** Preserve both native shrink trees when a later arbitrary depends on a value.
 *
 * Kotest 5.9's flatMap passes only Sample.value to its continuation. Keep the
 * source tree as well, so list lengths and constructor choices can still shrink.
 */
fun <A, B> lawspecFlatMap(source: Arb<A>, next: (A) -> Arb<B>): Arb<B> =
  object : Arb<B>() {
      override fun edgecase(rs: RandomSource): B? = null

      override fun sample(rs: RandomSource): Sample<B> {
          val sourceTree = source.sample(rs).shrinks
          val seed = rs.random.nextLong()
          fun draw(tree: RTree<A>): RTree<B> =
            next(tree.value()).sample(RandomSource.seeded(seed)).shrinks

          fun compose(sourceNode: RTree<A>, result: RTree<B>): RTree<B> =
            RTree(
                { result.value() },
                lazy {
                    sourceNode.children.value.map { compose(it, draw(it)) } +
                      result.children.value.map { compose(sourceNode, it) }
                },
            )

          val tree = compose(sourceTree, draw(sourceTree))
          return Sample(tree.value(), tree)
      }
  }
