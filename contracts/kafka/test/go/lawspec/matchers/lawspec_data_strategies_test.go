// Native Rapid combinators for LawSpec's data types; they retain the
// framework's generation and shrinking.
//
// Generated tests run in Rapid, a framework Go developers already use, so
// failures shrink, replay and report as they expect, and shrinking stays within
// the declared domain. ref:DEC-native-property-frameworks ref:rapid-go
// ref:DEC-shrink-within-domain
package matchers

import (
	"flag"
	"fmt"
	"math/big"
	"slices"
	"strconv"
	"strings"
	"sync"

	"pgregory.net/rapid"
)

// Generated tests are sequential. Serialize scoped flag changes as well, and
// restore them before another generated check or a parallel user test can run.
var lawSpecRapidSettings sync.Mutex

func lsRapidCheck(t rapid.TB, cases int, property func(*rapid.T)) {
	t.Helper()
	if cases < 1 {
		t.Fatalf("property case count must be positive")
		return
	}
	lawSpecRapidSettings.Lock()
	defer lawSpecRapidSettings.Unlock()
	setting := flag.Lookup("rapid.checks")
	previous := setting.Value.String()
	defer func() { _ = setting.Value.Set(previous) }()
	if err := setting.Value.Set(strconv.Itoa(cases)); err != nil {
		t.Fatalf("configuring Rapid: %v", err)
		return
	}
	rapid.Check(t, property)
}

// Rapid 1.2.0 caps each native filter at five attempts. Smaller requested
// limits use native discards, preserving replay and invalid-shrink handling.
// The surrounding native generators and engine retain their own retry limits.
func lsBoundedFilter[T any](source *rapid.Generator[T], attempts int, predicate func(T) bool) *rapid.Generator[T] {
	if attempts < 1 {
		panic("filter attempt limit must be positive")
	}
	if attempts >= 5 {
		return source.Filter(predicate)
	}
	return rapid.Custom(func(t *rapid.T) T {
		draws := 0
		bounded := rapid.Custom(func(inner *rapid.T) T {
			if draws >= attempts {
				inner.SkipNow()
			}
			draws++
			return source.Draw(inner, "candidate")
		})
		return bounded.Filter(predicate).Draw(t, "accepted")
	})
}

type lawSpecGeneratorKey struct {
	typeName string
	budget   int
}

func lsDataStrategy(schema *lawSpecSchema, reference lawSpecTypeRef, bits, budget int,
	scalar func(string) *rapid.Generator[LawSpecValue]) *rapid.Generator[LawSpecValue] {
	if schema.hasContracts() {
		panic("constructor contracts require checked strategies")
	}
	return lsBuildDataStrategy(schema, reference, bits, budget, scalar, nil)
}

type lawSpecCheckedValue struct {
	value    LawSpecValue
	problem  any
	rejected bool
}

func (c lawSpecCheckedValue) requireValue() LawSpecValue {
	if c.problem != nil {
		panic(c.problem)
	}
	if c.rejected {
		panic("rejected constructor candidate escaped filtering")
	}
	return c.value
}

// Catch validation failures only. Never intercept Rapid's internal discard panics.
func lsCheckCandidate(schema *lawSpecSchema, reference lawSpecTypeRef, value LawSpecValue,
	bits int, symbols map[string]*lawSpecSymbol) (result lawSpecCheckedValue) {
	result.value = value
	defer func() {
		if problem := recover(); problem != nil {
			if _, rejected := problem.(lawSpecRefinementViolation); rejected {
				result.rejected = true
			} else {
				result.problem = problem
			}
		}
	}()
	result.value = schema.validate(reference, value, bits, symbols)
	return result
}

// Native factories compose Rapid generators rather than sampling into a new RNG.
type lawSpecNativeFactory func(*lawSpecSchema, lawSpecTypeRef, int,
	map[string]*lawSpecSymbol, []*rapid.Generator[LawSpecValue]) *rapid.Generator[LawSpecValue]

