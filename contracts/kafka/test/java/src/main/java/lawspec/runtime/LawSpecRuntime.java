package lawspec.runtime;

import java.math.BigDecimal;
import java.math.BigInteger;
import java.math.RoundingMode;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.NoSuchElementException;
import java.util.Objects;
import java.util.Random;
import java.util.function.BiFunction;
import java.util.function.BiPredicate;
import java.util.function.Function;
import java.util.function.Supplier;

/**
 * Portable scalar and collection values, independent of test frameworks.
 *
 * <p>A law must mean the same thing on every target, so this class implements LawSpec's own
 * arithmetic, equality and conversions instead of Java's: integers are exact and reach a bounded
 * type only through a checked conversion, never wrapping as int and long do, exact division gives a
 * rational, decimals ignore any MathContext, and floats follow IEEE 754 at their declared
 * precision. It is emitted unchanged into every generated Java and Kotlin project, so the generated
 * tests and the adapters share one definition of the domain. ref:DEC-portable-exact-arithmetic
 * ref:ieee-754 ref:decimal-arithmetic ref:DEC-typed-core-boundary
 */
public final class LawSpecRuntime
{
  private LawSpecRuntime()
  {}

  /** A value is materialized in each test invocation's fresh symbol context. */
  @FunctionalInterface
  public interface FiniteValue
  {
    Value create(Map<String, Object> symbols);
  }

  /** Ordered finite-cover data. The planner establishes admission and completeness. */
  public sealed interface FiniteCover permits FiniteEnd, FiniteChoices
  {}

  private record FiniteEnd(long repetitions) implements FiniteCover
  {}

  private record FiniteChoices(List<FiniteBranch> alternatives) implements FiniteCover
  {}

  @FunctionalInterface
  public interface FiniteDomain
  {
    Iterator<FiniteValue> factories();
  }

  public record FiniteBranch(FiniteDomain domain, FiniteCover suffix)
  {}

  /** A stable, one-based case index and deferred values; no mutable values are shared. */
  public record FiniteCase(long index, List<FiniteValue> factories)
  {
    public List<Value> values(Map<String, Object> symbols)
    {
      return factories.stream().map(factory -> factory.create(symbols)).toList();
    }
  }

  public static FiniteCover finiteEnd(long repetitions)
  {
    if (repetitions < 0)
    {
      throw new IllegalArgumentException("negative finite-cover repetition count");
    }
    return new FiniteEnd(repetitions);
  }

  public static FiniteCover finiteChoices(FiniteBranch... alternatives)
  {
    return new FiniteChoices(List.of(alternatives));
  }

  public static FiniteBranch finiteBranch(FiniteDomain domain, FiniteCover suffix)
  {
    return new FiniteBranch(domain, suffix);
  }

  public static FiniteDomain finiteValues(FiniteValue... factories)
  {
    var stored = List.of(factories);
    return stored::iterator;
  }

  public static FiniteDomain finiteRange(String type, String first, String last)
  {
    var lower = new BigInteger(first);
    var upper = new BigInteger(last);
    return () -> new Iterator<>()
    {
      private BigInteger value = lower;

      @Override
      public boolean hasNext()
      {
        return value.compareTo(upper) <= 0;
      }

      @Override
      public FiniteValue next()
      {
        if (!hasNext())
        {
          throw new NoSuchElementException();
        }
        var current = value;
        value = current.add(BigInteger.ONE);
        return symbols -> integer(type, current.toString());
      }
    };
  }

  private static Iterator<List<FiniteValue>> finiteRows(FiniteCover cover)
  {
    if (cover instanceof FiniteEnd end)
    {
      return new Iterator<>()
      {
        private long remaining = end.repetitions();

        @Override
        public boolean hasNext()
        {
          return remaining > 0;
        }

        @Override
        public List<FiniteValue> next()
        {
          if (!hasNext())
          {
            throw new NoSuchElementException();
          }
          remaining--;
          return List.of();
        }
      };
    }
    var choices = (FiniteChoices) cover;
    return new Iterator<>()
    {
      private final Iterator<FiniteBranch> alternatives = choices.alternatives().iterator();
      private Iterator<FiniteValue> values = Collections.emptyIterator();
      private Iterator<List<FiniteValue>> suffixes = Collections.emptyIterator();
      private FiniteCover suffix;
      private FiniteValue first;

      @Override
      public boolean hasNext()
      {
        while (!suffixes.hasNext())
        {
          if (values.hasNext())
          {
            first = values.next();
            suffixes = finiteRows(suffix);
          }
          else if (alternatives.hasNext())
          {
            var branch = alternatives.next();
            values = branch.domain().factories();
            suffix = branch.suffix();
          }
          else
          {
            return false;
          }
        }
        return true;
      }

      @Override
      public List<FiniteValue> next()
      {
        if (!hasNext())
        {
          throw new NoSuchElementException();
        }
        var rest = suffixes.next();
        var row = new ArrayList<FiniteValue>(rest.size() + 1);
        row.add(first);
        row.addAll(rest);
        return List.copyOf(row);
      }
    };
  }

  /** Walk shared suffixes lazily in the planner's order, preserving duplicate observations. */
  public static Iterable<FiniteCase> finiteCases(FiniteCover cover)
  {
    return () -> new Iterator<>()
    {
      private final Iterator<List<FiniteValue>> rows = finiteRows(cover);
      private long index;

      @Override
      public boolean hasNext()
      {
        return rows.hasNext();
      }

      @Override
      public FiniteCase next()
      {
        return new FiniteCase(++index, rows.next());
      }
    };
  }

  /**
   * The native JUnit source may select one one-based case with -Dlawspec.case=N.
   * An invalid or absent selected row fails discovery instead of yielding an empty passing source.
   */
  public static Iterable<FiniteCase> selectedFiniteCases(FiniteCover cover, String count)
  {
    var selection = System.getProperty("lawspec.case");
    if (selection == null)
    {
      selection = System.getenv("LAWSPEC_CASE");
    }
    if (selection == null || selection.isBlank())
    {
      return finiteCases(cover);
    }
    BigInteger wanted;
    try
    {
      wanted = new BigInteger(selection);
    }
    catch (NumberFormatException error)
    {
      throw new IllegalArgumentException("lawspec.case must be a positive case index", error);
    }
    if (wanted.signum() <= 0 || wanted.compareTo(new BigInteger(count)) > 0)
    {
      throw new IllegalArgumentException("lawspec.case must be between 1 and " + count);
    }
    long index = wanted.longValueExact();
    return () -> {
      var rows = finiteCases(cover).iterator();
      for (long before = 1; before < index; before++)
      {
        rows.next();
      }
      var row = rows.next();
      if (row.index() != index)
      {
        throw new IllegalStateException("finite cover did not contain the selected case");
      }
      return List.of(row).iterator();
    };
  }

  public record Value(String type, Object data)
  {
    @Override
    public String toString()
    {
      return "Value[type=" + type + ", data=" + shownData() + "]";
    }

    // Text shows as the string it is, so a failure reads as the law was written.
    private String shownData()
    {
      boolean text =
          type.equals("Text") || type.equals("CodePointText") || type.equals("Utf16Text");
      if (text && data instanceof List<?> units)
      {
        var shown = new StringBuilder("\"");
        for (Object unit : units)
        {
          if (!(unit instanceof Integer point))
          {
            return String.valueOf(data);
          }
          if (type.equals("Utf16Text"))
          {
            shown.append((char) (int) point);
          }
          else
          {
            shown.appendCodePoint(point);
          }
        }
        return shown.append('"').toString();
      }
      return data instanceof int[] units ? Arrays.toString(units) : String.valueOf(data);
    }
  }

  public record Complex(double real, double imaginary)
  {}

  /**
   * A handle's logical data: the native object an adapter made, passed along unopened. Two handles
   * are equal only when they hold the same object; they have no portable order.
   */
  public static final class Handle
  {
    private final Object target;

    public Handle(Object target)
    {
      this.target = Objects.requireNonNull(target, "a handle cannot be null");
    }

    public Object target()
    {
      return target;
    }

    @Override
    public boolean equals(Object other)
    {
      return other instanceof Handle handle && handle.target == target;
    }

    @Override
    public int hashCode()
    {
      return System.identityHashCode(target);
    }

    @Override
    public String toString()
    {
      return "Handle[" + target.getClass().getName() + "]";
    }
  }

  /** A handle's logical value, carrying its native object. */
  public static Value handle(String type, Object target)
  {
    return new Value(type, new Handle(target));
  }

  /** The native object a handle's logical value carries. */
  public static Object handleTarget(Value value)
  {
    if (!(value.data() instanceof Handle handle))
    {
      throw new IllegalArgumentException("handle required: " + value.type());
    }
    return handle.target();
  }

  private static final Map<Object, Integer> HANDLE_LABELS = new java.util.IdentityHashMap<>();
  private static final Map<String, Integer> HANDLE_COUNTS = new java.util.HashMap<>();

  /** A handle's stable label, such as Jobs#1: its type's name, numbered by first appearance. */
  private static String handleLabel(String type, Object target)
  {
    int cut = type.lastIndexOf("::");
    String name = cut < 0 ? type : type.substring(cut + 2);
    synchronized (HANDLE_LABELS)
    {
      Integer number = HANDLE_LABELS.get(target);
      if (number == null)
      {
        number = HANDLE_COUNTS.merge(name, 1, Integer::sum);
        HANDLE_LABELS.put(target, number);
      }
      return name + "#" + number;
    }
  }

  public record SymbolValue(String description)
  {}

  public record Presence(boolean present, Value value)
  {}

  public sealed interface Maybe<T> permits Absent, Present
  {}

  public record Absent<T>() implements Maybe<T>
  {}

  public record Present<T>(T value) implements Maybe<T>
  {}

  public sealed interface Either<L, R> permits Left, Right
  {}

  public record Left<L, R>(L value) implements Either<L, R>
  {}

  public record Right<L, R>(R value) implements Either<L, R>
  {}

  public record Data(String tag, List<Value> fields)
  {
    public Data
    {
      fields = List.copyOf(fields);
    }
  }

  public record Ratio(BigInteger n, BigInteger d)
  {
    public Ratio
    {
      if (d.signum() == 0)
      {
        throw new ArithmeticException("exact division by zero");
      }
      if (d.signum() < 0)
      {
        n = n.negate();
        d = d.negate();
      }
      var gcd = n.gcd(d);
      n = n.divide(gcd);
      d = d.divide(gcd);
    }
  }

  private static final java.util.regex.Pattern INTEGER_TYPE =
      java.util.regex.Pattern.compile("(U?Int(8|16|32|64|Size)|UIntPtr|BigU?Int|Integer)");

  public static boolean integerType(String t)
  {
    return INTEGER_TYPE.matcher(t).matches();
  }

  public static boolean exactType(String t)
  {
    return integerType(t) || t.equals("Decimal") || t.equals("Rational");
  }

  public static Value integer(String t, String n)
  {
    return new Value(t, new BigInteger(n));
  }

  public static Value decimal(String c, String e)
  {
    return new Value(
        "Decimal", new BigDecimal(new BigInteger(c), Math.negateExact(Integer.parseInt(e))));
  }

  public static Value rational(String n, String d)
  {
    return new Value("Rational", new Ratio(new BigInteger(n), new BigInteger(d)));
  }

  public static Value bool(boolean b)
  {
    return new Value("Bool", b);
  }

  public static Value floating(String t, String bits)
  {
    return new Value(
        t,
        t.equals("Float32")
            ? (double) Float.intBitsToFloat(Integer.parseUnsignedInt(bits, 16))
            : Double.longBitsToDouble(Long.parseUnsignedLong(bits, 16)));
  }

  public static Value complex(String t, Value r, Value i)
  {
    return new Value(t, new Complex((double) r.data, (double) i.data));
  }

  public static Value sequence(String t, int[] units)
  {
    return new Value(t, Arrays.stream(units).boxed().toList());
  }

  /** Text written as the string it is: Utf16Text by its UTF-16 units, other text by code points. */
  public static Value text(String t, String value)
  {
    return sequence(
        t, t.equals("Utf16Text") ? value.chars().toArray() : value.codePoints().toArray());
  }

  /**
   * A native binding's error, as its bridge throws it: a value that fails to convert or validate
   * is reported in the binding's context; any other error is unchanged.
   */
  public static RuntimeException nativeBindingFailure(String context, RuntimeException error)
  {
    if (error instanceof IllegalArgumentException
        || error instanceof IllegalStateException
        || error instanceof ClassCastException
        || error instanceof ArithmeticException)
    {
      return new IllegalArgumentException(context + ": " + error.getMessage(), error);
    }
    return error;
  }

  /**
   * A law's expectation in a generated test: both sides are equal, or the failure names the law,
   * its case and both values.
   */
  public static void check(String context, Supplier<Value> actual, Supplier<Value> expected)
  {
    try
    {
      var a = actual.get();
      var b = expected.get();
      if (!equal(a, b))
      {
        throw new AssertionError(context + " | actual=" + a + " expected=" + b + difference(a, b));
      }
    }
    catch (RuntimeException error)
    {
      throw new AssertionError(context, error);
    }
  }

  /** A property's drawn inputs and the symbols they were drawn with. */
  public record Inputs(List<Value> values, Map<String, Object> symbols) {}

  public static Value character(String t, int c)
  {
    return new Value(t, c);
  }

  public static Value absent(String t)
  {
    return new Value(t, null);
  }

  public static Value present(String t, Value v)
  {
    return new Value(t, new Presence(v != null, v));
  }

  public static Value symbol(String id, String description, Map<String, Object> symbols)
  {
    return new Value("Symbol", symbols.computeIfAbsent(id, k -> new SymbolValue(description)));
  }

  public static Value list(String type, List<Value> values)
  {
    if (!type.startsWith("List "))
    {
      throw new IllegalArgumentException("List type required");
    }
    return new Value(type, List.copyOf(values));
  }

  private static List<Value> listElements(Value value)
  {
    if (value == null
        || !value.type.startsWith("List ")
        || !(value.data instanceof List<?> values))
    {
      throw new IllegalArgumentException("List value required");
    }
    var result = new ArrayList<Value>(values.size());
    for (Object element : values)
    {
      if (!(element instanceof Value field))
      {
        throw new IllegalArgumentException("invalid List element representation");
      }
      result.add(field);
    }
    return result;
  }

  public static Value construct(String type, String tag, Value[] fields)
  {
    if (sumType(type))
    {
      if (sumFields(type, tag).size() != fields.length)
      {
        throw new IllegalArgumentException("invalid constructor arity: " + tag);
      }
      return new Value(type, new Data(tag, Arrays.asList(fields)));
    }
    if (tag.equals("List::Nil") && fields.length == 0)
    {
      return list(type, List.of());
    }
    if (tag.equals("List::Cons") && fields.length == 2)
    {
      var values = new ArrayList<Value>();
      values.add(fields[0]);
      values.addAll(listElements(fields[1]));
      return list(type, values);
    }
    throw new IllegalArgumentException("invalid constructor or arity: " + tag);
  }

  private static boolean sumType(String type)
  {
    return type.startsWith("Maybe ") || type.startsWith("Either ");
  }

  private static List<String> eitherArguments(String type)
  {
    if (!type.startsWith("Either "))
    {
      throw new IllegalArgumentException("Either type required");
    }
    var arguments = new ArrayList<String>();
    int cursor = 7;
    while (cursor < type.length())
    {
      if (type.charAt(cursor) != '(')
      {
        throw new IllegalArgumentException("invalid Either type: " + type);
      }
      int start = ++cursor;
      int depth = 1;
      while (cursor < type.length() && depth > 0)
      {
        char unit = type.charAt(cursor++);
        if (unit == '(')
        {
          depth++;
        }
        if (unit == ')')
        {
          depth--;
        }
      }
      if (depth != 0 || cursor == start + 1)
      {
        throw new IllegalArgumentException("invalid Either type: " + type);
      }
      arguments.add(type.substring(start, cursor - 1));
      if (cursor < type.length() && type.charAt(cursor++) != ' ')
      {
        throw new IllegalArgumentException("invalid Either type: " + type);
      }
    }
    if (arguments.size() != 2)
    {
      throw new IllegalArgumentException("Either requires two type arguments");
    }
    return arguments;
  }

  private static List<String> sumFields(String type, String tag)
  {
    if (type.startsWith("Maybe "))
    {
      if (tag.equals("Maybe::Absent"))
      {
        return List.of();
      }
      if (tag.equals("Maybe::Present"))
      {
        return List.of(type.substring(6));
      }
    }
    else if (type.startsWith("Either "))
    {
      var arguments = eitherArguments(type);
      if (tag.equals("Either::Left"))
      {
        return List.of(arguments.get(0));
      }
      if (tag.equals("Either::Right"))
      {
        return List.of(arguments.get(1));
      }
    }
    throw new IllegalArgumentException("invalid constructor " + tag + " for " + type);
  }

  private static Data dataValue(Value value)
  {
    if (value == null
        || !(value.data instanceof Data data)
        || sumFields(value.type, data.tag).size() != data.fields.size())
    {
      throw new IllegalArgumentException("invalid sum value representation");
    }
    return data;
  }

  public static <T> Maybe<T> maybeToNative(
      String type, Value value, int bits, Function<Value, T> element)
  {
    if (!type.startsWith("Maybe "))
    {
      throw new IllegalArgumentException("Maybe type required");
    }
    var data = dataValue(convert(type, value, bits));
    return data.tag.equals("Maybe::Absent")
        ? new Absent<>()
        : new Present<>(element.apply(data.fields.getFirst()));
  }

  public static <L, R> Either<L, R> eitherToNative(
      String type, Value value, int bits, Function<Value, L> left, Function<Value, R> right)
  {
    if (!type.startsWith("Either "))
    {
      throw new IllegalArgumentException("Either type required");
    }
    var data = dataValue(convert(type, value, bits));
    return data.tag.equals("Either::Left")
        ? new Left<>(left.apply(data.fields.getFirst()))
        : new Right<>(right.apply(data.fields.getFirst()));
  }

  public static Value matchMaybe(
      Value value, Supplier<Value> nothing, Function<Value, Value> just)
  {
    var data = dataValue(value);
    return switch (data.tag)
    {
      case "Maybe::Absent" -> nothing.get();
      case "Maybe::Present" -> just.apply(data.fields.getFirst());
      default -> throw new IllegalArgumentException("Maybe value required");
    };
  }

  public static Value matchEither(
      Value value, Function<Value, Value> left, Function<Value, Value> right)
  {
    var data = dataValue(value);
    return switch (data.tag)
    {
      case "Either::Left" -> left.apply(data.fields.getFirst());
      case "Either::Right" -> right.apply(data.fields.getFirst());
      default -> throw new IllegalArgumentException("Either value required");
    };
  }

  public static <T> List<T> listToNative(
      String type, Value value, int bits, Function<Value, T> element)
  {
    var values = listElements(convert(type, value, bits));
    var result = new ArrayList<T>(values.size());
    for (Value field : values)
    {
      result.add(element.apply(field));
    }
    return result;
  }

  public static Value refineElements(
      Value value, Function<Value, Value> prepare, Function<Value, Value> predicate)
  {
    var accepted = new ArrayList<Value>();
    for (Value item : listElements(value))
    {
      Value prepared = prepare.apply(item);
      if (truth(predicate.apply(prepared)))
      {
        accepted.add(prepared);
      }
    }
    return list(value.type, accepted);
  }

  public static Value allElements(Value value, Function<Value, Value> predicate)
  {
    var values = listElements(value);
    for (int index = 0; index < values.size(); index++)
    {
      try
      {
        if (!truth(predicate.apply(values.get(index))))
        {
          return bool(false);
        }
      }
      catch (RuntimeException error)
      {
        throw new IllegalArgumentException(
            "List element " + index + ": " + error.getMessage(), error);
      }
    }
    return bool(true);
  }

  public static Value searchElements(
      boolean universal, Value value, Function<Value, Value> predicate)
  {
    int decisive = universal ? -1 : 1;
    int result = -decisive;
    for (var item : listElements(value))
    {
      int code = ((BigInteger) predicate.apply(item).data).intValueExact();
      if (code == decisive)
      {
        return integer("Int8", Integer.toString(code));
      }
      if (code == 0)
      {
        result = 0;
      }
      else if (code != -decisive)
      {
        throw new IllegalArgumentException("invalid finite search outcome");
      }
    }
    return integer("Int8", Integer.toString(result));
  }

  public static Value matchList(
      Value value, Supplier<Value> nil, BiFunction<Value, Value, Value> cons)
  {
    var values = listElements(value);
    if (values.isEmpty())
    {
      return nil.get();
    }
    return cons.apply(values.getFirst(), list(value.type, values.subList(1, values.size())));
  }

  private static Ratio ratio(Value v)
  {
    if (integerType(v.type))
    {
      return new Ratio((BigInteger) v.data, BigInteger.ONE);
    }
    if (v.data instanceof Ratio r)
    {
      return r;
    }
    if (v.data instanceof BigDecimal d)
    {
      int s = d.scale();
      return new Ratio(
          s < 0 ? d.unscaledValue().multiply(BigInteger.TEN.pow(-s)) : d.unscaledValue(),
          s < 0 ? BigInteger.ONE : BigInteger.TEN.pow(s));
    }
    throw new IllegalArgumentException("exact numeric value required");
  }

  private static double real(Value v)
  {
    if (v.data instanceof Double d)
    {
      return d;
    }
    Ratio r = ratio(v);
    return new BigDecimal(r.n)
        .divide(new BigDecimal(r.d), new java.math.MathContext(400, RoundingMode.HALF_EVEN))
        .doubleValue();
  }

  private static double precision(String t, double d)
  {
    return t.equals("Float32") || t.equals("Complex64") ? (double) (float) d : d;
  }

  /**
   * Moves a value into the declared type, failing instead of wrapping or rounding silently, because
   * a bounded type is only ever reached through a checked conversion.
   * ref:DEC-portable-exact-arithmetic
   */
  public static Value convert(String t, Value v, int bits)
  {
    if (sumType(t))
    {
      var data = dataValue(v);
      var types = sumFields(t, data.tag);
      var fields = new ArrayList<Value>();
      for (int i = 0; i < types.size(); i++)
      {
        fields.add(convert(types.get(i), data.fields.get(i), bits));
      }
      return new Value(t, new Data(data.tag, fields));
    }
    if (t.startsWith("List "))
    {
      var result = new ArrayList<Value>();
      for (Value field : listElements(v))
      {
        result.add(convert(t.substring(5), field, bits));
      }
      return list(t, result);
    }
    if (t.startsWith("Nullable ") || t.startsWith("Optional "))
    {
      int i = t.indexOf(' ');
      String k = t.substring(0, i);
      if (v.type.equals(k.equals("Nullable") ? "Null" : "Undefined"))
      {
        return new Value(t, new Presence(false, null));
      }
      if (v.data instanceof Presence p)
      {
        return new Value(
            t,
            new Presence(p.present, p.present ? convert(t.substring(i + 1), p.value, bits) : null));
      }
      throw new IllegalArgumentException("tagged presence required");
    }
    if (integerType(t))
    {
      Ratio r =
          v.data instanceof Double d ? ratio(new Value("Decimal", new BigDecimal(d))) : ratio(v);
      if (!r.d.equals(BigInteger.ONE))
      {
        throw new ArithmeticException("fractional conversion to " + t);
      }
      if (!t.equals("BigInt") && !t.equals("Integer"))
      {
        if (t.startsWith("U") || t.equals("BigUInt"))
        {
          if (r.n.signum() < 0)
          {
            throw new ArithmeticException("integer outside " + t + " range");
          }
        }
        if (!t.equals("BigUInt"))
        {
          int w =
              t.endsWith("Size") || t.equals("UIntPtr")
                  ? bits
                  : Integer.parseInt(t.replaceAll("\\D", ""));
          boolean signed = t.startsWith("Int");
          BigInteger hi = BigInteger.ONE.shiftLeft(signed ? w - 1 : w).subtract(BigInteger.ONE);
          BigInteger lo = signed ? BigInteger.ONE.shiftLeft(w - 1).negate() : BigInteger.ZERO;
          if (r.n.compareTo(lo) < 0 || r.n.compareTo(hi) > 0)
          {
            throw new ArithmeticException("integer outside " + t + " range");
          }
        }
      }
      return new Value(t, r.n);
    }
    if (t.equals("Rational") || t.equals("Decimal"))
    {
      Ratio r =
          v.data instanceof Double d ? ratio(new Value("Decimal", new BigDecimal(d))) : ratio(v);
      return new Value(
          t, t.equals("Rational") ? r : new BigDecimal(r.n).divide(new BigDecimal(r.d)));
    }
    if (t.startsWith("Float"))
    {
      return new Value(
          t, exactType(v.type) ? exactFloat(ratio(v), t.equals("Float32")) : precision(t, real(v)));
    }
    if (t.startsWith("Complex"))
    {
      Complex c =
          v.data instanceof Complex z
              ? z
              : new Complex(
                  (double) convert(t.equals("Complex64") ? "Float32" : "Float64", v, bits).data, 0);
      return new Value(t, new Complex(precision(t, c.real), precision(t, c.imaginary)));
    }
    if (!t.equals(v.type))
    {
      throw new IllegalArgumentException("cannot convert " + v.type + " to " + t);
    }
    return validate(t, v, bits);
  }

  private static boolean validUnit(String t, int c)
  {
    return c >= 0
        && c
            <= (t.equals("Bytes")
                ? 255
                : t.equals("Utf16Text") || t.equals("CodeUnit16") ? 65535 : 1114111)
        && (!(t.equals("Char") || t.equals("Text")) || c < 55296 || c > 57343);
  }

  /**
   * Checks a value an adapter produced against its declared domain: native code may return anything
   * its own type allows, and a law quantifies only over the declared domain.
   * ref:DEC-portable-exact-arithmetic
   */
  public static Value validate(String t, Value v, int bits)
  {
    if (v == null && t.equals("Unit"))
    {
      return absent("Unit");
    }
    if (v == null || !v.type.equals(t))
    {
      throw new IllegalArgumentException("invalid " + t + " representation");
    }
    if (sumType(t))
    {
      var data = dataValue(v);
      var types = sumFields(t, data.tag);
      var fields = new ArrayList<Value>();
      for (int i = 0; i < types.size(); i++)
      {
        fields.add(validate(types.get(i), data.fields.get(i), bits));
      }
      return new Value(t, new Data(data.tag, fields));
    }
    if (t.startsWith("List "))
    {
      var result = new ArrayList<Value>();
      for (Value field : listElements(v))
      {
        result.add(validate(t.substring(5), field, bits));
      }
      return list(t, result);
    }
    if (integerType(t))
    {
      return convert(t, v, bits);
    }
    Object data = v.data;
    boolean valid;
    if (t.startsWith("Nullable ") || t.startsWith("Optional "))
    {
      valid = data instanceof Presence;
      if (valid)
      {
        Presence p = (Presence) data;
        if (p.present)
        {
          validate(t.substring(t.indexOf(' ') + 1), p.value, bits);
        }
      }
    }
    else if (List.of("Text", "CodePointText", "Utf16Text", "Bytes").contains(t))
    {
      valid = data instanceof List<?>;
      if (valid)
      {
        for (Object x : (List<?>) data)
        {
          if (!(x instanceof Integer c) || !validUnit(t, c))
          {
            valid = false;
          }
        }
      }
    }
    else if (List.of("Char", "CodePoint", "CodeUnit16").contains(t))
    {
      valid = data instanceof Integer c && validUnit(t, c);
    }
    else if (t.equals("Bool"))
    {
      valid = data instanceof Boolean;
    }
    else if (t.equals("Decimal"))
    {
      valid = data instanceof BigDecimal;
    }
    else if (t.equals("Rational"))
    {
      valid = data instanceof Ratio;
    }
    else if (t.startsWith("Float"))
    {
      valid =
          data instanceof Double d
              && (t.equals("Float64") || Double.isNaN(d) || (double) (float) d.doubleValue() == d);
    }
    else if (t.startsWith("Complex"))
    {
      valid = data instanceof Complex;
      if (valid)
      {
        Complex c = (Complex) data;
        String component = t.equals("Complex64") ? "Float32" : "Float64";
        validate(component, new Value(component, c.real), bits);
        validate(component, new Value(component, c.imaginary), bits);
      }
    }
    else if (t.equals("Symbol"))
    {
      valid = data instanceof SymbolValue;
    }
    else
    {
      valid = List.of("Unit", "Null", "Undefined").contains(t) && data == null;
    }
    if (!valid)
    {
      throw new IllegalArgumentException("invalid " + t + " representation");
    }
    if (data instanceof List<?> xs)
    {
      return new Value(t, List.copyOf(xs));
    }
    if (data instanceof Presence p && p.present)
    {
      return new Value(
          t, new Presence(true, validate(t.substring(t.indexOf(' ') + 1), p.value, bits)));
    }
    return v;
  }

  private static String promote(String a, String b, String op)
  {
    if (exactType(a) != exactType(b))
    {
      throw new IllegalArgumentException("exact/inexact mixing requires explicit conversion");
    }
    if (exactType(a))
    {
      return op.equals("/") || a.equals("Rational") || b.equals("Rational")
          ? "Rational"
          : a.equals("Decimal") || b.equals("Decimal") ? "Decimal" : "Integer";
    }
    if (a.startsWith("Complex") || b.startsWith("Complex"))
    {
      return a.equals("Float64")
              || b.equals("Float64")
              || a.equals("Complex128")
              || b.equals("Complex128")
          ? "Complex128"
          : "Complex64";
    }
    return a.equals("Float64") || b.equals("Float64") ? "Float64" : "Float32";
  }

  private static boolean compare(String op, int c)
  {
    return switch (op)
    {
      case "==" -> c == 0;
      case "!=" -> c != 0;
      case "<" -> c < 0;
      case "<=" -> c <= 0;
      case ">" -> c > 0;
      case ">=" -> c >= 0;
      default -> throw new IllegalArgumentException(op);
    };
  }

  /**
   * Applies a LawSpec operator with LawSpec's semantics rather than Java's, so a law computes the
   * same result on every target. ref:DEC-portable-exact-arithmetic ref:ieee-754
   */
  public static Value binary(String op, Value a, Value b)
  {
    if ((op.equals("==") || op.equals("!="))
        && !exactType(a.type)
        && !a.type.startsWith("Float")
        && !a.type.startsWith("Complex"))
    {
      return bool(op.equals("==") == equal(a, b));
    }
    String t = promote(a.type, b.type, op);
    if (exactType(a.type))
    {
      Ratio x = ratio(a);
      Ratio y = ratio(b);
      BigInteger p = x.n.multiply(y.d);
      BigInteger q = y.n.multiply(x.d);
      BigInteger d = x.d.multiply(y.d);
      if (List.of("==", "!=", "<", "<=", ">", ">=").contains(op))
      {
        return bool(compare(op, p.compareTo(q)));
      }
      if (op.equals("pow"))
      {
        if (!x.d.equals(BigInteger.ONE) || !y.d.equals(BigInteger.ONE))
        {
          throw new IllegalArgumentException("integer required");
        }
        if (y.n.signum() < 0)
        {
          throw new IllegalArgumentException("negative exponent");
        }
        return new Value("Integer", x.n.pow(y.n.intValueExact()));
      }
      if (op.equals("quot") || op.equals("rem"))
      {
        if (!x.d.equals(BigInteger.ONE) || !y.d.equals(BigInteger.ONE))
        {
          throw new IllegalArgumentException("integer required");
        }
        return new Value("Integer", op.equals("quot") ? x.n.divide(y.n) : x.n.remainder(y.n));
      }
      Ratio r =
          switch (op)
          {
            case "+" -> new Ratio(p.add(q), d);
            case "-" -> new Ratio(p.subtract(q), d);
            case "*" -> new Ratio(x.n.multiply(y.n), d);
            case "/" -> new Ratio(x.n.multiply(y.d), x.d.multiply(y.n));
            default -> throw new IllegalArgumentException(op);
          };
      return convert(t, new Value("Rational", r), 64);
    }
    if (t.startsWith("Complex"))
    {
      Complex x = (Complex) convert(t, a, 64).data;
      Complex y = (Complex) convert(t, b, 64).data;
      double re;
      double im;
      if (op.equals("==") || op.equals("!="))
      {
        return bool(op.equals("==") == (x.real == y.real && x.imaginary == y.imaginary));
      }
      if (op.equals("+"))
      {
        re = x.real + y.real;
        im = x.imaginary + y.imaginary;
      }
      else if (op.equals("-"))
      {
        re = x.real - y.real;
        im = x.imaginary - y.imaginary;
      }
      else if (op.equals("*"))
      {
        re = precision(t, x.real * y.real) - precision(t, x.imaginary * y.imaginary);
        im = precision(t, x.real * y.imaginary) + precision(t, x.imaginary * y.real);
      }
      else
      {
        double d =
            precision(t, precision(t, y.real * y.real) + precision(t, y.imaginary * y.imaginary));
        re =
            precision(t, precision(t, x.real * y.real) + precision(t, x.imaginary * y.imaginary))
                / d;
        im =
            precision(t, precision(t, x.imaginary * y.real) - precision(t, x.real * y.imaginary))
                / d;
      }
      return new Value(t, new Complex(precision(t, re), precision(t, im)));
    }
    double x = real(a);
    double y = real(b);
    if (List.of("==", "!=", "<", "<=", ">", ">=").contains(op))
    {
      return bool(
          switch (op)
          {
            case "==" -> x == y;
            case "!=" -> x != y;
            case "<" -> x < y;
            case "<=" -> x <= y;
            case ">" -> x > y;
            default -> x >= y;
          });
    }
    return new Value(
        t,
        precision(
            t,
            switch (op)
            {
              case "+" -> x + y;
              case "-" -> x - y;
              case "*" -> x * y;
              case "/" -> x / y;
              default -> throw new IllegalArgumentException(op);
            }));
  }

  /**
   * Equality defined once for all targets instead of Java's equals: NaN differs from itself, signed
   * zeros are equal, handles and symbols compare by identity, and data compares field by field.
   * ref:DEC-portable-exact-arithmetic
   */
  public static boolean equal(Value a, Value b)
  {
    if (a.data instanceof Handle || b.data instanceof Handle)
    {
      return Objects.equals(a.data, b.data);
    }
    if (a.data instanceof Data left && b.data instanceof Data right)
    {
      if (!left.tag().equals(right.tag()) || left.fields().size() != right.fields().size())
      {
        return false;
      }
      if (left.tag().equals(EQ_SET_TAG))
      {
        return sameMembers(eqSetItems(left), eqSetItems(right), LawSpecRuntime::equal);
      }
      for (int index = 0; index < left.fields().size(); index++)
      {
        if (!equal(left.fields().get(index), right.fields().get(index)))
        {
          return false;
        }
      }
      return true;
    }
    if (sumType(a.type) || sumType(b.type))
    {
      if (!sumType(a.type) || !sumType(b.type))
      {
        return false;
      }
      var left = dataValue(a);
      var right = dataValue(b);
      if (!left.tag.equals(right.tag) || left.fields.size() != right.fields.size())
      {
        return false;
      }
      for (int i = 0; i < left.fields.size(); i++)
      {
        if (!equal(left.fields.get(i), right.fields.get(i)))
        {
          return false;
        }
      }
      return true;
    }
    if (a.type.startsWith("List ") && b.type.startsWith("List "))
    {
      var left = listElements(a);
      var right = listElements(b);
      if (left.size() != right.size())
      {
        return false;
      }
      for (int index = 0; index < left.size(); index++)
      {
        if (!equal(left.get(index), right.get(index)))
        {
          return false;
        }
      }
      return true;
    }
    if ((a.type.startsWith("Float") || a.type.startsWith("Complex"))
        && (b.type.startsWith("Float") || b.type.startsWith("Complex")))
    {
      return truth(binary("==", a, b));
    }
    if (exactType(a.type) && exactType(b.type))
    {
      return ratio(a).equals(ratio(b));
    }
    if (a.data instanceof Double x && b.data instanceof Double y)
    {
      return x.doubleValue() == y.doubleValue();
    }
    if (a.data instanceof Complex x && b.data instanceof Complex y)
    {
      return x.real == y.real && x.imaginary == y.imaginary;
    }
    if (a.data instanceof Presence x && b.data instanceof Presence y)
    {
      return a.type.equals(b.type)
          && x.present == y.present
          && (!x.present || equal(x.value, y.value));
    }
    if (a.type.equals("Symbol"))
    {
      return a.data == b.data;
    }
    return a.type.equals(b.type) && Objects.equals(a.data, b.data);
  }

  public static boolean truth(Value v)
  {
    return (boolean) validate("Bool", v, 64).data;
  }

  public static final String EQ_SET_TAG = "lawspec.collections::type::EqSet::EqSetItems";

  public static List<Value> eqSetItems(Data data)
  {
    if (data.fields().size() != 1)
    {
      throw new IllegalArgumentException("Eq-only set requires one List field");
    }
    return listElements(data.fields().get(0));
  }

  public static boolean sameMembers(
      List<Value> left, List<Value> right, BiPredicate<Value, Value> equal)
  {
    return left.stream()
            .allMatch(value -> right.stream().anyMatch(candidate -> equal.test(value, candidate)))
        && right.stream()
            .allMatch(value -> left.stream().anyMatch(candidate -> equal.test(value, candidate)));
  }

  public static boolean distinctMembers(List<Value> items, BiPredicate<Value, Value> equal)
  {
    for (int index = 0; index < items.size(); index++)
    {
      for (int earlier = 0; earlier < index; earlier++)
      {
        if (equal.test(items.get(index), items.get(earlier)))
        {
          return false;
        }
      }
    }
    return true;
  }

  private static final String ORDERING = "lawspec.collections::type::Ordering";

  /**
   * The portable total order: -1, 0 or 1. Exact numbers by value, sequences by unit, false before
   * true, absence before presence, lists element by element, Absent before Present, and other data
   * by constructor identity, then fields left to right.
   */
  public static int compareValues(Value a, Value b)
  {
    if (a.data instanceof Handle x && b.data instanceof Handle y)
    {
      if (x.target == y.target)
      {
        return 0;
      }
      throw new IllegalArgumentException("handles have no portable order: " + a.type);
    }
    if (a.data == null && b.data == null)
    {
      return 0;
    }
    if (a.data instanceof Boolean x && b.data instanceof Boolean y)
    {
      return Boolean.compare(x, y);
    }
    if (exactType(a.type) && exactType(b.type))
    {
      Ratio x = ratio(a);
      Ratio y = ratio(b);
      return Integer.signum(x.n().multiply(y.d()).compareTo(y.n().multiply(x.d())));
    }
    if (a.data instanceof Integer x && b.data instanceof Integer y)
    {
      return Integer.signum(Integer.compare(x, y));
    }
    if (a.data instanceof Presence x && b.data instanceof Presence y)
    {
      if (x.present() != y.present())
      {
        return x.present() ? 1 : -1;
      }
      return x.present() ? compareValues(x.value(), y.value()) : 0;
    }
    if (a.data instanceof List<?> x && b.data instanceof List<?> y)
    {
      return compareItems(x, y);
    }
    if (a.data instanceof Data x && b.data instanceof Data y)
    {
      if (!x.tag().equals(y.tag()))
      {
        if (x.tag().equals("Maybe::Absent") && y.tag().equals("Maybe::Present"))
        {
          return -1;
        }
        if (x.tag().equals("Maybe::Present") && y.tag().equals("Maybe::Absent"))
        {
          return 1;
        }
        return Integer.signum(x.tag().compareTo(y.tag()));
      }
      return compareItems(x.fields(), y.fields());
    }
    throw new IllegalArgumentException("values have no portable order: " + a.type);
  }

