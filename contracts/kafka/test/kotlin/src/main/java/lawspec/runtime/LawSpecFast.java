package lawspec.runtime;

import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Iterator;
import java.util.NoSuchElementException;
import java.util.Objects;
import java.util.function.BiFunction;
import lawspec.runtime.LawSpecRuntime.Data;
import lawspec.runtime.LawSpecRuntime.Value;

/**
 * Typed values for generated checked definitions and stateful spec handlers, shared by the Java
 * and Kotlin targets. A definition whose types all have a typed form runs over these instead of
 * the dynamic Value: bounded integers are longs, Integer a BigInteger, Text a String, lists
 * persistent cons lists that share their tails, KeyVal and Set persistent balanced trees in the
 * portable order. A handler's state is therefore shared, never copied, from one operation to the
 * next. Codecs convert each typed value to and from the dynamic Value where typed and dynamic code
 * meet. ref:DEC-typed-native-definitions
 */
public final class LawSpecFast {
  private LawSpecFast() {}

  /** Unit's one value. */
  public enum Unit {
    UNIT
  }

  /** Generated data: ordered by constructor identity, then fields. */
  public interface Ordered {
    int compareWith(Object other);
  }

  /** The portable total order of two values of the same type. */
  @SuppressWarnings("unchecked")
  public static int compare(Object a, Object b) {
    if (a == b) return 0;
    if (a instanceof Long x) return Long.compare(x, (Long) b);
    if (a instanceof String x) return compareText(x, (String) b);
    if (a instanceof Ordered x) return x.compareWith(b);
    if (a instanceof BigInteger x) return x.compareTo((BigInteger) b);
    if (a instanceof Boolean x) return Boolean.compare(x, (Boolean) b);
    if (a instanceof Bytes x) return x.compareTo((Bytes) b);
    if (a instanceof Unit) return 0;
    throw new IllegalArgumentException("values have no portable order");
  }

  /** Text by code point, which UTF-16's code unit order is not. */
  public static int compareText(String a, String b) {
    int n = Math.min(a.length(), b.length());
    for (int i = 0; i < n; i++) {
      char x = a.charAt(i);
      char y = b.charAt(i);
      if (x != y) {
        if (Character.isSurrogate(x) || Character.isSurrogate(y)) {
          return Integer.compare(a.codePointAt(i), b.codePointAt(i));
        }
        return Character.compare(x, y);
      }
    }
    return Integer.compare(a.length(), b.length());
  }

  public static int textLength(String s) {
    return s.codePointCount(0, s.length());
  }

  // Integers.

  public static IllegalArgumentException outside(String type) {
    return new IllegalArgumentException("integer outside " + type + " range");
  }

  /** A bounded integer, checked against its type's range. */
  public static long narrow(long value, long lo, long hi, String type) {
    if (value < lo || value > hi) throw outside(type);
    return value;
  }

  public static long narrow(BigInteger value, long lo, long hi, String type) {
    if (value.bitLength() > 63) throw outside(type);
    return narrow(value.longValue(), lo, hi, type);
  }

  public static long add(long a, long b, long lo, long hi, String type) {
    long r = a + b;
    if (((a ^ r) & (b ^ r)) < 0) throw outside(type);
    return narrow(r, lo, hi, type);
  }

  public static long subtract(long a, long b, long lo, long hi, String type) {
    long r = a - b;
    if (((a ^ b) & (a ^ r)) < 0) throw outside(type);
    return narrow(r, lo, hi, type);
  }

  public static long multiply(long a, long b, long lo, long hi, String type) {
    long high = Math.multiplyHigh(a, b);
    long r = a * b;
    if ((high != 0 || r < 0) && (high != -1 || r >= 0)) throw outside(type);
    return narrow(r, lo, hi, type);
  }

  public static BigInteger big(long value) {
    return BigInteger.valueOf(value);
  }

  public static BigInteger quot(BigInteger a, BigInteger b) {
    if (b.signum() == 0) throw new ArithmeticException("exact division by zero");
    return a.divide(b);
  }

  public static BigInteger rem(BigInteger a, BigInteger b) {
    if (b.signum() == 0) throw new ArithmeticException("exact division by zero");
    return a.remainder(b);
  }

  public static BigInteger pow(BigInteger a, BigInteger b) {
    if (b.signum() < 0) throw new ArithmeticException("negative exponent");
    if (b.bitLength() > 31) throw new ArithmeticException("exponent too large");
    return a.pow(b.intValue());
  }

