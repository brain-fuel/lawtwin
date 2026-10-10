package lawspec.testing;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Function;
import java.util.function.Predicate;
import lawspec.runtime.LawSpecRuntime.Data;
import lawspec.runtime.LawSpecRuntime.Presence;
import lawspec.runtime.LawSpecRuntime.Value;
import lawspec.runtime.LawSpecSchema;
import lawspec.runtime.LawSpecSchema.Named;
import org.jetbrains.jetCheck.Generator;
import org.jetbrains.jetCheck.IntDistribution;

/**
 * Native JetCheck generation and shrinking for the checked data schema.
 *
 * Generated tests run in JetCheck, so failures shrink and report as the framework does; LawSpec
 * supplies only the generators for its own types, built from JetCheck's own so its shrinker reduces
 * counterexamples and shrinking stays within the declared domain.
 * ref:DEC-native-property-frameworks ref:jetcheck ref:DEC-shrink-within-domain
 */
public final class LawSpecDataStrategies {
  private LawSpecDataStrategies() {}

  @FunctionalInterface
  public interface NativeFactory {
    Generator<Value> create(
        LawSpecSchema schema,
        Named type,
        int bits,
        Map<String, Object> symbols,
        List<Generator<Value>> arguments);
  }

  /** Conversion failures are values at the checked property boundary, even on replay. */
  private static final class NativeFailure extends IllegalArgumentException {
    NativeFailure(Named type, RuntimeException cause) {
      super("native generator " + LawSpecSchema.key(type) + ": " + cause.getMessage(), cause);
    }
  }

  public static <T> Generator<Value> nativeValues(
      LawSpecSchema.Codec<T> codec, Generator<T> source) {
    return source.map(
        value -> {
          try {
            return codec.encode(value);
          } catch (RuntimeException error) {
            throw new NativeFailure(codec.type(), error);
          }
        });
  }

  public static <T> Generator<T> nativeArguments(
      LawSpecSchema.Codec<T> codec, Generator<Value> source) {
    return source.map(
        value -> {
          try {
            return codec.decode(value);
          } catch (RuntimeException error) {
            throw new NativeFailure(codec.type(), error);
          }
        });
  }

  public record Captured<T>(T value, RuntimeException error) {
    public T requireValue() {
      if (error != null) throw error;
      return value;
    }
  }

  /** Carry native conversion errors across composed draws to the property callback. */
  public static <T> Generator<Captured<T>> capture(Generator<T> source) {
    return Generator.from(
        environment -> {
          try {
            return new Captured<>(environment.generate(source), null);
          } catch (NativeFailure error) {
            return new Captured<>(null, error);
          }
        });
  }

  public static Generator<Value> generator(
      LawSpecSchema schema,
      Named type,
      int bits,
      int nodeBudget,
      Function<String, Generator<Value>> scalar) {
    return new Builder(schema, bits, scalar).generate(type, nodeBudget);
  }

  public static Generator<Value> sizedGenerator(
      LawSpecSchema schema,
      Named type,
      int bits,
      int minimumBudget,
      Function<String, Generator<Value>> scalar) {
    var builder = new Builder(schema, bits, scalar);
    return Generator.from(
        environment -> {
          int budget =
              (int)
                  Math.min(
                      Integer.MAX_VALUE,
                      Math.max(minimumBudget, 4L * environment.getSizeHint() + 1));
          return environment.generate(builder.generate(type, budget));
        });
  }

  private static final long INDEX_SLACK = 16;
  private static final int INDEX_CHOICES = 6;