  private static int compareItems(List<?> x, List<?> y)
  {
    for (int i = 0; i < Math.min(x.size(), y.size()); i++)
    {
      int order =
          x.get(i) instanceof Value u
              ? compareValues(u, (Value) y.get(i))
              : Integer.signum(Integer.compare((Integer) x.get(i), (Integer) y.get(i)));
      if (order != 0)
      {
        return order;
      }
    }
    return Integer.signum(Integer.compare(x.size(), y.size()));
  }

  public static Value helper(String n, Value[] args, int bits)
  {
    if (n.equals("incompleteSearch"))
    {
      throw new IllegalStateException(codePointsText(args[0]));
    }
    Value sequenceResult = sequenceHelper(n, args);
    if (sequenceResult != null)
    {
      return sequenceResult;
    }
    switch (n)
    {
      case "startsWith":
        return bool(codePointsText(args[0]).startsWith(codePointsText(args[1])));
      case "endsWith":
        return bool(codePointsText(args[0]).endsWith(codePointsText(args[1])));
      case "textContains":
        return bool(codePointsText(args[0]).contains(codePointsText(args[1])));
      case "regexMatches":
        return bool(regexMatches(codePointsText(args[0]), codePoints(args[1])));
      case "recorded":
        return bool(recorded(codePointsText(args[0]), args[1]));
      case "acquireResource":
        return textValue(acquireResource(codePointsText(args[0])));
      case "releaseResource":
        releaseResource(codePointsText(args[0]), codePointsText(args[1]));
        return bool(true);
      case "freePort":
        return integer("Int32", Integer.toString(freePort()));
      default:
        break;
    }
    if (n.equals("checked"))
    {
      return bool(true);
    }
    if (n.equals("select"))
    {
      return truth(args[0]) ? args[1] : args[2];
    }
    if (n.equals("compare"))
    {
      var tag = new String[] {"Less", "Equal", "Greater"}[compareValues(args[0], args[1]) + 1];
      return new Value(ORDERING, new Data(ORDERING + "::" + tag, List.of()));
    }
    if (n.equals("length"))
    {
      return integer("Integer", Integer.toString(((List<?>) args[0].data).size()));
    }
    if (n.equals("isPresent"))
    {
      return bool(((Presence) args[0].data).present);
    }
    if (n.equals("presentValue"))
    {
      Presence p = (Presence) args[0].data;
      if (!p.present)
      {
        throw new IllegalArgumentException("absent presence value");
      }
      return p.value;
    }

    Value x = args[0];
    if (n.equals("real") || n.equals("imag"))
    {
      Complex c = (Complex) x.data;
      return new Value(
          x.type.equals("Complex64") ? "Float32" : "Float64",
          n.equals("real") ? c.real : c.imaginary);
    }
    if (n.equals("quot") || n.equals("rem") || n.equals("pow"))
    {
      return binary(n, x, args[1]);
    }
    if (n.equals("negate"))
    {
      if (exactType(x.type))
      {
        return binary("-", integer("BigInt", "0"), x);
      }
      if (x.data instanceof Complex c)
      {
        return new Value(x.type, new Complex(-c.real, -c.imaginary));
      }
      return new Value(x.type, -real(x));
    }
    if (n.equals("isNaN"))
    {
      return bool(Double.isNaN(real(x)));
    }
    if (n.equals("isInfinite"))
    {
      return bool(Double.isInfinite(real(x)));
    }
    if (n.equals("isFinite"))
    {
      return bool(Double.isFinite(real(x)));
    }
    if (n.equals("isNegativeZero"))
    {
      return bool(Double.doubleToRawLongBits(real(x)) == Long.MIN_VALUE);
    }
    if (n.equals("round"))
    {
      Ratio r = ratio(x);
      int scale = ((BigInteger) convert("Int32", args[1], bits).data).intValueExact();
      return new Value(
          "Decimal",
          new BigDecimal(r.n).divide(new BigDecimal(r.d), scale, RoundingMode.HALF_EVEN));
    }
    return convert(n, x, bits);
  }

  // Sequences: Text and Bytes hold their units, a List its values. Indices
  // count code points, octets or items, and a slice is clamped.
  @SuppressWarnings("unchecked")
  static Value sequenceHelper(String n, Value[] args)
  {
    switch (n)
    {
      case "append":
      {
        var items = new ArrayList<Object>((List<Object>) args[0].data);
        items.addAll((List<Object>) args[1].data);
        return new Value(args[0].type, List.copyOf(items));
      }
      case "slice":
      {
        var items = (List<Object>) args[0].data;
        long count = items.size();
        long i = Math.max(0, Math.min(count, clampedLong((BigInteger) args[1].data)));
        long j = Math.max(i, Math.min(count, clampedLong((BigInteger) args[2].data)));
        return new Value(args[0].type, List.copyOf(items.subList((int) i, (int) j)));
      }
      case "at":
      {
        var items = (List<Object>) args[0].data;
        boolean bytes = args[0].type.equals("Bytes");
        String element = bytes ? "UInt8" : args[0].type.substring("List ".length());
        String maybe = "Maybe " + element;
        BigInteger index = (BigInteger) args[1].data;
        if (index.signum() < 0 || index.compareTo(BigInteger.valueOf(items.size())) >= 0)
        {
          return new Value(maybe, new Data("Maybe::Absent", List.of()));
        }
        Object item = items.get(index.intValueExact());
        Value value = bytes ? integer("UInt8", item.toString()) : (Value) item;
        return new Value(maybe, new Data("Maybe::Present", List.of(value)));
      }
      case "range":
      {
        var values = new ArrayList<Value>();
        String t = args[0].type;
        for (BigInteger k = (BigInteger) args[0].data;
            k.compareTo((BigInteger) args[1].data) < 0;
            k = k.add(BigInteger.ONE))
        {
          values.add(new Value(t, k));
        }
        return new Value("List " + t, List.copyOf(values));
      }
      case "octets":
      {
        var values = new ArrayList<Value>();
        for (Object unit : (List<Object>) args[0].data)
        {
          values.add(integer("UInt8", unit.toString()));
        }
        return new Value("List UInt8", List.copyOf(values));
      }
      case "bytesOf":
      {
        var units = new ArrayList<Integer>();
        for (Object v : (List<Object>) args[0].data)
        {
          units.add(((BigInteger) ((Value) v).data).intValueExact());
        }
        return new Value("Bytes", List.copyOf(units));
      }
      case "utf8":
      {
        byte[] encoded =
            codePointsText(args[0]).getBytes(java.nio.charset.StandardCharsets.UTF_8);
        var units = new ArrayList<Integer>();
        for (byte b : encoded)
        {
          units.add(b & 0xff);
        }
        return new Value("Bytes", List.copyOf(units));
      }
      case "textOf":
        return textValue(((BigInteger) args[0].data).toString());
      case "join":
      {
        var parts = new ArrayList<String>();
        for (Object v : (List<Object>) args[1].data)
        {
          parts.add(codePointsText((Value) v));
        }
        return textValue(String.join(codePointsText(args[0]), parts));
      }
      default:
        return null;
    }
  }

  static long clampedLong(BigInteger n)
  {
    if (n.compareTo(BigInteger.valueOf(Long.MAX_VALUE)) > 0)
    {
      return Long.MAX_VALUE;
    }
    if (n.compareTo(BigInteger.valueOf(Long.MIN_VALUE)) < 0)
    {
      return Long.MIN_VALUE;
    }
    return n.longValue();
  }

  public static Value sample(String t, int seed, int bits)
  {
    Random random = new Random(seed);
    if (t.startsWith("Nullable ") || t.startsWith("Optional "))
    {
      int i = t.indexOf(' ');
      return new Value(
          t,
          new Presence(random.nextBoolean(), sample(t.substring(i + 1), random.nextInt(), bits)));
    }
    if (integerType(t))
    {
      if (t.equals("Integer") || t.equals("BigInt") || t.equals("BigUInt"))
      {
        BigInteger n = new BigInteger(256, random);
        return new Value(
            t,
            (t.equals("BigInt") || t.equals("Integer")) && random.nextBoolean() ? n.negate() : n);
      }
      int w =
          t.endsWith("Size") || t.equals("UIntPtr")
              ? bits
              : Integer.parseInt(t.replaceAll("\\D", ""));
      BigInteger n = new BigInteger(w, random);
      if (t.startsWith("Int") && n.testBit(w - 1))
      {
        n = n.subtract(BigInteger.ONE.shiftLeft(w));
      }
      return new Value(t, n);
    }
    if (t.equals("Bool"))
    {
      return bool(random.nextBoolean());
    }
    if (t.equals("Decimal"))
    {
      return new Value(t, new BigDecimal(new BigInteger(128, random), random.nextInt(41) - 20));
    }
    if (t.equals("Rational"))
    {
      return new Value(
          t,
          new Ratio(
              new BigInteger(128, random).subtract(BigInteger.ONE.shiftLeft(127)),
              new BigInteger(128, random).add(BigInteger.ONE)));
    }
    if (t.startsWith("Float"))
    {
      return new Value(
          t,
          t.equals("Float32")
              ? (double) Float.intBitsToFloat(random.nextInt())
              : Double.longBitsToDouble(random.nextLong()));
    }
    if (t.startsWith("Complex"))
    {
      String component = t.equals("Complex64") ? "Float32" : "Float64";
      return complex(
          t, sample(component, random.nextInt(), bits), sample(component, random.nextInt(), bits));
    }
    if (t.equals("Symbol"))
    {
      return new Value(t, new SymbolValue("same"));
    }
    if (List.of("Unit", "Null", "Undefined").contains(t))
    {
      return absent(t);
    }
    int max =
        t.equals("Bytes") ? 256 : t.equals("CodeUnit16") || t.equals("Utf16Text") ? 65536 : 1114112;
    if (List.of("Char", "CodePoint", "CodeUnit16").contains(t))
    {
      int c;
      do
      {
        c = random.nextInt(max);
      }
      while (!validUnit(t, c));
      return character(t, c);
    }
    int[] xs = new int[random.nextInt(40)];
    for (int j = 0; j < xs.length; j++)
    {
      do
      {
        xs[j] = random.nextInt(max);
      }
      while (!validUnit(t, xs[j]));
    }
    return sequence(t, xs);
  }

  public static Value unit(Runnable operation)
  {
    operation.run();
    return absent("Unit");
  }

  public static Object toNative(String t, Value v, int bits)
  {
    v = convert(t, v, bits);
    if (integerType(t))
    {
      BigInteger n = (BigInteger) v.data;
      return switch (t)
      {
        case "Int8" -> n.byteValueExact();
        case "Int16", "UInt8" -> n.shortValueExact();
        case "Int32", "UInt16" -> n.intValueExact();
        case "Int64", "UInt32" -> n.longValueExact();
        default -> n;
      };
    }
    if (t.equals("Char"))
    {
      return new String(Character.toChars((int) v.data));
    }
    if (t.equals("CodePoint"))
    {
      return v.data;
    }
    if (t.equals("CodeUnit16"))
    {
      return (char) (int) v.data;
    }
    if (t.equals("Bytes"))
    {
      List<?> xs = (List<?>) v.data;
      byte[] bytes = new byte[xs.size()];
      for (int i = 0; i < bytes.length; i++)
      {
        bytes[i] = (byte) (int) xs.get(i);
      }
      return bytes;
    }
    if (t.equals("Utf16Text"))
    {
      List<?> xs = (List<?>) v.data;
      char[] units = new char[xs.size()];
      for (int i = 0; i < units.length; i++)
      {
        units[i] = (char) (int) xs.get(i);
      }
      return new String(units);
    }
    if (t.equals("Float32"))
    {
      return ((Double) v.data).floatValue();
    }
    if (t.equals("Text"))
    {
      StringBuilder text = new StringBuilder();
      for (Object c : (List<?>) v.data)
      {
        text.appendCodePoint((int) c);
      }
      return text.toString();
    }
    return v.data;
  }

  public static Value fromNative(String t, Object value, int bits)
  {
    Value v;
    if (integerType(t)
        && !(value instanceof BigInteger
            || value instanceof Byte
            || value instanceof Short
            || value instanceof Integer
            || value instanceof Long
            || value instanceof Value))
    {
      throw new IllegalArgumentException("invalid " + t + " representation");
    }
    if (value instanceof Value scalar)
    {
      v = scalar;
    }
    else if (t.startsWith("Maybe "))
    {
      if (value instanceof Absent<?>)
      {
        v = construct(t, "Maybe::Absent", new Value[] {});
      }
      else if (value instanceof Present<?> just)
      {
        v =
            construct(
                t, "Maybe::Present", new Value[] {fromNative(t.substring(6), just.value, bits)});
      }
      else
      {
        throw new IllegalArgumentException("native Maybe requires Absent or Present");
      }
    }
    else if (t.startsWith("Either "))
    {
      var arguments = eitherArguments(t);
      if (value instanceof Left<?, ?> left)
      {
        v =
            construct(
                t, "Either::Left", new Value[] {fromNative(arguments.get(0), left.value, bits)});
      }
      else if (value instanceof Right<?, ?> right)
      {
        v =
            construct(
                t, "Either::Right", new Value[] {fromNative(arguments.get(1), right.value, bits)});
      }
      else
      {
        throw new IllegalArgumentException("native Either requires Left or Right");
      }
    }
    else if (t.startsWith("List "))
    {
      if (!(value instanceof List<?> values))
      {
        throw new IllegalArgumentException("native List requires java.util.List");
      }
      var result = new ArrayList<Value>(values.size());
      for (Object field : values)
      {
        result.add(fromNative(t.substring(5), field, bits));
      }
      v = list(t, result);
    }
    else if (integerType(t))
    {
      v =
          new Value(
              t,
              value instanceof BigInteger
                  ? value
                  : BigInteger.valueOf(((Number) value).longValue()));
    }
    else if (t.equals("Char"))
    {
      int[] xs = ((String) value).codePoints().toArray();
      if (xs.length != 1)
      {
        throw new IllegalArgumentException("Char requires one Unicode scalar");
      }
      v = character(t, xs[0]);
    }
    else if (t.equals("CodePoint"))
    {
      v = character(t, (int) value);
    }
    else if (t.equals("CodeUnit16"))
    {
      v = character(t, (char) value);
    }
    else if (t.equals("Bytes"))
    {
      byte[] xs = (byte[]) value;
      int[] units = new int[xs.length];
      for (int i = 0; i < xs.length; i++)
      {
        units[i] = Byte.toUnsignedInt(xs[i]);
      }
      v = sequence(t, units);
    }
    else if (t.equals("Utf16Text"))
    {
      v = sequence(t, ((String) value).chars().toArray());
    }
    else if (t.startsWith("Float"))
    {
      v = new Value(t, ((Number) value).doubleValue());
    }
    else if (t.equals("Text"))
    {
      v = sequence(t, ((String) value).codePoints().toArray());
    }
    else
    {
      v = new Value(t, value);
    }
    return validate(t, v, bits);
  }

  private static double exactFloat(Ratio r, boolean single)
  {
    if (r.n.signum() == 0)
    {
      return 0.0;
    }
    boolean negative = r.n.signum() < 0;
    BigInteger n = r.n.abs();
    BigInteger d = r.d;
    int p = single ? 24 : 53;
    int bias = single ? 127 : 1023;
    int emin = 1 - bias;
    int emax = bias;
    int e = n.bitLength() - d.bitLength();
    if (e >= 0 ? n.compareTo(d.shiftLeft(e)) < 0 : n.shiftLeft(-e).compareTo(d) < 0)
    {
      e--;
    }
    if (e > emax)
    {
      return negative ? Double.NEGATIVE_INFINITY : Double.POSITIVE_INFINITY;
    }
    int scale = Math.max(e, emin) - (p - 1);
    BigInteger num = scale < 0 ? n.shiftLeft(-scale) : n;
    BigInteger den = scale > 0 ? d.shiftLeft(scale) : d;
    BigInteger[] qr = num.divideAndRemainder(den);
    BigInteger q = qr[0];
    int cmp = qr[1].shiftLeft(1).compareTo(den);
    if (cmp > 0 || (cmp == 0 && q.testBit(0)))
    {
      q = q.add(BigInteger.ONE);
    }
    e = Math.max(e, emin);
    if (q.equals(BigInteger.ONE.shiftLeft(p)))
    {
      q = q.shiftRight(1);
      e++;
    }
    if (e > emax)
    {
      return negative ? Double.NEGATIVE_INFINITY : Double.POSITIVE_INFINITY;
    }
    BigInteger hidden = BigInteger.ONE.shiftLeft(p - 1);
    boolean subnormal = q.compareTo(hidden) < 0;
    long exponent = subnormal ? 0 : e + bias;
    long mantissa = (subnormal ? q : q.subtract(hidden)).longValue();
    long bits = ((negative ? 1L : 0L) << (single ? 31 : 63)) | (exponent << (p - 1)) | mantissa;
    return single ? (double) Float.intBitsToFloat((int) bits) : Double.longBitsToDouble(bits);
  }

  public record Bound(String op, Value value)
  {}

  public record Domain(
      java.util.function.BiFunction<List<Value>, Integer, List<Value>> candidates,
      java.util.function.Predicate<List<Value>> accept)
  {}

  private static BigInteger floor(Ratio r)
  {
    BigInteger[] qr = r.n.divideAndRemainder(r.d);
    return qr[1].signum() < 0 ? qr[0].subtract(BigInteger.ONE) : qr[0];
  }

  private static BigInteger ceil(Ratio r)
  {
    return floor(new Ratio(r.n.negate(), r.d)).negate();
  }

  public static List<Value> domainCandidates(
      String t, int seed, int bits, Bound[] restrictions, Value[] hints)
  {
    var values = new ArrayList<Value>();
    for (Value hint : hints)
    {
      try
      {
        values.add(convert(t, hint, bits));
      }
      catch (RuntimeException ignored)
      {
        // A hint the type does not admit is no candidate.
      }
    }
    if (integerType(t))
    {
      BigInteger lo = null;
      BigInteger hi = null;
      if (t.equals("BigUInt"))
      {
        lo = BigInteger.ZERO;
      }
      else if (!List.of("Integer", "BigInt").contains(t))
      {
        int w =
            t.endsWith("Size") || t.equals("UIntPtr")
                ? bits
                : Integer.parseInt(t.replaceAll("\\D", ""));
        boolean signed = t.startsWith("Int");
        lo = signed ? BigInteger.ONE.shiftLeft(w - 1).negate() : BigInteger.ZERO;
        hi = BigInteger.ONE.shiftLeft(signed ? w - 1 : w).subtract(BigInteger.ONE);
      }
      for (Bound b : restrictions)
      {
        Ratio r = ratio(b.value);
        if (List.of(">", ">=", "==").contains(b.op))
        {
          BigInteger v = b.op.equals(">") ? floor(r).add(BigInteger.ONE) : ceil(r);
          lo = lo == null ? v : lo.max(v);
        }
        if (List.of("<", "<=", "==").contains(b.op))
        {
          BigInteger v = b.op.equals("<") ? ceil(r).subtract(BigInteger.ONE) : floor(r);
          hi = hi == null ? v : hi.min(v);
        }
      }
      if (lo != null && hi != null && lo.compareTo(hi) > 0)
      {
        return List.of();
      }
      BigInteger lower =
          lo == null
              ? (hi == null ? BigInteger.ZERO : hi)
                  .min(BigInteger.ZERO)
                  .subtract(BigInteger.ONE.shiftLeft(256))
              : lo;
      BigInteger upper =
          hi == null
              ? (lo == null ? BigInteger.ZERO : lo)
                  .max(BigInteger.ZERO)
                  .add(BigInteger.ONE.shiftLeft(256))
              : hi;
      var ns =
          new ArrayList<BigInteger>(
              List.of(
                  lower,
                  upper,
                  BigInteger.ZERO,
                  BigInteger.ONE,
                  BigInteger.ONE.negate(),
                  lower.add(BigInteger.ONE),
                  upper.subtract(BigInteger.ONE)));
      Random random = new Random(seed);
      BigInteger width = upper.subtract(lower).add(BigInteger.ONE);
      for (int j = 0; j < 8; j++)
      {
        ns.add(lower.add(new BigInteger(width.bitLength(), random).mod(width)));
      }
      for (BigInteger n : ns)
      {
        values.add(new Value(t, n));
      }
      values.removeIf(
          v ->
              !integerType(v.type)
                  || ((BigInteger) v.data).compareTo(lower) < 0
                  || ((BigInteger) v.data).compareTo(upper) > 0);
      values.replaceAll(v -> convert(t, v, bits));
    }
    else
    {
      for (int j = 0; j < 8; j++)
      {
        values.add(sample(t, seed + j * 7919, bits));
      }
    }
    if (!values.isEmpty())
    {
      Collections.rotate(values, -Math.floorMod(seed, values.size()));
    }
    return values;
  }

  public static List<Value> generateTuple(
      Domain[] domains, int seed, int attempts, List<Value> prefix)
  {
    int[] used = {0};
    while (used[0] < attempts)
    {
      var result = searchDomain(domains, seed, attempts, used, new ArrayList<>(prefix));
      if (result != null)
      {
        return result;
      }
    }
    throw new IllegalArgumentException(
        "refinement-generation-exhausted after "
            + used[0]
            + " attempts; prefix="
            + prefix
            + "; seed="
            + seed);
  }

  private static List<Value> searchDomain(
      Domain[] domains, int seed, int attempts, int[] used, List<Value> values)
  {
    if (values.size() == domains.length)
    {
      return values;
    }
    if (used[0] >= attempts)
    {
      return null;
    }
    used[0]++;
    Domain domain = domains[values.size()];
    for (Value value : domain.candidates.apply(values, seed + used[0] * 7919))
    {
      if (used[0] >= attempts)
      {
        break;
      }
      used[0]++;
      var next = new ArrayList<>(values);
      next.add(value);
      if (domain.accept.test(next))
      {
        var result = searchDomain(domains, seed, attempts, used, next);
        if (result != null)
        {
          return result;
        }
      }
    }
    return null;
  }

  public static void requireContract(boolean condition, String context)
  {
    if (!condition)
    {
      throw new IllegalArgumentException(context);
    }
  }

  public static Value contract(String context, boolean condition, Value result)
  {
    requireContract(condition, context);
    return result;
  }

  public static void refinedCase(
      Domain[] domains,
      int seed,
      int attempts,
      int shrinks,
      java.util.function.Consumer<List<Value>> check,
      String context)
  {
    List<Value> values;
    try
    {
      values = generateTuple(domains, seed, attempts, List.of());
    }
    catch (RuntimeException error)
    {
      throw new IllegalArgumentException(context + ": " + error.getMessage(), error);
    }
    try
    {
      check.accept(values);
    }
    catch (RuntimeException | AssertionError original)
    {
      var best = values;
      int budget = shrinks;
      for (int i = 0; i < best.size(); i++)
      {
        Value value = best.get(i);
        var candidates = new ArrayList<Value>(domains[i].candidates.apply(best.subList(0, i), 0));
        if (integerType(value.type))
        {
          BigInteger initial = (BigInteger) value.data;
          candidates.add(0, new Value(value.type, BigInteger.ZERO));
          candidates.add(1, new Value(value.type, BigInteger.valueOf(initial.signum())));
          for (BigInteger v = initial.divide(BigInteger.TWO);
              v.abs().compareTo(BigInteger.ONE) > 0;
              v = v.divide(BigInteger.TWO))
          {
            candidates.add(2, new Value(value.type, v));
          }
        }
        for (Value candidate : candidates)
        {
          if (budget-- <= 0)
          {
            break;
          }
          if (complexity(candidate).compareTo(complexity(best.get(i))) >= 0)
          {
            continue;
          }
          var prefix = new ArrayList<>(best.subList(0, i));
          prefix.add(candidate);
          if (!domains[i].accept.test(prefix))
          {
            continue;
          }
          List<Value> trial;
          try
          {
            trial = generateTuple(domains, seed, Math.min(attempts, 100), prefix);
          }
          catch (IllegalArgumentException error)
          {
            if (error.getMessage().startsWith("refinement-generation-exhausted"))
            {
              continue;
            }
            throw error;
          }
          try
          {
            check.accept(trial);
          }
          catch (RuntimeException | AssertionError ignored)
          {
            best = trial;
          }
        }
      }
      throw new AssertionError(
          context
              + ": "
              + original.getMessage()
              + "; refined counterexample="
              + best
              + "; seed="
              + seed,
          original);
    }
  }

  private static BigInteger complexity(Value value)
  {
    if (integerType(value.type))
    {
      return ((BigInteger) value.data).abs();
    }
    if (value.data instanceof Presence p)
    {
      return p.present ? complexity(p.value).add(BigInteger.ONE) : BigInteger.ZERO;
    }
    if (value.data instanceof List<?> xs)
    {
      return BigInteger.valueOf(xs.size());
    }
    if (value.data instanceof Boolean b)
    {
      return b ? BigInteger.ONE : BigInteger.ZERO;
    }
    if (value.data instanceof Double d)
    {
      return BigInteger.valueOf(Double.doubleToRawLongBits(Math.abs(d)));
    }
    if (value.data instanceof Complex c)
    {
      return complexity(new Value("Float64", c.real))
          .add(complexity(new Value("Float64", c.imaginary)));
    }
    if (exactType(value.type))
    {
      Ratio r = ratio(value);
      return r.n.abs().add(r.d).subtract(BigInteger.ONE);
    }
    if (value.data instanceof Integer c)
    {
      return BigInteger.valueOf(c);
    }
    return value.data == null ? BigInteger.ZERO : BigInteger.ONE;
  }

  // Workflow runtime. A workflow runs under a runtime: a clock, a seeded
  // random source, a trace of what happened, and the state of stateful
  // stages. The runtime travels in the symbols map every generated method
  // takes; without one, the default runtime applies (real time, unless the
  // tests installed a virtual clock). Durations are long microseconds.

  private static final String WORKFLOW = "\0lawspec.workflow";

  /** Tells the time and waits, in microseconds. */
  public interface Clock
  {
    long now();

    void sleep(long micros);

    /**
     * Whether this clock is not real time: waits pass at once, and timeouts and hedges count only
     * the time it reports.
     */
    default boolean virtual()
    {
      return false;
    }
  }

  /** Monotonic wall time. */
  public static final class RealClock implements Clock
  {
    public long now()
    {
      return System.nanoTime() / 1000;
    }

    public void sleep(long micros)
    {
      try
      {
        Thread.sleep(micros / 1000, (int) (micros % 1000) * 1000);
      }
      catch (InterruptedException error)
      {
        Thread.currentThread().interrupt();
        throw new IllegalStateException("workflow interrupted while waiting", error);
      }
    }
  }

  /** Advances when slept on and returns at once. */
  public static final class VirtualClock implements Clock
  {
    public long time;

    public long now()
    {
      return time;
    }

    public void sleep(long micros)
    {
      time += micros;
    }

    @Override
    public boolean virtual()
    {
      return true;
    }
  }

  /** The Clock ability's key in the handlers a law installs. */
  public static final String CLOCK_ABILITY = "lawspec.time::ability::Clock";

  /** How the runtime reads a Clock handler (lawspec.time's registerClock gives it). */
  private record ClockReader(
      java.util.function.ToLongFunction<Object> now,
      java.util.function.ObjLongConsumer<Object> sleep,
      java.util.function.Predicate<Object> realTime)
  {}

  private static volatile ClockReader clockReader;

  /**
   * How the runtime reads a Clock handler, registered by lawspec.time's registerClock (the
   * generated tests call it): now(handler) gives microseconds, sleep(handler, micros) waits, and
   * realTime(handler) says whether the handler is the default real clock.
   */
  public static void registerClockAbility(
      java.util.function.ToLongFunction<Object> now,
      java.util.function.ObjLongConsumer<Object> sleep,
      java.util.function.Predicate<Object> realTime)
  {
    clockReader = new ClockReader(now, sleep, realTime);
  }

  /** handler as a clock, or null when there is none or no reader is registered. */
  private static AbilityClock abilityClock(Object handler)
  {
    var reader = clockReader;
    return reader == null || handler == null ? null : new AbilityClock(handler, reader);
  }

  /**
   * A workflow runtime's clock read through the Clock ability: the handler a law installs (the
   * virtual clock, or the default real one). Every handler but the default real clock is virtual:
   * waits pass at once, and timeouts and hedges count only the time it reports.
   */
  public static final class AbilityClock implements Clock
  {
    public final Object handler;
    private final ClockReader reader;
    private final boolean virtual;

    private AbilityClock(Object handler, ClockReader reader)
    {
      this.handler = handler;
      this.reader = reader;
      this.virtual = !reader.realTime().test(handler);
    }

    @Override
    public boolean virtual()
    {
      return virtual;
    }

    public long now()
    {
      return reader.now().applyAsLong(handler);
    }

    public void sleep(long micros)
    {
      reader.sleep().accept(handler, micros);
    }
  }

  /**
   * The same sequence on every target for the same seed.
   *
   * <p>SplitMix64 is small, fast and specified exactly, so every runtime implements the same
   * generator and a seed names the same case everywhere. ref:splitmix
   * ref:DEC-portable-seeded-generation
   */
  public static final class SplitMix64
  {
    private long state;

    public SplitMix64(long seed)
    {
      state = seed;
    }

    public long next()
    {
      state += 0x9E3779B97F4A7C15L;
      long z = state;
      z = (z ^ (z >>> 30)) * 0xBF58476D1CE4E5B9L;
      z = (z ^ (z >>> 27)) * 0x94D049BB133111EBL;
      return z ^ (z >>> 31);
    }

    /** Uniform in [0, bound); 0 when bound is 0. */
    public long below(long bound)
    {
      return bound <= 0 ? 0 : Long.remainderUnsigned(next(), bound);
    }

    /** Uniform in [0, bound) for any bound up to 2^64; 0 when bound is 0. */
    public BigInteger below(BigInteger bound)
    {
      if (bound.signum() <= 0)
      {
        return BigInteger.ZERO;
      }
      return new BigInteger(Long.toUnsignedString(next())).mod(bound);
    }
  }

  /** A stage starting or finishing an attempt, or a wait. */
  public record TraceEvent(String kind, String stage, long number, boolean succeeded)
  {}

  /**
   * gates: whether rate limits, breakers, bulkheads and caches apply. The runtime generated tests
   * install has them off: a workflow law calls the workflow and its composition, which would see
   * each other's state.
   */
  public static final class WorkflowRuntime
  {
    public final Clock clock;
    public final SplitMix64 random;
    public final List<TraceEvent> trace;
    public final Map<String, Object> state;
    public boolean gates = true;
    // A frame per running workflow: the undos of its completed stages.
    private final List<List<Map.Entry<String, Runnable>>> frames;
    // When the running attempt of a stage with a timeout must end
    // (System.nanoTime), or null.
    private Long deadline;
    // The running attempt's hedge, or null.
    private Hedge hedge;
    // The running attempt's hedge on a virtual clock, or null: attempts run
    // one after another (virtualHedge).
    private Hedge virtualHedge;

    public WorkflowRuntime(Clock clock, long seed)
    {
      this.clock = clock == null ? new RealClock() : clock;
      this.random = new SplitMix64(seed);
      this.trace = new ArrayList<>();
      this.state = new java.util.HashMap<>();
      this.frames = new ArrayList<>();
    }

    /** base on another clock: the same trace, state, random and frames. */
    private WorkflowRuntime(WorkflowRuntime base, Clock clock)
    {
      this.clock = clock;
      this.random = base.random;
      this.trace = base.trace;
      this.state = base.state;
      this.frames = base.frames;
      this.gates = base.gates;
      this.deadline = base.deadline;
      this.hedge = base.hedge;
      this.virtualHedge = base.virtualHedge;
    }

    /** A symbols map that runs workflows under this runtime. */
    public Map<String, Object> context(Map<String, Object> symbols)
    {
      symbols.put(WORKFLOW, this);
      return symbols;
    }
  }

  private static WorkflowRuntime defaultRuntime;

  private static final String CLOCK_VIEW = "\0lawspec.workflow.clock";

  /** The default runtime read through a context's Clock handler. */
  private record ClockView(Object handler, WorkflowRuntime base, WorkflowRuntime view)
  {}

  /**
   * Makes the default runtime virtual, as generated tests do. Timeouts and hedges stay on: they
   * count virtual time (see scoped).
   */
  public static synchronized void useVirtualClock(long seed)
  {
    defaultRuntime = new WorkflowRuntime(new VirtualClock(), seed);
    defaultRuntime.gates = false;
  }

  /**
   * The runtime a context's workflows run under: the one attached to it (context), else the default
   * runtime. Workflow time is the Clock ability's: where a law has installed a Clock handler, the
   * default runtime waits and times out on it (the same view each time for the same context and
   * handler).
   */
  public static synchronized WorkflowRuntime workflowRuntime(Map<String, Object> symbols)
  {
    if (symbols.get(WORKFLOW) instanceof WorkflowRuntime runtime)
    {
      return runtime;
    }
    if (defaultRuntime == null)
    {
      defaultRuntime = new WorkflowRuntime(null, 0);
    }
    WorkflowRuntime runtime = defaultRuntime;
    var clock =
        abilityClock(
            symbols.get(HANDLERS) instanceof Map<?, ?> table ? table.get(CLOCK_ABILITY) : null);
    if (clock == null)
    {
      return runtime;
    }
    if (symbols.get(CLOCK_VIEW) instanceof ClockView view
        && view.handler() == clock.handler
        && view.base() == runtime)
    {
      return view.view();
    }
    var view = new ClockView(clock.handler, runtime, new WorkflowRuntime(runtime, clock));
    symbols.put(CLOCK_VIEW, view);
    return view.view();
  }

  /** What a custom strategy decides: a delay, or null to stop. */
  public interface Decide
  {
    Long decide(long attempt, Value failure, long previous);
  }

  /**
   * strategy is immediate, fixed, linear, exponential, fibonacci or custom; delay, step, factor and
   * cap (negative for none) are its parameters.
   */
  public record Retry(
      String strategy,
      long delay,
      long step,
      long factor,
      long cap,
      long attempts,
      String jitter,
      Function<Value, Boolean> when,
      Decide decide)
  {}

  /**
   * A stateful policy: start gives its state, admit a Step of the next state and a Gate (Admit,
   * WaitFor or Reject), and finish (when set) the state after the call. waitMicros is -2 to fail at
   * once when not admitted, -1 to wait without bound, or the most it waits.
   */
  public record Gate(
      String kind,
      java.util.function.LongFunction<Value> start,
      java.util.function.BiFunction<Value, Long, Value> admit,
      GateFinish finish,
      long waitMicros)
  {}

  public interface GateFinish
  {
    Value finish(Value state, long now, boolean succeeded);
  }

  /**
   * key names the stage's state; cache is how long a success is reused (0 or less for none); wraps
   * says failures are StageFailures; fail gives the stage's result for a policy failure.
   */
  public record StagePolicy(
      String stage,
      Retry retry,
      long timeout,
      String key,
      List<Gate> gates,
      long cache,
      boolean wraps,
      Function<String, Value> fail,
      java.util.function.Consumer<Value> compensate,
      Hedge hedge)
  {
    public StagePolicy(String stage, Retry retry, long timeout)
    {
      this(stage, retry, timeout, stage, List.of(), -1, false, null, null, null);
    }
  }

  /**
   * When an attempt has not succeeded after delay microseconds, another starts beside it, up to
   * most in all; the first success wins.
   */
  public record Hedge(String stage, long delay, long most)
  {}

  /**
   * Runs a workflow whose stages compensate: when it fails, its completed stages' undos run, last
   * first.
   */
  public static Value runWorkflow(
      Map<String, Object> symbols, java.util.function.Supplier<Value> attempt)
  {
    WorkflowRuntime runtime = workflowRuntime(symbols);
    var frame = new ArrayList<Map.Entry<String, Runnable>>();
    runtime.frames.add(frame);
    Value result;
    try
    {
      result = attempt.get();
    }
    finally
    {
      runtime.frames.remove(runtime.frames.size() - 1);
    }
    if (result.data() instanceof Data data && data.tag().equals("Either::Left"))
    {
      for (int i = frame.size() - 1; i >= 0; i--)
      {
        runtime.trace.add(new TraceEvent("compensate", frame.get(i).getKey(), 0, true));
        frame.get(i).getValue().run();
      }
    }
    return result;
  }

  private record CacheEntry(Value key, Value value, long expires)
  {}

  private static final String STAGE_FAILURE = "lawspec.resilience::type::StageFailure::";
  private static final String GATE = "lawspec.resilience::type::Gate::";

  private static String passGate(WorkflowRuntime runtime, StagePolicy policy, Gate gate)
  {
    String key = policy.key() + "/" + gate.kind();
    String failure =
        switch (gate.kind())
        {
          case "breaker" -> "CircuitOpen";
          case "limit" -> "RateLimited";
          default -> "Saturated";
        };
    long waited = 0;
    while (true)
    {
      long now = runtime.clock.now();
      Value state =
          runtime.state.containsKey(key) ? (Value) runtime.state.get(key) : gate.start().apply(now);
      Data step = (Data) gate.admit().apply(state, now).data();
      runtime.state.put(key, step.fields().get(0));
      Data decision = (Data) step.fields().get(1).data();
      if (decision.tag().equals(GATE + "Admit"))
      {
        return null;
      }
      if (decision.tag().equals(GATE + "Reject") || gate.waitMicros() == -2)
      {
        return failure;
      }
      long delay = ((BigInteger) decision.fields().get(0).data()).longValueExact();
      if (gate.waitMicros() >= 0 && waited + delay > gate.waitMicros())
      {
        return failure;
      }
      runtime.trace.add(new TraceEvent("wait", policy.stage(), delay, true));
      runtime.clock.sleep(delay);
      waited += delay;
    }
  }

  private static void finishGate(
      WorkflowRuntime runtime, StagePolicy policy, Gate gate, boolean succeeded)
  {
    if (gate.finish() == null)
    {
      return;
    }
    String key = policy.key() + "/" + gate.kind();
    runtime.state.put(
        key, gate.finish().finish((Value) runtime.state.get(key), runtime.clock.now(), succeeded));
  }

  private static long fibonacci(long n)
  {
    long a = 1;
    long b = 1;
    for (long i = 1; i < n; i++)
    {
      long next = a + b;
      a = b;
      b = next;
    }
    return a;
  }