  public static IllegalStateException unreachable() {
    return new IllegalStateException("unreachable branch reached");
  }

  // Sequences: Text (by code point), Bytes and List. A slice is clamped to
  // the sequence; an index outside it is Nothing.

  /** An Integer index, clamped to a long. */
  public static long index(BigInteger value) {
    if (value.bitLength() < 64) return value.longValue();
    return value.signum() < 0 ? Long.MIN_VALUE : Long.MAX_VALUE;
  }

  private static int clamp(long index, int count) {
    return index < 0 ? 0 : index > count ? count : (int) index;
  }

  public static String sliceText(String s, long from, long to) {
    int count = s.codePointCount(0, s.length());
    int i = clamp(from, count);
    int j = Math.max(i, clamp(to, count));
    if (count == s.length()) return s.substring(i, j);
    int start = s.offsetByCodePoints(0, i);
    return s.substring(start, s.offsetByCodePoints(start, j - i));
  }

  public static Bytes sliceBytes(Bytes b, long from, long to) {
    int i = clamp(from, b.octets.length);
    int j = Math.max(i, clamp(to, b.octets.length));
    return new Bytes(java.util.Arrays.copyOfRange(b.octets, i, j));
  }

  /** A slice to the end shares the list's tail. */
  public static <T> List<T> slice(List<T> xs, long from, long to) {
    int count = xs.size();
    int i = clamp(from, count);
    int j = Math.max(i, clamp(to, count));
    List<T> rest = xs;
    for (int k = 0; k < i; k++) rest = rest.tail();
    if (j == count) return rest;
    var items = new ArrayList<T>(j - i);
    for (List<T> c = rest; items.size() < j - i; c = c.tail()) items.add(c.head());
    return List.of(items);
  }

  public static Bytes appendBytes(Bytes a, Bytes b) {
    byte[] out = java.util.Arrays.copyOf(a.octets, a.octets.length + b.octets.length);
    System.arraycopy(b.octets, 0, out, a.octets.length, b.octets.length);
    return new Bytes(out);
  }

  /** a ++ b shares b. */
  public static <T> List<T> append(List<T> a, List<T> b) {
    var items = a.toJava();
    List<T> out = b;
    for (int k = items.size() - 1; k >= 0; k--) out = List.cons(items.get(k), out);
    return out;
  }

  public static <T> Maybe<T> at(List<T> xs, long index) {
    if (index < 0 || index >= xs.size()) return Maybe.nothing();
    List<T> c = xs;
    for (long k = 0; k < index; k++) c = c.tail();
    return Maybe.just(c.head());
  }

  public static Maybe<Long> atBytes(Bytes b, long index) {
    if (index < 0 || index >= b.octets.length) return Maybe.nothing();
    return Maybe.just((long) (b.octets[(int) index] & 0xff));
  }

  public static List<Long> range(long lo, long hi) {
    List<Long> out = List.nil();
    for (long k = hi; k > lo; k--) out = List.cons(k - 1, out);
    return out;
  }

  public static List<BigInteger> range(BigInteger lo, BigInteger hi) {
    List<BigInteger> out = List.nil();
    for (BigInteger k = hi.subtract(BigInteger.ONE); k.compareTo(lo) >= 0; k = k.subtract(BigInteger.ONE)) out = List.cons(k, out);
    return out;
  }

  public static List<Long> octets(Bytes b) {
    List<Long> out = List.nil();
    for (int k = b.octets.length - 1; k >= 0; k--) out = List.cons((long) (b.octets[k] & 0xff), out);
    return out;
  }

  public static Bytes bytesOf(List<Long> xs) {
    byte[] out = new byte[xs.size()];
    int k = 0;
    for (Long x : xs) out[k++] = (byte) (long) x;
    return new Bytes(out);
  }

  public static Bytes utf8(String s) {
    return new Bytes(s.getBytes(java.nio.charset.StandardCharsets.UTF_8));
  }

  public static String join(String separator, List<String> parts) {
    return String.join(separator, parts);
  }

  // Bytes.

  /** Bytes, compared by content, octets unsigned. */
  public static final class Bytes implements Comparable<Bytes> {
    private final byte[] octets;

    public Bytes(byte[] octets) {
      this.octets = octets;
    }

    public static Bytes of(int... units) {
      byte[] b = new byte[units.length];
      for (int i = 0; i < units.length; i++) b[i] = (byte) units[i];
      return new Bytes(b);
    }