type lawSpecNativeGeneratorFailure struct {
	typeName string
	problem  any
}

func (e lawSpecNativeGeneratorFailure) Error() string {
	return fmt.Sprintf("native generator %s: %v", e.typeName, e.problem)
}

func lsNativeGeneratorConvert[T any](reference lawSpecTypeRef, convert func() T) (value T) {
	defer func() {
		if problem := recover(); problem != nil {
			panic(lawSpecNativeGeneratorFailure{reference.key(), problem})
		}
	}()
	return convert()
}

func lsNativeGeneratorValues[T any](codec lawSpecCodec[T], source *rapid.Generator[T]) *rapid.Generator[LawSpecValue] {
	return rapid.Map(source, func(value T) LawSpecValue {
		return lsNativeGeneratorConvert(codec.typeRef, func() LawSpecValue { return codec.fromNative(value) })
	})
}

func lsNativeGeneratorArguments[T any](codec lawSpecCodec[T], source *rapid.Generator[LawSpecValue]) *rapid.Generator[T] {
	return rapid.Map(source, func(value LawSpecValue) T {
		return lsNativeGeneratorConvert(codec.typeRef, func() T { return codec.toNative(value) })
	})
}

type lawSpecCheckedStrategy struct {
	factories map[string]lawSpecNativeFactory
	attempts  int
	problem   any
	symbols   map[string]*lawSpecSymbol
	witnesses map[string][]LawSpecValue
}

func lsDataNodes(value LawSpecValue) int {
	count := 1
	switch data := value.Data.(type) {
	case lawSpecData:
		for _, child := range data.fields {
			count += lsDataNodes(child)
		}
	case []LawSpecValue:
		for _, child := range data {
			count += lsDataNodes(child)
		}
	case lawSpecPresence:
		if data.value != nil {
			count += lsDataNodes(*data.value)
		}
	}
	return count
}

func (c *lawSpecCheckedStrategy) addWitness(schema *lawSpecSchema, reference lawSpecTypeRef,
	value LawSpecValue) {
	c.witnesses[reference.key()] = append(c.witnesses[reference.key()], value)
	switch data := value.Data.(type) {
	case []LawSpecValue:
		for _, child := range data {
			c.addWitness(schema, reference.arguments[0], child)
		}
	case lawSpecPresence:
		if data.value != nil {
			c.addWitness(schema, reference.arguments[0], *data.value)
		}
	case lawSpecData:
		constructors, custom := schema.constructors(reference)
		if custom {
			for _, constructor := range constructors {
				if constructor.tag == data.tag {
					types := lsWitnessed(constructor, lsWitnessKeys(constructor, data.fields))
					for index := range constructor.fields {
						c.addWitness(schema, types[index], data.fields[index])
					}
				}
			}
		} else if len(data.fields) != 0 {
			index := 0
			if data.tag == "Either::Right" {
				index = 1
			}
			c.addWitness(schema, reference.arguments[index], data.fields[0])
		}
	}
}

// Rapid's native Filter bounds retries and discards invalid replay candidates.
// Seed alternatives use native SampledFrom, preserving the framework's shrinker.
func lsCheckedDataStrategy(schema *lawSpecSchema, reference lawSpecTypeRef, bits, budget int,
	symbols map[string]*lawSpecSymbol, witnesses []LawSpecValue,
	scalar func(string) *rapid.Generator[LawSpecValue]) *rapid.Generator[lawSpecCheckedValue] {
	return lsCheckedDataStrategyWithAttempts(schema, reference, bits, budget, 5, symbols, witnesses, scalar)
}