  /** The delay before attempt (2 or more), before jitter. */
  public static long retryDelay(Retry retry, long attempt)
  {
    long n = attempt - 1;
    switch (retry.strategy())
    {
      case "immediate":
        return 0;
      case "fixed":
        return retry.delay();
      case "linear":
        return retry.delay() + retry.step() * (n - 1);
      case "exponential":
      {
        long delay = retry.delay();
        for (long i = 1; i < n; i++)
        {
          delay *= retry.factor();
          if (retry.cap() >= 0 && delay >= retry.cap())
          {
            return retry.cap();
          }
        }
        return retry.cap() >= 0 && delay > retry.cap() ? retry.cap() : delay;
      }
      case "fibonacci":
        return retry.delay() * fibonacci(n);
      default:
        throw new IllegalArgumentException("unknown retry strategy: " + retry.strategy());
    }
  }

  /**
   * Full: [0, delay]; equal: delay/2 + [0, delay/2]; decorrelated: [base, previous * 3], capped at
   * delay.
   */
  public static long jittered(
      String jitter, long delay, long previous, long base, SplitMix64 random)
  {
    switch (jitter)
    {
      case "full":
        return random.below(delay + 1);
      case "equal":
      {
        long half = delay / 2;
        return half + random.below(delay - half + 1);
      }
      case "decorrelated":
      {
        long high = Math.max(base, previous * 3);
        return Math.min(delay, base + random.below(high - base + 1));
      }
      default:
        return delay;
    }
  }

  /** Runs a stage's attempts under its policy; a Left is a failure. */
  public static Value runStage(
      Map<String, Object> symbols, StagePolicy policy, java.util.function.Supplier<Value> attempt)
  {
    return runStage(symbols, policy, attempt, null);
  }

  /** As runStage; key is the stage's input, for the cache. */
  @SuppressWarnings("unchecked")
  public static Value runStage(
      Map<String, Object> symbols,
      StagePolicy policy,
      java.util.function.Supplier<Value> attempt,
      Value key)
  {
    WorkflowRuntime runtime = workflowRuntime(symbols);
    List<Gate> gates = runtime.gates ? policy.gates() : List.of();
    String cacheKey = policy.key() + "/cache";
    boolean caching = policy.cache() > 0 && runtime.gates;
    if (caching)
    {
      long now = runtime.clock.now();
      for (var entry : (List<CacheEntry>) runtime.state.getOrDefault(cacheKey, List.of()))
      {
        if (now < entry.expires() && compareValues(entry.key(), key) == 0)
        {
          runtime.trace.add(new TraceEvent("cached", policy.stage(), 0, true));
          return entry.value();
        }
      }
    }
    for (int i = 0; i < gates.size(); i++)
    {
      String failure = passGate(runtime, policy, gates.get(i));
      if (failure != null)
      {
        for (var passed : gates.subList(0, i))
        {
          finishGate(runtime, policy, passed, false);
        }
        return policy.fail().apply(failure);
      }
    }
    Value result = attempts(runtime, policy, attempt);
    boolean succeeded = !(result.data() instanceof Data data && data.tag().equals("Either::Left"));
    for (var gate : gates)
    {
      finishGate(runtime, policy, gate, succeeded);
    }
    if (succeeded && policy.compensate() != null && !runtime.frames.isEmpty())
    {
      Value value = ((Data) result.data()).fields().get(0);
      runtime
          .frames
          .get(runtime.frames.size() - 1)
          .add(Map.entry(policy.stage(), () -> policy.compensate().accept(value)));
    }
    if (caching && succeeded)
    {
      var entries = new ArrayList<CacheEntry>();
      for (var entry : (List<CacheEntry>) runtime.state.getOrDefault(cacheKey, List.of()))
      {
        if (compareValues(entry.key(), key) != 0)
        {
          entries.add(entry);
        }
      }
      entries.add(new CacheEntry(key, result, runtime.clock.now() + policy.cache()));
      runtime.state.put(cacheKey, entries);
    }
    return result;
  }

  /**
   * The Async ability's default handler: the JVM's virtual threads. LawSpec code performs pause;
   * workflows reach the rest natively: spawn starts a function as a task, await gives a task's
   * result, and all runs functions side by side and gives their results in order. An async
   * adapter's CompletableFuture is awaited with await. lawspec.concurrent's default Async handler
   * extends it.
   */
  public static class NativeAsync
  {
    /** Lets other threads run. */
    public void pause()
    {
      Thread.yield();
    }

    /** Starts fn on a virtual thread of its own: its result, or its exception, when done. */
    public <T> java.util.concurrent.CompletableFuture<T> spawn(Supplier<? extends T> fn)
    {
      var task = new java.util.concurrent.CompletableFuture<T>();
      Thread.ofVirtual()
          .start(
              () -> {
                try
                {
                  task.complete(fn.get());
                }
                catch (Throwable error)
                {
                  // Given back by await.
                  task.completeExceptionally(error);
                }
              });
      return task;
    }

    /**
     * A task's result, waiting for it (as CompletableFuture.join: its exception is thrown wrapped
     * in a CompletionException).
     */
    public <T> T await(java.util.concurrent.Future<T> task)
    {
      if (task instanceof java.util.concurrent.CompletableFuture<T> future)
      {
        return future.join();
      }
      try
      {
        return task.get();
      }
      catch (java.util.concurrent.ExecutionException e)
      {
        throw new java.util.concurrent.CompletionException(e.getCause());
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        throw new java.util.concurrent.CompletionException(e);
      }
    }

    /**
     * Every function's result, in order; all finish before the first exception (in order) is
     * thrown, unwrapped.
     */
    public <T> List<T> all(List<? extends Supplier<? extends T>> fns)
    {
      var tasks = new ArrayList<java.util.concurrent.CompletableFuture<T>>();
      for (var fn : fns)
      {
        tasks.add(spawn(fn));
      }
      try
      {
        java.util.concurrent.CompletableFuture.allOf(
                tasks.toArray(new java.util.concurrent.CompletableFuture<?>[0]))
            .join();
      }
      catch (java.util.concurrent.CompletionException e)
      {
        // A failed task is raised below, in order.
      }
      var results = new ArrayList<T>();
      for (var task : tasks)
      {
        try
        {
          results.add(task.join());
        }
        catch (java.util.concurrent.CompletionException e)
        {
          if (e.getCause() instanceof RuntimeException failure)
          {
            throw failure;
          }
          if (e.getCause() instanceof Error failure)
          {
            throw failure;
          }
          throw e;
        }
      }
      return results;
    }
  }

  /** The runtime's native Async. */
  public static final NativeAsync ASYNC = new NativeAsync();

  /**
   * An all group's steps, run side by side as tasks of the Async ability's default handler (virtual
   * threads); body receives their results in declaration order. Every step finishes before a step's
   * exception (the first, in declaration order) is thrown.
   */
  @SafeVarargs
  public static Value concurrently(Function<List<Value>, Value> body, Supplier<Value>... steps)
  {
    return body.apply(ASYNC.all(Arrays.asList(steps)));
  }

  /** Raised by awaitWithin when an attempt outlives its stage's timeout. */
  private static final class TimedOut extends RuntimeException
  {
    TimedOut()
    {
      super("timed out", null, false, false);
    }
  }

  /**
   * An asynchronous step's logical result: start begins the step and convert turns its native
   * result into a logical value. The step runs within its stage's timeout and hedge, if any.
   */
  public static <T> Value awaitStep(
      Map<String, Object> symbols,
      java.util.function.Supplier<java.util.concurrent.CompletableFuture<T>> start,
      Function<T, Value> convert)
  {
    WorkflowRuntime runtime = workflowRuntime(symbols);
    Long deadline = runtime.deadline;
    Hedge hedge = runtime.hedge;
    if (runtime.virtualHedge != null)
    {
      return virtualHedge(runtime, runtime.virtualHedge, start, convert);
    }
    if (deadline == null && hedge == null)
    {
      return convert.apply(ASYNC.await(start.get()));
    }
    long most = hedge == null ? 1 : hedge.most();
    var pending = new ArrayList<java.util.concurrent.CompletableFuture<T>>();
    long started = 0;
    long next = Long.MAX_VALUE;
    boolean launch = true;
    try
    {
      while (true)
      {
        if (launch)
        {
          started++;
          if (started > 1)
          {
            runtime.trace.add(new TraceEvent("hedge", hedge.stage(), started, true));
          }
          pending.add(start.get());
          next = started < most ? System.nanoTime() + hedge.delay() * 1000 : Long.MAX_VALUE;
          launch = false;
        }
        long until = Math.min(next, deadline == null ? Long.MAX_VALUE : deadline);
        var any =
            java.util.concurrent.CompletableFuture.anyOf(
                pending.toArray(new java.util.concurrent.CompletableFuture<?>[0]));
        try
        {
          if (until == Long.MAX_VALUE)
          {
            any.get();
          }
          else
          {
            any.get(
                Math.max(0, until - System.nanoTime()), java.util.concurrent.TimeUnit.NANOSECONDS);
          }
        }
        catch (java.util.concurrent.TimeoutException
            | java.util.concurrent.ExecutionException e)
        {
          // A failed step is raised below; a timer passing is handled below.
        }
        Value last = null;
        for (var task : List.copyOf(pending))
        {
          if (!task.isDone())
          {
            continue;
          }
          pending.remove(task);
          Value value = convert.apply(task.join());
          if (!(value.data() instanceof Data data && data.tag().equals("Either::Left")))
          {
            return value;
          }
          last = value;
        }
        long now = System.nanoTime();
        if (deadline != null && now >= deadline)
        {
          throw new TimedOut();
        }
        if (pending.isEmpty())
        {
          if (started >= most)
          {
            return last;
          }
          launch = true;
        }
        else if (now >= next)
        {
          launch = true;
        }
      }
    }
    catch (InterruptedException e)
    {
      Thread.currentThread().interrupt();
      throw new java.util.concurrent.CompletionException(e);
    }
    finally
    {
      for (var task : pending)
      {
        task.cancel(true);
      }
    }
  }

  private static boolean isLeft(Value value)
  {
    return value.data() instanceof Data data && data.tag().equals("Either::Left");
  }

  /**
   * A hedge on a virtual clock: attempts run one after another, and the next starts when one fails,
   * so the first success wins as it would in real time when no attempt outlives the delay.
   */
  private static <T> Value virtualHedge(
      WorkflowRuntime runtime,
      Hedge hedge,
      java.util.function.Supplier<java.util.concurrent.CompletableFuture<T>> start,
      Function<T, Value> convert)
  {
    long started = 1;
    Value value = convert.apply(ASYNC.await(start.get()));
    while (isLeft(value) && started < hedge.most())
    {
      started++;
      runtime.trace.add(new TraceEvent("hedge", hedge.stage(), started, true));
      value = convert.apply(ASYNC.await(start.get()));
    }
    return value;
  }

  private static Value timedOut(StagePolicy policy)
  {
    return policy.fail() != null
        ? policy.fail().apply("TimedOut")
        : new Value(
            "Either",
            new Data(
                "Either::Left",
                List.of(
                    new Value(STAGE_FAILURE, new Data(STAGE_FAILURE + "TimedOut", List.of())))));
  }

  private static boolean timedOutCause(Throwable error)
  {
    for (Throwable cause = error; cause != null; cause = cause.getCause())
    {
      if (cause instanceof TimedOut)
      {
        return true;
      }
    }
    return false;
  }

  /**
   * An attempt under its stage's timeout (failing with TimedOut when it outlives it) and hedge: the
   * Timeout and Hedge transformers of the Async ability, measured on the runtime's Clock. On a
   * virtual clock (generated tests, or a law using a virtual clock) an attempt takes the virtual
   * time that passes while it runs, so both are deterministic. On a real clock with gates off, both
   * are off.
   */
  private static Value scoped(
      WorkflowRuntime runtime, StagePolicy policy, java.util.function.Supplier<Value> attempt)
  {
    if (policy.timeout() <= 0 && policy.hedge() == null)
    {
      return attempt.get();
    }
    if (runtime.clock.virtual())
    {
      return virtuallyScoped(runtime, policy, attempt);
    }
    if (!runtime.gates)
    {
      return attempt.get();
    }
    Long outer = runtime.deadline;
    Hedge outerHedge = runtime.hedge;
    if (policy.timeout() > 0)
    {
      runtime.deadline = System.nanoTime() + policy.timeout() * 1000;
    }
    if (policy.hedge() != null)
    {
      runtime.hedge = new Hedge(policy.stage(), policy.hedge().delay(), policy.hedge().most());
    }
    try
    {
      return attempt.get();
    }
    catch (RuntimeException e)
    {
      // Callers may have wrapped the timeout with context.
      if (timedOutCause(e))
      {
        return timedOut(policy);
      }
      throw e;
    }
    finally
    {
      runtime.deadline = outer;
      runtime.hedge = outerHedge;
    }
  }

  private static Value virtuallyScoped(
      WorkflowRuntime runtime, StagePolicy policy, java.util.function.Supplier<Value> attempt)
  {
    Long outer = runtime.deadline;
    Hedge outerHedge = runtime.hedge;
    Hedge outerVirtual = runtime.virtualHedge;
    long began = runtime.clock.now();
    runtime.deadline = null;
    runtime.hedge = null;
    runtime.virtualHedge =
        policy.hedge() == null
            ? null
            : new Hedge(policy.stage(), policy.hedge().delay(), policy.hedge().most());
    Value result;
    try
    {
      result = attempt.get();
    }
    catch (RuntimeException e)
    {
      if (timedOutCause(e))
      {
        return timedOut(policy);
      }
      throw e;
    }
    finally
    {
      runtime.deadline = outer;
      runtime.hedge = outerHedge;
      runtime.virtualHedge = outerVirtual;
    }
    if (policy.timeout() > 0 && runtime.clock.now() - began > policy.timeout())
    {
      return timedOut(policy);
    }
    return result;
  }

  private static Value attempts(
      WorkflowRuntime runtime, StagePolicy policy, java.util.function.Supplier<Value> attempt)
  {
    Retry retry = policy.retry();
    long number = 1;
    long previous = 0;
    while (true)
    {
      runtime.trace.add(new TraceEvent("start", policy.stage(), number, false));
      Value result = scoped(runtime, policy, attempt);
      boolean failed = result.data() instanceof Data data && data.tag().equals("Either::Left");
      runtime.trace.add(new TraceEvent("finish", policy.stage(), number, !failed));
      if (!failed || retry == null)
      {
        return result;
      }
      if (retry.attempts() > 0 && number >= retry.attempts())
      {
        return result;
      }
      Value failure = ((Data) result.data()).fields().get(0);
      if (policy.wraps())
      {
        // Only the step's own failures and timeouts are retried.
        Data wrapped = (Data) failure.data();
        if (wrapped.tag().equals(STAGE_FAILURE + "StepFailed"))
        {
          failure = wrapped.fields().get(0);
        }
        else if (!wrapped.tag().equals(STAGE_FAILURE + "TimedOut"))
        {
          return result;
        }
      }
      if (retry.when() != null && !retry.when().apply(failure))
      {
        return result;
      }
      number++;
      long delay;
      if (retry.strategy().equals("custom"))
      {
        Long wait = retry.decide().decide(number, failure, previous);
        if (wait == null)
        {
          return result;
        }
        delay = wait;
      }
      else
      {
        long base = retry.strategy().equals("immediate") ? 0 : retryDelay(retry, 2);
        delay = jittered(retry.jitter(), retryDelay(retry, number), previous, base, runtime.random);
      }
      runtime.trace.add(new TraceEvent("sleep", policy.stage(), delay, true));
      runtime.clock.sleep(delay);
      previous = delay;
    }
  }

  /** A logical Duration of whole microseconds. */
  public static Value duration(long micros)
  {
    return new Value(
        "lawspec.time::type::Duration",
        new Data(
            "lawspec.time::type::Duration::Duration",
            List.of(new Value("Integer", BigInteger.valueOf(micros)))));
  }

  /** A RetryDecision's delay, or null to stop. */
  public static Long retryDecision(Value decision)
  {
    Data data = (Data) decision.data();
    if (!data.tag().equals("lawspec.time::type::RetryDecision::RetryAfter"))
    {
      return null;
    }
    Data delay = (Data) data.fields().get(0).data();
    return ((BigInteger) delay.fields().get(0).data()).longValueExact();
  }

  /** A logical Integer. */
  public static Value integer64(long n)
  {
    return new Value("Integer", BigInteger.valueOf(n));
  }

  // Portable generation for stateful models. A type descriptor is an
  // s-expression: (int T lo hi) with _ for no bound, (bool), (text), (unit),
  // (list D), (maybe D), (either L R), (data NAME (ctor TAG D...) ...) and
  // (ref NAME) for a data type declared in the model's table. Every target
  // generates, shrinks and renders the same values for the same seed.

  /** A string atom of a descriptor. */
  public record Quoted(String text)
  {}

  /**
   * Parses s-expressions: lists (List), integers (BigInteger), strings (Quoted), symbols (String)
   * and _ (null).
   */
  public static List<Object> readDescriptor(String text)
  {
    var position = new int[] {0};
    var items = new ArrayList<Object>();
    skipBlank(text, position);
    while (position[0] < text.length())
    {
      items.add(descriptorItem(text, position));
      skipBlank(text, position);
    }
    return items;
  }

  private static void skipBlank(String text, int[] position)
  {
    while (position[0] < text.length() && " \t\r\n".indexOf(text.charAt(position[0])) >= 0)
    {
      position[0]++;
    }
  }

  private static Object descriptorItem(String text, int[] position)
  {
    skipBlank(text, position);
    char c = text.charAt(position[0]);
    if (c == '(')
    {
      position[0]++;
      var items = new ArrayList<Object>();
      skipBlank(text, position);
      while (text.charAt(position[0]) != ')')
      {
        items.add(descriptorItem(text, position));
        skipBlank(text, position);
      }
      position[0]++;
      return items;
    }
    if (c == '"')
    {
      position[0]++;
      var out = new StringBuilder();
      while (text.charAt(position[0]) != '"')
      {
        if (text.charAt(position[0]) == '\\')
        {
          position[0]++;
        }
        out.append(text.charAt(position[0]));
        position[0]++;
      }
      position[0]++;
      return new Quoted(out.toString());
    }
    int start = position[0];
    while (position[0] < text.length() && " \t\r\n()".indexOf(text.charAt(position[0])) < 0)
    {
      position[0]++;
    }
    String atom = text.substring(start, position[0]);
    if (atom.equals("_"))
    {
      return null;
    }
    String digits = atom.replaceFirst("^-+", "");
    if (!digits.isEmpty() && digits.chars().allMatch(d -> d >= '0' && d <= '9'))
    {
      return new BigInteger(atom);
    }
    return atom;
  }

  /** An atom's text: a symbol, a string or an integer. */
  private static String atomText(Object atom)
  {
    return atom instanceof Quoted q ? q.text() : String.valueOf(atom);
  }

  private static boolean sameAtom(Object a, Object b)
  {
    boolean textA = a instanceof String || a instanceof Quoted;
    boolean textB = b instanceof String || b instanceof Quoted;
    if (textA || textB)
    {
      return textA && textB && atomText(a).equals(atomText(b));
    }
    return Objects.equals(a, b);
  }

  @SuppressWarnings("unchecked")
  private static List<Object> form(Object d)
  {
    return (List<Object>) d;
  }

  private static final BigInteger UNBOUNDED = BigInteger.valueOf(1_000_000);

  /**
   * Generation, shrinking and rendering over a table of data types.
   *
   * <p>Shrinking stays inside the declared domain, so a reported counterexample is always a value
   * the law quantifies over. ref:DEC-shrink-within-domain ref:DEC-structural-size-budget
   */
  public static final class Values
  {
    public final Map<String, List<Object>> table;

    public Values(Map<String, List<Object>> table)
    {
      this.table = table;
    }

    public List<Object> resolve(Object d)
    {
      var f = form(d);
      return atomText(f.get(0)).equals("ref") ? table.get(atomText(f.get(1))) : f;
    }

    /**
     * An integer's range: a missing bound is 1,000,000 from zero, or 2,000,000 from the other bound
     * when that is beyond it.
     */
    public BigInteger[] bounds(List<Object> d)
    {
      var lo = (BigInteger) d.get(2);
      var hi = (BigInteger) d.get(3);
      var twice = UNBOUNDED.shiftLeft(1);
      if (lo == null && hi == null)
      {
        return new BigInteger[] {UNBOUNDED.negate(), UNBOUNDED};
      }
      if (lo == null)
      {
        return new BigInteger[] {UNBOUNDED.negate().min(hi.subtract(twice)), hi};
      }
      if (hi == null)
      {
        return new BigInteger[] {lo, UNBOUNDED.max(lo.add(twice))};
      }
      return new BigInteger[] {lo, hi};
    }

    /** The constructors whose fields mention no data type. */
    public List<Object> base(List<Object> d)
    {
      var all = d.subList(2, d.size());
      var found = new ArrayList<Object>();
      for (var c : all)
      {
        var ctor = form(c);
        boolean mentions = false;
        for (var f : ctor.subList(2, ctor.size()))
        {
          mentions |= mentionsData(f);
        }
        if (!mentions)
        {
          found.add(c);
        }
      }
      return found.isEmpty() ? all : found;
    }

    /** The logical type name of a descriptor's values. */
    public String typeName(Object descriptor)
    {
      var d = form(descriptor);
      return switch (atomText(d.get(0)))
      {
        case "int" -> atomText(d.get(1));
        case "bool" -> "Bool";
        case "text" -> "Text";
        case "unit" -> "Unit";
        case "list" -> "List " + typeName(d.get(1));
        case "maybe" -> "Maybe " + typeName(d.get(1));
        case "either" -> "Either (" + typeName(d.get(1)) + ") (" + typeName(d.get(2)) + ")";
        default -> atomText(d.get(1));
      };
    }

    private static Value text(List<Integer> units)
    {
      return new Value("Text", List.copyOf(units));
    }

    private static Value data(String type, String tag, List<Value> fields)
    {
      return new Value(type, new Data(tag, fields));
    }

    public Value generate(Object descriptor, SplitMix64 random, long size)
    {
      var d = resolve(descriptor);
      var type = typeName(d);
      switch (atomText(d.get(0)))
      {
        case "int" -> {
          var b = bounds(d);
          var lo = b[0];
          var hi = b[1];
          if (random.below(10) < 2)
          {
            var specials =
                List.of(lo, hi, BigInteger.ZERO.max(lo).min(hi), BigInteger.ONE.max(lo).min(hi));
            return new Value(type, specials.get((int) random.below(4)));
          }
          return new Value(type, lo.add(random.below(hi.subtract(lo).add(BigInteger.ONE))));
        }
        case "bool" -> {
          return bool(random.below(2) == 1);
        }
        case "text" -> {
          long n = random.below(size + 1);
          var units = new ArrayList<Integer>();
          for (long i = 0; i < n; i++)
          {
            units.add(32 + (int) random.below(95));
          }
          return text(units);
        }
        case "unit" -> {
          return absent("Unit");
        }
        case "list" -> {
          long n = random.below(size + 1);
          var items = new ArrayList<Value>();
          for (long i = 0; i < n; i++)
          {
            items.add(generate(d.get(1), random, size));
          }
          return new Value(type, List.copyOf(items));
        }
        case "maybe" -> {
          if (random.below(4) == 0)
          {
            return data(type, "Maybe::Absent", List.of());
          }
          return data(type, "Maybe::Present", List.of(generate(d.get(1), random, size)));
        }
        case "either" -> {
          if (random.below(2) == 0)
          {
            return data(type, "Either::Left", List.of(generate(d.get(1), random, size)));
          }
          return data(type, "Either::Right", List.of(generate(d.get(2), random, size)));
        }
        case "data" -> {
          var choices = size <= 0 ? base(d) : d.subList(2, d.size());
          var ctor = form(choices.get((int) random.below(choices.size())));
          var fields = new ArrayList<Value>();
          for (var f : ctor.subList(2, ctor.size()))
          {
            fields.add(generate(f, random, Math.max(size - 1, 0)));
          }
          return data(type, atomText(ctor.get(1)), fields);
        }
        default -> throw new IllegalArgumentException("unknown descriptor " + d);
      }
    }

    public Value minimal(Object descriptor)
    {
      var d = resolve(descriptor);
      var type = typeName(d);
      switch (atomText(d.get(0)))
      {
        case "int" -> {
          var b = bounds(d);
          return new Value(type, BigInteger.ZERO.max(b[0]).min(b[1]));
        }
        case "bool" -> {
          return bool(false);
        }
        case "text" -> {
          return text(List.of());
        }
        case "unit" -> {
          return absent("Unit");
        }
        case "list" -> {
          return new Value(type, List.of());
        }
        case "maybe" -> {
          return data(type, "Maybe::Absent", List.of());
        }
        case "either" -> {
          return data(type, "Either::Left", List.of(minimal(d.get(1))));
        }
        default -> {
          var ctor = form(base(d).get(0));
          var fields = new ArrayList<Value>();
          for (var f : ctor.subList(2, ctor.size()))
          {
            fields.add(minimal(f));
          }
          return data(type, atomText(ctor.get(1)), fields);
        }
      }
    }

    /** Smaller candidates for v, most aggressive first. */
    @SuppressWarnings("unchecked")
    public List<Value> shrink(Object descriptor, Value v)
    {
      var d = resolve(descriptor);
      var type = typeName(d);
      var out = new ArrayList<Value>();
      switch (atomText(d.get(0)))
      {
        case "int" -> {
          var target = (BigInteger) minimal(d).data();
          var n = (BigInteger) v.data();
          if (!n.equals(target))
          {
            out.add(new Value(type, target));
            out.add(new Value(type, n.subtract(n.subtract(target).divide(BigInteger.TWO))));
            out.add(
                new Value(
                    type,
                    n.subtract(
                        n.compareTo(target) > 0 ? BigInteger.ONE : BigInteger.ONE.negate())));
          }
        }
        case "bool" -> {
          if ((Boolean) v.data())
          {
            out.add(bool(false));
          }
        }
        case "text" -> {
          var units = (List<Integer>) v.data();
          if (!units.isEmpty())
          {
            out.add(text(List.of()));
            out.add(text(units.subList(0, units.size() / 2)));
            for (int i = 0; i < units.size(); i++)
            {
              var without = new ArrayList<>(units);
              without.remove(i);
              out.add(text(without));
            }
          }
        }
        case "list" -> {
          var items = (List<Value>) v.data();
          if (!items.isEmpty())
          {
            out.add(new Value(type, List.of()));
            out.add(new Value(type, List.copyOf(items.subList(0, items.size() / 2))));
            for (int i = 0; i < items.size(); i++)
            {
              var without = new ArrayList<>(items);
              without.remove(i);
              out.add(new Value(type, List.copyOf(without)));
            }
            for (int i = 0; i < items.size(); i++)
            {
              for (var c : shrink(d.get(1), items.get(i)))
              {
                var replaced = new ArrayList<>(items);
                replaced.set(i, c);
                out.add(new Value(type, List.copyOf(replaced)));
              }
            }
          }
        }
        case "maybe" -> {
          var value = (Data) v.data();
          if (value.tag().equals("Maybe::Present"))
          {
            out.add(data(type, "Maybe::Absent", List.of()));
            for (var c : shrink(d.get(1), value.fields().get(0)))
            {
              out.add(data(type, "Maybe::Present", List.of(c)));
            }
          }
        }
        case "either" -> {
          var value = (Data) v.data();
          var inner = value.tag().equals("Either::Left") ? d.get(1) : d.get(2);
          for (var c : shrink(inner, value.fields().get(0)))
          {
            out.add(data(type, value.tag(), List.of(c)));
          }
        }
        case "data" -> {
          var value = (Data) v.data();
          List<Object> ctor = null;
          for (var c : d.subList(2, d.size()))
          {
            if (atomText(form(c).get(1)).equals(value.tag()))
            {
              ctor = form(c);
              break;
            }
          }
          if (ctor == null)
          {
            throw new IllegalArgumentException("unknown constructor " + value.tag());
          }
          var fieldTypes = ctor.subList(2, ctor.size());
          int count = Math.min(value.fields().size(), fieldTypes.size());
          out.add(minimal(d));
          // A field of the same type is a smaller value of it.
          for (int i = 0; i < count; i++)
          {
            if (fieldTypes.get(i) instanceof List<?> fd
                && fd.size() == 2
                && sameAtom(fd.get(0), "ref")
                && sameAtom(fd.get(1), d.get(1)))
            {
              out.add(value.fields().get(i));
            }
          }
          for (int i = 0; i < count; i++)
          {
            for (var c : shrink(fieldTypes.get(i), value.fields().get(i)))
            {
              var replaced = new ArrayList<>(value.fields());
              replaced.set(i, c);
              out.add(data(type, value.tag(), replaced));
            }
          }
        }
        default -> {}
      }
      var original = render(v);
      var seen = new java.util.HashSet<String>();
      var unique = new ArrayList<Value>();
      for (var c : out)
      {
        var text = render(c);
        if (!text.equals(original) && seen.add(text))
        {
          unique.add(c);
        }
      }
      return unique;
    }
  }

  private static boolean mentionsData(Object d)
  {
    if (!(d instanceof List<?> f))
    {
      return false;
    }
    var head = atomText(f.get(0));
    if (head.equals("ref") || head.equals("data"))
    {
      return true;
    }
    for (var x : f.subList(1, f.size()))
    {
      if (mentionsData(x))
      {
        return true;
      }
    }
    return false;
  }

  /**
   * Where a structured actual value first differs from the expected one, by the portable rendering:
   * "" when they agree or neither has parts.
   */
  public static String difference(Value actual, Value expected)
  {
    var path = new ArrayList<String>();
    Value a = actual;
    Value b = expected;
    String shownA = null;
    String shownB = null;
    loop:
    while (true)
    {
      // Lists of values; a text's code points, code units or bytes are not.
      if (a.data() instanceof List<?> x
          && b.data() instanceof List<?> y
          && x.stream().allMatch(Value.class::isInstance)
          && y.stream().allMatch(Value.class::isInstance)
          && !a.type().equals("Text")
          && !b.type().equals("Text"))
      {
        for (int i = 0; i < x.size() && i < y.size(); i++)
        {
          if (!render((Value) x.get(i)).equals(render((Value) y.get(i))))
          {
            path.add("item " + (i + 1));
            a = (Value) x.get(i);
            b = (Value) y.get(i);
            continue loop;
          }
        }
        if (x.size() == y.size())
        {
          return "";
        }
        path.add("length");
        shownA = Integer.toString(x.size());
        shownB = Integer.toString(y.size());
        break;
      }
      if (a.data() instanceof Data x
          && b.data() instanceof Data y
          && x.tag().equals(y.tag())
          && x.fields().size() == y.fields().size())
      {
        String name = x.tag().substring(x.tag().lastIndexOf("::") + 2);
        for (int i = 0; i < x.fields().size(); i++)
        {
          if (!render(x.fields().get(i)).equals(render(y.fields().get(i))))
          {
            path.add("field " + (i + 1) + " of " + name);
            a = x.fields().get(i);
            b = y.fields().get(i);
            continue loop;
          }
        }
        return "";
      }
      break;
    }
    if (path.isEmpty())
    {
      return "";
    }
    if (shownA == null)
    {
      shownA = render(a);
      shownB = render(b);
    }
    return " | first difference at "
        + String.join(", ", path)
        + ": expected "
        + shownB
        + ", actual "
        + shownA;
  }

  /** A Text value's code points. */
  static int[] codePoints(Value v)
  {
    List<?> units = (List<?>) v.data();
    int[] points = new int[units.size()];
    for (int i = 0; i < points.length; i++)
    {
      points[i] = (Integer) units.get(i);
    }
    return points;
  }

  /** The text of a Text value's code points. */
  static String codePointsText(Value v)
  {
    int[] points = codePoints(v);
    return new String(points, 0, points.length);
  }

  // Portable regular expressions (see LawSpec.Regex): the subset of RE2 and
  // ECMAScript that means the same in both, matched against a whole text,
  // code point by code point. The compiler has checked every pattern; a
  // pattern that is not portable throws here too.
  private record RegexItem(boolean negated, int[][] ranges)
  {}

  private static final class RegexNode
  {
    final char kind; // 's' set, 'q' sequence, 'a' alternatives, 'r' repeat
    boolean negated;
    List<RegexItem> items = List.of();
    List<RegexNode> children = List.of();
    int low;
    int high = -1; // -1 when unbounded

    RegexNode(char kind)
    {
      this.kind = kind;
    }
  }

  private static final int[][] REGEX_DIGITS = {{48, 57}};
  private static final int[][] REGEX_WORD = {{48, 57}, {65, 90}, {95, 95}, {97, 122}};
  private static final int[][] REGEX_SPACE = {{9, 13}, {32, 32}};
  private static final java.util.concurrent.ConcurrentHashMap<String, RegexNode> REGEX_CACHE =
      new java.util.concurrent.ConcurrentHashMap<>();

  private static final class RegexParser
  {
    final String pattern;
    final int[] cs;
    int pos = 0;

    RegexParser(String pattern)
    {
      this.pattern = pattern;
      this.cs = pattern.codePoints().toArray();
    }

    int peek()
    {
      return pos < cs.length ? cs[pos] : -1;
    }

    RuntimeException fail(String message)
    {
      return new IllegalArgumentException("regex \"" + pattern + "\" is not portable: " + message);
    }

    RegexItem escape()
    {
      pos++;
      int c = peek();
      if (c < 0)
      {
        throw fail("the regex ends with a lone \\");
      }
      pos++;
      switch (c)
      {
        case 'd':
          return new RegexItem(false, REGEX_DIGITS);
        case 'D':
          return new RegexItem(true, REGEX_DIGITS);
        case 'w':
          return new RegexItem(false, REGEX_WORD);
        case 'W':
          return new RegexItem(true, REGEX_WORD);
        case 's':
          return new RegexItem(false, REGEX_SPACE);
        case 'S':
          return new RegexItem(true, REGEX_SPACE);
        case 'n':
          return new RegexItem(false, new int[][] {{10, 10}});
        case 't':
          return new RegexItem(false, new int[][] {{9, 9}});
        case 'r':
          return new RegexItem(false, new int[][] {{13, 13}});
        case 'f':
          return new RegexItem(false, new int[][] {{12, 12}});
        case 'v':
          return new RegexItem(false, new int[][] {{11, 11}});
        default:
          break;
      }
      if ("\\.^$|?*+()[]{}-/".indexOf(c) >= 0)
      {
        return new RegexItem(false, new int[][] {{c, c}});
      }
      throw fail("\\" + new String(Character.toChars(c)) + " is not a portable escape");
    }

    RegexItem literal()
    {
      int c = peek();
      if (c == '[')
      {
        throw fail("write \\[ for the character inside a class");
      }
      pos++;
      return new RegexItem(false, new int[][] {{c, c}});
    }

    static boolean single(RegexItem item)
    {
      return !item.negated()
          && item.ranges().length == 1
          && item.ranges()[0][0] == item.ranges()[0][1];
    }

    RegexNode charClass()
    {
      pos++;
      boolean negated = peek() == '^';
      if (negated)
      {
        pos++;
      }
      var items = new ArrayList<RegexItem>();
      boolean first = true;
      while (true)
      {
        int c = peek();
        if (c < 0)
        {
          throw fail("a [ is never closed");
        }
        if (c == ']')
        {
          if (first)
          {
            throw fail("an empty class is not portable");
          }
          pos++;
          var node = new RegexNode('s');
          node.negated = negated;
          node.items = items;
          return node;
        }
        first = false;
        RegexItem item = c == '\\' ? escape() : literal();
        if (single(item) && peek() == '-' && pos + 1 < cs.length && cs[pos + 1] != ']')
        {
          pos++;
          RegexItem high = peek() == '\\' ? escape() : literal();
          if (!single(high))
          {
            throw fail("a range ends with one character");
          }
          if (high.ranges()[0][0] < item.ranges()[0][0])
          {
            throw fail("a range must run from low to high");
          }
          items.add(new RegexItem(false, new int[][] {{item.ranges()[0][0], high.ranges()[0][0]}}));
        }
        else
        {
          items.add(item);
        }
      }
    }

    String digits()
    {
      int start = pos;
      while (peek() >= '0' && peek() <= '9')
      {
        pos++;
      }
      return new String(cs, start, pos - start);
    }

    static RegexNode set(boolean negated, RegexItem item)
    {
      var node = new RegexNode('s');
      node.negated = negated;
      node.items = List.of(item);
      return node;
    }

    RegexNode atom()
    {
      int c = peek();
      if (c == '(')
      {
        pos++;
        if (peek() == '?')
        {
          if (pos + 1 < cs.length && cs[pos + 1] == ':')
          {
            pos += 2;
          }
          else
          {
            throw fail("only (?: ...) groups are portable");
          }
        }
        RegexNode node = alternatives();
        if (peek() != ')')
        {
          throw fail("a ( is never closed");
        }
        pos++;
        return node;
      }
      if (c == '[')
      {
        return charClass();
      }
      if (c == '.')
      {
        pos++;
        return set(true, new RegexItem(false, new int[][] {{10, 10}}));
      }
      if (c == '\\')
      {
        return set(false, escape());
      }
      if ("*+?{^$]}".indexOf(c) >= 0)
      {
        throw fail("unexpected " + new String(Character.toChars(c)));
      }
      pos++;
      return set(false, new RegexItem(false, new int[][] {{c, c}}));
    }

    static boolean quantifier(int c)
    {
      return c >= 0 && "*+?{".indexOf(c) >= 0;
    }

    RegexNode quantified(RegexNode node)
    {
      int c = peek();
      if (!quantifier(c))
      {
        return node;
      }
      pos++;
      var repeat = new RegexNode('r');
      repeat.children = List.of(node);
      if (c == '*')
      {
        repeat.low = 0;
      }
      else if (c == '+')
      {
        repeat.low = 1;
      }
      else if (c == '?')
      {
        repeat.low = 0;
        repeat.high = 1;
      }
      else
      {
        String lowText = digits();
        String highText = null;
        boolean bounded = true;
        if (peek() == '}')
        {
          highText = lowText;
        }
        else if (peek() == ',')
        {
          pos++;
          highText = digits();
          bounded = !highText.isEmpty();
          if (peek() != '}')
          {
            throw fail("a repetition is {n}, {n,} or {n,m}");
          }
        }
        else
        {
          throw fail("a repetition is {n}, {n,} or {n,m}");
        }
        pos++;
        if (lowText.isEmpty())
        {
          throw fail("a repetition is {n}, {n,} or {n,m}");
        }
        repeat.low = Integer.parseInt(lowText);
        repeat.high = bounded ? Integer.parseInt(highText) : -1;
        if (repeat.low > 1000 || (bounded && (repeat.high > 1000 || repeat.high < repeat.low)))
        {
          throw fail("a repetition count is at most 1000, and n must not exceed m");
        }
      }
      if (quantifier(peek()))
      {
        throw fail("a repetition cannot itself be repeated");
      }
      return repeat;
    }

    RegexNode sequence()
    {
      var items = new ArrayList<RegexNode>();
      while (peek() >= 0 && peek() != '|' && peek() != ')')
      {
        items.add(quantified(atom()));
      }
      var node = new RegexNode('q');
      node.children = items;
      return node;
    }