  /**
   * Values whose structural index equals {@code target}. Each constructor carries its index term
   * then its guards, in prefix notation over field indices ({@code f<i>}), literals ({@code
   * c<n>}) and the natural operators. Reachability is a forward fixpoint over levels
   * 0..target+slack, so a child may exceed its parent's index; the target is then solved
   * backwards, and nothing is filtered away.
   */
  public static Generator<Value> indexedGenerator(
      LawSpecSchema schema,
      Named type,
      int bits,
      int nodeBudget,
      Object target,
      Map<String, List<String>> equations,
      Function<String, Generator<Value>> scalar) {
    long k = target instanceof Value value ? ((Number) value.data()).longValue()
        : ((Number) target).longValue();
    if (schema.isScalar(type)) {
      throw new IllegalArgumentException("indexed generation requires a data type");
    }
    long limit = Math.max(k, 0) + INDEX_SLACK;
    var indexed =
        new Indexed(schema, bits, nodeBudget, limit, equations, new Builder(schema, bits, scalar));
    indexed.explore(type);
    // An open target (negative), or one drawn from earlier inputs that breaks their preconditions
    // or names no value, generates from the smallest reachable indices; an index claim rejects a
    // mismatch.
    if (indexed.reach.contains(List.<Object>of(type, k))) return indexed.generate(type, k);
    var levels = new ArrayList<Long>();
    for (long level = 0; level <= limit && levels.size() < INDEX_CHOICES; level++) {
      if (indexed.reach.contains(List.<Object>of(type, level))) levels.add(level);
    }
    if (levels.isEmpty()) {
      throw new IllegalArgumentException("no value of " + LawSpecSchema.key(type) + " has an index");
    }
    return Generator.from(
        environment ->
            environment.generate(indexed.generate(type, environment.generate(Generator.sampledFrom(levels)))));
  }

  /** A prefix index term: kind is "c", "f" or an operator. */
  private record IndexTerm(String kind, long value, IndexTerm left, IndexTerm right) {
    static final List<String> OPERATORS = List.of("+", "-", "*", "div", "mod", "^");

    static IndexTerm parse(String[] tokens, int[] at) {
      if (at[0] >= tokens.length) throw new IllegalArgumentException("malformed index term");
      String token = tokens[at[0]++];
      if (token.startsWith("c") || token.startsWith("f")) {
        return new IndexTerm(token.substring(0, 1), Long.parseLong(token.substring(1)), null, null);
      }
      if (!OPERATORS.contains(token)) throw new IllegalArgumentException("malformed index term");
      var left = parse(tokens, at);
      var right = parse(tokens, at);
      return new IndexTerm(token, 0, left, right);
    }

    /** Natural index arithmetic; null when an operation has no natural value. */
    Long evaluate(Map<Integer, Long> fields) {
      if (kind.equals("c")) return value;
      if (kind.equals("f")) return fields.get((int) value);
      Long x = left.evaluate(fields), y = right.evaluate(fields);
      if (x == null || y == null) return null;
      var a = java.math.BigInteger.valueOf(x);
      var b = java.math.BigInteger.valueOf(y);
      java.math.BigInteger result =
          switch (kind) {
            case "+" -> a.add(b);
            case "-" -> x >= y ? a.subtract(b) : null;
            case "*" -> a.multiply(b);
            case "div" -> y > 0 ? a.divide(b) : null;
            case "mod" -> y > 0 ? a.mod(b) : null;
            default -> y <= 64 ? a.pow((int) (long) y) : null;
          };
      return result == null || result.bitLength() > 63 ? null : result.longValue();
    }

    void fields(List<Integer> into) {
      if (kind.equals("f")) {
        if (!into.contains((int) value)) into.add((int) value);
      } else if (left != null) {
        left.fields(into);
        right.fields(into);
      }
    }
  }

  private record IndexGuard(String relation, IndexTerm left, IndexTerm right) {
    boolean holds(Map<Integer, Long> fields) {
      Long x = left.evaluate(fields), y = right.evaluate(fields);
      if (x == null || y == null) return false;
      return relation.equals("==") ? x.longValue() == y.longValue() : x >= y;
    }
  }

  private record IndexEquation(IndexTerm term, List<IndexGuard> guards, List<Integer> positions) {}

  private record IndexSolution(long value, Map<Integer, Long> assignment) {}

  private static final class Indexed {
    private final LawSpecSchema schema;
    private final int bits;
    private final int budget;
    private final long limit;
    private final Map<String, List<String>> equations;
    private final Builder builder;
    private final Map<String, IndexEquation> parsed = new HashMap<>();
    private final List<Named> families = new ArrayList<>();
    final java.util.Set<List<Object>> reach = new java.util.HashSet<>();
    private final Map<List<Object>, List<Map<Integer, Long>>> solutions = new HashMap<>();
    private final Map<List<Object>, Generator<Value>> generators = new HashMap<>();

    Indexed(
        LawSpecSchema schema,
        int bits,
        int budget,
        long limit,
        Map<String, List<String>> equations,
        Builder builder) {
      this.schema = schema;
      this.bits = bits;
      this.budget = budget;
      this.limit = limit;
      this.equations = equations;
      this.builder = builder;
    }

