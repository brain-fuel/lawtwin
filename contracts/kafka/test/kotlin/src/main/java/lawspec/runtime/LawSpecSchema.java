package lawspec.runtime;

import java.math.BigInteger;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Function;
import lawspec.runtime.LawSpecRuntime.Data;
import lawspec.runtime.LawSpecRuntime.Presence;
import lawspec.runtime.LawSpecRuntime.Value;

/**
 * Runtime metadata derived from checked Core, independent of testing frameworks.
 *
 * Generated data types are described once, as Core schemas, and every value crossing between a law
 * and native code is validated against them, so an adapter cannot hand a law a value outside its
 * declared type. ref:DEC-typed-core-boundary ref:DEC-native-bindings-typed-identity
 */
public final class LawSpecSchema {
  public sealed interface TypeRef permits Parameter, Named {}

  public record Parameter(int index) implements TypeRef {
    public Parameter {
      if (index < 0) throw new IllegalArgumentException("negative schema parameter");
    }
  }

  public record Named(String name, List<TypeRef> arguments) implements TypeRef {
    public Named {
      arguments = List.copyOf(arguments);
    }

    public Named(String name, TypeRef... arguments) {
      this(name, List.of(arguments));
    }
  }

  public record Field(String name, TypeRef type) {}

  @FunctionalInterface
  public interface FieldPredicate {
    boolean test(
        LawSpecSchema schema,
        List<TypeRef> arguments,
        List<Value> fields,
        int bits,
        Map<String, Object> symbols);
  }

  public static final class RefinementViolation extends IllegalArgumentException {
    public RefinementViolation(String message) {
      super(message);
    }

    private RefinementViolation(String message, Throwable cause) {
      super(message, cause);
    }
  }

  public sealed interface ValueCheck permits Accepted, Rejected {}

  public record Accepted(Value value) implements ValueCheck {}

  public record Rejected(String reason) implements ValueCheck {}

  /**
   * A constructor; an indexed family's constructor also lists its index terms then its guards, in
   * prefix notation over field indices, and validation checks the guards.
   */
  public record Constructor(
      String tag,
      List<Field> fields,
      List<FieldPredicate> predicates,
      List<String> indices,
      List<Refinement> refinements,
      int existentials,
      List<Integer> witnesses) {
    // witnesses: existentials only a value determines (parameter numbers); their types travel
    // as the trailing Text witness fields.
    public Constructor {
      fields = List.copyOf(fields);
      predicates = List.copyOf(predicates);
      indices = List.copyOf(indices);
      refinements = List.copyOf(refinements);
      witnesses = List.copyOf(witnesses);
    }

    public Constructor(
        String tag,
        List<Field> fields,
        List<FieldPredicate> predicates,
        List<String> indices,
        List<Refinement> refinements,
        int existentials) {
      this(tag, fields, predicates, indices, refinements, existentials, List.of());
    }

    public Constructor(
        String tag, List<Field> fields, List<FieldPredicate> predicates, List<String> indices) {
      this(tag, fields, predicates, indices, List.of(), 0);
    }

    public Constructor(String tag, List<Field> fields, List<FieldPredicate> predicates) {
      this(tag, fields, predicates, List.of(), List.of(), 0);
    }

    public Constructor(String tag, List<Field> fields) {
      this(tag, fields, List.of(), List.of(), List.of(), 0);
    }
  }

  /**
   * A GADT constructor fixes a parameter to a pattern; its existentials are the parameters
   * numbered after the definition's own.
   */
  public record Refinement(int parameter, TypeRef pattern) {}

  /** A prefix index term: kind is "c", "f" or an operator. */
  private record IndexTerm(
      String kind, BigInteger value, int position, int index, IndexTerm left, IndexTerm right) {
    static IndexTerm parse(String[] tokens, int[] at) {
      if (at[0] >= tokens.length) throw new IllegalArgumentException("malformed index term");
      String token = tokens[at[0]++];
      if (token.startsWith("c")) {
        return new IndexTerm("c", new BigInteger(token.substring(1)), 0, 0, null, null);
      }
      if (token.startsWith("f")) {
        var parts = token.substring(1).split("\\.");
        return new IndexTerm(
            "f",
            null,
            Integer.parseInt(parts[0]),
            parts.length > 1 ? Integer.parseInt(parts[1]) : 0,
            null,
            null);
      }
      var left = parse(tokens, at);
      var right = parse(tokens, at);
      return new IndexTerm(token, null, 0, 0, left, right);
    }

    /** Natural index arithmetic; null when an operation has no natural value. */
    BigInteger evaluate(java.util.function.BiFunction<Integer, Integer, BigInteger> field) {
      if (kind.equals("c")) return value;
      if (kind.equals("f")) return field.apply(position, index);
      var x = left.evaluate(field);
      var y = right.evaluate(field);
      if (x == null || y == null) return null;
      return switch (kind) {
        case "+" -> x.add(y);
        case "-" -> x.compareTo(y) >= 0 ? x.subtract(y) : null;
        case "*" -> x.multiply(y);
        case "div" -> y.signum() > 0 ? x.divide(y) : null;
        case "mod" -> y.signum() > 0 ? x.mod(y) : null;
        case "^" -> y.compareTo(BigInteger.valueOf(64)) <= 0 ? x.pow(y.intValueExact()) : null;
        default -> throw new IllegalArgumentException("malformed index term");
      };
    }
  }