    RegexNode alternatives()
    {
      var branches = new ArrayList<RegexNode>();
      branches.add(sequence());
      while (peek() == '|')
      {
        pos++;
        branches.add(sequence());
      }
      if (branches.size() == 1)
      {
        return branches.get(0);
      }
      var node = new RegexNode('a');
      node.children = branches;
      return node;
    }

    RegexNode parse()
    {
      RegexNode node = alternatives();
      if (pos != cs.length)
      {
        throw fail("a ) has no ( before it");
      }
      return node;
    }
  }

  private static boolean[] regexReach(RegexNode node, int[] text, boolean[] positions)
  {
    var next = new boolean[text.length + 1];
    switch (node.kind)
    {
      case 's' -> {
        for (int p = 0; p < text.length; p++)
        {
          if (!positions[p])
          {
            continue;
          }
          int c = text[p];
          boolean inside = false;
          for (RegexItem item : node.items)
          {
            boolean in = false;
            for (int[] r : item.ranges())
            {
              if (r[0] <= c && c <= r[1])
              {
                in = true;
                break;
              }
            }
            if (in != item.negated())
            {
              inside = true;
              break;
            }
          }
          if (inside != node.negated)
          {
            next[p + 1] = true;
          }
        }
        return next;
      }
      case 'q' -> {
        for (RegexNode child : node.children)
        {
          positions = regexReach(child, text, positions);
        }
        return positions;
      }
      case 'a' -> {
        for (RegexNode child : node.children)
        {
          boolean[] reached = regexReach(child, text, positions);
          for (int p = 0; p < reached.length; p++)
          {
            if (reached[p])
            {
              next[p] = true;
            }
          }
        }
        return next;
      }
      default -> {
        RegexNode body = node.children.get(0);
        for (int i = 0; i < node.low; i++)
        {
          positions = regexReach(body, text, positions);
        }
        boolean[] seen = positions.clone();
        boolean[] frontier = positions;
        int limit = node.high >= 0 ? node.high - node.low : -1;
        while (limit != 0)
        {
          boolean[] reached = regexReach(body, text, frontier);
          frontier = new boolean[text.length + 1];
          boolean any = false;
          for (int p = 0; p < reached.length; p++)
          {
            if (reached[p] && !seen[p])
            {
              frontier[p] = true;
              seen[p] = true;
              any = true;
            }
          }
          if (!any)
          {
            break;
          }
          if (limit > 0)
          {
            limit--;
          }
        }
        return seen;
      }
    }
  }

  /** Whether the portable regex matches all of text, given as code points. */
  public static boolean regexMatches(String pattern, int[] text)
  {
    RegexNode node = REGEX_CACHE.computeIfAbsent(pattern, p -> new RegexParser(p).parse());
    var start = new boolean[text.length + 1];
    start[0] = true;
    return regexReach(node, text, start)[text.length];
  }

  // Recorded values: a law compares a value's portable rendering with the
  // text stored under recorded/<unit>/<name> in the project. LAWSPEC_RECORDED
  // names the folder; otherwise it is recorded/ in the nearest folder, from
  // the working one up, that holds lawspec.edn or recorded/. With
  // LAWSPEC_UPDATE_RECORDED=1 (lawspec test --update-recorded) a law records
  // the value instead.
  static java.nio.file.Path recordedRoot()
  {
    String given = System.getenv("LAWSPEC_RECORDED");
    if (given != null && !given.isEmpty())
    {
      return java.nio.file.Path.of(given);
    }
    java.nio.file.Path start = java.nio.file.Path.of("").toAbsolutePath();
    for (java.nio.file.Path folder = start; folder != null; folder = folder.getParent())
    {
      if (java.nio.file.Files.exists(folder.resolve("lawspec.edn"))
          || java.nio.file.Files.isDirectory(folder.resolve("recorded")))
      {
        return folder.resolve("recorded");
      }
    }
    return start.resolve("recorded");
  }

  public static boolean recorded(String key, Value value)
  {
    String text = recordedText(value);
    java.nio.file.Path path = recordedRoot();
    for (String part : key.split("/"))
    {
      path = path.resolve(part);
    }
    try
    {
      if ("1".equals(System.getenv("LAWSPEC_UPDATE_RECORDED")))
      {
        java.nio.file.Files.createDirectories(path.getParent());
        java.nio.file.Files.writeString(path, text + "\n");
        return true;
      }
      if (!java.nio.file.Files.exists(path))
      {
        throw new AssertionError(
            "no recording recorded/"
                + key
                + "; run lawspec test --update-recorded to record "
                + text);
      }
      String stored = java.nio.file.Files.readString(path);
      if (stored.endsWith("\n"))
      {
        stored = stored.substring(0, stored.length() - 1);
      }
      if (!stored.equals(text))
      {
        throw new AssertionError(
            "recorded/"
                + key
                + " differs: expected "
                + stored
                + ", actual "
                + text
                + " (lawspec test --update-recorded records the new value)");
      }
      return true;
    }
    catch (java.io.IOException e)
    {
      throw new java.io.UncheckedIOException(e);
    }
  }

  // Shared resources (share R per group | unit | run, in a harness): one value
  // per scope key. The first use acquires it; every later use resets it first,
  // so no case sees what another left. One case holds it at a time, and it is
  // released when the JVM exits.
  public interface SharedAcquire<T>
  {
    T get() throws Exception;
  }

  public interface SharedUse<T>
  {
    void accept(T value) throws Exception;
  }

  private static final class SharedEntry
  {
    // A semaphore, not a lock: a suspending Kotlin case may end on another thread.
    final java.util.concurrent.Semaphore lock = new java.util.concurrent.Semaphore(1);
    boolean held;
    int users;
    boolean concurrent;
    Object value;
  }

  private static final java.util.Map<String, SharedEntry> shared =
      new java.util.concurrent.ConcurrentHashMap<>();

  public static <T> T share(
      String key, SharedAcquire<T> acquire, SharedUse<T> reset, SharedUse<T> release)
  {
    return share(key, acquire, reset, release, false);
  }

  /**
   * A concurrent resource is held by any number of cases at once, and reset only when none holds
   * it.
   */
  @SuppressWarnings("unchecked")
  public static <T> T share(
      String key,
      SharedAcquire<T> acquire,
      SharedUse<T> reset,
      SharedUse<T> release,
      boolean concurrent)
  {
    SharedEntry entry = shared.computeIfAbsent(key, k -> new SharedEntry());
    if (concurrent)
    {
      synchronized (entry)
      {
        entry.concurrent = true;
        try
        {
          if (entry.held && entry.users == 0)
          {
            reset.accept((T) entry.value);
          }
          else if (!entry.held)
          {
            entry.value = acquire.get();
            entry.held = true;
            final T value = (T) entry.value;
            Runtime.getRuntime()
                .addShutdownHook(
                    new Thread(
                        () -> {
                          try
                          {
                            release.accept(value);
                          }
                          catch (Exception e)
                          {
                            throw new RuntimeException(e);
                          }
                        }));
          }
        }
        catch (RuntimeException | Error e)
        {
          throw e;
        }
        catch (Exception e)
        {
          throw new RuntimeException(e);
        }
        entry.users++;
        return (T) entry.value;
      }
    }
    entry.lock.acquireUninterruptibly();
    try
    {
      if (entry.held)
      {
        reset.accept((T) entry.value);
      }
      else
      {
        entry.value = acquire.get();
        entry.held = true;
        final T value = (T) entry.value;
        Runtime.getRuntime()
            .addShutdownHook(
                new Thread(
                    () -> {
                      try
                      {
                        release.accept(value);
                      }
                      catch (Exception e)
                      {
                        throw new RuntimeException(e);
                      }
                    }));
      }
      return (T) entry.value;
    }
    catch (RuntimeException | Error e)
    {
      entry.lock.release();
      throw e;
    }
    catch (Exception e)
    {
      entry.lock.release();
      throw new RuntimeException(e);
    }
  }

  public static void unshare(String key)
  {
    SharedEntry entry = shared.get(key);
    if (entry == null)
    {
      return;
    }
    if (entry.concurrent)
    {
      synchronized (entry)
      {
        entry.users--;
      }
      return;
    }
    entry.lock.release();
  }

  // Built-in resources (see LawSpec.Resources): a law acquires them before
  // each case and releases them after it. A JVM cannot change its process
  // environment, so the environment it saves and restores is its system
  // properties.
  public static String acquireResource(String kind)
  {
    try
    {
      switch (kind)
      {
        case "temporaryDirectory":
          return java.nio.file.Files.createTempDirectory("lawspec-").toString();
        case "temporaryFile":
          return java.nio.file.Files.createTempFile("lawspec-", "").toString();
        case "environment":
        {
          var saved = new java.util.TreeMap<String, String>();
          for (String name : System.getProperties().stringPropertyNames())
          {
            saved.put(name, System.getProperty(name));
          }
          var out = new StringBuilder();
          for (var entry : saved.entrySet())
          {
            out.append(entry.getKey()).append('\0').append(entry.getValue()).append('\0');
          }
          return out.toString();
        }
        default:
          throw new IllegalArgumentException("unknown resource kind " + kind);
      }
    }
    catch (java.io.IOException e)
    {
      throw new java.io.UncheckedIOException(e);
    }
  }

  public static void releaseResource(String kind, String value)
  {
    switch (kind)
    {
      case "temporaryDirectory", "temporaryFile" -> {
        try (var paths = java.nio.file.Files.walk(java.nio.file.Path.of(value)))
        {
          paths.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
        }
        catch (java.io.IOException e)
        {
          // Already gone.
        }
      }
      case "environment" -> {
        var saved = new java.util.HashMap<String, String>();
        String[] parts = value.split("\0", -1);
        for (int i = 0; i + 1 < parts.length; i += 2)
        {
          saved.put(parts[i], parts[i + 1]);
        }
        for (String name : System.getProperties().stringPropertyNames())
        {
          if (!saved.containsKey(name))
          {
            System.clearProperty(name);
          }
        }
        for (var entry : saved.entrySet())
        {
          if (!entry.getValue().equals(System.getProperty(entry.getKey())))
          {
            System.setProperty(entry.getKey(), entry.getValue());
          }
        }
      }
      default -> throw new IllegalArgumentException("unknown resource kind " + kind);
    }
  }

  /** A TCP port on the local host that is free now. */
  static int freePort()
  {
    try (var socket = new java.net.ServerSocket(0, 1, java.net.InetAddress.getLoopbackAddress()))
    {
      return socket.getLocalPort();
    }
    catch (java.io.IOException e)
    {
      throw new java.io.UncheckedIOException(e);
    }
  }

  /** A value's canonical text, the same on every target. */
  public static String render(Value v)
  {
    Object data = v.data();
    if (data instanceof Boolean b)
    {
      return b ? "true" : "false";
    }
    if (data instanceof BigInteger n)
    {
      return n.toString();
    }
    if (data instanceof Handle h)
    {
      return handleLabel(v.type(), h.target());
    }
    if (v.type().equals("Text") && data instanceof List<?> units)
    {
      var out = new StringBuilder("\"");
      for (Object unit : units)
      {
        int c = (Integer) unit;
        if (c == '\\')
        {
          out.append("\\\\");
        }
        else if (c == '"')
        {
          out.append("\\\"");
        }
        else
        {
          out.appendCodePoint(c);
        }
      }
      return out.append('"').toString();
    }
    if (data == null)
    {
      return "()";
    }
    if (data instanceof List<?> items)
    {
      var parts = new ArrayList<String>();
      for (Object item : items)
      {
        parts.add(render((Value) item));
      }
      return "[" + String.join(", ", parts) + "]";
    }
    if (data instanceof Data value)
    {
      String tag = value.tag();
      int cut = tag.lastIndexOf("::");
      String name = cut < 0 ? tag : tag.substring(cut + 2);
      if (value.fields().isEmpty())
      {
        return name;
      }
      var parts = new ArrayList<String>();
      for (Value field : value.fields())
      {
        parts.add(render(field));
      }
      return name + "(" + String.join(", ", parts) + ")";
    }
    return String.valueOf(data);
  }

  /**
   * Typed snapshot text, with identities numbered inside one recording. ref:REQ-law-primitives
   * ref:DEC-portable-exact-arithmetic
   */
  public static String recordedText(Value value)
  {
    return new RecordedText().render(value);
  }

  private static final class RecordedText
  {
    private final java.util.IdentityHashMap<Object, Integer> symbols =
        new java.util.IdentityHashMap<>();
    private final Map<String, java.util.IdentityHashMap<Object, Integer>> handles =
        new java.util.HashMap<>();

    private static String quote(String text)
    {
      return "\"" + text.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }

    private static String floating(double value, int width)
    {
      String name = "float" + width;
      if (Double.isNaN(value))
      {
        return name + "NaN";
      }
      String hex =
          width == 32
              ? String.format("%08x", Float.floatToRawIntBits((float) value))
              : String.format("%016x", Double.doubleToRawLongBits(value));
      return name + "Bits(\"" + hex + "\")";
    }

    private static int identity(java.util.IdentityHashMap<Object, Integer> known, Object value)
    {
      Integer found = known.get(value);
      if (found != null)
      {
        return found;
      }
      int number = known.size() + 1;
      known.put(value, number);
      return number;
    }

    private static String shortName(String type)
    {
      int cut = type.lastIndexOf("::");
      return cut < 0 ? type : type.substring(cut + 2);
    }

    private String parts(List<Value> values)
    {
      var parts = new ArrayList<String>();
      for (Value value : values)
      {
        parts.add(render(value));
      }
      return String.join(", ", parts);
    }

    private String render(Value v)
    {
      Object data = v.data();
      String type = v.type();
      if (data instanceof Boolean || data instanceof BigInteger)
      {
        return data.toString();
      }
      if (data instanceof BigDecimal decimal)
      {
        BigInteger c = decimal.unscaledValue();
        long e = -(long) decimal.scale();
        if (c.signum() == 0)
        {
          return "0e0";
        }
        while (c.remainder(BigInteger.TEN).signum() == 0)
        {
          c = c.divide(BigInteger.TEN);
          e++;
        }
        return c + "e" + e;
      }
      if (data instanceof Ratio r)
      {
        return "rational(" + r.n() + ", " + r.d() + ")";
      }
      if (data instanceof Double d)
      {
        return floating(d, type.equals("Float32") ? 32 : 64);
      }
      if (data instanceof Complex c)
      {
        int width = type.equals("Complex64") ? 32 : 64;
        return type + "(" + floating(c.real(), width) + ", " + floating(c.imaginary(), width) + ")";
      }
      if (data instanceof Integer c)
      {
        return type.equals("Char") ? quote(new String(Character.toChars(c))) : c.toString();
      }
      if (data instanceof SymbolValue symbol)
      {
        return "symbol(" + identity(symbols, symbol) + ", " + quote(symbol.description()) + ")";
      }
      if (data instanceof Handle h)
      {
        var known = handles.computeIfAbsent(type, key -> new java.util.IdentityHashMap<>());
        return shortName(type) + "#" + identity(known, h.target());
      }
      if (data instanceof Presence p)
      {
        boolean nullable = type.equals("Nullable") || type.startsWith("Nullable ");
        if (!p.present())
        {
          return nullable ? "null" : "undefined";
        }
        return (nullable ? "nullable(" : "optional(") + render(p.value()) + ")";
      }
      if (data == null)
      {
        return switch (type)
        {
          case "Unit" -> "()";
          case "Null" -> "null";
          case "Undefined" -> "undefined";
          default -> throw new IllegalArgumentException("invalid recorded absence: " + type);
        };
      }
      if (data instanceof List<?> items)
      {
        if (type.equals("Text"))
        {
          return quote(codePointsText(v));
        }
        if (List.of("Bytes", "CodePointText", "Utf16Text").contains(type))
        {
          String name =
              type.equals("Bytes") ? "bytes" : type.equals("Utf16Text") ? "utf16" : "codePoints";
          return name
              + "(["
              + items.stream()
                  .map(Object::toString)
                  .collect(java.util.stream.Collectors.joining(", "))
              + "])";
        }
        var values = new ArrayList<Value>();
        for (Object item : items)
        {
          values.add((Value) item);
        }
        return "[" + parts(values) + "]";
      }
      if (data instanceof Data d)
      {
        String name = shortName(d.tag());
        return d.fields().isEmpty() ? name : name + "(" + parts(d.fields()) + ")";
      }
      throw new IllegalArgumentException("invalid recorded value: " + type);
    }
  }

  /** A descriptor text's data types and its last form, the one generated. */
  public record Described(Values values, Object descriptor)
  {}

  public static Described valuesFrom(String text)
  {
    var forms = readDescriptor(text);
    var table = new java.util.HashMap<String, List<Object>>();
    for (var f : forms)
    {
      if (f instanceof List<?> && atomText(form(f).get(0)).equals("data"))
      {
        table.put(atomText(form(f).get(1)), form(f));
      }
    }
    return new Described(new Values(table), forms.getLast());
  }

  /** count values generated from one SplitMix64 seed, rendered. */
  public static List<String> generated(String text, long seed, long size, long count)
  {
    var described = valuesFrom(text);
    var random = new SplitMix64(seed);
    var result = new ArrayList<String>();
    for (long i = 0; i < count; i++)
    {
      result.add(render(described.values().generate(described.descriptor(), random, size)));
    }
    return result;
  }

  /** The shrink candidates of the first value generated, rendered. */
  public static List<String> shrunk(String text, long seed, long size)
  {
    var described = valuesFrom(text);
    var values = described.values();
    var first = values.generate(described.descriptor(), new SplitMix64(seed), size);
    var result = new ArrayList<String>();
    for (var c : values.shrink(described.descriptor(), first))
    {
      result.add(render(c));
    }
    return result;
  }

  // Stateful models. A model's spec (see LawSpec.MachineSpec) lists its data
  // types, start and commands; the callbacks beside it are the generated
  // definitions that call the adapters, the references over the model state,
  // preconditions, the abstraction and invariants, each taking symbols first.
  // A run is generated by simulating the pure model, so every command in it is
  // allowed by typestate, its precondition and its reference; it is then
  // executed against the adapters and every result, abstracted state and
  // invariant is checked. A failing run is shrunk by dropping commands and
  // shrinking arguments, replaying the model to keep each candidate valid.

  /** A generated definition: symbols first, then its arguments. */
  public interface ModelCallback
  {
    Value apply(Map<String, Object> symbols, List<Value> args);
  }

  /** A command of a model: its spec form and its callbacks. */
  public static final class ModelCommand
  {
    final String name;
    final List<Object> arguments;
    final int state;
    final boolean unit;
    final List<Object> needs;
    final List<Object> shifts;

    /** The argument naming the key the command touches, for per-key checks; -1 for none. */
    final int key;

    /** An actor's restart: never generated as a step; injected crashes run it. */
    final boolean restart;

    /** The injected crash of an actor model (a step with no arguments). */
    final boolean crash;

    ModelCallback run;
    final ModelCallback reference;
    final ModelCallback when;

    /** The injected crash step: run and reference take the system or model state. */
    ModelCommand(ModelCallback run, ModelCallback reference)
    {
      name = "crash";
      arguments = List.of();
      state = 0;
      unit = true;
      needs = List.of();
      shifts = List.of();
      key = -1;
      restart = false;
      crash = true;
      this.run = run;
      this.reference = reference;
      when = null;
    }

    ModelCommand(List<Object> form, ModelCallback[] callbacks)
    {
      var fields = new java.util.HashMap<String, List<Object>>();
      for (var f : form.subList(2, form.size()))
      {
        var field = form(f);
        fields.put(atomText(field.get(0)), field.subList(1, field.size()));
      }
      name = atomText(form.get(1));
      arguments = fields.get("arguments");
      state = ((BigInteger) fields.get("state").get(0)).intValueExact();
      unit = atomText(fields.get("unit").get(0)).equals("true");
      needs = fields.get("needs");
      shifts = fields.get("shifts");
      var keyField = fields.getOrDefault("key", List.of("none"));
      key = keyField.get(0) instanceof BigInteger k ? k.intValueExact() : -1;
      restart = atomText(fields.getOrDefault("restart", List.of("false")).get(0)).equals("true");
      crash = false;
      run = callbacks[0];
      reference = callbacks[1];
      when = callbacks.length > 2 ? callbacks[2] : null;
    }

    boolean admits(List<BigInteger> indices)
    {
      int n = Math.min(needs.size(), indices.size());
      for (int k = 0; k < n; k++)
      {
        var need = form(needs.get(k));
        var bound = (BigInteger) need.get(1);
        var i = indices.get(k);
        boolean ok =
            atomText(need.get(0)).equals("atleast") ? i.compareTo(bound) >= 0 : i.equals(bound);
        if (!ok)
        {
          return false;
        }
      }
      return true;
    }

    List<BigInteger> shifted(List<BigInteger> indices)
    {
      if (crash)
      {
        return indices;
      }
      int n = Math.min(shifts.size(), indices.size());
      var out = new ArrayList<BigInteger>();
      for (int k = 0; k < n; k++)
      {
        var shift = form(shifts.get(k));
        var by = (BigInteger) shift.get(1);
        out.add(atomText(shift.get(0)).equals("by") ? indices.get(k).add(by) : by);
      }
      return out;
    }
  }

  /** A model: its spec and the generated definitions beside it. */
  public static final class Model
  {
    final String name;
    final boolean shared;
    final Values values;
    final List<BigInteger> startIndices;
    final List<Object> startArguments;
    final ModelCallback startRun;
    final ModelCallback startModel;
    final List<ModelCommand> commands;

    /** The commands a sequential run may take: an actor's also has its crash, last. */
    final List<ModelCommand> steps;

    final ModelCallback abstractState;
    final List<String> invariantKinds;
    final List<ModelCallback> invariants;
    final boolean perKey;

    /** linearizable, sequential, causal or eventual. */
    final String consistency;

    /**
     * start: run, model; each command: run, reference, when (null when absent); abstractState may
     * be null.
     */
    public Model(
        String spec,
        ModelCallback[] start,
        ModelCallback[][] commands,
        ModelCallback abstractState,
        ModelCallback[] invariants)
    {
      var forms = readDescriptor(spec);
      var machine = form(forms.get(0));
      name = atomText(machine.get(1));
      shared = atomText(machine.get(2)).equals("shared");
      var table = new java.util.HashMap<String, List<Object>>();
      List<Object> startForm = null;
      List<Object> invariantForm = null;
      var commandForms = new ArrayList<List<Object>>();
      boolean keyed = false;
      boolean actor = false;
      String declaredConsistency = "linearizable";
      for (var f : forms)
      {
        var item = form(f);
        switch (atomText(item.get(0)))
        {
          case "data" -> table.put(atomText(item.get(1)), item);
          case "start" -> {
            if (startForm == null)
            {
              startForm = item;
            }
          }
          case "command" -> commandForms.add(item);
          case "invariants" -> {
            if (invariantForm == null)
            {
              invariantForm = item;
            }
          }
          case "perkey" -> {
            if (item.size() > 1 && atomText(item.get(1)).equals("true"))
            {
              keyed = true;
            }
          }
          case "actor" -> {
            if (item.size() > 1 && atomText(item.get(1)).equals("true"))
            {
              actor = true;
            }
          }
          case "consistency" -> {
            if (item.size() > 1)
            {
              declaredConsistency = atomText(item.get(1));
            }
          }
          default -> {}
        }
      }
      values = new Values(table);
      var startFields = new java.util.HashMap<String, List<Object>>();
      for (var f : startForm.subList(1, startForm.size()))
      {
        var field = form(f);
        startFields.put(atomText(field.get(0)), field.subList(1, field.size()));
      }
      var indices = new ArrayList<BigInteger>();
      for (var i : startFields.getOrDefault("indices", List.of()))
      {
        indices.add((BigInteger) i);
      }
      startIndices = indices;
      startArguments = startFields.getOrDefault("arguments", List.of());
      // An actor model's start and handlers run inside an actor; the
      // abstraction and state invariants read its state between messages.
      // Sequential runs of an actor also inject crashes: the actor restarts
      // from its last state (restart from) or its start, and the model
      // follows the restart's reference (or the start's model state).
      var built = new ArrayList<ModelCommand>();
      ModelCommand restart = null;
      for (int k = 0; k < Math.min(commandForms.size(), commands.length); k++)
      {
        var c = new ModelCommand(commandForms.get(k), commands[k]);
        if (c.restart)
        {
          if (restart == null)
          {
            restart = c;
          }
        }
        else
        {
          built.add(c);
        }
      }
      if (actor)
      {
        var run = start[0];
        var begin = start[1];
        startRun =
            (symbols, args) -> {
              symbols.put(START_ARGUMENTS, args);
              return handle(ACTOR_HANDLE, new Actor<Value>(run.apply(symbols, args)));
            };
        startModel =
            (symbols, args) -> {
              symbols.put(START_ARGUMENTS, args);
              return begin.apply(symbols, args);
            };
        for (var c : built)
        {
          c.run = actorCommand(c.run, c.unit);
        }
        var steps = new ArrayList<ModelCommand>(built);
        steps.add(crashStep(run, begin, restart));
        this.steps = steps;
      }
      else
      {
        startRun = start[0];
        startModel = start[1];
        this.steps = built;
      }
      this.commands = built;
      this.abstractState =
          actor && abstractState != null ? actorRead(abstractState) : abstractState;
      var kinds = new ArrayList<String>();
      var checks = new ArrayList<ModelCallback>();
      var invariantList = invariants == null ? new ModelCallback[0] : invariants;
      int n = Math.min(invariantForm.size() - 1, invariantList.length);
      for (int k = 0; k < n; k++)
      {
        kinds.add(atomText(invariantForm.get(k + 1)));
        boolean onState = !atomText(invariantForm.get(k + 1)).equals("model");
        checks.add(actor && onState ? actorRead(invariantList[k]) : invariantList[k]);
      }
      invariantKinds = kinds;
      this.invariants = checks;
      perKey = keyed;
      consistency = declaredConsistency;
    }
  }

  private static final String ACTOR_HANDLE = "lawspec.actor";

  @SuppressWarnings("unchecked")
  private static Actor<Value> actorOf(Value handle)
  {
    return (Actor<Value>) handleTarget(handle);
  }

  private static final String START_ARGUMENTS = "_lawspec_start";

  @SuppressWarnings("unchecked")
  private static List<Value> startArguments(Map<String, Object> symbols)
  {
    return (List<Value>) symbols.getOrDefault(START_ARGUMENTS, List.of());
  }

  /** An injected crash of an actor model, as a step with no arguments. */
  private static ModelCommand crashStep(
      ModelCallback startRun, ModelCallback startModel, ModelCommand restart)
  {
    ModelCallback run =
        (symbols, args) -> {
          var actor = actorOf(args.get(0));
          if (restart != null)
          {
            actor.restart(s -> restart.run.apply(symbols, List.of(s)));
          }
          else
          {
            actor.restart(s -> startRun.apply(symbols, startArguments(symbols)));
          }
          return absent("Unit");
        };
    ModelCallback reference =
        (symbols, args) ->
            restart != null
                ? restart.reference.apply(symbols, args)
                : startModel.apply(symbols, startArguments(symbols));
    return new ModelCommand(run, reference);
  }

  /**
   * A handler bridge (the state first, returning Pair reply state, or the state alone for a Unit
   * reply) as a command on an actor.
   */
  private static ModelCallback actorCommand(ModelCallback run, boolean unit)
  {
    return (symbols, args) ->
        actorOf(args.get(0))
            .call(
                state -> {
                  var full = new ArrayList<Value>(args);
                  full.set(0, state);
                  var out = run.apply(symbols, full);
                  if (unit)
                  {
                    return new Next<>(absent("Unit"), out);
                  }
                  var pair = (Data) out.data();
                  return new Next<>(pair.fields().get(0), pair.fields().get(1));
                });
  }

  /** A callback over the system state that reads an actor's state instead. */
  private static ModelCallback actorRead(ModelCallback callback)
  {
    return (symbols, args) -> {
      var full = new ArrayList<Value>(args);
      full.set(0, actorOf(args.get(0)).state());
      return callback.apply(symbols, full);
    };
  }

  /** A command the model does not allow here. */
  private static final class InvalidStep extends RuntimeException
  {
    InvalidStep()
    {
      super(null, null, false, false);
    }
  }

  private record ModelStep(int index, List<Value> args)
  {}

  private record ModelRun(List<Value> startArgs, List<ModelStep> steps)
  {}

  private record ModelFailure(int step, String message)
  {}

  private record Stepped(Value state, Value result)
  {}

  private static List<Value> withLast(List<Value> args, Value last)
  {
    var full = new ArrayList<Value>(args);
    full.add(last);
    return full;
  }

  /** The model states along a run; throws InvalidStep. */
  private static List<Value> simulate(Model model, Map<String, Object> symbols, ModelRun run)
  {
    Value state;
    try
    {
      state = model.startModel.apply(symbols, run.startArgs());
    }
    catch (Exception e)
    {
      throw new InvalidStep();
    }
    var indices = model.startIndices;
    var states = new ArrayList<Value>();
    states.add(state);
    for (var step : run.steps())
    {
      var command = model.steps.get(step.index());
      if (!command.admits(indices))
      {
        throw new InvalidStep();
      }
      state = stepModel(command, symbols, step.args(), state).state();
      indices = command.shifted(indices);
      states.add(state);
    }
    return states;
  }

  private static Stepped stepModel(
      ModelCommand command, Map<String, Object> symbols, List<Value> args, Value state)
  {
    Value out;
    try
    {
      if (command.when != null && !truth(command.when.apply(symbols, List.of(state))))
      {
        throw new InvalidStep();
      }
      out = command.reference.apply(symbols, withLast(args, state));
    }
    catch (InvalidStep e)
    {
      throw e;
    }
    catch (Exception e)
    {
      throw new InvalidStep();
    }
    if (command.unit)
    {
      return new Stepped(out, absent("Unit"));
    }
    var fields = ((Data) out.data()).fields();
    return new Stepped(fields.get(1), fields.get(0));
  }

  private static List<Value> generateAll(
      Values values, List<Object> descriptors, SplitMix64 random, long size)
  {
    var out = new ArrayList<Value>();
    for (var d : descriptors)
    {
      out.add(values.generate(d, random, size));
    }
    return out;
  }

  private static ModelRun generateRun(Model model, SplitMix64 random, long length, long size)
  {
    return generateRun(model, random, length, size, false);
  }

  private static ModelRun generateRun(
      Model model, SplitMix64 random, long length, long size, boolean crashes)
  {
    Map<String, Object> symbols = new java.util.HashMap<String, Object>();
    var startArgs = generateAll(model.values, model.startArguments, random, size);
    Value state;
    try
    {
      state = model.startModel.apply(symbols, startArgs);
    }
    catch (Exception e)
    {
      return new ModelRun(startArgs, List.of());
    }
    var indices = model.startIndices;
    var steps = new ArrayList<ModelStep>();
    for (long n = 0; n < length; n++)
    {
      var allowed = new ArrayList<Integer>();
      for (int i = 0; i < model.commands.size(); i++)
      {
        if (model.commands.get(i).admits(indices))
        {
          allowed.add(i);
        }
      }
      if (allowed.isEmpty())
      {
        break;
      }
      int index = allowed.get((int) random.below(allowed.size()));
      // One step in eight of an actor's run is a crash.
      if (crashes && model.steps.size() > model.commands.size() && random.below(8) == 0)
      {
        index = model.commands.size();
      }
      var command = model.steps.get(index);
      var args = generateAll(model.values, command.arguments, random, size);
      try
      {
        state = stepModel(command, symbols, args, state).state();
      }
      catch (InvalidStep e)
      {
        continue;
      }
      steps.add(new ModelStep(index, args));
      indices = command.shifted(indices);
    }
    return new ModelRun(startArgs, steps);
  }

  /**
   * null when the system agrees with the model along the run; otherwise the failing step's number
   * and what went wrong.
   */
  private static ModelFailure execute(Model model, ModelRun run)
  {
    Map<String, Object> symbols = new java.util.HashMap<String, Object>();
    int step = 0;
    try
    {
      var state = model.startRun.apply(symbols, run.startArgs());
      var expected = model.startModel.apply(symbols, run.startArgs());
      var failure = checkState(model, symbols, state, expected);
      if (failure != null)
      {
        return new ModelFailure(step, failure);
      }
      for (var s : run.steps())
      {
        step++;
        var command = model.steps.get(s.index());
        var full = new ArrayList<Value>(s.args());
        full.add(command.state, state);
        var out = command.run.apply(symbols, full);
        Value result;
        if (model.shared)
        {
          result = out;
        }
        else
        {
          var fields = ((Data) out.data()).fields();
          result = command.unit ? absent("Unit") : fields.get(0);
          state = fields.get(fields.size() - 1);
        }
        var stepped = stepModel(command, symbols, s.args(), expected);
        expected = stepped.state();
        var wanted = stepped.result();
        if (!command.unit && compareValues(result, wanted) != 0)
        {
          return new ModelFailure(
              step, "returned " + render(result) + "; the model returns " + render(wanted));
        }
        failure = checkState(model, symbols, state, expected);
        if (failure != null)
        {
          return new ModelFailure(step, failure);
        }
      }
    }
    catch (InvalidStep e)
    {
      return new ModelFailure(step, "the model does not allow this step");
    }
    catch (Exception e)
    {
      String message = e.getMessage();
      return new ModelFailure(
          step, "raised " + e.getClass().getSimpleName() + ": " + (message == null ? "" : message));
    }
    return null;
  }

  private static String checkState(
      Model model, Map<String, Object> symbols, Value state, Value expected)
  {
    if (model.abstractState != null)
    {
      var actual = model.abstractState.apply(symbols, List.of(state));
      if (compareValues(actual, expected) != 0)
      {
        return "the state is " + render(actual) + "; the model is " + render(expected);
      }
    }
    for (int k = 0; k < model.invariants.size(); k++)
    {
      var kind = model.invariantKinds.get(k);
      var subject = kind.equals("model") ? expected : state;
      if (!truth(model.invariants.get(k).apply(symbols, List.of(subject))))
      {
        return "an invariant on the " + kind + " fails";
      }
    }
    return null;
  }

  private static <T> List<T> replaced(List<T> xs, int at, T x)
  {
    var out = new ArrayList<T>(xs);
    out.set(at, x);
    return out;
  }

  private static List<ModelRun> shrinkCandidates(Model model, ModelRun run)
  {
    var out = new ArrayList<ModelRun>();
    var startArgs = run.startArgs();
    var steps = run.steps();
    int n = steps.size();
    for (int size = n / 2; size >= 1; size /= 2)
    {
      for (int begin = 0; begin < n; begin += size)
      {
        var kept = new ArrayList<ModelStep>(steps.subList(0, begin));
        kept.addAll(steps.subList(Math.min(begin + size, n), n));
        out.add(new ModelRun(startArgs, kept));
      }
    }
    for (int k = 0; k < n; k++)
    {
      var s = steps.get(k);
      var command = model.steps.get(s.index());
      int m = Math.min(command.arguments.size(), s.args().size());
      for (int j = 0; j < m; j++)
      {
        for (var c : model.values.shrink(command.arguments.get(j), s.args().get(j)))
        {
          out.add(
              new ModelRun(
                  startArgs,
                  replaced(steps, k, new ModelStep(s.index(), replaced(s.args(), j, c)))));
        }
      }
    }
    int m = Math.min(model.startArguments.size(), startArgs.size());
    for (int j = 0; j < m; j++)
    {
      for (var c : model.values.shrink(model.startArguments.get(j), startArgs.get(j)))
      {
        out.add(new ModelRun(replaced(startArgs, j, c), steps));
      }
    }
    return out;
  }

  private record Shrunk(ModelRun run, ModelFailure failure)
  {}

  private static Shrunk shrinkRun(Model model, ModelRun run, ModelFailure failure, int budget)
  {
    while (budget > 0)
    {
      boolean improved = false;
      for (var candidate : shrinkCandidates(model, run))
      {
        budget--;
        if (budget <= 0)
        {
          improved = true;
          break;
        }
        try
        {
          simulate(model, new java.util.HashMap<String, Object>(), candidate);
        }
        catch (InvalidStep e)
        {
          continue;
        }
        var found = execute(model, candidate);
        if (found != null)
        {
          run = candidate;
          failure = found;
          improved = true;
          break;
        }
      }
      if (!improved)
      {
        break;
      }
    }
    return new Shrunk(run, failure);
  }

  private static String renderAll(List<Value> args)
  {
    var parts = new ArrayList<String>();
    for (var a : args)
    {
      parts.add(render(a));
    }
    return String.join(", ", parts);
  }

  private static String describeRun(Model model, ModelRun run)
  {
    var parts = new ArrayList<String>();
    parts.add("start(" + renderAll(run.startArgs()) + ")");
    for (var s : run.steps())
    {
      parts.add(model.steps.get(s.index()).name + "(" + renderAll(s.args()) + ")");
    }
    return String.join("; ", parts);
  }

  /**
   * Checks the system against its model on generated runs; a failure throws AssertionError naming
   * the shortest failing run found.
   *
   * <p>A stateful model is checked by running generated command sequences against the system and
   * the model side by side, then shrinking a failing run. ref:DEC-stateful-models-linearizability
   */
  public static void checkModel(Model model)
  {
    String text = System.getenv("LAWSPEC_SEED");
    checkModel(model, 100, 20, 2000, text == null ? 0 : Long.parseUnsignedLong(text.trim()));
  }

  public static void checkModel(Model model, int cases, int maxLength, int maxShrinks, long seed)
  {
    var random = new SplitMix64(seed);
    for (int c = 0; c < cases; c++)
    {
      long length = random.below((long) maxLength + 1);
      var run = generateRun(model, random, length, 1 + c % 8, true);
      var failure = execute(model, run);
      if (failure != null)
      {
        var shrunk = shrinkRun(model, run, failure, maxShrinks);
        throw new AssertionError(
            "model "
                + model.name
                + " fails at step "
                + shrunk.failure().step()
                + " of "
                + describeRun(model, shrunk.run())
                + ": "
                + shrunk.failure().message());
      }
    }
  }

  // Parallel runs of a shared model. A case is a sequential prefix and one
  // branch per thread, generated so that the model allows every interleaving
  // of the branches (a search over each thread's position and the model
  // state, memoized). The system runs the branches at the same time, each
  // call's start and return recorded on one counter, with random yields and
  // short sleeps around calls to shake out rare schedules. The history must
  // be linearizable: some interleaving that keeps every call after those that
  // returned before it started must give every result the model gives and
  // leave the state it leaves (a Wing-Gong search, memoized on the same
  // positions and model state). Each case runs several times.

  private static final int THREADS = 3;
  private static final int BRANCH = 5;

  private record ParallelCase(ModelRun prefix, List<List<ModelStep>> branches)
  {}

  /** When a branch's call started and returned, and what it returned. */
  private record Call(long called, long returned, Value result)
  {}

  private static String searchKey(int[] positions, Value state)
  {
    return Arrays.toString(positions) + "|" + render(state);
  }

  private static int[] advanced(int[] positions, int i)
  {
    var next = positions.clone();
    next[i]++;
    return next;
  }