    public byte[] toArray() {
      return octets.clone();
    }

    public int length() {
      return octets.length;
    }

    @Override
    public boolean equals(Object other) {
      return other instanceof Bytes b && Arrays.equals(octets, b.octets);
    }

    @Override
    public int hashCode() {
      return Arrays.hashCode(octets);
    }

    @Override
    public int compareTo(Bytes other) {
      return Arrays.compareUnsigned(octets, other.octets);
    }

    @Override
    public String toString() {
      return Arrays.toString(octets);
    }
  }

  // Maybe and Either.

  /** Maybe: Nothing before Just. */
  public static final class Maybe<T> implements Ordered {
    private static final Maybe<?> NOTHING = new Maybe<>(null, false);
    private final T value;
    private final boolean present;

    private Maybe(T value, boolean present) {
      this.value = value;
      this.present = present;
    }

    @SuppressWarnings("unchecked")
    public static <T> Maybe<T> nothing() {
      return (Maybe<T>) NOTHING;
    }

    public static <T> Maybe<T> just(T value) {
      return new Maybe<>(value, true);
    }

    public boolean isJust() {
      return present;
    }

    public T get() {
      return value;
    }

    @Override
    public boolean equals(Object other) {
      return other instanceof Maybe<?> m && m.present == present && Objects.equals(value, m.value);
    }

    @Override
    public int hashCode() {
      return present ? 31 + Objects.hashCode(value) : 0;
    }

    @Override
    public int compareWith(Object other) {
      Maybe<?> m = (Maybe<?>) other;
      if (present != m.present) return present ? 1 : -1;
      return present ? compare(value, m.value) : 0;
    }

    @Override
    public String toString() {
      return present ? "Just(" + value + ")" : "Nothing";
    }
  }

  /** Either: Left before Right. */
  public static final class Either<L, R> implements Ordered {
    private final Object value;
    private final boolean right;

    private Either(Object value, boolean right) {
      this.value = value;
      this.right = right;
    }

    public static <L, R> Either<L, R> left(L value) {
      return new Either<>(value, false);
    }

    public static <L, R> Either<L, R> right(R value) {
      return new Either<>(value, true);
    }

    public boolean isLeft() {
      return !right;
    }

    @SuppressWarnings("unchecked")
    public L left() {
      return (L) value;
    }

    @SuppressWarnings("unchecked")
    public R right() {
      return (R) value;
    }

    @Override
    public boolean equals(Object other) {
      return other instanceof Either<?, ?> e && e.right == right && Objects.equals(value, e.value);
    }

    @Override
    public int hashCode() {
      return (right ? 17 : 0) + Objects.hashCode(value);
    }

    @Override
    public int compareWith(Object other) {
      Either<?, ?> e = (Either<?, ?>) other;
      if (right != e.right) return right ? 1 : -1;
      return compare(value, e.value);
    }

    @Override
    public String toString() {
      return (right ? "Right(" : "Left(") + value + ")";
    }
  }

  // Lists.

  /** A persistent list: cons shares its tail. It knows its length. */
  public static final class List<T> implements Ordered, Iterable<T> {
    private static final List<?> NIL = new List<>(null, null, 0);
    private final T head;
    private final List<T> tail;
    private final int size;

    private List(T head, List<T> tail, int size) {
      this.head = head;
      this.tail = tail;
      this.size = size;
    }

    @SuppressWarnings("unchecked")
    public static <T> List<T> nil() {
      return (List<T>) NIL;
    }

    public static <T> List<T> cons(T head, List<T> tail) {
      return new List<>(head, tail, tail.size + 1);
    }

    public static <T> List<T> of(java.util.List<T> items) {
      List<T> list = nil();
      for (int i = items.size() - 1; i >= 0; i--) list = cons(items.get(i), list);
      return list;
    }

    public boolean isEmpty() {
      return size == 0;
    }

    public int size() {
      return size;
    }

    public T head() {
      return head;
    }

    public List<T> tail() {
      return tail;
    }

    public java.util.List<T> toJava() {
      var out = new ArrayList<T>(size);
      for (T item : this) out.add(item);
      return out;
    }

    @Override
    public Iterator<T> iterator() {
      return new Iterator<>() {
        List<T> current = List.this;

        public boolean hasNext() {
          return current.size > 0;
        }

        public T next() {
          if (current.size == 0) throw new NoSuchElementException();
          T value = current.head;
          current = current.tail;
          return value;
        }
      };
    }