  private static boolean isIndexGuard(String text) {
    return text.startsWith("== ") || text.startsWith(">= ");
  }

  private void checkIndices(Named type, Constructor constructor, List<Value> fields) {
    var types = fields(type, constructor.tag());
    for (var text : constructor.indices()) {
      if (!isIndexGuard(text)) continue;
      var tokens = text.split(" ");
      var at = new int[] {1};
      var left = IndexTerm.parse(tokens, at);
      var right = IndexTerm.parse(tokens, at);
      java.util.function.BiFunction<Integer, Integer, BigInteger> field =
          (position, index) -> indexOf((Named) types.get(position).type(), fields.get(position), index);
      var x = left.evaluate(field);
      var y = right.evaluate(field);
      boolean holds =
          x != null && y != null && (tokens[0].equals("==") ? x.equals(y) : x.compareTo(y) >= 0);
      if (!holds) {
        throw new RefinementViolation(constructor.tag() + ": index guard " + text + " failed");
      }
    }
  }

  private BigInteger indexOf(Named type, Value value, int index) {
    var data = (Data) value.data();
    var constructor =
        definitions.get(type.name()).constructors().stream()
            .filter(candidate -> candidate.tag().equals(data.tag()))
            .findFirst()
            .orElseThrow();
    var terms = constructor.indices().stream().filter(text -> !isIndexGuard(text)).toList();
    if (index >= terms.size()) throw new IllegalArgumentException("no index for " + type.name());
    var types = fields(type, data.tag());
    return IndexTerm.parse(terms.get(index).split(" "), new int[] {0})
        .evaluate(
            (position, child) ->
                indexOf((Named) types.get(position).type(), data.fields().get(position), child));
  }

  /** A handle (handle == true) has no constructors: its values carry adapters' native objects. */
  public record Definition(
      String name, int parameters, List<Constructor> constructors, boolean handle) {
    public Definition {
      if (parameters < 0) throw new IllegalArgumentException("negative parameter count");
      constructors = List.copyOf(constructors);
      if (handle && !constructors.isEmpty())
        throw new IllegalArgumentException("a handle has no constructors");
    }

    public Definition(String name, int parameters, List<Constructor> constructors) {
      this(name, parameters, constructors, false);
    }
  }

  private static final Set<String> SCALARS =
      Set.of(
          "Bool",
          "Int8",
          "Int16",
          "Int32",
          "Int64",
          "UInt8",
          "UInt16",
          "UInt32",
          "UInt64",
          "IntSize",
          "UIntSize",
          "UIntPtr",
          "Integer",
          "BigInt",
          "BigUInt",
          "Decimal",
          "Rational",
          "Float32",
          "Float64",
          "Complex64",
          "Complex128",
          "Char",
          "CodePoint",
          "CodeUnit16",
          "Text",
          "CodePointText",
          "Utf16Text",
          "Bytes",
          "Symbol",
          "Unit",
          "Null",
          "Undefined");
  private static final Map<String, Integer> CONTAINERS =
      Map.of("List", 1, "Maybe", 1, "Either", 2, "Nullable", 1, "Optional", 1);
  private final Map<String, Definition> definitions;

  public LawSpecSchema(List<Definition> declarations) {
    var table = new HashMap<String, Definition>();
    var tags = new HashSet<String>();
    for (var definition : declarations) {
      if (SCALARS.contains(definition.name())
          || CONTAINERS.containsKey(definition.name())
          || table.putIfAbsent(definition.name(), definition) != null) {
        throw new IllegalArgumentException("duplicate schema type: " + definition.name());
      }
      for (var constructor : definition.constructors()) {
        if (!tags.add(constructor.tag()))
          throw new IllegalArgumentException("duplicate constructor tag");
      }
    }
    definitions = Map.copyOf(table);
    for (var definition : declarations) {
      for (var constructor : definition.constructors()) {
        var names = new HashSet<String>();
        for (var field : constructor.fields()) {
          if (!names.add(field.name())) throw new IllegalArgumentException("duplicate field name");
          checkType(field.type(), definition.parameters() + constructor.existentials());
        }
      }
    }
  }

  public boolean hasContracts() {
    return definitions.values().stream()
        .flatMap(definition -> definition.constructors().stream())
        .anyMatch(constructor -> !constructor.predicates().isEmpty());
  }

  private void checkType(TypeRef type, int parameters) {
    if (type instanceof Parameter variable) {
      if (variable.index() >= parameters)
        throw new IllegalArgumentException("unbound schema parameter");
      return;
    }
    if (!(type instanceof Named named)) throw new IllegalArgumentException("missing schema type");
    var definition = definitions.get(named.name());
    int arity =
        definition != null
            ? definition.parameters()
            : CONTAINERS.getOrDefault(named.name(), SCALARS.contains(named.name()) ? 0 : -1);
    if (arity < 0 || named.arguments().size() != arity) {
      throw new IllegalArgumentException("invalid schema type application: " + named.name());
    }
    for (var argument : named.arguments()) checkType(argument, parameters);
  }