  /** Whether the model allows the prefix then every interleaving. */
  private static boolean parallelAllowed(
      Model model, ModelRun prefix, List<List<ModelStep>> branches)
  {
    Map<String, Object> symbols = new java.util.HashMap<String, Object>();
    Value state;
    try
    {
      var states = simulate(model, symbols, prefix);
      state = states.get(states.size() - 1);
    }
    catch (InvalidStep e)
    {
      return false;
    }
    return allowedFrom(
        model, symbols, branches, new int[branches.size()], state, new java.util.HashSet<String>());
  }

  private static boolean allowedFrom(
      Model model,
      Map<String, Object> symbols,
      List<List<ModelStep>> branches,
      int[] positions,
      Value state,
      java.util.Set<String> seen)
  {
    if (!seen.add(searchKey(positions, state)))
    {
      return true;
    }
    for (int i = 0; i < branches.size(); i++)
    {
      var branch = branches.get(i);
      int k = positions[i];
      if (k < branch.size())
      {
        var s = branch.get(k);
        Value after;
        try
        {
          after = stepModel(model.commands.get(s.index()), symbols, s.args(), state).state();
        }
        catch (InvalidStep e)
        {
          return false;
        }
        if (!allowedFrom(model, symbols, branches, advanced(positions, i), after, seen))
        {
          return false;
        }
      }
    }
    return true;
  }

  private static List<ModelStep> generateBranch(
      Model model, SplitMix64 random, Value state, long length, long size)
  {
    Map<String, Object> symbols = new java.util.HashMap<String, Object>();
    var steps = new ArrayList<ModelStep>();
    for (long n = 0; n < length; n++)
    {
      int index = (int) random.below(model.commands.size());
      var command = model.commands.get(index);
      var args = generateAll(model.values, command.arguments, random, size);
      try
      {
        state = stepModel(command, symbols, args, state).state();
      }
      catch (InvalidStep e)
      {
        continue;
      }
      steps.add(new ModelStep(index, args));
    }
    return steps;
  }

  private static ParallelCase generateParallel(
      Model model, SplitMix64 random, long size, int threads, int branchLength)
  {
    var prefix = generateRun(model, random, random.below(4), size);
    Value state;
    try
    {
      var states = simulate(model, new java.util.HashMap<String, Object>(), prefix);
      state = states.get(states.size() - 1);
    }
    catch (InvalidStep e)
    {
      var empty = new ArrayList<List<ModelStep>>();
      for (int i = 0; i < threads; i++)
      {
        empty.add(List.of());
      }
      return new ParallelCase(prefix, empty);
    }
    var branches = new ArrayList<List<ModelStep>>();
    for (int i = 0; i < threads; i++)
    {
      branches.add(generateBranch(model, random, state, 1 + random.below(branchLength), size));
    }
    // Drop the last step of the longest branch (the first, among equals)
    // until every interleaving is allowed.
    while (!parallelAllowed(model, prefix, branches))
    {
      int longest = 0;
      for (int i = 1; i < threads; i++)
      {
        if (branches.get(i).size() > branches.get(longest).size())
        {
          longest = i;
        }
      }
      var b = branches.get(longest);
      branches.set(longest, b.subList(0, Math.max(0, b.size() - 1)));
    }
    return new ParallelCase(prefix, branches);
  }

  /** Nothing, a yield, or a sleep of 10 or 100 microseconds. */
  private static void perturb(SplitMix64 random)
  {
    long choice = random.below(4);
    if (choice == 1)
    {
      Thread.yield();
    }
    else if (choice >= 2)
    {
      java.util.concurrent.locks.LockSupport.parkNanos(choice == 2 ? 10_000L : 100_000L);
    }
  }

  private static String raised(Exception e)
  {
    String message = e.getMessage();
    return "raised " + e.getClass().getSimpleName() + ": " + (message == null ? "" : message);
  }

  private static String branchName(int i)
  {
    return String.valueOf((char) ('A' + i));
  }

  /** null when the history is linearizable; otherwise what went wrong. */
  private static String executeParallel(Model model, ParallelCase c, long shake)
  {
    var prefix = c.prefix();
    var branches = c.branches();
    Map<String, Object> symbols = new java.util.HashMap<String, Object>();
    Value started;
    try
    {
      started = model.startRun.apply(symbols, prefix.startArgs());
      for (var s : prefix.steps())
      {
        var command = model.commands.get(s.index());
        var full = new ArrayList<Value>(s.args());
        full.add(command.state, started);
        command.run.apply(symbols, full);
      }
    }
    catch (Exception e)
    {
      return "the prefix " + raised(e);
    }
    final Value state = started;
    var clock = new java.util.concurrent.atomic.AtomicLong();
    var history = new ArrayList<Call[]>();
    for (var b : branches)
    {
      history.add(new Call[b.size()]);
    }
    var errors = Collections.synchronizedList(new ArrayList<String>());
    var ready = new java.util.concurrent.CountDownLatch(1);
    var threads = new ArrayList<Thread>();
    for (int i = 0; i < branches.size(); i++)
    {
      final int branch = i;
      final var random = new SplitMix64(shake ^ ((long) (i + 1) * 0x9E3779B97F4A7C15L));
      threads.add(
          new Thread(
              () -> {
                Map<String, Object> own = new java.util.HashMap<String, Object>();
                try
                {
                  ready.await();
                }
                catch (InterruptedException e)
                {
                  Thread.currentThread().interrupt();
                }
                var steps = branches.get(branch);
                for (int k = 0; k < steps.size(); k++)
                {
                  var s = steps.get(k);
                  var command = model.commands.get(s.index());
                  var full = new ArrayList<Value>(s.args());
                  full.add(command.state, state);
                  perturb(random);
                  long called = clock.incrementAndGet();
                  Value result;
                  try
                  {
                    result = command.run.apply(own, full);
                  }
                  catch (Exception e)
                  {
                    errors.add(command.name + " " + raised(e));
                    result = null;
                  }
                  history.get(branch)[k] = new Call(called, clock.incrementAndGet(), result);
                  perturb(random);
                }
              }));
    }
    for (var t : threads)
    {
      t.start();
    }
    ready.countDown();
    for (var t : threads)
    {
      boolean interrupted = false;
      while (true)
      {
        try
        {
          t.join();
          break;
        }
        catch (InterruptedException e)
        {
          interrupted = true;
        }
      }
      if (interrupted)
      {
        Thread.currentThread().interrupt();
      }
    }
    if (!errors.isEmpty())
    {
      return errors.get(0);
    }
    var states = simulate(model, symbols, prefix);
    var expected = states.get(states.size() - 1);
    var finalState =
        model.abstractState != null ? model.abstractState.apply(symbols, List.of(state)) : null;
    if (linearizable(model, symbols, branches, history, expected, finalState, state))
    {
      return null;
    }
    var observed = new ArrayList<String>();
    for (int i = 0; i < branches.size(); i++)
    {
      for (int k = 0; k < branches.get(i).size(); k++)
      {
        observed.add(
            branchName(i)
                + ": "
                + model.commands.get(branches.get(i).get(k).index()).name
                + "() returned "
                + render(history.get(i)[k].result()));
      }
    }
    return "no order of the parallel calls agrees with the model ("
        + String.join("; ", observed)
        + ")";
  }

  /**
   * Whether the history linearizes, with the final state and invariants the model gives. For a set
   * or map whose every call touches one key, each key's calls are linearized separately (the keys
   * are independent), one group after another; otherwise all calls at once.
   */
  private static boolean linearizable(
      Model model,
      Map<String, Object> symbols,
      List<List<ModelStep>> branches,
      List<Call[]> history,
      Value expected,
      Value finalState,
      Value state)
  {
    java.util.function.Predicate<Value> finish =
        modelState -> {
          if (finalState != null && compareValues(finalState, modelState) != 0)
          {
            return false;
          }
          for (int k = 0; k < model.invariants.size(); k++)
          {
            var subject = model.invariantKinds.get(k).equals("model") ? modelState : state;
            if (!truth(model.invariants.get(k).apply(symbols, List.of(subject))))
            {
              return false;
            }
          }
          return true;
        };
    // Threads that never message each other see only their own calls, so
    // causal consistency checks each thread's results alone.
    if (model.consistency.equals("causal"))
    {
      for (int i = 0; i < branches.size(); i++)
      {
        var modelState = expected;
        for (int k = 0; k < branches.get(i).size(); k++)
        {
          var s = branches.get(i).get(k);
          var command = model.commands.get(s.index());
          Stepped stepped;
          try
          {
            stepped = stepModel(command, symbols, s.args(), modelState);
          }
          catch (InvalidStep e)
          {
            return false;
          }
          if (!command.unit && compareValues(history.get(i)[k].result(), stepped.result()) != 0)
          {
            return false;
          }
          modelState = stepped.state();
        }
      }
      return true;
    }
    if (!model.perKey)
    {
      return linearize(
          model,
          symbols,
          branches,
          history,
          new int[branches.size()],
          expected,
          finish,
          new java.util.HashSet<String>());
    }
    // Each key's calls, branch by branch, in order of the rendered key.
    var stepGroups = new java.util.TreeMap<String, List<List<ModelStep>>>();
    var callGroups = new java.util.HashMap<String, List<List<Call>>>();
    for (int i = 0; i < branches.size(); i++)
    {
      var branch = branches.get(i);
      for (int k = 0; k < branch.size(); k++)
      {
        var s = branch.get(k);
        String key = render(s.args().get(model.commands.get(s.index()).key));
        if (!stepGroups.containsKey(key))
        {
          var steps = new ArrayList<List<ModelStep>>();
          var calls = new ArrayList<List<Call>>();
          for (int j = 0; j < branches.size(); j++)
          {
            steps.add(new ArrayList<ModelStep>());
            calls.add(new ArrayList<Call>());
          }
          stepGroups.put(key, steps);
          callGroups.put(key, calls);
        }
        stepGroups.get(key).get(i).add(s);
        callGroups.get(key).get(i).add(history.get(i)[k]);
      }
    }
    Value modelState = expected;
    for (var entry : stepGroups.entrySet())
    {
      var parts = entry.getValue();
      var calls = new ArrayList<Call[]>();
      for (var c : callGroups.get(entry.getKey()))
      {
        calls.add(c.toArray(new Call[0]));
      }
      var ends = new ArrayList<Value>();
      if (!linearize(
          model,
          symbols,
          parts,
          calls,
          new int[parts.size()],
          modelState,
          end -> {
            ends.add(end);
            return true;
          },
          new java.util.HashSet<String>()))
      {
        return false;
      }
      modelState = ends.get(0);
    }
    return finish.test(modelState);
  }

  /**
   * A Wing-Gong search: linearize, next, a call no pending call on another thread returned before;
   * memoized on positions and the model state. finish judges each complete order's final model
   * state.
   */
  private static boolean linearize(
      Model model,
      Map<String, Object> symbols,
      List<List<ModelStep>> branches,
      List<Call[]> history,
      int[] positions,
      Value modelState,
      java.util.function.Predicate<Value> finish,
      java.util.Set<String> seen)
  {
    if (!seen.add(searchKey(positions, modelState)))
    {
      return false;
    }
    boolean done = true;
    for (int i = 0; i < branches.size(); i++)
    {
      if (positions[i] != branches.get(i).size())
      {
        done = false;
      }
    }
    if (done)
    {
      return finish.test(modelState);
    }
    for (int i = 0; i < branches.size(); i++)
    {
      var branch = branches.get(i);
      int k = positions[i];
      if (k == branch.size())
      {
        continue;
      }
      long called = history.get(i)[k].called();
      boolean blocked = false;
      // Sequential and eventual consistency drop real time; each thread's
      // own order remains.
      if (model.consistency.equals("linearizable"))
      {
        for (int j = 0; j < branches.size(); j++)
        {
          if (j != i
              && positions[j] < branches.get(j).size()
              && history.get(j)[positions[j]].returned() < called)
          {
            blocked = true;
          }
        }
      }
      if (blocked)
      {
        continue;
      }
      var s = branch.get(k);
      var command = model.commands.get(s.index());
      Stepped stepped;
      try
      {
        stepped = stepModel(command, symbols, s.args(), modelState);
      }
      catch (InvalidStep e)
      {
        continue;
      }
      if (!model.consistency.equals("eventual")
          && !command.unit
          && compareValues(history.get(i)[k].result(), stepped.result()) != 0)
      {
        continue;
      }
      if (linearize(
          model, symbols, branches, history, advanced(positions, i), stepped.state(), finish, seen))
      {
        return true;
      }
    }
    return false;
  }

  private static String consistent(String consistency)
  {
    return switch (consistency)
    {
      case "sequential" -> "sequentially consistent";
      case "causal" -> "causally consistent";
      case "eventual" -> "eventually consistent";
      default -> "linearizable";
    };
  }

  private static String parallelFails(Model model, ParallelCase c, int repeats, long shake)
  {
    for (int attempt = 0; attempt < repeats; attempt++)
    {
      var failure = executeParallel(model, c, shake + attempt);
      if (failure != null)
      {
        return failure;
      }
    }
    return null;
  }

  private static <T> List<T> without(List<T> xs, int at)
  {
    var out = new ArrayList<T>(xs.subList(0, at));
    out.addAll(xs.subList(at + 1, xs.size()));
    return out;
  }

  private record ShrunkParallel(ParallelCase c, String failure)
  {}

  private static ShrunkParallel shrinkParallel(
      Model model, ParallelCase c, String failure, int repeats, int budget, long shake)
  {
    while (budget > 0)
    {
      var prefix = c.prefix();
      var branches = c.branches();
      var candidates = new ArrayList<ParallelCase>();
      var steps = prefix.steps();
      for (int k = 0; k < steps.size(); k++)
      {
        candidates.add(
            new ParallelCase(new ModelRun(prefix.startArgs(), without(steps, k)), branches));
      }
      for (int i = 0; i < branches.size(); i++)
      {
        for (int k = 0; k < branches.get(i).size(); k++)
        {
          var shorter = new ArrayList<List<ModelStep>>(branches);
          shorter.set(i, without(branches.get(i), k));
          candidates.add(new ParallelCase(prefix, shorter));
        }
      }
      // Then smaller arguments, branch by branch, step by step.
      for (int i = 0; i < branches.size(); i++)
      {
        for (int k = 0; k < branches.get(i).size(); k++)
        {
          var s = branches.get(i).get(k);
          var command = model.commands.get(s.index());
          int m = Math.min(command.arguments.size(), s.args().size());
          for (int a = 0; a < m; a++)
          {
            for (var smaller : model.values.shrink(command.arguments.get(a), s.args().get(a)))
            {
              var changed = new ArrayList<List<ModelStep>>(branches);
              var step = new ModelStep(s.index(), replaced(s.args(), a, smaller));
              changed.set(i, replaced(branches.get(i), k, step));
              candidates.add(new ParallelCase(prefix, changed));
            }
          }
        }
      }
      boolean improved = false;
      for (var candidate : candidates)
      {
        budget--;
        if (budget <= 0)
        {
          improved = true;
          break;
        }
        if (!parallelAllowed(model, candidate.prefix(), candidate.branches()))
        {
          continue;
        }
        var found = parallelFails(model, candidate, repeats, shake);
        if (found != null)
        {
          c = candidate;
          failure = found;
          improved = true;
          break;
        }
      }
      if (!improved)
      {
        break;
      }
    }
    return new ShrunkParallel(c, failure);
  }

  private static String describeBranch(Model model, List<ModelStep> steps)
  {
    var parts = new ArrayList<String>();
    for (var s : steps)
    {
      parts.add(model.commands.get(s.index()).name + "(" + renderAll(s.args()) + ")");
    }
    return parts.isEmpty() ? "nothing" : String.join("; ", parts);
  }

  private static String describeParallel(Model model, ParallelCase c)
  {
    var parts = new ArrayList<String>();
    var branches = c.branches();
    for (int i = 0; i < branches.size(); i++)
    {
      parts.add(branchName(i) + ": " + describeBranch(model, branches.get(i)));
    }
    return describeRun(model, c.prefix())
        + ", then "
        + String.join(", ", parts.subList(0, parts.size() - 1))
        + " and "
        + parts.get(parts.size() - 1)
        + " at the same time";
  }

  /**
   * Checks a shared model's histories under concurrency; a failure throws AssertionError naming the
   * smallest failing case found.
   *
   * <p>A shared model promises linearizability unless it names a weaker consistency, so concurrent
   * histories are judged against some sequential order of the calls, found by a Wing-Gong search.
   * ref:herlihy-wing-linearizability ref:wing-gong-linearizability
   * ref:DEC-stateful-models-linearizability
   */
  public static void checkModelParallel(Model model)
  {
    String text = System.getenv("LAWSPEC_SEED");
    checkModelParallel(
        model,
        50,
        10,
        300,
        text == null ? 0 : Long.parseUnsignedLong(text.trim()),
        THREADS,
        BRANCH);
  }

  public static void checkModelParallel(
      Model model, int cases, int repeats, int maxShrinks, long seed)
  {
    checkModelParallel(model, cases, repeats, maxShrinks, seed, THREADS, BRANCH);
  }

  public static void checkModelParallel(
      Model model,
      int cases,
      int repeats,
      int maxShrinks,
      long seed,
      int threads,
      int branchLength)
  {
    var random = new SplitMix64(seed ^ 0x5BD1E995L);
    for (int n = 0; n < cases; n++)
    {
      var c = generateParallel(model, random, 1 + n % 8, threads, branchLength);
      long shake = random.next();
      var failure = parallelFails(model, c, repeats, shake);
      if (failure != null)
      {
        var shrunk = shrinkParallel(model, c, failure, Math.max(2, repeats / 2), maxShrinks, shake);
        throw new AssertionError(
            "model "
                + model.name
                + " is not "
                + consistent(model.consistency)
                + ": "
                + describeParallel(model, shrunk.c())
                + ": "
                + shrunk.failure());
      }
    }
  }

  // Scenarios: processes that drive a shared model's commands at the same time
  // and talk over channels (see LawSpec.Core.Program for the spec). Each
  // channel has a queue per direction; a process holds an end of a channel as
  // (channel, side), the first branch of a par to use a channel taking side 0.
  // A channel end sent over a channel moves to the receiver. Every command's
  // call and return are stamped on one counter; the history must linearize
  // against the model, and every expect must hold, on each of many schedules.

  /** A scenario channel: in memory, or with each side on a node of a network. */
  private interface ScenarioLink
  {
    void send(int side, Object value);

    /** The next value for side, SCENARIO_GONE once the other side has ended, null on timeout. */
    Object receive(int side);

    void gone(int side);

    default void close()
    {}
  }

  private static final class ScenarioChannel implements ScenarioLink
  {
    public Object receive(int side)
    {
      try
      {
        var value = queues[1 - side].poll(5, java.util.concurrent.TimeUnit.SECONDS);
        if (value == SCENARIO_GONE)
        {
          queues[1 - side].add(SCENARIO_GONE);
        }
        return value;
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        return null;
      }
    }

    @SuppressWarnings("unchecked")
    final java.util.concurrent.LinkedBlockingQueue<Object>[] queues =
        new java.util.concurrent.LinkedBlockingQueue[] {
          new java.util.concurrent.LinkedBlockingQueue<Object>(),
          new java.util.concurrent.LinkedBlockingQueue<Object>()
        };

    final boolean[] ended = new boolean[2];

    /** A channel end sent to a process that has ended is given up. */
    public void send(int side, Object value)
    {
      synchronized (this)
      {
        if (!(ended[1 - side] && value instanceof ScenarioEnd))
        {
          queues[side].add(value);
          return;
        }
      }
      var end = (ScenarioEnd) value;
      end.channel().gone(end.side());
    }

    /**
     * side's process has ended: the other side's receives that find nothing more fail instead of
     * waiting, and channel ends on their way to side are given up too.
     */
    public void gone(int side)
    {
      var stranded = new ArrayList<ScenarioEnd>();
      synchronized (this)
      {
        if (ended[side])
        {
          return;
        }
        ended[side] = true;
        queues[side].add(SCENARIO_GONE);
        while (true)
        {
          var value = queues[1 - side].poll();
          if (value == null)
          {
            break;
          }
          if (value instanceof ScenarioEnd end)
          {
            stranded.add(end);
          }
          else if (value == SCENARIO_GONE)
          {
            queues[1 - side].add(value);
            break;
          }
        }
      }
      for (var end : stranded)
      {
        end.channel().gone(end.side());
      }
    }
  }

  private static final Object SCENARIO_GONE = new Object();

  /**
   * A scenario's mailbox: any process sends, one receives. expected is how many sends the scenario
   * makes; a process that ends gives up the sends it did not make, and a receive with nothing left
   * to come fails (SCENARIO_GONE) instead of waiting. Over a network, messages go from a sender
   * node to the receiver's node, each send waiting until it is delivered.
   */
  private static final class ScenarioMailbox
  {
    final String name;
    final int expected;
    int received;
    int abandoned;
    final java.util.ArrayDeque<Object[]> items = new java.util.ArrayDeque<>();
    final Map<String, NetScenarioChannel> registry;
    final List<Node> nodes = new ArrayList<>();
    final Mailbox<Value> inbox;
    final RemoteMailbox remote;

    ScenarioMailbox(String name, int expected)
    {
      this.name = name;
      this.expected = expected;
      this.registry = null;
      this.inbox = null;
      this.remote = null;
    }

    ScenarioMailbox(
        String name,
        int expected,
        MemoryNetwork network,
        Object descriptor,
        Values values,
        Map<String, NetScenarioChannel> registry)
    {
      this.name = name;
      this.expected = expected;
      this.registry = registry;
      // Test machinery: the insecure transport, so scenarios need no crypto.
      var owner = new Node(network.insecureTransportForTests(name + "-owner"));
      var senders = new Node(network.insecureTransportForTests(name + "-senders"));
      nodes.add(owner);
      nodes.add(senders);
      Object d = atomText(form(descriptor).get(0)).equals("end") ? List.of("text") : descriptor;
      inbox = owner.mailbox(name, d, values);
      remote = senders.remoteMailbox(owner.address + "/" + name, d, values, 5.0);
    }

    void send(Object value, Map<String, Long> clock)
    {
      if (inbox == null)
      {
        synchronized (this)
        {
          items.add(new Object[] {value, clock});
          notifyAll();
        }
        return;
      }
      if (value instanceof ScenarioEnd end)
      {
        value = textValue(((NetScenarioChannel) end.channel()).name + "#" + end.side());
      }
      // The clock travels beside the network, in send order.
      synchronized (this)
      {
        items.add(new Object[] {null, clock});
      }
      remote.send((Value) value);
      synchronized (this)
      {
        notifyAll();
      }
    }

    synchronized void giveUp(int count)
    {
      abandoned += count;
      notifyAll();
    }

    /** {value, sender's clock}, {SCENARIO_GONE, empty}, or null after waiting too long. */
    @SuppressWarnings("unchecked")
    Object[] receive()
    {
      long giveUp = System.nanoTime() + 5_000_000_000L;
      while (true)
      {
        synchronized (this)
        {
          if (inbox == null && !items.isEmpty())
          {
            received++;
            return items.poll();
          }
          if (received + abandoned >= expected && (inbox == null || items.isEmpty()))
          {
            return new Object[] {SCENARIO_GONE, Map.of()};
          }
        }
        if (inbox != null)
        {
          Value value;
          try
          {
            value = inbox.receive(java.time.Duration.ofMillis(20));
          }
          catch (IllegalStateException e)
          {
            if (System.nanoTime() > giveUp)
            {
              return null;
            }
            continue;
          }
          Map<String, Long> carried;
          synchronized (this)
          {
            received++;
            var first = items.poll();
            carried = first == null ? Map.of() : (Map<String, Long>) first[1];
          }
          Object result = value;
          if (value.type().equals("Text"))
          {
            String text = textOf(value);
            int cut = text.lastIndexOf('#');
            if (cut > 0 && registry.containsKey(text.substring(0, cut)))
            {
              result =
                  new ScenarioEnd(
                      registry.get(text.substring(0, cut)),
                      Integer.parseInt(text.substring(cut + 1)));
            }
          }
          return new Object[] {result, carried};
        }
        synchronized (this)
        {
          long left = giveUp - System.nanoTime();
          if (left <= 0)
          {
            return null;
          }
          try
          {
            wait(Math.max(1, left / 1_000_000));
          }
          catch (InterruptedException e)
          {
            Thread.currentThread().interrupt();
            return null;
          }
        }
      }
    }

    void close()
    {
      for (var n : nodes)
      {
        n.close();
      }
    }
  }

  /** How many times these acts (not nested pars) send to name. */
  private static int scenarioSends(List<Object> acts, String name)
  {
    int count = 0;
    for (var a : acts)
    {
      var act = form(a);
      if (atomText(act.get(0)).equals("send") && atomText(act.get(1)).equals(name))
      {
        count++;
      }
    }
    return count;
  }

  /** How many sends to name the whole program makes. */
  private static int allSends(List<Object> acts, String name)
  {
    int total = 0;
    for (var a : acts)
    {
      var act = form(a);
      String head = atomText(act.get(0));
      if (head.equals("send") && atomText(act.get(1)).equals(name))
      {
        total++;
      }
      else if (head.equals("par"))
      {
        for (var branch : act.subList(1, act.size()))
        {
          var b = form(branch);
          total += allSends(b.subList(1, b.size()), name);
        }
      }
    }
    return total;
  }

  /** Every process of a par, outermost and first first (not or else), by identity. */
  private static List<Object> scenarioProcesses(List<Object> acts, List<Object> found)
  {
    for (var a : acts)
    {
      var act = form(a);
      if (atomText(act.get(0)).equals("par"))
      {
        for (var branch : act.subList(1, act.size()))
        {
          found.add(branch);
          var b = form(branch);
          scenarioProcesses(b.subList(1, b.size()), found);
        }
      }
    }
    return found;
  }

  /** A channel end in transit or held by a process. */
  private record ScenarioEnd(ScenarioLink channel, int side)
  {}

  /**
   * A scenario channel whose two sides are endpoints on two nodes of a faulty in-memory network. A
   * channel end sent over it travels as its name, and the receiver uses the end where it is.
   */
  private static final class NetScenarioChannel implements ScenarioLink
  {
    final String name;
    final Map<String, NetScenarioChannel> registry;
    final Node[] nodes = new Node[2];
    final NetEndpoint[] ends = new NetEndpoint[2];
    final boolean[] done = new boolean[2];

    NetScenarioChannel(
        MemoryNetwork network,
        String name,
        List<Step> steps,
        Values values,
        Map<String, NetScenarioChannel> registry)
    {
      this.name = name;
      this.registry = registry;
      // Test machinery: the insecure transport, so scenarios need no crypto.
      for (int side = 0; side < 2; side++)
      {
        nodes[side] = new Node(network.insecureTransportForTests(name + "-" + side));
      }
      var wired = new ArrayList<Step>();
      var flipped = new ArrayList<Step>();
      for (var s : steps)
      {
        var d = form(s.descriptor());
        Object descriptor = atomText(d.get(0)).equals("end") ? List.of("text") : s.descriptor();
        wired.add(new Step(s.sends(), descriptor));
        flipped.add(new Step(!s.sends(), descriptor));
      }
      ends[0] = nodes[0].listen(name, wired, values);
      ends[1] = nodes[1].dial(nodes[0].address + "/" + name, flipped, values);
      registry.put(name, this);
    }

    public void send(int side, Object value)
    {
      if (value instanceof ScenarioEnd end)
      {
        value = textValue(((NetScenarioChannel) end.channel()).name + "#" + end.side());
      }
      ends[side].send(side, value);
    }

    public Object receive(int side)
    {
      Value value;
      try
      {
        value = (Value) ends[side].receive(side, 5);
      }
      catch (PeerFailed e)
      {
        return SCENARIO_GONE;
      }
      catch (IllegalStateException e)
      {
        return null;
      }
      if (value.type().equals("Text"))
      {
        String text = textOf(value);
        int cut = text.lastIndexOf('#');
        if (cut > 0 && registry.containsKey(text.substring(0, cut)))
        {
          return new ScenarioEnd(
              registry.get(text.substring(0, cut)), Integer.parseInt(text.substring(cut + 1)));
        }
      }
      return value;
    }

    public synchronized void gone(int side)
    {
      if (done[side])
      {
        return;
      }
      done[side] = true;
      ends[side].abandon(side);
    }

    public void close()
    {
      for (var n : nodes)
      {
        n.close();
      }
    }
  }

  private record ScenarioCall(
      ModelCommand command,
      List<Value> args,
      Value result,
      long called,
      long returned,
      String process,
      Map<String, Long> atCall,
      Map<String, Long> atReturn)
  {}

  private record StampKey(ScenarioLink channel, int side)
  {}

  private static final class ScenarioRun
  {
    final Model model;
    final Map<String, ScenarioLink> channels = new java.util.HashMap<>();
    final Map<String, ScenarioMailbox> mailboxes = new java.util.HashMap<>();

    /** Each process's name for its clock: root, or its branch's place among all processes. */
    final java.util.IdentityHashMap<Object, String> processNames =
        new java.util.IdentityHashMap<>();

    /**
     * Vector clocks: each value sent carries its sender's clock (kept here, in order per channel
     * direction), so calls can be ordered by what happened before what.
     */
    final Map<StampKey, java.util.ArrayDeque<Map<String, Long>>> stamps = new java.util.HashMap<>();

    final Map<String, ModelCommand> commands = new java.util.HashMap<>();
    final java.util.concurrent.atomic.AtomicLong clock =
        new java.util.concurrent.atomic.AtomicLong();
    final List<ScenarioCall> history = Collections.synchronizedList(new ArrayList<ScenarioCall>());
    final List<String> failures = new java.util.concurrent.CopyOnWriteArrayList<String>();
    final long shake;
    Value state;

    /** The crashed process (a par's branch, by identity) and the act it crashes before. */
    Object victim;

    long victimAct = -1;

    ScenarioRun(Model model, long shake)
    {
      this.model = model;
      this.shake = shake;
    }
  }

  /** The names an act list sends, receives or sends away, with nested pars. */
  private static List<String> actsChannels(List<Object> acts)
  {
    var names = new ArrayList<String>();
    for (var a : acts)
    {
      var act = form(a);
      switch (atomText(act.get(0)))
      {
        case "send" -> {
          names.add(atomText(act.get(1)));
          var operand = form(act.get(2));
          if (atomText(operand.get(0)).equals("var"))
          {
            names.add(atomText(operand.get(1)));
          }
        }
        case "receive" -> names.add(atomText(act.get(1)));
        case "receiveor" -> {
          names.add(atomText(act.get(1)));
          var handler = form(act.get(3));
          names.addAll(actsChannels(handler.subList(1, handler.size())));
        }
        case "par" -> {
          for (var branch : act.subList(1, act.size()))
          {
            var b = form(branch);
            names.addAll(actsChannels(b.subList(1, b.size())));
          }
        }
        default -> {}
      }
    }
    return names;
  }

  private static Value scenarioConstant(List<Object> form)
  {
    return switch (atomText(form.get(0)))
    {
      case "int" -> new Value("Integer", (BigInteger) form.get(1));
      case "text" -> sequence("Text", atomText(form.get(1)).codePoints().toArray());
      case "bool" -> bool(atomText(form.get(1)).equals("true"));
      default -> {
        String tag = atomText(form.get(1));
        yield new Value(tag, new Data(tag, List.of()));
      }
    };
  }

  private static Value scenarioOperand(Object o, Map<String, Value> env)
  {
    var operand = form(o);
    if (atomText(operand.get(0)).equals("var"))
    {
      String name = atomText(operand.get(1));
      if (!env.containsKey(name))
      {
        throw new IllegalStateException("unbound variable " + name);
      }
      return env.get(name);
    }
    return scenarioConstant(operand);
  }

  /**
   * true when the process finished, false when it failed; either way, the ends it still holds are
   * given up.
   */
  private static boolean scenarioProcess(
      ScenarioRun run,
      List<Object> acts,
      Map<String, Value> env,
      Map<String, ScenarioEnd> ends,
      SplitMix64 random,
      Object identity,
      Map<String, Long> clock)
  {
    String me = identity == null ? "root" : run.processNames.get(identity);
    var sent = new java.util.HashMap<String, Integer>();
    try
    {
      return scenarioSteps(run, acts, env, ends, random, identity, clock, me, sent);
    }
    finally
    {
      for (var end : new ArrayList<ScenarioEnd>(ends.values()))
      {
        end.channel().gone(end.side());
      }
      // Sends this process will never make.
      for (var box : run.mailboxes.values())
      {
        int missing = scenarioSends(acts, box.name) - sent.getOrDefault(box.name, 0);
        if (missing > 0)
        {
          box.giveUp(missing);
        }
      }
    }
  }

  private static boolean scenarioSteps(
      ScenarioRun run,
      List<Object> acts,
      Map<String, Value> env,
      Map<String, ScenarioEnd> ends,
      SplitMix64 random,
      Object identity,
      Map<String, Long> clock,
      String me,
      Map<String, Integer> sent)
  {
    Map<String, Object> own = new java.util.HashMap<String, Object>();
    for (int index = 0; index < acts.size(); index++)
    {
      if (!run.failures.isEmpty())
      {
        return false;
      }
      if (run.victim != null && run.victim == identity && run.victimAct == index)
      {
        return false;
      }
      var act = form(acts.get(index));
      String kind = atomText(act.get(0));
      switch (kind)
      {
        case "call" -> {
          var command = run.commands.get(atomText(act.get(1)));
          var args = new ArrayList<Value>();
          var operands = act.subList(3, act.size());
          for (int j = 0; j < operands.size(); j++)
          {
            var value = scenarioOperand(operands.get(j), env);
            // An integer constant takes the argument's integer type.
            if (value.type().equals("Integer") && j < command.arguments.size())
            {
              var d = form(command.arguments.get(j));
              if (atomText(d.get(0)).equals("int"))
              {
                value = new Value(run.model.values.typeName(d), value.data());
              }
            }
            args.add(value);
          }
          var full = new ArrayList<Value>(args);
          full.add(command.state, run.state);
          perturb(random);
          clock.merge(me, 1L, Long::sum);
          var atCall = new java.util.HashMap<String, Long>(clock);
          long called = run.clock.incrementAndGet();
          Value result;
          try
          {
            result = command.run.apply(own, full);
          }
          catch (Exception e)
          {
            run.failures.add(command.name + " " + raised(e));
            return false;
          }
          long returned = run.clock.incrementAndGet();
          clock.merge(me, 1L, Long::sum);
          run.history.add(
              new ScenarioCall(
                  command,
                  args,
                  result,
                  called,
                  returned,
                  me,
                  atCall,
                  new java.util.HashMap<String, Long>(clock)));
          if (act.get(2) != null)
          {
            env.put(atomText(act.get(2)), result);
          }
        }
        case "send" -> {
          if (run.mailboxes.containsKey(atomText(act.get(1))))
          {
            String boxName = atomText(act.get(1));
            var operand = form(act.get(2));
            Object value;
            if (atomText(operand.get(0)).equals("var")
                && ends.containsKey(atomText(operand.get(1))))
            {
              value = ends.remove(atomText(operand.get(1)));
            }
            else
            {
              value = scenarioOperand(operand, env);
            }
            perturb(random);
            clock.merge(me, 1L, Long::sum);
            try
            {
              run.mailboxes.get(boxName).send(value, new java.util.HashMap<String, Long>(clock));
            }
            catch (Unreachable e)
            {
              run.failures.add("a send to mailbox " + boxName + " failed: " + e.getMessage());
              return false;
            }
            sent.merge(boxName, 1, Integer::sum);
            continue;
          }
          var end = ends.get(atomText(act.get(1)));
          var operand = form(act.get(2));
          Object value;
          if (atomText(operand.get(0)).equals("var") && ends.containsKey(atomText(operand.get(1))))
          {
            value = ends.remove(atomText(operand.get(1)));
          }
          else
          {
            value = scenarioOperand(operand, env);
          }
          perturb(random);
          clock.merge(me, 1L, Long::sum);
          synchronized (run.stamps)
          {
            run.stamps
                .computeIfAbsent(
                    new StampKey(end.channel(), end.side()), k -> new java.util.ArrayDeque<>())
                .add(new java.util.HashMap<String, Long>(clock));
          }
          end.channel().send(end.side(), value);
        }
        case "receive", "receiveor" -> {
          String name = atomText(act.get(1));
          if (run.mailboxes.containsKey(name))
          {
            var got = run.mailboxes.get(name).receive();
            if (got == null)
            {
              run.failures.add(
                  "a receive on mailbox " + name + " waited too long: the processes are blocked");
              return false;
            }
            if (got[0] == SCENARIO_GONE)
            {
              if (kind.equals("receive"))
              {
                return false;
              }
              var handler = form(act.get(3));
              return scenarioSteps(
                  run,
                  handler.subList(1, handler.size()),
                  env,
                  ends,
                  random,
                  null,
                  clock,
                  me,
                  sent);
            }
            @SuppressWarnings("unchecked")
            var from = (Map<String, Long>) got[1];
            for (var e : from.entrySet())
            {
              clock.merge(e.getKey(), e.getValue(), Math::max);
            }
            clock.merge(me, 1L, Long::sum);
            if (got[0] instanceof ScenarioEnd received)
            {
              ends.put(atomText(act.get(2)), received);
            }
            else
            {
              env.put(atomText(act.get(2)), (Value) got[0]);
            }
            continue;
          }
          var end = ends.get(name);
          Object value = end.channel().receive(end.side());
          if (value == null)
          {
            run.failures.add(
                "a receive on " + name + " waited too long: the processes are blocked");
            return false;
          }
          if (value == SCENARIO_GONE)
          {
            // The other process ended: or else runs instead of the rest;
            // without it, this process fails too.
            if (kind.equals("receive"))
            {
              return false;
            }
            ends.remove(name);
            var handler = form(act.get(3));
            return scenarioSteps(
                run, handler.subList(1, handler.size()), env, ends, random, null, clock, me, sent);
          }
          Map<String, Long> carried;
          synchronized (run.stamps)
          {
            var queue = run.stamps.get(new StampKey(end.channel(), 1 - end.side()));
            carried = queue == null || queue.isEmpty() ? Map.of() : queue.poll();
          }
          for (var e : carried.entrySet())
          {
            clock.merge(e.getKey(), e.getValue(), Math::max);
          }
          clock.merge(me, 1L, Long::sum);
          if (value instanceof ScenarioEnd received)
          {
            ends.put(atomText(act.get(2)), received);
          }
          else
          {
            env.put(atomText(act.get(2)), (Value) value);
          }
        }
        case "par" -> {
          var forms = act.subList(1, act.size());
          var branches = new ArrayList<List<Object>>();
          for (var b : forms)
          {
            var branch = form(b);
            branches.add(branch.subList(1, branch.size()));
          }
          var owned = new java.util.LinkedHashMap<String, List<Integer>>();
          for (int i = 0; i < branches.size(); i++)
          {
            for (var name : actsChannels(branches.get(i)))
            {
              var users = owned.computeIfAbsent(name, k -> new ArrayList<Integer>());
              if (!users.contains(i))
              {
                users.add(i);
              }
            }
          }
          var threads = new ArrayList<Thread>();
          var outcomes = new boolean[branches.size()];
          var clocks = new ArrayList<Map<String, Long>>();
          for (int i = 0; i < branches.size(); i++)
          {
            clocks.add(new java.util.concurrent.ConcurrentHashMap<String, Long>(clock));
          }
          for (int i = 0; i < branches.size(); i++)
          {
            var mine = new java.util.HashMap<String, ScenarioEnd>();
            for (var entry : owned.entrySet())
            {
              var users = entry.getValue();
              if (!users.contains(i))
              {
                continue;
              }
              String name = entry.getKey();
              if (ends.containsKey(name))
              {
                mine.put(name, ends.remove(name));
              }
              else if (run.channels.containsKey(name))
              {
                mine.put(name, new ScenarioEnd(run.channels.get(name), users.indexOf(i)));
              }
            }
            final int slot = i;
            final var branch = branches.get(i);
            final var branchIdentity = forms.get(i);
            final var copy = new java.util.HashMap<String, Value>(env);
            final var branchRandom =
                new SplitMix64(run.shake ^ ((long) (threads.size() + 1) * 0x9E3779B97F4A7C15L));
            threads.add(
                new Thread(
                    () ->
                        outcomes[slot] =
                            scenarioProcess(
                                run,
                                branch,
                                copy,
                                mine,
                                branchRandom,
                                branchIdentity,
                                clocks.get(slot))));
          }
          for (var t : threads)
          {
            t.start();
          }
          for (var t : threads)
          {
            boolean interrupted = false;
            while (true)
            {
              try
              {
                t.join();
                break;
              }
              catch (InterruptedException e)
              {
                interrupted = true;
              }
            }
            if (interrupted)
            {
              Thread.currentThread().interrupt();
            }
          }
          for (var child : clocks)
          {
            for (var e : child.entrySet())
            {
              clock.merge(e.getKey(), e.getValue(), Math::max);
            }
          }
          clock.merge(me, 1L, Long::sum);
          // A failed branch fails the process that ran the par.
          for (var ok : outcomes)
          {
            if (!ok)
            {
              return false;
            }
          }
        }
        case "expect" -> {
          String name = atomText(act.get(1));
          Value actual = env.get(name);
          Value wanted = scenarioConstant(form(act.get(2)));
          if (actual == null || compareValues(actual, wanted) != 0)
          {
            run.failures.add(
                "expect "
                    + name
                    + " = "
                    + render(wanted)
                    + " failed: "
                    + name
                    + " is "
                    + (actual == null ? "None" : render(actual)));
            return false;
          }
        }
        default -> {}
      }
    }
    if (run.victim != null && run.victim == identity && run.victimAct == acts.size())
    {
      return false;
    }
    return true;
  }