    @Override
    public boolean equals(Object other) {
      if (!(other instanceof List<?> o)) return false;
      List<?> a = this;
      while (true) {
        if (a == o) return true;
        if (a.size != o.size) return false;
        if (a.size == 0) return true;
        if (!Objects.equals(a.head, o.head)) return false;
        a = a.tail;
        o = o.tail;
      }
    }

    @Override
    public int hashCode() {
      int h = 1;
      for (T item : this) h = 31 * h + Objects.hashCode(item);
      return h;
    }

    @Override
    public int compareWith(Object other) {
      List<?> a = this;
      List<?> b = (List<?>) other;
      while (true) {
        if (a == b) return 0;
        if (a.size == 0) return b.size == 0 ? 0 : -1;
        if (b.size == 0) return 1;
        int c = compare(a.head, b.head);
        if (c != 0) return c;
        a = a.tail;
        b = b.tail;
      }
    }

    @Override
    public String toString() {
      return toJava().toString();
    }
  }

  // Maps and sets.

  /** A persistent map in the portable order of its keys: an AVL tree. */
  public static final class Map<K, V> implements Ordered {
    private static final Map<?, ?> EMPTY = new Map<>(null, null, null, null, 0, 0);
    private final K key;
    private final V value;
    private final Map<K, V> left;
    private final Map<K, V> right;
    private final int height;
    private final int size;

    private Map(K key, V value, Map<K, V> left, Map<K, V> right, int height, int size) {
      this.key = key;
      this.value = value;
      this.left = left;
      this.right = right;
      this.height = height;
      this.size = size;
    }

    @SuppressWarnings("unchecked")
    public static <K, V> Map<K, V> empty() {
      return (Map<K, V>) EMPTY;
    }

    private static <K, V> Map<K, V> node(K key, V value, Map<K, V> left, Map<K, V> right) {
      return new Map<>(key, value, left, right, Math.max(left.height, right.height) + 1, left.size + right.size + 1);
    }

    private static <K, V> Map<K, V> balance(K key, V value, Map<K, V> left, Map<K, V> right) {
      if (left.height > right.height + 1) {
        if (left.left.height >= left.right.height) {
          return node(left.key, left.value, left.left, node(key, value, left.right, right));
        }
        Map<K, V> lr = left.right;
        return node(lr.key, lr.value, node(left.key, left.value, left.left, lr.left), node(key, value, lr.right, right));
      }
      if (right.height > left.height + 1) {
        if (right.right.height >= right.left.height) {
          return node(right.key, right.value, node(key, value, left, right.left), right.right);
        }
        Map<K, V> rl = right.left;
        return node(rl.key, rl.value, node(key, value, left, rl.left), node(right.key, right.value, rl.right, right.right));
      }
      return node(key, value, left, right);
    }

    public int size() {
      return size;
    }

    public boolean isEmpty() {
      return size == 0;
    }

    public Maybe<V> get(K k) {
      Map<K, V> current = this;
      while (current.size > 0) {
        int c = compare(k, current.key);
        if (c == 0) return Maybe.just(current.value);
        current = c < 0 ? current.left : current.right;
      }
      return Maybe.nothing();
    }

    public boolean containsKey(K k) {
      Map<K, V> current = this;
      while (current.size > 0) {
        int c = compare(k, current.key);
        if (c == 0) return true;
        current = c < 0 ? current.left : current.right;
      }
      return false;
    }

    public Map<K, V> put(K k, V v) {
      if (size == 0) return node(k, v, empty(), empty());
      int c = compare(k, key);
      if (c < 0) return balance(key, value, left.put(k, v), right);
      if (c > 0) return balance(key, value, left, right.put(k, v));
      return node(k, v, left, right);
    }

    private Map<K, V> withoutMin(Object[] min) {
      if (left.size == 0) {
        min[0] = key;
        min[1] = value;
        return right;
      }
      return balance(key, value, left.withoutMin(min), right);
    }

    @SuppressWarnings("unchecked")
    public Map<K, V> remove(K k) {
      if (size == 0) return this;
      int c = compare(k, key);
      if (c < 0) return left.size == 0 ? this : balance(key, value, left.remove(k), right);
      if (c > 0) return right.size == 0 ? this : balance(key, value, left, right.remove(k));
      if (left.size == 0) return right;
      if (right.size == 0) return left;
      Object[] min = new Object[2];
      Map<K, V> rest = right.withoutMin(min);
      return balance((K) min[0], (V) min[1], left, rest);
    }