  /** A constructor field's type at an instantiated type, with its existentials substituted. */
  public Named fieldType(Named type, String tag, int index) {
    return (Named) fields(type, tag).get(index).type();
  }

  /** Generated GADT codecs build cases whose refined types the schema has already checked. */
  @SuppressWarnings("unchecked")
  public static <T> T cast(Object value) {
    return (T) value;
  }

  /**
   * Arguments extended with the existentials a constructor's refinements bind; null when the
   * refinements do not match.
   */
  private static List<TypeRef> refine(
      Constructor constructor, List<TypeRef> arguments, int parameters) {
    var bound = new HashMap<Integer, TypeRef>();
    for (var refinement : constructor.refinements()) {
      if (!matches(refinement.pattern(), arguments.get(refinement.parameter()), arguments, parameters, bound)) {
        return null;
      }
    }
    var extended = new ArrayList<TypeRef>(arguments);
    for (int k = 0; k < constructor.existentials(); k++) {
      var found = bound.get(parameters + k);
      extended.add(found != null ? found : new Parameter(parameters + k));
    }
    return extended;
  }

  /** Types a generator may choose for an existential that only a value fixes. */
  public static final List<Named> WITNESS_POOL =
      List.of(new Named("Bool", List.of()), new Named("Int32", List.of()));

  /** A witness spells a type as its name, or a parenthesized application. */
  public static String witnessKey(TypeRef type) {
    var named = (Named) type;
    if (named.arguments().isEmpty()) return named.name();
    var parts = new ArrayList<String>();
    parts.add(named.name());
    for (var argument : named.arguments()) parts.add(witnessKey(argument));
    return "(" + String.join(" ", parts) + ")";
  }

  /** Reads a witness key back into a type reference. */
  public static Named parseWitness(String text) {
    var tokens =
        java.util.Arrays.stream(text.replace("(", " ( ").replace(")", " ) ").trim().split("\\s+"))
            .filter(token -> !token.isEmpty())
            .toList();
    int[] at = {0};
    var result = readWitness(tokens, at);
    if (at[0] != tokens.size()) throw new IllegalArgumentException("malformed type witness");
    return result;
  }

  private static Named readWitness(List<String> tokens, int[] at) {
    if (at[0] >= tokens.size()) throw new IllegalArgumentException("malformed type witness");
    if (!tokens.get(at[0]).equals("(")) return new Named(tokens.get(at[0]++), List.of());
    if (at[0] + 1 >= tokens.size()) throw new IllegalArgumentException("malformed type witness");
    var name = tokens.get(at[0] + 1);
    at[0] += 2;
    var arguments = new ArrayList<TypeRef>();
    while (at[0] < tokens.size() && !tokens.get(at[0]).equals(")"))
      arguments.add(readWitness(tokens, at));
    if (at[0] >= tokens.size()) throw new IllegalArgumentException("malformed type witness");
    at[0]++;
    return new Named(name, arguments);
  }

  private Constructor constructorOf(Named type, String tag) {
    var definition = definitions.get(type.name());
    if (definition == null) return null;
    return definition.constructors().stream()
        .filter(candidate -> candidate.tag().equals(tag))
        .findFirst()
        .orElse(null);
  }

  /** Field types with witnessed existentials read from the given witness keys. */
  private List<Field> witnessedFields(Named type, String tag, List<String> keys) {
    var fields = fields(type, tag);
    var constructor = constructorOf(type, tag);
    if (constructor == null || constructor.witnesses().isEmpty()) return fields;
    var known = new ArrayList<TypeRef>();
    for (int k = 0; k < keys.size(); k++) {
      int index = constructor.witnesses().get(k);
      while (known.size() <= index) known.add(new Parameter(known.size()));
      known.set(index, parseWitness(keys.get(k)));
    }
    return fields.stream()
        .map(field -> new Field(field.name(), substitute(field.type(), known)))
        .toList();
  }

  /** A value's fields at their types, with witnessed existentials read from its witnesses. */
  public List<Field> fieldsOf(Named type, String tag, List<Value> values) {
    var constructor = constructorOf(type, tag);
    if (constructor == null || constructor.witnesses().isEmpty()) return fields(type, tag);
    int count = constructor.witnesses().size();
    if (values.size() < count) throw new IllegalArgumentException("missing type witness");
    var keys = new ArrayList<String>();
    for (int k = 0; k < count; k++) {
      var witness = values.get(values.size() - count + k);
      if (!"Text".equals(witness.type()))
        throw new IllegalArgumentException("type witness must be text");
      keys.add((String) LawSpecRuntime.toNative("Text", witness, 64));
    }
    var result = witnessedFields(type, tag, keys);
    for (var field : result) checkType(field.type(), 0);
    return result;
  }

  /** A field's type at a value's witnesses, for generated codecs of existential fields. */
  public Named fieldType(Named type, String tag, int index, List<String> keys) {
    var result = witnessedFields(type, tag, keys).get(index).type();
    checkType(result, 0);
    return (Named) result;
  }

  /** One choice of witnesses: the declared fields at it, and the witness keys. */
  public record WitnessInstance(List<Field> fields, List<String> keys) {}