    private IndexEquation equation(String tag) {
      return parsed.computeIfAbsent(tag, ignored -> {
        var texts = equations.get(tag);
        if (texts == null || texts.isEmpty()) {
          throw new IllegalArgumentException("missing index equation for " + tag);
        }
        var tokens = texts.get(0).split(" ");
        var at = new int[] {0};
        var term = IndexTerm.parse(tokens, at);
        if (at[0] != tokens.length) throw new IllegalArgumentException("malformed index term");
        var positions = new ArrayList<Integer>();
        term.fields(positions);
        var guards = new ArrayList<IndexGuard>();
        for (var text : texts.subList(1, texts.size())) {
          var parts = text.split(" ");
          if (!parts[0].equals("==") && !parts[0].equals(">=")) {
            throw new IllegalArgumentException("malformed index guard");
          }
          var position = new int[] {1};
          var left = IndexTerm.parse(parts, position);
          var right = IndexTerm.parse(parts, position);
          left.fields(positions);
          right.fields(positions);
          guards.add(new IndexGuard(parts[0], left, right));
        }
        return new IndexEquation(term, guards, positions);
      });
    }

    private boolean plainFields(Named type, String tag) {
      var fields = schema.fields(type, tag);
      var positions = equation(tag).positions();
      for (int i = 0; i < fields.size(); i++) {
        if (!positions.contains(i) && !builder.canGenerate((Named) fields.get(i).type(), budget)) {
          return false;
        }
      }
      return true;
    }

    /** Every guard-satisfying assignment of reachable indices to the index fields. */
    private List<IndexSolution> assignments(Named type, String tag) {
      var found = equation(tag);
      var fields = schema.fields(type, tag);
      var results = new ArrayList<IndexSolution>();
      extend(found, fields, 0, new HashMap<>(), results);
      return results;
    }

    private void extend(
        IndexEquation found,
        List<LawSpecSchema.Field> fields,
        int at,
        Map<Integer, Long> current,
        List<IndexSolution> results) {
      if (at == found.positions().size()) {
        for (var guard : found.guards()) if (!guard.holds(current)) return;
        Long value = found.term().evaluate(current);
        if (value != null && value <= limit) results.add(new IndexSolution(value, Map.copyOf(current)));
        return;
      }
      int position = found.positions().get(at);
      var fieldType = (Named) fields.get(position).type();
      for (long value = 0; value <= limit; value++) {
        if (reach.contains(List.<Object>of(fieldType, value))) {
          current.put(position, value);
          extend(found, fields, at + 1, current, results);
        }
      }
      current.remove(position);
    }

    void explore(Named root) {
      var pending = new ArrayList<Named>(List.of(root));
      while (!pending.isEmpty()) {
        var type = pending.remove(pending.size() - 1);
        if (families.contains(type)) continue;
        families.add(type);
        for (var tag : schema.constructors(type)) {
          var fields = schema.fields(type, tag);
          for (int position : equation(tag).positions()) {
            pending.add((Named) fields.get(position).type());
          }
        }
      }
      boolean grown = true;
      while (grown) {
        var next = new java.util.HashSet<List<Object>>();
        for (var type : families) {
          for (var tag : schema.constructors(type)) {
            if (!plainFields(type, tag)) continue;
            for (var solution : assignments(type, tag)) {
              next.add(List.<Object>of(type, solution.value()));
            }
          }
        }
        grown = reach.addAll(next);
      }
    }

    private List<Map<Integer, Long>> solve(Named type, String tag, long k) {
      return solutions.computeIfAbsent(List.<Object>of(type, tag, k), ignored ->
          assignments(type, tag).stream()
              .filter(solution -> solution.value() == k)
              .map(IndexSolution::assignment)
              .toList());
    }