    /** The entries in key order, each made by f, as a list. */
    public <E> List<E> toList(BiFunction<K, V, E> f) {
      List<E> out = List.nil();
      var stack = new ArrayList<Map<K, V>>();
      Map<K, V> current = this;
      while (true) {
        while (current.size > 0) {
          stack.add(current);
          current = current.right;
        }
        if (stack.isEmpty()) return out;
        Map<K, V> node = stack.remove(stack.size() - 1);
        out = List.cons(f.apply(node.key, node.value), out);
        current = node.left;
      }
    }

    /** Visits the entries in key order. */
    public void forEach(java.util.function.BiConsumer<K, V> f) {
      var stack = new ArrayList<Map<K, V>>();
      Map<K, V> current = this;
      while (true) {
        while (current.size > 0) {
          stack.add(current);
          current = current.left;
        }
        if (stack.isEmpty()) return;
        Map<K, V> node = stack.remove(stack.size() - 1);
        f.accept(node.key, node.value);
        current = node.right;
      }
    }

    private java.util.List<Object[]> entries() {
      var out = new ArrayList<Object[]>(size);
      forEach((k, v) -> out.add(new Object[] {k, v}));
      return out;
    }

    @Override
    public boolean equals(Object other) {
      if (this == other) return true;
      if (!(other instanceof Map<?, ?> m) || m.size != size) return false;
      var a = entries();
      var b = m.entries();
      for (int i = 0; i < a.size(); i++) {
        if (!Objects.equals(a.get(i)[0], b.get(i)[0]) || !Objects.equals(a.get(i)[1], b.get(i)[1])) return false;
      }
      return true;
    }

    @Override
    public int hashCode() {
      int[] h = {1};
      forEach((k, v) -> h[0] = 31 * h[0] + Objects.hashCode(k) * 7 + Objects.hashCode(v));
      return h[0];
    }

    @Override
    public int compareWith(Object other) {
      var a = entries();
      var b = ((Map<?, ?>) other).entries();
      int n = Math.min(a.size(), b.size());
      for (int i = 0; i < n; i++) {
        int c = compare(a.get(i)[0], b.get(i)[0]);
        if (c == 0) c = compare(a.get(i)[1], b.get(i)[1]);
        if (c != 0) return c;
      }
      return Integer.compare(a.size(), b.size());
    }

    @Override
    public String toString() {
      var parts = new ArrayList<String>();
      forEach((k, v) -> parts.add(k + "=" + v));
      return parts.toString();
    }
  }

  /** A persistent set in the portable order. */
  public static final class Set<T> implements Ordered {
    private static final Set<?> EMPTY = new Set<>(Map.empty());
    private final Map<T, Unit> items;

    private Set(Map<T, Unit> items) {
      this.items = items;
    }

    @SuppressWarnings("unchecked")
    public static <T> Set<T> empty() {
      return (Set<T>) EMPTY;
    }

    public static <T> Set<T> of(Iterable<T> items) {
      Set<T> set = empty();
      for (T item : items) set = set.insert(item);
      return set;
    }

    public int size() {
      return items.size();
    }

    public boolean isEmpty() {
      return items.isEmpty();
    }

    public boolean contains(T item) {
      return items.containsKey(item);
    }

    public Set<T> insert(T item) {
      return items.containsKey(item) ? this : new Set<>(items.put(item, Unit.UNIT));
    }

    public Set<T> remove(T item) {
      return new Set<>(items.remove(item));
    }

    public List<T> toList() {
      return items.toList((k, v) -> k);
    }

    public Set<T> union(Set<T> other) {
      Set<T> small = size() <= other.size() ? this : other;
      Set<T> large = small == this ? other : this;
      Set<T> out = large;
      for (T item : small.toList()) out = out.insert(item);
      return out;
    }

    public Set<T> intersection(Set<T> other) {
      Set<T> out = empty();
      for (T item : toList()) if (other.contains(item)) out = out.insert(item);
      return out;
    }

    public Set<T> difference(Set<T> other) {
      Set<T> out = empty();
      for (T item : toList()) if (!other.contains(item)) out = out.insert(item);
      return out;
    }

    @Override
    public boolean equals(Object other) {
      return other instanceof Set<?> s && items.equals(s.items);
    }