  /** Each choice of witnesses from the pool. */
  public List<WitnessInstance> instances(Named type, String tag) {
    var constructor = constructorOf(type, tag);
    var all = fields(type, tag);
    if (constructor == null || constructor.witnesses().isEmpty())
      return List.of(new WitnessInstance(all, List.of()));
    int count = constructor.witnesses().size();
    List<List<String>> choices = List.of(List.of());
    for (int k = 0; k < count; k++) {
      var next = new ArrayList<List<String>>();
      for (var choice : choices)
        for (var pooled : WITNESS_POOL) {
          var extended = new ArrayList<>(choice);
          extended.add(witnessKey(pooled));
          next.add(List.copyOf(extended));
        }
      choices = next;
    }
    var result = new ArrayList<WitnessInstance>();
    for (var keys : choices) {
      var resolved = witnessedFields(type, tag, keys);
      result.add(new WitnessInstance(resolved.subList(0, resolved.size() - count), keys));
    }
    return result;
  }

  /** The trailing witness keys of a value's fields, for generated codecs. */
  public static List<String> witnessKeys(List<Value> fields, int count) {
    var keys = new ArrayList<String>();
    for (int k = 0; k < count; k++)
      keys.add((String) LawSpecRuntime.toNative("Text", fields.get(fields.size() - count + k), 64));
    return keys;
  }

  /** A witness key as a Text value. */
  public static Value witnessText(String key) {
    return LawSpecRuntime.sequence("Text", key.codePoints().toArray());
  }

  private static boolean matches(
      TypeRef pattern,
      TypeRef actual,
      List<TypeRef> arguments,
      int parameters,
      Map<Integer, TypeRef> bound) {
    if (pattern instanceof Parameter parameter) {
      if (parameter.index() < parameters) return arguments.get(parameter.index()).equals(actual);
      var previous = bound.putIfAbsent(parameter.index(), actual);
      return previous == null || previous.equals(actual);
    }
    var named = (Named) pattern;
    if (!(actual instanceof Named other)
        || !named.name().equals(other.name())
        || named.arguments().size() != other.arguments().size()) {
      return false;
    }
    for (int i = 0; i < named.arguments().size(); i++) {
      if (!matches(named.arguments().get(i), other.arguments().get(i), arguments, parameters, bound)) {
        return false;
      }
    }
    return true;
  }

  public TypeRef substitute(TypeRef type, List<TypeRef> arguments) {
    if (type instanceof Parameter variable) {
      // A witnessed existential stays open until a value supplies it.
      if (variable.index() >= arguments.size()) return variable;
      return arguments.get(variable.index());
    }
    var named = (Named) type;
    return new Named(
        named.name(),
        named.arguments().stream().map(argument -> substitute(argument, arguments)).toList());
  }

  public boolean isScalar(Named type) {
    checkType(type, 0);
    return SCALARS.contains(type.name());
  }

  public List<String> constructors(Named type) {
    checkType(type, 0);
    var definition = definitions.get(type.name());
    if (definition != null)
      return definition.constructors().stream()
          .filter(constructor -> refine(constructor, type.arguments(), definition.parameters()) != null)
          .map(Constructor::tag)
          .toList();
    return switch (type.name()) {
      case "Maybe" -> List.of("Maybe::Nothing", "Maybe::Just");
      case "Either" -> List.of("Either::Left", "Either::Right");
      default -> throw new IllegalArgumentException("algebraic type required");
    };
  }

  public List<Field> fields(Named type, String tag) {
    checkType(type, 0);
    var definition = definitions.get(type.name());
    if (definition != null) {
      for (var constructor : definition.constructors()) {
        if (constructor.tag().equals(tag)) {
          var arguments = refine(constructor, type.arguments(), definition.parameters());
          if (arguments == null) {
            throw new IllegalArgumentException(
                "constructor " + tag + " is not a value of " + key(type));
          }
          return constructor.fields().stream()
              .map(field -> new Field(field.name(), substitute(field.type(), arguments)))
              .toList();
        }
      }
    } else if (type.name().equals("Maybe")) {
      if (tag.equals("Maybe::Nothing")) return List.of();
      if (tag.equals("Maybe::Just")) return List.of(new Field("value", type.arguments().get(0)));
    } else if (type.name().equals("Either")) {
      if (tag.equals("Either::Left")) return List.of(new Field("value", type.arguments().get(0)));
      if (tag.equals("Either::Right")) return List.of(new Field("value", type.arguments().get(1)));
    }
    throw new IllegalArgumentException("constructor " + tag + " does not belong to " + type.name());
  }

  public static String key(TypeRef type) {
    if (!(type instanceof Named named))
      throw new IllegalArgumentException("unbound schema parameter");
    if (named.arguments().isEmpty()) return named.name();
    if (named.arguments().size() == 1 && CONTAINERS.containsKey(named.name())) {
      return named.name() + " " + key(named.arguments().get(0));
    }
    var result = new StringBuilder(named.name());
    for (var argument : named.arguments()) result.append(" (").append(key(argument)).append(")");
    return result.toString();
  }

  private sealed interface PayloadPlan permits IgnorePayload, PayloadSlot, PayloadApplication {}

