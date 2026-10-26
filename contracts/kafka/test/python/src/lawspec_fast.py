"""Typed values for generated checked definitions and stateful spec handlers.

A definition whose types all have a typed form runs over these instead of the
dynamic values: Bool, Text, Bytes and the integers are Python's own; Unit is
None; Maybe is () or (x,), Either (0, l) or (1, r); data is a tuple of its
fields, after its constructor's rank when it has several (a constructor
without fields of a type without any is its rank alone); a List is None or a
cell (head, tail, length) that shares its tail; KeyVal and Set are persistent
AVL trees in the portable order, (key, value, left, right, height) nodes
whose updates copy only the path to the key. Python orders and compares
these tuples as LawSpec does, so maps of such keys use < and == directly;
other keys pass their order. A handler's state is therefore shared, never
copied, from one operation to the next. The generated lawspec_typed.py
converts each typed value to and from the dynamic values where typed and
dynamic code meet. ref:DEC-typed-native-definitions
"""

import lawspec_runtime as ls


def outside(t):
    raise ValueError("integer outside " + t + " range")


def narrow(v, lo, hi, t):
    if v < lo or v > hi:
        raise ValueError("integer outside " + t + " range")
    return v


def quot(a, b):
    if b == 0:
        raise ValueError("exact division by zero")
    q = abs(a) // abs(b)
    return q if (a < 0) == (b < 0) else -q


def rem(a, b):
    if b == 0:
        raise ValueError("exact division by zero")
    return a - quot(a, b) * b


def power(a, b):
    if b < 0:
        raise ValueError("negative exponent")
    if b > 1 << 31:
        raise ValueError("exponent too large")
    return a**b


def unreachable():
    raise ValueError("unreachable branch reached")


# Lists.


def cons(h, t):
    return (h, t, 1 if t is None else t[2] + 1)


def length(xs):
    return 0 if xs is None else xs[2]


def list_of(items):
    out = None
    n = 0
    for x in reversed(items):
        n += 1
        out = (x, out, n)
    return out


def items(xs):
    out = []
    while xs is not None:
        out.append(xs[0])
        xs = xs[1]
    return out


def eq_list(a, b, eq):
    while True:
        if a is b:
            return True
        if a is None or b is None or a[2] != b[2] or not eq(a[0], b[0]):
            return False
        a, b = a[1], b[1]


def eq_list_plain(a, b):
    while True:
        if a is b:
            return True
        if a is None or b is None or a[2] != b[2] or a[0] != b[0]:
            return False
        a, b = a[1], b[1]


def cmp_list(a, b, cmp):
    while True:
        if a is b:
            return 0
        if a is None:
            return -1
        if b is None:
            return 1
        c = cmp(a[0], b[0])
        if c:
            return c
        a, b = a[1], b[1]


def cmp_plain(a, b):
    return -1 if a < b else (1 if b < a else 0)


def cmp_unit(a, b):
    return 0


def eq_unit(a, b):
    return True


def cmp_list_plain(a, b):
    return cmp_list(a, b, cmp_plain)


def append(a, b):
    out = b
    for x in reversed(items(a)):
        out = cons(x, out)
    return out


def bounds(i, j, count):
    i = 0 if i < 0 else (count if i > count else i)
    j = 0 if j < 0 else (count if j > count else j)
    return i, (i if j < i else j)


def slice_list(xs, i, j):
    """Shares the list's tail when the slice runs to its end."""
    count = length(xs)
    i, j = bounds(i, j, count)
    rest = xs
    for _ in range(i):
        rest = rest[1]
    if j == count:
        return rest
    taken = []
    while len(taken) < j - i:
        taken.append(rest[0])
        rest = rest[1]
    return list_of(taken)


def slice_seq(s, i, j):
    i, j = bounds(i, j, len(s))
    return s[i:j]


def at_list(xs, i):
    if i < 0 or i >= length(xs):
        return ()
    for _ in range(i):
        xs = xs[1]
    return (xs[0],)