func lsCheckedDataStrategyWithAttempts(schema *lawSpecSchema, reference lawSpecTypeRef, bits, budget, attempts int,
	symbols map[string]*lawSpecSymbol, witnesses []LawSpecValue,
	scalar func(string) *rapid.Generator[LawSpecValue], factories ...map[string]lawSpecNativeFactory) *rapid.Generator[lawSpecCheckedValue] {
	if attempts < 1 {
		panic("filter attempt limit must be positive")
	}
	context := &lawSpecCheckedStrategy{attempts: attempts, symbols: lsSchemaSymbols([]map[string]*lawSpecSymbol{symbols}),
		witnesses: map[string][]LawSpecValue{}}
	if len(factories) > 1 {
		panic("only one native factory registry is allowed")
	}
	if len(factories) == 1 {
		context.factories = factories[0]
	}
	for _, witness := range witnesses {
		context.addWitness(schema, reference, schema.validate(reference, witness, bits, context.symbols))
	}
	// Validate the plan now; every sample/replay gets its own error state.
	_ = lsBuildDataStrategy(schema, reference, bits, budget, scalar, context)
	return rapid.Custom(func(t *rapid.T) (result lawSpecCheckedValue) {
		defer func() {
			if problem := recover(); problem != nil {
				if _, native := problem.(lawSpecNativeGeneratorFailure); native {
					result = lawSpecCheckedValue{problem: problem}
				} else {
					panic(problem) // Rapid discards and replay control remain Rapid-owned.
				}
			}
		}()
		sample := *context
		raw := lsBuildDataStrategy(schema, reference, bits, budget, scalar, &sample)
		value := raw.Draw(t, "candidate")
		if sample.problem != nil {
			return lawSpecCheckedValue{problem: sample.problem}
		}
		return lsCheckCandidate(schema, reference, value, bits, sample.symbols)
	})
}