    @Override
    public int hashCode() {
      return items.hashCode();
    }

    @Override
    public int compareWith(Object other) {
      return toList().compareWith(((Set<?>) other).toList());
    }

    @Override
    public String toString() {
      return toList().toString();
    }
  }

  // Codecs: typed values to and from the dynamic Value.

  /** Converts one typed type to and from the dynamic Value, which carries its type key. */
  public interface Codec<T> {
    String key();

    Value encode(T value);

    T decode(Value value);
  }

  public static final String COLLECTIONS = "lawspec.collections::type::";

  private static long integerOf(Object data) {
    if (data instanceof BigInteger b) {
      if (b.bitLength() > 63) throw new IllegalArgumentException("integer outside Int64 range");
      return b.longValue();
    }
    if (data instanceof Number n) return n.longValue();
    throw new IllegalArgumentException("expected an integer");
  }

  /** A bounded integer type, a long natively. */
  public static Codec<Long> integer(String type, long lo, long hi) {
    return new Codec<>() {
      public String key() {
        return type;
      }

      public Value encode(Long value) {
        return new Value(type, BigInteger.valueOf(value));
      }

      public Long decode(Value value) {
        return narrow(integerOf(value.data()), lo, hi, type);
      }
    };
  }

  public static final Codec<BigInteger> INTEGER = new Codec<>() {
    public String key() {
      return "Integer";
    }

    public Value encode(BigInteger value) {
      return new Value("Integer", value);
    }

    public BigInteger decode(Value value) {
      Object data = value.data();
      if (data instanceof BigInteger b) return b;
      if (data instanceof Number n) return BigInteger.valueOf(n.longValue());
      throw new IllegalArgumentException("expected an Integer");
    }
  };

  public static final Codec<Boolean> BOOL = new Codec<>() {
    public String key() {
      return "Bool";
    }

    public Value encode(Boolean value) {
      return new Value("Bool", value);
    }

    public Boolean decode(Value value) {
      return (Boolean) value.data();
    }
  };

  public static final Codec<Unit> UNIT = new Codec<>() {
    public String key() {
      return "Unit";
    }

    public Value encode(Unit value) {
      return new Value("Unit", null);
    }

    public Unit decode(Value value) {
      return Unit.UNIT;
    }
  };

  private static int[] units(Object data) {
    if (data instanceof String s) return s.codePoints().toArray();
    if (data instanceof int[] a) return a;
    if (data instanceof byte[] b) {
      int[] out = new int[b.length];
      for (int i = 0; i < b.length; i++) out[i] = b[i] & 0xff;
      return out;
    }
    if (data instanceof java.util.List<?> l) {
      int[] out = new int[l.size()];
      for (int i = 0; i < out.length; i++) {
        Object item = l.get(i);
        out[i] = item instanceof Value v ? (int) integerOf(v.data()) : ((Number) item).intValue();
      }
      return out;
    }
    throw new IllegalArgumentException("expected a sequence");
  }

  public static final Codec<String> TEXT = new Codec<>() {
    public String key() {
      return "Text";
    }

    public Value encode(String value) {
      return new Value("Text", value.codePoints().boxed().toList());
    }

    public String decode(Value value) {
      int[] u = units(value.data());
      return new String(u, 0, u.length);
    }
  };

  public static final Codec<Bytes> BYTES = new Codec<>() {
    public String key() {
      return "Bytes";
    }

    public Value encode(Bytes value) {
      var out = new ArrayList<Integer>(value.octets.length);
      for (byte b : value.octets) out.add(b & 0xff);
      return new Value("Bytes", java.util.List.copyOf(out));
    }

    public Bytes decode(Value value) {
      int[] u = units(value.data());
      byte[] b = new byte[u.length];
      for (int i = 0; i < u.length; i++) b[i] = (byte) u[i];
      return new Bytes(b);
    }
  };

  private static Data data(Value value) {
    if (value.data() instanceof Data d) return d;
    throw new IllegalArgumentException("expected a data value of " + value.type());
  }

  public static <T> Codec<List<T>> list(Codec<T> item) {
    String key = "List " + item.key();
    return new Codec<>() {
      public String key() {
        return key;
      }

      public Value encode(List<T> value) {
        var out = new ArrayList<Value>(value.size());
        for (T x : value) out.add(item.encode(x));
        return new Value(key, java.util.List.copyOf(out));
      }

      public List<T> decode(Value value) {
        var out = new ArrayList<T>();
        for (Object x : (java.util.List<?>) value.data()) out.add(item.decode((Value) x));
        return List.of(out);
      }
    };
  }