def at_bytes(s, i):
    if i < 0 or i >= len(s):
        return ()
    return (s[i],)


def range_list(lo, hi):
    out = None
    k = hi
    while k > lo:
        k -= 1
        out = cons(k, out)
    return out


def octets(s):
    return list_of(list(s))


def bytes_of(xs):
    return bytes(items(xs))


def utf8(s):
    return s.encode("utf-8", "surrogatepass")


def join(separator, parts):
    return separator.join(items(parts))


def regex_matches(pattern, text):
    return ls.regex_matches(pattern, text)


# Persistent maps: None is empty, a node is (key, value, left, right,
# height). The plain functions order keys with < and ==; the _by functions
# take an order, cmp(a, b) -> -1, 0 or 1.


def _h(m):
    return 0 if m is None else m[4]


def _node(k, v, l, r):
    hl = 0 if l is None else l[4]
    hr = 0 if r is None else r[4]
    return (k, v, l, r, (hl if hl > hr else hr) + 1)


def _balance(k, v, l, r):
    hl = 0 if l is None else l[4]
    hr = 0 if r is None else r[4]
    if hl > hr + 1:
        lk, lv, ll, lr, _ = l
        if _h(ll) >= _h(lr):
            return _node(lk, lv, ll, _node(k, v, lr, r))
        return _node(lr[0], lr[1], _node(lk, lv, ll, lr[2]), _node(k, v, lr[3], r))
    if hr > hl + 1:
        rk, rv, rl, rr, _ = r
        if _h(rr) >= _h(rl):
            return _node(rk, rv, _node(k, v, l, rl), rr)
        return _node(rl[0], rl[1], _node(k, v, l, rl[2]), _node(rk, rv, rl[3], rr))
    return (k, v, l, r, (hl if hl > hr else hr) + 1)


def lookup(m, k):
    while m is not None:
        mk = m[0]
        if k == mk:
            return (m[1],)
        m = m[2] if k < mk else m[3]
    return ()


def member(m, k):
    while m is not None:
        mk = m[0]
        if k == mk:
            return True
        m = m[2] if k < mk else m[3]
    return False


def put(m, k, v):
    if m is None:
        return (k, v, None, None, 1)
    mk, mv, l, r, h = m
    if k == mk:
        return (k, v, l, r, h)
    if k < mk:
        return _balance(mk, mv, put(l, k, v), r)
    return _balance(mk, mv, l, put(r, k, v))


def _without_min(m):
    k, v, l, r, _ = m
    if l is None:
        return k, v, r
    mk, mv, rest = _without_min(l)
    return mk, mv, _balance(k, v, rest, r)


def _join(l, r):
    if l is None:
        return r
    if r is None:
        return l
    k, v, rest = _without_min(r)
    return _balance(k, v, l, rest)


def delete(m, k):
    if m is None:
        return None
    mk, mv, l, r, _ = m
    if k == mk:
        return _join(l, r)
    if k < mk:
        nl = delete(l, k)
        return m if nl is l else _balance(mk, mv, nl, r)
    nr = delete(r, k)
    return m if nr is r else _balance(mk, mv, l, nr)


def lookup_by(m, k, cmp):
    while m is not None:
        c = cmp(k, m[0])
        if c == 0:
            return (m[1],)
        m = m[2] if c < 0 else m[3]
    return ()


def member_by(m, k, cmp):
    return lookup_by(m, k, cmp) != ()


def put_by(m, k, v, cmp):
    if m is None:
        return (k, v, None, None, 1)
    mk, mv, l, r, h = m
    c = cmp(k, mk)
    if c == 0:
        return (k, v, l, r, h)
    if c < 0:
        return _balance(mk, mv, put_by(l, k, v, cmp), r)
    return _balance(mk, mv, l, put_by(r, k, v, cmp))