func lsBuildDataStrategy(schema *lawSpecSchema, reference lawSpecTypeRef, bits, budget int,
	scalar func(string) *rapid.Generator[LawSpecValue], checked *lawSpecCheckedStrategy) *rapid.Generator[LawSpecValue] {
	if budget < 1 {
		panic("structural node budget must be positive")
	}
	if bits != 32 && bits != 64 {
		panic("machineBits must be 32 or 64")
	}
	schema.check(reference, 0)
	construct := func(t lawSpecTypeRef, tag string, values []LawSpecValue, bits int) LawSpecValue {
		if checked != nil || t.name == "lawspec.collections::type::EqSet" {
			return LawSpecValue{t.key(), lawSpecData{tag, values}}
		}
		return schema.construct(t, tag, values, bits)
	}
	minima := map[lawSpecGeneratorKey]int{}
	inhabitants := map[lawSpecGeneratorKey]bool{}
	arbitraries := map[lawSpecGeneratorKey]*rapid.Generator[LawSpecValue]{}
	var minimum func(lawSpecTypeRef, int) int
	var inhabited func(lawSpecTypeRef, int) bool
	var allocation func([]lawSpecFieldSchema, int) ([]int, bool)
	var build func(lawSpecTypeRef, int) *rapid.Generator[LawSpecValue]
	minimum = func(typeRef lawSpecTypeRef, limit int) int {
		key := lawSpecGeneratorKey{typeRef.key(), limit}
		if cost, exists := minima[key]; exists {
			return cost
		}
		for cost := 1; cost <= limit; cost++ {
			if inhabited(typeRef, cost) {
				minima[key] = cost
				return cost
			}
		}
		minima[key] = 0
		return 0
	}
	allocation = func(fields []lawSpecFieldSchema, available int) ([]int, bool) {
		costs := make([]int, len(fields))
		for index, field := range fields {
			cost := minimum(field.typeRef, available)
			if cost == 0 {
				return nil, false
			}
			costs[index] = cost
			available -= cost
		}
		for index := range costs {
			costs[index] += available / len(costs)
			if index < available%len(costs) {
				costs[index]++
			}
		}
		return costs, true
	}
	inhabited = func(typeRef lawSpecTypeRef, available int) bool {
		if available < 1 {
			return false
		}
		key := lawSpecGeneratorKey{typeRef.key(), available}
		if result, exists := inhabitants[key]; exists {
			return result
		}
		if checked != nil && checked.factories[typeRef.name] != nil {
			return true
		}
		constructors, custom := schema.constructors(typeRef)
		result := false
		if custom {
			for _, constructor := range constructors {
				fieldSets, _ := lsWitnessInstances(constructor)
				for _, fields := range fieldSets {
					if _, ok := allocation(fields, available-1); ok {
						result = true
					}
				}
			}
		} else {
			switch typeRef.name {
			case "List", "Maybe", "Nullable", "Optional":
				result = true
			case "Either":
				result = inhabited(typeRef.arguments[0], available-1) || inhabited(typeRef.arguments[1], available-1)
			default:
				result = len(typeRef.arguments) == 0
			}
		}
		inhabitants[key] = result
		return result
	}
	build = func(typeRef lawSpecTypeRef, available int) *rapid.Generator[LawSpecValue] {
		key := lawSpecGeneratorKey{typeRef.key(), available}
		if result, exists := arbitraries[key]; exists {
			return result
		}
		if !inhabited(typeRef, available) {
			panic(fmt.Sprintf("no value of %s within structural node budget %d", typeRef.key(), available))
		}
		if checked != nil && checked.factories[typeRef.name] != nil {
			arguments := make([]*rapid.Generator[LawSpecValue], len(typeRef.arguments))
			for i, argument := range typeRef.arguments {
				if inhabited(argument, available) {
					arguments[i] = build(argument, available)
				} else {
					// Phantom parameters may be empty. Rapid owns discards if
					// the native factory actually draws this argument.
					arguments[i] = rapid.Custom(func(t *rapid.T) LawSpecValue {
						t.Skipf("no native generator argument for %s within node budget %d", argument.key(), available)
						return LawSpecValue{} // Skipf does not return.
					})
				}
			}
			source := checked.factories[typeRef.name](schema, typeRef, bits, checked.symbols, arguments)
			result := rapid.Map(source, func(value LawSpecValue) LawSpecValue {
				return lsNativeGeneratorConvert(typeRef, func() LawSpecValue {
					return schema.validate(typeRef, value, bits, checked.symbols)
				})
			})
			arbitraries[key] = result
			return result // Custom distributions never use witness fallback or contract rejection.
		}
		constructors, custom := schema.constructors(typeRef)
		var result *rapid.Generator[LawSpecValue]
		if custom {
			alternatives := []*rapid.Generator[LawSpecValue]{}
			for _, constructor := range constructors {
				fieldSets, keySets := lsWitnessInstances(constructor)
				for choice, declared := range fieldSets {
					keys := keySets[choice]
					costs, ok := allocation(declared, available-1)
					if !ok {
						continue
					}
					witnesses := make([]LawSpecValue, len(keys))
					for k, key := range keys {
						witnesses[k] = lsWitnessText(key)
					}
					if len(costs) == 0 {
						alternatives = append(alternatives, rapid.Just(construct(typeRef, constructor.tag, witnesses, bits)))
						continue
					}
					fields := make([]*rapid.Generator[LawSpecValue], len(costs))
					for index, field := range declared {
						fields[index] = build(field.typeRef, costs[index])
					}
					alternatives = append(alternatives, rapid.Custom(func(t *rapid.T) LawSpecValue {
						values := make([]LawSpecValue, 0, len(fields)+len(witnesses))
						for index, field := range fields {
							values = append(values, field.Draw(t, declared[index].name))
						}
						return construct(typeRef, constructor.tag, append(values, witnesses...), bits)
					}))
				}
			}
			result = rapid.OneOf(alternatives...)
			if typeRef.name == "lawspec.collections::type::EqSet" {
				result = rapid.Map(result, lsCanonicalEqSet)
			} else if key, canonical := lsCanonicalKeys[typeRef.name]; canonical {
				result = rapid.Map(result, func(value LawSpecValue) LawSpecValue { return lsCanonical(value, key) })
			}
		} else if len(typeRef.arguments) == 0 {
			result = rapid.Map(scalar(typeRef.name), func(value LawSpecValue) LawSpecValue {
				if checked != nil {
					return value
				}
				return schema.validate(typeRef, value, bits)
			})
		} else {
			switch typeRef.name {
			case "List":
				remaining := available - 1
				cost := minimum(typeRef.arguments[0], remaining)
				maximum := 0
				if cost != 0 {
					maximum = remaining / cost
				}
				// Prebuild choices so draws never mutate the construction caches.
				lists := make([]*rapid.Generator[[]LawSpecValue], maximum+1)
				lists[0] = rapid.Just([]LawSpecValue{})
				for length := 1; length <= maximum; length++ {
					lists[length] = rapid.SliceOfN(build(typeRef.arguments[0], remaining/length), length, length)
				}
				result = rapid.Custom(func(t *rapid.T) LawSpecValue {
					length := rapid.IntRange(0, maximum).Draw(t, "length")
					return LawSpecValue{typeRef.key(), lists[length].Draw(t, "items")}
				})
			case "Maybe", "Nullable", "Optional":
				absent := lsPresent(typeRef.key(), nil)
				if typeRef.name == "Maybe" {
					absent = construct(typeRef, "Maybe::Absent", nil, bits)
				}
				result = rapid.Just(absent)
				if inhabited(typeRef.arguments[0], available-1) {
					present := rapid.Map(build(typeRef.arguments[0], available-1), func(value LawSpecValue) LawSpecValue {
						if typeRef.name == "Maybe" {
							return construct(typeRef, "Maybe::Present", []LawSpecValue{value}, bits)
						}
						return lsPresent(typeRef.key(), &value)
					})
					result = rapid.OneOf(result, present)
				}
			case "Either":
				alternatives := []*rapid.Generator[LawSpecValue]{}
				for index, child := range typeRef.arguments {
					if inhabited(child, available-1) {
						tag := []string{"Either::Left", "Either::Right"}[index]
						alternatives = append(alternatives, rapid.Map(build(child, available-1), func(value LawSpecValue) LawSpecValue {
							return construct(typeRef, tag, []LawSpecValue{value}, bits)
						}))
					}
				}
				result = rapid.OneOf(alternatives...)
			default:
				panic("unsupported data generator: " + typeRef.name)
			}
		}
		if checked != nil {
			seeds := []LawSpecValue{}
			for _, witness := range checked.witnesses[typeRef.key()] {
				if lsDataNodes(witness) <= available {
					seeds = append(seeds, witness)
				}
			}
			if len(seeds) != 0 {
				result = rapid.OneOf(result, rapid.SampledFrom(seeds))
			}
			filtered := lsBoundedFilter(result, checked.attempts, func(value LawSpecValue) bool {
				if checked.problem != nil {
					return true
				}
				candidate := lsCheckCandidate(schema, typeRef, value, bits, checked.symbols)
				if candidate.problem != nil {
					checked.problem = candidate.problem
				}
				return !candidate.rejected
			})
			result = rapid.Custom(func(t *rapid.T) LawSpecValue {
				// Do not draw more fields after an evaluator failure.
				if checked.problem != nil {
					return rapid.Just(LawSpecValue{}).Draw(t, "failed")
				}
				return filtered.Draw(t, "checked")
			})
		}
		arbitraries[key] = result
		return result
	}
	return build(reference, budget)
}