  private record ScenarioOutcome(String title, String failure)
  {}

  private static ScenarioOutcome runScenario(
      Model model, String spec, long shake, boolean crash, boolean network)
  {
    var forms = readDescriptor(spec);
    String title = atomText(form(forms.get(0)).get(1));
    List<Object> channelNames = null;
    List<Object> body = null;
    List<Object> wire = null;
    var boxes = new ArrayList<String>();
    for (var f : forms)
    {
      var item = form(f);
      String head = atomText(item.get(0));
      if (head.equals("mailboxes"))
      {
        for (var m : item.subList(1, item.size()))
        {
          boxes.add(atomText(m));
        }
      }
      if (head.equals("channels") && channelNames == null)
      {
        channelNames = item.subList(1, item.size());
      }
      if (head.equals("process") && body == null)
      {
        body = item.subList(1, item.size());
      }
      if (head.equals("wire") && wire == null)
      {
        wire = item.subList(1, item.size());
      }
    }
    var run = new ScenarioRun(model, shake);
    if (network && wire != null)
    {
      // Loss, duplication and delay (which reorders); the channels'
      // numbered, acknowledged frames must hide them all.
      var net = new MemoryNetwork(shake ^ 0x7F4A7C159E3779B9L, 0.1, 0.1, 0.002);
      var table = new java.util.HashMap<String, List<Object>>();
      var steps = new java.util.HashMap<String, List<Step>>();
      for (var f : wire)
      {
        var item = form(f);
        if (atomText(item.get(0)).equals("data"))
        {
          table.put(atomText(item.get(1)), item);
        }
        else if (atomText(item.get(0)).equals("channel"))
        {
          var list = new ArrayList<Step>();
          for (var s : item.subList(2, item.size()))
          {
            var st = form(s);
            list.add(new Step(atomText(st.get(0)).equals("send"), st.get(1)));
          }
          steps.put(atomText(item.get(1)), list);
        }
      }
      var kinds = new java.util.HashMap<String, Object>();
      for (var f : wire)
      {
        var item = form(f);
        if (atomText(item.get(0)).equals("mailbox"))
        {
          kinds.put(atomText(item.get(1)), item.get(2));
        }
      }
      var types = new Values(table);
      var registry = new java.util.HashMap<String, NetScenarioChannel>();
      for (var c : channelNames)
      {
        run.channels.put(
            atomText(c),
            new NetScenarioChannel(net, atomText(c), steps.get(atomText(c)), types, registry));
      }
      for (var m : boxes)
      {
        run.mailboxes.put(
            m,
            kinds.containsKey(m)
                ? new ScenarioMailbox(m, allSends(body, m), net, kinds.get(m), types, registry)
                : new ScenarioMailbox(m, allSends(body, m)));
      }
    }
    else
    {
      for (var c : channelNames)
      {
        run.channels.put(atomText(c), new ScenarioChannel());
      }
      for (var m : boxes)
      {
        run.mailboxes.put(m, new ScenarioMailbox(m, allSends(body, m)));
      }
    }
    for (var c : model.commands)
    {
      run.commands.put(c.name, c);
    }
    Map<String, Object> symbols = new java.util.HashMap<String, Object>();
    var startArgs = new ArrayList<Value>();
    for (var d : model.startArguments)
    {
      startArgs.add(model.values.minimal(d));
    }
    run.state = model.startRun.apply(symbols, startArgs);
    var expected = model.startModel.apply(symbols, startArgs);
    var processes = scenarioProcesses(body, new ArrayList<Object>());
    for (int k = 0; k < processes.size(); k++)
    {
      run.processNames.put(processes.get(k), "p" + k);
    }
    if (crash && !processes.isEmpty())
    {
      var chooser = new SplitMix64(shake ^ 0xC3A5C85C97CB3127L);
      run.victim = processes.get((int) chooser.below(processes.size()));
      run.victimAct = chooser.below(form(run.victim).size() - 1 + 1);
    }
    boolean finished =
        scenarioProcess(
            run,
            body,
            new java.util.HashMap<String, Value>(),
            new java.util.HashMap<String, ScenarioEnd>(),
            new SplitMix64(shake),
            null,
            new java.util.HashMap<String, Long>());
    for (var c : run.channels.values())
    {
      c.close();
    }
    for (var m : run.mailboxes.values())
    {
      m.close();
    }
    if (!run.failures.isEmpty())
    {
      return new ScenarioOutcome(
          title, run.failures.get(0) + (run.victim != null ? " (with a process crashed)" : ""));
    }
    if (!finished && run.victim == null)
    {
      return new ScenarioOutcome(title, "a process failed");
    }
    var finalState =
        model.abstractState != null ? model.abstractState.apply(symbols, List.of(run.state)) : null;
    var history = new ArrayList<ScenarioCall>(run.history);
    if (!linearizesHistory(model, symbols, history, expected, finalState, run.state))
    {
      var sorted = new ArrayList<ScenarioCall>(history);
      sorted.sort((x, y) -> Long.compare(x.called(), y.called()));
      var observed = new ArrayList<String>();
      for (var c : sorted)
      {
        observed.add(
            c.command().name + "(" + renderAll(c.args()) + ") returned " + render(c.result()));
      }
      return new ScenarioOutcome(
          title,
          "the calls are not "
              + consistent(model.consistency)
              + " with the model ("
              + String.join("; ", observed)
              + ")");
    }
    return new ScenarioOutcome(title, null);
  }

  /** Whether call a returned before call b began, as far as messages tell. */
  private static boolean happenedBefore(ScenarioCall a, ScenarioCall b)
  {
    for (var e : a.atReturn().entrySet())
    {
      if (b.atCall().getOrDefault(e.getKey(), 0L) < e.getValue())
      {
        return false;
      }
    }
    return true;
  }

  /**
   * A Wing-Gong search over the scenario's calls, memoized on the calls done and the state.
   * Linearizable: next, a call no pending call returned before (real time). Sequential: next, a
   * call every call that happened before it (its process's order, and messages) is done. Causal:
   * each process's results from an order of what happened before them. Eventual: no results, only
   * the final state.
   */
  private static boolean linearizesHistory(
      Model model,
      Map<String, Object> symbols,
      List<ScenarioCall> history,
      Value expected,
      Value finalState,
      Value state)
  {
    String mode = model.consistency;
    var everything = new ArrayList<Integer>();
    for (int i = 0; i < history.size(); i++)
    {
      everything.add(i);
    }
    if (mode.equals("causal"))
    {
      var processes = new java.util.LinkedHashSet<String>();
      for (var c : history)
      {
        processes.add(c.process());
      }
      for (var process : processes)
      {
        var own = new java.util.HashSet<Integer>();
        for (int i : everything)
        {
          if (history.get(i).process().equals(process))
          {
            own.add(i);
          }
        }
        var seenBy = new java.util.TreeSet<Integer>(own);
        for (int j : everything)
        {
          for (int i : own)
          {
            if (j != i && happenedBefore(history.get(j), history.get(i)))
            {
              seenBy.add(j);
            }
          }
        }
        if (!visitHistory(
            model,
            symbols,
            history,
            new ArrayList<Integer>(seenBy),
            own,
            false,
            finalState,
            state,
            new java.util.BitSet(),
            expected,
            new java.util.HashSet<String>()))
        {
          return false;
        }
      }
      return true;
    }
    var checked = new java.util.HashSet<Integer>(mode.equals("eventual") ? List.of() : everything);
    return visitHistory(
        model,
        symbols,
        history,
        everything,
        checked,
        true,
        finalState,
        state,
        new java.util.BitSet(),
        expected,
        new java.util.HashSet<String>());
  }

  private static boolean visitHistory(
      Model model,
      Map<String, Object> symbols,
      List<ScenarioCall> history,
      List<Integer> members,
      java.util.Set<Integer> checked,
      boolean judgeFinal,
      Value finalState,
      Value state,
      java.util.BitSet done,
      Value modelState,
      java.util.Set<String> seen)
  {
    if (!seen.add(done + "|" + render(modelState)))
    {
      return false;
    }
    boolean linear = model.consistency.equals("linearizable");
    if (done.cardinality() == members.size())
    {
      if (!judgeFinal)
      {
        return true;
      }
      if (finalState != null && compareValues(finalState, modelState) != 0)
      {
        return false;
      }
      for (int k = 0; k < model.invariants.size(); k++)
      {
        var subject = model.invariantKinds.get(k).equals("model") ? modelState : state;
        if (!truth(model.invariants.get(k).apply(symbols, List.of(subject))))
        {
          return false;
        }
      }
      return true;
    }
    for (int i : members)
    {
      if (done.get(i))
      {
        continue;
      }
      var call = history.get(i);
      boolean blocked = false;
      for (int j : members)
      {
        if (j != i
            && !done.get(j)
            && (linear
                ? history.get(j).returned() < call.called()
                : happenedBefore(history.get(j), call)))
        {
          blocked = true;
        }
      }
      if (blocked)
      {
        continue;
      }
      Stepped stepped;
      try
      {
        stepped = stepModel(call.command(), symbols, call.args(), modelState);
      }
      catch (InvalidStep e)
      {
        continue;
      }
      if (checked.contains(i)
          && !call.command().unit
          && compareValues(call.result(), stepped.result()) != 0)
      {
        continue;
      }
      var next = (java.util.BitSet) done.clone();
      next.set(i);
      if (visitHistory(
          model,
          symbols,
          history,
          members,
          checked,
          judgeFinal,
          finalState,
          state,
          next,
          stepped.state(),
          seen))
      {
        return true;
      }
    }
    return false;
  }

  /** Runs a scenario on many schedules; a failure throws AssertionError. */
  public static void checkScenario(Model model, String spec)
  {
    String text = System.getenv("LAWSPEC_SEED");
    checkScenario(model, spec, 30, text == null ? 0 : Long.parseUnsignedLong(text.trim()));
  }

  public static void checkScenario(Model model, String spec, int runs, long seed)
  {
    var random = new SplitMix64(seed ^ 0x2545F4914F6CDD1DL);
    for (int n = 0; n < runs; n++)
    {
      // Every third run crashes one process of a par at a random point, and
      // every third other one sends each channel over a faulty network.
      var outcome = runScenario(model, spec, random.next(), n % 3 == 2, n % 3 == 1);
      if (outcome.failure() != null)
      {
        throw new AssertionError("scenario " + outcome.title() + " fails: " + outcome.failure());
      }
    }
  }

  // Sessions: typed channel ends for implementation code (lawspec.sessions,
  // generated from a unit's protocols). An end's send and receive go through a
  // Channel; spawn and par run the processes that hold the ends.

  /**
   * A two-sided channel: side 0 holds a protocol's first end, side 1 its second. A networked
   * transport can implement it too.
   */
  public interface Channel
  {
    /** Sends a value from the given side to the other. */
    void send(int side, Object value);

    /**
     * Blocks until the other side has sent a value to the given side; throws PeerFailed once the
     * other side has given up and nothing is left.
     */
    Object receive(int side);

    /** The given side gives up: the other side's receives fail after the values already sent. */
    default void abandon(int side)
    {
      throw new UnsupportedOperationException("this channel cannot be abandoned");
    }
  }

  /**
   * A receive whose other end gave up: its process failed, or it called abandon(). Catch it to
   * handle the failure (or else); otherwise this process fails too.
   */
  public static final class PeerFailed extends IllegalStateException
  {
    public PeerFailed(String message)
    {
      super(message);
    }
  }

  /** A fresh in-memory channel: one queue per direction. */
  public static Channel channel()
  {
    return new LocalChannel();
  }

  private static final class LocalChannel implements Channel
  {
    // A box, so that a null value can travel through the queue.
    private record Message(Object value)
    {}

    @SuppressWarnings("unchecked")
    private final java.util.concurrent.LinkedBlockingQueue<Message>[] inboxes =
        new java.util.concurrent.LinkedBlockingQueue[] {
          new java.util.concurrent.LinkedBlockingQueue<Message>(),
          new java.util.concurrent.LinkedBlockingQueue<Message>()
        };

    @Override
    public void send(int side, Object value)
    {
      inboxes[1 - side].add(new Message(value));
    }

    @Override
    public Object receive(int side)
    {
      try
      {
        var message = inboxes[side].take();
        if (message == ABANDONED)
        {
          inboxes[side].add(message);
          throw new PeerFailed(
              "the other end gave up the conversation (its process failed or abandoned it)");
        }
        return message.value();
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        throw new IllegalStateException("interrupted while receiving", e);
      }
    }

    @Override
    public void abandon(int side)
    {
      inboxes[1 - side].add(ABANDONED);
    }

    private static final Message ABANDONED = new Message(null);
  }

  /** What a receive returns: the value and the end's next step. */
  public record Received<T, NextT>(T value, NextT next)
  {}

  /** Claims a single-use end: the first claim succeeds, any later one throws. */
  public static void claimEnd(java.util.concurrent.atomic.AtomicBoolean used)
  {
    if (used.getAndSet(true))
    {
      throw new IllegalStateException(
          "this end was already used; use the end its last step returned");
    }
  }

  /** A process started by spawn. */
  public static final class Spawned<T>
  {
    private final java.util.concurrent.FutureTask<T> task;

    private Spawned(java.util.concurrent.FutureTask<T> task)
    {
      this.task = task;
    }

    /** Waits for the process and returns its result, rethrowing its failure. */
    public T join()
    {
      try
      {
        return task.get();
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        throw new IllegalStateException("interrupted while joining", e);
      }
      catch (java.util.concurrent.ExecutionException e)
      {
        var cause = e.getCause();
        if (cause instanceof RuntimeException runtime)
        {
          throw runtime;
        }
        if (cause instanceof Error error)
        {
          throw error;
        }
        throw new IllegalStateException(cause);
      }
    }
  }

  /** Runs a process on its own (virtual) thread. */
  public static <T> Spawned<T> spawn(java.util.concurrent.Callable<T> process)
  {
    var task = new java.util.concurrent.FutureTask<>(process);
    Thread.ofVirtual().start(task);
    return new Spawned<>(task);
  }

  /** Runs a process that returns nothing on its own (virtual) thread. */
  public static Spawned<Void> spawn(Runnable process)
  {
    return spawn(
        () -> {
          process.run();
          return null;
        });
  }

  /** Runs processes at the same time and waits for all; rethrows the first failure. */
  public static void par(Runnable... processes)
  {
    var spawned = new ArrayList<Spawned<Void>>();
    for (var process : processes)
    {
      spawned.add(spawn(process));
    }
    RuntimeException failure = null;
    for (var process : spawned)
    {
      try
      {
        process.join();
      }
      catch (RuntimeException e)
      {
        if (failure == null)
        {
          failure = e;
        }
      }
    }
    if (failure != null)
    {
      throw failure;
    }
  }

  // Actors. An actor owns a state and handles one message at a time, in the
  // order they arrive. It is not a thread: a message sent to an idle actor
  // starts a virtual thread that drains its mailbox and then ends, so an idle
  // actor costs only its state and queue.

  /** A message sent to an actor or mailbox that has stopped. */
  public static final class ActorStopped extends IllegalStateException
  {
    public ActorStopped(String message)
    {
      super(message);
    }
  }

  /**
   * A handler failed, so the actor crashed; the cause is what the handler threw. A supervised actor
   * restarts; any other stops.
   */
  public static final class ActorCrashed extends IllegalStateException
  {
    public ActorCrashed(Throwable cause)
    {
      super("the actor crashed: " + cause, cause);
    }
  }

  /** A message that crashes an actor on purpose (crash, links). */
  private static final class RestartSignal extends RuntimeException
  {
    final Object reason;
    final long origin;

    RestartSignal(Object reason, long origin)
    {
      super(null, null, false, false);
      this.reason = reason;
      this.origin = origin;
    }
  }

  private static final java.util.concurrent.atomic.AtomicLong CRASH_IDS =
      new java.util.concurrent.atomic.AtomicLong();

  private static long nextCrash()
  {
    return CRASH_IDS.incrementAndGet();
  }

  /**
   * How an actor or supervisor ended, as monitors hear it: kind "crashed" (with the cause) or
   * "stopped" (cause null).
   */
  public record Exit(String kind, Object cause)
  {}

  /** What a handler returns: its reply and the actor's next state. */
  public record Next<R, S>(R reply, S state)
  {}

  /**
   * An actor owning a state of type S, handling one message at a time.
   *
   * <p>restart (the last state to the state after a crash) lets a supervisor restart it; without
   * it, a crash stops the actor even under a supervisor. A supervised actor restarts in place: it
   * keeps its address and the messages waiting for it.
   */
  public static final class Actor<S>
  {
    private record Message<S>(
        Function<S, ? extends Next<?, S>> handler,
        java.util.concurrent.CompletableFuture<Object> reply)
    {}

    private S state;
    private final Function<S, S> restartState;
    private final java.util.ArrayDeque<Message<S>> mailbox = new java.util.ArrayDeque<>();
    private boolean draining;
    private boolean stopped;
    private volatile Supervisor supervisor;
    private final List<java.util.function.Consumer<Exit>> monitors =
        new java.util.concurrent.CopyOnWriteArrayList<>();
    private final List<Actor<?>> links = new java.util.concurrent.CopyOnWriteArrayList<>();
    private final java.util.Set<Long> seen = java.util.concurrent.ConcurrentHashMap.newKeySet();

    public Actor(S state)
    {
      this(state, null);
    }

    public Actor(S state, Function<S, S> restart)
    {
      this.state = state;
      this.restartState = restart;
    }

    private void post(Message<S> message)
    {
      synchronized (mailbox)
      {
        if (stopped)
        {
          throw new ActorStopped("the actor has stopped");
        }
        mailbox.add(message);
        if (draining)
        {
          return;
        }
        draining = true;
      }
      Thread.ofVirtual().start(this::drain);
    }

    private void drain()
    {
      while (true)
      {
        Message<S> message;
        synchronized (mailbox)
        {
          message = mailbox.poll();
          if (message == null)
          {
            draining = false;
            return;
          }
        }
        Object reply = null;
        Throwable failure = null;
        try
        {
          var next = message.handler().apply(state);
          state = next.state();
          reply = next.reply();
        }
        catch (RestartSignal crash)
        {
          // A crash that already reached this actor (through another link) is not repeated.
          if (!seen.contains(crash.origin))
          {
            crashed(crash.reason, crash.origin);
          }
        }
        catch (Throwable error)
        {
          failure = new ActorCrashed(error);
          crashed(error, nextCrash());
        }
        if (message.reply() != null)
        {
          if (failure != null)
          {
            message.reply().completeExceptionally(failure);
          }
          else
          {
            message.reply().complete(reply);
          }
        }
      }
    }

    /**
     * On the actor's turn: restart or stop, then tell monitors and links. origin names the first
     * crash, so a crash crosses each link once.
     */
    private void crashed(Object cause, long origin)
    {
      seen.add(origin);
      var parent = supervisor;
      boolean restarted =
          parent != null && restartState != null && parent.childCrashed(this, cause);
      if (!restarted)
      {
        halt();
      }
      for (var monitor : monitors)
      {
        monitor.accept(new Exit("crashed", cause));
      }
      for (var other : links)
      {
        other.linkCrash(cause, origin);
      }
    }

    /** On the actor's turn: the restarted state from the last one. */
    void restartNow()
    {
      state = restartState.apply(state);
    }

    /** A restart a supervisor asks of a sibling, in mailbox order. */
    void restartLater()
    {
      if (restartState == null)
      {
        return;
      }
      try
      {
        post(new Message<S>(s -> new Next<Object, S>(null, restartState.apply(s)), null));
      }
      catch (ActorStopped e)
      {
        // already stopped
      }
    }

    private void linkCrash(Object cause, long origin)
    {
      if (seen.contains(origin))
      {
        return;
      }
      try
      {
        post(
            new Message<S>(
                s -> {
                  throw new RestartSignal(cause, origin);
                },
                null));
      }
      catch (ActorStopped e)
      {
        // already stopped
      }
    }

    /** Stops the actor; messages still waiting fail with ActorStopped. */
    void halt()
    {
      List<Message<S>> waiting;
      synchronized (mailbox)
      {
        stopped = true;
        waiting = new ArrayList<>(mailbox);
        mailbox.clear();
      }
      for (var m : waiting)
      {
        if (m.reply() != null)
        {
          m.reply().completeExceptionally(new ActorStopped("the actor has stopped"));
        }
      }
    }

    void setSupervisor(Supervisor parent)
    {
      supervisor = parent;
    }

    /**
     * Runs the handler on the state in turn and returns its reply. A handler that throws crashes
     * the actor, and call throws ActorCrashed.
     */
    @SuppressWarnings("unchecked")
    public <R> R call(Function<S, Next<R, S>> handler)
    {
      var reply = new java.util.concurrent.CompletableFuture<Object>();
      post(new Message<S>(handler, reply));
      try
      {
        return (R) reply.join();
      }
      catch (java.util.concurrent.CompletionException e)
      {
        var cause = e.getCause();
        if (cause instanceof RuntimeException runtime)
        {
          throw runtime;
        }
        if (cause instanceof Error error)
        {
          throw error;
        }
        throw new IllegalStateException(cause);
      }
    }

    /** Queues the handler without waiting for its reply. */
    public void cast(Function<S, ? extends Next<?, S>> handler)
    {
      post(new Message<S>(handler, null));
    }

    /**
     * Crashes the actor once the messages before this one are handled, as a failing handler would:
     * for testing supervision.
     */
    public void crash()
    {
      crash("crashed on purpose");
    }

    public void crash(Object cause)
    {
      long origin = nextCrash();
      call(
          s -> {
            throw new RestartSignal(cause, origin);
          });
    }

    /**
     * Replaces the state by restart(last state) between messages, as a supervised restart does
     * (crash injection in model runs).
     */
    public void restart(Function<S, S> restart)
    {
      call(s -> new Next<Object, S>(null, restart.apply(s)));
    }

    /** The state after every message sent before this call. */
    public S state()
    {
      return call(s -> new Next<S, S>(s, s));
    }

    /** notify gets Exit("crashed", cause) after each crash, and Exit("stopped", null) once. */
    public void monitor(java.util.function.Consumer<Exit> notify)
    {
      monitors.add(notify);
    }

    /** Links two actors: when either crashes, the other crashes too. */
    public void link(Actor<?> other)
    {
      links.add(other);
      other.links.add(this);
    }

    /**
     * Refuses further messages; those already queued still run. A permanent child of a supervisor
     * restarts instead.
     */
    public void stop()
    {
      var parent = supervisor;
      if (parent != null && parent.childStopped(this))
      {
        return;
      }
      boolean already;
      synchronized (mailbox)
      {
        already = stopped;
        stopped = true;
      }
      if (!already)
      {
        for (var monitor : monitors)
        {
          monitor.accept(new Exit("stopped", null));
        }
      }
    }
  }

  /**
   * Starts children (actors or supervisors) and restarts them after a crash. Strategy "one_for_one"
   * restarts the child that crashed, "one_for_all" every child, "rest_for_one" it and those added
   * after it. A child's lifetime: "permanent" restarts after a crash or a stop, "transient" only
   * after a crash, "temporary" never. More than maxRestarts within period seconds is the
   * supervisor's own crash: its supervisor restarts all of its children, or, at the top, every
   * child stops.
   *
   * <p>Supervision follows OTP's strategies and restart types, so a supervision tree means what an
   * Erlang programmer expects on every target. ref:DEC-actors-otp-supervision
   * ref:erlang-otp-supervisors
   */
  public static final class Supervisor
  {
    public static final String ONE_FOR_ONE = "one_for_one";
    public static final String ONE_FOR_ALL = "one_for_all";
    public static final String REST_FOR_ONE = "rest_for_one";
    public static final String PERMANENT = "permanent";
    public static final String TRANSIENT = "transient";
    public static final String TEMPORARY = "temporary";

    private static final class Entry
    {
      final Object child;
      final String lifetime;

      Entry(Object child, String lifetime)
      {
        this.child = child;
        this.lifetime = lifetime;
      }
    }

    private final String strategy;
    private final int maxRestarts;
    private final long periodNanos;
    private List<Entry> children = new ArrayList<>();
    private final java.util.ArrayDeque<Long> restarts = new java.util.ArrayDeque<>();
    private volatile Supervisor supervisor;
    private boolean stopped;
    private final List<java.util.function.Consumer<Exit>> monitors =
        new java.util.concurrent.CopyOnWriteArrayList<>();

    public Supervisor()
    {
      this(ONE_FOR_ONE, 3, 5.0);
    }

    public Supervisor(String strategy)
    {
      this(strategy, 3, 5.0);
    }

    public Supervisor(String strategy, int maxRestarts, double periodSeconds)
    {
      if (!List.of(ONE_FOR_ONE, ONE_FOR_ALL, REST_FOR_ONE).contains(strategy))
      {
        throw new IllegalArgumentException("unknown strategy " + strategy);
      }
      this.strategy = strategy;
      this.maxRestarts = maxRestarts;
      this.periodNanos = (long) (periodSeconds * 1e9);
    }

    /** Adds a started child (an Actor or a Supervisor), and returns it. */
    public <C> C supervise(C child)
    {
      return supervise(child, PERMANENT);
    }

    public synchronized <C> C supervise(C child, String lifetime)
    {
      if (!List.of(PERMANENT, TRANSIENT, TEMPORARY).contains(lifetime))
      {
        throw new IllegalArgumentException("unknown lifetime " + lifetime);
      }
      setParent(child, this);
      children.add(new Entry(child, lifetime));
      return child;
    }

    public synchronized List<Object> children()
    {
      var out = new ArrayList<Object>();
      for (var e : children)
      {
        out.add(e.child);
      }
      return out;
    }

    /** How many restarts are counted in the current period (for the runtime's own check). */
    synchronized int restartCount()
    {
      return restarts.size();
    }

    private static void setParent(Object child, Supervisor parent)
    {
      if (child instanceof Actor<?> a)
      {
        a.setSupervisor(parent);
      }
      else if (child instanceof Supervisor s)
      {
        s.supervisor = parent;
      }
      else
      {
        throw new IllegalArgumentException("a supervisor's child is an Actor or a Supervisor");
      }
    }

    private static void restartLater(Object child)
    {
      if (child instanceof Actor<?> a)
      {
        a.restartLater();
      }
      else
      {
        ((Supervisor) child).restartLater();
      }
    }

    private static void halt(Object child)
    {
      if (child instanceof Actor<?> a)
      {
        a.halt();
      }
      else
      {
        ((Supervisor) child).stop();
      }
    }

    private static void stopChild(Object child)
    {
      if (child instanceof Actor<?> a)
      {
        a.stop();
      }
      else
      {
        ((Supervisor) child).stop();
      }
    }

    private boolean allowRestart()
    {
      long now = System.nanoTime();
      while (!restarts.isEmpty() && now - restarts.peekFirst() > periodNanos)
      {
        restarts.pollFirst();
      }
      if (restarts.size() >= maxRestarts)
      {
        return false;
      }
      restarts.addLast(now);
      return true;
    }

    private Entry entry(Object child)
    {
      for (var e : children)
      {
        if (e.child == child)
        {
          return e;
        }
      }
      return null;
    }

    /** Under the lock: the children to restart for entry's crash, or null when it gives up. */
    private List<Entry> restarting(Entry entry, Object cause, Object crashed)
    {
      if (allowRestart())
      {
        int index = children.indexOf(entry);
        return switch (strategy)
        {
          case ONE_FOR_ALL -> new ArrayList<>(children);
          case REST_FOR_ONE -> new ArrayList<>(children.subList(index, children.size()));
          default -> List.of(entry);
        };
      }
      var parent = supervisor;
      if (parent != null && parent.childFailed(this, cause))
      {
        restarts.clear();
        return new ArrayList<>(children);
      }
      fail(crashed, cause);
      return null;
    }

    /** On child's turn: true when it restarts now. */
    boolean childCrashed(Actor<?> child, Object cause)
    {
      List<Entry> group;
      synchronized (this)
      {
        var entry = entry(child);
        if (stopped || entry == null)
        {
          return false;
        }
        if (entry.lifetime.equals(TEMPORARY))
        {
          children.remove(entry);
          return false;
        }
        group = restarting(entry, cause, child);
        if (group == null)
        {
          return false;
        }
      }
      for (var other : group)
      {
        if (other.child != child)
        {
          restartLater(other.child);
        }
      }
      child.restartNow();
      return true;
    }

    /** A child supervisor gave up: true when it may restart its children. */
    boolean childFailed(Supervisor child, Object cause)
    {
      List<Entry> group;
      synchronized (this)
      {
        var entry = entry(child);
        if (stopped || entry == null)
        {
          return false;
        }
        if (entry.lifetime.equals(TEMPORARY))
        {
          children.remove(entry);
          return false;
        }
        group = restarting(entry, cause, child);
        if (group == null)
        {
          return false;
        }
      }
      for (var other : group)
      {
        if (other.child != child)
        {
          restartLater(other.child);
        }
      }
      return true;
    }

    /** True when a stopped child is permanent and restarts instead. */
    boolean childStopped(Object child)
    {
      List<Entry> group;
      synchronized (this)
      {
        var entry = entry(child);
        if (entry == null || stopped)
        {
          return false;
        }
        if (!entry.lifetime.equals(PERMANENT))
        {
          children.remove(entry);
          return false;
        }
        group = restarting(entry, "stopped", child);
        if (group == null)
        {
          return false;
        }
      }
      for (var other : group)
      {
        restartLater(other.child);
      }
      return true;
    }

    /**
     * Under the lock: every child but the crashing one (which stops itself) stops, and so does the
     * supervisor.
     */
    private void fail(Object crashed, Object cause)
    {
      var all = children;
      children = new ArrayList<>();
      stopped = true;
      for (int k = all.size() - 1; k >= 0; k--)
      {
        var other = all.get(k).child;
        setParent(other, null);
        if (other != crashed)
        {
          halt(other);
        }
      }
      for (var monitor : monitors)
      {
        monitor.accept(new Exit("crashed", cause));
      }
    }

    /** Restarted by its own supervisor: every child restarts. */
    private void restartLater()
    {
      List<Entry> all;
      synchronized (this)
      {
        restarts.clear();
        all = new ArrayList<>(children);
      }
      for (var e : all)
      {
        restartLater(e.child);
      }
    }

    /**
     * notify gets Exit("crashed", cause) when it passes its restart limit, and Exit("stopped",
     * null) once stopped.
     */
    public void monitor(java.util.function.Consumer<Exit> notify)
    {
      monitors.add(notify);
    }

    /** Stops every child, last added first, without restarting them. */
    public void stop()
    {
      List<Entry> all;
      synchronized (this)
      {
        if (stopped)
        {
          return;
        }
        stopped = true;
        all = children;
        children = new ArrayList<>();
      }
      for (int k = all.size() - 1; k >= 0; k--)
      {
        var child = all.get(k).child;
        setParent(child, null);
        stopChild(child);
      }
      for (var monitor : monitors)
      {
        monitor.accept(new Exit("stopped", null));
      }
    }
  }

  /**
   * The runtime's own check of crashes, links, monitors and supervision: every strategy, lifetime,
   * the restart limit and escalation. Throws AssertionError naming the first behaviour that
   * differs.
   */
  public static void checkSupervision()
  {
    java.util.function.Supplier<Actor<Long>> counter = () -> new Actor<Long>(0L, s -> 0L);
    java.util.function.Consumer<Actor<Long>> bump =
        a -> a.call(s -> new Next<Long, Long>(s + 1, s + 1));
    java.util.function.Consumer<Actor<Long>> fail =
        a -> {
          try
          {
            a.call(
                s -> {
                  throw new ArithmeticException("/ by zero");
                });
          }
          catch (ActorCrashed e)
          {
            return;
          }
          throw new AssertionError("a failing handler did not throw ActorCrashed");
        };
    java.util.function.BiConsumer<Actor<Long>, String> stopped =
        (a, what) -> {
          try
          {
            a.state();
          }
          catch (ActorStopped e)
          {
            return;
          }
          throw new AssertionError(what + " should have stopped");
        };
    var a = counter.get();
    bump.accept(a);
    fail.accept(a);
    stopped.accept(a, "an unsupervised actor that crashed");
    var sup = new Supervisor(Supervisor.ONE_FOR_ONE);
    var x = sup.supervise(counter.get());
    var y = sup.supervise(counter.get());
    bump.accept(x);
    bump.accept(y);
    bump.accept(y);
    fail.accept(x);
    expectSupervision(
        List.of(x.state(), y.state()),
        List.of(0L, 2L),
        "one for one restarts only the crashed child");
    sup = new Supervisor(Supervisor.ONE_FOR_ALL);
    x = sup.supervise(counter.get());
    y = sup.supervise(counter.get());
    bump.accept(x);
    bump.accept(y);
    fail.accept(x);
    expectSupervision(
        List.of(x.state(), y.state()), List.of(0L, 0L), "one for all restarts every child");
    sup = new Supervisor(Supervisor.REST_FOR_ONE);
    x = sup.supervise(counter.get());
    y = sup.supervise(counter.get());
    var z = sup.supervise(counter.get());
    bump.accept(x);
    bump.accept(y);
    bump.accept(z);
    fail.accept(y);
    expectSupervision(
        List.of(x.state(), y.state(), z.state()),
        List.of(1L, 0L, 0L),
        "rest for one restarts the child and later ones");
    sup = new Supervisor();
    var t = sup.supervise(counter.get(), Supervisor.TEMPORARY);
    fail.accept(t);
    stopped.accept(t, "a temporary child that crashed");
    sup = new Supervisor();
    var p = sup.supervise(counter.get(), Supervisor.PERMANENT);
    var q = sup.supervise(counter.get(), Supervisor.TRANSIENT);
    bump.accept(p);
    p.stop();
    expectSupervision(p.state(), 0L, "a permanent child restarts after a stop");
    q.stop();
    stopped.accept(q, "a transient child that was stopped");
    var events = new java.util.concurrent.CopyOnWriteArrayList<Exit>();
    sup = new Supervisor(Supervisor.ONE_FOR_ONE, 2, 10);
    sup.monitor(events::add);
    x = sup.supervise(counter.get());
    y = sup.supervise(counter.get());
    fail.accept(x);
    fail.accept(x);
    fail.accept(x);
    stopped.accept(y, "a child of a supervisor past its restart limit");
    expectSupervision(
        kinds(events), List.of("crashed"), "a supervisor past its limit tells its monitors");
    var outer = new Supervisor(Supervisor.ONE_FOR_ONE, 5, 10);
    var inner = outer.supervise(new Supervisor(Supervisor.ONE_FOR_ONE, 1, 10));
    x = inner.supervise(counter.get());
    y = inner.supervise(counter.get());
    bump.accept(y);
    fail.accept(x);
    fail.accept(x);
    expectSupervision(
        List.of(x.state(), y.state()),
        List.of(0L, 0L),
        "a supervisor past its limit is restarted by its own");
    var seen = new java.util.concurrent.CopyOnWriteArrayList<Exit>();
    a = counter.get();
    var b = counter.get();
    a.link(b);
    b.monitor(seen::add);
    fail.accept(a);
    for (int k = 0; k < 100 && seen.isEmpty(); k++)
    {
      sleepQuietly(10);
    }
    stopped.accept(b, "an unsupervised actor linked to one that crashed");
    expectSupervision(kinds(seen), List.of("crashed"), "a monitor hears of a crash");
    sup = new Supervisor(Supervisor.ONE_FOR_ONE, 10, 5.0);
    a = sup.supervise(counter.get());
    b = sup.supervise(counter.get());
    var c = sup.supervise(counter.get());
    a.link(b);
    b.link(c);
    c.link(a);
    bump.accept(a);
    bump.accept(b);
    bump.accept(c);
    fail.accept(a);
    for (int k = 0; k < 100 && sup.restartCount() < 3; k++)
    {
      sleepQuietly(10);
    }
    sleepQuietly(50);
    expectSupervision(
        List.of(a.state(), b.state(), c.state(), (long) sup.restartCount()),
        List.of(0L, 0L, 0L, 3L),
        "a crash crosses each link once");
  }

  private static List<String> kinds(List<Exit> exits)
  {
    var out = new ArrayList<String>();
    for (var e : exits)
    {
      out.add(e.kind());
    }
    return out;
  }

  private static void expectSupervision(Object actual, Object wanted, String what)
  {
    if (!actual.equals(wanted))
    {
      throw new AssertionError(what + ": got " + actual + ", expected " + wanted);
    }
  }

  private static void sleepQuietly(long millis)
  {
    try
    {
      Thread.sleep(millis);
    }
    catch (InterruptedException e)
    {
      Thread.currentThread().interrupt();
    }
  }