  private enum IgnorePayload implements PayloadPlan {
    INSTANCE
  }

  private record PayloadSlot(int index) implements PayloadPlan {}

  private record PayloadApplication(String name, List<PayloadPlan> arguments)
      implements PayloadPlan {}

  private static PayloadPlan payloadRecipe(TypeRef type, List<PayloadPlan> arguments) {
    if (type instanceof Parameter parameter) return arguments.get(parameter.index());
    var named = (Named) type;
    var children =
        named.arguments().stream().map(child -> payloadRecipe(child, arguments)).toList();
    return children.stream().allMatch(child -> child == IgnorePayload.INSTANCE)
        ? IgnorePayload.INSTANCE
        : new PayloadApplication(named.name(), children);
  }

  /** Check stored type arguments after full value and constructor-contract validation. */
  public Value allPayloads(
      Named type,
      Value value,
      List<Function<Value, Value>> predicates,
      int bits,
      Map<String, Object> symbols) {
    checkType(type, 0);
    if (!definitions.containsKey(type.name()) && !CONTAINERS.containsKey(type.name()))
      throw new IllegalArgumentException("payload predicates require a data type");
    predicates = List.copyOf(predicates);
    if (predicates.size() != type.arguments().size())
      throw new IllegalArgumentException("payload predicate arity mismatch");
    var checked = validate(type, value, bits, symbols);
    var arguments = new ArrayList<PayloadPlan>();
    for (int index = 0; index < predicates.size(); index++) arguments.add(new PayloadSlot(index));
    return LawSpecRuntime.bool(
        walkPayload(new PayloadApplication(type.name(), arguments), checked, predicates));
  }

  private boolean contextualPayload(
      PayloadPlan plan, Value value, List<Function<Value, Value>> predicates, String context) {
    try {
      return walkPayload(plan, value, predicates);
    } catch (RuntimeException error) {
      throw new IllegalArgumentException(context + ": " + error.getMessage(), error);
    }
  }

  private boolean walkPayload(
      PayloadPlan plan, Value value, List<Function<Value, Value>> predicates) {
    if (plan == IgnorePayload.INSTANCE) return true;
    if (plan instanceof PayloadSlot slot)
      return LawSpecRuntime.truth(predicates.get(slot.index()).apply(value));
    var application = (PayloadApplication) plan;
    var arguments = application.arguments();
    var name = application.name();
    if (name.equals("List")) {
      var values = (List<?>) value.data();
      for (int index = 0; index < values.size(); index++)
        if (!contextualPayload(
            arguments.get(0), (Value) values.get(index), predicates, "List[" + index + "]"))
          return false;
      return true;
    }
    if (name.equals("Nullable") || name.equals("Optional")) {
      var presence = (Presence) value.data();
      return !presence.present()
          || contextualPayload(arguments.get(0), presence.value(), predicates, name + ".value");
    }
    var data = (Data) value.data();
    if (name.equals("Maybe"))
      return data.tag().equals("Maybe::Nothing")
          || contextualPayload(
              arguments.get(0), data.fields().get(0), predicates, "Maybe::Just.value");
    if (name.equals("Either")) {
      int index = data.tag().equals("Either::Left") ? 0 : 1;
      return contextualPayload(
          arguments.get(index), data.fields().get(0), predicates, data.tag() + ".value");
    }
    var constructor =
        definitions.get(name).constructors().stream()
            .filter(candidate -> candidate.tag().equals(data.tag()))
            .findFirst()
            .orElseThrow();
    for (int index = 0; index < constructor.fields().size(); index++) {
      var field = constructor.fields().get(index);
      if (!contextualPayload(
          payloadRecipe(field.type(), arguments),
          data.fields().get(index),
          predicates,
          data.tag() + "." + field.name())) return false;
    }
    return true;
  }

  public Value validate(Named type, Value value, int bits) {
    return validate(type, value, bits, new HashMap<>());
  }

  public ValueCheck check(Named type, Value value, int bits, Map<String, Object> symbols) {
    try {
      return new Accepted(validate(type, value, bits, symbols));
    } catch (RefinementViolation rejected) {
      return new Rejected(rejected.getMessage());
    }
  }

  public Value validate(Named type, Value value, int bits, Map<String, Object> symbols) {
    return validate(type, value, bits, symbols, true);
  }