    Generator<Value> generate(Named type, long k) {
      var key = List.<Object>of(type, k);
      if (generators.containsKey(key)) return generators.get(key);
      var variants = new ArrayList<Generator<Value>>();
      for (var tag : schema.constructors(type)) {
        if (!plainFields(type, tag)) continue;
        var choices = solve(type, tag, k);
        if (choices.isEmpty()) continue;
        var fields = schema.fields(type, tag);
        variants.add(
            Generator.from(
                environment -> {
                  var targets = environment.generate(Generator.sampledFrom(choices));
                  var values = new ArrayList<Value>();
                  for (int i = 0; i < fields.size(); i++) {
                    var fieldType = (Named) fields.get(i).type();
                    Long target = targets.get(i);
                    values.add(environment.generate(target != null
                        ? generate(fieldType, target)
                        : builder.generate(fieldType, budget)));
                  }
                  return schema.construct(type, tag, values, bits);
                }));
      }
      if (variants.isEmpty()) {
        throw new IllegalArgumentException(
            "no value of " + LawSpecSchema.key(type) + " has index " + k);
      }
      var generator = variants.size() == 1 ? variants.get(0) : Generator.anyOf(variants);
      generators.put(key, generator);
      return generator;
    }
  }

  /** A rejected predicate is retried; an evaluation error is delivered to the property. */
  public record Checked(Value value, RuntimeException error) {
    public Value requireValue() {
      if (error != null) throw error;
      return value;
    }
  }

  public static Generator<Checked> checkedGenerator(
      LawSpecSchema schema,
      Named type,
      int bits,
      int minimumBudget,
      Map<String, Object> symbols,
      List<Value> witnesses,
      Function<String, Generator<Value>> scalar) {
    return checkedGenerator(schema, type, bits, minimumBudget, 100, symbols, witnesses, scalar);
  }

  public static Generator<Checked> checkedGenerator(
      LawSpecSchema schema,
      Named type,
      int bits,
      int minimumBudget,
      int maxAttempts,
      Map<String, Object> symbols,
      List<Value> witnesses,
      Function<String, Generator<Value>> scalar) {
    return checkedGenerator(
        schema, type, bits, minimumBudget, maxAttempts, symbols, witnesses, scalar, Map.of());
  }

  public static Generator<Checked> checkedGenerator(
      LawSpecSchema schema,
      Named type,
      int bits,
      int minimumBudget,
      int maxAttempts,
      Map<String, Object> symbols,
      List<Value> witnesses,
      Function<String, Generator<Value>> scalar,
      Map<String, NativeFactory> factories) {
    if (maxAttempts < 1) throw new IllegalArgumentException("maxAttempts must be positive");
    var builder = new Builder(schema, bits, scalar, symbols, maxAttempts, factories);
    for (var witness : witnesses) {
      // Invalid witnesses indicate a compiler/caller error, not an empty domain.
      builder.addWitness(type, schema.validate(type, witness, bits, symbols));
    }
    return Generator.from(
        environment -> {
          try {
            int budget =
                (int)
                    Math.min(
                        Integer.MAX_VALUE,
                        Math.max(minimumBudget, 4L * environment.getSizeHint() + 1));
            var candidate =
                builder
                    .generate(type, budget)
                    .map(
                        value -> {
                          try {
                            var result = schema.check(type, value, bits, symbols);
                            if (result instanceof LawSpecSchema.Accepted accepted)
                              return new Checked(accepted.value(), null);
                            return new Checked(null, null);
                          } catch (RuntimeException error) {
                            return new Checked(null, error);
                          }
                        });
            // JetCheck bounds rejection at 100 attempts and discards invalid replays
            // while shrinking. Errors remain values so properties report them.
            return environment.generate(
                boundedFilter(
                    candidate,
                    checked -> checked.value() != null || checked.error() != null,
                    maxAttempts));
          } catch (NativeFailure error) {
            // Do not catch JetCheck's internal replay-control exceptions.
            return new Checked(null, error);
          }
        });
  }

  private static <T> Generator<T> boundedFilter(
      Generator<T> source, Predicate<T> predicate, int maxAttempts) {
    // JetCheck itself caps suchThat at 100 attempts. A smaller LawSpec budget
    // stops before drawing an additional candidate. This counter is recreated
    // during replay, so a rejected shrink is still discarded by native suchThat.
    if (maxAttempts >= 100) return source.suchThat(predicate);
    return Generator.from(
        environment -> {
          int[] attempts = {0};
          Generator<T> limited =
              Generator.from(
                  candidate -> {
                    if (attempts[0]++ >= maxAttempts)
                      throw new IllegalArgumentException(
                          "constructor generation exhausted " + maxAttempts + " attempts");
                    return candidate.generate(source);
                  });
          return environment.generate(limited.suchThat(predicate));
        });
  }