  /**
   * A queue with many senders and one receiver: the channel form of an actor. A process that loops
   * over receive and answers each message is an actor written by hand; send never waits.
   */
  public static final class Mailbox<T>
  {
    private record Box<T>(T value)
    {}

    private final java.util.ArrayDeque<Box<T>> items = new java.util.ArrayDeque<>();
    private boolean closed;

    /** Sends a message; throws ActorStopped once the mailbox is closed. */
    public synchronized void send(T value)
    {
      if (closed)
      {
        throw new ActorStopped("the mailbox is closed");
      }
      items.add(new Box<>(value));
      notifyAll();
    }

    /** Waits for the next message, forever. */
    public T receive()
    {
      return receive(null);
    }

    /**
     * Waits up to timeout (forever when null) for the next message; throws
     * java.util.concurrent.TimeoutException wrapped in an IllegalStateException on timeout, and
     * ActorStopped once closed and empty.
     */
    public synchronized T receive(java.time.Duration timeout)
    {
      long deadline = timeout == null ? 0 : System.nanoTime() + timeout.toNanos();
      try
      {
        while (items.isEmpty() && !closed)
        {
          if (timeout == null)
          {
            wait();
          }
          else
          {
            long left = deadline - System.nanoTime();
            if (left <= 0)
            {
              throw new IllegalStateException(
                  new java.util.concurrent.TimeoutException("no message arrived in time"));
            }
            wait(left / 1_000_000, (int) (left % 1_000_000));
          }
        }
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        throw new IllegalStateException("interrupted while receiving", e);
      }
      if (items.isEmpty())
      {
        throw new ActorStopped("the mailbox is closed");
      }
      return items.poll().value();
    }

    /** receiveWithin on the real clock. */
    public java.util.Optional<T> receiveWithin(java.time.Duration within)
    {
      return receiveWithin(within, null);
    }

    /**
     * The Mailbox ability's receive ... within d: the next message, or empty when none arrives
     * within the duration. clock is a Clock ability handler (the real clock when null); on a
     * virtual one (any but the default real clock, as lawspec.time's registerClock tells) it waits
     * no real time: it takes a message already sent, or lets the time pass on that clock and gives
     * empty. Throws ActorStopped once closed and empty.
     */
    public java.util.Optional<T> receiveWithin(java.time.Duration within, Object clock)
    {
      var read = abilityClock(clock);
      if (read != null && read.virtual())
      {
        synchronized (this)
        {
          if (!items.isEmpty())
          {
            return java.util.Optional.ofNullable(items.poll().value());
          }
          if (closed)
          {
            throw new ActorStopped("the mailbox is closed");
          }
        }
        long micros =
            within.isNegative() ? 0 : within.getSeconds() * 1_000_000L + within.getNano() / 1000;
        read.sleep(micros);
        return java.util.Optional.empty();
      }
      synchronized (this)
      {
        long deadline = System.nanoTime() + (within.isNegative() ? 0 : within.toNanos());
        try
        {
          while (items.isEmpty() && !closed)
          {
            long left = deadline - System.nanoTime();
            if (left <= 0)
            {
              return java.util.Optional.empty();
            }
            wait(left / 1_000_000, (int) (left % 1_000_000));
          }
        }
        catch (InterruptedException e)
        {
          Thread.currentThread().interrupt();
          throw new IllegalStateException("interrupted while receiving", e);
        }
        if (items.isEmpty())
        {
          throw new ActorStopped("the mailbox is closed");
        }
        return java.util.Optional.ofNullable(items.poll().value());
      }
    }

    /** Refuses further messages; those already sent can still be received. */
    public synchronized void close()
    {
      closed = true;
      notifyAll();
    }
  }

  // Distribution. Values cross the network in a canonical binary encoding
  // driven by their type descriptor (the same descriptors as generation), so
  // no tags are sent and every target writes the same bytes:
  //   int: zigzag LEB128 of the integer (any size)      bool: 0 or 1
  //   text: LEB128 length, then UTF-8                    unit: nothing
  //   list: LEB128 count, then items                     maybe: 0, or 1 then the value
  //   either: 0 then left, or 1 then right               data: LEB128 constructor index, then
  // fields
  // A node sends frames over a Transport (in memory, TCP or HTTP): kind,
  // entity name, the sender's address, an id and a payload.

  /** Bytes that are not an encoding of a value of the expected type. */
  public static final class WireError extends IllegalArgumentException
  {
    public WireError(String message)
    {
      super(message);
    }
  }

  /** A node could not be reached, or did not answer in time. */
  public static final class Unreachable extends IllegalStateException
  {
    public Unreachable(String message)
    {
      super(message);
    }
  }

  private static void putVarint(java.io.ByteArrayOutputStream out, BigInteger n)
  {
    var seven = BigInteger.valueOf(0x7F);
    while (true)
    {
      int b = n.and(seven).intValue();
      n = n.shiftRight(7);
      if (n.signum() != 0)
      {
        out.write(b | 0x80);
      }
      else
      {
        out.write(b);
        return;
      }
    }
  }

  /** A position in bytes being decoded. */
  private static final class Reader
  {
    final byte[] buf;
    int pos;

    Reader(byte[] buf, int pos)
    {
      this.buf = buf;
      this.pos = pos;
    }

    int next()
    {
      if (pos >= buf.length)
      {
        throw new WireError("the bytes end in the middle of a value");
      }
      return buf[pos++] & 0xFF;
    }

    BigInteger varint()
    {
      var result = BigInteger.ZERO;
      int shift = 0;
      while (true)
      {
        int b = next();
        result = result.or(BigInteger.valueOf(b & 0x7F).shiftLeft(shift));
        if (b < 0x80)
        {
          return result;
        }
        shift += 7;
      }
    }

    byte[] take(int n)
    {
      if (n < 0 || pos + n > buf.length)
      {
        throw new WireError("the bytes end in the middle of a value");
      }
      var out = Arrays.copyOfRange(buf, pos, pos + n);
      pos += n;
      return out;
    }
  }

  private static BigInteger zigzag(BigInteger n)
  {
    return n.signum() >= 0 ? n.shiftLeft(1) : n.negate().shiftLeft(1).subtract(BigInteger.ONE);
  }

  private static BigInteger unzigzag(BigInteger z)
  {
    return z.testBit(0) ? z.add(BigInteger.ONE).shiftRight(1).negate() : z.shiftRight(1);
  }

  private static void putText(java.io.ByteArrayOutputStream out, String text)
  {
    var raw = text.getBytes(java.nio.charset.StandardCharsets.UTF_8);
    putVarint(out, BigInteger.valueOf(raw.length));
    out.writeBytes(raw);
  }