const (
	lsIndexSlack   = 16
	lsIndexChoices = 6
)

// lsIndexTerm is a prefix index term over field indices (f<i>), literals
// (c<n>) and the natural operators + - * div mod ^.
type lsIndexTerm struct {
	op          string
	value       int64
	left, right *lsIndexTerm
}

func lsParseIndexTerm(tokens []string, at int) (*lsIndexTerm, int) {
	if at >= len(tokens) {
		panic("malformed index term")
	}
	token := tokens[at]
	switch {
	case token[0] == 'c' || token[0] == 'f':
		value, err := strconv.ParseInt(token[1:], 10, 64)
		if err != nil {
			panic("malformed index term")
		}
		return &lsIndexTerm{op: token[:1], value: value}, at + 1
	case slices.Contains([]string{"+", "-", "*", "div", "mod", "^"}, token):
		left, next := lsParseIndexTerm(tokens, at+1)
		right, end := lsParseIndexTerm(tokens, next)
		return &lsIndexTerm{op: token, left: left, right: right}, end
	}
	panic("malformed index term")
}

// lsEvalIndexTerm is natural index arithmetic; false when an operation has
// no natural value or leaves the int64 range.
func lsEvalIndexTerm(term *lsIndexTerm, fields map[int]int64) (int64, bool) {
	switch term.op {
	case "c":
		return term.value, true
	case "f":
		value, ok := fields[int(term.value)]
		return value, ok
	}
	x, ok := lsEvalIndexTerm(term.left, fields)
	if !ok {
		return 0, false
	}
	y, ok := lsEvalIndexTerm(term.right, fields)
	if !ok {
		return 0, false
	}
	result := new(big.Int)
	switch term.op {
	case "+":
		result.Add(big.NewInt(x), big.NewInt(y))
	case "-":
		if x < y {
			return 0, false
		}
		result.SetInt64(x - y)
	case "*":
		result.Mul(big.NewInt(x), big.NewInt(y))
	case "div", "mod":
		if y <= 0 {
			return 0, false
		}
		if term.op == "div" {
			result.SetInt64(x / y)
		} else {
			result.SetInt64(x % y)
		}
	default:
		if y > 64 {
			return 0, false
		}
		result.Exp(big.NewInt(x), big.NewInt(y), nil)
	}
	if !result.IsInt64() {
		return 0, false
	}
	return result.Int64(), true
}