  private static int nodes(Value value) {
    if (value.data() instanceof Data data)
      return 1 + data.fields().stream().mapToInt(LawSpecDataStrategies::nodes).sum();
    if (value.data() instanceof Presence presence)
      return 1 + (presence.present() ? nodes(presence.value()) : 0);
    if (value.type().startsWith("List "))
      return 1 + ((List<?>) value.data()).stream().mapToInt(item -> nodes((Value) item)).sum();
    return 1;
  }

  private record Request(Named type, int budget) {}

  private static final class Builder {
    private final LawSpecSchema schema;
    private final int bits;
    private final Function<String, Generator<Value>> scalar;
    private final boolean unchecked;
    private final int maxAttempts;
    private final Map<String, Object> symbols;
    private final Map<Named, List<Value>> witnesses = new HashMap<>();
    private final Map<Request, Boolean> inhabited = new HashMap<>();
    private final Map<Request, Generator<Value>> generators = new HashMap<>();
    private final Map<String, NativeFactory> factories;

    Builder(LawSpecSchema schema, int bits, Function<String, Generator<Value>> scalar) {
      this(schema, bits, scalar, null, 100);
    }

    Builder(
        LawSpecSchema schema,
        int bits,
        Function<String, Generator<Value>> scalar,
        Map<String, Object> symbols,
        int maxAttempts) {
      this(schema, bits, scalar, symbols, maxAttempts, Map.of());
    }

    Builder(
        LawSpecSchema schema,
        int bits,
        Function<String, Generator<Value>> scalar,
        Map<String, Object> symbols,
        int maxAttempts,
        Map<String, NativeFactory> factories) {
      boolean unchecked = symbols != null;
      if (!unchecked && schema.hasContracts())
        throw new IllegalArgumentException("constructor contracts require checked generation");
      this.unchecked = unchecked;
      this.maxAttempts = maxAttempts;
      this.symbols = symbols;
      this.schema = schema;
      this.bits = bits;
      this.scalar = scalar;
      this.factories = Map.copyOf(factories);
    }

    private void addWitness(Named type, Value value) {
      witnesses.computeIfAbsent(type, ignored -> new ArrayList<>()).add(value);
      if (schema.isScalar(type)) return;
      if (type.name().equals("List")) {
        for (var item : (List<?>) value.data())
          addWitness((Named) type.arguments().get(0), (Value) item);
      } else if (type.name().equals("Nullable") || type.name().equals("Optional")) {
        var presence = (Presence) value.data();
        if (presence.present()) addWitness((Named) type.arguments().get(0), presence.value());
      } else {
        var data = (Data) value.data();
        var fields = schema.fieldsOf(type, data.tag(), data.fields());
        for (int i = 0; i < fields.size(); i++)
          addWitness((Named) fields.get(i).type(), data.fields().get(i));
      }
    }

    private boolean canGenerate(Named type, int budget) {
      if (budget < 1) return false;
      if (factories.containsKey(type.name())) return true;
      var request = new Request(type, budget);
      if (inhabited.containsKey(request)) return inhabited.get(request);
      boolean result;
      if (schema.isScalar(type)
          || type.name().equals("List")
          || type.name().equals("Nullable")
          || type.name().equals("Optional")) {
        result = true;
      } else {
        result =
            schema.constructors(type).stream()
                .anyMatch(
                    tag ->
                        schema.instances(type, tag).stream()
                            .anyMatch(choice -> allocation(choice.fields(), budget - 1) != null));
      }
      inhabited.put(request, result);
      return result;
    }

    private List<Integer> allocation(List<LawSpecSchema.Field> fields, int budget) {
      var costs = new ArrayList<Integer>();
      int remaining = budget;
      for (var field : fields) {
        int minimum = 1;
        while (minimum <= remaining && !canGenerate((Named) field.type(), minimum)) minimum++;
        if (minimum > remaining) return null;
        costs.add(minimum);
        remaining -= minimum;
      }
      for (int i = 0; i < costs.size(); i++) {
        costs.set(
            i, costs.get(i) + remaining / costs.size() + (i < remaining % costs.size() ? 1 : 0));
      }
      return costs;
    }