  public static <T> Codec<Maybe<T>> maybe(Codec<T> item) {
    String key = "Maybe " + item.key();
    return new Codec<>() {
      public String key() {
        return key;
      }

      public Value encode(Maybe<T> value) {
        return new Value(key, value.isJust()
            ? new Data("Maybe::Just", java.util.List.of(item.encode(value.get())))
            : new Data("Maybe::Nothing", java.util.List.of()));
      }

      public Maybe<T> decode(Value value) {
        Data d = data(value);
        return d.tag().equals("Maybe::Just") ? Maybe.just(item.decode(d.fields().get(0))) : Maybe.nothing();
      }
    };
  }

  public static <L, R> Codec<Either<L, R>> either(Codec<L> left, Codec<R> right) {
    String key = "Either (" + left.key() + ") (" + right.key() + ")";
    return new Codec<>() {
      public String key() {
        return key;
      }

      public Value encode(Either<L, R> value) {
        return new Value(key, value.isLeft()
            ? new Data("Either::Left", java.util.List.of(left.encode(value.left())))
            : new Data("Either::Right", java.util.List.of(right.encode(value.right()))));
      }

      public Either<L, R> decode(Value value) {
        Data d = data(value);
        return d.tag().equals("Either::Left")
            ? Either.left(left.decode(d.fields().get(0)))
            : Either.right(right.decode(d.fields().get(0)));
      }
    };
  }

  /** A collection's items: its one field's list, or a native collection's plain list. */
  private static java.util.List<?> items(Value value) {
    if (value.data() instanceof Data d) return (java.util.List<?>) d.fields().get(0).data();
    if (value.data() instanceof java.util.List<?> l) return l;
    throw new IllegalArgumentException("expected a collection");
  }

  public static <K, V> Codec<Map<K, V>> keyVal(Codec<K> k, Codec<V> v) {
    String arguments = " (" + k.key() + ") (" + v.key() + ")";
    String key = COLLECTIONS + "KeyVal" + arguments;
    String entry = COLLECTIONS + "Entry" + arguments;
    String listKey = "List " + entry;
    return new Codec<>() {
      public String key() {
        return key;
      }

      public Value encode(Map<K, V> value) {
        var out = new ArrayList<Value>(value.size());
        value.forEach((a, b) -> out.add(new Value(entry,
            new Data(COLLECTIONS + "Entry::Entry", java.util.List.of(k.encode(a), v.encode(b))))));
        return new Value(key, new Data(COLLECTIONS + "KeyVal::KeyValEntries",
            java.util.List.of(new Value(listKey, java.util.List.copyOf(out)))));
      }

      public Map<K, V> decode(Value value) {
        Map<K, V> out = Map.empty();
        for (Object item : items(value)) {
          Data d = data((Value) item);
          out = out.put(k.decode(d.fields().get(0)), v.decode(d.fields().get(1)));
        }
        return out;
      }
    };
  }

  public static <T> Codec<Set<T>> set(Codec<T> item) {
    String key = COLLECTIONS + "Set (" + item.key() + ")";
    String listKey = "List " + item.key();
    return new Codec<>() {
      public String key() {
        return key;
      }

      public Value encode(Set<T> value) {
        var out = new ArrayList<Value>(value.size());
        for (T x : value.toList()) out.add(item.encode(x));
        return new Value(key, new Data(COLLECTIONS + "Set::SetItems",
            java.util.List.of(new Value(listKey, java.util.List.copyOf(out)))));
      }

      public Set<T> decode(Value value) {
        Set<T> out = Set.empty();
        for (Object x : items(value)) out = out.insert(item.decode((Value) x));
        return out;
      }
    };
  }

  /** A data type's key: its name, then its arguments' keys in parentheses. */
  public static String dataKey(String name, Codec<?>... arguments) {
    var key = new StringBuilder(name);
    for (Codec<?> a : arguments) key.append(" (").append(a.key()).append(')');
    return key.toString();
  }

  /** A data value's constructor and fields, for a generated decoder. */
  public static Data fields(Value value) {
    return data(value);
  }

  public static Value value(String key, String tag, Value... fields) {
    return new Value(key, new Data(tag, java.util.List.of(fields)));
  }
}