func lsIndexTermFields(term *lsIndexTerm) []int {
	switch term.op {
	case "c":
		return nil
	case "f":
		return []int{int(term.value)}
	}
	return append(lsIndexTermFields(term.left), lsIndexTermFields(term.right)...)
}

type lsIndexGuard struct {
	relation    string
	left, right *lsIndexTerm
}

type lsIndexEquation struct {
	term      *lsIndexTerm
	guards    []lsIndexGuard
	positions []int
}

// lsIndexedDataStrategy constructs values whose structural index equals
// target. Each constructor carries its index term then its guards, in prefix
// notation over field indices. Reachability is a forward fixpoint over levels
// 0..target+slack, so a child may exceed its parent's index; the target is
// then solved backwards, and nothing is filtered away.
func lsIndexedDataStrategy(schema *lawSpecSchema, reference lawSpecTypeRef, bits, budget int,
	target LawSpecValue, equations map[string][]string,
	scalar func(string) *rapid.Generator[LawSpecValue]) *rapid.Generator[lawSpecCheckedValue] {
	number, ok := target.Data.(*big.Int)
	if !ok || !number.IsInt64() {
		panic("index target must be an integer")
	}
	if _, custom := schema.constructors(reference); !custom {
		panic("indexed generation requires a data type")
	}
	goal := number.Int64()
	limit := max(goal, 0) + lsIndexSlack
	plain := map[string]*rapid.Generator[LawSpecValue]{}
	plainField := func(typeRef lawSpecTypeRef) (generator *rapid.Generator[LawSpecValue]) {
		if cached, exists := plain[typeRef.key()]; exists {
			return cached
		}
		defer func() {
			if recover() != nil {
				generator = nil
			}
			plain[typeRef.key()] = generator
		}()
		return lsBuildDataStrategy(schema, typeRef, bits, budget, scalar, nil)
	}
	parsed := map[string]lsIndexEquation{}
	equation := func(tag string) lsIndexEquation {
		if found, exists := parsed[tag]; exists {
			return found
		}
		texts, exists := equations[tag]
		if !exists || len(texts) == 0 {
			panic("missing index equation for " + tag)
		}
		tokens := strings.Fields(texts[0])
		term, end := lsParseIndexTerm(tokens, 0)
		if end != len(tokens) {
			panic("malformed index term")
		}
		result := lsIndexEquation{term: term}
		positions := lsIndexTermFields(term)
		for _, text := range texts[1:] {
			parts := strings.Fields(text)
			if len(parts) == 0 || (parts[0] != "==" && parts[0] != ">=") {
				panic("malformed index guard")
			}
			left, next := lsParseIndexTerm(parts, 1)
			right, _ := lsParseIndexTerm(parts, next)
			result.guards = append(result.guards, lsIndexGuard{parts[0], left, right})
			positions = append(append(positions, lsIndexTermFields(left)...), lsIndexTermFields(right)...)
		}
		for _, position := range positions {
			if !slices.Contains(result.positions, position) {
				result.positions = append(result.positions, position)
			}
		}
		parsed[tag] = result
		return result
	}
	ready := func(constructor lawSpecConstructorSchema) bool {
		positions := equation(constructor.tag).positions
		for index, field := range constructor.fields {
			if !slices.Contains(positions, index) && plainField(field.typeRef) == nil {
				return false
			}
		}
		return true
	}
	families := []lawSpecTypeRef{}
	seen := map[string]bool{}
	pending := []lawSpecTypeRef{reference}
	for len(pending) > 0 {
		typeRef := pending[len(pending)-1]
		pending = pending[:len(pending)-1]
		if seen[typeRef.key()] {
			continue
		}
		constructors, custom := schema.constructors(typeRef)
		if !custom {
			panic("indexed generation requires a data type")
		}
		seen[typeRef.key()] = true
		families = append(families, typeRef)
		for _, constructor := range constructors {
			for _, position := range equation(constructor.tag).positions {
				pending = append(pending, constructor.fields[position].typeRef)
			}
		}
	}
	type level struct {
		key   string
		index int64
	}
	type solution struct {
		value      int64
		assignment map[int]int64
	}
	// assignments lists every assignment of reachable indices to the index
	// fields that satisfies the guards, with the index it produces.
	assignments := func(constructor lawSpecConstructorSchema, reach map[level]bool) []solution {
		found := equation(constructor.tag)
		results := []solution{}
		current := map[int]int64{}
		var extend func(int)
		extend = func(at int) {
			if at == len(found.positions) {
				for _, guard := range found.guards {
					x, okLeft := lsEvalIndexTerm(guard.left, current)
					y, okRight := lsEvalIndexTerm(guard.right, current)
					if !okLeft || !okRight || (guard.relation == "==" && x != y) || (guard.relation == ">=" && x < y) {
						return
					}
				}
				if value, ok := lsEvalIndexTerm(found.term, current); ok && value <= limit {
					copied := make(map[int]int64, len(current))
					for position, index := range current {
						copied[position] = index
					}
					results = append(results, solution{value, copied})
				}
				return
			}
			position := found.positions[at]
			for value := int64(0); value <= limit; value++ {
				if reach[level{constructor.fields[position].typeRef.key(), value}] {
					current[position] = value
					extend(at + 1)
				}
			}
			delete(current, position)
		}
		extend(0)
		return results
	}
	reach := map[level]bool{}
	for {
		grown := false
		next := map[level]bool{}
		for key := range reach {
			next[key] = true
		}
		for _, typeRef := range families {
			constructors, _ := schema.constructors(typeRef)
			for _, constructor := range constructors {
				if !ready(constructor) {
					continue
				}
				for _, found := range assignments(constructor, reach) {
					key := level{typeRef.key(), found.value}
					if !next[key] {
						next[key] = true
						grown = true
					}
				}
			}
		}
		reach = next
		if !grown {
			break
		}
	}
	// An open target (negative), or one drawn from earlier inputs that breaks
	// their preconditions or names no value, generates from the smallest
	// reachable indices; an index claim rejects a mismatch.
	levels := []int64{goal}
	if !reach[level{reference.key(), goal}] {
		levels = nil
		for candidate := int64(0); candidate <= limit && len(levels) < lsIndexChoices; candidate++ {
			if reach[level{reference.key(), candidate}] {
				levels = append(levels, candidate)
			}
		}
		if len(levels) == 0 {
			panic(fmt.Sprintf("no value of %s has an index", reference.key()))
		}
	}
	solutions := map[string][]map[int]int64{}
	solve := func(constructor lawSpecConstructorSchema, k int64) []map[int]int64 {
		key := constructor.tag + "@" + strconv.FormatInt(k, 10)
		if found, exists := solutions[key]; exists {
			return found
		}
		matches := []map[int]int64{}
		for _, found := range assignments(constructor, reach) {
			if found.value == k {
				matches = append(matches, found.assignment)
			}
		}
		solutions[key] = matches
		return matches
	}
	var draw func(*rapid.T, lawSpecTypeRef, int64) LawSpecValue
	draw = func(t *rapid.T, typeRef lawSpecTypeRef, k int64) LawSpecValue {
		constructors, _ := schema.constructors(typeRef)
		options := []lawSpecConstructorSchema{}
		for _, constructor := range constructors {
			if ready(constructor) && len(solve(constructor, k)) > 0 {
				options = append(options, constructor)
			}
		}
		if len(options) == 0 {
			panic(fmt.Sprintf("no value of %s has index %d", typeRef.key(), k))
		}
		constructor := options[rapid.IntRange(0, len(options)-1).Draw(t, "constructor")]
		choices := solve(constructor, k)
		targets := choices[rapid.IntRange(0, len(choices)-1).Draw(t, "split")]
		values := make([]LawSpecValue, len(constructor.fields))
		for position, field := range constructor.fields {
			if childIndex, indexed := targets[position]; indexed {
				values[position] = draw(t, field.typeRef, childIndex)
			} else {
				values[position] = plainField(field.typeRef).Draw(t, field.name)
			}
		}
		return schema.construct(typeRef, constructor.tag, values, bits)
	}
	return rapid.Custom(func(t *rapid.T) lawSpecCheckedValue {
		chosen := levels[rapid.IntRange(0, len(levels)-1).Draw(t, "index")]
		return lawSpecCheckedValue{value: draw(t, reference, chosen)}
	})
}