def delete_by(m, k, cmp):
    if m is None:
        return None
    mk, mv, l, r, _ = m
    c = cmp(k, mk)
    if c == 0:
        return _join(l, r)
    if c < 0:
        nl = delete_by(l, k, cmp)
        return m if nl is l else _balance(mk, mv, nl, r)
    nr = delete_by(r, k, cmp)
    return m if nr is r else _balance(mk, mv, l, nr)


def entries(m):
    """The (key, value) pairs in order."""
    out = []
    stack = []
    while stack or m is not None:
        while m is not None:
            stack.append(m)
            m = m[2]
        m = stack.pop()
        out.append((m[0], m[1]))
        m = m[3]
    return out


def keys_list(m):
    return list_of([k for k, _ in entries(m)])


def values_list(m):
    return list_of([v for _, v in entries(m)])


def entries_list(m):
    return list_of(entries(m))


def size(m):
    return len(entries(m))


def key_val_of(es):
    m = None
    while es is not None:
        k, v = es[0]
        m = put(m, k, v)
        es = es[1]
    return m


def key_val_of_by(es, cmp):
    m = None
    while es is not None:
        k, v = es[0]
        m = put_by(m, k, v, cmp)
        es = es[1]
    return m


def set_of(xs):
    s = None
    while xs is not None:
        if not member(s, xs[0]):
            s = put(s, xs[0], None)
        xs = xs[1]
    return s


def set_of_by(xs, cmp):
    s = None
    while xs is not None:
        if not member_by(s, xs[0], cmp):
            s = put_by(s, xs[0], None, cmp)
        xs = xs[1]
    return s


def set_insert(s, x):
    return s if member(s, x) else put(s, x, None)


def set_insert_by(s, x, cmp):
    return s if member_by(s, x, cmp) else put_by(s, x, None, cmp)


def set_union(a, b):
    small, large = (b, a) if size(a) > size(b) else (a, b)
    for k, _ in entries(small):
        large = set_insert(large, k)
    return large


def set_union_by(a, b, cmp):
    small, large = (b, a) if size(a) > size(b) else (a, b)
    for k, _ in entries(small):
        large = set_insert_by(large, k, cmp)
    return large


def set_intersection(a, b):
    out = None
    for k, _ in entries(a):
        if member(b, k):
            out = put(out, k, None)
    return out


def set_intersection_by(a, b, cmp):
    out = None
    for k, _ in entries(a):
        if member_by(b, k, cmp):
            out = put_by(out, k, None, cmp)
    return out


def set_difference(a, b):
    out = None
    for k, _ in entries(a):
        if not member(b, k):
            out = put(out, k, None)
    return out


def set_difference_by(a, b, cmp):
    out = None
    for k, _ in entries(a):
        if not member_by(b, k, cmp):
            out = put_by(out, k, None, cmp)
    return out


def eq_map(a, b, ek, ev):
    if a is b:
        return True
    ea, eb = entries(a), entries(b)
    if len(ea) != len(eb):
        return False
    for (ka, va), (kb, vb) in zip(ea, eb):
        if not ek(ka, kb) or not ev(va, vb):
            return False
    return True


def cmp_map(a, b, ck, cv):
    ea, eb = entries(a), entries(b)
    for (ka, va), (kb, vb) in zip(ea, eb):
        c = ck(ka, kb)
        if c:
            return c
        c = cv(va, vb)
        if c:
            return c
    return (len(ea) > len(eb)) - (len(ea) < len(eb))


def eq_plain(a, b):
    return a == b


# Failures: raise and attempt, as the dynamic code makes and catches them.


def fail(ability, value):
    raise ls.Failure(ability, value)


# Dynamic values.


def data(tag, *fields):
    return ls.DataValue(tag, fields)


def from_list(v, f):
    return list_of([f(x) for x in v])


def to_list(xs, f):
    out = []
    while xs is not None:
        out.append(f(xs[0]))
        xs = xs[1]
    return out
