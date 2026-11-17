"""Native Hypothesis strategies for instantiated Core data schemas.

Generated tests run in Hypothesis, the framework Python developers already use,
so failures shrink, replay and report as they expect; LawSpec supplies only the
strategies for its own types. ref:DEC-native-property-frameworks ref:hypothesis
"""

import threading
from functools import cache, cmp_to_key

from hypothesis import HealthCheck, Phase, find, reject, settings
from hypothesis import strategies as st
from hypothesis.errors import NoSuchExample

import lawspec_runtime as ls
from lawspec_schema import RefinementViolation, witness_instances, witnessed

# A law's inputs may be planned again for every case it draws, and a probe's
# answer is the same each time: a witness that fits needs no probe. The schema
# is kept with the answer so that its identity is not given to another.
_PROBE_ATTEMPTS = 1000
_PROBES = {}
# The draws one restricted value may take within a case.
_FILTER_ATTEMPTS = 100


@st.composite
def _restricted(draw, candidates, accepted):
    """Draw a candidate its restriction accepts.

    Hypothesis's own filter gives a value three draws and then refuses the
    whole case, which a restriction met by one draw in five spends in a
    short list."""
    for _ in range(_FILTER_ATTEMPTS):
        value = draw(candidates)
        if accepted(value):
            return value
    reject()


def _probe(candidates, accepted):
    """Return whether a fixed run of draws finds an accepted candidate.

    Hypothesis refuses a search begun inside a running test on the same
    thread, so the run has a thread of its own. It is derandomized, so a
    law's values for a seed do not depend on it. A run that fails for
    another reason decides nothing."""
    found = []

    def run():
        try:
            find(
                candidates,
                accepted,
                settings=settings(
                    max_examples=_PROBE_ATTEMPTS,
                    database=None,
                    derandomize=True,
                    deadline=None,
                    phases=[Phase.generate],
                    suppress_health_check=list(HealthCheck),
                ),
            )
            found.append(True)
        except NoSuchExample:
            found.append(False)
        except Exception:
            found.append(True)

    thread = threading.Thread(target=run)
    thread.start()
    thread.join()
    return found[0]