// Generated collections are canonicalised rather than filtered: a Set's
// items and a KeyVal's keys are sorted and distinct.
var lsCanonicalKeys = map[string]func(LawSpecValue) LawSpecValue{
	"lawspec.collections::type::Set":    func(item LawSpecValue) LawSpecValue { return item },
	"lawspec.collections::type::KeyVal": func(entry LawSpecValue) LawSpecValue { return entry.Data.(lawSpecData).fields[0] },
}

func lsCanonical(value LawSpecValue, key func(LawSpecValue) LawSpecValue) LawSpecValue {
	data := value.Data.(lawSpecData)
	list := data.fields[0]
	items := slices.Clone(list.Data.([]LawSpecValue))
	slices.SortStableFunc(items, func(a, b LawSpecValue) int { return lsCompareValues(key(a), key(b)) })
	distinct := []LawSpecValue{}
	for index, item := range items {
		if index == 0 || lsCompareValues(key(items[index-1]), key(item)) != 0 {
			distinct = append(distinct, item)
		}
	}
	return LawSpecValue{value.Type, lawSpecData{data.tag, []LawSpecValue{{list.Type, distinct}}}}
}

// Equality-based sets retain first occurrences without invoking the portable order.
func lsCanonicalEqSet(value LawSpecValue) LawSpecValue {
	data := value.Data.(lawSpecData)
	list := data.fields[0]
	distinct := []LawSpecValue{}
	for _, item := range list.Data.([]LawSpecValue) {
		found := false
		for _, prior := range distinct {
			if lsEqual(item, prior) {
				found = true
				break
			}
		}
		if !found {
			distinct = append(distinct, item)
		}
	}
	return LawSpecValue{value.Type, lawSpecData{data.tag, []LawSpecValue{{list.Type, distinct}}}}
}