  // A shallow check (deep == false) trusts fields, which were checked when built.
  private Value validate(
      Named type, Value value, int bits, Map<String, Object> symbols, boolean deep) {
    checkType(type, 0);
    if (bits != 32 && bits != 64)
      throw new IllegalArgumentException("machineBits must be 32 or 64");
    if (value == null || !key(type).equals(value.type())) {
      throw new IllegalArgumentException("invalid " + key(type) + " representation");
    }
    var handle = definitions.get(type.name());
    if (handle != null && handle.handle()) {
      if (!(value.data() instanceof LawSpecRuntime.Handle))
        throw new IllegalArgumentException("handle required: " + type.name());
      return value;
    }
    if (definitions.containsKey(type.name())
        || type.name().equals("Maybe")
        || type.name().equals("Either")) {
      if (!(value.data() instanceof Data data))
        throw new IllegalArgumentException("tagged data required");
      if (fields(type, data.tag()).size() != data.fields().size())
        throw new IllegalArgumentException("invalid constructor arity");
      var types = fieldsOf(type, data.tag(), data.fields());
      var checked = new ArrayList<Value>();
      for (int index = 0; index < types.size(); index++) {
        var field = types.get(index);
        if (!deep) {
          checked.add(data.fields().get(index));
          continue;
        }
        checked.add(
            validateField(
                (Named) field.type(),
                data.fields().get(index),
                bits,
                data.tag() + "." + field.name(),
                symbols));
      }
      var definition = definitions.get(type.name());
      if (definition != null) {
        var constructor =
            definition.constructors().stream()
                .filter(candidate -> candidate.tag().equals(data.tag()))
                .findFirst()
                .orElseThrow();
        var logical = List.copyOf(checked);
        for (int index = 0; index < constructor.predicates().size(); index++) {
          boolean accepted;
          String context = data.tag() + ": field refinement " + (index + 1);
          try {
            accepted =
                constructor
                    .predicates()
                    .get(index)
                    .test(this, type.arguments(), logical, bits, symbols);
          } catch (RuntimeException error) {
            throw new IllegalArgumentException(context + ": " + error.getMessage(), error);
          }
          if (!accepted) throw new RefinementViolation(context + " failed");
        }
        checkIndices(type, constructor, logical);
      }
      return new Value(key(type), new Data(data.tag(), checked));
    }
    if (type.name().equals("List")) {
      if (!(value.data() instanceof List<?> items))
        throw new IllegalArgumentException("list required");
      var checked = new ArrayList<Value>();
      for (int index = 0; index < items.size(); index++) {
        if (!(items.get(index) instanceof Value element))
          throw new IllegalArgumentException("invalid list element");
        checked.add(
            validateField(
                (Named) type.arguments().get(0), element, bits, "[" + index + "]", symbols));
      }
      return new Value(key(type), List.copyOf(checked));
    }
    if (type.name().equals("Nullable") || type.name().equals("Optional")) {
      if (!(value.data() instanceof Presence p))
        throw new IllegalArgumentException("tagged presence required");
      if (!p.present() && p.value() != null)
        throw new IllegalArgumentException("absent payload must be empty");
      return new Value(
          key(type),
          new Presence(
              p.present(),
              p.present()
                  ? validateField(
                      (Named) type.arguments().get(0), p.value(), bits, "present", symbols)
                  : null));
    }
    return LawSpecRuntime.validate(type.name(), value, bits);
  }

  private Value validateField(
      Named type, Value value, int bits, String context, Map<String, Object> symbols) {
    try {
      return validate(type, value, bits, symbols);
    } catch (RefinementViolation error) {
      throw new RefinementViolation(context + ": " + error.getMessage(), error);
    } catch (IllegalArgumentException | ArithmeticException error) {
      throw new IllegalArgumentException(context + ": " + error.getMessage(), error);
    }
  }

  public Value construct(Named type, String tag, List<Value> fields, int bits) {
    return construct(type, tag, fields, bits, new HashMap<>());
  }

  public Value construct(
      Named type, String tag, List<Value> fields, int bits, Map<String, Object> symbols) {
    // Values in generated code are checked where they are built, decoded or
    // drawn, so constructing checks only the new node and matching only
    // dispatches; a deep check of each would make recursion quadratic.
    return validate(type, new Value(key(type), new Data(tag, fields)), bits, symbols, false);
  }

  public Value match(Named type, Value value, int bits, Function<Data, Value> branch) {
    return match(type, value, bits, new HashMap<>(), branch);
  }

  public Value match(
      Named type,
      Value value,
      int bits,
      Map<String, Object> symbols,
      Function<Data, Value> branch) {
    var checked = value;
    if (!(checked.data() instanceof Data data))
      throw new IllegalArgumentException("algebraic data required");
    return branch.apply(data);
  }

  public boolean equal(Named type, Value left, Value right, int bits) {
    return equal(type, left, right, bits, new HashMap<>());
  }

  public boolean equal(Named type, Value left, Value right, int bits, Map<String, Object> symbols) {
    return equalValidated(
        validate(type, left, bits, symbols), validate(type, right, bits, symbols));
  }

  /** A native bridge always validates before decoding and after encoding. */
  public interface Codec<T> {
    Named type();

    Value encode(T value);

    T decode(Value value);
  }

  public <T> Codec<T> codec(
      Named type, int bits, Function<T, Value> encode, Function<Value, T> decode) {
    return codec(type, bits, new HashMap<>(), encode, decode);
  }

  public <T> Codec<T> codec(
      Named type,
      int bits,
      Map<String, Object> symbols,
      Function<T, Value> encode,
      Function<Value, T> decode) {
    checkType(type, 0);
    if (bits != 32 && bits != 64)
      throw new IllegalArgumentException("machineBits must be 32 or 64");
    return new Codec<>() {
      public Named type() {
        return type;
      }

      public Value encode(T value) {
        return validate(type, encode.apply(value), bits, symbols);
      }

      public T decode(Value value) {
        return decode.apply(validate(type, value, bits, symbols));
      }
    };
  }