def strategy(
    schema,
    reference,
    bits,
    budget,
    scalar,
    symbols=None,
    witnesses=(),
    native_generators=None,
    native_schema=None,
    index=None,
):
    """Generate values with native strategies and checked witnesses.

    Witness subvalues seed nested domains in the given Symbol context.
    Sampled witnesses supplement native generators and their shrinkers;
    sampled alternatives need not shrink to a globally minimal value.
    Only witnesses fitting the current node budget can be drawn.


    Strategies are built from Hypothesis's own combinators so that its
    shrinker, not a second one, reduces counterexamples, and shrinking stays
    within the declared domain. ref:DEC-shrink-within-domain"""
    if type(budget) is not int or budget < 1:
        raise ValueError("structural node budget must be positive")
    schema._check(reference)
    schema._bits(bits)
    symbols = {} if symbols is None else symbols
    native_generators = (
        {} if native_generators is None else dict(native_generators)
    )
    native_schema = schema if native_schema is None else native_schema
    for name, factory in native_generators.items():
        if name not in schema._arity or not callable(factory):
            raise ValueError("invalid native generator binding: " + str(name))

    seeds = {}

    def remember(ty, value):
        constructors = schema.constructors(ty)
        children = []
        if constructors is not None:
            constructor = next(
                item for item in constructors if item.tag == value.tag
            )
            children = list(
                zip(witnessed(constructor, value.fields), value.fields)
            )
        elif ty.name == "List":
            children = [(ty.arguments[0], child) for child in value]
        elif ty.name in ("Nullable", "Optional") and value.present:
            children = [(ty.arguments[0], value.value)]
        elif ty.name == "Maybe" and value.tag == "Maybe::Present":
            children = [(ty.arguments[0], value.fields[0])]
        elif ty.name == "Either":
            index = 0 if value.tag == "Either::Left" else 1
            children = [(ty.arguments[index], value.fields[0])]
        cost = 1 + sum(
            remember(child_type, child) for child_type, child in children
        )
        seeds.setdefault(ty, []).append((cost, value))
        return cost

    for witness in witnesses:
        remember(reference, schema.validate(reference, witness, bits, symbols))

    def valid(ty, value):
        try:
            schema.validate(ty, value, bits, symbols)
            return True
        except RefinementViolation:
            return False

    @cache
    def reservation(ty):
        """Return the nodes a restricted type reserves.

        That is the size of its smallest witness, which stands for its
        smallest value: the smallest shape of such a type (an empty list in
        a wrapper that refuses one) need not be a value. A type with no
        restriction, or no witness, reserves nothing beyond its shape."""
        constructors = schema.constructors(ty)
        if constructors is None or not any(
            item.predicates for item in constructors
        ):
            return 0
        return min((cost for cost, _ in seeds.get(ty, ())), default=0)

    @cache
    def minimum(ty, limit):
        return next(
            (cost for cost in range(1, limit + 1) if inhabited(ty, cost)), None
        )

    def allocation(fields, available):
        costs = []
        for field in fields:
            cost = minimum(field.type, available)
            if cost is None:
                return None
            costs.append(cost)
            available -= cost
        count = len(costs)
        return tuple(
            cost + available // count + (index < available % count)
            for index, cost in enumerate(costs)
        )

    proofs = {}
    proving = set()
    founds = {}

    def proven(ty):
        """Return whether a probe that found no value of a type is believed.

        It is when a probe found a value at the law's whole budget, so that
        it is the smaller budget that has none. A type no probe finds a
        value of at the whole budget fails the law. A type met again inside
        its own proof is taken as proven."""
        if ty in proofs:
            return proofs[ty]
        if ty in proving:
            return True
        proving.add(ty)
        inhabited(ty, limit)
        proving.discard(ty)
        return proofs.setdefault(ty, True)

    @cache
    def inhabited(ty, available):
        """Return whether a type has a value at this budget.

        A restricted type that no witness fits has one when a probe finds
        it. A budget that only holds shapes the restriction refuses (an
        empty list in a wrapper that needs an element) is then no budget for
        the type, so an enclosing constructor gives it more or is left
        out."""
        if not shaped(ty, available):
            return False
        constructors = schema.constructors(ty)
        if (
            ty.name in native_generators
            or constructors is None
            or not any(item.predicates for item in constructors)
        ):
            return True
        found = any(cost <= available for cost, _ in seeds.get(ty, ()))
        if not found:
            key = (id(schema), ty, available, bits)
            if key not in _PROBES:
                _PROBES[key] = (
                    schema,
                    _probe(
                        candidates(ty, available),
                        lambda value: valid(ty, value),
                    ),
                )
            found = _PROBES[key][1]
        founds[ty, available] = found
        if available == limit:
            proofs[ty] = found
            if not found:
                raise ValueError(
                    f"LawSpec: no value of {ty} found in {_PROBE_ATTEMPTS}"
                    " draws, so no case of this law could hold one. Give the"
                    " law an example whose input holds one, or bind a"
                    " generator."
                )
            return True
        return found or not proven(ty)

    def within(ty, available):
        """Return the strategy at the largest budget that has a value."""
        for cost in range(available, 0, -1):
            if inhabited(ty, cost):
                return build(ty, cost)
        raise ValueError(
            f"no value of {ty} within structural node budget {available}"
        )

    @cache
    def shaped(ty, available):
        if available < 1:
            return False
        if ty.name in native_generators:
            return True
        constructors = schema.constructors(ty)
        if constructors is not None:
            return available >= reservation(ty) and any(
                allocation(fields, available - 1) is not None
                for item in constructors
                for fields, _ in witness_instances(item)
            )
        if not ty.arguments or ty.name in (
            "List",
            "Maybe",
            "Nullable",
            "Optional",
        ):
            return True
        if ty.name == "Either":
            return any(
                inhabited(child, available - 1) for child in ty.arguments
            )
        raise ValueError("unsupported generator type: " + ty.name)

    @cache
    def build(ty, available):
        """Return a type's strategy at a budget.

        A restricted type whose values a witness or a probe found draws
        them through its restriction. One whose values a probe did not find
        keeps its shape, and the law's own filter refuses a draw."""
        inhabited(ty, available)
        if founds.get((ty, available), False):
            return _restricted(
                candidates(ty, available), lambda value: valid(ty, value)
            )
        return candidates(ty, available)

    def candidates(ty, available):
        if ty.name in native_generators:

            def native_child(child):
                remaining = max(1, available - 1)
                # A parameter need not be stored (Phantom Empty), and native
                # containers may be inhabited without an element (List Empty).
                if not inhabited(child, remaining):
                    return st.nothing()
                return build(child, remaining).map(
                    lambda value: native_schema.to_native(
                        child, value, bits, symbols
                    )
                )

            def checked(value):
                try:
                    return native_schema.from_native(ty, value, bits, symbols)
                except (
                    ValueError,
                    TypeError,
                    ArithmeticError,
                    AttributeError,
                ) as error:
                    raise ValueError(
                        f"native generator {ty.name}: {error}"
                    ) from error

            factory = native_generators[ty.name]
            generated = factory(
                *(native_child(child) for child in ty.arguments)
            )
            if not isinstance(generated, st.SearchStrategy):
                raise TypeError(
                    f"native generator {ty.name} must return a strategy"
                )
            # Mapping retains Hypothesis's shrink choices. Invalid samples
            # and shrinks fail; they are never filtered into rejections.
            return generated.map(checked)
        native = build_native(ty, available)
        fitting = [
            value for cost, value in seeds.get(ty, ()) if cost <= available
        ]
        if fitting:
            return st.one_of(native, st.sampled_from(fitting))
        return native

    def build_native(ty, available):
        if not shaped(ty, available):
            raise ValueError(
                f"no value of {ty} within structural node budget {available}"
            )
        constructors = schema.constructors(ty)
        if constructors is not None:
            alternatives = []
            for constructor in constructors:
                for fields, keys in witness_instances(constructor):
                    costs = allocation(fields, available - 1)
                    if costs is None:
                        continue
                    children = [
                        within(field.type, cost)
                        for field, cost in zip(fields, costs)
                    ]
                    # A field a regex refines also draws the regex's
                    # samples, so the refinement below rarely rejects.
                    for index, texts in constructor.hints:
                        if index < len(children) and texts:
                            children[index] = st.one_of(
                                children[index], st.sampled_from(texts)
                            )
                    values = st.tuples(*children).map(
                        lambda values, tag=constructor.tag, keys=keys: (
                            ls.DataValue(tag, tuple(values) + keys)
                        )
                    )
                    alternatives.append(values)
            if ty.name == "lawspec.collections::type::EqSet":

                def distinct(value):
                    items = []
                    try:
                        for item in value.fields[0]:
                            if not any(
                                schema.equal(
                                    ty.arguments[0], item, prior, bits, symbols
                                )
                                for prior in items
                            ):
                                items.append(item)
                    except RefinementViolation:
                        return value
                    return ls.DataValue(value.tag, (items,))

                return (
                    st.one_of(*alternatives)
                    .map(distinct)
                    .filter(lambda value: valid(ty, value))
                )
            if ty.name in CANONICAL:
                return st.one_of(*alternatives).map(CANONICAL[ty.name])
            return st.one_of(*alternatives)
        if not ty.arguments:
            return scalar(ty.name).map(
                lambda value: schema.validate(ty, value, bits, symbols)
            )
        name, arguments = ty.name, ty.arguments
        if name == "List":
            remaining = available - 1
            cost = minimum(arguments[0], remaining)
            maximum = 0 if cost is None else remaining // cost

            def sized(length):
                if length == 0:
                    return st.just([])
                return st.lists(
                    within(arguments[0], remaining // length),
                    min_size=length,
                    max_size=length,
                )

            return st.integers(0, maximum).flatmap(sized)
        if name in ("Nullable", "Optional", "Maybe"):
            absent = (
                ls.DataValue("Maybe::Absent", ())
                if name == "Maybe"
                else ls.Presence(name, False)
            )
            if minimum(arguments[0], available - 1) is None:
                return st.just(absent)
            present = within(arguments[0], available - 1).map(
                lambda value: (
                    ls.DataValue("Maybe::Present", (value,))
                    if name == "Maybe"
                    else ls.Presence(name, True, value)
                )
            )
            return st.one_of(st.just(absent), present)
        if name == "Either":
            return st.one_of(
                *[
                    build(child, available - 1).map(
                        lambda value, tag=tag: ls.DataValue(tag, (value,))
                    )
                    for tag, child in zip(
                        ("Either::Left", "Either::Right"), arguments
                    )
                    if inhabited(child, available - 1)
                ]
            )
        raise ValueError("unsupported generator type: " + name)

    # The root's own witnesses always fit, whatever budget is stated.
    limit = max(budget, reservation(reference))
    if index is not None:
        target, equations = index
        generated = indexed_strategy(
            schema,
            reference,
            int(target),
            equations,
            budget,
            build,
            inhabited,
            valid,
        )
    else:
        generated = build(reference, limit)
    # Validate complete candidates and shrink results, including nested index
    # guards. Invalid native factory samples still fail in their checked map.
    return generated.filter(lambda value: valid(reference, value))


def _canonical(key):
    """Sorted items without repeated keys: a Set's or a KeyVal's invariant."""

    def canonical(value):
        items = sorted(
            value.fields[0],
            key=cmp_to_key(lambda a, b: ls.compare_values(key(a), key(b))),
        )
        distinct = [
            item
            for index, item in enumerate(items)
            if index == 0
            or ls.compare_values(key(items[index - 1]), key(item)) != 0
        ]
        return ls.DataValue(value.tag, (distinct,))

    return canonical


# Generated collections are canonicalised rather than filtered.
CANONICAL = {
    "lawspec.collections::type::Set": _canonical(lambda item: item),
    "lawspec.collections::type::KeyVal": _canonical(
        lambda entry: entry.fields[0]
    ),
}


INDEX_SLACK = 16
INDEX_CHOICES = 6
INDEX_OPERATORS = ("+", "-", "*", "div", "mod", "^")
# Solved reachability is shared by strategies over the same equations, so a
# target drawn per example does not repeat the fixpoint.
_INDEX_TABLES = {}


def parse_index_term(tokens, at=0):
    """Parse a prefix index term: f<field>, c<literal> or an operator."""
    token = tokens[at]
    if token[0] == "c":
        return ("c", int(token[1:])), at + 1
    if token[0] == "f":
        return ("f", int(token[1:])), at + 1
    if token in INDEX_OPERATORS:
        left, at = parse_index_term(tokens, at + 1)
        right, at = parse_index_term(tokens, at)
        return (token, left, right), at
    raise ValueError("malformed index term")


def eval_index_term(term, fields):
    """Natural index arithmetic; None when an operation has no natural value."""
    kind = term[0]
    if kind == "c":
        return term[1]
    if kind == "f":
        return fields.get(term[1])
    x = eval_index_term(term[1], fields)
    y = eval_index_term(term[2], fields)
    if x is None or y is None:
        return None
    if kind == "+":
        return x + y
    if kind == "-":
        return x - y if x >= y else None
    if kind == "*":
        return x * y
    if kind == "div":
        return x // y if y > 0 else None
    if kind == "mod":
        return x % y if y > 0 else None
    return x**y if 0 <= y <= 64 else None


def index_term_fields(term):
    if term[0] == "c":
        return []
    if term[0] == "f":
        return [term[1]]
    return index_term_fields(term[1]) + index_term_fields(term[2])


def indexed_strategy(
    schema, reference, target, equations, budget, build, inhabited, valid
):
    """Construct values whose structural index equals target.

    Each constructor carries its index term then its guards, in prefix
    notation over field indices. Reachability is a forward fixpoint over
    levels 0..target+slack, so a child may exceed its parent's index; the
    target is then solved backwards, and nothing is filtered away.


    Generating an indexed family by construction, rather than filtering,
    keeps generation from discarding almost every candidate.
    ref:DEC-indexed-families-as-evidence"""
    limit = max(target, 0) + INDEX_SLACK

    def parse(texts):
        tokens = texts[0].split()
        term, at = parse_index_term(tokens)
        if at != len(tokens):
            raise ValueError("malformed index term")
        guards = []
        for text in texts[1:]:
            tokens = text.split()
            if tokens[0] not in ("==", ">="):
                raise ValueError("malformed index guard")
            left, at = parse_index_term(tokens, 1)
            right, at = parse_index_term(tokens, at)
            guards.append((tokens[0], left, right))
        fields = index_term_fields(term)
        for _, left, right in guards:
            fields += index_term_fields(left) + index_term_fields(right)
        return term, guards, tuple(dict.fromkeys(fields))

    tables = {tag: parse(texts) for tag, texts in equations.items()}

    def equation(constructor):
        if constructor.tag not in tables:
            raise ValueError("missing index equation for " + constructor.tag)
        return tables[constructor.tag]

    def plain_fields(constructor, positions):
        return all(
            inhabited(field.type, budget)
            for index, field in enumerate(constructor.fields)
            if index not in positions
        )

    families = []
    pending = [reference]
    while pending:
        ty = pending.pop()
        if ty in families:
            continue
        constructors = schema.constructors(ty)
        if constructors is None:
            raise ValueError("indexed generation requires a data type")
        families.append(ty)
        for constructor in constructors:
            _, _, positions = equation(constructor)
            pending.extend(constructor.fields[p].type for p in positions)

    def holds(guard, fields):
        relation, left, right = guard
        x = eval_index_term(left, fields)
        y = eval_index_term(right, fields)
        if x is None or y is None:
            return False
        return x == y if relation == "==" else x >= y

    def assignments(constructor, reach):
        term, guards, positions = equation(constructor)
        choices = [
            [
                (p, v)
                for v in range(limit + 1)
                if (constructor.fields[p].type, v) in reach
            ]
            for p in positions
        ]
        found = []

        # A guard is checked as soon as its fields are assigned, so an
        # equality between siblings prunes every mismatched pair at once.
        assigned_by = {}
        for guard in guards:
            needed = set(
                index_term_fields(guard[1]) + index_term_fields(guard[2])
            )
            at = max((positions.index(p) for p in needed), default=-1)
            assigned_by.setdefault(at, []).append(guard)

        def extend(at, current):
            if at == len(choices):
                value = eval_index_term(term, dict(current))
                if value is not None and value <= limit:
                    found.append((value, tuple(current)))
                return
            for choice in choices[at]:
                following = current + [choice]
                fields = dict(following)
                if all(
                    holds(guard, fields) for guard in assigned_by.get(at, ())
                ):
                    extend(at + 1, following)

        if not all(holds(guard, {}) for guard in assigned_by.get(-1, ())):
            return found
        extend(0, [])
        return found

    # Keep the schema in the entry so its id cannot be reused while cached.
    table_key = (
        id(schema),
        reference,
        tuple(sorted((tag, tuple(texts)) for tag, texts in equations.items())),
    )
    cached = _INDEX_TABLES.get(table_key)
    if cached is not None and cached[1] >= limit:
        limit = cached[1]
    else:
        reach = set()
        while True:
            grown = set(reach)
            for ty in families:
                for constructor in schema.constructors(ty):
                    _, _, positions = equation(constructor)
                    if not plain_fields(constructor, positions):
                        continue
                    for value, _ in assignments(constructor, reach):
                        grown.add((ty, value))
            if grown == reach:
                break
            reach = grown
        cached = _INDEX_TABLES[table_key] = (schema, limit, reach, {})
    _, _, reach, solutions = cached

    def solve(constructor, k):
        key = (constructor.tag, k)
        if key not in solutions:
            solutions[key] = [
                assignment
                for value, assignment in assignments(constructor, reach)
                if value == k
            ]
        return solutions[key]

    @cache
    def indexed(ty, k):
        alternatives = []
        for constructor in schema.constructors(ty):
            _, _, positions = equation(constructor)
            if not plain_fields(constructor, positions):
                continue
            choices = solve(constructor, k)
            if not choices:
                continue

            def assemble(assignment, constructor=constructor):
                targets = dict(assignment)
                children = [
                    indexed(field.type, targets[index])
                    if index in targets
                    else build(field.type, budget)
                    for index, field in enumerate(constructor.fields)
                ]
                return st.tuples(*children).map(
                    lambda fields, tag=constructor.tag: ls.DataValue(
                        tag, fields
                    )
                )

            values = st.sampled_from(choices).flatmap(assemble)
            if constructor.predicates:
                values = values.filter(lambda value, ty=ty: valid(ty, value))
            alternatives.append(values)
        if not alternatives:
            raise ValueError(f"no value of {ty} has index {k}")
        return st.one_of(*alternatives)

    if (reference, target) not in reach:
        # An open target (negative), or one drawn from earlier inputs that
        # breaks their preconditions or names no value, generates from the
        # smallest reachable indices; an index claim rejects a mismatch.
        levels = sorted(level for ty, level in reach if ty == reference)[
            :INDEX_CHOICES
        ]
        if not levels:
            raise ValueError(f"no value of {reference} has an index")
        return st.sampled_from(levels).flatmap(
            lambda level: indexed(reference, level)
        )
    return indexed(reference, target)