    Generator<Value> generate(Named type, int budget) {
      if (!canGenerate(type, budget)) {
        throw new IllegalArgumentException(
            "no inhabitant of "
                + LawSpecSchema.key(type)
                + " within structural node budget "
                + budget);
      }
      var request = new Request(type, budget);
      if (generators.containsKey(request)) return generators.get(request);
      if (factories.containsKey(type.name())) {
        var arguments = new ArrayList<Generator<Value>>();
        for (var argument : type.arguments()) {
          int remaining = Math.max(1, budget - 1);
          // An uninhabited parameter need not be stored by the native type.
          // JetCheck bounds rejection if its factory actually draws this child.
          arguments.add(
              canGenerate((Named) argument, remaining)
                  ? generate((Named) argument, remaining)
                  : Generator.constant(new Value("Unit", null)).suchThat(ignored -> false));
        }
        var source = factories.get(type.name()).create(schema, type, bits, symbols, arguments);
        if (source == null) {
          throw new IllegalArgumentException("native generator " + type.name() + " returned null");
        }
        var nativeGenerator =
            source.map(
                value -> {
                  try {
                    return schema.validate(type, value, bits, symbols);
                  } catch (RuntimeException error) {
                    throw new NativeFailure(type, error);
                  }
                });
        generators.put(request, nativeGenerator);
        return nativeGenerator;
      }
      Generator<Value> generator;
      if (schema.isScalar(type)) {
        generator = scalar.apply(type.name()).map(value -> schema.validate(type, value, bits));
      } else if (type.name().equals("List")) {
        var element = (Named) type.arguments().get(0);
        int maxLength = budget - 1;
        while (maxLength > 0 && !canGenerate(element, (budget - 1) / maxLength)) maxLength--;
        generator =
            Generator.integers(0, maxLength)
                .flatMap(
                    length ->
                        length == 0
                            ? Generator.constant(new Value(LawSpecSchema.key(type), List.of()))
                            : Generator.listsOf(
                                    IntDistribution.uniform(length, length),
                                    generate(element, (budget - 1) / length))
                                .map(
                                    values ->
                                        new Value(LawSpecSchema.key(type), List.copyOf(values))));
      } else if (type.name().equals("Nullable") || type.name().equals("Optional")) {
        var element = (Named) type.arguments().get(0);
        var absent =
            Generator.constant(new Value(LawSpecSchema.key(type), new Presence(false, null)));
        generator =
            canGenerate(element, budget - 1)
                ? Generator.anyOf(
                    absent,
                    generate(element, budget - 1)
                        .map(
                            value -> new Value(LawSpecSchema.key(type), new Presence(true, value))))
                : absent;
      } else {
        var variants = new ArrayList<Generator<Value>>();
        for (var tag : schema.constructors(type)) {
         for (var choice : schema.instances(type, tag)) {
          var fields = choice.fields();
          var budgets = allocation(fields, budget - 1);
          if (budgets == null) continue;
          var children = new ArrayList<Generator<Value>>();
          for (int i = 0; i < fields.size(); i++) {
            children.add(generate((Named) fields.get(i).type(), budgets.get(i)));
          }
          var keys = choice.keys();
          variants.add(
              Generator.from(
                  environment -> {
                    var values = new ArrayList<Value>();
                    for (var child : children) values.add(environment.generate(child));
                    for (var key : keys) values.add(LawSpecSchema.witnessText(key));
                    return unchecked
                        ? new Value(LawSpecSchema.key(type), new Data(tag, List.copyOf(values)))
                        : schema.construct(type, tag, values, bits);
                  }));
         }
        }
        generator = Generator.anyOf(variants);
        // Generated collections are canonicalised rather than filtered.
        if (LawSpecSchema.canonicalCollection(type.name())) {
          generator = generator.map(LawSpecSchema::canonical);
        }
      }
      if (witnesses.containsKey(type)) {
        var seeds = witnesses.get(type).stream().filter(value -> nodes(value) <= budget).toList();
        if (!seeds.isEmpty()) generator = Generator.anyOf(generator, Generator.sampledFrom(seeds));
      }
      if (unchecked && !schema.isScalar(type)) {
        generator =
            boundedFilter(
                generator,
                value -> {
                  try {
                    return schema.check(type, value, bits, symbols)
                        instanceof LawSpecSchema.Accepted;
                  } catch (RuntimeException error) {
                    // Keep evaluation errors for the outer checked result to report.
                    return true;
                  }
                },
                maxAttempts);
      }
      generators.put(request, generator);
      return generator;
    }
  }
}