  public static <T> Value encodeField(Codec<T> codec, T value, String context) {
    try {
      return codec.encode(value);
    } catch (RefinementViolation error) {
      throw new RefinementViolation(context + ": " + error.getMessage(), error);
    } catch (IllegalArgumentException | ArithmeticException | ClassCastException error) {
      throw new IllegalArgumentException(context + ": " + error.getMessage(), error);
    }
  }

  public <T> Codec<T> scalar(String name, int bits, Class<T> nativeClass) {
    if (!SCALARS.contains(name)) throw new IllegalArgumentException("scalar type required");
    return codec(
        new Named(name),
        bits,
        value -> LawSpecRuntime.fromNative(name, nativeClass.cast(value), bits),
        value -> nativeClass.cast(LawSpecRuntime.toNative(name, value, bits)));
  }

  public Codec<Value> supported(Named type, int bits) {
    return supported(type, bits, new HashMap<>());
  }

  public Codec<Value> supported(Named type, int bits, Map<String, Object> symbols) {
    return codec(type, bits, symbols, Function.identity(), Function.identity());
  }

  private static final String DURATION = "lawspec.time::type::Duration";
  private static final java.math.BigInteger MICROS_PER_SECOND = java.math.BigInteger.valueOf(1_000_000);

  /**
   * A Duration natively: a java.time.Duration of whole microseconds. One that
   * is negative or has a fraction of a microsecond is not a Duration.
   */
  public Codec<java.time.Duration> duration(int bits, Map<String, Object> symbols) {
    var type = new Named(DURATION);
    return codec(
        type,
        bits,
        symbols,
        duration -> {
          if (duration.isNegative() || duration.getNano() % 1000 != 0)
            throw new IllegalArgumentException("a Duration is a non-negative whole number of microseconds");
          var micros = java.math.BigInteger.valueOf(duration.getSeconds()).multiply(MICROS_PER_SECOND)
              .add(java.math.BigInteger.valueOf(duration.getNano() / 1000));
          return new Value(key(type), new Data(DURATION + "::Duration", List.of(new Value("Integer", micros))));
        },
        value -> {
          var micros = (java.math.BigInteger) ((Data) value.data()).fields().get(0).data();
          var parts = micros.divideAndRemainder(MICROS_PER_SECOND);
          return java.time.Duration.ofSeconds(parts[0].longValueExact(), parts[1].longValueExact() * 1000);
        });
  }

  private static final String COLLECTIONS = "lawspec.collections::type::";

  /** Whether a type's values keep sorted, distinct items (a Set) or keys (a KeyVal). */
  public static boolean canonicalCollection(String name) {
    return name.equals(COLLECTIONS + "Set") || name.equals(COLLECTIONS + "KeyVal");
  }

  /** A Set or KeyVal with its items sorted by key, keeping the last of equal keys. */
  public static Value canonical(Value collection) {
    var data = (Data) collection.data();
    var list = data.fields().get(0);
    boolean keyed = collection.type().startsWith(COLLECTIONS + "KeyVal");
    Function<Value, Value> key = item -> keyed ? ((Data) item.data()).fields().get(0) : item;
    var items = new ArrayList<Value>();
    for (var item : (List<?>) list.data()) items.add((Value) item);
    items.sort((a, b) -> LawSpecRuntime.compareValues(key.apply(a), key.apply(b)));
    var distinct = new ArrayList<Value>();
    for (var item : items) {
      if (!distinct.isEmpty()
          && LawSpecRuntime.compareValues(key.apply(distinct.get(distinct.size() - 1)), key.apply(item)) == 0) {
        distinct.set(distinct.size() - 1, item);
      } else {
        distinct.add(item);
      }
    }
    return new Value(collection.type(), new Data(data.tag(), List.of(new Value(list.type(), List.copyOf(distinct)))));
  }

  /** A Set natively, in canonical order when LawSpec builds it. */
  public <T> Codec<java.util.Set<T>> set(Codec<T> element, int bits, Map<String, Object> symbols) {
    var type = new Named(COLLECTIONS + "Set", element.type());
    var listKey = key(new Named("List", element.type()));
    return codec(
        type,
        bits,
        symbols,
        values -> canonical(new Value(key(type), new Data(COLLECTIONS + "Set::SetItems",
            List.of(new Value(listKey, values.stream().map(element::encode).toList()))))),
        value -> {
          var result = new java.util.LinkedHashSet<T>();
          for (var item : items(value)) result.add(element.decode(item));
          return java.util.Collections.unmodifiableSet(result);
        });
  }

  /** A KeyVal natively, in canonical key order when LawSpec builds it. */
  public <K, V> Codec<java.util.Map<K, V>> keyVal(
      Codec<K> keys, Codec<V> values, int bits, Map<String, Object> symbols) {
    var type = new Named(COLLECTIONS + "KeyVal", keys.type(), values.type());
    var entryType = new Named(COLLECTIONS + "Entry", keys.type(), values.type());
    var listKey = key(new Named("List", entryType));
    return codec(
        type,
        bits,
        symbols,
        map -> canonical(new Value(key(type), new Data(COLLECTIONS + "KeyVal::KeyValEntries",
            List.of(new Value(listKey, map.entrySet().stream()
                .map(entry -> new Value(key(entryType), new Data(COLLECTIONS + "Entry::Entry",
                    List.of(keys.encode(entry.getKey()), values.encode(entry.getValue())))))
                .toList()))))),
        value -> {
          var result = new java.util.LinkedHashMap<K, V>();
          for (var entry : items(value)) {
            var fields = ((Data) entry.data()).fields();
            result.put(keys.decode(fields.get(0)), values.decode(fields.get(1)));
          }
          return java.util.Collections.unmodifiableMap(result);
        });
  }