  private static String getText(Reader in)
  {
    int n = in.varint().intValueExact();
    var raw = in.take(n);
    var decoder =
        java.nio.charset.StandardCharsets.UTF_8
            .newDecoder()
            .onMalformedInput(java.nio.charset.CodingErrorAction.REPORT)
            .onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT);
    try
    {
      return decoder.decode(java.nio.ByteBuffer.wrap(raw)).toString();
    }
    catch (java.nio.charset.CharacterCodingException e)
    {
      throw new WireError("text that is not UTF-8");
    }
  }

  private static String textOf(Value v)
  {
    if (v.data() instanceof String s)
    {
      return s;
    }
    var out = new StringBuilder();
    for (Object unit : (List<?>) v.data())
    {
      out.appendCodePoint((Integer) unit);
    }
    return out.toString();
  }

  /** A text's logical value. */
  public static Value textValue(String text)
  {
    return sequence("Text", text.codePoints().toArray());
  }

  @SuppressWarnings("unchecked")
  private static void wirePut(
      Values values, Object descriptor, Value v, java.io.ByteArrayOutputStream out)
  {
    var d = values.resolve(endAsText(descriptor));
    switch (atomText(d.get(0)))
    {
      case "int" -> {
        if (!(v.data() instanceof BigInteger n))
        {
          throw new WireError(render(v) + " is not an integer");
        }
        var lo = (BigInteger) d.get(2);
        var hi = (BigInteger) d.get(3);
        if ((lo != null && n.compareTo(lo) < 0) || (hi != null && n.compareTo(hi) > 0))
        {
          throw new WireError(n + " is not a " + atomText(d.get(1)));
        }
        putVarint(out, zigzag(n));
      }
      case "bool" -> out.write(Boolean.TRUE.equals(v.data()) ? 1 : 0);
      case "text", "end" -> putText(out, textOf(v));
      case "unit" -> {}
      case "list" -> {
        var items = (List<Value>) v.data();
        putVarint(out, BigInteger.valueOf(items.size()));
        for (var item : items)
        {
          wirePut(values, d.get(1), item, out);
        }
      }
      case "maybe" -> {
        var data = (Data) v.data();
        if (data.tag().endsWith("Absent"))
        {
          out.write(0);
        }
        else
        {
          out.write(1);
          wirePut(values, d.get(1), data.fields().get(0), out);
        }
      }
      case "either" -> {
        var data = (Data) v.data();
        boolean left = data.tag().endsWith("Left");
        out.write(left ? 0 : 1);
        wirePut(values, left ? d.get(1) : d.get(2), data.fields().get(0), out);
      }
      case "data" -> {
        var data = (Data) v.data();
        for (int index = 2; index < d.size(); index++)
        {
          var ctor = form(d.get(index));
          if (atomText(ctor.get(1)).equals(data.tag()))
          {
            putVarint(out, BigInteger.valueOf(index - 2));
            for (int k = 2; k < ctor.size(); k++)
            {
              wirePut(values, ctor.get(k), data.fields().get(k - 2), out);
            }
            return;
          }
        }
        throw new WireError(data.tag() + " is not a constructor of " + atomText(d.get(1)));
      }
      default -> throw new WireError("unknown descriptor " + d);
    }
  }

  /** A step sending a channel end, (end), carries the end's address as text. */
  private static Object endAsText(Object descriptor)
  {
    if (descriptor instanceof List<?> l && !l.isEmpty() && atomText(l.get(0)).equals("end"))
    {
      return List.of("text");
    }
    return descriptor;
  }

  private static Value wireGet(Values values, Object descriptor, Reader in)
  {
    var d = values.resolve(endAsText(descriptor));
    var type = values.typeName(d);
    switch (atomText(d.get(0)))
    {
      case "int" -> {
        var n = unzigzag(in.varint());
        var lo = (BigInteger) d.get(2);
        var hi = (BigInteger) d.get(3);
        if ((lo != null && n.compareTo(lo) < 0) || (hi != null && n.compareTo(hi) > 0))
        {
          throw new WireError(n + " is out of range for " + atomText(d.get(1)));
        }
        return new Value(type, n);
      }
      case "bool" -> {
        int b = in.next();
        if (b > 1)
        {
          throw new WireError("not a Bool");
        }
        return bool(b == 1);
      }
      case "text", "end" -> {
        return textValue(getText(in));
      }
      case "unit" -> {
        return absent("Unit");
      }
      case "list" -> {
        int n = in.varint().intValueExact();
        var items = new ArrayList<Value>();
        for (int i = 0; i < n; i++)
        {
          items.add(wireGet(values, d.get(1), in));
        }
        return new Value(type, List.copyOf(items));
      }
      case "maybe" -> {
        int which = in.next();
        if (which > 1)
        {
          throw new WireError("not a Maybe");
        }
        if (which == 0)
        {
          return new Value(type, new Data("Maybe::Absent", List.of()));
        }
        return new Value(type, new Data("Maybe::Present", List.of(wireGet(values, d.get(1), in))));
      }
      case "either" -> {
        int which = in.next();
        if (which > 1)
        {
          throw new WireError("not an Either");
        }
        var inner = wireGet(values, which == 0 ? d.get(1) : d.get(2), in);
        return new Value(
            type, new Data(which == 0 ? "Either::Left" : "Either::Right", List.of(inner)));
      }
      case "data" -> {
        var index = in.varint();
        if (index.compareTo(BigInteger.valueOf(d.size() - 2)) >= 0)
        {
          throw new WireError("no constructor " + index + " in " + atomText(d.get(1)));
        }
        var ctor = form(d.get(2 + index.intValueExact()));
        var fields = new ArrayList<Value>();
        for (int k = 2; k < ctor.size(); k++)
        {
          fields.add(wireGet(values, ctor.get(k), in));
        }
        return new Value(type, new Data(atomText(ctor.get(1)), fields));
      }
      default -> throw new WireError("unknown descriptor " + d);
    }
  }

  // LawSpec's own search over a law's inputs (see LawSpec.Search). The
  // failure database keeps a failing case's inputs in the wire encoding,
  // under LAWSPEC_FAILURES/inputs, and they are replayed before the law's
  // next generated cases. target maximize climbs: it generates inputs from
  // seeds, keeps the best-scoring case, and moves its integers (one step,
  // doubling, halving the distance to a bound), keeping the best move while
  // the score rises and drawing afresh when it does not, checking the law on
  // every case it tries (at most 400). A case gives its score, or null when
  // its inputs fall outside the law's refinements.
  public interface SearchCase
  {
    Double check(List<Value> values) throws Exception;
  }

  private static java.nio.file.Path searchFile(String law)
  {
    var directory = System.getenv("LAWSPEC_FAILURES");
    if (directory == null || directory.isEmpty())
    {
      return null;
    }
    return java.nio.file.Path.of(
        directory, "inputs", law.replaceAll("[^A-Za-z0-9_-]", "_") + ".json");
  }

  public static void searchRemember(String law, List<String> descriptors, List<Value> values)
  {
    var file = searchFile(law);
    if (file == null)
    {
      return;
    }
    var inputs = new StringBuilder();
    try
    {
      for (int i = 0; i < descriptors.size(); i++)
      {
        var described = valuesFrom(descriptors.get(i));
        var bytes = wireEncode(described.values(), described.descriptor(), values.get(i));
        inputs
            .append(i == 0 ? "" : ",")
            .append('"')
            .append(java.util.HexFormat.of().formatHex(bytes))
            .append('"');
      }
      java.nio.file.Files.createDirectories(file.getParent());
      java.nio.file.Files.writeString(
          file,
          "{\"law\": \""
              + law.replace("\\", "\\\\").replace("\"", "\\\"")
              + "\", \"inputs\": ["
              + inputs
              + "]}");
    }
    catch (RuntimeException | java.io.IOException e)
    {
      // A value the wire cannot encode is kept by its seed only.
    }
  }

  public static void searchReplay(String law, List<String> descriptors, SearchCase check)
      throws Exception
  {
    var file = searchFile(law);
    if (file == null || !java.nio.file.Files.exists(file))
    {
      return;
    }
    var text = java.nio.file.Files.readString(file);
    var values = new ArrayList<Value>();
    try
    {
      var start = text.indexOf("\"inputs\"");
      var matcher =
          java.util.regex.Pattern.compile("\"([0-9a-fA-F]*)\"").matcher(text.substring(start + 8));
      for (var descriptor : descriptors)
      {
        if (!matcher.find())
        {
          break;
        }
        var described = valuesFrom(descriptor);
        values.add(
            wireDecode(
                described.values(),
                described.descriptor(),
                java.util.HexFormat.of().parseHex(matcher.group(1))));
      }
    }
    catch (RuntimeException e)
    {
      values.clear();
    }
    if (values.size() == descriptors.size())
    {
      System.out.println(law + ": replaying the failing inputs kept in .lawspec/failures");
      check.check(values);
    }
    java.nio.file.Files.deleteIfExists(file);
  }

  private static List<Value> searchMoves(Values table, Object descriptor, Value v)
  {
    var d = table.resolve(descriptor);
    var out = new ArrayList<Value>();
    switch (atomText(d.get(0)))
    {
      case "int" -> {
        if (!(v.data() instanceof BigInteger x))
        {
          return out;
        }
        var b = table.bounds(d);
        var two = BigInteger.TWO;
        var seen = new java.util.HashSet<BigInteger>();
        seen.add(x);
        for (var c :
            List.of(
                x.add(BigInteger.ONE),
                x.subtract(BigInteger.ONE),
                x.multiply(two),
                x.divide(two),
                x.add(b[1].subtract(x).divide(two)),
                x.subtract(x.subtract(b[0]).divide(two))))
        {
          if (c.compareTo(b[0]) >= 0 && c.compareTo(b[1]) <= 0 && seen.add(c))
          {
            out.add(new Value(v.type(), c));
          }
        }
      }
      case "list" -> {
        if (!(v.data() instanceof List<?> items))
        {
          return out;
        }
        for (int i = 0; i < items.size(); i++)
        {
          for (var m : searchMoves(table, d.get(1), (Value) items.get(i)))
          {
            var changed = new ArrayList<Object>(items);
            changed.set(i, m);
            out.add(new Value(v.type(), List.copyOf(changed)));
          }
        }
      }
      case "maybe", "either", "data" -> {
        if (!(v.data() instanceof Data data))
        {
          return out;
        }
        List<Object> fields = new ArrayList<>();
        switch (atomText(d.get(0)))
        {
          case "maybe" -> fields.add(d.get(1));
          case "either" -> fields.add(data.tag().equals("Either::Left") ? d.get(1) : d.get(2));
          default -> {
            for (var c : d.subList(2, d.size()))
            {
              var ctor = form(c);
              if (atomText(ctor.get(1)).equals(data.tag()))
              {
                fields = ctor.subList(2, ctor.size());
              }
            }
          }
        }
        for (int i = 0; i < fields.size() && i < data.fields().size(); i++)
        {
          for (var m : searchMoves(table, fields.get(i), data.fields().get(i)))
          {
            var changed = new ArrayList<Value>(data.fields());
            changed.set(i, m);
            out.add(new Value(v.type(), new Data(data.tag(), List.copyOf(changed))));
          }
        }
      }
      default -> {}
    }
    return out;
  }

  public static void searchClimb(String law, List<String> descriptors, SearchCase check)
      throws Exception
  {
    var tables = new ArrayList<Described>();
    for (var text : descriptors)
    {
      tables.add(valuesFrom(text));
    }
    long base = 0;
    try
    {
      base = Long.parseLong(System.getenv().getOrDefault("LAWSPEC_SEED", "0"));
    }
    catch (NumberFormatException e)
    {
      // A malformed seed is seed 0, as an unset one is.
    }
    final long seed0 = base;
    java.util.function.LongFunction<List<Value>> generate =
        seed -> {
          var random = new SplitMix64(seed);
          var out = new ArrayList<Value>();
          for (var t : tables)
          {
            out.add(t.values().generate(t.descriptor(), random, 8));
          }
          return out;
        };
    var state =
        new Object()
        {
          List<Value> best = null;
          double bestScore = Double.NEGATIVE_INFINITY;
          int tried = 0;
        };
    SearchCase attempt =
        values -> {
          if (state.tried >= 400)
          {
            return null;
          }
          state.tried++;
          Double score;
          try
          {
            score = check.check(values);
          }
          catch (Exception | Error e)
          {
            searchRemember(law, descriptors, values);
            throw e;
          }
          if (score == null || (state.best != null && !(score > state.bestScore)))
          {
            return null;
          }
          state.best = values;
          state.bestScore = score;
          return score;
        };
    for (long k = 0; k < 10; k++)
    {
      attempt.check(generate.apply(seed0 + k));
    }
    for (long step = 0; step < 60 && state.tried < 400; step++)
    {
      boolean rose = false;
      if (state.best != null)
      {
        var current = state.best;
        var moves = new ArrayList<List<Value>>();
        for (int i = 0; i < current.size(); i++)
        {
          for (var m :
              searchMoves(tables.get(i).values(), tables.get(i).descriptor(), current.get(i)))
          {
            var changed = new ArrayList<Value>(current);
            changed.set(i, m);
            moves.add(changed);
          }
        }
        for (var candidate : moves.subList(0, Math.min(24, moves.size())))
        {
          if (attempt.check(candidate) != null)
          {
            rose = true;
          }
        }
      }
      if (!rose)
      {
        attempt.check(generate.apply(seed0 + 1000 + step));
      }
    }
    System.out.println(
        law
            + ": targeted search tried "
            + state.tried
            + " case(s)"
            + (state.best == null ? "" : "; best score " + state.bestScore));
  }

  /** A score as a double: the targeted search maximizes it. */
  public static Double searchNumber(Value v)
  {
    if (v.data() instanceof Number n)
    {
      return n.doubleValue();
    }
    try
    {
      return Double.parseDouble(String.valueOf(v.data()));
    }
    catch (NumberFormatException e)
    {
      return Double.NaN;
    }
  }

  /** The value's canonical bytes. */
  public static byte[] wireEncode(Values values, Object descriptor, Value v)
  {
    var out = new java.io.ByteArrayOutputStream();
    wirePut(values, descriptor, v, out);
    return out.toByteArray();
  }

  /** The value encoded by exactly these bytes. */
  public static Value wireDecode(Values values, Object descriptor, byte[] data)
  {
    var in = new Reader(data, 0);
    var v = wireGet(values, descriptor, in);
    if (in.pos != data.length)
    {
      throw new WireError("extra bytes after the value");
    }
    return v;
  }

  private static String hex(byte[] bytes)
  {
    var out = new StringBuilder();
    for (byte b : bytes)
    {
      out.append(String.format("%02x", b & 0xFF));
    }
    return out.toString();
  }

  /** count values generated from one seed, encoded, in hexadecimal. */
  public static List<String> wireEncoded(String text, long seed, long size, long count)
  {
    var described = valuesFrom(text);
    var random = new SplitMix64(seed);
    var result = new ArrayList<String>();
    for (long i = 0; i < count; i++)
    {
      result.add(
          hex(
              wireEncode(
                  described.values(),
                  described.descriptor(),
                  described.values().generate(described.descriptor(), random, size))));
    }
    return result;
  }

  /** Whether count generated values decode to themselves. */
  public static boolean wireRoundTrips(String text, long seed, long size, long count)
  {
    var described = valuesFrom(text);
    var random = new SplitMix64(seed);
    for (long i = 0; i < count; i++)
    {
      var v = described.values().generate(described.descriptor(), random, size);
      var back =
          wireDecode(
              described.values(),
              described.descriptor(),
              wireEncode(described.values(), described.descriptor(), v));
      if (!render(back).equals(render(v)))
      {
        return false;
      }
    }
    return true;
  }

  private static final Values NO_TYPES = new Values(Map.of());

  private record Frame(String kind, String to, String source, long id, byte[] payload)
  {}

  private static byte[] frameEncode(
      String kind, String to, String source, long id, byte[] payload)
  {
    var out = new java.io.ByteArrayOutputStream();
    putText(out, kind);
    putText(out, to);
    putText(out, source);
    putVarint(out, zigzag(new BigInteger(Long.toUnsignedString(id))));
    putVarint(out, BigInteger.valueOf(payload.length));
    out.writeBytes(payload);
    return out.toByteArray();
  }

  private static Frame frameDecode(byte[] data)
  {
    var in = new Reader(data, 0);
    var kind = getText(in);
    var to = getText(in);
    var source = getText(in);
    var id = unzigzag(in.varint());
    if (id.signum() < 0)
    {
      throw new WireError("a frame's id is negative");
    }
    var payload = in.take(in.varint().intValueExact());
    if (in.pos != data.length)
    {
      throw new WireError("extra bytes after a frame");
    }
    return new Frame(kind, to, source, id.longValue(), payload);
  }

  /** 'tcp://host:port/name' as {'tcp://host:port', 'name'}. */
  private static String[] splitAddress(String address)
  {
    int cut = address.lastIndexOf('/');
    String node = cut < 0 ? "" : address.substring(0, cut);
    if (node.isEmpty() || !node.contains("://"))
    {
      throw new IllegalArgumentException(
          address + " is not an address such as tcp://127.0.0.1:7000/name");
    }
    return new String[] {node, address.substring(cut + 1)};
  }

  private static void startDaemon(Runnable task)
  {
    Thread.ofVirtual().start(task);
  }

  /**
   * Moves frames between nodes. start(deliver) begins calling deliver for every frame that arrives;
   * send(node, frame) sends one to the node at that address, best effort; close() stops.
   */
  public interface Transport
  {
    String address();

    void start(java.util.function.Consumer<byte[]> deliver);

    void send(String node, byte[] frame);

    default void close()
    {}
  }

  /**
   * Nodes in one process, with faults for testing: each frame may be lost or duplicated, and is
   * delayed by up to delay seconds (so frames can overtake each other); partition(...) cuts nodes
   * off until heal().
   */
  public static final class MemoryNetwork
  {
    private final SplitMix64 random;
    private final double loss;
    private final double duplicate;
    private final double delay;
    private final Map<String, java.util.function.Consumer<byte[]>> nodes =
        new java.util.HashMap<>();
    private List<java.util.Set<String>> groups;
    // With record, every record sent, as the network saw it; else null.
    private final List<byte[]> recorded;

    public MemoryNetwork(long seed, double loss, double duplicate, double delay)
    {
      this(seed, loss, duplicate, delay, false);
    }

    /** record: keep every record sent (see recorded). */
    public MemoryNetwork(long seed, double loss, double duplicate, double delay, boolean record)
    {
      this.random = new SplitMix64(seed);
      this.loss = loss;
      this.duplicate = duplicate;
      this.delay = delay;
      this.recorded = record ? new ArrayList<>() : null;
    }

    public MemoryNetwork()
    {
      this(0, 0, 0, 0);
    }

    public Transport transport(String name)
    {
      return new MemoryTransport(this, "mem://" + name);
    }

    /**
     * A transport whose node skips the handshake and sends frames in the clear: for tests of the
     * frame layer only. Only an in-memory network makes one, and no configuration selects it.
     */
    public InsecureMemoryTransport insecureTransportForTests(String name)
    {
      return new InsecureMemoryTransport(this, "mem://" + name);
    }

    /** Every record sent so far, in order (empty unless made with record). */
    public synchronized List<byte[]> recorded()
    {
      var copy = new ArrayList<byte[]>();
      if (recorded != null)
      {
        for (var each : recorded)
        {
          copy.add(each.clone());
        }
      }
      return copy;
    }

    /** Only nodes named in the same group reach each other. */
    public synchronized void partition(List<List<String>> named)
    {
      groups = new ArrayList<>();
      for (var g : named)
      {
        var set = new java.util.HashSet<String>();
        for (var n : g)
        {
          set.add("mem://" + n);
        }
        groups.add(set);
      }
    }

    public synchronized void heal()
    {
      groups = null;
    }

    private boolean chance(double p)
    {
      return p > 0 && random.below(1L << 30) < p * (1L << 30);
    }

    private void deliver(String source, String node, byte[] frame)
    {
      java.util.function.Consumer<byte[]> deliver;
      long[] waits;
      synchronized (this)
      {
        if (recorded != null)
        {
          recorded.add(frame.clone());
        }
        deliver = nodes.get(node);
        if (deliver == null)
        {
          throw new Unreachable("no node at " + node);
        }
        if (groups != null)
        {
          boolean together = false;
          for (var g : groups)
          {
            if (g.contains(source) && g.contains(node))
            {
              together = true;
            }
          }
          if (!together)
          {
            return;
          }
        }
        if (chance(loss))
        {
          return;
        }
        int copies = chance(duplicate) ? 2 : 1;
        waits = new long[copies];
        for (int i = 0; i < copies; i++)
        {
          waits[i] = (long) (random.below(1001) * delay * 1_000_000.0);
        }
      }
      for (long wait : waits)
      {
        startDaemon(
            () -> {
              if (wait > 0)
              {
                try
                {
                  Thread.sleep(wait / 1_000_000, (int) (wait % 1_000_000));
                }
                catch (InterruptedException e)
                {
                  return;
                }
              }
              deliver.accept(frame);
            });
      }
    }
  }

  /** A node's place on a MemoryNetwork (MemoryNetwork.transport). */
  public static class MemoryTransport implements Transport
  {
    private final MemoryNetwork network;
    private final String address;

    MemoryTransport(MemoryNetwork network, String address)
    {
      this.network = network;
      this.address = address;
    }

    public String address()
    {
      return address;
    }

    public void start(java.util.function.Consumer<byte[]> deliver)
    {
      synchronized (network)
      {
        network.nodes.put(address, deliver);
      }
    }

    public void send(String node, byte[] frame)
    {
      network.deliver(address, node, frame);
    }

    public void close()
    {
      synchronized (network)
      {
        network.nodes.remove(address);
      }
    }
  }

  /** In memory, without the handshake: tests only (MemoryNetwork.insecureTransportForTests). */
  public static final class InsecureMemoryTransport extends MemoryTransport
  {
    InsecureMemoryTransport(MemoryNetwork network, String address)
    {
      super(network, address);
    }
  }

  /** Frames over TCP, each a 4-byte big-endian length then the frame. */
  public static final class TcpTransport implements Transport
  {
    private final java.net.ServerSocket server;
    private final String address;
    private final Map<String, java.net.Socket> connections = new java.util.HashMap<>();
    private volatile boolean closed;

    public TcpTransport()
    {
      this("127.0.0.1", 0);
    }

    public TcpTransport(String host, int port)
    {
      try
      {
        server = new java.net.ServerSocket(port, 50, java.net.InetAddress.getByName(host));
      }
      catch (java.io.IOException e)
      {
        throw new Unreachable("cannot listen on " + host + ":" + port + ": " + e.getMessage());
      }
      address = "tcp://" + host + ":" + server.getLocalPort();
    }

    public String address()
    {
      return address;
    }

    public void start(java.util.function.Consumer<byte[]> deliver)
    {
      startDaemon(
          () -> {
            while (!closed)
            {
              java.net.Socket connection;
              try
              {
                connection = server.accept();
              }
              catch (java.io.IOException e)
              {
                return;
              }
              startDaemon(() -> read(connection, deliver));
            }
          });
    }

    private static void read(
        java.net.Socket connection, java.util.function.Consumer<byte[]> deliver)
    {
      try (connection;
          var in =
              new java.io.DataInputStream(
                  new java.io.BufferedInputStream(connection.getInputStream())))
      {
        while (true)
        {
          int n = in.readInt();
          var frame = new byte[n];
          in.readFully(frame);
          deliver.accept(frame);
        }
      }
      catch (java.io.IOException e)
      {
        // The connection closed.
      }
    }

    public synchronized void send(String node, byte[] frame)
    {
      String rest = node.substring("tcp://".length());
      int colon = rest.lastIndexOf(':');
      for (int attempt = 0; attempt < 2; attempt++)
      {
        var connection = connections.get(node);
        try
        {
          if (connection == null)
          {
            connection = new java.net.Socket();
            connection.connect(
                new java.net.InetSocketAddress(
                    rest.substring(0, colon), Integer.parseInt(rest.substring(colon + 1))),
                5000);
            connections.put(node, connection);
          }
          var out = new java.io.DataOutputStream(connection.getOutputStream());
          out.writeInt(frame.length);
          out.write(frame);
          out.flush();
          return;
        }
        catch (java.io.IOException e)
        {
          connections.remove(node);
          if (attempt == 1)
          {
            throw new Unreachable("cannot reach " + node + ": " + e.getMessage());
          }
        }
      }
    }

    public synchronized void close()
    {
      closed = true;
      try
      {
        server.close();
      }
      catch (java.io.IOException e)
      {
        // Already closed.
      }
      for (var c : connections.values())
      {
        try
        {
          c.close();
        }
        catch (java.io.IOException e)
        {
          // Already closed.
        }
      }
      connections.clear();
    }
  }

  /** Frames as HTTP POST bodies to /lawspec. */
  public static final class HttpTransport implements Transport
  {
    private final com.sun.net.httpserver.HttpServer server;
    private final String address;
    private final java.net.http.HttpClient client =
        java.net.http.HttpClient.newBuilder()
            .connectTimeout(java.time.Duration.ofSeconds(5))
            .build();
    private volatile java.util.function.Consumer<byte[]> deliver;

    public HttpTransport()
    {
      this("127.0.0.1", 0);
    }

    public HttpTransport(String host, int port)
    {
      try
      {
        server =
            com.sun.net.httpserver.HttpServer.create(new java.net.InetSocketAddress(host, port), 0);
      }
      catch (java.io.IOException e)
      {
        throw new Unreachable("cannot listen on " + host + ":" + port + ": " + e.getMessage());
      }
      address = "http://" + host + ":" + server.getAddress().getPort();
      server.createContext(
          "/",
          exchange -> {
            var body = exchange.getRequestBody().readAllBytes();
            boolean ours =
                exchange.getRequestMethod().equals("POST")
                    && exchange.getRequestURI().getPath().equals("/lawspec");
            exchange.sendResponseHeaders(ours ? 204 : 404, -1);
            exchange.close();
            var target = deliver;
            if (ours && target != null)
            {
              target.accept(body);
            }
          });
      server.setExecutor(java.util.concurrent.Executors.newVirtualThreadPerTaskExecutor());
    }

    public String address()
    {
      return address;
    }

    public void start(java.util.function.Consumer<byte[]> deliver)
    {
      this.deliver = deliver;
      server.start();
    }

    public void send(String node, byte[] frame)
    {
      var request =
          java.net.http.HttpRequest.newBuilder(java.net.URI.create(node + "/lawspec"))
              .timeout(java.time.Duration.ofSeconds(5))
              .header("Content-Type", "application/octet-stream")
              .POST(java.net.http.HttpRequest.BodyPublishers.ofByteArray(frame))
              .build();
      try
      {
        client.send(request, java.net.http.HttpResponse.BodyHandlers.discarding());
      }
      catch (java.io.IOException e)
      {
        throw new Unreachable("cannot reach " + node + ": " + e.getMessage());
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        throw new Unreachable("interrupted while sending to " + node);
      }
    }

    public void close()
    {
      server.stop(0);
    }
  }

  /** Something a node names: a mailbox, an actor, definitions or a channel end. */
  private interface Entity
  {
    void receive(Node node, String kind, String source, long id, byte[] payload);
  }

  /** A message an actor serves: its handler (on the actor's state) and its types. */
  public record Served<S>(
      java.util.function.BiFunction<S, List<Value>, Next<Value, S>> handler,
      List<Object> arguments,
      Object reply)
  {}

  /** A message's types, for calling an actor on another node. */
  public record Signature(List<Object> arguments, Object reply)
  {}

  /** A definition other nodes can evaluate: the function and its types. */
  public record Definition(
      Function<List<Value>, Value> function, List<Object> arguments, Object result)
  {}

  // The secure network handler lives in LawSpecNetwork, which the compiler
  // writes beside this runtime when a program imports lawspec.network: it
  // needs the JDK's ML-KEM and ML-DSA and Bouncy Castle's SHAKE256, which
  // programs without nodes do without. Its static initializer registers it
  // here (registerSecureNetwork); a node loads it when the slot is empty.

  /** Makes a node's secure layer (LawSpecNetwork registers one). */
  public interface SecureNetwork
  {
    /**
     * The layer of node: identity is a LawSpecNetwork.NodeIdentity or null (the configured one, or
     * a fresh one); trusted the fingerprints of the only peers to talk to, or null.
     */
    SecureLayer layer(Node node, Object identity, java.util.Collection<String> trusted);
  }

  /** Handshakes, sessions and sealed frames for one node. */
  public interface SecureLayer
  {
    /** Sends frame to the node at peer, sealed (after a handshake when there is no session yet). */
    void send(String peer, byte[] frame);

    /**
     * The frame a record carries, or null (a handshake record, or one that fails to verify or
     * open).
     */
    byte[] receive(byte[] record);

    /** The node's identity (a LawSpecNetwork.NodeIdentity). */
    Object identity();
  }

  private static volatile SecureNetwork secureNetwork;

  /** Installs the secure network handler: LawSpecNetwork's static initializer calls it. */
  public static void registerSecureNetwork(SecureNetwork provider)
  {
    secureNetwork = provider;
  }

  private static SecureNetwork secureNetwork()
  {
    if (secureNetwork == null)
    {
      try
      {
        // Its static initializer registers it.
        Class.forName(
            "lawspec.runtime.LawSpecNetwork", true, LawSpecRuntime.class.getClassLoader());
      }
      catch (ClassNotFoundException missing)
      {
        // Not generated: the program does not import lawspec.network.
      }
    }
    var provider = secureNetwork;
    if (provider == null)
    {
      throw new IllegalStateException(
          "a node needs the secure network handler: add `import lawspec.network` "
              + "to a unit of the program, so LawSpecNetwork is generated");
    }
    return provider;
  }

  private static final class SecureRandomHolder
  {
    static final java.security.SecureRandom SECURE = new java.security.SecureRandom();
  }

  /**
   * A one-time token from the operating system's secure generator: 32 bytes as 64 hexadecimal
   * digits, as SecureRandom's secureToken gives.
   */
  public static String secureToken()
  {
    byte[] out = new byte[32];
    SecureRandomHolder.SECURE.nextBytes(out);
    return java.util.HexFormat.of().formatHex(out);
  }

  /**
   * A process's presence on a network: it names local mailboxes, actors, channel ends and
   * definitions, so other nodes can reach them at {node address}/{name}, and it sends to theirs.
   * Order is kept within one channel; a mailbox or an actor call is best effort: a lost call fails
   * with Unreachable after its timeout.
   *
   * <p>Every target's node speaks the same frames, so nodes written in different languages talk to
   * each other. ref:DEC-distribution-canonical-wire
   */
  public static final class Node
  {
    final Transport transport;
    public final String address;
    private final Map<String, Entity> entities = new java.util.concurrent.ConcurrentHashMap<>();
    private final Map<Long, java.util.concurrent.CompletableFuture<byte[]>> pending =
        new java.util.concurrent.ConcurrentHashMap<>();
    // Requests already seen, by sender and id, with their reply once sent: a
    // request sent again (lost reply, duplicated frame) is answered again
    // without running twice.
    private final java.util.LinkedHashMap<String, byte[]> seen = new java.util.LinkedHashMap<>();
    private final java.util.concurrent.atomic.AtomicLong ids =
        new java.util.concurrent.atomic.AtomicLong();
    volatile boolean closed;
    // Handshakes, sessions and sealed frames; null on the insecure transport for tests.
    private final SecureLayer secure;

    /**
     * This node's identity (a LawSpecNetwork.NodeIdentity; LawSpecNetwork.identity(node) gives it
     * typed); null on the insecure transport for tests.
     */
    public final Object identity;

    public Node(Transport transport)
    {
      this(transport, null, null);
    }

    /**
     * identity: a LawSpecNetwork.NodeIdentity, by default the one lawspec.edn binds
     * (lawspec-network.conf) or a fresh one; trusted: the fingerprints of the only peers to talk to
     * (by default any peer, each address keeping the first identity it shows). A node needs the
     * secure network handler (`import lawspec.network`), except on a transport made for tests only
     * (MemoryNetwork.insecureTransportForTests), which skips the handshake; no other transport can.
     * LawSpecNetwork.node(transport, identity, trusted) is the typed form.
     */
    public Node(Transport transport, Object identity, java.util.Collection<String> trusted)
    {
      this.transport = transport;
      this.address = transport.address();
      this.secure =
          transport instanceof InsecureMemoryTransport
              ? null
              : secureNetwork().layer(this, identity, trusted);
      this.identity = secure == null ? null : secure.identity();
      transport.start(this::arrive);
    }

    private void arrive(byte[] record)
    {
      if (secure == null)
      {
        deliver(record);
        return;
      }
      byte[] frame = secure.receive(record);
      if (frame != null)
      {
        deliver(frame);
      }
    }

    private void transmit(String node, byte[] frame)
    {
      if (secure == null)
      {
        transport.send(node, frame);
      }
      else
      {
        secure.send(node, frame);
      }
    }

    public void close()
    {
      closed = true;
      transport.close();
    }

    long nextId()
    {
      return ids.incrementAndGet();
    }

    void send(String address, String kind, byte[] payload, long id)
    {
      var parts = splitAddress(address);
      transmit(parts[0], frameEncode(kind, parts[1], this.address, id, payload));
    }

    String register(String name, Entity entity)
    {
      if (name.isEmpty() || name.contains("/"))
      {
        throw new IllegalArgumentException(name + " is not a name: use letters, digits and dashes");
      }
      if (entities.putIfAbsent(name, entity) != null)
      {
        throw new IllegalArgumentException(name + " is already registered on " + address);
      }
      return address + "/" + name;
    }

    private void deliver(byte[] data)
    {
      Frame frame;
      try
      {
        frame = frameDecode(data);
      }
      catch (RuntimeException e)
      {
        return;
      }
      if (frame.kind().equals("reply"))
      {
        var slot = pending.remove(frame.id());
        if (slot != null)
        {
          slot.complete(frame.payload());
        }
        return;
      }
      var entity = entities.get(frame.to());
      if (entity == null)
      {
        if (frame.id() != 0)
        {
          reply(
              frame.source(),
              frame.id(),
              3,
              utf8("nothing is registered as " + frame.to() + " on " + address));
        }
        return;
      }
      if (frame.id() != 0)
      {
        String key = frame.source() + "#" + frame.id();
        byte[] answer;
        synchronized (seen)
        {
          if (seen.containsKey(key))
          {
            answer = seen.get(key);
            if (answer != null)
            {
              var again = answer;
              startDaemon(
                  () -> {
                    try
                    {
                      send(frame.source() + "/", "reply", again, frame.id());
                    }
                    catch (RuntimeException e)
                    {
                      // Unreachable: the sender asks again.
                    }
                  });
            }
            return;
          }
          seen.put(key, null);
          if (seen.size() > 10000)
          {
            var it = seen.keySet().iterator();
            for (int i = 0; i < 5000 && it.hasNext(); i++)
            {
              it.next();
              it.remove();
            }
          }
        }
      }
      // Handled off the transport's thread, so a slow handler does not hold
      // up other frames.
      startDaemon(
          () -> entity.receive(this, frame.kind(), frame.source(), frame.id(), frame.payload()));
    }

    void reply(String source, long id, int status, byte[] body)
    {
      var payload = new byte[body.length + 1];
      payload[0] = (byte) status;
      System.arraycopy(body, 0, payload, 1, body.length);
      synchronized (seen)
      {
        String key = source + "#" + id;
        if (seen.containsKey(key))
        {
          seen.put(key, payload);
        }
      }
      try
      {
        send(source + "/", "reply", payload, id);
      }
      catch (RuntimeException e)
      {
        // Unreachable: the sender asks again.
      }
    }

    /** Sends a request until answered (the receiver runs it once): its reply. */
    byte[] request(String address, String kind, byte[] payload, double timeout)
    {
      long id = nextId();
      var slot = new java.util.concurrent.CompletableFuture<byte[]>();
      pending.put(id, slot);
      long giveUp = System.nanoTime() + (long) (timeout * 1e9);
      while (true)
      {
        try
        {
          send(address, kind, payload, id);
        }
        catch (Unreachable e)
        {
          // Tried again below.
        }
        long left = giveUp - System.nanoTime();
        try
        {
          return slot.get(
              Math.max(0, Math.min(100_000_000L, left)), java.util.concurrent.TimeUnit.NANOSECONDS);
        }
        catch (java.util.concurrent.TimeoutException e)
        {
          if (System.nanoTime() >= giveUp)
          {
            pending.remove(id);
            throw new Unreachable(address + " did not answer within " + timeout + "s");
          }
        }
        catch (InterruptedException e)
        {
          Thread.currentThread().interrupt();
          pending.remove(id);
          throw new Unreachable("interrupted while waiting for " + address);
        }
        catch (java.util.concurrent.ExecutionException e)
        {
          throw new IllegalStateException(e.getCause());
        }
      }
    }

    // Mailboxes: values of one type sent by any node.

    /** A local Mailbox that other nodes send to at {address}/name. */
    public Mailbox<Value> mailbox(String name, Object descriptor, Values values)
    {
      var box = new Mailbox<Value>();
      register(
          name,
          (node, kind, source, id, payload) -> {
            if (!kind.equals("mail"))
            {
              return;
            }
            int status = 0;
            byte[] body = new byte[0];
            try
            {
              box.send(wireDecode(values, descriptor, payload));
            }
            catch (ActorStopped e)
            {
              status = 2;
              body = utf8(String.valueOf(e.getMessage()));
            }
            catch (RuntimeException e)
            {
              status = 3;
              body = utf8("not a message of this mailbox: " + e.getMessage());
            }
            if (id != 0)
            {
              node.reply(source, id, status, body);
            }
          });
      return box;
    }

    public RemoteMailbox remoteMailbox(String address, Object descriptor, Values values)
    {
      return new RemoteMailbox(this, address, descriptor, values, 5.0);
    }

    public RemoteMailbox remoteMailbox(
        String address, Object descriptor, Values values, double timeout)
    {
      return new RemoteMailbox(this, address, descriptor, values, timeout);
    }

    // Actors: calls by message name, with each message's types.

    /**
     * Lets other nodes call actor at {address}/name: handlers maps a message name to its handler
     * and types.
     */
    public <S> String serve(
        String name, Actor<S> actor, Map<String, Served<S>> handlers, Values values)
    {
      return register(
          name,
          (node, kind, source, id, payload) -> {
            if (!kind.equals("call"))
            {
              return;
            }
            Served<S> served;
            List<Value> args = new ArrayList<>();
            try
            {
              var in = new Reader(payload, 0);
              String message = getText(in);
              served = handlers.get(message);
              if (served == null)
              {
                throw new WireError("no message " + message);
              }
              for (var d : served.arguments())
              {
                args.add(wireGet(values, d, in));
              }
              if (in.pos != payload.length)
              {
                throw new WireError("extra bytes after the arguments");
              }
            }
            catch (RuntimeException e)
            {
              node.reply(
                  source, id, 3, utf8("not a message this actor handles: " + e.getMessage()));
              return;
            }
            try
            {
              var result = actor.call(s -> served.handler().apply(s, args));
              node.reply(source, id, 0, wireEncode(values, served.reply(), result));
            }
            catch (ActorCrashed e)
            {
              node.reply(source, id, 1, utf8(e.getMessage()));
            }
            catch (ActorStopped e)
            {
              node.reply(source, id, 2, utf8(e.getMessage()));
            }
          });
    }

    /** A proxy calling the actor at address. */
    public RemoteActor remoteActor(
        String address, Map<String, Signature> signatures, Values values, double timeout)
    {
      return new RemoteActor(this, address, signatures, values, timeout);
    }

    // Definitions, by content hash.

    /** Lets other nodes evaluate definitions: table maps a content hash to a definition. */
    public String serveDefinitions(Map<String, Definition> table, Values values)
    {
      return serveDefinitions(table, values, "definitions");
    }

    public String serveDefinitions(Map<String, Definition> table, Values values, String name)
    {
      return register(
          name,
          (node, kind, source, id, payload) -> {
            if (!kind.equals("eval"))
            {
              return;
            }
            Definition definition;
            var args = new ArrayList<Value>();
            try
            {
              var in = new Reader(payload, 0);
              definition = table.get(getText(in));
              if (definition == null)
              {
                throw new WireError("unknown hash");
              }
              for (var d : definition.arguments())
              {
                args.add(wireGet(values, d, in));
              }
            }
            catch (RuntimeException e)
            {
              node.reply(source, id, 3, utf8("this node has no definition with that content hash"));
              return;
            }
            try
            {
              node.reply(
                  source,
                  id,
                  0,
                  wireEncode(values, definition.result(), definition.function().apply(args)));
            }
            catch (RuntimeException e)
            {
              node.reply(source, id, 1, utf8(e.getClass().getSimpleName() + ": " + e.getMessage()));
            }
          });
    }

    /** Evaluates the definition with this content hash on another node. */
    public Value evaluate(
        String node,
        String digest,
        List<Value> args,
        List<Object> arguments,
        Object result,
        Values values,
        double timeout)
    {
      var out = new java.io.ByteArrayOutputStream();
      putText(out, digest);
      for (int i = 0; i < arguments.size(); i++)
      {
        wirePut(values, arguments.get(i), args.get(i), out);
      }
      return replyValue(
          request(node + "/definitions", "eval", out.toByteArray(), timeout), values, result);
    }

    // Channels: one side here, the other on any node.

    /**
     * The first end of a channel named name here; its other end is dial(...)ed from any node.
     * steps: (sends, descriptor) per step, from this end's side.
     */
    public NetEndpoint listen(String name, List<Step> steps, Values values)
    {
      var endpoint = new NetEndpoint(this, steps, values, 5.0);
      endpoint.address = register(name, endpoint);
      return endpoint;
    }

    /** The second end of the channel listening at address; steps are from this end's side. */
    public NetEndpoint dial(String address, List<Step> steps, Values values)
    {
      var endpoint = new NetEndpoint(this, steps, values, 5.0);
      endpoint.address = register("end-" + nextId(), endpoint);
      endpoint.connect(address);
      return endpoint;
    }

    /**
     * Takes over a channel end another node moves here: address is {old address}?take={token}, as
     * that node offered it. Returns once the end's state has arrived and its peer has been told (or
     * after the deadline; the old node then forwards to the end).
     */
    public NetEndpoint take(String address, List<Step> steps, Values values)
    {
      var endpoint = new NetEndpoint(this, steps, values, 5.0);
      endpoint.address = register("end-" + nextId(), endpoint);
      endpoint.takeOver(address);
      return endpoint;
    }

    /** Passes a frame on to address unchanged, keeping its source. */
    void forward(String address, String kind, String source, long id, byte[] payload)
    {
      try
      {
        var parts = splitAddress(address);
        transmit(parts[0], frameEncode(kind, parts[1], source, id, payload));
      }
      catch (RuntimeException e)
      {
        // The sender sends again.
      }
    }
  }

  /** A protocol step: whether this end sends, and the value's descriptor. */
  public record Step(boolean sends, Object descriptor)
  {}

  private static byte[] utf8(String text)
  {
    return text.getBytes(java.nio.charset.StandardCharsets.UTF_8);
  }

  private static Value replyValue(byte[] payload, Values values, Object descriptor)
  {
    int status = payload[0] & 0xFF;
    var body = Arrays.copyOfRange(payload, 1, payload.length);
    if (status == 0)
    {
      return wireDecode(values, descriptor, body);
    }
    String message = new String(body, java.nio.charset.StandardCharsets.UTF_8);
    if (status == 1)
    {
      throw new ActorCrashed(new IllegalStateException(message));
    }
    if (status == 2)
    {
      throw new ActorStopped(message);
    }
    throw new Unreachable(message);
  }

  /**
   * Sends to a mailbox on another node. A send waits until the mailbox has the message (resending a
   * lost one; the mailbox takes it once), and throws Unreachable after the timeout, or ActorStopped
   * if it is closed.
   */
  public static final class RemoteMailbox
  {
    private final Node node;
    public final String address;
    private final Object descriptor;
    private final Values values;
    private final double timeout;

    RemoteMailbox(Node node, String address, Object descriptor, Values values, double timeout)
    {
      this.node = node;
      this.address = address;
      this.descriptor = descriptor;
      this.values = values;
      this.timeout = timeout;
    }

    public void send(Value value)
    {
      var reply = node.request(address, "mail", wireEncode(values, descriptor, value), timeout);
      if ((reply[0] & 0xFF) != 0)
      {
        replyValue(reply, values, List.of("unit"));
      }
    }
  }

  /**
   * Calls an actor on another node: call(message, args) sends the message and waits for the reply,
   * throwing Unreachable after the timeout, or what the actor's call threw (ActorCrashed,
   * ActorStopped).
   */
  public static final class RemoteActor
  {
    private final Node node;
    public final String address;
    private final Map<String, Signature> signatures;
    private final Values values;
    private final double timeout;

    RemoteActor(
        Node node,
        String address,
        Map<String, Signature> signatures,
        Values values,
        double timeout)
    {
      this.node = node;
      this.address = address;
      this.signatures = signatures;
      this.values = values;
      this.timeout = timeout;
    }

    public Value call(String message, List<Value> args)
    {
      var signature = signatures.get(message);
      var out = new java.io.ByteArrayOutputStream();
      putText(out, message);
      for (int i = 0; i < signature.arguments().size(); i++)
      {
        wirePut(values, signature.arguments().get(i), args.get(i), out);
      }
      return replyValue(
          node.request(address, "call", out.toByteArray(), timeout), values, signature.reply());
    }
  }

  private static final Object NET_ABANDONED = new Object();

  /**
   * One end of a channel between nodes, as a Channel. Each value travels in a numbered frame that
   * is sent again until acknowledged, so loss, duplication and reordering are repaired; a peer
   * silent for the deadline is treated as failed (PeerFailed). Order is kept within the channel.
   *
   * <p>An unused end can move to another node: offer() gives the address the new node takes it over
   * from ({address}?take={token}). On a take frame with that token, this end hands its state over
   * (a state frame) and from then on forwards every frame it gets to the new end; the new end tells
   * the peer (a moved frame) so the peer sends to it directly.
   */
  public static final class NetEndpoint implements Channel, Entity
  {
    private final Node node;
    private final List<Step> steps;
    private final Values values;
    private final double deadline;
    public String address;
    private String peer;
    private long out;
    private final Map<Long, Object[]> unacked = new java.util.HashMap<>();
    private long expected;
    private final Map<Long, byte[]> early = new java.util.HashMap<>();
    private final java.util.concurrent.LinkedBlockingQueue<Object[]> inbox =
        new java.util.concurrent.LinkedBlockingQueue<>();
    private int step;
    private volatile boolean gone;
    private String failure = "";
    // Moving: the addresses this end had before (oldest first), the token a taker must show, where
    // the end went and the state frame it was given, and, on the new node, the takeover in
    // progress.
    private List<String> history = new ArrayList<>();
    private String token;
    private String movedTo;
    private byte[] state;
    private String takeToken;
    private final java.util.concurrent.CountDownLatch taken =
        new java.util.concurrent.CountDownLatch(1);
    private boolean announcing;
    private long announcedAt;
    private final java.util.concurrent.CountDownLatch confirmed =
        new java.util.concurrent.CountDownLatch(1);

    NetEndpoint(Node node, List<Step> steps, Values values, double deadline)
    {
      this.node = node;
      this.steps = steps;
      this.values = values;
      this.deadline = deadline;
      startDaemon(this::resend);
    }

    private static void putSeq(java.io.ByteArrayOutputStream out, long seq)
    {
      putVarint(out, zigzag(BigInteger.valueOf(seq)));
    }

    private static long getSeq(Reader in)
    {
      return unzigzag(in.varint()).longValueExact();
    }

    private static void putBytes(java.io.ByteArrayOutputStream out, byte[] bytes)
    {
      putVarint(out, BigInteger.valueOf(bytes.length));
      out.writeBytes(bytes);
    }

    private static byte[] getBytes(Reader in)
    {
      return in.take(in.varint().intValueExact());
    }

    private static void putTexts(java.io.ByteArrayOutputStream out, List<String> texts)
    {
      putVarint(out, BigInteger.valueOf(texts.size()));
      for (var t : texts)
      {
        putText(out, t);
      }
    }

    private static List<String> getTexts(Reader in)
    {
      int count = in.varint().intValueExact();
      var texts = new ArrayList<String>();
      for (int i = 0; i < count; i++)
      {
        texts.add(getText(in));
      }
      return texts;
    }

    private static void putNumbered(
        java.io.ByteArrayOutputStream out, java.util.SortedMap<Long, byte[]> items)
    {
      putVarint(out, BigInteger.valueOf(items.size()));
      for (var item : items.entrySet())
      {
        putSeq(out, item.getKey());
        putBytes(out, item.getValue());
      }
    }

    private static java.util.SortedMap<Long, byte[]> getNumbered(Reader in)
    {
      int count = in.varint().intValueExact();
      var items = new java.util.TreeMap<Long, byte[]>();
      for (int i = 0; i < count; i++)
      {
        long seq = getSeq(in);
        items.put(seq, getBytes(in));
      }
      return items;
    }

    void connect(String address)
    {
      synchronized (this)
      {
        peer = address;
      }
      transmit(-1, utf8("hello"));
    }

    private byte[] frame(long seq, byte[] body)
    {
      var buffer = new java.io.ByteArrayOutputStream();
      putSeq(buffer, seq);
      putText(buffer, address);
      buffer.writeBytes(body);
      return buffer.toByteArray();
    }

    private void quietly(String target, String kind, byte[] payload)
    {
      try
      {
        node.send(target, kind, payload, 0);
      }
      catch (RuntimeException e)
      {
        // Sent again later, or by the other side.
      }
    }

    /** Sends a numbered frame (seq -1 is the hello) until it is acked. */
    private void transmit(long seq, byte[] body)
    {
      var payload = frame(seq, body);
      String target;
      synchronized (this)
      {
        long now = System.nanoTime();
        unacked.put(seq, new Object[] {payload, now, now, body});
        target = peer;
      }
      if (target != null)
      {
        quietly(target, "chan", payload);
      }
    }

    private void resend()
    {
      while (!gone && !node.closed)
      {
        try
        {
          Thread.sleep(20);
        }
        catch (InterruptedException e)
        {
          return;
        }
        long now = System.nanoTime();
        String target;
        var due = new ArrayList<Object[]>();
        boolean stale = false;
        byte[] moved = null;
        synchronized (this)
        {
          if (movedTo != null)
          {
            return;
          }
          if (takeToken != null && taken.getCount() > 0)
          {
            continue;
          }
          target = peer;
          for (var entry : unacked.values())
          {
            if (now - (long) entry[2] > 50_000_000L)
            {
              due.add(entry);
              if (now - (long) entry[1] > (long) (deadline * 1e9))
              {
                stale = true;
              }
            }
          }
          if (announcing
              && target != null
              && confirmed.getCount() > 0
              && now - announcedAt > 50_000_000L)
          {
            announcedAt = now;
            var buffer = new java.io.ByteArrayOutputStream();
            putTexts(buffer, history);
            putText(buffer, address);
            moved = buffer.toByteArray();
          }
        }
        if (stale)
        {
          fail("the other end did not answer in time (unreachable)");
          return;
        }
        if (target == null)
        {
          continue;
        }
        if (moved != null)
        {
          quietly(target, "moved", moved);
        }
        for (var entry : due)
        {
          entry[2] = now;
          quietly(target, "chan", (byte[]) entry[0]);
        }
      }
    }

    private void fail(String reason)
    {
      synchronized (this)
      {
        if (gone)
        {
          return;
        }
        gone = true;
        failure = reason;
        unacked.clear();
        inbox.add(new Object[] {NET_ABANDONED, reason});
      }
    }

    @Override
    public void receive(Node node, String kind, String source, long id, byte[] payload)
    {
      try
      {
        handle(node, kind, source, id, payload);
      }
      catch (RuntimeException e)
      {
        // A malformed frame is dropped.
      }
    }

    private void handle(Node node, String kind, String source, long id, byte[] payload)
    {
      if (kind.equals("take"))
      {
        give(payload);
        return;
      }
      String forward;
      boolean waiting;
      synchronized (this)
      {
        forward = movedTo;
        waiting = takeToken != null && taken.getCount() > 0;
      }
      if (forward != null)
      {
        node.forward(forward, kind, source, id, payload);
        return;
      }
      if (waiting)
      {
        // Until the state arrives, frames are dropped: their senders send them again.
        if (kind.equals("state"))
        {
          install(payload);
        }
        return;
      }
      var in = new Reader(payload, 0);
      switch (kind)
      {
        case "ack" -> {
          long seq = getSeq(in);
          synchronized (this)
          {
            unacked.remove(seq);
          }
          return;
        }
        case "moved" -> {
          peerMoved(in);
          return;
        }
        case "moved-ack" -> {
          if (getText(in).equals(address))
          {
            confirmed.countDown();
          }
          return;
        }
        case "chan" -> {}
        default -> {
          return;
        }
      }
      long seq = getSeq(in);
      String sender = getText(in);
      var body = Arrays.copyOfRange(payload, in.pos, payload.length);
      synchronized (this)
      {
        forward = movedTo;
        if (forward == null)
        {
          if (seq == -1)
          {
            if (peer == null)
            {
              peer = sender;
            }
          }
          else if (seq >= expected && !early.containsKey(seq))
          {
            early.put(seq, body);
            while (early.containsKey(expected))
            {
              inbox.add(new Object[] {null, early.remove(expected++)});
            }
          }
        }
      }
      if (forward != null)
      {
        // Moved meanwhile: the new end acknowledges it.
        node.forward(forward, kind, source, id, payload);
        return;
      }
      var ack = new java.io.ByteArrayOutputStream();
      putSeq(ack, seq);
      quietly(sender, "ack", ack.toByteArray());
    }

    /** The address another node takes this unused end over from. */
    synchronized String offer()
    {
      if (token == null)
      {
        token = secureToken();
      }
      return address + "?take=" + token;
    }

    /**
     * A take frame: hands the state over once, to the first taker with the token, and answers that
     * taker's repeats with the same state.
     */
    private void give(byte[] payload)
    {
      var in = new Reader(payload, 0);
      String offered = getText(in);
      String taker = getText(in);
      byte[] answer;
      synchronized (this)
      {
        if (token == null || !offered.equals(token))
        {
          return;
        }
        if (movedTo == null)
        {
          var buffer = new java.io.ByteArrayOutputStream();
          putText(buffer, token);
          putText(buffer, failure);
          putText(buffer, peer == null ? "" : peer);
          var former = new ArrayList<>(history);
          former.add(address);
          putTexts(buffer, former);
          putSeq(buffer, out);
          putSeq(buffer, expected);
          var waiting = new java.util.TreeMap<Long, byte[]>();
          for (var entry : unacked.entrySet())
          {
            waiting.put(entry.getKey(), (byte[]) entry.getValue()[3]);
          }
          putNumbered(buffer, waiting);
          putNumbered(buffer, new java.util.TreeMap<>(early));
          var received = new ArrayList<byte[]>();
          for (var item : inbox)
          {
            if (item[0] == null)
            {
              received.add((byte[]) item[1]);
            }
          }
          putVarint(buffer, BigInteger.valueOf(received.size()));
          for (var body : received)
          {
            putBytes(buffer, body);
          }
          movedTo = taker;
          state = buffer.toByteArray();
          unacked.clear();
          early.clear();
        }
        else if (!movedTo.equals(taker))
        {
          return;
        }
        answer = state;
      }
      quietly(taker, "state", answer);
    }

    /**
     * Takes over the end offered at address ({old address}?take={token}): asks for its state until
     * it comes, then tells the peer where the end is now. Returns once the peer knows, or after the
     * deadline (the old node then keeps forwarding to this end, as a relay would).
     */
    void takeOver(String offered)
    {
      int cut = offered.indexOf('?');
      String old = offered.substring(0, cut);
      String wanted = offered.substring(cut + 1).substring("take=".length());
      synchronized (this)
      {
        takeToken = wanted;
      }
      var request = new java.io.ByteArrayOutputStream();
      putText(request, wanted);
      putText(request, address);
      long giveUp = System.nanoTime() + (long) (deadline * 1e9);
      try
      {
        quietly(old, "take", request.toByteArray());
        while (!taken.await(50, java.util.concurrent.TimeUnit.MILLISECONDS))
        {
          if (System.nanoTime() >= giveUp)
          {
            synchronized (this)
            {
              takeToken = null;
            }
            fail("the node the end came from did not hand it over in time (unreachable)");
            return;
          }
          quietly(old, "take", request.toByteArray());
        }
        confirmed.await(
            Math.max(0, giveUp - System.nanoTime()), java.util.concurrent.TimeUnit.NANOSECONDS);
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
      }
    }

    private void install(byte[] payload)
    {
      var in = new Reader(payload, 0);
      String offered = getText(in);
      String reason = getText(in);
      String from = getText(in);
      var former = getTexts(in);
      long next = getSeq(in);
      long wanted = getSeq(in);
      var waiting = getNumbered(in);
      var arrived = getNumbered(in);
      int count = in.varint().intValueExact();
      var received = new ArrayList<byte[]>();
      for (int i = 0; i < count; i++)
      {
        received.add(getBytes(in));
      }
      synchronized (this)
      {
        if (takeToken == null || !offered.equals(takeToken) || taken.getCount() == 0)
        {
          return;
        }
        peer = from.isEmpty() ? null : from;
        history = former;
        out = next;
        expected = wanted;
        // Sent again from here at once, under this end's address.
        for (var item : waiting.entrySet())
        {
          unacked.put(
              item.getKey(),
              new Object[] {
                frame(item.getKey(), item.getValue()), System.nanoTime(), 0L, item.getValue()
              });
        }
        early.putAll(arrived);
        for (var body : received)
        {
          inbox.add(new Object[] {null, body});
        }
        announcing = true;
        taken.countDown();
      }
      if (!reason.isEmpty())
      {
        fail(reason);
      }
    }

    /** The peer moved: from now on send to its new address. */
    private void peerMoved(Reader in)
    {
      var former = getTexts(in);
      String to = getText(in);
      boolean known;
      synchronized (this)
      {
        if (peer == null || former.contains(peer))
        {
          peer = to;
        }
        known = peer.equals(to);
      }
      if (known)
      {
        var answer = new java.io.ByteArrayOutputStream();
        putText(answer, to);
        quietly(to, "moved-ack", answer.toByteArray());
      }
    }

    private Object stepDescriptor(boolean sends)
    {
      synchronized (this)
      {
        if (step >= steps.size())
        {
          throw new IllegalStateException("this channel's protocol has ended");
        }
        var s = steps.get(step);
        if (s.sends() != sends)
        {
          throw new IllegalStateException("this step " + (s.sends() ? "sends" : "receives"));
        }
        step++;
        return s.descriptor();
      }
    }

    @Override
    public void send(int side, Object value)
    {
      if (gone)
      {
        throw new PeerFailed("the other end has failed");
      }
      var d = stepDescriptor(true);
      var body = new java.io.ByteArrayOutputStream();
      body.write(0);
      wirePut(values, d, (Value) value, body);
      long seq;
      synchronized (this)
      {
        seq = out++;
      }
      transmit(seq, body.toByteArray());
    }

    @Override
    public Object receive(int side)
    {
      return receive(side, 0);
    }

    /** Waits up to timeout seconds (forever when 0); throws IllegalStateException on timeout. */
    public Object receive(int side, double timeout)
    {
      var d = stepDescriptor(false);
      Object[] item;
      try
      {
        item =
            timeout <= 0
                ? inbox.take()
                : inbox.poll((long) (timeout * 1e9), java.util.concurrent.TimeUnit.NANOSECONDS);
      }
      catch (InterruptedException e)
      {
        Thread.currentThread().interrupt();
        throw new IllegalStateException("interrupted while receiving", e);
      }
      if (item == null)
      {
        throw new IllegalStateException(
            new java.util.concurrent.TimeoutException("no message arrived in time"));
      }
      if (item[0] == NET_ABANDONED)
      {
        inbox.add(item);
        throw new PeerFailed((String) item[1]);
      }
      var body = (byte[]) item[1];
      if (body.length > 0 && body[0] == 1)
      {
        fail("the other end gave up the conversation");
        throw new PeerFailed(
            "the other end gave up the conversation (its process failed or abandoned it)");
      }
      return wireDecode(values, d, Arrays.copyOfRange(body, 1, body.length));
    }

    /** Gives up: the other end's receives fail after what was sent. */
    @Override
    public void abandon(int side)
    {
      long seq;
      synchronized (this)
      {
        seq = out++;
      }
      transmit(seq, new byte[] {1});
    }
  }

  /** Converts a step's native values to logical ones and back. */
  public interface Conversion
  {
    Value toLogical(Object nativeValue);

    Object toNative(Value logical);
  }

  /** A conversion from a codec's encode and decode. */
  @SuppressWarnings("unchecked")
  public static <T> Conversion conversion(Function<T, Value> encode, Function<Value, T> decode)
  {
    return new Conversion()
    {
      public Value toLogical(Object nativeValue)
      {
        return encode.apply((T) nativeValue);
      }

      public Object toNative(Value logical)
      {
        return decode.apply(logical);
      }
    };
  }

  /** The conversion of a scalar type's boxed Java values. */
  public static Conversion scalarConversion(String type, int bits)
  {
    return new Conversion()
    {
      public Value toLogical(Object nativeValue)
      {
        return fromNative(type, nativeValue, bits);
      }

      public Object toNative(Value logical)
      {
        return LawSpecRuntime.toNative(type, logical, bits);
      }
    };
  }

  /** The data types of descriptor forms, for decoding values that use them. */
  public static Values valuesOf(String forms)
  {
    var table = new java.util.HashMap<String, List<Object>>();
    for (var f : readDescriptor(forms))
    {
      if (f instanceof List<?> && atomText(form(f).get(0)).equals("data"))
      {
        table.put(atomText(form(f).get(1)), form(f));
      }
    }
    return new Values(table);
  }

  /** One descriptor, such as (int Int32 _ _). */
  public static Object descriptor(String text)
  {
    return readDescriptor(text).get(0);
  }

  /**
   * A protocol's steps over a network, from its first end: each step's descriptor, and its part:
   * null (no conversion), a Conversion, or an EndPart for a step that sends another protocol's
   * first end. values decodes the descriptors.
   */
  public record Wire(List<Step> steps, List<Object> parts, Values values)
  {}

  /**
   * A step that sends another protocol's first end: start makes that end's start class on a
   * channel, channelOf takes the channel of an unused end being sent (claiming it), and wire is
   * that protocol's Wire.
   */
  public record EndPart(
      Function<Channel, Object> start,
      Function<Object, Channel> channelOf,
      java.util.function.Supplier<Wire> wire)
  {}

  /**
   * A network channel end seen through native values: each step's part converts its value, or, for
   * an EndPart, sends a channel end and receives one. A network end moves to the receiving node
   * (the value is {address}?take={token}); a local end stays here behind a relay (the value is the
   * relay's address).
   */
  public static final class NativeChannel implements Channel
  {
    private final NetEndpoint endpoint;
    private final List<?> parts;
    private int step;

    public NativeChannel(NetEndpoint endpoint, List<?> parts)
    {
      this.endpoint = endpoint;
      this.parts = parts;
    }

    synchronized boolean unused()
    {
      return step == 0;
    }

    private synchronized Object part()
    {
      var c = step < parts.size() ? parts.get(step) : null;
      step++;
      return c;
    }

    @Override
    public void send(int side, Object value)
    {
      var part = part();
      if (part instanceof EndPart end)
      {
        value = textValue(offerEnd(endpoint.node, value, end));
      }
      else if (part instanceof Conversion c)
      {
        value = c.toLogical(value);
      }
      endpoint.send(side, value);
    }

    @Override
    public Object receive(int side)
    {
      var part = part();
      var value = (Value) endpoint.receive(side);
      if (part instanceof EndPart end)
      {
        var wire = end.wire().get();
        var address = textOf(value);
        var taken =
            address.contains("?take=")
                ? endpoint.node.take(address, wire.steps(), wire.values())
                : endpoint.node.dial(address, wire.steps(), wire.values());
        return end.start().apply(new NativeChannel(taken, wire.parts()));
      }
      return part instanceof Conversion c ? c.toNative(value) : value;
    }

    @Override
    public void abandon(int side)
    {
      endpoint.abandon(side);
    }
  }

  /**
   * The text that gives an unused channel end to another node. An end that is itself between nodes
   * moves there; a local end stays here and a relay on node carries its conversation.
   */
  static String offerEnd(Node node, Object end, EndPart part)
  {
    var channel = part.channelOf().apply(end);
    if (channel instanceof NativeChannel network && network.unused())
    {
      return network.endpoint.offer();
    }
    return relayEnd(node, channel, part);
  }

  /**
   * Offers a local channel end to another node: a relay on node listens for the receiver and passes
   * each step between it and the end, which stays here. Returns the relay's address. A failure on
   * either side gives up the other.
   */
  static String relayEnd(Node node, Channel channel, EndPart part)
  {
    var wire = part.wire().get();
    var flipped = new ArrayList<Step>();
    for (var s : wire.steps())
    {
      flipped.add(new Step(!s.sends(), s.descriptor()));
    }
    var relay = node.listen("relay-" + node.nextId(), flipped, wire.values());
    var relayed = new NativeChannel(relay, wire.parts());
    startDaemon(
        () -> {
          try
          {
            for (var s : wire.steps())
            {
              if (s.sends())
              {
                channel.send(0, relayed.receive(0));
              }
              else
              {
                relayed.send(0, channel.receive(0));
              }
            }
          }
          catch (RuntimeException e)
          {
            try
            {
              channel.abandon(0);
            }
            catch (RuntimeException ignored)
            {
              // Already failed.
            }
            try
            {
              relay.abandon(0);
            }
            catch (RuntimeException ignored)
            {
              // Already failed.
            }
          }
        });
    return relay.address;
  }

  // Abilities (docs/explanation/abilities.md). Handlers travel in the symbols
  // map generated code passes to every definition: that map is the evidence
  // of evidence-passing compilation. A law installs one handler per ability;
  // an operation finds the handler of its ability there. The Fail ability's
  // handlers abort, so raise throws a Failure and attempt catches it.
  private static final String HANDLERS = "\0lawspec.handlers";

  /** A failure raised with the Fail ability. */
  public static final class Failure extends RuntimeException
  {
    public final String ability;
    public final transient Value value;

    public Failure(String ability, Value value)
    {
      super("failed with " + value + " (" + ability + ")", null, false, false);
      this.ability = ability;
      this.value = value;
    }
  }

  /** One call a recording handler saw. */
  public record RecordedCall(String operation, List<Value> arguments)
  {}

  /** A recording handler: the calls it has seen. */
  public interface Recorded
  {
    List<RecordedCall> lawSpecCalls();
  }

  @SuppressWarnings("unchecked")
  public static void installHandlers(Map<String, Object> symbols, Map<String, Object> handlers)
  {
    var table = new java.util.HashMap<String, Object>();
    if (symbols.get(HANDLERS) instanceof Map<?, ?> existing)
    {
      table.putAll((Map<String, Object>) existing);
    }
    table.putAll(handlers);
    symbols.put(HANDLERS, table);
  }

  public static Object handler(Map<String, Object> symbols, String ability)
  {
    if (symbols.get(HANDLERS) instanceof Map<?, ?> table && table.containsKey(ability))
    {
      return table.get(ability);
    }
    throw new IllegalStateException(
        "no handler for the ability "
            + ability
            + ": a law names one with `using`, or runs under each lawful handler");
  }

  /** The handler installed for the ability, as the ability's native interface. */
  public static <T> T handler(Map<String, Object> symbols, String ability, Class<T> type)
  {
    return type.cast(handler(symbols, ability));
  }

  public static Value raiseFailure(String ability, Value value)
  {
    throw new Failure(ability, value);
  }

  public static Value attempt(
      String ability,
      java.util.function.Supplier<Value> body,
      java.util.function.UnaryOperator<Value> right,
      java.util.function.UnaryOperator<Value> left)
  {
    Value value;
    try
    {
      value = body.get();
    }
    catch (Failure failure)
    {
      if (!failure.ability.equals(ability))
      {
        throw failure;
      }
      return left.apply(failure.value);
    }
    return right.apply(value);
  }

  public static Value countCalls(
      Object recording, String operation, java.util.function.Predicate<List<Value>> matches)
  {
    if (!(recording instanceof Recorded recorded))
    {
      throw new IllegalStateException("calls of needs a recording handler: `using recording`");
    }
    long count =
        recorded.lawSpecCalls().stream()
            .filter(
                call ->
                    call.operation().equals(operation)
                        && (matches == null || matches.test(call.arguments())))
            .count();
    return integer64(count);
  }

  /** let x = e in body: e runs first, once. */
  public static Value let(Value value, java.util.function.UnaryOperator<Value> body)
  {
    return body.apply(value);
  }

  /** A chain of lets, as a block of declarations in a generated test. */
  public static Value block(Supplier<Value> body)
  {
    return body.get();
  }

  /**
   * handle e with h end: runs body with these handlers installed, then puts back the ones they
   * replaced.
   */
  @SuppressWarnings("unchecked")
  public static Value withHandlers(
      Map<String, Object> symbols,
      Map<String, Object> handlers,
      java.util.function.Supplier<Value> body)
  {
    var previous = symbols.get(HANDLERS);
    installHandlers(symbols, handlers);
    try
    {
      return body.get();
    }
    finally
    {
      if (previous == null)
      {
        symbols.remove(HANDLERS);
      }
      else
      {
        symbols.put(HANDLERS, previous);
      }
    }
  }

  /**
   * What native code (an adapter, or a production handler) throws to fail with a value of the
   * failure type its signature names: {@code throw new LawSpecRuntime.Fail(value)} for {@code fails
   * with E}, the value a native E.
   */
  public static class Fail extends RuntimeException
  {
    public final transient Object value;

    public Fail(Object value)
    {
      super("failed with " + value);
      this.value = value;
    }
  }

  /** A native exception lawspec.edn maps to a failure: its class, and the failure it becomes. */
  public record MappedFailure(
      Class<? extends Throwable> kind, java.util.function.Function<Throwable, Value> make)
  {}

  /**
   * Calls native code that may fail: a Fail it throws, or an exception lawspec.edn maps to a
   * failure, becomes a failure of the ability.
   */
  public static Value nativeFailures(
      String ability,
      java.util.function.Function<Object, Value> convert,
      java.util.function.Supplier<Value> body,
      MappedFailure... mapped)
  {
    try
    {
      return body.get();
    }
    catch (Fail failure)
    {
      throw new Failure(ability, convert.apply(failure.value));
    }
    catch (Failure failure)
    {
      throw failure;
    }
    catch (RuntimeException error)
    {
      // A bridge may wrap the application's exception, and a future's join
      // wraps what its task threw (CompletionException); causes count too.
      for (Throwable cause = error; cause != null; cause = cause.getCause())
      {
        if (cause instanceof Fail fail)
        {
          throw new Failure(ability, convert.apply(fail.value));
        }
        if (cause instanceof Failure failure)
        {
          throw failure;
        }
        for (var each : mapped)
        {
          if (each.kind().isInstance(cause))
          {
            throw new Failure(ability, each.make().apply(cause));
          }
        }
      }
      throw error;
    }
  }

  /** A Pair's two fields: a stateful handler clause's result and next state. */
  public static List<Value> pairFields(Value pair)
  {
    if (!(pair.data() instanceof Data data) || data.fields().size() != 2)
    {
      throw new IllegalStateException("a handler clause must give Pair result state");
    }
    return data.fields();
  }
}