  /** A Queue, Stack or Deque natively: an ArrayDeque from its head (a Stack's top). */
  public <T> Codec<java.util.ArrayDeque<T>> sequence(
      String name, Codec<T> element, int bits, Map<String, Object> symbols) {
    var type = new Named(COLLECTIONS + name, element.type());
    var listKey = key(new Named("List", element.type()));
    return codec(
        type,
        bits,
        symbols,
        deque -> new Value(key(type), new Data(COLLECTIONS + name + "::" + name + "Items",
            List.of(new Value(listKey, deque.stream().map(element::encode).toList())))),
        value -> {
          var result = new java.util.ArrayDeque<T>();
          for (var item : items(value)) result.add(element.decode(item));
          return result;
        });
  }

  private static List<Value> items(Value collection) {
    var result = new ArrayList<Value>();
    for (var item : (List<?>) ((Data) collection.data()).fields().get(0).data()) result.add((Value) item);
    return result;
  }

  public <T> Codec<List<T>> list(Codec<T> element, int bits) {
    return list(element, bits, new HashMap<>());
  }

  public <T> Codec<List<T>> list(Codec<T> element, int bits, Map<String, Object> symbols) {
    var type = new Named("List", element.type());
    return codec(
        type,
        bits,
        symbols,
        values -> new Value(key(type), values.stream().map(element::encode).toList()),
        value -> {
          var values = new ArrayList<T>();
          for (var item : (List<?>) value.data()) values.add(element.decode((Value) item));
          return values;
        });
  }

  public <T> Codec<LawSpecRuntime.Maybe<T>> maybe(Codec<T> element, int bits) {
    return maybe(element, bits, new HashMap<>());
  }

  public <T> Codec<LawSpecRuntime.Maybe<T>> maybe(
      Codec<T> element, int bits, Map<String, Object> symbols) {
    var type = new Named("Maybe", element.type());
    return codec(
        type,
        bits,
        symbols,
        value ->
            switch (value) {
              case LawSpecRuntime.Nothing<T> ignored ->
                  construct(type, "Maybe::Nothing", List.of(), bits, symbols);
              case LawSpecRuntime.Just<T> just ->
                  construct(
                      type, "Maybe::Just", List.of(element.encode(just.value())), bits, symbols);
            },
        value -> {
          var data = (Data) value.data();
          return data.tag().equals("Maybe::Nothing")
              ? new LawSpecRuntime.Nothing<>()
              : new LawSpecRuntime.Just<>(element.decode(data.fields().get(0)));
        });
  }

  public <L, R> Codec<LawSpecRuntime.Either<L, R>> either(Codec<L> left, Codec<R> right, int bits) {
    return either(left, right, bits, new HashMap<>());
  }

  public <L, R> Codec<LawSpecRuntime.Either<L, R>> either(
      Codec<L> left, Codec<R> right, int bits, Map<String, Object> symbols) {
    var type = new Named("Either", left.type(), right.type());
    return codec(
        type,
        bits,
        symbols,
        value ->
            switch (value) {
              case LawSpecRuntime.Left<L, R> item ->
                  construct(
                      type, "Either::Left", List.of(left.encode(item.value())), bits, symbols);
              case LawSpecRuntime.Right<L, R> item ->
                  construct(
                      type, "Either::Right", List.of(right.encode(item.value())), bits, symbols);
            },
        value -> {
          var data = (Data) value.data();
          return data.tag().equals("Either::Left")
              ? new LawSpecRuntime.Left<>(left.decode(data.fields().get(0)))
              : new LawSpecRuntime.Right<>(right.decode(data.fields().get(0)));
        });
  }

  private boolean equalValidated(Value left, Value right) {
    if (left.data() instanceof Data a && right.data() instanceof Data b) {
      if (!a.tag().equals(b.tag()) || a.fields().size() != b.fields().size()) return false;
      for (int index = 0; index < a.fields().size(); index++) {
        if (!equalValidated(a.fields().get(index), b.fields().get(index))) return false;
      }
      return true;
    }
    if (left.type().startsWith("List ")) {
      var a = (List<?>) left.data();
      var b = (List<?>) right.data();
      if (a.size() != b.size()) return false;
      for (int index = 0; index < a.size(); index++) {
        if (!equalValidated((Value) a.get(index), (Value) b.get(index))) return false;
      }
      return true;
    }
    if (left.data() instanceof Presence a && right.data() instanceof Presence b) {
      return a.present() == b.present() && (!a.present() || equalValidated(a.value(), b.value()));
    }
    return LawSpecRuntime.equal(left, right);
  }
}
