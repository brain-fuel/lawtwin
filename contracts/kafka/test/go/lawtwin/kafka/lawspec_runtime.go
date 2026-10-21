// LawSpec's runtime for Go: the portable scalar domain, seeded generation,
// models, actors, sessions and nodes, with no test-framework dependency.
//
// A law must mean the same thing on every target, so this file implements
// LawSpec's own arithmetic, equality and conversions instead of Go's machine
// integers and float64: integers are exact and reach a bounded type only
// through a checked conversion, exact division gives a rational, decimals are
// exact, and floats follow IEEE 754 at their declared precision.
// ref:DEC-portable-exact-arithmetic ref:ieee-754 ref:decimal-arithmetic
//
// It is emitted into every generated Go package, so the generated tests and the
// adapters share one definition of the domain. ref:DEC-typed-core-boundary
package kafka

import (
	"bytes"
	cryptorand "crypto/rand"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"math/big"
	"math/rand"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	goruntime "runtime"
	"sort"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"
	"unicode/utf8"
)

// lsEvaluate preserves expression-local scope without letting Go's inliner
// expand hundreds of nested search closures into their callers.
//
//go:noinline
func lsEvaluate(expression func() LawSpecValue) LawSpecValue { return expression() }

// LawSpecValue is a value of a LawSpec type together with that type, because a
// Go value alone cannot say whether it is an Int32, a BigInt or a Decimal, and
// the law's semantics depend on which. ref:DEC-portable-exact-arithmetic
type LawSpecValue struct {
	Type string
	Data any
}
type lawSpecDecimal struct {
	coefficient *big.Int
	exponent    int
}
type lawSpecSymbol struct {
	description string
	// A workflow runtime travels in the symbols map under lsWorkflowKey.
	workflow *LawSpecWorkflowRuntime
	// So do a law's handlers, by ability, under lsHandlersKey.
	handlers map[string]any
	// On the handlers entry: the default workflow runtime as seen through
	// the Clock handler installed there (lsWorkflowRuntime).
	clockView *lawSpecClockView
}
type lawSpecPresence struct{ value *LawSpecValue }

// lawSpecHandle is a handle's logical value: the adapter's native value,
// carried unopened. Two handles are equal only when they are the same native
// value; they have no portable order.
type lawSpecHandle struct{ native any }

// lsSameNative is identity: == for comparable values, the same backing
// pointer for maps, slices, functions and channels.
func lsSameNative(x, y any) bool {
	if x == nil || y == nil {
		return x == nil && y == nil
	}
	tx, ty := reflect.TypeOf(x), reflect.TypeOf(y)
	if tx != ty {
		return false
	}
	if tx.Comparable() {
		same := false
		func() {
			defer func() { _ = recover() }()
			same = x == y
		}()
		return same
	}
	vx, vy := reflect.ValueOf(x), reflect.ValueOf(y)
	switch vx.Kind() {
	case reflect.Map, reflect.Func, reflect.Chan, reflect.Pointer, reflect.UnsafePointer:
		return vx.Pointer() == vy.Pointer()
	case reflect.Slice:
		return vx.Pointer() == vy.Pointer() && vx.Len() == vy.Len()
	}
	return false
}

// Handle labels, numbered per type by first appearance in the process.
var (
	lsHandleLock   sync.Mutex
	lsHandleLabels = map[string][]lawSpecHandleLabel{}
)

type lawSpecHandleLabel struct {
	native any
	number int
}

// lsHandleLabel is a handle's stable label, such as Jobs#1.
func lsHandleLabel(typeName string, native any) string {
	segments := strings.Split(typeName, "::")
	name := segments[len(segments)-1]
	lsHandleLock.Lock()
	defer lsHandleLock.Unlock()
	labels := lsHandleLabels[typeName]
	for _, label := range labels {
		if lsSameNative(label.native, native) {
			return name + "#" + strconv.Itoa(label.number)
		}
	}
	number := len(labels) + 1
	lsHandleLabels[typeName] = append(labels, lawSpecHandleLabel{native, number})
	return name + "#" + strconv.Itoa(number)
}

// Named support types preserve scalar domains in native data fields.
type LawSpecDecimal = lawSpecDecimal
type LawSpecSymbol = lawSpecSymbol
type LawSpecUnit struct{}
type LawSpecNull struct{}
type LawSpecUndefined struct{}

// Presence is tagged so nested nullable and optional states remain distinct.
type LawSpecNullable[T any] struct {
	Present bool
	Value   T
}

// LawSpecOptional keeps an absent value distinct from a present zero value,
// which Go's zero values cannot. ref:DEC-algebraic-maybe-either
type LawSpecOptional[T any] struct {
	Present bool
	Value   T
}

// LawSpecMaybe keeps algebraic absence separate from nullable payloads.
type LawSpecMaybe[T any] struct {
	present bool
	value   T
}

func LawSpecNothing[T any]() LawSpecMaybe[T]     { return LawSpecMaybe[T]{} }
func LawSpecJust[T any](value T) LawSpecMaybe[T] { return LawSpecMaybe[T]{true, value} }
func (m LawSpecMaybe[T]) Value() (T, bool)       { return m.value, m.present }
func (m LawSpecMaybe[T]) lawSpecTag() string {
	if m.present {
		return "Maybe::Just"
	}
	return "Maybe::Nothing"
}
func (m LawSpecMaybe[T]) lawSpecFields() []any {
	if m.present {
		return []any{m.value}
	}
	return nil
}

// LawSpecEither has an explicit constructor; its zero value is invalid.
type LawSpecEither[L, R any] struct {
	tag   uint8
	left  L
	right R
}

func LawSpecLeft[L, R any](value L) LawSpecEither[L, R] {
	return LawSpecEither[L, R]{tag: 1, left: value}
}
func LawSpecRight[L, R any](value R) LawSpecEither[L, R] {
	return LawSpecEither[L, R]{tag: 2, right: value}
}
func (e LawSpecEither[L, R]) Left() (L, bool)  { return e.left, e.tag == 1 }
func (e LawSpecEither[L, R]) Right() (R, bool) { return e.right, e.tag == 2 }
func (e LawSpecEither[L, R]) lawSpecTag() string {
	switch e.tag {
	case 1:
		return "Either::Left"
	case 2:
		return "Either::Right"
	default:
		panic("Either requires Left or Right")
	}
}
func (e LawSpecEither[L, R]) lawSpecFields() []any {
	switch e.tag {
	case 1:
		return []any{e.left}
	case 2:
		return []any{e.right}
	default:
		panic("Either requires Left or Right")
	}
}

type lawSpecSum interface {
	lawSpecTag() string
	lawSpecFields() []any
}
type lawSpecData struct {
	tag    string
	fields []LawSpecValue
}

func (d lawSpecData) String() string { return fmt.Sprintf("%s(%v)", d.tag, d.fields) }

func lsSumType(t string) bool {
	return strings.HasPrefix(t, "Maybe ") || strings.HasPrefix(t, "Either ")
}
func lsEitherArguments(t string) []string {
	if !strings.HasPrefix(t, "Either ") {
		panic("Either type required")
	}
	var result []string
	for cursor := 7; cursor < len(t); {
		if t[cursor] != '(' {
			panic("invalid Either type: " + t)
		}
		cursor++
		start, depth := cursor, 1
		for cursor < len(t) && depth > 0 {
			if t[cursor] == '(' {
				depth++
			}
			if t[cursor] == ')' {
				depth--
			}
			cursor++
		}
		if depth != 0 || cursor == start+1 {
			panic("invalid Either type: " + t)
		}
		result = append(result, t[start:cursor-1])
		if cursor < len(t) {
			if t[cursor] != ' ' || cursor+1 == len(t) {
				panic("invalid Either type: " + t)
			}
			cursor++
		}
	}
	if len(result) != 2 {
		panic("Either requires two type arguments")
	}
	return result
}
func lsSumFields(t, tag string) []string {
	if strings.HasPrefix(t, "Maybe ") {
		switch tag {
		case "Maybe::Nothing":
			return nil
		case "Maybe::Just":
			return []string{strings.TrimPrefix(t, "Maybe ")}
		}
	}
	if strings.HasPrefix(t, "Either ") {
		arguments := lsEitherArguments(t)
		switch tag {
		case "Either::Left":
			return arguments[:1]
		case "Either::Right":
			return arguments[1:]
		}
	}
	panic("invalid constructor " + tag + " for " + t)
}
func lsDataValue(value LawSpecValue) lawSpecData {
	data, ok := value.Data.(lawSpecData)
	if !ok || len(lsSumFields(value.Type, data.tag)) != len(data.fields) {
		panic("invalid sum representation")
	}
	return data
}
func lsMaybeToNative[T any](t string, value LawSpecValue, bits int, element func(LawSpecValue) T) LawSpecMaybe[T] {
	if !strings.HasPrefix(t, "Maybe ") {
		panic("Maybe type required")
	}
	lsCheckNativeProfile(t, bits)
	data := lsDataValue(lsConvert(t, value, bits))
	if data.tag == "Maybe::Nothing" {
		return LawSpecNothing[T]()
	}
	return LawSpecJust(element(data.fields[0]))
}
func lsEitherToNative[L, R any](t string, value LawSpecValue, bits int, left func(LawSpecValue) L, right func(LawSpecValue) R) LawSpecEither[L, R] {
	if !strings.HasPrefix(t, "Either ") {
		panic("Either type required")
	}
	lsCheckNativeProfile(t, bits)
	data := lsDataValue(lsConvert(t, value, bits))
	if data.tag == "Either::Left" {
		return LawSpecLeft[L, R](left(data.fields[0]))
	}
	return LawSpecRight[L, R](right(data.fields[0]))
}
func lsAllElements(value LawSpecValue, predicate func(LawSpecValue) LawSpecValue) LawSpecValue {
	values, ok := value.Data.([]LawSpecValue)
	if !ok {
		panic("expected List in element predicate")
	}
	index := 0
	defer func() {
		if failure := recover(); failure != nil {
			if raised, ok := failure.(*LawSpecFailure); ok {
				panic(raised)
			}
			panic(fmt.Sprintf("List element %d: %v", index, failure))
		}
	}()
	for index = range values {
		if !lsTruth(predicate(values[index])) {
			return lsBool(false)
		}
	}
	return lsBool(true)
}

func lsMatchMaybe(value LawSpecValue, nothing func() LawSpecValue, just func(LawSpecValue) LawSpecValue) LawSpecValue {
	data := lsDataValue(value)
	switch data.tag {
	case "Maybe::Nothing":
		return nothing()
	case "Maybe::Just":
		return just(data.fields[0])
	default:
		panic("Maybe value required")
	}
}
func lsMatchEither(value LawSpecValue, left, right func(LawSpecValue) LawSpecValue) LawSpecValue {
	data := lsDataValue(value)
	switch data.tag {
	case "Either::Left":
		return left(data.fields[0])
	case "Either::Right":
		return right(data.fields[0])
	default:
		panic("Either value required")
	}
}

// Sequences: Text and Bytes hold their units, a List its values. Indices
// count code points, octets or items, and a slice is clamped.
func lsSequenceHelper(n string, args []LawSpecValue) (LawSpecValue, bool) {
	clamp := func(v LawSpecValue, count int) int {
		k := v.Data.(*big.Int)
		if k.Sign() < 0 {
			return 0
		}
		if !k.IsInt64() || k.Int64() > int64(count) {
			return count
		}
		return int(k.Int64())
	}
	switch n {
	case "append":
		if units, ok := args[0].Data.([]int); ok {
			return LawSpecValue{args[0].Type, append(append([]int{}, units...), args[1].Data.([]int)...)}, true
		}
		return lsList(args[0].Type, append(append([]LawSpecValue{}, args[0].Data.([]LawSpecValue)...), args[1].Data.([]LawSpecValue)...)), true
	case "slice":
		if units, ok := args[0].Data.([]int); ok {
			i := clamp(args[1], len(units))
			j := clamp(args[2], len(units))
			if j < i {
				j = i
			}
			return LawSpecValue{args[0].Type, append([]int{}, units[i:j]...)}, true
		}
		values := args[0].Data.([]LawSpecValue)
		i := clamp(args[1], len(values))
		j := clamp(args[2], len(values))
		if j < i {
			j = i
		}
		return lsList(args[0].Type, values[i:j]), true
	case "at":
		index := args[1].Data.(*big.Int)
		if units, ok := args[0].Data.([]int); ok {
			if index.Sign() < 0 || !index.IsInt64() || index.Int64() >= int64(len(units)) {
				return LawSpecValue{"Maybe UInt8", lawSpecData{"Maybe::Nothing", nil}}, true
			}
			return LawSpecValue{"Maybe UInt8", lawSpecData{"Maybe::Just", []LawSpecValue{lsInteger("UInt8", strconv.Itoa(units[index.Int64()]))}}}, true
		}
		values := args[0].Data.([]LawSpecValue)
		maybe := "Maybe " + strings.TrimPrefix(args[0].Type, "List ")
		if index.Sign() < 0 || !index.IsInt64() || index.Int64() >= int64(len(values)) {
			return LawSpecValue{maybe, lawSpecData{"Maybe::Nothing", nil}}, true
		}
		return LawSpecValue{maybe, lawSpecData{"Maybe::Just", []LawSpecValue{values[index.Int64()]}}}, true
	case "range":
		values := []LawSpecValue{}
		hi := args[1].Data.(*big.Int)
		for k := new(big.Int).Set(args[0].Data.(*big.Int)); k.Cmp(hi) < 0; k = new(big.Int).Add(k, big.NewInt(1)) {
			values = append(values, LawSpecValue{args[0].Type, k})
		}
		return lsList("List "+args[0].Type, values), true
	case "octets":
		values := []LawSpecValue{}
		for _, unit := range args[0].Data.([]int) {
			values = append(values, lsInteger("UInt8", strconv.Itoa(unit)))
		}
		return lsList("List UInt8", values), true
	case "bytesOf":
		units := []int{}
		for _, v := range args[0].Data.([]LawSpecValue) {
			units = append(units, int(v.Data.(*big.Int).Int64()))
		}
		return LawSpecValue{"Bytes", units}, true
	case "utf8":
		units := []int{}
		for _, b := range []byte(lsCodePointsText(args[0].Data.([]int))) {
			units = append(units, int(b))
		}
		return LawSpecValue{"Bytes", units}, true
	case "textOf":
		return LawSpecValue{"Text", lsTextUnits(args[0].Data.(*big.Int).String())}, true
	case "join":
		parts := []string{}
		for _, v := range args[1].Data.([]LawSpecValue) {
			parts = append(parts, lsCodePointsText(v.Data.([]int)))
		}
		return LawSpecValue{"Text", lsTextUnits(strings.Join(parts, lsCodePointsText(args[0].Data.([]int))))}, true
	}
	return LawSpecValue{}, false
}

func lsList(t string, values []LawSpecValue) LawSpecValue {
	return LawSpecValue{t, append([]LawSpecValue{}, values...)}
}

func lsConstruct(t, tag string, fields []LawSpecValue) LawSpecValue {
	if lsSumType(t) {
		if len(lsSumFields(t, tag)) != len(fields) {
			panic("invalid constructor arity: " + tag)
		}
		return LawSpecValue{t, lawSpecData{tag, append([]LawSpecValue{}, fields...)}}
	}
	switch tag {
	case "List::Nil":
		if len(fields) == 0 {
			return lsList(t, nil)
		}
	case "List::Cons":
		if len(fields) == 2 {
			if tail, ok := fields[1].Data.([]LawSpecValue); ok {
				return lsList(t, append([]LawSpecValue{fields[0]}, tail...))
			}
		}
	}
	panic("invalid constructor or arity: " + tag)
}

func lsSignedInteger(t string, value int64) LawSpecValue {
	return lsInteger(t, strconv.FormatInt(value, 10))
}

func lsUnsignedInteger(t string, value uint64) LawSpecValue {
	return lsInteger(t, strconv.FormatUint(value, 10))
}

func lsListToNative[T any](t string, value LawSpecValue, bits int, element func(LawSpecValue) T) []T {
	lsCheckNativeProfile(t, bits)
	values := lsConvert(t, value, bits).Data.([]LawSpecValue)
	result := make([]T, len(values))
	for index, item := range values {
		result[index] = element(item)
	}
	return result
}

func lsCheckNativeProfile(t string, bits int) {
	if strings.HasPrefix(t, "List ") || strings.HasPrefix(t, "Maybe ") {
		lsCheckNativeProfile(strings.SplitN(t, " ", 2)[1], bits)
		return
	}
	if strings.HasPrefix(t, "Either ") {
		for _, argument := range lsEitherArguments(t) {
			lsCheckNativeProfile(argument, bits)
		}
		return
	}
	if (t == "IntSize" || t == "UIntSize" || t == "UIntPtr") && strconv.IntSize != bits {
		panic("machineBits does not match native architecture")
	}
}

func lsIntegerType(t string) bool {
	return strings.HasPrefix(t, "Int") || strings.HasPrefix(t, "UInt") || t == "BigInt" || t == "BigUInt"
}
func lsExactType(t string) bool { return lsIntegerType(t) || t == "Decimal" || t == "Rational" }
func lsInt(s string) *big.Int {
	n, ok := new(big.Int).SetString(s, 10)
	if !ok {
		panic("invalid integer")
	}
	return n
}
func lsInteger(t, n string) LawSpecValue { return LawSpecValue{t, lsInt(n)} }
func lsBool(b bool) LawSpecValue         { return LawSpecValue{"Bool", b} }
func lsDecimal(c, e string) LawSpecValue {
	n, err := strconv.Atoi(e)
	if err != nil {
		panic(err)
	}
	return LawSpecValue{"Decimal", lawSpecDecimal{lsInt(c), n}}
}
func lsRational(n, d string) LawSpecValue {
	return LawSpecValue{"Rational", new(big.Rat).SetFrac(lsInt(n), lsInt(d))}
}
func lsFloating(t, b string) LawSpecValue {
	n, err := strconv.ParseUint(b, 16, 64)
	if err != nil {
		panic(err)
	}
	v := math.Float64frombits(n)
	if t == "Float32" {
		v = float64(math.Float32frombits(uint32(n)))
	}
	return LawSpecValue{t, v}
}
func lsComplex(t string, r, i LawSpecValue) LawSpecValue {
	return LawSpecValue{t, complex(r.Data.(float64), i.Data.(float64))}
}
func lsSequence(t string, u []int) LawSpecValue        { return LawSpecValue{t, append([]int{}, u...)} }
func lsCharacter(t string, c int) LawSpecValue         { return LawSpecValue{t, c} }
func lsAbsent(t string) LawSpecValue                   { return LawSpecValue{t, nil} }
func lsPresent(t string, v *LawSpecValue) LawSpecValue { return LawSpecValue{t, lawSpecPresence{v}} }
func lsPointer(v LawSpecValue) *LawSpecValue           { return &v }
func lsSymbol(id, d string, symbols map[string]*lawSpecSymbol) LawSpecValue {
	s, ok := symbols[id]
	if !ok {
		s = &lawSpecSymbol{description: d}
		symbols[id] = s
	}
	return LawSpecValue{"Symbol", s}
}
func lsPow(n int) *big.Int { return new(big.Int).Exp(big.NewInt(10), big.NewInt(int64(n)), nil) }
func lsRatio(v LawSpecValue) *big.Rat {
	switch x := v.Data.(type) {
	case *big.Int:
		return new(big.Rat).SetInt(x)
	case *big.Rat:
		return new(big.Rat).Set(x)
	case lawSpecDecimal:
		if x.exponent >= 0 {
			return new(big.Rat).SetInt(new(big.Int).Mul(x.coefficient, lsPow(x.exponent)))
		}
		return new(big.Rat).SetFrac(x.coefficient, lsPow(-x.exponent))
	case float64:
		r := new(big.Rat).SetFloat64(x)
		if r == nil {
			panic("non-finite exact conversion")
		}
		return r
	default:
		panic("exact numeric value required")
	}
}
func lsReal(v LawSpecValue) float64 {
	if x, ok := v.Data.(float64); ok {
		return x
	}
	r, _ := lsRatio(v).Float64()
	return r
}
func lsPrecision(t string, x float64) float64 {
	if t == "Float32" || t == "Complex64" {
		return float64(float32(x))
	}
	return x
}

// lsConvert moves a value into the declared type, failing instead of wrapping
// or rounding silently, because a bounded type is only ever reached through a
// checked conversion. ref:DEC-portable-exact-arithmetic
func lsConvert(t string, v LawSpecValue, bits int) LawSpecValue {
	if lsSumType(t) {
		data := lsDataValue(v)
		types := lsSumFields(t, data.tag)
		fields := make([]LawSpecValue, len(types))
		for index, fieldType := range types {
			fields[index] = lsConvert(fieldType, data.fields[index], bits)
		}
		return lsConstruct(t, data.tag, fields)
	}
	if strings.HasPrefix(t, "List ") {
		values, ok := v.Data.([]LawSpecValue)
		if !ok || !strings.HasPrefix(v.Type, "List ") {
			panic("List required")
		}
		converted := make([]LawSpecValue, len(values))
		for index, value := range values {
			converted[index] = lsConvert(strings.TrimPrefix(t, "List "), value, bits)
		}
		return lsList(t, converted)
	}
	if strings.HasPrefix(t, "Nullable ") || strings.HasPrefix(t, "Optional ") {
		parts := strings.SplitN(t, " ", 2)
		missing := "Null"
		if parts[0] == "Optional" {
			missing = "Undefined"
		}
		if v.Type == missing {
			return LawSpecValue{t, lawSpecPresence{nil}}
		}
		p, ok := v.Data.(lawSpecPresence)
		if !ok {
			panic("tagged presence required")
		}
		if p.value == nil {
			return LawSpecValue{t, p}
		}
		x := lsConvert(parts[1], *p.value, bits)
		return LawSpecValue{t, lawSpecPresence{&x}}
	}
	if lsIntegerType(t) {
		r := lsRatio(v)
		if !r.IsInt() {
			panic("fractional conversion to " + t)
		}
		n := new(big.Int).Set(r.Num())
		if t != "BigInt" && t != "Integer" {
			unsigned := strings.HasPrefix(t, "U") || t == "BigUInt"
			if unsigned && n.Sign() < 0 {
				panic("integer outside " + t + " range")
			}
			if t != "BigUInt" {
				w := bits
				if t != "IntSize" && t != "UIntSize" && t != "UIntPtr" {
					i := 3
					if unsigned {
						i = 4
					}
					var err error
					w, err = strconv.Atoi(t[i:])
					if err != nil {
						panic(err)
					}
				}
				p := w
				if !unsigned {
					p--
				}
				limit := new(big.Int).Lsh(big.NewInt(1), uint(p))
				lo := new(big.Int)
				if !unsigned {
					lo.Neg(limit)
				}
				hi := new(big.Int).Sub(limit, big.NewInt(1))
				if n.Cmp(lo) < 0 || n.Cmp(hi) > 0 {
					panic("integer outside " + t + " range")
				}
			}
		}
		return LawSpecValue{t, n}
	}
	if t == "Rational" {
		return LawSpecValue{t, lsRatio(v)}
	}
	if t == "Decimal" {
		r := lsRatio(v)
		d := new(big.Int).Set(r.Denom())
		a, b := 0, 0
		rem := new(big.Int)
		for rem.Mod(d, big.NewInt(2)).Sign() == 0 {
			d.Div(d, big.NewInt(2))
			a++
		}
		for rem.Mod(d, big.NewInt(5)).Sign() == 0 {
			d.Div(d, big.NewInt(5))
			b++
		}
		if d.Cmp(big.NewInt(1)) != 0 {
			panic("Decimal conversion is not finite; use round")
		}
		scale := max(a, b)
		c := new(big.Int).Set(r.Num())
		c.Mul(c, new(big.Int).Exp(big.NewInt(2), big.NewInt(int64(scale-a)), nil))
		c.Mul(c, new(big.Int).Exp(big.NewInt(5), big.NewInt(int64(scale-b)), nil))
		return LawSpecValue{t, lawSpecDecimal{c, -scale}}
	}
	if strings.HasPrefix(t, "Float") {
		if t == "Float32" && lsExactType(v.Type) {
			x, _ := lsRatio(v).Float32()
			return LawSpecValue{t, float64(x)}
		}
		return LawSpecValue{t, lsPrecision(t, lsReal(v))}
	}
	if strings.HasPrefix(t, "Complex") {
		z, ok := v.Data.(complex128)
		if !ok {
			component := "Float64"
			if t == "Complex64" {
				component = "Float32"
			}
			z = complex(lsConvert(component, v, bits).Data.(float64), 0)
		}
		return LawSpecValue{t, complex(lsPrecision(t, real(z)), lsPrecision(t, imag(z)))}
	}
	if t != v.Type {
		panic("cannot convert " + v.Type + " to " + t)
	}
	return lsValidate(t, v, bits)
}
func lsValidUnit(t string, c int) bool {
	hi := 1114111
	if t == "Bytes" {
		hi = 255
	}
	if t == "Utf16Text" || t == "CodeUnit16" {
		hi = 65535
	}
	return c >= 0 && c <= hi && (!(t == "Text" || t == "Char") || c < 55296 || c > 57343)
}

// lsValidate checks a value an adapter produced against its declared domain:
// native code may return anything its own type allows, and a law quantifies
// only over the declared domain. ref:DEC-portable-exact-arithmetic
func lsValidate(t string, v LawSpecValue, bits int) LawSpecValue {
	if t != v.Type {
		panic("invalid " + t + " representation")
	}
	if lsSumType(t) {
		data := lsDataValue(v)
		for index, fieldType := range lsSumFields(t, data.tag) {
			lsValidate(fieldType, data.fields[index], bits)
		}
		return lsClone(v)
	}
	if strings.HasPrefix(t, "List ") {
		values, ok := v.Data.([]LawSpecValue)
		if !ok {
			panic("List required")
		}
		for _, value := range values {
			lsValidate(strings.TrimPrefix(t, "List "), value, bits)
		}
		return lsClone(v)
	}
	if lsIntegerType(t) {
		return lsConvert(t, v, bits)
	}
	valid := false
	if strings.HasPrefix(t, "Nullable ") || strings.HasPrefix(t, "Optional ") {
		x, ok := v.Data.(lawSpecPresence)
		valid = ok
		if ok && x.value != nil {
			lsValidate(strings.SplitN(t, " ", 2)[1], *x.value, bits)
		}
	} else {
		switch t {
		case "Text", "CodePointText", "Utf16Text", "Bytes":
			x, ok := v.Data.([]int)
			valid = ok
			if ok {
				for _, c := range x {
					if !lsValidUnit(t, c) {
						valid = false
					}
				}
			}
		case "Char", "CodePoint", "CodeUnit16":
			x, ok := v.Data.(int)
			valid = ok && lsValidUnit(t, x)
		case "Bool":
			_, valid = v.Data.(bool)
		case "Decimal":
			x, ok := v.Data.(lawSpecDecimal)
			valid = ok && x.coefficient != nil
		case "Rational":
			x, ok := v.Data.(*big.Rat)
			valid = ok && x != nil
		case "Float32", "Float64":
			x, ok := v.Data.(float64)
			valid = ok && (t == "Float64" || math.IsNaN(x) || float64(float32(x)) == x)
		case "Complex64", "Complex128":
			x, ok := v.Data.(complex128)
			valid = ok
			if ok && t == "Complex64" {
				lsValidate("Float32", LawSpecValue{"Float32", real(x)}, bits)
				lsValidate("Float32", LawSpecValue{"Float32", imag(x)}, bits)
			}
		case "Symbol":
			x, ok := v.Data.(*lawSpecSymbol)
			valid = ok && x != nil
		case "Unit", "Null", "Undefined":
			valid = v.Data == nil
		}
	}
	if !valid {
		panic("invalid " + t + " representation")
	}
	return v
}
func lsPromote(a, b, op string) string {
	if lsExactType(a) != lsExactType(b) {
		panic("exact/inexact mixing requires explicit conversion")
	}
	if lsExactType(a) {
		if op == "/" || a == "Rational" || b == "Rational" {
			return "Rational"
		}
		if a == "Decimal" || b == "Decimal" {
			return "Decimal"
		}
		return "Integer"
	}
	if strings.HasPrefix(a, "Complex") || strings.HasPrefix(b, "Complex") {
		if a == "Float64" || b == "Float64" || a == "Complex128" || b == "Complex128" {
			return "Complex128"
		}
		return "Complex64"
	}
	if a == "Float64" || b == "Float64" {
		return "Float64"
	}
	return "Float32"
}
func lsComparison(op string, c int) bool {
	switch op {
	case "==":
		return c == 0
	case "!=":
		return c != 0
	case "<":
		return c < 0
	case "<=":
		return c <= 0
	case ">":
		return c > 0
	case ">=":
		return c >= 0
	}
	panic("unknown comparison")
}

// lsBinary applies a LawSpec operator with LawSpec's semantics rather than
// Go's, so a law computes the same result on every target.
// ref:DEC-portable-exact-arithmetic ref:ieee-754
func lsBinary(op string, a, b LawSpecValue) LawSpecValue {
	if (op == "==" || op == "!=") && !lsExactType(a.Type) && !strings.HasPrefix(a.Type, "Float") && !strings.HasPrefix(a.Type, "Complex") {
		return lsBool((op == "==") == lsEqual(a, b))
	}
	t := lsPromote(a.Type, b.Type, op)
	comparison := strings.Contains(" == != < <= > >= ", " "+op+" ")
	if lsExactType(a.Type) {
		x, y := lsRatio(a), lsRatio(b)
		if comparison {
			return lsBool(lsComparison(op, x.Cmp(y)))
		}
		r := new(big.Rat)
		switch op {
		case "+":
			r.Add(x, y)
		case "-":
			r.Sub(x, y)
		case "*":
			r.Mul(x, y)
		case "/":
			r.Quo(x, y)
		case "pow":
			if !x.IsInt() || !y.IsInt() {
				panic("integer operands required")
			}
			if y.Num().Sign() < 0 {
				panic("negative exponent")
			}
			return LawSpecValue{"Integer", new(big.Int).Exp(x.Num(), y.Num(), nil)}
		case "quot", "rem":
			if !x.IsInt() || !y.IsInt() {
				panic("integer operands required")
			}
			q := new(big.Int)
			if op == "quot" {
				q.Quo(x.Num(), y.Num())
			} else {
				q.Rem(x.Num(), y.Num())
			}
			return LawSpecValue{"Integer", q}
		default:
			panic("unknown operator")
		}
		return lsConvert(t, LawSpecValue{"Rational", r}, 64)
	}
	if strings.HasPrefix(t, "Complex") {
		x, y := lsConvert(t, a, 64).Data.(complex128), lsConvert(t, b, 64).Data.(complex128)
		ar, ai, br, bi := real(x), imag(x), real(y), imag(y)
		rnd := func(x float64) float64 { return lsPrecision(t, x) }
		var re, im float64
		switch op {
		case "==":
			return lsBool(x == y)
		case "!=":
			return lsBool(x != y)
		case "+":
			re, im = ar+br, ai+bi
		case "-":
			re, im = ar-br, ai-bi
		case "*":
			re, im = rnd(ar*br)-rnd(ai*bi), rnd(ar*bi)+rnd(ai*br)
		case "/":
			d := rnd(rnd(br*br) + rnd(bi*bi))
			re, im = rnd(rnd(ar*br)+rnd(ai*bi))/d, rnd(rnd(ai*br)-rnd(ar*bi))/d
		default:
			panic("complex values are not ordered")
		}
		return LawSpecValue{t, complex(rnd(re), rnd(im))}
	}
	x, y := lsReal(a), lsReal(b)
	if comparison {
		switch op {
		case "==":
			return lsBool(x == y)
		case "!=":
			return lsBool(x != y)
		case "<":
			return lsBool(x < y)
		case "<=":
			return lsBool(x <= y)
		case ">":
			return lsBool(x > y)
		case ">=":
			return lsBool(x >= y)
		}
	}
	var v float64
	switch op {
	case "+":
		v = x + y
	case "-":
		v = x - y
	case "*":
		v = x * y
	case "/":
		v = x / y
	default:
		panic("unknown operator")
	}
	return LawSpecValue{t, lsPrecision(t, v)}
}

// lsEqual is equality defined once for all targets: NaN differs from itself,
// signed zeros are equal, handles and symbols compare by identity, and data
// compares field by field. ref:DEC-portable-exact-arithmetic
func lsEqual(a, b LawSpecValue) bool {
	if x, ok := a.Data.(lawSpecData); ok {
		y, ok := b.Data.(lawSpecData)
		if !ok || x.tag != y.tag || len(x.fields) != len(y.fields) {
			return false
		}
		if x.tag == lsEqSetTag {
			return lsSameMembers(lsEqSetItems(x), lsEqSetItems(y), lsEqual)
		}
		for index, field := range x.fields {
			if !lsEqual(field, y.fields[index]) {
				return false
			}
		}
		return true
	}
	if lsSumType(a.Type) || lsSumType(b.Type) {
		if !lsSumType(a.Type) || !lsSumType(b.Type) {
			return false
		}
		x, y := lsDataValue(a), lsDataValue(b)
		if x.tag != y.tag || len(x.fields) != len(y.fields) {
			return false
		}
		for index, field := range x.fields {
			if !lsEqual(field, y.fields[index]) {
				return false
			}
		}
		return true
	}
	if strings.HasPrefix(a.Type, "List ") && strings.HasPrefix(b.Type, "List ") {
		xs, ys := a.Data.([]LawSpecValue), b.Data.([]LawSpecValue)
		if len(xs) != len(ys) {
			return false
		}
		for index, value := range xs {
			if !lsEqual(value, ys[index]) {
				return false
			}
		}
		return true
	}
	if (strings.HasPrefix(a.Type, "Float") || strings.HasPrefix(a.Type, "Complex")) && (strings.HasPrefix(b.Type, "Float") || strings.HasPrefix(b.Type, "Complex")) {
		return lsTruth(lsBinary("==", a, b))
	}
	if lsExactType(a.Type) && lsExactType(b.Type) {
		return lsRatio(a).Cmp(lsRatio(b)) == 0
	}
	if a.Type != b.Type {
		return false
	}
	switch x := a.Data.(type) {
	case float64:
		return x == b.Data.(float64)
	case complex128:
		return x == b.Data.(complex128)
	case *lawSpecSymbol:
		return x == b.Data.(*lawSpecSymbol)
	case lawSpecHandle:
		y, ok := b.Data.(lawSpecHandle)
		return ok && lsSameNative(x.native, y.native)
	case lawSpecPresence:
		y := b.Data.(lawSpecPresence)
		if x.value == nil || y.value == nil {
			return x.value == nil && y.value == nil
		}
		return lsEqual(*x.value, *y.value)
	}
	return reflect.DeepEqual(a.Data, b.Data)
}
func lsTruth(v LawSpecValue) bool { return lsValidate("Bool", v, 64).Data.(bool) }

const lsEqSetTag = "lawspec.collections::type::EqSet::EqSetItems"

func lsEqSetItems(data lawSpecData) []LawSpecValue {
	if len(data.fields) != 1 {
		panic("Eq-only set requires one List field")
	}
	items, ok := data.fields[0].Data.([]LawSpecValue)
	if !ok {
		panic("Eq-only set requires List storage")
	}
	return items
}

func lsSameMembers(left, right []LawSpecValue, equal func(LawSpecValue, LawSpecValue) bool) bool {
	included := func(values, candidates []LawSpecValue) bool {
		for _, value := range values {
			found := false
			for _, candidate := range candidates {
				if equal(value, candidate) {
					found = true
					break
				}
			}
			if !found {
				return false
			}
		}
		return true
	}
	return included(left, right) && included(right, left)
}

func lsDistinctMembers(items []LawSpecValue, equal func(LawSpecValue, LawSpecValue) bool) bool {
	for index, value := range items {
		for _, earlier := range items[:index] {
			if equal(value, earlier) {
				return false
			}
		}
	}
	return true
}

// LawSpecTask is an async adapter's result: a goroutine's value, awaited where
// it is used. A panic in the goroutine is raised again by Await.
type LawSpecTask[T any] struct{ state *lawSpecTaskState[T] }

type lawSpecTaskState[T any] struct {
	done    chan struct{}
	value   T
	failure any
}

// LawSpecGo starts work in a goroutine.
func LawSpecGo[T any](work func() T) LawSpecTask[T] {
	state := &lawSpecTaskState[T]{done: make(chan struct{})}
	go func() {
		defer close(state.done)
		defer func() { state.failure = recover() }()
		state.value = work()
	}()
	return LawSpecTask[T]{state}
}

// Await blocks until the task is done and returns its value.
func (t LawSpecTask[T]) Await() T {
	<-t.state.done
	if t.state.failure != nil {
		panic(t.state.failure)
	}
	return t.state.value
}

// LawSpecTransport carries a session's values between its two ends: side 0
// is the first end, side 1 the second. Send queues a value from one side;
// Receive waits for the next value the other side sent, and panics with
// LawSpecPeerFailed once the other side has given up and nothing is left;
// Abandon gives a side up. LawSpecNewChannel is the in-process transport; a
// networked one implements the same three methods.
type LawSpecTransport interface {
	Send(side int, value any)
	Receive(side int) any
	Abandon(side int)
}

// LawSpecPeerFailed is the failure of a receive whose other end gave up: its
// process failed, or it called Abandon. Receive panics with it, failing this
// process too; TryReceive returns it instead (or else).
var LawSpecPeerFailed = errors.New("the other end gave up the conversation (its process failed or abandoned it)")

// lawSpecAbandoned follows the last value an abandoned side sent.
type lawSpecAbandoned struct{}

// lawSpecSessionChannel is a buffered Go channel per direction: toward[side] holds
// the values sent to that side.
type lawSpecSessionChannel struct{ toward [2]chan any }

// LawSpecNewChannel makes an in-process transport whose sends do not block
// until buffer values wait in one direction.
func LawSpecNewChannel(buffer int) LawSpecTransport {
	return &lawSpecSessionChannel{[2]chan any{make(chan any, buffer+1), make(chan any, buffer+1)}}
}

func (c *lawSpecSessionChannel) Send(side int, value any) { c.toward[1-side] <- value }

func (c *lawSpecSessionChannel) Receive(side int) any {
	value := <-c.toward[side]
	if _, gone := value.(lawSpecAbandoned); gone {
		c.toward[side] <- value
		panic(LawSpecPeerFailed)
	}
	return value
}

func (c *lawSpecSessionChannel) Abandon(side int) {
	select {
	case c.toward[1-side] <- lawSpecAbandoned{}:
	default:
		go func() { c.toward[1-side] <- lawSpecAbandoned{} }()
	}
}

// LawSpecEnd is one end of a session at one step. Generated session types
// wrap it; each is used once, and its Send or Receive returns the end for the
// next step.
//
// Using an end exactly once is what makes a session follow its protocol; with
// channels joined in a tree, scenarios are deadlock-free by construction.
// ref:DEC-sessions-by-construction ref:caires-pfenning-session-types
// ref:wadler-propositions-as-sessions
type LawSpecEnd struct {
	transport LawSpecTransport
	side      int
	used      atomic.Bool
}

// LawSpecOpenEnds returns the first and second ends of a session over a
// transport.
func LawSpecOpenEnds(transport LawSpecTransport) (*LawSpecEnd, *LawSpecEnd) {
	return &LawSpecEnd{transport: transport, side: 0}, &LawSpecEnd{transport: transport, side: 1}
}

// use spends the end and returns the end for its next step.
func (e *LawSpecEnd) use() *LawSpecEnd {
	if e == nil {
		panic("lawspec session: this end was never opened; ends come from the protocol's Open function")
	}
	if !e.used.CompareAndSwap(false, true) {
		panic("lawspec session: this end was already used; use the end its last step returned")
	}
	return &LawSpecEnd{transport: e.transport, side: e.side}
}

// Send sends a value and returns the end for the next step.
func (e *LawSpecEnd) Send(value any) *LawSpecEnd {
	next := e.use()
	e.transport.Send(e.side, value)
	return next
}

// Receive waits for the other end's next value and returns it with the end
// for the next step.
func (e *LawSpecEnd) Receive() (any, *LawSpecEnd) {
	next := e.use()
	return e.transport.Receive(e.side), next
}

// TryReceive is Receive, but returns LawSpecPeerFailed instead of panicking
// when the other end gave up.
func (e *LawSpecEnd) TryReceive() (value any, next *LawSpecEnd, err error) {
	next = e.use()
	defer func() {
		if r := recover(); r != nil {
			if r == LawSpecPeerFailed {
				err = LawSpecPeerFailed
				return
			}
			panic(r)
		}
	}()
	return e.transport.Receive(e.side), next, nil
}

// Abandon gives up the conversation: the other end's receives fail with
// LawSpecPeerFailed once it has received what was already sent.
func (e *LawSpecEnd) Abandon() {
	e.use()
	e.transport.Abandon(e.side)
}

// HandOver spends the end so that it can be sent to another process: the
// returned end is the receiver's, at the same step.
func (e *LawSpecEnd) HandOver() *LawSpecEnd { return e.use() }

// LawSpecProcess is a goroutine started by LawSpecSpawn.
type LawSpecProcess struct {
	done    chan struct{}
	failure any
}

// LawSpecSpawn runs work in a goroutine.
func LawSpecSpawn(work func()) *LawSpecProcess {
	process := &LawSpecProcess{done: make(chan struct{})}
	go func() {
		defer close(process.done)
		defer func() { process.failure = recover() }()
		work()
	}()
	return process
}

// Join waits for the process to finish and raises its panic again, if any.
func (p *LawSpecProcess) Join() {
	<-p.done
	if p.failure != nil {
		panic(p.failure)
	}
}

// LawSpecPar runs each function in its own goroutine, waits for all of them
// and then raises the first one's panic (in argument order), if any.
func LawSpecPar(work ...func()) {
	processes := make([]*LawSpecProcess, len(work))
	for i, w := range work {
		processes[i] = LawSpecSpawn(w)
	}
	for _, p := range processes {
		<-p.done
	}
	for _, p := range processes {
		p.Join()
	}
}

const lsOrdering = "lawspec.collections::type::Ordering"

// lsCompareValues is the portable total order: -1, 0 or 1. Exact numbers by
// value, sequences by unit, false before true, absence before presence,
// lists element by element, Nothing before Just, and other data by
// constructor identity, then fields left to right.
func lsCompareValues(a, b LawSpecValue) int {
	switch x := a.Data.(type) {
	case bool:
		y := b.Data.(bool)
		if x == y {
			return 0
		} else if x {
			return 1
		}
		return -1
	case *big.Int, *big.Rat, lawSpecDecimal:
		return lsRatio(a).Cmp(lsRatio(b))
	case int:
		return lsSign(x - b.Data.(int))
	case []int:
		y := b.Data.([]int)
		for i := 0; i < len(x) && i < len(y); i++ {
			if x[i] != y[i] {
				return lsSign(x[i] - y[i])
			}
		}
		return lsSign(len(x) - len(y))
	case []LawSpecValue:
		y := b.Data.([]LawSpecValue)
		for i := 0; i < len(x) && i < len(y); i++ {
			if order := lsCompareValues(x[i], y[i]); order != 0 {
				return order
			}
		}
		return lsSign(len(x) - len(y))
	case lawSpecPresence:
		y := b.Data.(lawSpecPresence)
		switch {
		case x.value == nil && y.value == nil:
			return 0
		case x.value == nil:
			return -1
		case y.value == nil:
			return 1
		}
		return lsCompareValues(*x.value, *y.value)
	case lawSpecData:
		y := b.Data.(lawSpecData)
		if x.tag != y.tag {
			if x.tag == "Maybe::Nothing" && y.tag == "Maybe::Just" {
				return -1
			}
			if x.tag == "Maybe::Just" && y.tag == "Maybe::Nothing" {
				return 1
			}
			return strings.Compare(x.tag, y.tag)
		}
		return lsCompareValues(LawSpecValue{"", x.fields}, LawSpecValue{"", y.fields})
	case lawSpecHandle:
		if y, ok := b.Data.(lawSpecHandle); ok && lsSameNative(x.native, y.native) {
			return 0
		}
		panic("handles have no portable order: " + a.Type)
	case nil:
		return 0
	}
	panic("values have no portable order: " + a.Type)
}

func lsSign(n int) int {
	switch {
	case n < 0:
		return -1
	case n > 0:
		return 1
	}
	return 0
}

func lsHelper(n string, args []LawSpecValue, bits int) LawSpecValue {
	if n == "incompleteSearch" {
		panic(lsCodePointsText(args[0].Data.([]int)))
	}
	if result, ok := lsSequenceHelper(n, args); ok {
		return result
	}
	switch n {
	case "startsWith", "endsWith", "textContains":
		text, part := lsCodePointsText(args[0].Data.([]int)), lsCodePointsText(args[1].Data.([]int))
		switch n {
		case "startsWith":
			return lsBool(strings.HasPrefix(text, part))
		case "endsWith":
			return lsBool(strings.HasSuffix(text, part))
		}
		return lsBool(strings.Contains(text, part))
	case "regexMatches":
		return lsBool(LsRegexMatches(lsCodePointsText(args[0].Data.([]int)), args[1].Data.([]int)))
	case "recorded":
		return lsBool(lsRecorded(lsCodePointsText(args[0].Data.([]int)), args[1]))
	case "acquireResource":
		return LawSpecValue{"Text", lsTextUnits(lsAcquireResource(lsCodePointsText(args[0].Data.([]int))))}
	case "releaseResource":
		lsReleaseResource(lsCodePointsText(args[0].Data.([]int)), lsCodePointsText(args[1].Data.([]int)))
		return lsBool(true)
	case "freePort":
		return lsInteger("Int32", strconv.Itoa(lsFreePort()))
	case "checked":
		return lsBool(true)
	case "select":
		if lsTruth(args[0]) {
			return args[1]
		}
		return args[2]
	case "compare":
		tag := [...]string{"Less", "Equal", "Greater"}[lsCompareValues(args[0], args[1])+1]
		return LawSpecValue{lsOrdering, lawSpecData{lsOrdering + "::" + tag, nil}}
	case "length":
		if values, ok := args[0].Data.([]LawSpecValue); ok {
			return lsInteger("Integer", strconv.Itoa(len(values)))
		}
		return lsInteger("Integer", strconv.Itoa(len(args[0].Data.([]int))))
	case "isPresent":
		return lsBool(args[0].Data.(lawSpecPresence).value != nil)
	case "presentValue":
		p := args[0].Data.(lawSpecPresence).value
		if p == nil {
			panic("absent presence value")
		}
		return *p
	}

	x := args[0]
	switch n {
	case "real", "imag":
		z := x.Data.(complex128)
		f := real(z)
		if n == "imag" {
			f = imag(z)
		}
		t := "Float64"
		if x.Type == "Complex64" {
			t = "Float32"
		}
		return LawSpecValue{t, f}
	case "quot", "rem", "pow":
		return lsBinary(n, x, args[1])
	case "negate":
		if lsExactType(x.Type) {
			return lsBinary("-", lsInteger("BigInt", "0"), x)
		}
		if z, ok := x.Data.(complex128); ok {
			return LawSpecValue{x.Type, -z}
		}
		return LawSpecValue{x.Type, -lsReal(x)}
	case "isNaN":
		return lsBool(math.IsNaN(lsReal(x)))
	case "isInfinite":
		return lsBool(math.IsInf(lsReal(x), 0))
	case "isFinite":
		return lsBool(!math.IsInf(lsReal(x), 0) && !math.IsNaN(lsReal(x)))
	case "isNegativeZero":
		return lsBool(lsReal(x) == 0 && math.Signbit(lsReal(x)))
	case "round":
		scale := int(lsConvert("Int32", args[1], bits).Data.(*big.Int).Int64())
		r := lsRatio(x)
		factor := new(big.Rat)
		if scale >= 0 {
			factor.SetInt(lsPow(scale))
		} else {
			factor.SetFrac(big.NewInt(1), lsPow(-scale))
		}
		r.Mul(r, factor)
		q, rem := new(big.Int), new(big.Int)
		q.QuoRem(r.Num(), r.Denom(), rem)
		cmp := new(big.Int).Lsh(new(big.Int).Abs(rem), 1).Cmp(r.Denom())
		if cmp > 0 || (cmp == 0 && q.Bit(0) == 1) {
			q.Add(q, big.NewInt(int64(r.Sign())))
		}
		return lsConvert("Decimal", LawSpecValue{"Rational", new(big.Rat).Quo(new(big.Rat).SetInt(q), factor)}, bits)
	}
	return lsConvert(n, x, bits)
}
func (v LawSpecValue) String() string { return fmt.Sprintf("%s(%v)", v.Type, v.Data) }

func lsSample(t string, seed int, bits int) LawSpecValue {
	r := rand.New(rand.NewSource(int64(seed)))
	if strings.HasPrefix(t, "Nullable ") || strings.HasPrefix(t, "Optional ") {
		parts := strings.SplitN(t, " ", 2)
		if r.Intn(2) == 0 {
			return LawSpecValue{t, lawSpecPresence{nil}}
		}
		x := lsSample(parts[1], int(r.Int31()), bits)
		return LawSpecValue{t, lawSpecPresence{&x}}
	}
	if lsIntegerType(t) {
		w := 256
		if t != "BigInt" && t != "Integer" && t != "BigUInt" {
			w = bits
			if t != "IntSize" && t != "UIntSize" && t != "UIntPtr" {
				i := 3
				if strings.HasPrefix(t, "U") {
					i = 4
				}
				w, _ = strconv.Atoi(t[i:])
			}
		}
		limit := new(big.Int).Lsh(big.NewInt(1), uint(w))
		n := new(big.Int).Rand(r, limit)
		if (strings.HasPrefix(t, "Int") || t == "BigInt") && n.Bit(w-1) == 1 {
			n.Sub(n, limit)
		}
		return LawSpecValue{t, n}
	}
	switch t {
	case "Bool":
		return lsBool(r.Intn(2) == 0)
	case "Decimal":
		return LawSpecValue{t, lawSpecDecimal{lsSample("BigInt", int(r.Int31()), bits).Data.(*big.Int), r.Intn(41) - 20}}
	case "Rational":
		return LawSpecValue{t, new(big.Rat).SetFrac(lsSample("BigInt", int(r.Int31()), bits).Data.(*big.Int), new(big.Int).Add(lsSample("BigUInt", int(r.Int31()), bits).Data.(*big.Int), big.NewInt(1)))}
	case "Float32":
		return LawSpecValue{t, float64(math.Float32frombits(r.Uint32()))}
	case "Float64":
		return LawSpecValue{t, math.Float64frombits(r.Uint64())}
	case "Complex64", "Complex128":
		component := "Float64"
		if t == "Complex64" {
			component = "Float32"
		}
		return lsComplex(t, lsSample(component, int(r.Int31()), bits), lsSample(component, int(r.Int31()), bits))
	case "Symbol":
		return LawSpecValue{t, &lawSpecSymbol{description: "same"}}
	case "Unit", "Null", "Undefined":
		return lsAbsent(t)
	}
	max := 1114112
	if t == "Bytes" {
		max = 256
	}
	if t == "CodeUnit16" || t == "Utf16Text" {
		max = 65536
	}
	unit := func() int {
		for {
			c := r.Intn(max)
			if lsValidUnit(t, c) {
				return c
			}
		}
	}
	if t == "Char" || t == "CodePoint" || t == "CodeUnit16" {
		return lsCharacter(t, unit())
	}
	xs := make([]int, r.Intn(40))
	for i := range xs {
		xs[i] = unit()
	}
	return lsSequence(t, xs)
}

// LawSpecBigInt is an unbounded integer, since LawSpec's integer arithmetic is
// exact and never wraps. ref:DEC-portable-exact-arithmetic
type LawSpecBigInt = big.Int

// LawSpecRational is the exact result of dividing exact numbers.
// ref:DEC-portable-exact-arithmetic
type LawSpecRational = big.Rat

func lsToNative(t string, v LawSpecValue, bits int) any {
	v = lsConvert(t, v, bits)
	lsCheckNativeProfile(t, bits)
	if lsIntegerType(t) {
		n := v.Data.(*big.Int)
		switch t {
		case "Int8":
			return int8(n.Int64())
		case "Int16":
			return int16(n.Int64())
		case "Int32":
			return int32(n.Int64())
		case "Int64":
			return n.Int64()
		case "UInt8":
			return uint8(n.Uint64())
		case "UInt16":
			return uint16(n.Uint64())
		case "UInt32":
			return uint32(n.Uint64())
		case "UInt64":
			return n.Uint64()
		case "IntSize":
			return int(n.Int64())
		case "UIntSize":
			return uint(n.Uint64())
		case "UIntPtr":
			return uintptr(n.Uint64())
		default:
			return new(big.Int).Set(n)
		}
	}
	if t == "Char" || t == "CodePoint" {
		return rune(v.Data.(int))
	}
	if t == "CodeUnit16" {
		return uint16(v.Data.(int))
	}
	if t == "Bytes" {
		xs := v.Data.([]int)
		out := make([]byte, len(xs))
		for i, c := range xs {
			out[i] = byte(c)
		}
		return out
	}
	if t == "Utf16Text" {
		xs := v.Data.([]int)
		out := make([]uint16, len(xs))
		for i, c := range xs {
			out[i] = uint16(c)
		}
		return out
	}
	if t == "CodePointText" {
		xs := v.Data.([]int)
		out := make([]rune, len(xs))
		for i, c := range xs {
			out[i] = rune(c)
		}
		return out
	}
	if t == "Text" {
		u := v.Data.([]int)
		r := make([]rune, len(u))
		for i, c := range u {
			r[i] = rune(c)
		}
		return string(r)
	}
	if t == "Rational" {
		return lsRatio(v)
	}
	if t == "Float32" {
		return float32(v.Data.(float64))
	}
	if t == "Complex64" {
		return complex64(v.Data.(complex128))
	}
	return v.Data
}
func lsFromNative(t string, value any, bits int) LawSpecValue {
	if v, ok := value.(LawSpecValue); ok {
		return lsClone(lsValidate(t, v, bits))
	}
	if lsSumType(t) {
		lsCheckNativeProfile(t, bits)
		native, ok := value.(lawSpecSum)
		if !ok {
			panic("native sum requires a constructor")
		}
		tag := native.lawSpecTag()
		types, values := lsSumFields(t, tag), native.lawSpecFields()
		if len(types) != len(values) {
			panic("invalid native constructor arity")
		}
		fields := make([]LawSpecValue, len(types))
		for index, fieldType := range types {
			fields[index] = lsFromNative(fieldType, values[index], bits)
		}
		return lsConstruct(t, tag, fields)
	}
	if strings.HasPrefix(t, "List ") {
		lsCheckNativeProfile(t, bits)
		values := reflect.ValueOf(value)
		if !values.IsValid() || values.Kind() != reflect.Slice {
			panic("native List requires a slice")
		}
		converted := make([]LawSpecValue, values.Len())
		for index := range converted {
			converted[index] = lsFromNative(strings.TrimPrefix(t, "List "), values.Index(index).Interface(), bits)
		}
		return lsList(t, converted)
	}
	lsCheckNativeProfile(t, bits)
	var data any = value
	if lsIntegerType(t) {
		rv := reflect.ValueOf(value)
		switch rv.Kind() {
		case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
			data = big.NewInt(rv.Int())
		case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
			data = new(big.Int).SetUint64(rv.Uint())
		default:
			switch n := value.(type) {
			case *big.Int:
				if n == nil {
					panic("invalid Integer representation")
				}
				data = new(big.Int).Set(n)
			case big.Int:
				data = new(big.Int).Set(&n)
			default:
				panic("invalid Integer representation")
			}
		}
	}
	if t == "Char" || t == "CodePoint" {
		data = int(value.(rune))
	}
	if t == "CodeUnit16" {
		data = int(value.(uint16))
	}
	if t == "Bytes" {
		xs := value.([]byte)
		out := make([]int, len(xs))
		for i, c := range xs {
			out[i] = int(c)
		}
		data = out
	}
	if t == "Utf16Text" {
		xs := value.([]uint16)
		out := make([]int, len(xs))
		for i, c := range xs {
			out[i] = int(c)
		}
		data = out
	}
	if t == "CodePointText" {
		xs := value.([]rune)
		out := make([]int, len(xs))
		for i, c := range xs {
			out[i] = int(c)
		}
		data = out
	}
	if t == "Text" {
		text := value.(string)
		if !utf8.ValidString(text) {
			panic("Text cannot contain invalid UTF-8 bytes")
		}
		u := []int{}
		for _, c := range text {
			u = append(u, int(c))
		}
		data = u
	}
	if t == "Float32" {
		data = float64(value.(float32))
	}
	if t == "Complex64" {
		data = complex128(value.(complex64))
	}
	return lsClone(lsValidate(t, LawSpecValue{t, data}, bits))
}

func lsClone(v LawSpecValue) LawSpecValue {
	switch x := v.Data.(type) {
	case lawSpecData:
		fields := make([]LawSpecValue, len(x.fields))
		for index, field := range x.fields {
			fields[index] = lsClone(field)
		}
		v.Data = lawSpecData{x.tag, fields}
	case []LawSpecValue:
		values := make([]LawSpecValue, len(x))
		for index, value := range x {
			values[index] = lsClone(value)
		}
		v.Data = values
	case *big.Int:
		v.Data = new(big.Int).Set(x)
	case *big.Rat:
		v.Data = new(big.Rat).Set(x)
	case []int:
		v.Data = append([]int{}, x...)
	case lawSpecDecimal:
		v.Data = lawSpecDecimal{new(big.Int).Set(x.coefficient), x.exponent}
	case lawSpecPresence:
		if x.value != nil {
			c := lsClone(*x.value)
			v.Data = lawSpecPresence{&c}
		}
	}
	return v
}

type lawSpecBound struct {
	op    string
	value LawSpecValue
}
type lawSpecDomain struct {
	candidates func([]LawSpecValue, int) []LawSpecValue
	accept     func([]LawSpecValue) bool
}

func lsFloor(r *big.Rat) *big.Int {
	q, rem := new(big.Int).QuoRem(r.Num(), r.Denom(), new(big.Int))
	if rem.Sign() < 0 {
		q.Sub(q, big.NewInt(1))
	}
	return q
}
func lsCeil(r *big.Rat) *big.Int { return new(big.Int).Neg(lsFloor(new(big.Rat).Neg(r))) }
func lsDomainCandidates(t string, seed, bits int, restrictions []lawSpecBound, hints []LawSpecValue) []LawSpecValue {
	values := []LawSpecValue{}
	for _, hint := range hints {
		func() { defer func() { recover() }(); values = append(values, lsConvert(t, hint, bits)) }()
	}
	if lsIntegerType(t) {
		var lo, hi *big.Int
		if t == "BigUInt" {
			lo = big.NewInt(0)
		} else if t != "Integer" && t != "BigInt" {
			w := bits
			if t != "IntSize" && t != "UIntSize" && t != "UIntPtr" {
				w, _ = strconv.Atoi(strings.TrimLeft(t, "UInt"))
			}
			if strings.HasPrefix(t, "Int") {
				hi = new(big.Int).Lsh(big.NewInt(1), uint(w-1))
				lo = new(big.Int).Neg(new(big.Int).Set(hi))
				hi.Sub(hi, big.NewInt(1))
			} else {
				lo = big.NewInt(0)
				hi = new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), uint(w)), big.NewInt(1))
			}
		}
		for _, b := range restrictions {
			r := lsRatio(b.value)
			if b.op == ">" || b.op == ">=" || b.op == "==" {
				v := lsCeil(r)
				if b.op == ">" {
					v.Add(lsFloor(r), big.NewInt(1))
				}
				if lo == nil || v.Cmp(lo) > 0 {
					lo = v
				}
			}
			if b.op == "<" || b.op == "<=" || b.op == "==" {
				v := lsFloor(r)
				if b.op == "<" {
					v.Sub(lsCeil(r), big.NewInt(1))
				}
				if hi == nil || v.Cmp(hi) < 0 {
					hi = v
				}
			}
		}
		if lo != nil && hi != nil && lo.Cmp(hi) > 0 {
			return nil
		}
		lower, upper := lo, hi
		if lower == nil {
			lower = new(big.Int).Neg(new(big.Int).Lsh(big.NewInt(1), 256))
			if hi != nil && hi.Sign() < 0 {
				lower.Add(lower, hi)
			}
		}
		if upper == nil {
			upper = new(big.Int).Lsh(big.NewInt(1), 256)
			if lo != nil && lo.Sign() > 0 {
				upper.Add(upper, lo)
			}
		}
		ns := []*big.Int{lower, upper, big.NewInt(0), big.NewInt(1), big.NewInt(-1), new(big.Int).Add(lower, big.NewInt(1)), new(big.Int).Sub(upper, big.NewInt(1))}
		random := rand.New(rand.NewSource(int64(seed)))
		width := new(big.Int).Add(new(big.Int).Sub(upper, lower), big.NewInt(1))
		for j := 0; j < 8; j++ {
			ns = append(ns, new(big.Int).Add(lower, new(big.Int).Rand(random, width)))
		}
		for _, n := range ns {
			values = append(values, LawSpecValue{t, n})
		}
		filtered := []LawSpecValue{}
		for _, v := range values {
			if lsIntegerType(v.Type) {
				n := v.Data.(*big.Int)
				if n.Cmp(lower) >= 0 && n.Cmp(upper) <= 0 {
					filtered = append(filtered, lsConvert(t, v, bits))
				}
			}
		}
		values = filtered
	} else {
		for j := 0; j < 8; j++ {
			values = append(values, lsSample(t, seed+j*7919, bits))
		}
	}
	if len(values) > 0 {
		offset := ((seed % len(values)) + len(values)) % len(values)
		values = append(append([]LawSpecValue{}, values[offset:]...), values[:offset]...)
	}
	return values
}
func lsGenerateTuple(domains []lawSpecDomain, seed, attempts int, prefix []LawSpecValue) ([]LawSpecValue, bool) {
	used := 0
	lastPrefix := prefix
	var search func([]LawSpecValue) ([]LawSpecValue, bool)
	search = func(values []LawSpecValue) ([]LawSpecValue, bool) {
		lastPrefix = values
		if len(values) == len(domains) {
			return values, true
		}
		if used >= attempts {
			return nil, false
		}
		used++
		d := domains[len(values)]
		for _, v := range d.candidates(values, seed+used*7919) {
			if used >= attempts {
				break
			}
			used++
			next := append(append([]LawSpecValue{}, values...), v)
			if d.accept(next) {
				if result, ok := search(next); ok {
					return result, true
				}
			}
		}
		return nil, false
	}
	for used < attempts {
		if result, ok := search(append([]LawSpecValue{}, prefix...)); ok {
			return result, true
		}
	}
	return lastPrefix, false
}
func lsRequireContract(condition bool, context string) {
	if !condition {
		panic(context)
	}
}
func lsContract(context string, condition bool, result LawSpecValue) LawSpecValue {
	lsRequireContract(condition, context)
	return result
}
func lsCapture(check func([]LawSpecValue), values []LawSpecValue) (failure any) {
	defer func() { failure = recover() }()
	check(values)
	return nil
}
func lsRefinedCase(domains []lawSpecDomain, seed, attempts, shrinks int, check func([]LawSpecValue), context string) {
	values, ok := lsGenerateTuple(domains, seed, attempts, nil)
	if !ok {
		panic(fmt.Sprintf("%s: refinement-generation-exhausted after %d attempts; prefix=%v; seed=%d", context, attempts, values, seed))
	}
	if original := lsCapture(check, values); original != nil {
		best, budget := values, shrinks
		for i := range best {
			value := best[i]
			candidates := domains[i].candidates(best[:i], 0)
			if lsIntegerType(value.Type) {
				initial := value.Data.(*big.Int)
				candidates = append([]LawSpecValue{{value.Type, big.NewInt(0)}, {value.Type, big.NewInt(int64(initial.Sign()))}}, candidates...)
				for n := new(big.Int).Quo(initial, big.NewInt(2)); new(big.Int).Abs(n).Cmp(big.NewInt(1)) > 0; n = new(big.Int).Quo(n, big.NewInt(2)) {
					candidates = append(candidates, LawSpecValue{value.Type, n})
				}
			}
			for _, candidate := range candidates {
				if budget <= 0 {
					break
				}
				budget--
				if lsComplexity(candidate).Cmp(lsComplexity(best[i])) >= 0 {
					continue
				}
				prefix := append(append([]LawSpecValue{}, best[:i]...), candidate)
				if !domains[i].accept(prefix) {
					continue
				}
				trial, ok := lsGenerateTuple(domains, seed, min(attempts, 100), prefix)
				if ok && lsCapture(check, trial) != nil {
					best = trial
				}
			}
		}
		panic(fmt.Sprintf("%s: %v; refined counterexample=%v; seed=%d", context, original, best, seed))
	}
}
func lsAssert(context string, actual, expected func() LawSpecValue) {
	defer func() {
		if err := recover(); err != nil {
			panic(fmt.Sprintf("%s: %v", context, err))
		}
	}()
	a, b := actual(), expected()
	if !lsEqual(a, b) {
		panic(fmt.Sprintf("%s | actual=%v expected=%v%s", context, a, b, lsDifference(a, b)))
	}
}

func lsComplexity(v LawSpecValue) *big.Int {
	switch x := v.Data.(type) {
	case *big.Int:
		return new(big.Int).Abs(x)
	case []int:
		return big.NewInt(int64(len(x)))
	case lawSpecPresence:
		if x.value == nil {
			return big.NewInt(0)
		}
		return new(big.Int).Add(big.NewInt(1), lsComplexity(*x.value))
	case bool:
		if x {
			return big.NewInt(1)
		}
		return big.NewInt(0)
	case float64:
		return new(big.Int).SetUint64(math.Float64bits(math.Abs(x)))
	case complex128:
		return new(big.Int).Add(lsComplexity(LawSpecValue{"Float64", real(x)}), lsComplexity(LawSpecValue{"Float64", imag(x)}))
	case int:
		return big.NewInt(int64(x))
	}
	if lsExactType(v.Type) {
		r := lsRatio(v)
		return new(big.Int).Sub(new(big.Int).Add(new(big.Int).Abs(r.Num()), r.Denom()), big.NewInt(1))
	}
	if v.Data == nil {
		return big.NewInt(0)
	}
	return big.NewInt(1)
}

// Workflow runtime. A workflow runs under a runtime: a clock, a seeded
// random source, a trace of what happened, and the state of stateful stages.
// The runtime travels in the symbols map every generated function takes;
// without one, the default runtime applies (real time, unless the tests
// installed a virtual clock). Durations are int64 microseconds.

const lsWorkflowKey = "\x00lawspec.workflow"

// LawSpecClock tells the time and waits, in microseconds. A clock with a
// Virtual method that answers true is virtual: timeouts and hedges count
// only the time it reports (see lsScoped); any other clock is real time.
type LawSpecClock interface {
	Now() int64
	Sleep(micros int64)
}

// LawSpecRealClock is monotonic wall time.
type LawSpecRealClock struct{ start time.Time }

func (c *LawSpecRealClock) Now() int64         { return time.Since(c.start).Microseconds() }
func (c *LawSpecRealClock) Sleep(micros int64) { time.Sleep(time.Duration(micros) * time.Microsecond) }
func (c *LawSpecRealClock) Virtual() bool      { return false }

// LawSpecVirtualClock advances when slept on and returns at once.
type LawSpecVirtualClock struct{ Time int64 }

func (c *LawSpecVirtualClock) Now() int64         { return c.Time }
func (c *LawSpecVirtualClock) Sleep(micros int64) { c.Time += micros }
func (c *LawSpecVirtualClock) Virtual() bool      { return true }

// lsClockIsVirtual is whether a workflow clock is virtual.
func lsClockIsVirtual(clock LawSpecClock) bool {
	virtual, ok := clock.(interface{ Virtual() bool })
	return ok && virtual.Virtual()
}

// lsClockAbility is the Clock ability's key in a law's handlers.
const lsClockAbility = "lawspec.time::ability::Clock"

// lawSpecClockReader is how the runtime reads a Clock handler, which is the
// generated Clock interface's: lawspec.time's RegisterClock (in the package's
// copy of the default handlers, called by the generated tests) registers it.
type lawSpecClockReader struct {
	now      func(handler any) int64
	sleep    func(handler any, micros int64)
	realTime func(handler any) bool
}

var lsClockReader *lawSpecClockReader

// LawSpecRegisterClockAbility registers how the runtime reads a Clock
// handler: now gives microseconds, sleep waits, and realTime says whether
// the handler is the default real clock.
func LawSpecRegisterClockAbility(now func(handler any) int64, sleep func(handler any, micros int64), realTime func(handler any) bool) {
	lsClockLock.Lock()
	defer lsClockLock.Unlock()
	lsClockReader = &lawSpecClockReader{now: now, sleep: sleep, realTime: realTime}
}

// lsAbilityClockOf is a Clock handler read as a workflow clock, or nil when
// there is no handler or no reader is registered.
func lsAbilityClockOf(handler any) *lawSpecAbilityClock {
	lsClockLock.Lock()
	reader := lsClockReader
	lsClockLock.Unlock()
	if reader == nil || handler == nil {
		return nil
	}
	return &lawSpecAbilityClock{handler: handler, reader: reader, virtual: !reader.realTime(handler)}
}

// lawSpecAbilityClock is a workflow runtime's clock read through the Clock
// ability: the handler a law installs (the virtual clock, or the default
// real one). Every handler but the default real clock is virtual: waits pass
// at once, and timeouts and hedges count only the time it reports.
type lawSpecAbilityClock struct {
	handler any
	reader  *lawSpecClockReader
	virtual bool
}

func (c *lawSpecAbilityClock) Now() int64         { return c.reader.now(c.handler) }
func (c *lawSpecAbilityClock) Sleep(micros int64) { c.reader.sleep(c.handler, micros) }
func (c *lawSpecAbilityClock) Virtual() bool      { return c.virtual }

// lawSpecClockView is the default runtime seen through a Clock handler,
// kept on the handlers entry so a stage and its steps share one view (and
// with it the running attempt's deadline and hedge).
type lawSpecClockView struct {
	handler any
	base    *LawSpecWorkflowRuntime
	view    *LawSpecWorkflowRuntime
}

// lsClockLock guards the clock reader and the views.
var lsClockLock sync.Mutex

// LawSpecSplitMix64 gives the same sequence on every target for a seed.
//
// SplitMix64 is small, fast and specified exactly, so every runtime implements
// the same generator and a seed names the same case everywhere. ref:splitmix
// ref:DEC-portable-seeded-generation
type LawSpecSplitMix64 struct{ state uint64 }

// Next advances the generator by one step of SplitMix64, bit for bit as every
// other target does. ref:splitmix
func (r *LawSpecSplitMix64) Next() uint64 {
	r.state += 0x9E3779B97F4A7C15
	z := r.state
	z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9
	z = (z ^ (z >> 27)) * 0x94D049BB133111EB
	return z ^ (z >> 31)
}

// Below is uniform in [0, bound); 0 when bound is 0.
func (r *LawSpecSplitMix64) Below(bound uint64) uint64 {
	if bound == 0 {
		return 0
	}
	return r.Next() % bound
}

// LawSpecTraceEvent records a stage starting or finishing an attempt, or a wait.
type LawSpecTraceEvent struct {
	Kind, Stage string
	Number      int64
	Succeeded   bool
}

// LawSpecWorkflowRuntime is what a workflow runs under. Gates says whether
// rate limits, breakers, bulkheads and caches apply; the runtime generated
// tests install has them off, since a workflow law calls the workflow and
// its composition, which would see each other's state.
type LawSpecWorkflowRuntime struct {
	Clock  LawSpecClock
	Random LawSpecSplitMix64
	Trace  []LawSpecTraceEvent
	State  map[string]any
	Gates  bool
	// A frame per running workflow: the undos of its completed stages.
	frames [][]lawSpecUndo
	// When the running attempt of a stage with a timeout must end, or zero.
	deadline time.Time
	// The running attempt's hedge, or nil.
	hedge *lawSpecHedge
}

// lawSpecHedge: when an attempt has not succeeded after Delay microseconds,
// another starts beside it, up to Most in all; the first success wins. On a
// virtual clock (virtual set) the attempts run one after another instead.
type lawSpecHedge struct {
	Stage       string
	Delay, Most int64
	virtual     bool
}

// lawSpecTimedOut is raised by lsAwaitStep when an attempt outlives its
// stage's timeout, and recovered by the stage.
type lawSpecTimedOut struct{}

// lsAwaitStep is an asynchronous step's logical result: start begins the
// step and convert turns its native result into a logical value. The step
// runs within its stage's timeout and hedge, if any.
func lsAwaitStep[T any](symbols map[string]*lawSpecSymbol, start func() LawSpecTask[T], convert func(T) LawSpecValue) LawSpecValue {
	runtime := lsWorkflowRuntime(symbols)
	deadline, hedge := runtime.deadline, runtime.hedge
	if deadline.IsZero() && hedge == nil {
		return convert(start().Await())
	}
	if hedge != nil && hedge.virtual {
		return lsVirtualHedge(runtime, hedge, start, convert)
	}
	var expiry <-chan time.Time
	if !deadline.IsZero() {
		timer := time.NewTimer(time.Until(deadline))
		defer timer.Stop()
		expiry = timer.C
	}
	most, delay := int64(1), time.Duration(0)
	if hedge != nil {
		most, delay = hedge.Most, time.Duration(hedge.Delay)*time.Microsecond
	}
	results := make(chan *lawSpecTaskState[T], most)
	started, pending := int64(0), 0
	var next <-chan time.Time
	var nextTimer *time.Timer
	defer func() {
		if nextTimer != nil {
			nextTimer.Stop()
		}
	}()
	launch := func() {
		started++
		pending++
		if started > 1 {
			runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"hedge", hedge.Stage, started, true})
		}
		task := start()
		go func() {
			<-task.state.done
			results <- task.state
		}()
		if nextTimer != nil {
			nextTimer.Stop()
		}
		next = nil
		if started < most {
			nextTimer = time.NewTimer(delay)
			next = nextTimer.C
		}
	}
	launch()
	for {
		select {
		case state := <-results:
			pending--
			if state.failure != nil {
				panic(state.failure)
			}
			value := convert(state.value)
			if data, ok := value.Data.(lawSpecData); !ok || data.tag != "Either::Left" || (pending == 0 && started >= most) {
				return value
			}
			if pending == 0 {
				launch()
			}
		case <-next:
			launch()
		case <-expiry:
			panic(lawSpecTimedOut{})
		}
	}
}

// lsVirtualHedge is a hedge on a virtual clock: attempts run one after
// another, and the next starts when one fails, so the first success wins as
// it would in real time when no attempt outlives the delay.
func lsVirtualHedge[T any](runtime *LawSpecWorkflowRuntime, hedge *lawSpecHedge, start func() LawSpecTask[T], convert func(T) LawSpecValue) LawSpecValue {
	started := int64(1)
	value := convert(start().Await())
	for lsIsLeft(value) && started < hedge.Most {
		started++
		runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"hedge", hedge.Stage, started, true})
		value = convert(start().Await())
	}
	return value
}

func lsIsLeft(value LawSpecValue) bool {
	data, ok := value.Data.(lawSpecData)
	return ok && data.tag == "Either::Left"
}

// lsTimedOut is a stage's TimedOut failure, of its own type when it has one.
func lsTimedOut(policy lawSpecStagePolicy) LawSpecValue {
	if policy.Fail != nil {
		return policy.Fail("TimedOut")
	}
	return lsStageFailureValue("TimedOut")
}

// lsScoped runs an attempt under its stage's timeout (failing with TimedOut
// when it outlives it) and hedge: the Timeout and Hedge transformers of the
// Async ability, measured on the runtime's Clock. On a virtual clock
// (generated tests, or a law using virtual clock) an attempt takes the
// virtual time that passes while it runs, so both are deterministic. On a
// real clock with gates off, both are off.
func lsScoped(runtime *LawSpecWorkflowRuntime, policy lawSpecStagePolicy, attempt func() LawSpecValue) (result LawSpecValue) {
	if policy.Timeout <= 0 && policy.Hedge == nil {
		return attempt()
	}
	if lsClockIsVirtual(runtime.Clock) {
		return lsVirtuallyScoped(runtime, policy, attempt)
	}
	if !runtime.Gates {
		return attempt()
	}
	outerDeadline, outerHedge := runtime.deadline, runtime.hedge
	if policy.Timeout > 0 {
		runtime.deadline = time.Now().Add(time.Duration(policy.Timeout) * time.Microsecond)
	}
	if policy.Hedge != nil {
		hedge := *policy.Hedge
		hedge.Stage = policy.Stage
		runtime.hedge = &hedge
	}
	defer func() {
		runtime.deadline, runtime.hedge = outerDeadline, outerHedge
		if failure := recover(); failure != nil {
			if _, ok := failure.(lawSpecTimedOut); !ok {
				panic(failure)
			}
			result = lsTimedOut(policy)
		}
	}()
	return attempt()
}

// lsVirtuallyScoped is lsScoped on a virtual clock: no real deadline; the
// attempt times out when the clock moved on by more than the timeout while
// it ran, and its hedge's attempts run one after another.
func lsVirtuallyScoped(runtime *LawSpecWorkflowRuntime, policy lawSpecStagePolicy, attempt func() LawSpecValue) (result LawSpecValue) {
	outerDeadline, outerHedge := runtime.deadline, runtime.hedge
	began := runtime.Clock.Now()
	runtime.deadline = time.Time{}
	runtime.hedge = nil
	if policy.Hedge != nil {
		hedge := *policy.Hedge
		hedge.Stage = policy.Stage
		hedge.virtual = true
		runtime.hedge = &hedge
	}
	timedOut := false
	func() {
		defer func() {
			runtime.deadline, runtime.hedge = outerDeadline, outerHedge
			if failure := recover(); failure != nil {
				if _, ok := failure.(lawSpecTimedOut); !ok {
					panic(failure)
				}
				timedOut = true
			}
		}()
		result = attempt()
	}()
	if timedOut || (policy.Timeout > 0 && runtime.Clock.Now()-began > policy.Timeout) {
		return lsTimedOut(policy)
	}
	return result
}

type lawSpecUndo struct {
	stage string
	undo  func()
}

// lsRunWorkflow runs a workflow whose stages compensate: when it fails, the
// undos of its completed stages run, last first.
func lsRunWorkflow(symbols map[string]*lawSpecSymbol, attempt func() LawSpecValue) LawSpecValue {
	runtime := lsWorkflowRuntime(symbols)
	runtime.frames = append(runtime.frames, nil)
	depth := len(runtime.frames)
	var frame []lawSpecUndo
	result := func() LawSpecValue {
		// The frame is taken back even when the workflow panics.
		defer func() {
			frame = runtime.frames[depth-1]
			runtime.frames = runtime.frames[:depth-1]
		}()
		return attempt()
	}()
	if data, ok := result.Data.(lawSpecData); ok && data.tag == "Either::Left" {
		for i := len(frame) - 1; i >= 0; i-- {
			runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"compensate", frame[i].stage, 0, true})
			frame[i].undo()
		}
	}
	return result
}

// NewLawSpecWorkflowRuntime makes a runtime; a nil clock is real time.
func NewLawSpecWorkflowRuntime(clock LawSpecClock, seed uint64) *LawSpecWorkflowRuntime {
	if clock == nil {
		clock = &LawSpecRealClock{time.Now()}
	}
	return &LawSpecWorkflowRuntime{Clock: clock, Random: LawSpecSplitMix64{seed}, State: map[string]any{}, Gates: true}
}

// Context returns a symbols map that runs workflows under this runtime.
func (r *LawSpecWorkflowRuntime) Context(symbols map[string]*LawSpecSymbol) map[string]*LawSpecSymbol {
	if symbols == nil {
		symbols = map[string]*LawSpecSymbol{}
	}
	symbols[lsWorkflowKey] = &lawSpecSymbol{workflow: r}
	return symbols
}

var lsDefaultWorkflowRuntime *LawSpecWorkflowRuntime

// LawSpecUseVirtualClock makes the default runtime virtual, as generated
// tests do. Timeouts and hedges stay on: they count virtual time (see
// lsScoped).
func LawSpecUseVirtualClock(seed uint64) {
	lsDefaultWorkflowRuntime = NewLawSpecWorkflowRuntime(&LawSpecVirtualClock{}, seed)
	lsDefaultWorkflowRuntime.Gates = false
}

// LawSpecNativeAsync is the Async ability's default handler: goroutines.
// LawSpec code performs Pause; workflows reach the rest natively: Spawn
// starts a function as a task, Wait gives a task's result (a panic in the
// task is raised again), and All runs functions side by side and gives their
// results in order. An async adapter's LawSpecTask is such a task.
// lawspec.concurrent's AsyncHandler embeds it.
type LawSpecNativeAsync struct{}

// Pause lets other goroutines run.
func (LawSpecNativeAsync) Pause() { goruntime.Gosched() }

// Spawn starts work on a goroutine of its own.
func (LawSpecNativeAsync) Spawn(work func() any) LawSpecTask[any] { return LawSpecGo(work) }

// Wait blocks until the task is done and gives its result.
func (LawSpecNativeAsync) Wait(task LawSpecTask[any]) any { return task.Await() }

// All is every function's result, in order; all finish before the first
// panic (in order) is raised again.
func (LawSpecNativeAsync) All(works ...func() any) []any {
	tasks := make([]LawSpecTask[any], len(works))
	for i, work := range works {
		tasks[i] = LawSpecGo(work)
	}
	for _, task := range tasks {
		<-task.state.done
	}
	results := make([]any, len(tasks))
	for i, task := range tasks {
		results[i] = task.Await()
	}
	return results
}

// LawSpecAsync is the runtime's native Async.
var LawSpecAsync = LawSpecNativeAsync{}

// lsConcurrently runs an all group's steps side by side as tasks of the
// Async ability's default handler, so an asynchronous step waits only for
// itself, and gives their results in declaration order. Every step finishes
// before a step's panic (the first, in declaration order) is raised again
// here.
func lsConcurrently(steps ...func() LawSpecValue) []LawSpecValue {
	// The default runtime exists before the steps look it up.
	lsWorkflowRuntime(nil)
	works := make([]func() any, len(steps))
	for i := range steps {
		step := steps[i]
		works[i] = func() any { return step() }
	}
	results := make([]LawSpecValue, len(steps))
	for i, result := range LawSpecAsync.All(works...) {
		results[i] = result.(LawSpecValue)
	}
	return results
}

// lsWorkflowRuntime is the runtime a workflow runs under: the one attached
// to symbols (which keeps its own clock), else the default one. Workflow
// time is the Clock ability's: where a law has installed a Clock handler,
// the default runtime waits and times out on it, through a view kept on
// the handlers entry.
func lsWorkflowRuntime(symbols map[string]*lawSpecSymbol) *LawSpecWorkflowRuntime {
	if entry, ok := symbols[lsWorkflowKey]; ok && entry.workflow != nil {
		return entry.workflow
	}
	if lsDefaultWorkflowRuntime == nil {
		lsDefaultWorkflowRuntime = NewLawSpecWorkflowRuntime(nil, 0)
	}
	runtime := lsDefaultWorkflowRuntime
	entry, ok := symbols[lsHandlersKey]
	if !ok || entry == nil {
		return runtime
	}
	clock := lsAbilityClockOf(entry.handlers[lsClockAbility])
	if clock == nil {
		return runtime
	}
	lsClockLock.Lock()
	defer lsClockLock.Unlock()
	if view := entry.clockView; view != nil && view.base == runtime && lsSameNative(view.handler, clock.handler) {
		return view.view
	}
	under := *runtime
	under.Clock = clock
	entry.clockView = &lawSpecClockView{handler: clock.handler, base: runtime, view: &under}
	return &under
}

// lawSpecRetry: Strategy is immediate, fixed, linear, exponential, fibonacci
// or custom; Delay, Step, Factor and Cap (negative for none) are its
// parameters; Decide stops (false) or waits, for custom strategies.
type lawSpecRetry struct {
	Strategy                 string
	Delay, Step, Factor, Cap int64
	Attempts                 int64
	Jitter                   string
	When                     func(LawSpecValue) bool
	Decide                   func(attempt int64, failure LawSpecValue, previous int64) (int64, bool)
}

// lawSpecGate is a stateful policy: Start gives its state, Admit a Step of
// the next state and a Gate (Admit, WaitFor or Reject), and Finish (when
// set) the state after the call. Wait is -2 to fail at once when not
// admitted, -1 to wait without bound, or the most it waits.
type lawSpecGate struct {
	Kind   string
	Start  func(now int64) LawSpecValue
	Admit  func(state LawSpecValue, now int64) LawSpecValue
	Finish func(state LawSpecValue, now int64, succeeded bool) LawSpecValue
	Wait   int64
}

// lawSpecStagePolicy: Key names the stage's state, Cache is how long a
// success is reused (-1 for no cache), and Wraps says failures are
// StageFailures.
type lawSpecStagePolicy struct {
	Stage   string
	Retry   *lawSpecRetry
	Timeout int64
	Key     string
	Gates   []lawSpecGate
	Cache   int64
	Wraps   bool
	// Fail gives the stage's result for a policy failure, of its own type.
	Fail func(kind string) LawSpecValue
	// Compensate undoes the stage's success value when its workflow fails.
	Compensate func(value LawSpecValue)
	Hedge      *lawSpecHedge
}

type lawSpecCacheEntry struct {
	key, value LawSpecValue
	expires    int64
}

const lsStageFailure = "lawspec.resilience::type::StageFailure::"
const lsGate = "lawspec.resilience::type::Gate::"

func lsStageFailureValue(kind string) LawSpecValue {
	return LawSpecValue{"Either", lawSpecData{"Either::Left", []LawSpecValue{
		{"lawspec.resilience::type::StageFailure", lawSpecData{lsStageFailure + kind, nil}}}}}
}

// lsPassGate admits the call, or gives the failure to return instead.
func lsPassGate(runtime *LawSpecWorkflowRuntime, policy lawSpecStagePolicy, gate lawSpecGate) string {
	key := policy.Key + "/" + gate.Kind
	waited := int64(0)
	failure := map[string]string{"breaker": "CircuitOpen", "limit": "RateLimited", "bulkhead": "Saturated"}[gate.Kind]
	for {
		now := runtime.Clock.Now()
		state, ok := runtime.State[key].(LawSpecValue)
		if !ok {
			state = gate.Start(now)
		}
		step := gate.Admit(state, now).Data.(lawSpecData)
		runtime.State[key] = step.fields[0]
		decision := step.fields[1].Data.(lawSpecData)
		if decision.tag == lsGate+"Admit" {
			return ""
		}
		if decision.tag == lsGate+"Reject" || gate.Wait == -2 {
			return failure
		}
		delay := decision.fields[0].Data.(*big.Int).Int64()
		if gate.Wait >= 0 && waited+delay > gate.Wait {
			return failure
		}
		runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"wait", policy.Stage, delay, true})
		runtime.Clock.Sleep(delay)
		waited += delay
	}
}

func lsFinishGate(runtime *LawSpecWorkflowRuntime, policy lawSpecStagePolicy, gate lawSpecGate, succeeded bool) {
	if gate.Finish != nil {
		key := policy.Key + "/" + gate.Kind
		runtime.State[key] = gate.Finish(runtime.State[key].(LawSpecValue), runtime.Clock.Now(), succeeded)
	}
}

func lsFibonacci(n int64) int64 {
	a, b := int64(1), int64(1)
	for i := int64(1); i < n; i++ {
		a, b = b, a+b
	}
	return a
}

// lsRetryDelay is the delay before attempt (2 or more), before jitter.
func lsRetryDelay(retry *lawSpecRetry, attempt int64) int64 {
	n := attempt - 1
	switch retry.Strategy {
	case "immediate":
		return 0
	case "fixed":
		return retry.Delay
	case "linear":
		return retry.Delay + retry.Step*(n-1)
	case "exponential":
		delay := retry.Delay
		for i := int64(1); i < n; i++ {
			delay *= retry.Factor
			if retry.Cap >= 0 && delay >= retry.Cap {
				return retry.Cap
			}
		}
		if retry.Cap >= 0 && delay > retry.Cap {
			return retry.Cap
		}
		return delay
	case "fibonacci":
		return retry.Delay * lsFibonacci(n)
	}
	panic("unknown retry strategy: " + retry.Strategy)
}

// lsJittered: full is [0, delay]; equal is delay/2 + [0, delay/2];
// decorrelated is [base, previous * 3], capped at delay.
func lsJittered(jitter string, delay, previous, base int64, random *LawSpecSplitMix64) int64 {
	switch jitter {
	case "full":
		return int64(random.Below(uint64(delay + 1)))
	case "equal":
		half := delay / 2
		return half + int64(random.Below(uint64(delay-half+1)))
	case "decorrelated":
		high := previous * 3
		if high < base {
			high = base
		}
		value := base + int64(random.Below(uint64(high-base+1)))
		if value > delay {
			return delay
		}
		return value
	}
	return delay
}

// lsRunStage runs a stage's attempts under its policy; a Left is a failure.
// key is the stage's input, for the cache.
func lsRunStage(symbols map[string]*lawSpecSymbol, policy lawSpecStagePolicy, attempt func() LawSpecValue, key LawSpecValue) LawSpecValue {
	runtime := lsWorkflowRuntime(symbols)
	if policy.Key == "" {
		policy.Key = policy.Stage
	}
	gates := policy.Gates
	if !runtime.Gates {
		gates = nil
	}
	cacheKey := policy.Key + "/cache"
	// A cache of no time (or none, -1) caches nothing.
	caching := policy.Cache > 0 && runtime.Gates
	if caching {
		now := runtime.Clock.Now()
		entries, _ := runtime.State[cacheKey].([]lawSpecCacheEntry)
		for _, entry := range entries {
			if now < entry.expires && lsCompareValues(entry.key, key) == 0 {
				runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"cached", policy.Stage, 0, true})
				return entry.value
			}
		}
	}
	for i, gate := range gates {
		if failure := lsPassGate(runtime, policy, gate); failure != "" {
			for _, passed := range gates[:i] {
				lsFinishGate(runtime, policy, passed, false)
			}
			if policy.Fail != nil {
				return policy.Fail(failure)
			}
			return lsStageFailureValue(failure)
		}
	}
	result := lsAttempts(runtime, policy, attempt)
	data, isData := result.Data.(lawSpecData)
	succeeded := !(isData && data.tag == "Either::Left")
	for _, gate := range gates {
		lsFinishGate(runtime, policy, gate, succeeded)
	}
	if succeeded && policy.Compensate != nil && len(runtime.frames) > 0 {
		value := data.fields[0]
		last := len(runtime.frames) - 1
		runtime.frames[last] = append(runtime.frames[last], lawSpecUndo{policy.Stage, func() { policy.Compensate(value) }})
	}
	if caching && succeeded {
		entries, _ := runtime.State[cacheKey].([]lawSpecCacheEntry)
		kept := []lawSpecCacheEntry{}
		for _, entry := range entries {
			if lsCompareValues(entry.key, key) != 0 {
				kept = append(kept, entry)
			}
		}
		runtime.State[cacheKey] = append(kept, lawSpecCacheEntry{key, result, runtime.Clock.Now() + policy.Cache})
	}
	return result
}

func lsAttempts(runtime *LawSpecWorkflowRuntime, policy lawSpecStagePolicy, attempt func() LawSpecValue) LawSpecValue {
	retry := policy.Retry
	number, previous := int64(1), int64(0)
	for {
		runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"start", policy.Stage, number, false})
		result := lsScoped(runtime, policy, attempt)
		data, isData := result.Data.(lawSpecData)
		failed := isData && data.tag == "Either::Left"
		runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"finish", policy.Stage, number, !failed})
		if !failed || retry == nil {
			return result
		}
		if retry.Attempts > 0 && number >= retry.Attempts {
			return result
		}
		failure := data.fields[0]
		if policy.Wraps {
			// Only the step's own failures and timeouts are retried.
			wrapped := failure.Data.(lawSpecData)
			if wrapped.tag == lsStageFailure+"StepFailed" {
				failure = wrapped.fields[0]
			} else if wrapped.tag != lsStageFailure+"TimedOut" {
				return result
			}
		}
		if retry.When != nil && !retry.When(failure) {
			return result
		}
		number++
		var delay int64
		if retry.Strategy == "custom" {
			wait, ok := retry.Decide(number, failure, previous)
			if !ok {
				return result
			}
			delay = wait
		} else {
			base := int64(0)
			if retry.Strategy != "immediate" {
				base = lsRetryDelay(retry, 2)
			}
			delay = lsJittered(retry.Jitter, lsRetryDelay(retry, number), previous, base, &runtime.Random)
		}
		runtime.Trace = append(runtime.Trace, LawSpecTraceEvent{"sleep", policy.Stage, delay, true})
		runtime.Clock.Sleep(delay)
		previous = delay
	}
}

func lsInteger64(n int64) LawSpecValue { return LawSpecValue{"Integer", big.NewInt(n)} }

// lsDuration is a logical Duration of whole microseconds.
func lsDuration(micros int64) LawSpecValue {
	return LawSpecValue{"lawspec.time::type::Duration", lawSpecData{"lawspec.time::type::Duration::Duration", []LawSpecValue{lsInteger64(micros)}}}
}

// lsRetryDecision reads a RetryDecision: RetryAfter's delay, or false to stop.
func lsRetryDecision(decision LawSpecValue) (int64, bool) {
	data := decision.Data.(lawSpecData)
	if data.tag != "lawspec.time::type::RetryDecision::RetryAfter" {
		return 0, false
	}
	delay := data.fields[0].Data.(lawSpecData).fields[0].Data.(*big.Int)
	return delay.Int64(), true
}

// Portable generation for stateful models. A type descriptor is an
// s-expression: (int T lo hi) with _ for no bound, (bool), (text), (unit),
// (list D), (maybe D), (either L R), (data NAME (ctor TAG D...) ...) and
// (ref NAME) for a data type declared in the model's table. Every target
// generates, shrinks and renders the same values for the same seed. Values
// are logical: a data type's value has its NAME as Type.

// lawSpecQuoted is a string atom of a descriptor; a bare atom is a string.
type lawSpecQuoted string

// lsReadDescriptor parses s-expressions: lists ([]any), integers (*big.Int),
// strings, symbols and _ (nil).
func lsReadDescriptor(source string) []any {
	text := []rune(source)
	position := 0
	space := func(c rune) bool { return c == ' ' || c == '\t' || c == '\r' || c == '\n' }
	skip := func() {
		for position < len(text) && space(text[position]) {
			position++
		}
	}
	var item func() any
	item = func() any {
		skip()
		c := text[position]
		if c == '(' {
			position++
			items := []any{}
			skip()
			for text[position] != ')' {
				items = append(items, item())
				skip()
			}
			position++
			return items
		}
		if c == '"' {
			position++
			out := []rune{}
			for text[position] != '"' {
				if text[position] == '\\' {
					position++
				}
				out = append(out, text[position])
				position++
			}
			position++
			return lawSpecQuoted(string(out))
		}
		start := position
		for position < len(text) && !space(text[position]) && text[position] != '(' && text[position] != ')' {
			position++
		}
		atom := string(text[start:position])
		if atom == "_" {
			return nil
		}
		if digits := strings.TrimLeft(atom, "-"); digits != "" && strings.Trim(digits, "0123456789") == "" {
			return lsInt(atom)
		}
		return atom
	}
	items := []any{}
	skip()
	for position < len(text) {
		items = append(items, item())
		skip()
	}
	return items
}

// lsAtom is an atom's text, quoted or not.
func lsAtom(x any) string {
	switch x := x.(type) {
	case string:
		return x
	case lawSpecQuoted:
		return string(x)
	}
	return fmt.Sprint(x)
}

// lsBelowBig is SplitMix64's Below for any bound, 2^64 (the full UInt64
// range) included: the raw draw modulo the bound.
func lsBelowBig(random *LawSpecSplitMix64, bound *big.Int) *big.Int {
	if bound.Sign() <= 0 {
		return new(big.Int)
	}
	draw := new(big.Int).SetUint64(random.Next())
	return draw.Mod(draw, bound)
}

// lsBelowCount is Below for a signed count; nothing is drawn below 1.
func lsBelowCount(random *LawSpecSplitMix64, bound int64) int64 {
	if bound <= 0 {
		return 0
	}
	return int64(random.Below(uint64(bound)))
}

const lsUnbounded = 1_000_000

// lawSpecValues generates, shrinks and renders over a table of data types.
type lawSpecValues struct{ table map[string][]any }

func (s lawSpecValues) resolve(d any) []any {
	form := d.([]any)
	if lsAtom(form[0]) == "ref" {
		return s.table[lsAtom(form[1])]
	}
	return form
}

// typeOf is the logical type name the runtime's codecs give such a value.
func (s lawSpecValues) typeOf(d any) string {
	form := s.resolve(d)
	switch lsAtom(form[0]) {
	case "int":
		return lsAtom(form[1])
	case "bool":
		return "Bool"
	case "text":
		return "Text"
	case "unit":
		return "Unit"
	case "list":
		return "List " + s.typeOf(form[1])
	case "maybe":
		return "Maybe " + s.typeOf(form[1])
	case "either":
		return "Either (" + s.typeOf(form[1]) + ") (" + s.typeOf(form[2]) + ")"
	}
	return lsAtom(form[1])
}

// bounds is an integer's range: a missing bound is 1,000,000 from zero, or
// 2,000,000 from the other bound when that is beyond it.
func (s lawSpecValues) bounds(d []any) (*big.Int, *big.Int) {
	unbounded, twice := big.NewInt(lsUnbounded), big.NewInt(2*lsUnbounded)
	lo, _ := d[2].(*big.Int)
	hi, _ := d[3].(*big.Int)
	switch {
	case lo == nil && hi == nil:
		return new(big.Int).Neg(unbounded), unbounded
	case lo == nil:
		low := new(big.Int).Sub(hi, twice)
		if minus := new(big.Int).Neg(unbounded); minus.Cmp(low) < 0 {
			low = minus
		}
		return low, hi
	case hi == nil:
		high := new(big.Int).Add(lo, twice)
		if unbounded.Cmp(high) > 0 {
			high = unbounded
		}
		return lo, high
	}
	return lo, hi
}

// lsClamp is min(max(n, lo), hi).
func lsClamp(n int64, lo, hi *big.Int) *big.Int {
	x := big.NewInt(n)
	if x.Cmp(lo) < 0 {
		x = lo
	}
	if x.Cmp(hi) > 0 {
		x = hi
	}
	return new(big.Int).Set(x)
}

func lsMentionsData(d any) bool {
	form, ok := d.([]any)
	if !ok {
		return false
	}
	if kind := lsAtom(form[0]); kind == "ref" || kind == "data" {
		return true
	}
	for _, x := range form[1:] {
		if lsMentionsData(x) {
			return true
		}
	}
	return false
}

// base is the constructors whose fields mention no data type.
func (s lawSpecValues) base(d []any) []any {
	found := []any{}
	for _, c := range d[2:] {
		mentions := false
		for _, f := range c.([]any)[2:] {
			mentions = mentions || lsMentionsData(f)
		}
		if !mentions {
			found = append(found, c)
		}
	}
	if len(found) == 0 {
		return d[2:]
	}
	return found
}

func lsSum(t, tag string, fields ...LawSpecValue) LawSpecValue {
	if len(fields) == 0 {
		fields = nil
	}
	return LawSpecValue{t, lawSpecData{tag, fields}}
}

func (s lawSpecValues) generate(d any, random *LawSpecSplitMix64, size int64) LawSpecValue {
	form := s.resolve(d)
	t := s.typeOf(form)
	switch lsAtom(form[0]) {
	case "int":
		lo, hi := s.bounds(form)
		if random.Below(10) < 2 {
			specials := []*big.Int{lo, hi, lsClamp(0, lo, hi), lsClamp(1, lo, hi)}
			return LawSpecValue{t, new(big.Int).Set(specials[random.Below(4)])}
		}
		span := new(big.Int).Sub(hi, lo)
		offset := lsBelowBig(random, span.Add(span, big.NewInt(1)))
		return LawSpecValue{t, offset.Add(offset, lo)}
	case "bool":
		return lsBool(random.Below(2) == 1)
	case "text":
		units := []int{}
		for n := lsBelowCount(random, size+1); n > 0; n-- {
			units = append(units, 32+int(random.Below(95)))
		}
		return LawSpecValue{t, units}
	case "unit":
		return LawSpecValue{t, nil}
	case "list":
		items := []LawSpecValue{}
		for n := lsBelowCount(random, size+1); n > 0; n-- {
			items = append(items, s.generate(form[1], random, size))
		}
		return LawSpecValue{t, items}
	case "maybe":
		if random.Below(4) == 0 {
			return lsSum(t, "Maybe::Nothing")
		}
		return lsSum(t, "Maybe::Just", s.generate(form[1], random, size))
	case "either":
		if random.Below(2) == 0 {
			return lsSum(t, "Either::Left", s.generate(form[1], random, size))
		}
		return lsSum(t, "Either::Right", s.generate(form[2], random, size))
	case "data":
		choices := form[2:]
		if size <= 0 {
			choices = s.base(form)
		}
		ctor := choices[random.Below(uint64(len(choices)))].([]any)
		fields := []LawSpecValue{}
		for _, f := range ctor[2:] {
			fields = append(fields, s.generate(f, random, max(size-1, 0)))
		}
		return lsSum(t, lsAtom(ctor[1]), fields...)
	}
	panic("unknown descriptor " + fmt.Sprint(form))
}

func (s lawSpecValues) minimal(d any) LawSpecValue {
	form := s.resolve(d)
	t := s.typeOf(form)
	switch lsAtom(form[0]) {
	case "int":
		lo, hi := s.bounds(form)
		return LawSpecValue{t, lsClamp(0, lo, hi)}
	case "bool":
		return lsBool(false)
	case "text":
		return LawSpecValue{t, []int{}}
	case "unit":
		return LawSpecValue{t, nil}
	case "list":
		return LawSpecValue{t, []LawSpecValue{}}
	case "maybe":
		return lsSum(t, "Maybe::Nothing")
	case "either":
		return lsSum(t, "Either::Left", s.minimal(form[1]))
	}
	ctor := s.base(form)[0].([]any)
	fields := []LawSpecValue{}
	for _, f := range ctor[2:] {
		fields = append(fields, s.minimal(f))
	}
	return lsSum(t, lsAtom(ctor[1]), fields...)
}

// lsSplice is items with index i replaced by the given values (or removed).
func lsSplice[T any](items []T, i int, with ...T) []T {
	out := append([]T{}, items[:i]...)
	out = append(out, with...)
	return append(out, items[i+1:]...)
}

// shrink is smaller candidates for v, most aggressive first.
func (s lawSpecValues) shrink(d any, v LawSpecValue) []LawSpecValue {
	form := s.resolve(d)
	t := s.typeOf(form)
	out := []LawSpecValue{}
	switch lsAtom(form[0]) {
	case "int":
		target := s.minimal(form)
		x, goal := v.Data.(*big.Int), target.Data.(*big.Int)
		if x.Cmp(goal) != 0 {
			// big.Int's Quo truncates toward zero.
			half := new(big.Int).Quo(new(big.Int).Sub(x, goal), big.NewInt(2))
			step := big.NewInt(-1)
			if x.Cmp(goal) > 0 {
				step = big.NewInt(1)
			}
			out = []LawSpecValue{target, {t, half.Sub(x, half)}, {t, step.Sub(x, step)}}
		}
	case "bool":
		if v.Data.(bool) {
			out = []LawSpecValue{lsBool(false)}
		}
	case "text":
		units := v.Data.([]int)
		if len(units) > 0 {
			out = []LawSpecValue{{t, []int{}}, {t, append([]int{}, units[:len(units)/2]...)}}
			for i := range units {
				out = append(out, LawSpecValue{t, lsSplice(units, i)})
			}
		}
	case "list":
		items := v.Data.([]LawSpecValue)
		if len(items) > 0 {
			out = []LawSpecValue{{t, []LawSpecValue{}}, {t, append([]LawSpecValue{}, items[:len(items)/2]...)}}
			for i := range items {
				out = append(out, LawSpecValue{t, lsSplice(items, i)})
			}
			for i, item := range items {
				for _, c := range s.shrink(form[1], item) {
					out = append(out, LawSpecValue{t, lsSplice(items, i, c)})
				}
			}
		}
	case "maybe":
		data := v.Data.(lawSpecData)
		if data.tag == "Maybe::Just" {
			out = []LawSpecValue{lsSum(t, "Maybe::Nothing")}
			for _, c := range s.shrink(form[1], data.fields[0]) {
				out = append(out, lsSum(t, "Maybe::Just", c))
			}
		}
	case "either":
		data := v.Data.(lawSpecData)
		inner := form[2]
		if data.tag == "Either::Left" {
			inner = form[1]
		}
		for _, c := range s.shrink(inner, data.fields[0]) {
			out = append(out, lsSum(t, data.tag, c))
		}
	case "data":
		data := v.Data.(lawSpecData)
		var ctor []any
		for _, c := range form[2:] {
			if lsAtom(c.([]any)[1]) == data.tag {
				ctor = c.([]any)
				break
			}
		}
		out = []LawSpecValue{s.minimal(form)}
		// A field of the same type is a smaller value of it.
		for i, fd := range ctor[2:] {
			if ref, ok := fd.([]any); ok && len(ref) == 2 && lsAtom(ref[0]) == "ref" && lsAtom(ref[1]) == lsAtom(form[1]) {
				out = append(out, data.fields[i])
			}
		}
		for i, fd := range ctor[2:] {
			for _, c := range s.shrink(fd, data.fields[i]) {
				out = append(out, lsSum(t, data.tag, lsSplice(data.fields, i, c)...))
			}
		}
	}
	// Candidates are distinct by rendered text, and none is v itself.
	seen := map[string]bool{lsRender(v): true}
	unique := []LawSpecValue{}
	for _, c := range out {
		if text := lsRender(c); !seen[text] {
			seen[text] = true
			unique = append(unique, c)
		}
	}
	return unique
}

// lsDifference is where a structured actual value first differs from the
// expected one, by the portable rendering: "" when they agree or neither has
// parts.
func lsDifference(actual, expected LawSpecValue) string {
	path := []string{}
	a, b := actual, expected
	shownA, shownB := "", ""
loop:
	for {
		switch x := a.Data.(type) {
		case []LawSpecValue:
			y, ok := b.Data.([]LawSpecValue)
			if !ok {
				break loop
			}
			for i := 0; i < len(x) && i < len(y); i++ {
				if lsRender(x[i]) != lsRender(y[i]) {
					path = append(path, fmt.Sprintf("item %d", i+1))
					a, b = x[i], y[i]
					continue loop
				}
			}
			if len(x) == len(y) {
				return ""
			}
			path = append(path, "length")
			shownA, shownB = strconv.Itoa(len(x)), strconv.Itoa(len(y))
			break loop
		case lawSpecData:
			y, ok := b.Data.(lawSpecData)
			if !ok || x.tag != y.tag || len(x.fields) != len(y.fields) {
				break loop
			}
			segments := strings.Split(x.tag, "::")
			for i := range x.fields {
				if lsRender(x.fields[i]) != lsRender(y.fields[i]) {
					path = append(path, fmt.Sprintf("field %d of %s", i+1, segments[len(segments)-1]))
					a, b = x.fields[i], y.fields[i]
					continue loop
				}
			}
			return ""
		default:
			break loop
		}
	}
	if len(path) == 0 {
		return ""
	}
	if shownA == "" {
		shownA, shownB = lsRender(a), lsRender(b)
	}
	return " | first difference at " + strings.Join(path, ", ") + ": expected " + shownB + ", actual " + shownA
}

// lsCodePointsText is the text of a Text value's code points.
func lsCodePointsText(units []int) string {
	runes := make([]rune, len(units))
	for i, c := range units {
		runes[i] = rune(c)
	}
	return string(runes)
}

// Portable regular expressions (see LawSpec.Regex): the subset of RE2 and
// ECMAScript that means the same in both, matched against a whole text, code
// point by code point. The compiler has checked every pattern; a pattern
// that is not portable panics here too.
type lsRegexItem struct {
	negated bool
	ranges  [][2]int
}

type lsRegexNode struct {
	kind     byte // 's' set, 'q' sequence, 'a' alternatives, 'r' repeat
	negated  bool
	items    []lsRegexItem
	children []*lsRegexNode
	low      int
	high     int // -1 when unbounded
}

// The portable dialect's character classes, and each pattern compiled once
// per process.
var (
	lsRegexDigits = [][2]int{{48, 57}}
	lsRegexWord   = [][2]int{{48, 57}, {65, 90}, {95, 95}, {97, 122}}
	lsRegexSpace  = [][2]int{{9, 13}, {32, 32}}
	lsRegexCache  sync.Map
)

func lsRegexParse(pattern string) *lsRegexNode {
	cs := []rune(pattern)
	n := len(cs)
	pos := 0
	peek := func() rune {
		if pos < n {
			return cs[pos]
		}
		return -1
	}
	fail := func(message string) {
		panic(fmt.Sprintf("regex %q is not portable: %s", pattern, message))
	}
	escape := func() lsRegexItem {
		pos++
		c := peek()
		if c < 0 {
			fail("the regex ends with a lone \\")
		}
		pos++
		switch c {
		case 'd':
			return lsRegexItem{false, lsRegexDigits}
		case 'D':
			return lsRegexItem{true, lsRegexDigits}
		case 'w':
			return lsRegexItem{false, lsRegexWord}
		case 'W':
			return lsRegexItem{true, lsRegexWord}
		case 's':
			return lsRegexItem{false, lsRegexSpace}
		case 'S':
			return lsRegexItem{true, lsRegexSpace}
		case 'n':
			return lsRegexItem{false, [][2]int{{10, 10}}}
		case 't':
			return lsRegexItem{false, [][2]int{{9, 9}}}
		case 'r':
			return lsRegexItem{false, [][2]int{{13, 13}}}
		case 'f':
			return lsRegexItem{false, [][2]int{{12, 12}}}
		case 'v':
			return lsRegexItem{false, [][2]int{{11, 11}}}
		}
		if strings.ContainsRune("\\.^$|?*+()[]{}-/", c) {
			return lsRegexItem{false, [][2]int{{int(c), int(c)}}}
		}
		fail("\\" + string(c) + " is not a portable escape")
		return lsRegexItem{}
	}
	literal := func() lsRegexItem {
		c := peek()
		if c == '[' {
			fail("write \\[ for the character inside a class")
		}
		pos++
		return lsRegexItem{false, [][2]int{{int(c), int(c)}}}
	}
	single := func(item lsRegexItem) bool {
		return !item.negated && len(item.ranges) == 1 && item.ranges[0][0] == item.ranges[0][1]
	}
	charClass := func() *lsRegexNode {
		pos++
		negated := peek() == '^'
		if negated {
			pos++
		}
		items := []lsRegexItem{}
		first := true
		for {
			c := peek()
			if c < 0 {
				fail("a [ is never closed")
			}
			if c == ']' {
				if first {
					fail("an empty class is not portable")
				}
				pos++
				return &lsRegexNode{kind: 's', negated: negated, items: items}
			}
			first = false
			var item lsRegexItem
			if c == '\\' {
				item = escape()
			} else {
				item = literal()
			}
			if single(item) && peek() == '-' && pos+1 < n && cs[pos+1] != ']' {
				pos++
				var high lsRegexItem
				if peek() == '\\' {
					high = escape()
				} else {
					high = literal()
				}
				if !single(high) {
					fail("a range ends with one character")
				}
				if high.ranges[0][0] < item.ranges[0][0] {
					fail("a range must run from low to high")
				}
				items = append(items, lsRegexItem{false, [][2]int{{item.ranges[0][0], high.ranges[0][0]}}})
			} else {
				items = append(items, item)
			}
		}
	}
	digits := func() string {
		start := pos
		for peek() >= '0' && peek() <= '9' {
			pos++
		}
		return string(cs[start:pos])
	}
	var alternatives func() *lsRegexNode
	atom := func() *lsRegexNode {
		c := peek()
		switch {
		case c == '(':
			pos++
			if peek() == '?' {
				if pos+1 < n && cs[pos+1] == ':' {
					pos += 2
				} else {
					fail("only (?: ...) groups are portable")
				}
			}
			node := alternatives()
			if peek() != ')' {
				fail("a ( is never closed")
			}
			pos++
			return node
		case c == '[':
			return charClass()
		case c == '.':
			pos++
			return &lsRegexNode{kind: 's', negated: true, items: []lsRegexItem{{false, [][2]int{{10, 10}}}}}
		case c == '\\':
			return &lsRegexNode{kind: 's', items: []lsRegexItem{escape()}}
		case strings.ContainsRune("*+?{^$]}", c):
			fail("unexpected " + string(c))
		}
		pos++
		return &lsRegexNode{kind: 's', items: []lsRegexItem{{false, [][2]int{{int(c), int(c)}}}}}
	}
	quantifier := func(c rune) bool { return c >= 0 && strings.ContainsRune("*+?{", c) }
	quantified := func(node *lsRegexNode) *lsRegexNode {
		c := peek()
		if !quantifier(c) {
			return node
		}
		pos++
		switch c {
		case '*':
			node = &lsRegexNode{kind: 'r', children: []*lsRegexNode{node}, low: 0, high: -1}
		case '+':
			node = &lsRegexNode{kind: 'r', children: []*lsRegexNode{node}, low: 1, high: -1}
		case '?':
			node = &lsRegexNode{kind: 'r', children: []*lsRegexNode{node}, low: 0, high: 1}
		default:
			lowText := digits()
			highText, bounded := "", true
			if peek() == '}' {
				highText = lowText
			} else if peek() == ',' {
				pos++
				highText = digits()
				bounded = highText != ""
				if peek() != '}' {
					fail("a repetition is {n}, {n,} or {n,m}")
				}
			} else {
				fail("a repetition is {n}, {n,} or {n,m}")
			}
			pos++
			if lowText == "" {
				fail("a repetition is {n}, {n,} or {n,m}")
			}
			low, _ := strconv.Atoi(lowText)
			high := -1
			if bounded {
				high, _ = strconv.Atoi(highText)
			}
			if low > 1000 || (bounded && (high > 1000 || high < low)) {
				fail("a repetition count is at most 1000, and n must not exceed m")
			}
			node = &lsRegexNode{kind: 'r', children: []*lsRegexNode{node}, low: low, high: high}
		}
		if quantifier(peek()) {
			fail("a repetition cannot itself be repeated")
		}
		return node
	}
	sequence := func() *lsRegexNode {
		items := []*lsRegexNode{}
		for peek() >= 0 && peek() != '|' && peek() != ')' {
			items = append(items, quantified(atom()))
		}
		return &lsRegexNode{kind: 'q', children: items}
	}
	alternatives = func() *lsRegexNode {
		branches := []*lsRegexNode{sequence()}
		for peek() == '|' {
			pos++
			branches = append(branches, sequence())
		}
		if len(branches) == 1 {
			return branches[0]
		}
		return &lsRegexNode{kind: 'a', children: branches}
	}
	node := alternatives()
	if pos != n {
		fail("a ) has no ( before it")
	}
	return node
}

func lsRegexReach(node *lsRegexNode, text []int, positions []bool) []bool {
	next := make([]bool, len(text)+1)
	switch node.kind {
	case 's':
		for p, at := range positions {
			if !at || p >= len(text) {
				continue
			}
			c := text[p]
			inside := false
			for _, item := range node.items {
				in := false
				for _, r := range item.ranges {
					if r[0] <= c && c <= r[1] {
						in = true
						break
					}
				}
				if in != item.negated {
					inside = true
					break
				}
			}
			if inside != node.negated {
				next[p+1] = true
			}
		}
		return next
	case 'q':
		for _, child := range node.children {
			positions = lsRegexReach(child, text, positions)
		}
		return positions
	case 'a':
		for _, child := range node.children {
			for p, at := range lsRegexReach(child, text, positions) {
				if at {
					next[p] = true
				}
			}
		}
		return next
	}
	body := node.children[0]
	for i := 0; i < node.low; i++ {
		positions = lsRegexReach(body, text, positions)
	}
	seen := append([]bool(nil), positions...)
	frontier := positions
	limit := -1
	if node.high >= 0 {
		limit = node.high - node.low
	}
	for limit != 0 {
		reached := lsRegexReach(body, text, frontier)
		frontier = make([]bool, len(text)+1)
		any := false
		for p, at := range reached {
			if at && !seen[p] {
				frontier[p], seen[p], any = true, true, true
			}
		}
		if !any {
			break
		}
		if limit > 0 {
			limit--
		}
	}
	return seen
}

// LsRegexMatches reports whether the portable regex matches all of text,
// given as code points.
func LsRegexMatches(pattern string, text []int) bool {
	cached, ok := lsRegexCache.Load(pattern)
	if !ok {
		cached, _ = lsRegexCache.LoadOrStore(pattern, lsRegexParse(pattern))
	}
	start := make([]bool, len(text)+1)
	start[0] = true
	return lsRegexReach(cached.(*lsRegexNode), text, start)[len(text)]
}

// Recorded values: a law compares a value's portable rendering with the
// text stored under recorded/<unit>/<name> in the project. LAWSPEC_RECORDED
// names the folder; otherwise it is recorded/ in the nearest folder, from
// the working one up, that holds lawspec.json or recorded/. With
// LAWSPEC_UPDATE_RECORDED=1 (lawspec test --update-recorded) a law records
// the value instead.
func lsRecordedRoot() string {
	if given := os.Getenv("LAWSPEC_RECORDED"); given != "" {
		return given
	}
	start, _ := os.Getwd()
	folder := start
	for {
		if _, err := os.Stat(filepath.Join(folder, "lawspec.json")); err == nil {
			return filepath.Join(folder, "recorded")
		}
		if info, err := os.Stat(filepath.Join(folder, "recorded")); err == nil && info.IsDir() {
			return filepath.Join(folder, "recorded")
		}
		parent := filepath.Dir(folder)
		if parent == folder {
			return filepath.Join(start, "recorded")
		}
		folder = parent
	}
}

func lsRecorded(key string, value LawSpecValue) bool {
	text := lsRecordedText(value)
	path := filepath.Join(append([]string{lsRecordedRoot()}, strings.Split(key, "/")...)...)
	if os.Getenv("LAWSPEC_UPDATE_RECORDED") == "1" {
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			panic(err)
		}
		if err := os.WriteFile(path, []byte(text+"\n"), 0o644); err != nil {
			panic(err)
		}
		return true
	}
	stored, err := os.ReadFile(path)
	if err != nil {
		panic("no recording recorded/" + key + "; run lawspec test --update-recorded to record " + text)
	}
	if storedText := strings.TrimSuffix(string(stored), "\n"); storedText != text {
		panic("recorded/" + key + " differs: expected " + storedText + ", actual " + text +
			" (lawspec test --update-recorded records the new value)")
	}
	return true
}

// Shared resources (share R per group | unit | run, in a harness): one value
// per scope key. The first use acquires it; every later use resets it first,
// so no case sees what another left. One case holds it at a time, and it is
// released when the test binary ends (lsReleaseShared, from TestMain).
type lsSharedEntry struct {
	lock       sync.Mutex
	guard      sync.Mutex
	held       bool
	users      int
	concurrent bool
	value      LawSpecValue
	release    func(LawSpecValue)
}

// The resources shared by a harness, by scope key, and the order they were
// first acquired in, so they are released last first when the process ends.
var (
	lsSharedGuard sync.Mutex
	lsShared      = map[string]*lsSharedEntry{}
	lsSharedOrder []string
)

// A concurrent resource (resource T is concurrent) is held by any number of
// cases at once, and reset only when none holds it.
func lsShare(key string, acquire func() LawSpecValue, reset func(LawSpecValue), release func(LawSpecValue), concurrent bool) LawSpecValue {
	lsSharedGuard.Lock()
	entry, ok := lsShared[key]
	if !ok {
		entry = &lsSharedEntry{concurrent: concurrent}
		lsShared[key] = entry
		lsSharedOrder = append(lsSharedOrder, key)
	}
	lsSharedGuard.Unlock()
	if concurrent {
		entry.guard.Lock()
		defer entry.guard.Unlock()
		if entry.held && entry.users == 0 {
			reset(entry.value)
		} else if !entry.held {
			entry.value = acquire()
			entry.held = true
			entry.release = release
		}
		entry.users++
		return entry.value
	}
	entry.lock.Lock()
	ready := false
	defer func() {
		if !ready {
			entry.lock.Unlock()
		}
	}()
	if entry.held {
		reset(entry.value)
	} else {
		entry.value = acquire()
		entry.held = true
		entry.release = release
	}
	ready = true
	return entry.value
}

func lsUnshare(key string) {
	lsSharedGuard.Lock()
	entry := lsShared[key]
	lsSharedGuard.Unlock()
	if entry.concurrent {
		entry.guard.Lock()
		entry.users--
		entry.guard.Unlock()
		return
	}
	entry.lock.Unlock()
}

// lsReleaseShared releases every shared resource, the last acquired first.
func lsReleaseShared() {
	lsSharedGuard.Lock()
	defer lsSharedGuard.Unlock()
	for i := len(lsSharedOrder) - 1; i >= 0; i-- {
		if entry := lsShared[lsSharedOrder[i]]; entry.held {
			entry.release(entry.value)
			entry.held = false
		}
	}
}

// Built-in resources (see LawSpec.Resources): a law acquires them before
// each case and releases them after it.
func lsAcquireResource(kind string) string {
	switch kind {
	case "temporaryDirectory":
		dir, err := os.MkdirTemp("", "lawspec-")
		if err != nil {
			panic(err)
		}
		return dir
	case "temporaryFile":
		file, err := os.CreateTemp("", "lawspec-")
		if err != nil {
			panic(err)
		}
		file.Close()
		return file.Name()
	case "environment":
		saved := map[string]string{}
		for _, entry := range os.Environ() {
			if i := strings.IndexByte(entry, '='); i > 0 {
				saved[entry[:i]] = entry[i+1:]
			}
		}
		text, _ := json.Marshal(saved)
		return string(text)
	}
	panic("unknown resource kind " + kind)
}

func lsReleaseResource(kind, value string) {
	switch kind {
	case "temporaryDirectory", "temporaryFile":
		os.RemoveAll(value)
	case "environment":
		saved := map[string]string{}
		if err := json.Unmarshal([]byte(value), &saved); err != nil {
			panic(err)
		}
		for _, entry := range os.Environ() {
			if i := strings.IndexByte(entry, '='); i > 0 {
				if _, kept := saved[entry[:i]]; !kept {
					os.Unsetenv(entry[:i])
				}
			}
		}
		for name, text := range saved {
			if current, set := os.LookupEnv(name); !set || current != text {
				os.Setenv(name, text)
			}
		}
	default:
		panic("unknown resource kind " + kind)
	}
}

// lsFreePort is a TCP port on the local host that is free now.
func lsFreePort() int {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		panic(err)
	}
	defer listener.Close()
	return listener.Addr().(*net.TCPAddr).Port
}

// lsRender is a value's canonical text, the same on every target.
func lsRender(v LawSpecValue) string {
	switch x := v.Data.(type) {
	case bool:
		return strconv.FormatBool(x)
	case *big.Int:
		return x.String()
	case []int:
		var b strings.Builder
		b.WriteByte('"')
		for _, c := range x {
			if c == '\\' || c == '"' {
				b.WriteByte('\\')
			}
			b.WriteRune(rune(c))
		}
		b.WriteByte('"')
		return b.String()
	case nil:
		return "()"
	case []LawSpecValue:
		parts := make([]string, len(x))
		for i, item := range x {
			parts[i] = lsRender(item)
		}
		return "[" + strings.Join(parts, ", ") + "]"
	case lawSpecData:
		segments := strings.Split(x.tag, "::")
		name := segments[len(segments)-1]
		if len(x.fields) == 0 {
			return name
		}
		parts := make([]string, len(x.fields))
		for i, field := range x.fields {
			parts[i] = lsRender(field)
		}
		return name + "(" + strings.Join(parts, ", ") + ")"
	case lawSpecHandle:
		return lsHandleLabel(v.Type, x.native)
	}
	return fmt.Sprint(v.Data)
}

// lsRecordedText preserves the basic model rendering and gives extended
// scalars explicit forms. Identity numbers are local to this recording.
// ref:REQ-law-primitives ref:DEC-portable-exact-arithmetic
func lsRecordedText(value LawSpecValue) string {
	symbols := map[*lawSpecSymbol]int{}
	handles := map[string][]any{}
	quote := func(s string) string {
		return "\"" + strings.ReplaceAll(strings.ReplaceAll(s, "\\", "\\\\"), "\"", "\\\"") + "\""
	}
	floating := func(v float64, width int) string {
		name := "float" + strconv.Itoa(width)
		if math.IsNaN(v) {
			return name + "NaN"
		}
		if width == 32 {
			return fmt.Sprintf("%sBits(\"%08x\")", name, math.Float32bits(float32(v)))
		}
		return fmt.Sprintf("%sBits(\"%016x\")", name, math.Float64bits(v))
	}
	var visit func(LawSpecValue) string
	visit = func(v LawSpecValue) string {
		switch x := v.Data.(type) {
		case bool:
			return strconv.FormatBool(x)
		case *big.Int:
			return x.String()
		case lawSpecDecimal:
			if x.coefficient.Sign() == 0 {
				return "0e0"
			}
			c, e := new(big.Int).Set(x.coefficient), big.NewInt(int64(x.exponent))
			ten, one := big.NewInt(10), big.NewInt(1)
			for new(big.Int).Rem(c, ten).Sign() == 0 {
				c.Quo(c, ten)
				e.Add(e, one)
			}
			return c.String() + "e" + e.String()
		case *big.Rat:
			return "rational(" + x.Num().String() + ", " + x.Denom().String() + ")"
		case float64:
			width := 64
			if v.Type == "Float32" {
				width = 32
			}
			return floating(x, width)
		case complex128:
			width := 64
			if v.Type == "Complex64" {
				width = 32
			}
			return v.Type + "(" + floating(real(x), width) + ", " + floating(imag(x), width) + ")"
		case int:
			if v.Type == "Char" {
				return quote(string(rune(x)))
			}
			return strconv.Itoa(x)
		case []int:
			if v.Type == "Text" {
				runes := make([]rune, len(x))
				for i, n := range x {
					runes[i] = rune(n)
				}
				return quote(string(runes))
			}
			name := map[string]string{"Bytes": "bytes", "CodePointText": "codePoints", "Utf16Text": "utf16"}[v.Type]
			if name == "" {
				panic("invalid recorded sequence type: " + v.Type)
			}
			parts := make([]string, len(x))
			for i, n := range x {
				parts[i] = strconv.Itoa(n)
			}
			return name + "([" + strings.Join(parts, ", ") + "])"
		case *lawSpecSymbol:
			n, ok := symbols[x]
			if !ok {
				n = len(symbols) + 1
				symbols[x] = n
			}
			return "symbol(" + strconv.Itoa(n) + ", " + quote(x.description) + ")"
		case nil:
			switch v.Type {
			case "Unit":
				return "()"
			case "Null":
				return "null"
			case "Undefined":
				return "undefined"
			}
		case lawSpecPresence:
			kind := strings.SplitN(v.Type, " ", 2)[0]
			if x.value == nil {
				if kind == "Nullable" {
					return "null"
				}
				return "undefined"
			}
			return strings.ToLower(kind) + "(" + visit(*x.value) + ")"
		case []LawSpecValue:
			parts := make([]string, len(x))
			for i, item := range x {
				parts[i] = visit(item)
			}
			return "[" + strings.Join(parts, ", ") + "]"
		case lawSpecData:
			segments := strings.Split(x.tag, "::")
			name := segments[len(segments)-1]
			if len(x.fields) == 0 {
				return name
			}
			parts := make([]string, len(x.fields))
			for i, item := range x.fields {
				parts[i] = visit(item)
			}
			return name + "(" + strings.Join(parts, ", ") + ")"
		case lawSpecHandle:
			known := handles[v.Type]
			number := 0
			for i, other := range known {
				if lsSameNative(other, x.native) {
					number = i + 1
					break
				}
			}
			if number == 0 {
				number = len(known) + 1
				handles[v.Type] = append(known, x.native)
			}
			segments := strings.Split(v.Type, "::")
			return segments[len(segments)-1] + "#" + strconv.Itoa(number)
		}
		panic("invalid recorded value: " + v.Type)
	}
	return visit(value)
}

// lsValuesFrom is a descriptor text's data types and its last form, the one
// generated.
func lsValuesFrom(text string) (lawSpecValues, any) {
	forms := lsReadDescriptor(text)
	table := map[string][]any{}
	for _, f := range forms {
		if form, ok := f.([]any); ok && lsAtom(form[0]) == "data" {
			table[lsAtom(form[1])] = form
		}
	}
	return lawSpecValues{table}, forms[len(forms)-1]
}

// LawSpecGenerated is count values generated from one SplitMix64 seed, rendered.
func LawSpecGenerated(text string, seed uint64, size int64, count int64) []string {
	values, d := lsValuesFrom(text)
	random := LawSpecSplitMix64{seed}
	result := []string{}
	for ; count > 0; count-- {
		result = append(result, lsRender(values.generate(d, &random, size)))
	}
	return result
}

// LawSpecShrunk is the shrink candidates of the first value generated, rendered.
func LawSpecShrunk(text string, seed uint64, size int64) []string {
	values, d := lsValuesFrom(text)
	random := LawSpecSplitMix64{seed}
	result := []string{}
	for _, c := range values.shrink(d, values.generate(d, &random, size)) {
		result = append(result, lsRender(c))
	}
	return result
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
// Errors from generated definitions are panics, recovered here.

// LawSpecModelCallback is a generated definition: symbols, then arguments.
type LawSpecModelCallback func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue

// LawSpecModel is a model's spec and its callbacks. Start is the system's
// start and the model's; each command is its run, reference and
// precondition (nil when absent); Abstract may be nil.
type LawSpecModel struct {
	Spec       string
	Start      [2]LawSpecModelCallback
	Commands   [][3]LawSpecModelCallback
	Abstract   LawSpecModelCallback
	Invariants []LawSpecModelCallback
}

type lawSpecModelCommand struct {
	name                 string
	arguments            []any
	state                int
	unit                 bool
	needs, shifts        [][2]any
	key                  int // the argument naming the key it touches, for per-key checks; -1 when none
	restart              bool
	run, reference, when LawSpecModelCallback
}

type lawSpecMachine struct {
	name           string
	shared         bool
	values         lawSpecValues
	startIndices   []int64
	startArguments []any
	startRun       LawSpecModelCallback
	startModel     LawSpecModelCallback
	commands       []lawSpecModelCommand
	abstract       LawSpecModelCallback
	invariantKinds []string
	invariants     []LawSpecModelCallback
	perKey         bool
	// consistency is what histories are checked by: linearizable,
	// sequential, causal or eventual.
	consistency string
	// steps is the commands a sequential run may take: an actor's also end
	// with an injected crash.
	steps []lawSpecModelCommand
	// lastStart is the start arguments of the run being simulated or
	// executed, for a crash that restarts from the start.
	lastStart []LawSpecValue
}

type lawSpecModelStep struct {
	index int
	args  []LawSpecValue
}

type lawSpecModelRun struct {
	start []LawSpecValue
	steps []lawSpecModelStep
}

// lawSpecInvalid is panicked for a command the model does not allow here.
type lawSpecInvalid struct{}

func lsFormFields(forms []any) map[string][]any {
	fields := map[string][]any{}
	for _, f := range forms {
		form := f.([]any)
		fields[lsAtom(form[0])] = form[1:]
	}
	return fields
}

func lsPairs(forms []any) [][2]any {
	pairs := [][2]any{}
	for _, f := range forms {
		form := f.([]any)
		pairs = append(pairs, [2]any{lsAtom(form[0]), form[1].(*big.Int).Int64()})
	}
	return pairs
}

func (c *lawSpecModelCommand) admits(indices []int64) bool {
	for k := 0; k < len(c.needs) && k < len(indices); k++ {
		n := c.needs[k][1].(int64)
		if c.needs[k][0] == "atleast" {
			if indices[k] < n {
				return false
			}
		} else if indices[k] != n {
			return false
		}
	}
	return true
}

func (c *lawSpecModelCommand) shifted(indices []int64) []int64 {
	out := []int64{}
	for k := 0; k < len(c.shifts) && k < len(indices); k++ {
		s := c.shifts[k][1].(int64)
		if c.shifts[k][0] == "by" {
			out = append(out, indices[k]+s)
		} else {
			out = append(out, s)
		}
	}
	return out
}

func lsNewMachine(model LawSpecModel) *lawSpecMachine {
	forms := lsReadDescriptor(model.Spec)
	head := forms[0].([]any)
	m := &lawSpecMachine{name: lsAtom(head[1]), shared: lsAtom(head[2]) == "shared", consistency: "linearizable"}
	table := map[string][]any{}
	commandIndex := 0
	actor := false
	for _, f := range forms {
		form := f.([]any)
		switch lsAtom(form[0]) {
		case "data":
			table[lsAtom(form[1])] = form
		case "start":
			fields := lsFormFields(form[1:])
			for _, i := range fields["indices"] {
				m.startIndices = append(m.startIndices, i.(*big.Int).Int64())
			}
			m.startArguments = fields["arguments"]
		case "command":
			fields := lsFormFields(form[2:])
			callbacks := model.Commands[commandIndex]
			commandIndex++
			key := -1
			if k, ok := fields["key"]; ok && len(k) > 0 {
				if n, isNumber := k[0].(*big.Int); isNumber {
					key = int(n.Int64())
				}
			}
			m.commands = append(m.commands, lawSpecModelCommand{
				name:      lsAtom(form[1]),
				arguments: fields["arguments"],
				state:     int(fields["state"][0].(*big.Int).Int64()),
				unit:      lsAtom(fields["unit"][0]) == "true",
				needs:     lsPairs(fields["needs"]),
				shifts:    lsPairs(fields["shifts"]),
				key:       key,
				restart:   len(fields["restart"]) > 0 && lsAtom(fields["restart"][0]) == "true",
				run:       callbacks[0], reference: callbacks[1], when: callbacks[2],
			})
		case "invariants":
			for _, k := range form[1:] {
				m.invariantKinds = append(m.invariantKinds, lsAtom(k))
			}
		case "perkey":
			m.perKey = len(form) > 1 && lsAtom(form[1]) == "true"
		case "consistency":
			if len(form) > 1 {
				m.consistency = lsAtom(form[1])
			}
		case "actor":
			actor = len(form) > 1 && lsAtom(form[1]) == "true"
		}
	}
	m.values = lawSpecValues{table}
	m.startRun, m.startModel = model.Start[0], model.Start[1]
	m.abstract = model.Abstract
	m.invariants = model.Invariants
	// A restart command is never a step; an actor's injected crashes run it.
	restarts, commands := []lawSpecModelCommand{}, []lawSpecModelCommand{}
	for _, c := range m.commands {
		if c.restart {
			restarts = append(restarts, c)
		} else {
			commands = append(commands, c)
		}
	}
	m.commands = commands
	m.steps = m.commands
	if actor {
		lsActorMachine(m, restarts)
	}
	return m
}

// lsActorMachine runs an actor model's start and handlers inside an actor;
// the abstraction and state invariants read its state between messages.
func lsActorMachine(m *lawSpecMachine, restarts []lawSpecModelCommand) {
	start, begin := m.startRun, m.startModel
	if start != nil {
		m.startRun = func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue {
			m.lastStart = args
			return LawSpecValue{"actor", lawSpecHandle{NewLawSpecActor(start(symbols, args))}}
		}
	}
	if begin != nil {
		m.startModel = func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue {
			m.lastStart = args
			return begin(symbols, args)
		}
	}
	// Sequential runs also inject crashes: the actor restarts from its last
	// state (restart from) or its start, and the model follows the restart's
	// reference (or the start's model state).
	var restart *lawSpecModelCommand
	if len(restarts) > 0 {
		restart = &restarts[0]
	}
	crash := lawSpecModelCommand{name: "crash", unit: true, key: -1}
	crash.run = func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue {
		err := lsActorOf(args[0]).Restart(func(state any) any {
			if restart != nil {
				return restart.run(symbols, []LawSpecValue{state.(LawSpecValue)})
			}
			return start(symbols, m.lastStart)
		})
		if err != nil {
			panic(err.Error())
		}
		return lsAbsent("Unit")
	}
	crash.reference = func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue {
		if restart != nil {
			return restart.reference(symbols, args[len(args)-1:])
		}
		return begin(symbols, m.lastStart)
	}
	defer func() { m.steps = append(append([]lawSpecModelCommand{}, m.commands...), crash) }()
	for i := range m.commands {
		run, unit := m.commands[i].run, m.commands[i].unit
		m.commands[i].run = func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue {
			actor := lsActorOf(args[0])
			rest := args[1:]
			reply, err := actor.Call(func(state any) (any, any) {
				out := run(symbols, append([]LawSpecValue{state.(LawSpecValue)}, rest...))
				if unit {
					return lsAbsent("Unit"), out
				}
				fields := lsFields(out)
				return fields[0], fields[1]
			})
			if err != nil {
				panic(err.Error())
			}
			return reply.(LawSpecValue)
		}
	}
	read := func(f LawSpecModelCallback) LawSpecModelCallback {
		if f == nil {
			return nil
		}
		return func(symbols map[string]*LawSpecSymbol, args []LawSpecValue) LawSpecValue {
			state, err := lsActorOf(args[0]).State()
			if err != nil {
				panic(err.Error())
			}
			return f(symbols, []LawSpecValue{state.(LawSpecValue)})
		}
	}
	m.abstract = read(m.abstract)
	invariants := append([]LawSpecModelCallback{}, m.invariants...)
	for i, kind := range m.invariantKinds {
		if i < len(invariants) && kind != "model" {
			invariants[i] = read(invariants[i])
		}
	}
	m.invariants = invariants
}

func lsActorOf(v LawSpecValue) *LawSpecActor {
	return v.Data.(lawSpecHandle).native.(*LawSpecActor)
}

// Actors. An actor owns a state and handles one message at a time, in the
// order they arrive. It is not a goroutine: a message sent to an idle actor
// starts a goroutine that drains its mailbox and then exits, so an idle
// actor costs only its state and queue. A handler that panics crashes the
// actor: the caller gets a LawSpecActorCrashed error, a supervised actor
// restarts in place (keeping its address and the messages waiting for it)
// and any other actor stops.

// LawSpecActorStopped is the error of a message sent to a stopped actor, or
// received from a closed, empty mailbox.
var LawSpecActorStopped = errors.New("the actor has stopped")

// LawSpecActorCrashed is the error of a call whose handler failed; Cause is
// what the handler panicked with.
type LawSpecActorCrashed struct{ Cause any }

func (e LawSpecActorCrashed) Error() string { return fmt.Sprintf("the actor crashed: %v", e.Cause) }

// LawSpecActorHandler takes the actor's state and gives the reply and the
// next state; a panic in it crashes the actor.
type LawSpecActorHandler func(state any) (reply, next any)

// LawSpecExit is what a monitor hears: "crashed" with the cause, or
// "stopped".
type LawSpecExit struct {
	Reason string
	Cause  any
}

type lawSpecActorMessage struct {
	handler LawSpecActorHandler
	reply   chan lawSpecActorReply
}

type lawSpecActorReply struct {
	value any
	err   error
}

// lawSpecRestart is panicked by a message that crashes the actor on
// purpose (Crash, links); origin names the first crash.
type lawSpecRestart struct {
	cause  any
	origin uint64
}

var lawSpecCrashes atomic.Uint64

// lawSpecChild is an actor or a supervisor under a supervisor.
type lawSpecChild interface {
	restartLater()
	halt()
	setSupervisor(*LawSpecSupervisor)
	Stop()
}

// LawSpecActor is a running actor.
type LawSpecActor struct {
	mu         sync.Mutex
	state      any
	restart    func(any) any
	mailbox    []lawSpecActorMessage
	draining   bool
	stopped    bool
	supervisor *LawSpecSupervisor
	monitors   []func(LawSpecExit)
	links      []*LawSpecActor
	seen       map[uint64]bool
}

// NewLawSpecActor starts an actor owning state; a crash stops it.
func NewLawSpecActor(state any) *LawSpecActor {
	return &LawSpecActor{state: state, seen: map[uint64]bool{}}
}

// NewLawSpecActorWithRestart starts an actor owning state; restart gives its
// state after a crash from the last one, when a supervisor restarts it.
func NewLawSpecActorWithRestart(state any, restart func(any) any) *LawSpecActor {
	return &LawSpecActor{state: state, restart: restart, seen: map[uint64]bool{}}
}

func (a *LawSpecActor) setSupervisor(s *LawSpecSupervisor) {
	a.mu.Lock()
	a.supervisor = s
	a.mu.Unlock()
}

func (a *LawSpecActor) post(message lawSpecActorMessage) error {
	a.mu.Lock()
	if a.stopped {
		a.mu.Unlock()
		return LawSpecActorStopped
	}
	a.mailbox = append(a.mailbox, message)
	start := !a.draining
	a.draining = true
	a.mu.Unlock()
	if start {
		go a.drain()
	}
	return nil
}

func (a *LawSpecActor) drain() {
	for {
		a.mu.Lock()
		if len(a.mailbox) == 0 {
			a.draining = false
			a.mu.Unlock()
			return
		}
		message := a.mailbox[0]
		a.mailbox[0] = lawSpecActorMessage{}
		a.mailbox = a.mailbox[1:]
		a.mu.Unlock()
		outcome := a.handle(message.handler)
		if message.reply != nil {
			message.reply <- outcome
		}
	}
}

// handle runs one message; only the draining goroutine touches the state.
func (a *LawSpecActor) handle(handler LawSpecActorHandler) (outcome lawSpecActorReply) {
	crashed, cause, origin := false, any(nil), uint64(0)
	func() {
		defer func() {
			if problem := recover(); problem != nil {
				crashed = true
				if r, ok := problem.(lawSpecRestart); ok {
					// A linked crash already handled here is not handled again.
					a.mu.Lock()
					crashed = !a.seen[r.origin]
					a.mu.Unlock()
					cause, origin = r.cause, r.origin
					outcome = lawSpecActorReply{nil, nil}
				} else {
					cause, origin = problem, lawSpecCrashes.Add(1)
					outcome = lawSpecActorReply{nil, LawSpecActorCrashed{problem}}
				}
			}
		}()
		reply, next := handler(a.state)
		a.state = next
		outcome = lawSpecActorReply{reply, nil}
	}()
	if crashed {
		a.crashed(cause, origin)
	}
	return outcome
}

// crashed runs on the actor's turn: restart or stop, then tell monitors and
// links. origin names the first crash, so a crash crosses each link once.
func (a *LawSpecActor) crashed(cause any, origin uint64) {
	a.mu.Lock()
	a.seen[origin] = true
	supervisor := a.supervisor
	monitors := append([]func(LawSpecExit){}, a.monitors...)
	links := append([]*LawSpecActor{}, a.links...)
	a.mu.Unlock()
	restarted := supervisor != nil && a.restart != nil && supervisor.childCrashed(a, cause)
	if !restarted {
		a.halt()
	}
	for _, m := range monitors {
		m(LawSpecExit{"crashed", cause})
	}
	for _, other := range links {
		other.linkCrash(cause, origin)
	}
}

func (a *LawSpecActor) restartNow() { a.state = a.restart(a.state) }

// restartLater is a restart a supervisor asks of a sibling, in mailbox order.
func (a *LawSpecActor) restartLater() {
	if a.restart == nil {
		return
	}
	_ = a.post(lawSpecActorMessage{func(state any) (any, any) { return nil, a.restart(state) }, nil})
}

func (a *LawSpecActor) linkCrash(cause any, origin uint64) {
	a.mu.Lock()
	seen := a.seen[origin]
	a.mu.Unlock()
	if seen {
		return
	}
	_ = a.post(lawSpecActorMessage{func(any) (any, any) { panic(lawSpecRestart{cause, origin}) }, nil})
}

// halt stops the actor; messages still waiting fail with LawSpecActorStopped.
func (a *LawSpecActor) halt() {
	a.mu.Lock()
	a.stopped = true
	waiting := a.mailbox
	a.mailbox = nil
	a.mu.Unlock()
	for _, m := range waiting {
		if m.reply != nil {
			m.reply <- lawSpecActorReply{nil, LawSpecActorStopped}
		}
	}
}

// Call sends a message and waits for its reply. A handler that panics
// crashes the actor, and Call returns a LawSpecActorCrashed error.
func (a *LawSpecActor) Call(handler LawSpecActorHandler) (any, error) {
	reply := make(chan lawSpecActorReply, 1)
	if err := a.post(lawSpecActorMessage{handler, reply}); err != nil {
		return nil, err
	}
	outcome := <-reply
	return outcome.value, outcome.err
}

// Cast sends a message without waiting for its reply.
func (a *LawSpecActor) Cast(handler LawSpecActorHandler) error {
	return a.post(lawSpecActorMessage{handler, nil})
}

// Crash crashes the actor once the messages before it are handled, as a
// failing handler would: for testing supervision.
func (a *LawSpecActor) Crash(cause any) error {
	origin := lawSpecCrashes.Add(1)
	_, err := a.Call(func(any) (any, any) { panic(lawSpecRestart{cause, origin}) })
	return err
}

// Restart replaces the state by restart(last state) between messages, as a
// supervised restart does.
func (a *LawSpecActor) Restart(restart func(any) any) error {
	_, err := a.Call(func(state any) (any, any) { return nil, restart(state) })
	return err
}

// State is the state after every message sent before this call.
func (a *LawSpecActor) State() (any, error) {
	return a.Call(func(state any) (any, any) { return state, state })
}

// Monitor calls notify with "crashed" after each crash, and "stopped" once
// the actor stops.
func (a *LawSpecActor) Monitor(notify func(LawSpecExit)) {
	a.mu.Lock()
	a.monitors = append(a.monitors, notify)
	a.mu.Unlock()
}

// Link links two actors: when either crashes, the other crashes too.
func (a *LawSpecActor) Link(other *LawSpecActor) {
	a.mu.Lock()
	a.links = append(a.links, other)
	a.mu.Unlock()
	other.mu.Lock()
	other.links = append(other.links, a)
	other.mu.Unlock()
}

// Stop refuses further messages; those already sent are still handled. A
// permanent child of a supervisor restarts instead.
func (a *LawSpecActor) Stop() {
	a.mu.Lock()
	supervisor := a.supervisor
	a.mu.Unlock()
	if supervisor != nil && supervisor.childStopped(a) {
		return
	}
	a.mu.Lock()
	already := a.stopped
	a.stopped = true
	monitors := append([]func(LawSpecExit){}, a.monitors...)
	a.mu.Unlock()
	if !already {
		for _, m := range monitors {
			m(LawSpecExit{"stopped", nil})
		}
	}
}

// Supervision strategies and lifetimes.
const (
	LawSpecOneForOne  = "one_for_one"
	LawSpecOneForAll  = "one_for_all"
	LawSpecRestForOne = "rest_for_one"
	LawSpecPermanent  = "permanent"
	LawSpecTransient  = "transient"
	LawSpecTemporary  = "temporary"
)

type lawSpecSupervised struct {
	child    lawSpecChild
	lifetime string
}

// LawSpecSupervisor starts children (actors or supervisors) and restarts
// them after a crash: LawSpecOneForOne restarts the child that crashed,
// LawSpecOneForAll every child, LawSpecRestForOne it and those added after
// it. A child's lifetime: permanent restarts after a crash or a stop,
// transient only after a crash, temporary never. More than maxRestarts
// within period is the supervisor's own crash: its supervisor restarts all
// of its children, or, at the top, every child stops.
//
// Supervision follows OTP's strategies and restart types, so a supervision tree
// means what an Erlang programmer expects on every target.
// ref:DEC-actors-otp-supervision ref:erlang-otp-supervisors
type LawSpecSupervisor struct {
	mu          sync.Mutex
	strategy    string
	maxRestarts int
	period      time.Duration
	children    []*lawSpecSupervised
	restarts    []time.Time
	supervisor  *LawSpecSupervisor
	stopped     bool
	monitors    []func(LawSpecExit)
}

// NewLawSpecSupervisor makes a supervisor with no children.
func NewLawSpecSupervisor(strategy string, maxRestarts int, period time.Duration) *LawSpecSupervisor {
	if strategy != LawSpecOneForOne && strategy != LawSpecOneForAll && strategy != LawSpecRestForOne {
		panic(fmt.Sprintf("unknown supervision strategy %q", strategy))
	}
	return &LawSpecSupervisor{strategy: strategy, maxRestarts: maxRestarts, period: period}
}

// SuperviseActor adds a started actor, and returns it.
func (s *LawSpecSupervisor) SuperviseActor(child *LawSpecActor, lifetime string) *LawSpecActor {
	s.supervise(child, lifetime)
	return child
}

// SuperviseSupervisor adds a started supervisor, and returns it.
func (s *LawSpecSupervisor) SuperviseSupervisor(child *LawSpecSupervisor, lifetime string) *LawSpecSupervisor {
	s.supervise(child, lifetime)
	return child
}

func (s *LawSpecSupervisor) supervise(child lawSpecChild, lifetime string) {
	if lifetime != LawSpecPermanent && lifetime != LawSpecTransient && lifetime != LawSpecTemporary {
		panic(fmt.Sprintf("unknown lifetime %q", lifetime))
	}
	child.setSupervisor(s)
	s.mu.Lock()
	s.children = append(s.children, &lawSpecSupervised{child, lifetime})
	s.mu.Unlock()
}

func (s *LawSpecSupervisor) setSupervisor(parent *LawSpecSupervisor) {
	s.mu.Lock()
	s.supervisor = parent
	s.mu.Unlock()
}

func (s *LawSpecSupervisor) allowRestart() bool {
	now := time.Now()
	for len(s.restarts) > 0 && now.Sub(s.restarts[0]) > s.period {
		s.restarts = s.restarts[1:]
	}
	if len(s.restarts) >= s.maxRestarts {
		return false
	}
	s.restarts = append(s.restarts, now)
	return true
}

func (s *LawSpecSupervisor) entry(child lawSpecChild) int {
	for i, e := range s.children {
		if e.child == child {
			return i
		}
	}
	return -1
}

func (s *LawSpecSupervisor) remove(index int) {
	s.children = append(append([]*lawSpecSupervised{}, s.children[:index]...), s.children[index+1:]...)
}

// restarting (under the lock) is the children to restart for the crash of
// the child at index, or nil when the supervisor gives up.
func (s *LawSpecSupervisor) restarting(index int, cause any, crashed lawSpecChild) []*lawSpecSupervised {
	if s.allowRestart() {
		switch s.strategy {
		case LawSpecOneForAll:
			return append([]*lawSpecSupervised{}, s.children...)
		case LawSpecRestForOne:
			return append([]*lawSpecSupervised{}, s.children[index:]...)
		}
		return []*lawSpecSupervised{s.children[index]}
	}
	if s.supervisor != nil && s.supervisor.childFailed(s, cause) {
		s.restarts = nil
		return append([]*lawSpecSupervised{}, s.children...)
	}
	s.fail(crashed, cause)
	return nil
}

// childCrashed runs on the child's turn: true when it restarts now.
func (s *LawSpecSupervisor) childCrashed(child *LawSpecActor, cause any) bool {
	s.mu.Lock()
	index := s.entry(child)
	if s.stopped || index < 0 {
		s.mu.Unlock()
		return false
	}
	if s.children[index].lifetime == LawSpecTemporary {
		s.remove(index)
		s.mu.Unlock()
		return false
	}
	group := s.restarting(index, cause, child)
	s.mu.Unlock()
	if group == nil {
		return false
	}
	for _, e := range group {
		if e.child != lawSpecChild(child) {
			e.child.restartLater()
		}
	}
	child.restartNow()
	return true
}

// childFailed: a child supervisor gave up; true when it may restart its
// children.
func (s *LawSpecSupervisor) childFailed(child *LawSpecSupervisor, cause any) bool {
	s.mu.Lock()
	index := s.entry(child)
	if s.stopped || index < 0 {
		s.mu.Unlock()
		return false
	}
	if s.children[index].lifetime == LawSpecTemporary {
		s.remove(index)
		s.mu.Unlock()
		return false
	}
	group := s.restarting(index, cause, child)
	s.mu.Unlock()
	if group == nil {
		return false
	}
	for _, e := range group {
		if e.child != lawSpecChild(child) {
			e.child.restartLater()
		}
	}
	return true
}

// childStopped is true when a stopped child is permanent and restarts
// instead.
func (s *LawSpecSupervisor) childStopped(child lawSpecChild) bool {
	s.mu.Lock()
	index := s.entry(child)
	if index < 0 || s.stopped {
		s.mu.Unlock()
		return false
	}
	if s.children[index].lifetime != LawSpecPermanent {
		s.remove(index)
		s.mu.Unlock()
		return false
	}
	group := s.restarting(index, "stopped", child)
	s.mu.Unlock()
	if group == nil {
		return false
	}
	for _, e := range group {
		e.child.restartLater()
	}
	return true
}

// fail (under the lock): every child but the one crashing (which stops
// itself) stops, and so does the supervisor.
func (s *LawSpecSupervisor) fail(crashed lawSpecChild, cause any) {
	children := s.children
	s.children = nil
	s.stopped = true
	for i := len(children) - 1; i >= 0; i-- {
		other := children[i].child
		other.setSupervisor(nil)
		if other != crashed {
			other.halt()
		}
	}
	for _, m := range s.monitors {
		m(LawSpecExit{"crashed", cause})
	}
}

// restartLater: restarted by its own supervisor, every child restarts.
func (s *LawSpecSupervisor) restartLater() {
	s.mu.Lock()
	s.restarts = nil
	children := append([]*lawSpecSupervised{}, s.children...)
	s.mu.Unlock()
	for _, e := range children {
		e.child.restartLater()
	}
}

func (s *LawSpecSupervisor) halt() { s.Stop() }

// Monitor calls notify with "crashed" when the supervisor passes its
// restart limit, and "stopped" once stopped.
func (s *LawSpecSupervisor) Monitor(notify func(LawSpecExit)) {
	s.mu.Lock()
	s.monitors = append(s.monitors, notify)
	s.mu.Unlock()
}

// Stop stops every child, last added first, without restarting them.
func (s *LawSpecSupervisor) Stop() {
	s.mu.Lock()
	if s.stopped {
		s.mu.Unlock()
		return
	}
	s.stopped = true
	children := s.children
	s.children = nil
	monitors := append([]func(LawSpecExit){}, s.monitors...)
	s.mu.Unlock()
	for i := len(children) - 1; i >= 0; i-- {
		children[i].child.setSupervisor(nil)
		children[i].child.Stop()
	}
	for _, m := range monitors {
		m(LawSpecExit{"stopped", nil})
	}
}

// LawSpecCheckSupervision is the runtime's own check of crashes, links,
// monitors and supervision: every strategy, lifetime, the restart limit and
// escalation. It returns an error naming the first behaviour that differs.
func LawSpecCheckSupervision() (failure error) {
	type problem struct{ message string }
	defer func() {
		if r := recover(); r != nil {
			if p, ok := r.(problem); ok {
				failure = errors.New(p.message)
				return
			}
			panic(r)
		}
	}()
	counter := func() *LawSpecActor {
		return NewLawSpecActorWithRestart(int64(0), func(any) any { return int64(0) })
	}
	bump := func(a *LawSpecActor) {
		if _, err := a.Call(func(s any) (any, any) { return s.(int64) + 1, s.(int64) + 1 }); err != nil {
			panic(problem{"a call failed: " + err.Error()})
		}
	}
	fail := func(a *LawSpecActor) {
		_, err := a.Call(func(any) (any, any) { panic("division by zero") })
		var crashed LawSpecActorCrashed
		if !errors.As(err, &crashed) {
			panic(problem{"a failing handler did not give LawSpecActorCrashed"})
		}
	}
	state := func(a *LawSpecActor) any {
		s, err := a.State()
		if err != nil {
			return err
		}
		return s
	}
	stopped := func(a *LawSpecActor, what string) {
		if _, err := a.State(); !errors.Is(err, LawSpecActorStopped) {
			panic(problem{what + " should have stopped"})
		}
	}
	expect := func(actual, wanted []any, what string) {
		if fmt.Sprint(actual) != fmt.Sprint(wanted) {
			panic(problem{fmt.Sprintf("%s: got %v, expected %v", what, actual, wanted)})
		}
	}
	zero, one, two := int64(0), int64(1), int64(2)
	a := counter()
	bump(a)
	fail(a)
	stopped(a, "an unsupervised actor that crashed")
	sup := NewLawSpecSupervisor(LawSpecOneForOne, 3, 5*time.Second)
	x, y := sup.SuperviseActor(counter(), LawSpecPermanent), sup.SuperviseActor(counter(), LawSpecPermanent)
	bump(x)
	bump(y)
	bump(y)
	fail(x)
	expect([]any{state(x), state(y)}, []any{zero, two}, "one for one restarts only the crashed child")
	sup = NewLawSpecSupervisor(LawSpecOneForAll, 3, 5*time.Second)
	x, y = sup.SuperviseActor(counter(), LawSpecPermanent), sup.SuperviseActor(counter(), LawSpecPermanent)
	bump(x)
	bump(y)
	fail(x)
	expect([]any{state(x), state(y)}, []any{zero, zero}, "one for all restarts every child")
	sup = NewLawSpecSupervisor(LawSpecRestForOne, 3, 5*time.Second)
	x, y = sup.SuperviseActor(counter(), LawSpecPermanent), sup.SuperviseActor(counter(), LawSpecPermanent)
	z := sup.SuperviseActor(counter(), LawSpecPermanent)
	bump(x)
	bump(y)
	bump(z)
	fail(y)
	expect([]any{state(x), state(y), state(z)}, []any{one, zero, zero}, "rest for one restarts the child and later ones")
	sup = NewLawSpecSupervisor(LawSpecOneForOne, 3, 5*time.Second)
	t := sup.SuperviseActor(counter(), LawSpecTemporary)
	fail(t)
	stopped(t, "a temporary child that crashed")
	sup = NewLawSpecSupervisor(LawSpecOneForOne, 3, 5*time.Second)
	p, q := sup.SuperviseActor(counter(), LawSpecPermanent), sup.SuperviseActor(counter(), LawSpecTransient)
	bump(p)
	p.Stop()
	expect([]any{state(p)}, []any{zero}, "a permanent child restarts after a stop")
	q.Stop()
	stopped(q, "a transient child that was stopped")
	var eventsMu sync.Mutex
	events := []string{}
	sup = NewLawSpecSupervisor(LawSpecOneForOne, 2, 10*time.Second)
	sup.Monitor(func(e LawSpecExit) { eventsMu.Lock(); events = append(events, e.Reason); eventsMu.Unlock() })
	x, y = sup.SuperviseActor(counter(), LawSpecPermanent), sup.SuperviseActor(counter(), LawSpecPermanent)
	fail(x)
	fail(x)
	fail(x)
	stopped(y, "a child of a supervisor past its restart limit")
	eventsMu.Lock()
	expect([]any{strings.Join(events, ",")}, []any{"crashed"}, "a supervisor past its limit tells its monitors")
	eventsMu.Unlock()
	outer := NewLawSpecSupervisor(LawSpecOneForOne, 5, 10*time.Second)
	inner := outer.SuperviseSupervisor(NewLawSpecSupervisor(LawSpecOneForOne, 1, 10*time.Second), LawSpecPermanent)
	x, y = inner.SuperviseActor(counter(), LawSpecPermanent), inner.SuperviseActor(counter(), LawSpecPermanent)
	bump(y)
	fail(x)
	fail(x)
	expect([]any{state(x), state(y)}, []any{zero, zero}, "a supervisor past its limit is restarted by its own")
	seen := make(chan string, 4)
	a, b := counter(), counter()
	a.Link(b)
	b.Monitor(func(e LawSpecExit) { seen <- e.Reason })
	fail(a)
	select {
	case reason := <-seen:
		expect([]any{reason}, []any{"crashed"}, "a monitor hears of a crash")
	case <-time.After(time.Second):
		panic(problem{"a monitor heard nothing of a linked crash"})
	}
	stopped(b, "an unsupervised actor linked to one that crashed")
	sup = NewLawSpecSupervisor(LawSpecOneForOne, 10, 5*time.Second)
	a, b = sup.SuperviseActor(counter(), LawSpecPermanent), sup.SuperviseActor(counter(), LawSpecPermanent)
	c := sup.SuperviseActor(counter(), LawSpecPermanent)
	a.Link(b)
	b.Link(c)
	c.Link(a)
	bump(a)
	bump(b)
	bump(c)
	fail(a)
	restarts := func() int { sup.mu.Lock(); defer sup.mu.Unlock(); return len(sup.restarts) }
	for i := 0; i < 100 && restarts() < 3; i++ {
		time.Sleep(10 * time.Millisecond)
	}
	time.Sleep(50 * time.Millisecond)
	expect([]any{state(a), state(b), state(c), restarts()}, []any{zero, zero, zero, 3}, "a crash crosses each link once")
	return nil
}

// LawSpecMailbox is a queue with many senders and one receiver: the channel
// form of an actor. A goroutine that loops over Receive and answers each
// message is an actor written by hand; Send never waits.
type LawSpecMailbox struct {
	mu     sync.Mutex
	items  []any
	closed bool
	ready  chan struct{}
}

// NewLawSpecMailbox makes an empty mailbox.
func NewLawSpecMailbox() *LawSpecMailbox {
	return &LawSpecMailbox{ready: make(chan struct{}, 1)}
}

// Send queues value; it fails once the mailbox is closed.
func (m *LawSpecMailbox) Send(value any) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.closed {
		return LawSpecActorStopped
	}
	m.items = append(m.items, value)
	select {
	case m.ready <- struct{}{}:
	default:
	}
	return nil
}

// Receive is the next message, waiting up to timeout (forever when zero or
// negative); it fails on timeout, or once closed and empty.
func (m *LawSpecMailbox) Receive(timeout time.Duration) (any, error) {
	var deadline <-chan time.Time
	if timeout > 0 {
		timer := time.NewTimer(timeout)
		defer timer.Stop()
		deadline = timer.C
	}
	for {
		m.mu.Lock()
		if len(m.items) > 0 {
			value := m.items[0]
			m.items[0] = nil
			m.items = m.items[1:]
			if len(m.items) > 0 {
				select {
				case m.ready <- struct{}{}:
				default:
				}
			}
			m.mu.Unlock()
			return value, nil
		}
		if m.closed {
			m.mu.Unlock()
			return nil, LawSpecActorStopped
		}
		m.mu.Unlock()
		select {
		case <-m.ready:
		case <-deadline:
			return nil, errors.New("no message arrived in time")
		}
	}
}

// ReceiveWithin is the Mailbox ability's receive ... within d: the next
// message and true, or false when none arrives within d. With any Clock
// handler but the default real one (as lawspec.time's RegisterClock tells)
// it waits no real time: it takes a message already sent, or lets d pass on
// that clock and gives none. Otherwise it waits up to d. It fails with
// LawSpecActorStopped once closed and empty.
func (m *LawSpecMailbox) ReceiveWithin(within time.Duration, clock ...any) (any, bool, error) {
	var read *lawSpecAbilityClock
	if len(clock) > 0 {
		read = lsAbilityClockOf(clock[0])
	}
	if read != nil && read.virtual {
		if value, ok, err := m.take(); ok || err != nil {
			return value, ok, err
		}
		read.Sleep(within.Microseconds())
		return nil, false, nil
	}
	if within <= 0 {
		return m.take()
	}
	timer := time.NewTimer(within)
	defer timer.Stop()
	for {
		if value, ok, err := m.take(); ok || err != nil {
			return value, ok, err
		}
		select {
		case <-m.ready:
		case <-timer.C:
			return m.take()
		}
	}
}

// take is the next message without waiting, if there is one.
func (m *LawSpecMailbox) take() (any, bool, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if len(m.items) > 0 {
		value := m.items[0]
		m.items[0] = nil
		m.items = m.items[1:]
		if len(m.items) > 0 {
			select {
			case m.ready <- struct{}{}:
			default:
			}
		}
		return value, true, nil
	}
	if m.closed {
		return nil, false, LawSpecActorStopped
	}
	return nil, false, nil
}

// Close refuses further messages; those already sent can still be received.
func (m *LawSpecMailbox) Close() {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.closed = true
	select {
	case m.ready <- struct{}{}:
	default:
	}
}

func lsFields(v LawSpecValue) []LawSpecValue { return v.Data.(lawSpecData).fields }

// lsStepModel is the model's next state and result, panicking lawSpecInvalid
// when the command is not allowed (its precondition or reference fails).
func lsStepModel(c *lawSpecModelCommand, symbols map[string]*LawSpecSymbol, args []LawSpecValue, state LawSpecValue) (next, result LawSpecValue) {
	out := func() (out LawSpecValue) {
		defer func() {
			if recover() != nil {
				panic(lawSpecInvalid{})
			}
		}()
		if c.when != nil && !lsTruth(c.when(symbols, []LawSpecValue{state})) {
			panic(lawSpecInvalid{})
		}
		full := append(append([]LawSpecValue{}, args...), state)
		return c.reference(symbols, full)
	}()
	if c.unit {
		return out, lsAbsent("Unit")
	}
	fields := lsFields(out)
	return fields[1], fields[0]
}

// lsAllowed is whether f runs without panicking lawSpecInvalid; other
// panics propagate.
func lsAllowed(f func()) (allowed bool) {
	defer func() {
		if r := recover(); r != nil {
			if _, ok := r.(lawSpecInvalid); !ok {
				panic(r)
			}
			allowed = false
		}
	}()
	f()
	return true
}

// startState is the model's start state, panicking lawSpecInvalid on failure.
func (m *lawSpecMachine) startState(symbols map[string]*LawSpecSymbol, args []LawSpecValue) (state LawSpecValue) {
	defer func() {
		if recover() != nil {
			panic(lawSpecInvalid{})
		}
	}()
	return m.startModel(symbols, args)
}

// simulate is whether the model allows the run.
func (m *lawSpecMachine) simulate(run lawSpecModelRun) bool {
	symbols := map[string]*LawSpecSymbol{}
	return lsAllowed(func() {
		state := m.startState(symbols, run.start)
		indices := append([]int64{}, m.startIndices...)
		for _, step := range run.steps {
			c := &m.steps[step.index]
			if !c.admits(indices) {
				panic(lawSpecInvalid{})
			}
			state, _ = lsStepModel(c, symbols, step.args, state)
			indices = c.shifted(indices)
		}
	})
}

func (m *lawSpecMachine) generateRun(random *LawSpecSplitMix64, length, size int64, crashes bool) lawSpecModelRun {
	symbols := map[string]*LawSpecSymbol{}
	start := []LawSpecValue{}
	for _, d := range m.startArguments {
		start = append(start, m.values.generate(d, random, size))
	}
	run := lawSpecModelRun{start: start}
	var state LawSpecValue
	if !lsAllowed(func() { state = m.startState(symbols, start) }) {
		return run
	}
	indices := append([]int64{}, m.startIndices...)
	for ; length > 0; length-- {
		allowed := []int{}
		for i := range m.commands {
			if m.commands[i].admits(indices) {
				allowed = append(allowed, i)
			}
		}
		if len(allowed) == 0 {
			break
		}
		index := allowed[random.Below(uint64(len(allowed)))]
		// One step in eight of an actor's run is a crash.
		if crashes && len(m.steps) > len(m.commands) && random.Below(8) == 0 {
			index = len(m.commands)
		}
		c := &m.steps[index]
		args := []LawSpecValue{}
		for _, d := range c.arguments {
			args = append(args, m.values.generate(d, random, size))
		}
		if !lsAllowed(func() { state, _ = lsStepModel(c, symbols, args, state) }) {
			continue
		}
		run.steps = append(run.steps, lawSpecModelStep{index, args})
		indices = c.shifted(indices)
	}
	return run
}

// execute is the failing step's number and what went wrong, or failed false
// when the system agrees with the model along the run.
func (m *lawSpecMachine) execute(run lawSpecModelRun) (step int, message string, failed bool) {
	symbols := map[string]*LawSpecSymbol{}
	defer func() {
		if r := recover(); r != nil {
			failed = true
			if _, ok := r.(lawSpecInvalid); ok {
				message = "the model does not allow this step"
			} else {
				message = fmt.Sprintf("raised panic: %v", r)
			}
		}
	}()
	state := m.startRun(symbols, run.start)
	expected := m.startModel(symbols, run.start)
	if message, failed = m.checkState(symbols, state, expected); failed {
		return
	}
	for _, s := range run.steps {
		step++
		c := &m.steps[s.index]
		full := append(append([]LawSpecValue{}, s.args[:c.state]...), state)
		full = append(full, s.args[c.state:]...)
		out := c.run(symbols, full)
		var result LawSpecValue
		if m.shared {
			result = out
		} else {
			fields := lsFields(out)
			if c.unit {
				result = lsAbsent("Unit")
			} else {
				result = fields[0]
			}
			state = fields[len(fields)-1]
		}
		var wanted LawSpecValue
		expected, wanted = lsStepModel(c, symbols, s.args, expected)
		if !c.unit && lsCompareValues(result, wanted) != 0 {
			return step, fmt.Sprintf("returned %s; the model returns %s", lsRender(result), lsRender(wanted)), true
		}
		if message, failed = m.checkState(symbols, state, expected); failed {
			return
		}
	}
	return 0, "", false
}

func (m *lawSpecMachine) checkState(symbols map[string]*LawSpecSymbol, state, expected LawSpecValue) (string, bool) {
	if m.abstract != nil {
		actual := m.abstract(symbols, []LawSpecValue{state})
		if lsCompareValues(actual, expected) != 0 {
			return fmt.Sprintf("the state is %s; the model is %s", lsRender(actual), lsRender(expected)), true
		}
	}
	for i, kind := range m.invariantKinds {
		if i >= len(m.invariants) {
			break
		}
		subject := state
		if kind == "model" {
			subject = expected
		}
		if !lsTruth(m.invariants[i](symbols, []LawSpecValue{subject})) {
			return "an invariant on the " + kind + " fails", true
		}
	}
	return "", false
}

// shrinkCandidates calls yield on each candidate in order until it returns false.
func (m *lawSpecMachine) shrinkCandidates(run lawSpecModelRun, yield func(lawSpecModelRun) bool) {
	steps := run.steps
	n := len(steps)
	for size := n / 2; size >= 1; size /= 2 {
		for begin := 0; begin < n; begin += size {
			kept := append([]lawSpecModelStep{}, steps[:begin]...)
			kept = append(kept, steps[min(begin+size, n):]...)
			if !yield(lawSpecModelRun{run.start, kept}) {
				return
			}
		}
	}
	for k, s := range steps {
		c := &m.steps[s.index]
		for j := 0; j < len(c.arguments) && j < len(s.args); j++ {
			for _, candidate := range m.values.shrink(c.arguments[j], s.args[j]) {
				args := append([]LawSpecValue{}, s.args...)
				args[j] = candidate
				changed := append([]lawSpecModelStep{}, steps...)
				changed[k] = lawSpecModelStep{s.index, args}
				if !yield(lawSpecModelRun{run.start, changed}) {
					return
				}
			}
		}
	}
	for j := 0; j < len(m.startArguments) && j < len(run.start); j++ {
		for _, candidate := range m.values.shrink(m.startArguments[j], run.start[j]) {
			start := append([]LawSpecValue{}, run.start...)
			start[j] = candidate
			if !yield(lawSpecModelRun{start, steps}) {
				return
			}
		}
	}
}

func (m *lawSpecMachine) shrinkRun(run lawSpecModelRun, step int, message string, budget int) (lawSpecModelRun, int, string) {
	for budget > 0 {
		found := false
		m.shrinkCandidates(run, func(candidate lawSpecModelRun) bool {
			budget--
			if budget <= 0 {
				return false
			}
			if !m.simulate(candidate) {
				return true
			}
			if k, text, failed := m.execute(candidate); failed {
				run, step, message, found = candidate, k, text, true
				return false
			}
			return true
		})
		if !found {
			break
		}
	}
	return run, step, message
}

func (m *lawSpecMachine) describeRun(run lawSpecModelRun) string {
	render := func(values []LawSpecValue) string {
		parts := make([]string, len(values))
		for i, v := range values {
			parts[i] = lsRender(v)
		}
		return strings.Join(parts, ", ")
	}
	parts := []string{"start(" + render(run.start) + ")"}
	for _, s := range run.steps {
		parts = append(parts, m.steps[s.index].name+"("+render(s.args)+")")
	}
	return strings.Join(parts, "; ")
}

// LawSpecCheckModel checks the system against its model on generated runs
// (100 cases of at most 20 commands, seeded from LAWSPEC_SEED); a failure is
// an error naming the shortest failing run found.
func LawSpecCheckModel(model LawSpecModel) error {
	var seed uint64
	if text := os.Getenv("LAWSPEC_SEED"); text != "" {
		parsed, err := strconv.ParseUint(text, 10, 64)
		if err != nil {
			return fmt.Errorf("invalid LAWSPEC_SEED %q", text)
		}
		seed = parsed
	}
	return lsCheckModel(model, 100, 20, 2000, seed)
}

func lsCheckModel(model LawSpecModel, cases, maxLength, maxShrinks int, seed uint64) error {
	m := lsNewMachine(model)
	random := &LawSpecSplitMix64{seed}
	for c := 0; c < cases; c++ {
		length := int64(random.Below(uint64(maxLength + 1)))
		run := m.generateRun(random, length, int64(1+c%8), true)
		if step, message, failed := m.execute(run); failed {
			run, step, message = m.shrinkRun(run, step, message, maxShrinks)
			return fmt.Errorf("model %s fails at step %d of %s: %s", m.name, step, m.describeRun(run), message)
		}
	}
	return nil
}

// Parallel runs of a shared model. A case is a sequential prefix and one
// branch per thread, generated so that the model allows every interleaving
// of the branches (a search over each thread's position and the model state,
// memoized). The system runs the branches at the same time, each call's start
// and return recorded on one counter, with random yields and short sleeps
// around calls to shake out rare schedules. The history must be
// linearizable: some interleaving that keeps every call after those that
// returned before it started must give every result the model gives and
// leave the state it leaves (a Wing-Gong search, memoized on the same
// positions and model state). Each case runs several times.

const (
	lawSpecParallelThreads = 3
	lawSpecParallelBranch  = 5
)

type lawSpecParallelCase struct {
	prefix   lawSpecModelRun
	branches [][]lawSpecModelStep
}

// lawSpecCall is a branch call's history: when it started and returned on
// a shared counter, and its result.
type lawSpecCall struct {
	called, returned int64
	result           LawSpecValue
}

// simulateState is the model's state after the run, panicking lawSpecInvalid
// when the model does not allow it.
func (m *lawSpecMachine) simulateState(symbols map[string]*LawSpecSymbol, run lawSpecModelRun) LawSpecValue {
	state := m.startState(symbols, run.start)
	indices := append([]int64{}, m.startIndices...)
	for _, step := range run.steps {
		c := &m.commands[step.index]
		if !c.admits(indices) {
			panic(lawSpecInvalid{})
		}
		state, _ = lsStepModel(c, symbols, step.args, state)
		indices = c.shifted(indices)
	}
	return state
}

// lsAdvanced is the positions with thread i one step further.
func lsAdvanced(positions []int, i int) []int {
	next := append([]int{}, positions...)
	next[i]++
	return next
}

// lsPositionsKey is the memo key of the threads' positions and a model state.
func lsPositionsKey(positions []int, state LawSpecValue) string {
	return fmt.Sprint(positions) + "|" + lsRender(state)
}

// parallelAllowed is whether the model allows the prefix then every
// interleaving of the branches.
func (m *lawSpecMachine) parallelAllowed(c lawSpecParallelCase) bool {
	symbols := map[string]*LawSpecSymbol{}
	var start LawSpecValue
	if !lsAllowed(func() { start = m.simulateState(symbols, c.prefix) }) {
		return false
	}
	seen := map[string]bool{}
	var visit func(positions []int, state LawSpecValue) bool
	visit = func(positions []int, state LawSpecValue) bool {
		key := lsPositionsKey(positions, state)
		if seen[key] {
			return true
		}
		seen[key] = true
		for i, branch := range c.branches {
			k := positions[i]
			if k < len(branch) {
				step := branch[k]
				var after LawSpecValue
				if !lsAllowed(func() { after, _ = lsStepModel(&m.commands[step.index], symbols, step.args, state) }) {
					return false
				}
				if !visit(lsAdvanced(positions, i), after) {
					return false
				}
			}
		}
		return true
	}
	return visit(make([]int, len(c.branches)), start)
}

func (m *lawSpecMachine) generateBranch(random *LawSpecSplitMix64, state LawSpecValue, length, size int64) []lawSpecModelStep {
	symbols := map[string]*LawSpecSymbol{}
	steps := []lawSpecModelStep{}
	for ; length > 0; length-- {
		index := int(random.Below(uint64(len(m.commands))))
		c := &m.commands[index]
		args := []LawSpecValue{}
		for _, d := range c.arguments {
			args = append(args, m.values.generate(d, random, size))
		}
		if !lsAllowed(func() { state, _ = lsStepModel(c, symbols, args, state) }) {
			continue
		}
		steps = append(steps, lawSpecModelStep{index, args})
	}
	return steps
}

func (m *lawSpecMachine) generateParallel(random *LawSpecSplitMix64, size int64, threads, branchLength int) lawSpecParallelCase {
	prefix := m.generateRun(random, int64(random.Below(4)), size, false)
	c := lawSpecParallelCase{prefix: prefix, branches: make([][]lawSpecModelStep, threads)}
	for i := range c.branches {
		c.branches[i] = []lawSpecModelStep{}
	}
	var state LawSpecValue
	if !lsAllowed(func() { state = m.simulateState(map[string]*LawSpecSymbol{}, prefix) }) {
		return c
	}
	for i := range c.branches {
		c.branches[i] = m.generateBranch(random, state, 1+int64(random.Below(uint64(branchLength))), size)
	}
	// Drop the last step of the longest branch (the first, among equals)
	// until every interleaving is allowed.
	for !m.parallelAllowed(c) {
		longest := 0
		for i := range c.branches {
			if len(c.branches[i]) > len(c.branches[longest]) {
				longest = i
			}
		}
		c.branches[longest] = c.branches[longest][:len(c.branches[longest])-1]
	}
	return c
}

func lsWithState(c *lawSpecModelCommand, args []LawSpecValue, state LawSpecValue) []LawSpecValue {
	full := append(append([]LawSpecValue{}, args[:c.state]...), state)
	return append(full, args[c.state:]...)
}

// lsPerturb is nothing, a yield, or a sleep of 10 or 100 microseconds.
func lsPerturb(random *LawSpecSplitMix64) {
	switch random.Below(4) {
	case 1:
		goruntime.Gosched()
	case 2:
		time.Sleep(10 * time.Microsecond)
	case 3:
		time.Sleep(100 * time.Microsecond)
	}
}

// executeParallel is what went wrong, or failed false when the history is
// linearizable.
func (m *lawSpecMachine) executeParallel(pc lawSpecParallelCase, shake uint64) (message string, failed bool) {
	symbols := map[string]*LawSpecSymbol{}
	var state LawSpecValue
	prefixFailure := func() (message string) {
		defer func() {
			if r := recover(); r != nil {
				message = fmt.Sprintf("the prefix raised panic: %v", r)
			}
		}()
		state = m.startRun(symbols, pc.prefix.start)
		for _, s := range pc.prefix.steps {
			c := &m.commands[s.index]
			c.run(symbols, lsWithState(c, s.args, state))
		}
		return ""
	}()
	if prefixFailure != "" {
		return prefixFailure, true
	}
	var clock int64
	var lock sync.Mutex
	history := make([][]lawSpecCall, len(pc.branches))
	errors := []string{}
	var group sync.WaitGroup
	for i := range pc.branches {
		history[i] = make([]lawSpecCall, len(pc.branches[i]))
		group.Add(1)
		go func(i int) {
			defer group.Done()
			own := map[string]*LawSpecSymbol{}
			random := &LawSpecSplitMix64{shake ^ (uint64(i+1) * 0x9E3779B97F4A7C15)}
			for k, s := range pc.branches[i] {
				c := &m.commands[s.index]
				full := lsWithState(c, s.args, state)
				lsPerturb(random)
				called := atomic.AddInt64(&clock, 1)
				result := func() (result LawSpecValue) {
					defer func() {
						if r := recover(); r != nil {
							lock.Lock()
							errors = append(errors, fmt.Sprintf("%s raised panic: %v", c.name, r))
							lock.Unlock()
							result = LawSpecValue{}
						}
					}()
					return c.run(own, full)
				}()
				history[i][k] = lawSpecCall{called, atomic.AddInt64(&clock, 1), result}
				lsPerturb(random)
			}
		}(i)
	}
	group.Wait()
	if len(errors) > 0 {
		return errors[0], true
	}
	expected := m.simulateState(symbols, pc.prefix)
	var final *LawSpecValue
	if m.abstract != nil {
		actual := m.abstract(symbols, []LawSpecValue{state})
		final = &actual
	}
	if m.linearizable(symbols, pc.branches, history, expected, final, state) {
		return "", false
	}
	observed := []string{}
	for i := range pc.branches {
		for k, s := range pc.branches[i] {
			observed = append(observed, fmt.Sprintf("%s: %s() returned %s", lsBranchName(i), m.commands[s.index].name, lsRender(history[i][k].result)))
		}
	}
	return "no order of the parallel calls agrees with the model (" + strings.Join(observed, "; ") + ")", true
}

func lsBranchName(i int) string {
	return string(rune('A' + i))
}

// linearizable is whether the history linearizes, with the final state and
// invariants the model gives. For a set or map whose every call touches one
// key, each key's calls are linearized separately (the keys are
// independent), one group after another in the order of their rendered keys;
// otherwise all calls at once.
func (m *lawSpecMachine) linearizable(symbols map[string]*LawSpecSymbol, branches [][]lawSpecModelStep, history [][]lawSpecCall, expected LawSpecValue, final *LawSpecValue, state LawSpecValue) bool {
	finish := func(model LawSpecValue) bool {
		if final != nil && lsCompareValues(*final, model) != 0 {
			return false
		}
		for i, kind := range m.invariantKinds {
			if i >= len(m.invariants) {
				break
			}
			subject := state
			if kind == "model" {
				subject = model
			}
			if !lsTruth(m.invariants[i](symbols, []LawSpecValue{subject})) {
				return false
			}
		}
		return true
	}
	if !m.perKey {
		return m.linearize(symbols, branches, history, expected, finish)
	}
	type group struct {
		branches [][]lawSpecModelStep
		history  [][]lawSpecCall
	}
	groups := map[string]*group{}
	keys := []string{}
	for i, branch := range branches {
		for k, step := range branch {
			key := lsRender(step.args[m.commands[step.index].key])
			g, ok := groups[key]
			if !ok {
				g = &group{make([][]lawSpecModelStep, len(branches)), make([][]lawSpecCall, len(branches))}
				groups[key] = g
				keys = append(keys, key)
			}
			g.branches[i] = append(g.branches[i], step)
			g.history[i] = append(g.history[i], history[i][k])
		}
	}
	sort.Strings(keys)
	model := expected
	for _, key := range keys {
		g := groups[key]
		var end LawSpecValue
		found := false
		record := func(state LawSpecValue) bool {
			if !found {
				end, found = state, true
			}
			return true
		}
		if !m.linearize(symbols, g.branches, g.history, model, record) {
			return false
		}
		model = end
	}
	return finish(model)
}

// linearize is a Wing-Gong search: linearize, next, a call no pending call
// on another thread returned before; memoized on positions and the model
// state. finish judges each complete order's final model state.
// lsConsistent names each consistency for failure messages.
var lsConsistent = map[string]string{"linearizable": "linearizable", "sequential": "sequentially consistent",
	"causal": "causally consistent", "eventual": "eventually consistent"}

// linearize is a Wing-Gong search; with weaker consistency, sequential drops
// real time (each thread's own order remains), causal checks each thread's
// results alone (threads that never message each other see only their own
// calls), and eventual checks no results, only the final state.
//
// A shared model promises linearizability unless it names a weaker consistency.
// ref:wing-gong-linearizability ref:herlihy-wing-linearizability
// ref:DEC-stateful-models-linearizability
func (m *lawSpecMachine) linearize(symbols map[string]*LawSpecSymbol, branches [][]lawSpecModelStep, history [][]lawSpecCall, expected LawSpecValue, finish func(LawSpecValue) bool) bool {
	mode := m.consistency
	if mode == "causal" {
		for i, branch := range branches {
			state := expected
			for k, step := range branch {
				c := &m.commands[step.index]
				var after, wanted LawSpecValue
				if !lsAllowed(func() { after, wanted = lsStepModel(c, symbols, step.args, state) }) {
					return false
				}
				if !c.unit && lsCompareValues(history[i][k].result, wanted) != 0 {
					return false
				}
				state = after
			}
		}
		return true
	}
	seen := map[string]bool{}
	var visit func(positions []int, model LawSpecValue) bool
	visit = func(positions []int, model LawSpecValue) bool {
		key := lsPositionsKey(positions, model)
		if seen[key] {
			return false
		}
		seen[key] = true
		done := true
		for i := range branches {
			if positions[i] < len(branches[i]) {
				done = false
			}
		}
		if done {
			return finish(model)
		}
		for i, branch := range branches {
			k := positions[i]
			if k == len(branch) {
				continue
			}
			called := history[i][k].called
			blocked := false
			for j := range branches {
				if mode != "linearizable" {
					break
				}
				if j != i && positions[j] < len(branches[j]) && history[j][positions[j]].returned < called {
					blocked = true
					break
				}
			}
			if blocked {
				continue
			}
			step := branch[k]
			c := &m.commands[step.index]
			var after, wanted LawSpecValue
			if !lsAllowed(func() { after, wanted = lsStepModel(c, symbols, step.args, model) }) {
				continue
			}
			if mode != "eventual" && !c.unit && lsCompareValues(history[i][k].result, wanted) != 0 {
				continue
			}
			if visit(lsAdvanced(positions, i), after) {
				return true
			}
		}
		return false
	}
	return visit(make([]int, len(branches)), expected)
}

func (m *lawSpecMachine) parallelFails(c lawSpecParallelCase, repeats int, shake uint64) (string, bool) {
	for attempt := 0; attempt < repeats; attempt++ {
		if message, failed := m.executeParallel(c, shake+uint64(attempt)); failed {
			return message, true
		}
	}
	return "", false
}

func (m *lawSpecMachine) shrinkParallel(pc lawSpecParallelCase, failure string, repeats, budget int, shake uint64) (lawSpecParallelCase, string) {
	for budget > 0 {
		candidates := []lawSpecParallelCase{}
		steps := pc.prefix.steps
		for k := range steps {
			kept := append(append([]lawSpecModelStep{}, steps[:k]...), steps[k+1:]...)
			candidates = append(candidates, lawSpecParallelCase{lawSpecModelRun{pc.prefix.start, kept}, pc.branches})
		}
		for i := range pc.branches {
			for k := range pc.branches[i] {
				shorter := append([][]lawSpecModelStep{}, pc.branches...)
				b := pc.branches[i]
				shorter[i] = append(append([]lawSpecModelStep{}, b[:k]...), b[k+1:]...)
				candidates = append(candidates, lawSpecParallelCase{pc.prefix, shorter})
			}
		}
		// Then smaller arguments, branch by branch, step by step.
		for i := range pc.branches {
			for k, s := range pc.branches[i] {
				c := &m.commands[s.index]
				for a := 0; a < len(c.arguments) && a < len(s.args); a++ {
					for _, candidate := range m.values.shrink(c.arguments[a], s.args[a]) {
						args := append([]LawSpecValue{}, s.args...)
						args[a] = candidate
						changed := append([][]lawSpecModelStep{}, pc.branches...)
						b := append([]lawSpecModelStep{}, pc.branches[i]...)
						b[k] = lawSpecModelStep{s.index, args}
						changed[i] = b
						candidates = append(candidates, lawSpecParallelCase{pc.prefix, changed})
					}
				}
			}
		}
		exhausted := true
		for _, candidate := range candidates {
			budget--
			if budget <= 0 {
				exhausted = false
				break
			}
			if !m.parallelAllowed(candidate) {
				continue
			}
			if message, failed := m.parallelFails(candidate, repeats, shake); failed {
				pc, failure, exhausted = candidate, message, false
				break
			}
		}
		if exhausted {
			break
		}
	}
	return pc, failure
}

func (m *lawSpecMachine) describeParallel(pc lawSpecParallelCase) string {
	describe := func(steps []lawSpecModelStep) string {
		parts := []string{}
		for _, s := range steps {
			args := make([]string, len(s.args))
			for j, a := range s.args {
				args[j] = lsRender(a)
			}
			parts = append(parts, m.commands[s.index].name+"("+strings.Join(args, ", ")+")")
		}
		if len(parts) == 0 {
			return "nothing"
		}
		return strings.Join(parts, "; ")
	}
	parts := make([]string, len(pc.branches))
	for i, b := range pc.branches {
		parts[i] = lsBranchName(i) + ": " + describe(b)
	}
	last := len(parts) - 1
	return fmt.Sprintf("%s, then %s and %s at the same time", m.describeRun(pc.prefix), strings.Join(parts[:last], ", "), parts[last])
}

// LawSpecCheckModelParallel checks a shared model's histories under
// concurrency (50 cases of 3 threads, each run 10 times, seeded from
// LAWSPEC_SEED); a failure is an error naming the smallest failing case
// found.
func LawSpecCheckModelParallel(model LawSpecModel) error {
	var seed uint64
	if text := os.Getenv("LAWSPEC_SEED"); text != "" {
		parsed, err := strconv.ParseUint(text, 10, 64)
		if err != nil {
			return fmt.Errorf("invalid LAWSPEC_SEED %q", text)
		}
		seed = parsed
	}
	return lsCheckModelParallel(model, 50, 10, 300, lawSpecParallelThreads, lawSpecParallelBranch, seed)
}

func lsCheckModelParallel(model LawSpecModel, cases, repeats, maxShrinks, threads, branchLength int, seed uint64) error {
	m := lsNewMachine(model)
	random := &LawSpecSplitMix64{seed ^ 0x5BD1E995}
	for n := 0; n < cases; n++ {
		c := m.generateParallel(random, int64(1+n%8), threads, branchLength)
		shake := random.Next()
		if failure, failed := m.parallelFails(c, repeats, shake); failed {
			c, failure = m.shrinkParallel(c, failure, max(2, repeats/2), maxShrinks, shake)
			return fmt.Errorf("model %s is not %s: %s: %s", m.name, lsConsistent[m.consistency], m.describeParallel(c), failure)
		}
	}
	return nil
}

// Scenarios: processes that drive a shared model's commands at the same time
// and talk over channels (see LawSpec.Core.Program for the spec). Each
// channel has a queue per direction; a process holds an end of a channel as
// (channel, side), the first branch of a par to use a channel taking side 0.
// A channel end sent over a channel moves to the receiver. Every command's
// call and return are stamped on one counter; the history must linearize
// against the model, and every expect must hold, on each of many schedules.

// lawSpecScenarioChannel is a scenario channel: in memory, or two
// endpoints on a faulty network.
type lawSpecScenarioChannel interface {
	send(side int, value any)
	// receive is the next value for side, lawSpecGone{} once the other side
	// has ended, or ok false after waiting too long.
	receive(side int) (value any, ok bool)
	gone(side int)
}

type lawSpecChannel struct {
	queues [2]chan any
	mu     sync.Mutex
	ended  [2]bool
}

func (c *lawSpecChannel) receive(side int) (any, bool) {
	select {
	case value := <-c.queues[1-side]:
		if _, gone := value.(lawSpecGone); gone {
			c.queues[1-side] <- value
		}
		return value, true
	case <-time.After(5 * time.Second):
		return nil, false
	}
}

func lsNewChannel() *lawSpecChannel {
	return &lawSpecChannel{queues: [2]chan any{make(chan any, 4096), make(chan any, 4096)}}
}

// lawSpecGone follows the last value a process that has ended sent.
type lawSpecGone struct{}

// send queues value from side; a channel end sent to a process that has
// ended is given up.
func (c *lawSpecChannel) send(side int, value any) {
	c.mu.Lock()
	end, isEnd := value.(lawSpecEnd)
	if !(c.ended[1-side] && isEnd) {
		c.queues[side] <- value
		c.mu.Unlock()
		return
	}
	c.mu.Unlock()
	end.channel.gone(end.side)
}

// gone: side's process has ended. The other side's receives that find
// nothing more fail instead of waiting, and channel ends on their way to
// side are given up too.
func (c *lawSpecChannel) gone(side int) {
	stranded := []lawSpecEnd{}
	c.mu.Lock()
	if c.ended[side] {
		c.mu.Unlock()
		return
	}
	c.ended[side] = true
	c.queues[side] <- lawSpecGone{}
drain:
	for {
		select {
		case value := <-c.queues[1-side]:
			if end, ok := value.(lawSpecEnd); ok {
				stranded = append(stranded, end)
			} else if _, ok := value.(lawSpecGone); ok {
				c.queues[1-side] <- value
				break drain
			}
		default:
			break drain
		}
	}
	c.mu.Unlock()
	for _, end := range stranded {
		end.channel.gone(end.side)
	}
}

// lsScenarioProcesses is every process of a par (by its form), outermost
// and first first, not counting or else.
func lsScenarioProcesses(acts []any, found [][]any) [][]any {
	for _, a := range acts {
		act := a.([]any)
		if lsAtom(act[0]) == "par" {
			for _, b := range act[1:] {
				branch := b.([]any)
				found = append(found, branch)
				found = lsScenarioProcesses(branch[1:], found)
			}
		}
	}
	return found
}

// lawSpecEnd is a channel end in transit or held by a process.
type lawSpecEnd struct {
	channel lawSpecScenarioChannel
	side    int
}

// lawSpecNetScenarioChannel is a scenario channel whose two sides are
// endpoints on two nodes of a faulty in-memory network. A channel end sent
// over it travels as its name, and the receiver uses the end where it is
// (its owner).
type lawSpecNetScenarioChannel struct {
	name     string
	nodes    [2]*LawSpecNode
	ends     [2]*LawSpecNetEndpoint
	mu       sync.Mutex
	done     [2]bool
	registry map[string]*lawSpecNetScenarioChannel
}

func lsNewNetScenarioChannel(network *LawSpecMemoryNetwork, name string, steps []LawSpecWireStep, values lawSpecValues, registry map[string]*lawSpecNetScenarioChannel) *lawSpecNetScenarioChannel {
	c := &lawSpecNetScenarioChannel{name: name, registry: registry}
	wire := func(flip bool) []LawSpecWireStep {
		out := []LawSpecWireStep{}
		for _, s := range steps {
			d := s.Descriptor
			if form, ok := d.([]any); ok && lsAtom(form[0]) == "end" {
				d = []any{"text"}
			}
			out = append(out, LawSpecWireStep{s.Sends != flip, d})
		}
		return out
	}
	for side := 0; side < 2; side++ {
		c.nodes[side] = NewLawSpecNode(network.InsecureTransportForTests(fmt.Sprintf("%s-%d", name, side)))
	}
	c.ends[0], _ = c.nodes[0].Listen(name, wire(false), values, 5*time.Second)
	c.ends[1], _ = c.nodes[1].Dial(c.nodes[0].Address()+"/"+name, wire(true), values, 5*time.Second)
	registry[name] = c
	return c
}

func (c *lawSpecNetScenarioChannel) send(side int, value any) {
	if end, ok := value.(lawSpecEnd); ok {
		net := end.channel.(*lawSpecNetScenarioChannel)
		value = LawSpecValue{"Text", lsTextUnits(fmt.Sprintf("%s#%d", net.name, end.side))}
	}
	c.ends[side].Send(side, value)
}

func (c *lawSpecNetScenarioChannel) receive(side int) (any, bool) {
	value, err := c.ends[side].ReceiveWithin(5 * time.Second)
	if errors.Is(err, LawSpecPeerFailed) {
		return lawSpecGone{}, true
	}
	if err != nil {
		return nil, false
	}
	if units, ok := value.Data.([]int); ok && value.Type == "Text" {
		text, _ := lsUnitsText(units)
		if i := strings.LastIndex(text, "#"); i >= 0 {
			if owner, known := c.registry[text[:i]]; known {
				which, _ := strconv.Atoi(text[i+1:])
				return lawSpecEnd{owner, which}, true
			}
		}
	}
	return value, true
}

func (c *lawSpecNetScenarioChannel) gone(side int) {
	c.mu.Lock()
	already := c.done[side]
	c.done[side] = true
	c.mu.Unlock()
	if !already {
		c.ends[side].Abandon(side)
	}
}

func (c *lawSpecNetScenarioChannel) close() {
	for side := 0; side < 2; side++ {
		c.ends[side].Close()
		c.nodes[side].Close()
	}
}

// lawSpecScenarioMailbox is a scenario's mailbox: any process sends, one
// receives. expected is how many sends the scenario makes; a process that
// ends gives up the sends it did not make, and a receive with nothing left
// to come fails (lawSpecGone) instead of waiting. Over a network, messages
// go from a sender node to the receiver's node, each send waiting until it
// is delivered.
type lawSpecScenarioMailbox struct {
	name                          string
	expected, received, abandoned int
	mu                            sync.Mutex
	items                         []lawSpecMail
	inbox                         *LawSpecMailbox
	remote                        *LawSpecRemoteMailbox
	nodes                         []*LawSpecNode
	registry                      map[string]*lawSpecNetScenarioChannel
}

type lawSpecMail struct {
	value any
	clock map[string]int64
}

func lsNewScenarioMailbox(name string, expected int, network *LawSpecMemoryNetwork, descriptor any, values lawSpecValues, registry map[string]*lawSpecNetScenarioChannel) *lawSpecScenarioMailbox {
	box := &lawSpecScenarioMailbox{name: name, expected: expected, registry: registry}
	if network != nil {
		owner := NewLawSpecNode(network.InsecureTransportForTests(name + "-owner"))
		senders := NewLawSpecNode(network.InsecureTransportForTests(name + "-senders"))
		box.nodes = []*LawSpecNode{owner, senders}
		if form, ok := descriptor.([]any); ok && lsAtom(form[0]) == "end" {
			descriptor = []any{"text"}
		}
		box.inbox, _ = owner.Mailbox(name, descriptor, values)
		box.remote = senders.RemoteMailbox(owner.Address()+"/"+name, descriptor, values, 5*time.Second)
	}
	return box
}

func (b *lawSpecScenarioMailbox) send(value any, clock map[string]int64) error {
	if b.inbox == nil {
		b.mu.Lock()
		b.items = append(b.items, lawSpecMail{value, clock})
		b.mu.Unlock()
		return nil
	}
	if end, ok := value.(lawSpecEnd); ok {
		net := end.channel.(*lawSpecNetScenarioChannel)
		value = LawSpecValue{"Text", lsTextUnits(fmt.Sprintf("%s#%d", net.name, end.side))}
	}
	// The clock travels beside the network, in send order.
	b.mu.Lock()
	b.items = append(b.items, lawSpecMail{nil, clock})
	b.mu.Unlock()
	return b.remote.Send(value.(LawSpecValue))
}

func (b *lawSpecScenarioMailbox) giveUp(count int) {
	b.mu.Lock()
	b.abandoned += count
	b.mu.Unlock()
}

// receive is the next message and its sender's clock, lawSpecGone{} when
// nothing is left to come, or ok false after waiting too long.
func (b *lawSpecScenarioMailbox) receive() (any, map[string]int64, bool) {
	giveUp := time.Now().Add(5 * time.Second)
	for {
		b.mu.Lock()
		if b.inbox == nil && len(b.items) > 0 {
			mail := b.items[0]
			b.items = b.items[1:]
			b.received++
			b.mu.Unlock()
			return mail.value, mail.clock, true
		}
		if b.received+b.abandoned >= b.expected && (b.inbox == nil || len(b.items) == 0) {
			b.mu.Unlock()
			return lawSpecGone{}, map[string]int64{}, true
		}
		b.mu.Unlock()
		if time.Now().After(giveUp) {
			return nil, nil, false
		}
		if b.inbox == nil {
			time.Sleep(time.Millisecond)
			continue
		}
		value, err := b.inbox.Receive(20 * time.Millisecond)
		if err != nil {
			continue
		}
		b.mu.Lock()
		b.received++
		clock := map[string]int64{}
		if len(b.items) > 0 {
			clock, b.items = b.items[0].clock, b.items[1:]
		}
		b.mu.Unlock()
		v := value.(LawSpecValue)
		if units, ok := v.Data.([]int); ok && v.Type == "Text" {
			text, _ := lsUnitsText(units)
			if i := strings.LastIndex(text, "#"); i >= 0 {
				if owner, known := b.registry[text[:i]]; known {
					which, _ := strconv.Atoi(text[i+1:])
					return lawSpecEnd{owner, which}, clock, true
				}
			}
		}
		return v, clock, true
	}
}

func (b *lawSpecScenarioMailbox) close() {
	for _, node := range b.nodes {
		node.Close()
	}
}

// lsScenarioSends is how many times these acts (not nested pars) send to
// name.
func lsScenarioSends(acts []any, name string) int {
	count := 0
	for _, a := range acts {
		act := a.([]any)
		if lsAtom(act[0]) == "send" && lsAtom(act[1]) == name {
			count++
		}
	}
	return count
}

// lsAllSends is how many sends to name the whole program makes.
func lsAllSends(acts []any, name string) int {
	count := 0
	for _, a := range acts {
		act := a.([]any)
		switch lsAtom(act[0]) {
		case "send":
			if lsAtom(act[1]) == name {
				count++
			}
		case "par":
			for _, branch := range act[1:] {
				count += lsAllSends(branch.([]any)[1:], name)
			}
		}
	}
	return count
}

type lawSpecScenarioCall struct {
	command          *lawSpecModelCommand
	args             []LawSpecValue
	result           LawSpecValue
	called, returned int64
	// The process that made the call, and its vector clock when the call
	// began and when it returned.
	process          string
	atCall, atReturn map[string]int64
}

// lsActsChannels is the names an act list sends, receives or sends away,
// with nested pars.
func lsActsChannels(acts []any) []string {
	names := []string{}
	for _, a := range acts {
		act := a.([]any)
		switch lsAtom(act[0]) {
		case "send":
			names = append(names, lsAtom(act[1]))
			if operand := act[2].([]any); lsAtom(operand[0]) == "var" {
				names = append(names, lsAtom(operand[1]))
			}
		case "receive":
			names = append(names, lsAtom(act[1]))
		case "receiveor":
			names = append(names, lsAtom(act[1]))
			names = append(names, lsActsChannels(act[3].([]any)[1:])...)
		case "par":
			for _, branch := range act[1:] {
				names = append(names, lsActsChannels(branch.([]any)[1:])...)
			}
		}
	}
	return names
}

func lsScenarioConstant(form []any) LawSpecValue {
	switch lsAtom(form[0]) {
	case "int":
		return LawSpecValue{"Integer", new(big.Int).Set(form[1].(*big.Int))}
	case "text":
		units := []int{}
		for _, c := range lsAtom(form[1]) {
			units = append(units, int(c))
		}
		return LawSpecValue{"Text", units}
	case "bool":
		return lsBool(lsAtom(form[1]) == "true")
	}
	tag := lsAtom(form[1])
	t := tag
	if i := strings.LastIndex(tag, "::"); i >= 0 {
		t = tag[:i]
	}
	return LawSpecValue{t, lawSpecData{tag, nil}}
}

func lsRunScenario(model LawSpecModel, spec string, shake uint64, crash, network bool) (string, string, bool) {
	m := lsNewMachine(model)
	forms := lsReadDescriptor(spec)
	title := lsAtom(forms[0].([]any)[1])
	var names, body, wire, boxNames []any
	for _, f := range forms {
		form := f.([]any)
		switch lsAtom(form[0]) {
		case "channels":
			if names == nil {
				names = form[1:]
			}
		case "mailboxes":
			boxNames = form[1:]
		case "process":
			if body == nil {
				body = form[1:]
			}
		case "wire":
			wire = form
		}
	}
	channels := map[string]lawSpecScenarioChannel{}
	mailboxes := map[string]*lawSpecScenarioMailbox{}
	if network && wire != nil {
		// Loss, duplication and delay (which reorders); the channels'
		// numbered, acknowledged frames must hide them all.
		net := NewLawSpecMemoryNetwork(shake^0x7F4A7C159E3779B9, 0.1, 0.1, 2*time.Millisecond)
		table := map[string][]any{}
		steps := map[string][]LawSpecWireStep{}
		for _, f := range wire[1:] {
			form := f.([]any)
			switch lsAtom(form[0]) {
			case "data":
				table[lsAtom(form[1])] = form
			case "channel":
				for _, s := range form[2:] {
					st := s.([]any)
					steps[lsAtom(form[1])] = append(steps[lsAtom(form[1])], LawSpecWireStep{lsAtom(st[0]) == "send", st[1]})
				}
			}
		}
		registry := map[string]*lawSpecNetScenarioChannel{}
		for _, name := range names {
			c := lsNewNetScenarioChannel(net, lsAtom(name), steps[lsAtom(name)], lawSpecValues{table}, registry)
			defer c.close()
			channels[lsAtom(name)] = c
		}
		kinds := map[string]any{}
		for _, f := range wire[1:] {
			if form := f.([]any); lsAtom(form[0]) == "mailbox" {
				kinds[lsAtom(form[1])] = form[2]
			}
		}
		for _, n := range boxNames {
			name := lsAtom(n)
			if d, ok := kinds[name]; ok {
				mailboxes[name] = lsNewScenarioMailbox(name, lsAllSends(body, name), net, d, lawSpecValues{table}, registry)
			} else {
				mailboxes[name] = lsNewScenarioMailbox(name, lsAllSends(body, name), nil, nil, lawSpecValues{}, nil)
			}
			defer mailboxes[name].close()
		}
	} else {
		for _, name := range names {
			channels[lsAtom(name)] = lsNewChannel()
		}
		for _, n := range boxNames {
			name := lsAtom(n)
			mailboxes[name] = lsNewScenarioMailbox(name, lsAllSends(body, name), nil, nil, lawSpecValues{}, nil)
		}
	}
	commands := map[string]*lawSpecModelCommand{}
	for i := range m.commands {
		commands[m.commands[i].name] = &m.commands[i]
	}
	symbols := map[string]*LawSpecSymbol{}
	startArgs := []LawSpecValue{}
	for _, d := range m.startArguments {
		startArgs = append(startArgs, m.values.minimal(d))
	}
	state := m.startRun(symbols, startArgs)
	expected := m.startModel(symbols, startArgs)
	var lock sync.Mutex
	var ticks int64
	history := []lawSpecScenarioCall{}
	failures := []string{}
	fail := func(message string) {
		lock.Lock()
		failures = append(failures, message)
		lock.Unlock()
	}
	failed := func() bool {
		lock.Lock()
		defer lock.Unlock()
		return len(failures) > 0
	}
	// The crashed process (a par's branch) and the act it crashes before.
	var victim *any
	victimAct := -1
	if processes := lsScenarioProcesses(body, nil); crash && len(processes) > 0 {
		chooser := &LawSpecSplitMix64{shake ^ 0xC3A5C85C97CB3127}
		branch := processes[chooser.Below(uint64(len(processes)))]
		victim = &branch[0]
		victimAct = int(chooser.Below(uint64(len(branch) - 1 + 1)))
	}
	// Vector clocks: each value sent carries its sender's clock (kept here,
	// in order per channel direction), so calls can be ordered by what
	// happened before what.
	type stampKey struct {
		channel lawSpecScenarioChannel
		side    int
	}
	stamps := map[stampKey][]map[string]int64{}
	copyClock := func(clock map[string]int64) map[string]int64 {
		out := map[string]int64{}
		for k, v := range clock {
			out[k] = v
		}
		return out
	}
	stamp := func(end lawSpecEnd, clock map[string]int64) {
		lock.Lock()
		key := stampKey{end.channel, end.side}
		stamps[key] = append(stamps[key], copyClock(clock))
		lock.Unlock()
	}
	unstamp := func(end lawSpecEnd, clock map[string]int64, me string) {
		lock.Lock()
		key := stampKey{end.channel, 1 - end.side}
		var sent map[string]int64
		if queue := stamps[key]; len(queue) > 0 {
			sent, stamps[key] = queue[0], queue[1:]
		}
		lock.Unlock()
		for p, n := range sent {
			if n > clock[p] {
				clock[p] = n
			}
		}
		clock[me]++
	}
	var process func(acts []any, env map[string]LawSpecValue, ends map[string]lawSpecEnd, random *LawSpecSplitMix64, identity *any, clock map[string]int64, me string) bool
	var steps func(acts []any, env map[string]LawSpecValue, ends map[string]lawSpecEnd, random *LawSpecSplitMix64, identity *any, clock map[string]int64, me string, sent map[string]int) bool
	// process is false when the process failed; either way, the ends it
	// still holds are given up, and so are the mailbox sends it did not make.
	process = func(acts []any, env map[string]LawSpecValue, ends map[string]lawSpecEnd, random *LawSpecSplitMix64, identity *any, clock map[string]int64, me string) bool {
		sent := map[string]int{}
		defer func() {
			for _, end := range ends {
				end.channel.gone(end.side)
			}
			for name, box := range mailboxes {
				if missing := lsScenarioSends(acts, name) - sent[name]; missing > 0 {
					box.giveUp(missing)
				}
			}
		}()
		return steps(acts, env, ends, random, identity, clock, me, sent)
	}
	steps = func(acts []any, env map[string]LawSpecValue, ends map[string]lawSpecEnd, random *LawSpecSplitMix64, identity *any, clock map[string]int64, me string, sent map[string]int) bool {
		own := map[string]*LawSpecSymbol{}
		for index, a := range acts {
			if failed() {
				return false
			}
			if victim != nil && identity == victim && index == victimAct {
				return false
			}
			act := a.([]any)
			switch lsAtom(act[0]) {
			case "call":
				command := commands[lsAtom(act[1])]
				args := []LawSpecValue{}
				for j, o := range act[3:] {
					operand := o.([]any)
					if lsAtom(operand[0]) == "var" {
						args = append(args, env[lsAtom(operand[1])])
					} else {
						// An integer constant takes the argument's integer type.
						value := lsScenarioConstant(operand)
						if value.Type == "Integer" && j < len(command.arguments) {
							if form := m.values.resolve(command.arguments[j]); lsAtom(form[0]) == "int" {
								value = LawSpecValue{m.values.typeOf(form), value.Data}
							}
						}
						args = append(args, value)
					}
				}
				full := lsWithState(command, args, state)
				lsPerturb(random)
				clock[me]++
				atCall := copyClock(clock)
				called := atomic.AddInt64(&ticks, 1)
				result, ok := func() (result LawSpecValue, ok bool) {
					defer func() {
						if r := recover(); r != nil {
							fail(fmt.Sprintf("%s raised panic: %v", command.name, r))
							ok = false
						}
					}()
					return command.run(own, full), true
				}()
				if !ok {
					return false
				}
				returned := atomic.AddInt64(&ticks, 1)
				clock[me]++
				lock.Lock()
				history = append(history, lawSpecScenarioCall{command, args, result, called, returned, me, atCall, copyClock(clock)})
				lock.Unlock()
				if act[2] != nil && lsAtom(act[2]) != "_" {
					env[lsAtom(act[2])] = result
				}
			case "send":
				if box, isBox := mailboxes[lsAtom(act[1])]; isBox {
					operand := act[2].([]any)
					var value any
					if held, isEnd := ends[lsAtom(operand[1])]; lsAtom(operand[0]) == "var" && isEnd {
						delete(ends, lsAtom(operand[1]))
						value = held
					} else if lsAtom(operand[0]) == "var" {
						value = env[lsAtom(operand[1])]
					} else {
						value = lsScenarioConstant(operand)
					}
					lsPerturb(random)
					clock[me]++
					if err := box.send(value, copyClock(clock)); err != nil {
						fail(fmt.Sprintf("a send to mailbox %s failed: %v", lsAtom(act[1]), err))
						return false
					}
					sent[lsAtom(act[1])]++
					continue
				}
				end := ends[lsAtom(act[1])]
				operand := act[2].([]any)
				var value any
				if held, isEnd := ends[lsAtom(operand[1])]; lsAtom(operand[0]) == "var" && isEnd {
					delete(ends, lsAtom(operand[1]))
					value = held
				} else if lsAtom(operand[0]) == "var" {
					value = env[lsAtom(operand[1])]
				} else {
					value = lsScenarioConstant(operand)
				}
				lsPerturb(random)
				clock[me]++
				stamp(end, clock)
				end.channel.send(end.side, value)
			case "receive", "receiveor":
				name := lsAtom(act[1])
				if box, isBox := mailboxes[name]; isBox {
					value, carried, ok := box.receive()
					if !ok {
						fail(fmt.Sprintf("a receive on mailbox %s waited too long: the processes are blocked", name))
						return false
					}
					if _, gone := value.(lawSpecGone); gone {
						if lsAtom(act[0]) == "receive" {
							return false
						}
						return steps(act[3].([]any)[1:], env, ends, random, nil, clock, me, sent)
					}
					for p, n := range carried {
						if n > clock[p] {
							clock[p] = n
						}
					}
					clock[me]++
					if held, isEnd := value.(lawSpecEnd); isEnd {
						ends[lsAtom(act[2])] = held
					} else {
						env[lsAtom(act[2])] = value.(LawSpecValue)
					}
					continue
				}
				end := ends[name]
				value, ok := end.channel.receive(end.side)
				if !ok {
					fail(fmt.Sprintf("a receive on %s waited too long: the processes are blocked", name))
					return false
				}
				if _, gone := value.(lawSpecGone); gone {
					// The other process ended: or else runs instead of the
					// rest; without it, this process fails too.
					if lsAtom(act[0]) == "receive" {
						return false
					}
					delete(ends, name)
					return steps(act[3].([]any)[1:], env, ends, random, nil, clock, me, sent)
				}
				unstamp(end, clock, me)
				if held, isEnd := value.(lawSpecEnd); isEnd {
					ends[lsAtom(act[2])] = held
				} else {
					env[lsAtom(act[2])] = value.(LawSpecValue)
				}
			case "par":
				branches := [][]any{}
				for _, b := range act[1:] {
					branches = append(branches, b.([]any))
				}
				owned := map[string][]int{}
				order := []string{}
				for i, branch := range branches {
					for _, name := range lsActsChannels(branch[1:]) {
						users, seen := owned[name]
						if !seen {
							order = append(order, name)
						}
						if len(users) == 0 || users[len(users)-1] != i {
							owned[name] = append(users, i)
						}
					}
				}
				var group sync.WaitGroup
				outcomes := make([]bool, len(branches))
				clocks := make([]map[string]int64, len(branches))
				for i, branch := range branches {
					mine := map[string]lawSpecEnd{}
					for _, name := range order {
						for side, user := range owned[name] {
							if user != i {
								continue
							}
							if held, ok := ends[name]; ok {
								mine[name] = held
								delete(ends, name)
							} else if channel, ok := channels[name]; ok {
								mine[name] = lawSpecEnd{channel, side}
							}
						}
					}
					copied := map[string]LawSpecValue{}
					for k, v := range env {
						copied[k] = v
					}
					random := &LawSpecSplitMix64{shake ^ (uint64(i+1) * 0x9E3779B97F4A7C15)}
					clocks[i] = copyClock(clock)
					group.Add(1)
					go func(i int, branch []any) {
						defer group.Done()
						outcomes[i] = process(branch[1:], copied, mine, random, &branch[0], clocks[i], fmt.Sprintf("%p", &branch[0]))
					}(i, branch)
				}
				group.Wait()
				for _, child := range clocks {
					for p, n := range child {
						if n > clock[p] {
							clock[p] = n
						}
					}
				}
				clock[me]++
				// A failed branch fails the process that ran the par.
				for _, ok := range outcomes {
					if !ok {
						return false
					}
				}
			case "expect":
				name := lsAtom(act[1])
				wanted := lsScenarioConstant(act[2].([]any))
				actual, bound := env[name]
				if !bound || !lsScenarioEqual(actual, wanted) {
					rendered := "None"
					if bound {
						rendered = lsRender(actual)
					}
					fail(fmt.Sprintf("expect %s = %s failed: %s is %s", name, lsRender(wanted), name, rendered))
					return false
				}
			}
		}
		if victim != nil && identity == victim && victimAct == len(acts) {
			return false
		}
		return true
	}
	outcome := process(body, map[string]LawSpecValue{}, map[string]lawSpecEnd{}, &LawSpecSplitMix64{shake}, nil, map[string]int64{}, "root")
	if len(failures) > 0 {
		if victim != nil {
			return title, failures[0] + " (with a process crashed)", true
		}
		return title, failures[0], true
	}
	if !outcome && victim == nil {
		return title, "a process failed", true
	}
	var final *LawSpecValue
	if m.abstract != nil {
		actual := m.abstract(symbols, []LawSpecValue{state})
		final = &actual
	}
	if !m.linearizesHistory(symbols, history, expected, final, state) {
		sorted := append([]lawSpecScenarioCall{}, history...)
		sort.SliceStable(sorted, func(a, b int) bool { return sorted[a].called < sorted[b].called })
		observed := []string{}
		for _, h := range sorted {
			args := []string{}
			for _, a := range h.args {
				args = append(args, lsRender(a))
			}
			observed = append(observed, fmt.Sprintf("%s(%s) returned %s", h.command.name, strings.Join(args, ", "), lsRender(h.result)))
		}
		return title, "the calls are not " + lsConsistent[m.consistency] + " with the model (" + strings.Join(observed, "; ") + ")", true
	}
	return title, "", false
}

// lsScenarioEqual is whether lsCompareValues finds the values equal; values
// with no common order are unequal.
func lsScenarioEqual(a, b LawSpecValue) (equal bool) {
	defer func() {
		if recover() != nil {
			equal = false
		}
	}()
	return lsCompareValues(a, b) == 0
}

// lsHappenedBefore is whether call a returned before call b began, as far
// as messages tell: a's return clock is at or below b's call clock
// everywhere.
func lsHappenedBefore(a, b lawSpecScenarioCall) bool {
	for p, n := range a.atReturn {
		if b.atCall[p] < n {
			return false
		}
	}
	return true
}

// linearizesHistory is a Wing-Gong search over the scenario's calls,
// memoized on the calls done and the state. Linearizable: next, a call no
// pending call returned before (real time). Sequential: next, a call every
// call that happened before it (its process's order, and messages) is done.
// Causal: each process's results from an order of what happened before
// them. Eventual: no results, only the final state.
func (m *lawSpecMachine) linearizesHistory(symbols map[string]*LawSpecSymbol, history []lawSpecScenarioCall, expected LawSpecValue, final *LawSpecValue, state LawSpecValue) bool {
	mode := m.consistency
	before := func(j, i int) bool {
		if mode == "linearizable" {
			return history[j].returned < history[i].called
		}
		return lsHappenedBefore(history[j], history[i])
	}
	search := func(members []int, checked map[int]bool, judgeFinal bool) bool {
		seen := map[string]bool{}
		done := make([]byte, len(history))
		var visit func(remaining int, model LawSpecValue) bool
		visit = func(remaining int, model LawSpecValue) bool {
			key := string(done) + "|" + lsRender(model)
			if seen[key] {
				return false
			}
			seen[key] = true
			if remaining == 0 {
				if !judgeFinal {
					return true
				}
				if final != nil && lsCompareValues(*final, model) != 0 {
					return false
				}
				for i, kind := range m.invariantKinds {
					if i >= len(m.invariants) {
						break
					}
					subject := state
					if kind == "model" {
						subject = model
					}
					if !lsTruth(m.invariants[i](symbols, []LawSpecValue{subject})) {
						return false
					}
				}
				return true
			}
			for _, i := range members {
				if done[i] == 1 {
					continue
				}
				blocked := false
				for _, j := range members {
					if j != i && done[j] == 0 && before(j, i) {
						blocked = true
						break
					}
				}
				if blocked {
					continue
				}
				call := history[i]
				var after, wanted LawSpecValue
				if !lsAllowed(func() { after, wanted = lsStepModel(call.command, symbols, call.args, model) }) {
					continue
				}
				if checked[i] && !call.command.unit && !lsScenarioEqual(call.result, wanted) {
					continue
				}
				done[i] = 1
				ok := visit(remaining-1, after)
				done[i] = 0
				if ok {
					return true
				}
			}
			return false
		}
		return visit(len(members), expected)
	}
	everything := []int{}
	all := map[int]bool{}
	for i := range history {
		everything = append(everything, i)
		all[i] = true
	}
	if mode == "causal" {
		processes := map[string]bool{}
		for _, h := range history {
			processes[h.process] = true
		}
		for process := range processes {
			own := map[int]bool{}
			for i, h := range history {
				if h.process == process {
					own[i] = true
				}
			}
			members := []int{}
			for j := range history {
				include := own[j]
				for i := range own {
					include = include || (j != i && lsHappenedBefore(history[j], history[i]))
				}
				if include {
					members = append(members, j)
				}
			}
			if !search(members, own, false) {
				return false
			}
		}
		return true
	}
	if mode == "eventual" {
		return search(everything, map[int]bool{}, true)
	}
	return search(everything, all, true)
}

// LawSpecCheckScenario runs a scenario on many schedules; a failure is an
// error.
func LawSpecCheckScenario(model LawSpecModel, spec string) error {
	var seed uint64
	if text := os.Getenv("LAWSPEC_SEED"); text != "" {
		parsed, err := strconv.ParseUint(text, 10, 64)
		if err != nil {
			return fmt.Errorf("invalid LAWSPEC_SEED %q", text)
		}
		seed = parsed
	}
	return lsCheckScenario(model, spec, 30, seed)
}

func lsCheckScenario(model LawSpecModel, spec string, runs int, seed uint64) error {
	random := &LawSpecSplitMix64{seed ^ 0x2545F4914F6CDD1D}
	for n := 0; n < runs; n++ {
		// Every third run crashes one process of a par at a random point, and
		// every third other one sends each channel over a faulty network.
		if title, failure, failed := lsRunScenario(model, spec, random.Next(), n%3 == 2, n%3 == 1); failed {
			return fmt.Errorf("scenario %s fails: %s", title, failure)
		}
	}
	return nil
}

// Distribution. Values cross the network in a canonical binary encoding
// driven by their type descriptor (the same descriptors as generation), so
// no tags are sent and every target writes the same bytes:
//   int: zigzag LEB128 of the integer (any size)      bool: 0 or 1
//   text: LEB128 length, then UTF-8                   unit: nothing
//   list: LEB128 count, then items                     maybe: 0, or 1 then the value
//   either: 0 then left, or 1 then right               data: LEB128 constructor index, then fields
// A node sends frames over a transport (in memory, TCP or HTTP): kind,
// entity name, the sender's address, an id and a payload.

// LawSpecWireError is bytes that are not an encoding of a value of the
// expected type.
type LawSpecWireError struct{ Message string }

func (e LawSpecWireError) Error() string { return e.Message }

// LawSpecUnreachable is the error of a node that could not be reached, or
// did not answer in time; errors.Is finds it in a wrapped error.
var LawSpecUnreachable = errors.New("unreachable")

func lsUnreachable(message string) error { return fmt.Errorf("%w: %s", LawSpecUnreachable, message) }

func lsPutUvarint(out []byte, n uint64) []byte {
	for {
		b := byte(n & 0x7F)
		n >>= 7
		if n != 0 {
			out = append(out, b|0x80)
		} else {
			return append(out, b)
		}
	}
}

func lsPutBigUvarint(out []byte, n *big.Int) []byte {
	z := new(big.Int).Set(n)
	mask := big.NewInt(0x7F)
	for {
		b := byte(new(big.Int).And(z, mask).Int64())
		z.Rsh(z, 7)
		if z.Sign() != 0 {
			out = append(out, b|0x80)
		} else {
			return append(out, b)
		}
	}
}

func lsGetBigUvarint(buf []byte, pos int) (*big.Int, int, error) {
	result := new(big.Int)
	shift := uint(0)
	for {
		if pos >= len(buf) {
			return nil, pos, LawSpecWireError{"the bytes end in the middle of a value"}
		}
		b := buf[pos]
		pos++
		result.Or(result, new(big.Int).Lsh(big.NewInt(int64(b&0x7F)), shift))
		if b < 0x80 {
			return result, pos, nil
		}
		shift += 7
	}
}

func lsGetUvarint(buf []byte, pos int) (uint64, int, error) {
	n, pos, err := lsGetBigUvarint(buf, pos)
	if err != nil {
		return 0, pos, err
	}
	if !n.IsUint64() {
		return 0, pos, LawSpecWireError{"a count too large"}
	}
	return n.Uint64(), pos, nil
}

func lsZigzag(n *big.Int) *big.Int {
	if n.Sign() >= 0 {
		return new(big.Int).Lsh(n, 1)
	}
	z := new(big.Int).Lsh(new(big.Int).Neg(n), 1)
	return z.Sub(z, big.NewInt(1))
}

func lsUnzigzag(z *big.Int) *big.Int {
	if z.Bit(0) == 0 {
		return new(big.Int).Rsh(z, 1)
	}
	n := new(big.Int).Add(z, big.NewInt(1))
	n.Rsh(n, 1)
	return n.Neg(n)
}

func lsPutText(out []byte, text string) []byte {
	out = lsPutUvarint(out, uint64(len(text)))
	return append(out, text...)
}

func lsGetText(buf []byte, pos int) (string, int, error) {
	raw, pos, err := lsGetRaw(buf, pos)
	if err != nil {
		return "", pos, err
	}
	if !utf8.Valid(raw) {
		return "", pos, LawSpecWireError{"text that is not UTF-8"}
	}
	return string(raw), pos, nil
}

func lsGetRaw(buf []byte, pos int) ([]byte, int, error) {
	n, pos, err := lsGetUvarint(buf, pos)
	if err != nil {
		return nil, pos, err
	}
	if uint64(len(buf)-pos) < n {
		return nil, pos, LawSpecWireError{"the bytes end in the middle of a value"}
	}
	return buf[pos : pos+int(n)], pos + int(n), nil
}

func lsUnitsText(units []int) (string, error) {
	var b strings.Builder
	for _, c := range units {
		if c < 0 || c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF) {
			return "", LawSpecWireError{"text with a code point that UTF-8 cannot hold"}
		}
		b.WriteRune(rune(c))
	}
	return b.String(), nil
}

func lsTextUnits(text string) []int {
	units := []int{}
	for _, c := range text {
		units = append(units, int(c))
	}
	return units
}

func lsWirePut(values lawSpecValues, d any, v LawSpecValue, out []byte) ([]byte, error) {
	form := values.resolve(d)
	switch lsAtom(form[0]) {
	case "int":
		n, ok := v.Data.(*big.Int)
		if !ok {
			return out, LawSpecWireError{fmt.Sprintf("%s is not a %s", lsRender(v), lsAtom(form[1]))}
		}
		lo, _ := form[2].(*big.Int)
		hi, _ := form[3].(*big.Int)
		if (lo != nil && n.Cmp(lo) < 0) || (hi != nil && n.Cmp(hi) > 0) {
			return out, LawSpecWireError{fmt.Sprintf("%s is not a %s", n, lsAtom(form[1]))}
		}
		return lsPutBigUvarint(out, lsZigzag(n)), nil
	case "bool":
		b, ok := v.Data.(bool)
		if !ok {
			return out, LawSpecWireError{"not a Bool"}
		}
		if b {
			return append(out, 1), nil
		}
		return append(out, 0), nil
	case "text", "end":
		units, ok := v.Data.([]int)
		if !ok {
			return out, LawSpecWireError{"not a Text"}
		}
		text, err := lsUnitsText(units)
		if err != nil {
			return out, err
		}
		return lsPutText(out, text), nil
	case "unit":
		return out, nil
	case "list":
		items, ok := v.Data.([]LawSpecValue)
		if !ok {
			return out, LawSpecWireError{"not a List"}
		}
		out = lsPutUvarint(out, uint64(len(items)))
		for _, item := range items {
			var err error
			if out, err = lsWirePut(values, form[1], item, out); err != nil {
				return out, err
			}
		}
		return out, nil
	case "maybe", "either":
		data, ok := v.Data.(lawSpecData)
		if !ok {
			return out, LawSpecWireError{"not a " + lsAtom(form[0])}
		}
		if lsAtom(form[0]) == "maybe" {
			if strings.HasSuffix(data.tag, "Nothing") {
				return append(out, 0), nil
			}
			return lsWirePut(values, form[1], data.fields[0], append(out, 1))
		}
		if strings.HasSuffix(data.tag, "Left") {
			return lsWirePut(values, form[1], data.fields[0], append(out, 0))
		}
		return lsWirePut(values, form[2], data.fields[0], append(out, 1))
	case "data":
		data, ok := v.Data.(lawSpecData)
		if !ok {
			return out, LawSpecWireError{"not a " + lsAtom(form[1])}
		}
		for index, c := range form[2:] {
			ctor := c.([]any)
			if lsAtom(ctor[1]) != data.tag {
				continue
			}
			out = lsPutUvarint(out, uint64(index))
			for k, fd := range ctor[2:] {
				var err error
				if out, err = lsWirePut(values, fd, data.fields[k], out); err != nil {
					return out, err
				}
			}
			return out, nil
		}
		return out, LawSpecWireError{data.tag + " is not a constructor of " + lsAtom(form[1])}
	}
	return out, LawSpecWireError{"unknown descriptor " + fmt.Sprint(form)}
}

func lsWireGet(values lawSpecValues, d any, buf []byte, pos int) (LawSpecValue, int, error) {
	form := values.resolve(d)
	switch lsAtom(form[0]) {
	case "int":
		z, next, err := lsGetBigUvarint(buf, pos)
		if err != nil {
			return LawSpecValue{}, next, err
		}
		n := lsUnzigzag(z)
		lo, _ := form[2].(*big.Int)
		hi, _ := form[3].(*big.Int)
		if (lo != nil && n.Cmp(lo) < 0) || (hi != nil && n.Cmp(hi) > 0) {
			return LawSpecValue{}, next, LawSpecWireError{fmt.Sprintf("%s is out of range for %s", n, lsAtom(form[1]))}
		}
		return LawSpecValue{values.typeOf(form), n}, next, nil
	case "bool":
		if pos >= len(buf) || buf[pos] > 1 {
			return LawSpecValue{}, pos, LawSpecWireError{"not a Bool"}
		}
		return lsBool(buf[pos] == 1), pos + 1, nil
	case "text", "end":
		text, next, err := lsGetText(buf, pos)
		if err != nil {
			return LawSpecValue{}, next, err
		}
		return LawSpecValue{"Text", lsTextUnits(text)}, next, nil
	case "unit":
		return LawSpecValue{"Unit", nil}, pos, nil
	case "list":
		n, next, err := lsGetUvarint(buf, pos)
		if err != nil {
			return LawSpecValue{}, next, err
		}
		pos = next
		items := []LawSpecValue{}
		for ; n > 0; n-- {
			var item LawSpecValue
			if item, pos, err = lsWireGet(values, form[1], buf, pos); err != nil {
				return LawSpecValue{}, pos, err
			}
			items = append(items, item)
		}
		return LawSpecValue{values.typeOf(form), items}, pos, nil
	case "maybe", "either":
		if pos >= len(buf) || buf[pos] > 1 {
			return LawSpecValue{}, pos, LawSpecWireError{"not a " + lsAtom(form[0])}
		}
		which := buf[pos]
		pos++
		t := values.typeOf(form)
		if lsAtom(form[0]) == "maybe" {
			if which == 0 {
				return lsSum(t, "Maybe::Nothing"), pos, nil
			}
			v, next, err := lsWireGet(values, form[1], buf, pos)
			return lsSum(t, "Maybe::Just", v), next, err
		}
		if which == 0 {
			v, next, err := lsWireGet(values, form[1], buf, pos)
			return lsSum(t, "Either::Left", v), next, err
		}
		v, next, err := lsWireGet(values, form[2], buf, pos)
		return lsSum(t, "Either::Right", v), next, err
	case "data":
		index, next, err := lsGetUvarint(buf, pos)
		if err != nil {
			return LawSpecValue{}, next, err
		}
		ctors := form[2:]
		if index >= uint64(len(ctors)) {
			return LawSpecValue{}, next, LawSpecWireError{fmt.Sprintf("no constructor %d in %s", index, lsAtom(form[1]))}
		}
		ctor := ctors[index].([]any)
		pos = next
		fields := []LawSpecValue{}
		for _, fd := range ctor[2:] {
			var v LawSpecValue
			if v, pos, err = lsWireGet(values, fd, buf, pos); err != nil {
				return LawSpecValue{}, pos, err
			}
			fields = append(fields, v)
		}
		return lsSum(values.typeOf(form), lsAtom(ctor[1]), fields...), pos, nil
	}
	return LawSpecValue{}, pos, LawSpecWireError{"unknown descriptor " + fmt.Sprint(form)}
}

// LawSpecTypes is the data types of a descriptor text, for the wire.
func LawSpecTypes(text string) lawSpecValues {
	table := map[string][]any{}
	for _, f := range lsReadDescriptor(text) {
		if form, ok := f.([]any); ok && lsAtom(form[0]) == "data" {
			table[lsAtom(form[1])] = form
		}
	}
	return lawSpecValues{table}
}

// LawSpecDescriptor is the first descriptor in a text.
func LawSpecDescriptor(text string) any { return lsReadDescriptor(text)[0] }

// LawSpecWireEncode is a value's canonical bytes.
func LawSpecWireEncode(values lawSpecValues, d any, v LawSpecValue) ([]byte, error) {
	return lsWirePut(values, d, v, nil)
}

// LawSpecWireDecode is the value encoded by exactly these bytes.
func LawSpecWireDecode(values lawSpecValues, d any, data []byte) (LawSpecValue, error) {
	v, pos, err := lsWireGet(values, d, data, 0)
	if err != nil {
		return LawSpecValue{}, err
	}
	if pos != len(data) {
		return LawSpecValue{}, LawSpecWireError{"extra bytes after the value"}
	}
	return v, nil
}

// LawSpec's own search over a law's inputs (see LawSpec.Search). The
// failure database keeps a failing case's inputs in the wire encoding, under
// LAWSPEC_FAILURES/inputs, and they are replayed before the law's next
// generated cases. target maximize climbs: it generates inputs from seeds,
// keeps the best-scoring case, and moves its integers (one step, doubling,
// halving the distance to a bound), keeping the best move while the score
// rises and drawing afresh when it does not, checking the law on every case
// it tries (at most 400). A case gives its score, and false when its inputs
// fall outside the law's refinements.
type LawSpecSearchCase func(values []LawSpecValue) (float64, bool)

func lsSearchFile(law string) string {
	directory := os.Getenv("LAWSPEC_FAILURES")
	if directory == "" {
		return ""
	}
	safe := strings.Map(func(r rune) rune {
		if r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '-' || r == '_' {
			return r
		}
		return '_'
	}, law)
	return filepath.Join(directory, "inputs", safe+".json")
}

func LawSpecSearchRemember(law string, descriptors []string, values []LawSpecValue) {
	file := lsSearchFile(law)
	if file == "" {
		return
	}
	inputs := []string{}
	for i, text := range descriptors {
		table, d := lsValuesFrom(text)
		encoded, err := LawSpecWireEncode(table, d, values[i])
		if err != nil {
			return
		}
		inputs = append(inputs, fmt.Sprintf("%x", encoded))
	}
	data, _ := json.Marshal(map[string]any{"law": law, "inputs": inputs})
	_ = os.MkdirAll(filepath.Dir(file), 0o755)
	_ = os.WriteFile(file, data, 0o644)
}

func LawSpecSearchReplay(law string, descriptors []string, check LawSpecSearchCase) {
	file := lsSearchFile(law)
	if file == "" {
		return
	}
	data, err := os.ReadFile(file)
	if err != nil {
		return
	}
	var entry struct {
		Inputs []string `json:"inputs"`
	}
	values := []LawSpecValue{}
	if json.Unmarshal(data, &entry) == nil && len(entry.Inputs) == len(descriptors) {
		for i, text := range descriptors {
			table, d := lsValuesFrom(text)
			raw := make([]byte, len(entry.Inputs[i])/2)
			if _, err := fmt.Sscanf(entry.Inputs[i], "%x", &raw); err != nil && len(raw) > 0 {
				break
			}
			value, err := LawSpecWireDecode(table, d, raw)
			if err != nil {
				break
			}
			values = append(values, value)
		}
	}
	if len(values) == len(descriptors) {
		fmt.Printf("%s: replaying the failing inputs kept in .lawspec/failures\n", law)
		check(values)
	}
	_ = os.Remove(file)
}

// Each value with one integer moved, wherever it sits in the value.
func (s lawSpecValues) searchMoves(d any, v LawSpecValue) []LawSpecValue {
	form := s.resolve(d)
	switch lsAtom(form[0]) {
	case "int":
		lo, hi := s.bounds(form)
		x, ok := v.Data.(*big.Int)
		if !ok {
			return nil
		}
		two := big.NewInt(2)
		candidates := []*big.Int{
			new(big.Int).Add(x, big.NewInt(1)), new(big.Int).Sub(x, big.NewInt(1)), new(big.Int).Mul(x, two),
			new(big.Int).Quo(x, two),
			new(big.Int).Add(x, new(big.Int).Quo(new(big.Int).Sub(hi, x), two)),
			new(big.Int).Sub(x, new(big.Int).Quo(new(big.Int).Sub(x, lo), two)),
		}
		out := []LawSpecValue{}
		seen := map[string]bool{x.String(): true}
		for _, c := range candidates {
			if c.Cmp(lo) >= 0 && c.Cmp(hi) <= 0 && !seen[c.String()] {
				seen[c.String()] = true
				out = append(out, LawSpecValue{v.Type, c})
			}
		}
		return out
	case "list":
		items, _ := v.Data.([]LawSpecValue)
		out := []LawSpecValue{}
		for i, item := range items {
			for _, m := range s.searchMoves(form[1], item) {
				changed := append([]LawSpecValue{}, items...)
				changed[i] = m
				out = append(out, LawSpecValue{v.Type, changed})
			}
		}
		return out
	case "maybe", "either", "data":
		data, ok := v.Data.(lawSpecData)
		if !ok {
			return nil
		}
		var fields []any
		switch lsAtom(form[0]) {
		case "maybe":
			fields = []any{form[1]}
		case "either":
			if data.tag == "Either::Left" {
				fields = []any{form[1]}
			} else {
				fields = []any{form[2]}
			}
		default:
			for _, c := range form[2:] {
				if ctor := c.([]any); lsAtom(ctor[1]) == data.tag {
					fields = ctor[2:]
				}
			}
		}
		out := []LawSpecValue{}
		for i, f := range fields {
			if i >= len(data.fields) {
				break
			}
			for _, m := range s.searchMoves(f, data.fields[i]) {
				changed := append([]LawSpecValue{}, data.fields...)
				changed[i] = m
				out = append(out, LawSpecValue{v.Type, lawSpecData{data.tag, changed}})
			}
		}
		return out
	}
	return nil
}

func LawSpecSearchClimb(law string, descriptors []string, check LawSpecSearchCase) {
	type described struct {
		table lawSpecValues
		d     any
	}
	tables := []described{}
	for _, text := range descriptors {
		table, d := lsValuesFrom(text)
		tables = append(tables, described{table, d})
	}
	var base uint64
	if seed := os.Getenv("LAWSPEC_SEED"); seed != "" {
		_, _ = fmt.Sscan(seed, &base)
	}
	generate := func(seed uint64) []LawSpecValue {
		random := LawSpecSplitMix64{seed}
		out := []LawSpecValue{}
		for _, t := range tables {
			out = append(out, t.table.generate(t.d, &random, 8))
		}
		return out
	}
	var best []LawSpecValue
	bestScore := math.Inf(-1)
	tried := 0
	attempt := func(values []LawSpecValue) bool {
		if tried >= 400 {
			return false
		}
		tried++
		defer func() {
			if problem := recover(); problem != nil {
				LawSpecSearchRemember(law, descriptors, values)
				panic(problem)
			}
		}()
		score, ok := check(values)
		if !ok || (best != nil && !(score > bestScore)) {
			return false
		}
		best, bestScore = values, score
		return true
	}
	for k := uint64(0); k < 10; k++ {
		attempt(generate(base + k))
	}
	for step := uint64(0); step < 60 && tried < 400; step++ {
		rose := false
		if best != nil {
			current := best
			moves := [][]LawSpecValue{}
			for i, v := range current {
				for _, m := range tables[i].table.searchMoves(tables[i].d, v) {
					changed := append([]LawSpecValue{}, current...)
					changed[i] = m
					moves = append(moves, changed)
				}
			}
			if len(moves) > 24 {
				moves = moves[:24]
			}
			for _, candidate := range moves {
				if attempt(candidate) {
					rose = true
				}
			}
		}
		if !rose {
			attempt(generate(base + 1000 + step))
		}
	}
	if best == nil {
		fmt.Printf("%s: targeted search tried %d case(s)\n", law, tried)
	} else {
		fmt.Printf("%s: targeted search tried %d case(s); best score %v\n", law, tried, bestScore)
	}
}

// lsSearchNumber is a score as a float: the targeted search maximizes it.
func lsSearchNumber(v LawSpecValue) float64 {
	switch x := v.Data.(type) {
	case float64:
		return x
	case float32:
		return float64(x)
	default:
		var f float64
		if _, err := fmt.Sscan(fmt.Sprint(x), &f); err == nil {
			return f
		}
		return math.NaN()
	}
}

// LawSpecWireEncoded is count values generated from one seed, encoded, in
// hexadecimal.
func LawSpecWireEncoded(text string, seed uint64, size int64, count int64) []string {
	values, d := lsValuesFrom(text)
	random := LawSpecSplitMix64{seed}
	result := []string{}
	for ; count > 0; count-- {
		encoded, err := LawSpecWireEncode(values, d, values.generate(d, &random, size))
		if err != nil {
			panic(err)
		}
		result = append(result, fmt.Sprintf("%x", encoded))
	}
	return result
}

// LawSpecWireRoundTrips is whether count generated values decode to
// themselves.
func LawSpecWireRoundTrips(text string, seed uint64, size int64, count int64) bool {
	values, d := lsValuesFrom(text)
	random := LawSpecSplitMix64{seed}
	for ; count > 0; count-- {
		v := values.generate(d, &random, size)
		encoded, err := LawSpecWireEncode(values, d, v)
		if err != nil {
			return false
		}
		back, err := LawSpecWireDecode(values, d, encoded)
		if err != nil || lsRender(back) != lsRender(v) {
			return false
		}
	}
	return true
}

// lsInt64Bytes encodes a signed integer as (int Int64 _ _) does.
func lsInt64Bytes(out []byte, n int64) []byte {
	return lsPutBigUvarint(out, lsZigzag(big.NewInt(n)))
}

func lsGetInt64(buf []byte, pos int) (int64, int, error) {
	z, pos, err := lsGetBigUvarint(buf, pos)
	if err != nil {
		return 0, pos, err
	}
	n := lsUnzigzag(z)
	if !n.IsInt64() {
		return 0, pos, LawSpecWireError{"an integer out of range"}
	}
	return n.Int64(), pos, nil
}

func lsFrameEncode(kind, to, source string, id uint64, payload []byte) []byte {
	out := lsPutText(nil, kind)
	out = lsPutText(out, to)
	out = lsPutText(out, source)
	out = lsPutBigUvarint(out, lsZigzag(new(big.Int).SetUint64(id)))
	out = lsPutUvarint(out, uint64(len(payload)))
	return append(out, payload...)
}

func lsFrameDecode(frame []byte) (kind, to, source string, id uint64, payload []byte, err error) {
	pos := 0
	if kind, pos, err = lsGetText(frame, pos); err != nil {
		return
	}
	if to, pos, err = lsGetText(frame, pos); err != nil {
		return
	}
	if source, pos, err = lsGetText(frame, pos); err != nil {
		return
	}
	var z *big.Int
	if z, pos, err = lsGetBigUvarint(frame, pos); err != nil {
		return
	}
	n := lsUnzigzag(z)
	if n.Sign() < 0 || !n.IsUint64() {
		err = LawSpecWireError{"a frame id out of range"}
		return
	}
	id = n.Uint64()
	if payload, pos, err = lsGetRaw(frame, pos); err != nil {
		return
	}
	if pos != len(frame) {
		err = LawSpecWireError{"extra bytes after a frame"}
	}
	return
}

// lsSplitAddress is "tcp://host:port/name" as ("tcp://host:port", "name").
func lsSplitAddress(address string) (string, string, error) {
	i := strings.LastIndex(address, "/")
	if i < 0 || !strings.Contains(address[:i], "://") {
		return "", "", fmt.Errorf("%q is not an address such as tcp://127.0.0.1:7000/name", address)
	}
	return address[:i], address[i+1:], nil
}

// LawSpecNetTransport moves frames between nodes. Start begins calling
// deliver for every frame that arrives; Send sends one to the node at that
// address, best effort; Close stops.
type LawSpecNetTransport interface {
	Address() string
	Start(deliver func(frame []byte))
	Send(node string, frame []byte) error
	Close()
}

// LawSpecMemoryNetwork is nodes in one process, with faults for testing:
// each frame may be lost or duplicated, and is delayed by up to Delay (so
// frames can overtake each other); Partition cuts nodes off until Heal.
type LawSpecMemoryNetwork struct {
	mu        sync.Mutex
	random    LawSpecSplitMix64
	loss      float64
	duplicate float64
	delay     time.Duration
	nodes     map[string]func([]byte)
	groups    []map[string]bool
	// With Record, every record sent, as the network saw it.
	recording bool
	recorded  [][]byte
}

// NewLawSpecMemoryNetwork makes an in-memory network with these faults.
func NewLawSpecMemoryNetwork(seed uint64, loss, duplicate float64, delay time.Duration) *LawSpecMemoryNetwork {
	return &LawSpecMemoryNetwork{random: LawSpecSplitMix64{seed}, loss: loss, duplicate: duplicate, delay: delay, nodes: map[string]func([]byte){}}
}

// Transport is a node's transport on the network, at mem://name.
func (n *LawSpecMemoryNetwork) Transport(name string) LawSpecNetTransport {
	return &lawSpecMemoryTransport{n, "mem://" + name}
}

// InsecureTransportForTests is a transport at mem://name whose node skips
// the handshake and sends frames in the clear: for tests of the frame layer
// only. Only an in-memory network makes one, and no configuration selects
// it.
func (n *LawSpecMemoryNetwork) InsecureTransportForTests(name string) LawSpecNetTransport {
	return &LawSpecInsecureMemoryTransport{lawSpecMemoryTransport{n, "mem://" + name}}
}

// Record keeps every record sent from now on (see Recorded), and returns
// the network.
func (n *LawSpecMemoryNetwork) Record() *LawSpecMemoryNetwork {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.recording = true
	return n
}

// Recorded is every record sent while recording, as the network saw it.
func (n *LawSpecMemoryNetwork) Recorded() [][]byte {
	n.mu.Lock()
	defer n.mu.Unlock()
	return append([][]byte{}, n.recorded...)
}

// Partition lets only nodes named in the same group reach each other.
func (n *LawSpecMemoryNetwork) Partition(groups ...[]string) {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.groups = nil
	for _, g := range groups {
		set := map[string]bool{}
		for _, name := range g {
			set["mem://"+name] = true
		}
		n.groups = append(n.groups, set)
	}
}

// Heal ends a partition.
func (n *LawSpecMemoryNetwork) Heal() {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.groups = nil
}

func (n *LawSpecMemoryNetwork) chance(p float64) bool {
	return p > 0 && float64(n.random.Below(1<<30)) < p*float64(1<<30)
}

func (n *LawSpecMemoryNetwork) send(source, node string, frame []byte) error {
	n.mu.Lock()
	if n.recording {
		n.recorded = append(n.recorded, append([]byte{}, frame...))
	}
	deliver, ok := n.nodes[node]
	if !ok {
		n.mu.Unlock()
		return lsUnreachable("no node at " + node)
	}
	if n.groups != nil {
		together := false
		for _, g := range n.groups {
			together = together || (g[source] && g[node])
		}
		if !together {
			n.mu.Unlock()
			return nil
		}
	}
	if n.chance(n.loss) {
		n.mu.Unlock()
		return nil
	}
	copies := 1
	if n.chance(n.duplicate) {
		copies = 2
	}
	delays := []time.Duration{}
	for k := 0; k < copies; k++ {
		delays = append(delays, time.Duration(n.random.Below(1001))*n.delay/1000)
	}
	n.mu.Unlock()
	for _, wait := range delays {
		copied := append([]byte{}, frame...)
		go func(wait time.Duration) {
			if wait > 0 {
				time.Sleep(wait)
			}
			deliver(copied)
		}(wait)
	}
	return nil
}

type lawSpecMemoryTransport struct {
	network *LawSpecMemoryNetwork
	address string
}

func (t *lawSpecMemoryTransport) Address() string { return t.address }
func (t *lawSpecMemoryTransport) Start(deliver func([]byte)) {
	t.network.mu.Lock()
	t.network.nodes[t.address] = deliver
	t.network.mu.Unlock()
}
func (t *lawSpecMemoryTransport) Send(node string, frame []byte) error {
	return t.network.send(t.address, node, frame)
}
func (t *lawSpecMemoryTransport) Close() {
	t.network.mu.Lock()
	delete(t.network.nodes, t.address)
	t.network.mu.Unlock()
}

// LawSpecInsecureMemoryTransport is in memory, without the handshake: tests
// only (LawSpecMemoryNetwork.InsecureTransportForTests).
type LawSpecInsecureMemoryTransport struct{ lawSpecMemoryTransport }

// lawSpecTcpTransport sends frames over TCP, each a 4-byte big-endian
// length then the frame.
type lawSpecTcpTransport struct {
	listener net.Listener
	address  string
	mu       sync.Mutex
	conns    map[string]net.Conn
	inbound  map[net.Conn]bool
	closed   atomic.Bool
}

// NewLawSpecTcpTransport listens on host:port (port 0 picks a free one);
// its address is tcp://host:port.
func NewLawSpecTcpTransport(host string, port int) (LawSpecNetTransport, error) {
	listener, err := net.Listen("tcp", net.JoinHostPort(host, strconv.Itoa(port)))
	if err != nil {
		return nil, err
	}
	actual := listener.Addr().(*net.TCPAddr).Port
	return &lawSpecTcpTransport{listener: listener, address: fmt.Sprintf("tcp://%s:%d", host, actual),
		conns: map[string]net.Conn{}, inbound: map[net.Conn]bool{}}, nil
}

func (t *lawSpecTcpTransport) Address() string { return t.address }

func (t *lawSpecTcpTransport) Start(deliver func([]byte)) {
	go func() {
		for {
			conn, err := t.listener.Accept()
			if err != nil {
				return
			}
			t.mu.Lock()
			t.inbound[conn] = true
			t.mu.Unlock()
			go func() {
				defer conn.Close()
				header := make([]byte, 4)
				for {
					if _, err := io.ReadFull(conn, header); err != nil {
						return
					}
					frame := make([]byte, binary.BigEndian.Uint32(header))
					if _, err := io.ReadFull(conn, frame); err != nil {
						return
					}
					deliver(frame)
				}
			}()
		}
	}()
}

func (t *lawSpecTcpTransport) Send(node string, frame []byte) error {
	data := binary.BigEndian.AppendUint32(nil, uint32(len(frame)))
	data = append(data, frame...)
	t.mu.Lock()
	defer t.mu.Unlock()
	var last error
	for attempt := 0; attempt < 2; attempt++ {
		conn := t.conns[node]
		if conn == nil {
			var err error
			conn, err = net.DialTimeout("tcp", strings.TrimPrefix(node, "tcp://"), 5*time.Second)
			if err != nil {
				last = err
				continue
			}
			t.conns[node] = conn
		}
		if _, err := conn.Write(data); err != nil {
			conn.Close()
			delete(t.conns, node)
			last = err
			continue
		}
		return nil
	}
	return lsUnreachable(fmt.Sprintf("cannot reach %s: %v", node, last))
}

func (t *lawSpecTcpTransport) Close() {
	t.closed.Store(true)
	t.listener.Close()
	t.mu.Lock()
	defer t.mu.Unlock()
	for _, conn := range t.conns {
		conn.Close()
	}
	for conn := range t.inbound {
		conn.Close()
	}
	t.conns = map[string]net.Conn{}
}

// lawSpecHttpTransport sends frames as HTTP POST bodies to /lawspec.
type lawSpecHttpTransport struct {
	listener net.Listener
	server   *http.Server
	address  string
	client   *http.Client
}

// NewLawSpecHttpTransport listens on host:port (port 0 picks a free one);
// its address is http://host:port.
func NewLawSpecHttpTransport(host string, port int) (LawSpecNetTransport, error) {
	listener, err := net.Listen("tcp", net.JoinHostPort(host, strconv.Itoa(port)))
	if err != nil {
		return nil, err
	}
	actual := listener.Addr().(*net.TCPAddr).Port
	return &lawSpecHttpTransport{listener: listener, address: fmt.Sprintf("http://%s:%d", host, actual),
		client: &http.Client{Timeout: 5 * time.Second}}, nil
}

func (t *lawSpecHttpTransport) Address() string { return t.address }

func (t *lawSpecHttpTransport) Start(deliver func([]byte)) {
	mux := http.NewServeMux()
	mux.HandleFunc("/lawspec", func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			w.WriteHeader(http.StatusNotFound)
			return
		}
		body, err := io.ReadAll(r.Body)
		w.WriteHeader(http.StatusNoContent)
		if err == nil {
			deliver(body)
		}
	})
	t.server = &http.Server{Handler: mux}
	go t.server.Serve(t.listener)
}

func (t *lawSpecHttpTransport) Send(node string, frame []byte) error {
	response, err := t.client.Post(node+"/lawspec", "application/octet-stream", bytes.NewReader(frame))
	if err != nil {
		return lsUnreachable(fmt.Sprintf("cannot reach %s: %v", node, err))
	}
	io.Copy(io.Discard, response.Body)
	response.Body.Close()
	return nil
}

func (t *lawSpecHttpTransport) Close() {
	if t.server != nil {
		t.server.Close()
	} else {
		t.listener.Close()
	}
	t.client.CloseIdleConnections()
}

// The secure network handler lives in lawspec_network.go, which the
// compiler writes beside this runtime when a program imports
// lawspec.network: it needs ML-KEM, ML-DSA, SHA3 and AES-GCM, which programs
// without nodes do without. It registers itself here in its init
// (lsSecureNetwork); a node on any transport but the one made for tests only
// asks it for the node's secure layer.

// lawSpecSecureChannel is a node's secure layer: send seals a frame for a
// peer (or queues it behind a handshake), receive gives the frame a record
// carries, if any, and identity is the node's identity.
type lawSpecSecureChannel interface {
	send(peer string, frame []byte) error
	receive(record []byte) ([]byte, bool)
	nodeIdentity() any
}

// lsSecureNetwork makes a node's secure layer, given the identity and
// trusted fingerprints its options name (nil for the configured ones); nil
// until lawspec_network.go registers it.
var lsSecureNetwork func(node *LawSpecNode, identity any, trusted map[string]bool) lawSpecSecureChannel

// LawSpecSecureToken is a one-time token from the operating system's secure
// generator: 32 bytes as 64 hexadecimal digits, as SecureRandom's
// secureToken gives.
func LawSpecSecureToken() string {
	random := make([]byte, 32)
	if _, err := cryptorand.Read(random); err != nil {
		panic(err)
	}
	return hex.EncodeToString(random)
}

// LawSpecNodeOption is an option of NewLawSpecNode.
type LawSpecNodeOption func(*lawSpecNodeOptions)

type lawSpecNodeOptions struct {
	identity any
	// nil when no option names them.
	trusted map[string]bool
}

// LawSpecNodeWithIdentity gives the node this identity, a
// *LawSpecNodeIdentity of lawspec_network.go (by default the one
// lawspec.json binds, or a fresh one).
func LawSpecNodeWithIdentity(identity any) LawSpecNodeOption {
	return func(options *lawSpecNodeOptions) { options.identity = identity }
}

// LawSpecNodeTrusting has the node talk only to peers with these
// fingerprints (by default any peer, each address keeping the first
// identity it shows).
func LawSpecNodeTrusting(fingerprints ...string) LawSpecNodeOption {
	return func(options *lawSpecNodeOptions) {
		options.trusted = map[string]bool{}
		for _, fingerprint := range fingerprints {
			options.trusted[strings.ToLower(fingerprint)] = true
		}
	}
}

// lawSpecEntity is something a node names: a mailbox, an actor, a channel
// end or definitions.
type lawSpecEntity interface {
	receive(node *LawSpecNode, kind, source string, id uint64, payload []byte)
}

// LawSpecNode is a process's presence on a network: it names local
// mailboxes, actors, channel ends and definitions, so other nodes can reach
// them at <node address>/<name>, and it sends to theirs. Order is kept
// within one channel; a mailbox send is best effort, and a call is resent
// until answered (and run once), failing with LawSpecUnreachable after its
// timeout.
//
// Every target's node speaks the same frames, so nodes written in different
// languages talk to each other. ref:DEC-distribution-canonical-wire
type LawSpecNode struct {
	transport LawSpecNetTransport
	address   string
	mu        sync.Mutex
	entities  map[string]lawSpecEntity
	pending   map[uint64]chan []byte
	seen      map[string][]byte
	seenOrder []string
	nextID    uint64
	closed    chan struct{}
	closing   sync.Once
	// The handshakes and sessions of the secure network handler; nil on a
	// transport made for tests only (InsecureTransportForTests).
	secure lawSpecSecureChannel
}

// NewLawSpecNode starts a node on a transport. Options give its identity
// (LawSpecNodeWithIdentity) and the only peers it talks to
// (LawSpecNodeTrusting). A transport made for tests only
// (LawSpecMemoryNetwork.InsecureTransportForTests) skips the handshake; no
// other transport can.
func NewLawSpecNode(transport LawSpecNetTransport, options ...LawSpecNodeOption) *LawSpecNode {
	n := &LawSpecNode{transport: transport, address: transport.Address(), entities: map[string]lawSpecEntity{},
		pending: map[uint64]chan []byte{}, seen: map[string][]byte{}, closed: make(chan struct{})}
	if _, insecure := transport.(*LawSpecInsecureMemoryTransport); !insecure {
		if lsSecureNetwork == nil {
			panic("a node needs the secure network handler: add `import lawspec.network` to a unit of the program")
		}
		chosen := lawSpecNodeOptions{}
		for _, option := range options {
			option(&chosen)
		}
		n.secure = lsSecureNetwork(n, chosen.identity, chosen.trusted)
	}
	transport.Start(n.arrive)
	return n
}

// Address is the node's address, such as tcp://127.0.0.1:7000.
func (n *LawSpecNode) Address() string { return n.address }

// Identity is the node's identity (a *LawSpecNodeIdentity of
// lawspec_network.go), or nil on a transport made for tests only.
func (n *LawSpecNode) Identity() any {
	if n.secure == nil {
		return nil
	}
	return n.secure.nodeIdentity()
}

// arrive takes a record from the transport: the frame it carries, if any,
// goes on to deliver.
func (n *LawSpecNode) arrive(record []byte) {
	if n.secure == nil {
		n.deliver(record)
		return
	}
	if frame, ok := n.secure.receive(record); ok {
		n.deliver(frame)
	}
}

// transmit sends a frame to a node: sealed, or in the clear on a transport
// made for tests only.
func (n *LawSpecNode) transmit(node string, frame []byte) error {
	if n.secure == nil {
		return n.transport.Send(node, frame)
	}
	return n.secure.send(node, frame)
}

// Close stops the node's transport, and its channel ends' resending.
func (n *LawSpecNode) Close() {
	n.closing.Do(func() {
		close(n.closed)
		n.transport.Close()
	})
}

// forward passes a frame on to address unchanged, keeping its source.
func (n *LawSpecNode) forward(address, kind, source string, id uint64, payload []byte) {
	node, name, err := lsSplitAddress(address)
	if err != nil {
		return
	}
	n.transmit(node, lsFrameEncode(kind, name, source, id, payload))
}

func (n *LawSpecNode) newID() uint64 {
	n.mu.Lock()
	defer n.mu.Unlock()
	n.nextID++
	return n.nextID
}

func (n *LawSpecNode) send(address, kind string, payload []byte, id uint64) error {
	node, name, err := lsSplitAddress(address)
	if err != nil {
		return err
	}
	return n.transmit(node, lsFrameEncode(kind, name, n.address, id, payload))
}

func (n *LawSpecNode) register(name string, entity lawSpecEntity) (string, error) {
	if name == "" || strings.Contains(name, "/") {
		return "", fmt.Errorf("%q is not a name: use letters, digits and dashes", name)
	}
	n.mu.Lock()
	defer n.mu.Unlock()
	if _, taken := n.entities[name]; taken {
		return "", fmt.Errorf("%s is already registered on %s", name, n.address)
	}
	n.entities[name] = entity
	return n.address + "/" + name, nil
}

func (n *LawSpecNode) deliver(frame []byte) {
	kind, to, source, id, payload, err := lsFrameDecode(frame)
	if err != nil {
		return
	}
	if kind == "reply" {
		n.mu.Lock()
		slot, ok := n.pending[id]
		delete(n.pending, id)
		n.mu.Unlock()
		if ok {
			slot <- payload
		}
		return
	}
	n.mu.Lock()
	entity := n.entities[to]
	n.mu.Unlock()
	if entity == nil {
		if id != 0 {
			n.reply(source, id, 3, []byte(fmt.Sprintf("nothing is registered as %s on %s", to, n.address)))
		}
		return
	}
	if id != 0 {
		// A request sent again (lost reply, duplicated frame) is answered
		// again without running twice.
		key := source + "\x00" + strconv.FormatUint(id, 10)
		n.mu.Lock()
		if answer, ok := n.seen[key]; ok {
			n.mu.Unlock()
			if answer != nil {
				go n.send(source+"/", "reply", answer, id)
			}
			return
		}
		n.seen[key] = nil
		n.seenOrder = append(n.seenOrder, key)
		if len(n.seenOrder) > 10000 {
			for _, old := range n.seenOrder[:5000] {
				delete(n.seen, old)
			}
			n.seenOrder = append([]string{}, n.seenOrder[5000:]...)
		}
		n.mu.Unlock()
	}
	go entity.receive(n, kind, source, id, payload)
}

func (n *LawSpecNode) reply(source string, id uint64, status byte, body []byte) {
	payload := append([]byte{status}, body...)
	key := source + "\x00" + strconv.FormatUint(id, 10)
	n.mu.Lock()
	if _, ok := n.seen[key]; ok {
		n.seen[key] = payload
	}
	n.mu.Unlock()
	n.send(source+"/", "reply", payload, id)
}

// request sends a request until it is answered, and returns the reply's
// status and body.
func (n *LawSpecNode) request(address, kind string, payload []byte, timeout time.Duration) (byte, []byte, error) {
	id := n.newID()
	slot := make(chan []byte, 1)
	n.mu.Lock()
	n.pending[id] = slot
	n.mu.Unlock()
	giveUp := time.Now().Add(timeout)
	for {
		n.send(address, kind, payload, id)
		wait := time.Until(giveUp)
		if wait > 100*time.Millisecond {
			wait = 100 * time.Millisecond
		}
		if wait < 0 {
			wait = 0
		}
		select {
		case answer := <-slot:
			if len(answer) == 0 {
				return 3, nil, nil
			}
			return answer[0], answer[1:], nil
		case <-time.After(wait):
		}
		if !time.Now().Before(giveUp) {
			n.mu.Lock()
			delete(n.pending, id)
			n.mu.Unlock()
			return 0, nil, lsUnreachable(fmt.Sprintf("%s did not answer within %v", address, timeout))
		}
	}
}

func lsReplyValue(status byte, body []byte, values lawSpecValues, d any) (LawSpecValue, error) {
	switch status {
	case 0:
		return LawSpecWireDecode(values, d, body)
	case 1:
		return LawSpecValue{}, LawSpecActorCrashed{string(body)}
	case 2:
		return LawSpecValue{}, fmt.Errorf("%w: %s", LawSpecActorStopped, body)
	}
	return LawSpecValue{}, lsUnreachable(string(body))
}

type lawSpecMailEntity struct {
	box        *LawSpecMailbox
	values     lawSpecValues
	descriptor any
}

func (e *lawSpecMailEntity) receive(node *LawSpecNode, kind, source string, id uint64, payload []byte) {
	if kind != "mail" {
		return
	}
	status, body := byte(0), []byte{}
	if v, err := LawSpecWireDecode(e.values, e.descriptor, payload); err != nil {
		status, body = 3, []byte("not a message of this mailbox: "+err.Error())
	} else if err := e.box.Send(v); err != nil {
		status, body = 2, []byte(err.Error())
	}
	if id != 0 {
		node.reply(source, id, status, body)
	}
}

// Mailbox is a local mailbox that other nodes send values of one type to,
// at <node address>/name; its messages are LawSpecValues.
func (n *LawSpecNode) Mailbox(name string, descriptor any, values lawSpecValues) (*LawSpecMailbox, error) {
	box := NewLawSpecMailbox()
	if _, err := n.register(name, &lawSpecMailEntity{box, values, descriptor}); err != nil {
		return nil, err
	}
	return box, nil
}

// LawSpecRemoteMailbox sends to a mailbox on another node. A send waits
// until the mailbox has the message (resending a lost one; the mailbox
// takes it once), and fails with LawSpecUnreachable after the timeout, or
// LawSpecActorStopped if the mailbox is closed.
type LawSpecRemoteMailbox struct {
	node       *LawSpecNode
	address    string
	descriptor any
	values     lawSpecValues
	timeout    time.Duration
}

// RemoteMailbox is the mailbox at address on another node; a zero timeout
// means 5 seconds.
func (n *LawSpecNode) RemoteMailbox(address string, descriptor any, values lawSpecValues, timeout ...time.Duration) *LawSpecRemoteMailbox {
	wait := 5 * time.Second
	if len(timeout) > 0 && timeout[0] > 0 {
		wait = timeout[0]
	}
	return &LawSpecRemoteMailbox{n, address, descriptor, values, wait}
}

// Send sends a value and waits until the mailbox has it.
func (m *LawSpecRemoteMailbox) Send(value LawSpecValue) error {
	encoded, err := LawSpecWireEncode(m.values, m.descriptor, value)
	if err != nil {
		return err
	}
	status, body, err := m.node.request(m.address, "mail", encoded, m.timeout)
	if err != nil {
		return err
	}
	if status != 0 {
		_, err := lsReplyValue(status, body, m.values, []any{"unit"})
		return err
	}
	return nil
}

// LawSpecRemoteHandler is how a served actor handles a message from another
// node: Handle takes the state and the arguments and gives the reply and the
// next state.
type LawSpecRemoteHandler struct {
	Handle    func(state any, args []LawSpecValue) (LawSpecValue, any)
	Arguments []any
	Reply     any
}

// LawSpecSignature is a message's argument and reply descriptors.
type LawSpecSignature struct {
	Arguments []any
	Reply     any
}

type lawSpecActorEntity struct {
	actor    *LawSpecActor
	handlers map[string]LawSpecRemoteHandler
	values   lawSpecValues
}

func lsDecodeArguments(values lawSpecValues, arguments []any, payload []byte, pos int) ([]LawSpecValue, error) {
	args := []LawSpecValue{}
	for _, d := range arguments {
		v, next, err := lsWireGet(values, d, payload, pos)
		if err != nil {
			return nil, err
		}
		pos = next
		args = append(args, v)
	}
	if pos != len(payload) {
		return nil, LawSpecWireError{"extra bytes after the arguments"}
	}
	return args, nil
}

func (e *lawSpecActorEntity) receive(node *LawSpecNode, kind, source string, id uint64, payload []byte) {
	if kind != "call" {
		return
	}
	message, pos, err := lsGetText(payload, 0)
	handler, known := e.handlers[message]
	var args []LawSpecValue
	if err == nil && known {
		args, err = lsDecodeArguments(e.values, handler.Arguments, payload, pos)
	}
	if err != nil || !known {
		node.reply(source, id, 3, []byte(fmt.Sprintf("not a message this actor handles: %s", message)))
		return
	}
	reply, err := e.actor.Call(func(state any) (any, any) { return handler.Handle(state, args) })
	if err != nil {
		var crashed LawSpecActorCrashed
		if errors.As(err, &crashed) {
			node.reply(source, id, 1, []byte(err.Error()))
		} else {
			node.reply(source, id, 2, []byte(err.Error()))
		}
		return
	}
	encoded, err := LawSpecWireEncode(e.values, handler.Reply, reply.(LawSpecValue))
	if err != nil {
		node.reply(source, id, 1, []byte(err.Error()))
		return
	}
	node.reply(source, id, 0, encoded)
}

// Serve lets other nodes call actor at <node address>/name, and returns
// that address.
func (n *LawSpecNode) Serve(name string, actor *LawSpecActor, handlers map[string]LawSpecRemoteHandler, values lawSpecValues) (string, error) {
	return n.register(name, &lawSpecActorEntity{actor, handlers, values})
}

// LawSpecRemoteActor calls an actor on another node.
type LawSpecRemoteActor struct {
	node       *LawSpecNode
	address    string
	signatures map[string]LawSpecSignature
	values     lawSpecValues
	timeout    time.Duration
}

// RemoteActor is a proxy calling the actor served at address.
func (n *LawSpecNode) RemoteActor(address string, signatures map[string]LawSpecSignature, values lawSpecValues, timeout time.Duration) *LawSpecRemoteActor {
	return &LawSpecRemoteActor{n, address, signatures, values, timeout}
}

// Call sends a message and waits for the reply: LawSpecUnreachable after
// the timeout, or what the actor's call failed with (LawSpecActorCrashed,
// LawSpecActorStopped).
func (r *LawSpecRemoteActor) Call(message string, args ...LawSpecValue) (LawSpecValue, error) {
	signature, ok := r.signatures[message]
	if !ok {
		return LawSpecValue{}, fmt.Errorf("the actor at %s has no message %s", r.address, message)
	}
	payload := lsPutText(nil, message)
	for i, d := range signature.Arguments {
		var err error
		if payload, err = lsWirePut(r.values, d, args[i], payload); err != nil {
			return LawSpecValue{}, err
		}
	}
	status, body, err := r.node.request(r.address, "call", payload, r.timeout)
	if err != nil {
		return LawSpecValue{}, err
	}
	return lsReplyValue(status, body, r.values, signature.Reply)
}

// LawSpecRemoteDefinition is a checked definition other nodes can evaluate.
type LawSpecRemoteDefinition struct {
	Function  func(args []LawSpecValue) LawSpecValue
	Arguments []any
	Result    any
}

type lawSpecDefinitionEntity struct {
	table  map[string]LawSpecRemoteDefinition
	values lawSpecValues
}

func (e *lawSpecDefinitionEntity) receive(node *LawSpecNode, kind, source string, id uint64, payload []byte) {
	if kind != "eval" {
		return
	}
	digest, pos, err := lsGetText(payload, 0)
	definition, known := e.table[digest]
	var args []LawSpecValue
	if err == nil && known {
		args, err = lsDecodeArguments(e.values, definition.Arguments, payload, pos)
	}
	if err != nil || !known {
		node.reply(source, id, 3, []byte("this node has no definition with that content hash"))
		return
	}
	result, failure := func() (result LawSpecValue, failure any) {
		defer func() { failure = recover() }()
		return definition.Function(args), nil
	}()
	if failure != nil {
		node.reply(source, id, 1, []byte(fmt.Sprint(failure)))
		return
	}
	encoded, err := LawSpecWireEncode(e.values, definition.Result, result)
	if err != nil {
		node.reply(source, id, 1, []byte(err.Error()))
		return
	}
	node.reply(source, id, 0, encoded)
}

// ServeDefinitions lets other nodes evaluate definitions, by content hash,
// at <node address>/name (name is "definitions" by convention).
func (n *LawSpecNode) ServeDefinitions(table map[string]LawSpecRemoteDefinition, values lawSpecValues, name string) (string, error) {
	return n.register(name, &lawSpecDefinitionEntity{table, values})
}

// Evaluate evaluates the definition with this content hash on the node at
// nodeAddress.
func (n *LawSpecNode) Evaluate(nodeAddress, digest string, args []LawSpecValue, arguments []any, result any, values lawSpecValues, timeout time.Duration, name string) (LawSpecValue, error) {
	payload := lsPutText(nil, digest)
	for i, d := range arguments {
		var err error
		if payload, err = lsWirePut(values, d, args[i], payload); err != nil {
			return LawSpecValue{}, err
		}
	}
	status, body, err := n.request(nodeAddress+"/"+name, "eval", payload, timeout)
	if err != nil {
		return LawSpecValue{}, err
	}
	return lsReplyValue(status, body, values, result)
}

// LawSpecWireStep is one step of a channel end: whether it sends, and its
// value's descriptor.
type LawSpecWireStep struct {
	Sends      bool
	Descriptor any
}

// LawSpecNetEndpoint is one end of a channel between nodes, a
// LawSpecTransport over LawSpecValues. Each value travels in a numbered
// frame that is sent again until acknowledged, so loss, duplication and
// reordering are repaired; a peer silent for the deadline is treated as
// failed (LawSpecPeerFailed). Order is kept within the channel.
//
// An unused end can move to another node: offer gives the address the new
// node takes it over from (<address>?take=<token>). On a take frame with
// that token, this end hands its state over (a state frame) and from then
// on forwards every frame it gets to the new end; the new end tells the
// peer (a moved frame) so the peer sends to it directly.
type LawSpecNetEndpoint struct {
	node     *LawSpecNode
	steps    []LawSpecWireStep
	values   lawSpecValues
	deadline time.Duration
	address  string
	mu       sync.Mutex
	peer     string
	out      int64
	unacked  map[int64]*lawSpecUnacked
	expected int64
	early    map[int64][]byte
	inbox    chan lawSpecInbound
	step     int
	gone     bool
	failure  string
	stop     chan struct{}
	// Moving: the addresses this end had before (oldest first), the token
	// a taker must show, where the end went and the state frame it was
	// given, and, on the new node, the takeover in progress.
	history     []string
	token       string
	movedTo     string
	state       []byte
	taking      bool
	takeToken   string
	taken       chan struct{}
	isTaken     bool
	announcing  bool
	announcedAt time.Time
	confirmed   chan struct{}
	isConfirmed bool
}

type lawSpecUnacked struct {
	payload   []byte
	first, at time.Time
	body      []byte
}

type lawSpecInbound struct {
	failure string
	body    []byte
}

func lsNewNetEndpoint(node *LawSpecNode, steps []LawSpecWireStep, values lawSpecValues, deadline time.Duration) *LawSpecNetEndpoint {
	e := &LawSpecNetEndpoint{node: node, steps: steps, values: values, deadline: deadline,
		unacked: map[int64]*lawSpecUnacked{}, early: map[int64][]byte{}, inbox: make(chan lawSpecInbound, 4096), stop: make(chan struct{}),
		taken: make(chan struct{}), confirmed: make(chan struct{})}
	go e.resend()
	return e
}

// Address is where this end is registered.
func (e *LawSpecNetEndpoint) Address() string { return e.address }

// Listen is the first end of a channel named name on this node; another
// node Dials its address. steps are from this end's side.
func (n *LawSpecNode) Listen(name string, steps []LawSpecWireStep, values lawSpecValues, deadline time.Duration) (*LawSpecNetEndpoint, error) {
	e := lsNewNetEndpoint(n, steps, values, deadline)
	address, err := n.register(name, e)
	if err != nil {
		close(e.stop)
		return nil, err
	}
	e.address = address
	return e, nil
}

// Dial is the second end of the channel listening at address. steps are
// from this end's side.
func (n *LawSpecNode) Dial(address string, steps []LawSpecWireStep, values lawSpecValues, deadline time.Duration) (*LawSpecNetEndpoint, error) {
	e := lsNewNetEndpoint(n, steps, values, deadline)
	own, err := n.register(fmt.Sprintf("end-%d", n.newID()), e)
	if err != nil {
		close(e.stop)
		return nil, err
	}
	e.address = own
	e.mu.Lock()
	e.peer = address
	e.mu.Unlock()
	e.transmit(-1, []byte("hello"))
	return e, nil
}

// Take takes over a channel end another node moves here: address is
// <old address>?take=<token>, as that node offered it. It returns once the
// end's state has arrived and its peer has been told (or after the
// deadline; the old node then forwards to the end).
func (n *LawSpecNode) Take(address string, steps []LawSpecWireStep, values lawSpecValues, deadline time.Duration) (*LawSpecNetEndpoint, error) {
	e := lsNewNetEndpoint(n, steps, values, deadline)
	own, err := n.register(fmt.Sprintf("end-%d", n.newID()), e)
	if err != nil {
		close(e.stop)
		return nil, err
	}
	e.address = own
	e.takeOver(address)
	return e, nil
}

func (e *LawSpecNetEndpoint) frame(seq int64, body []byte) []byte {
	payload := lsInt64Bytes(nil, seq)
	payload = lsPutText(payload, e.address)
	return append(payload, body...)
}

func (e *LawSpecNetEndpoint) transmit(seq int64, body []byte) {
	payload := e.frame(seq, body)
	now := time.Now()
	e.mu.Lock()
	e.unacked[seq] = &lawSpecUnacked{payload, now, now, body}
	peer := e.peer
	e.mu.Unlock()
	if peer != "" {
		e.node.send(peer, "chan", payload, 0)
	}
}

func (e *LawSpecNetEndpoint) resend() {
	ticker := time.NewTicker(20 * time.Millisecond)
	defer ticker.Stop()
	for {
		select {
		case <-e.stop:
			return
		case <-e.node.closed:
			return
		case <-ticker.C:
		}
		now := time.Now()
		e.mu.Lock()
		if e.gone || e.movedTo != "" {
			e.mu.Unlock()
			return
		}
		if e.taking && !e.isTaken {
			e.mu.Unlock()
			continue
		}
		peer := e.peer
		due := [][]byte{}
		stale := false
		for _, entry := range e.unacked {
			if now.Sub(entry.at) > 50*time.Millisecond {
				stale = stale || now.Sub(entry.first) > e.deadline
				entry.at = now
				due = append(due, entry.payload)
			}
		}
		var moved []byte
		if e.announcing && peer != "" && !e.isConfirmed && now.Sub(e.announcedAt) > 50*time.Millisecond {
			e.announcedAt = now
			moved = lsPutTexts(nil, e.history)
			moved = lsPutText(moved, e.address)
		}
		e.mu.Unlock()
		if stale {
			e.fail("the other end did not answer in time (unreachable)")
			return
		}
		if peer == "" {
			continue
		}
		if moved != nil {
			e.node.send(peer, "moved", moved, 0)
		}
		for _, payload := range due {
			e.node.send(peer, "chan", payload, 0)
		}
	}
}

func (e *LawSpecNetEndpoint) fail(reason string) {
	e.mu.Lock()
	if e.gone {
		e.mu.Unlock()
		return
	}
	e.gone = true
	e.failure = reason
	e.unacked = map[int64]*lawSpecUnacked{}
	e.mu.Unlock()
	e.inbox <- lawSpecInbound{failure: reason}
}

func (e *LawSpecNetEndpoint) receive(node *LawSpecNode, kind, source string, id uint64, payload []byte) {
	if kind == "take" {
		e.give(payload)
		return
	}
	e.mu.Lock()
	forward := e.movedTo
	waiting := e.taking && !e.isTaken
	e.mu.Unlock()
	if forward != "" {
		node.forward(forward, kind, source, id, payload)
		return
	}
	if waiting {
		// Until the state arrives, frames are dropped: their senders send
		// them again.
		if kind == "state" {
			e.install(payload)
		}
		return
	}
	switch kind {
	case "ack":
		if seq, _, err := lsGetInt64(payload, 0); err == nil {
			e.mu.Lock()
			delete(e.unacked, seq)
			e.mu.Unlock()
		}
		return
	case "moved":
		e.peerMoved(payload)
		return
	case "moved-ack":
		if to, _, err := lsGetText(payload, 0); err == nil && to == e.address {
			e.mu.Lock()
			if !e.isConfirmed {
				e.isConfirmed = true
				close(e.confirmed)
			}
			e.mu.Unlock()
		}
		return
	case "chan":
	default:
		return
	}
	seq, pos, err := lsGetInt64(payload, 0)
	if err != nil {
		return
	}
	sender, pos, err := lsGetText(payload, pos)
	if err != nil {
		return
	}
	body := append([]byte{}, payload[pos:]...)
	e.mu.Lock()
	if e.movedTo != "" {
		// Moved meanwhile: the new end acknowledges it.
		forward = e.movedTo
		e.mu.Unlock()
		node.forward(forward, kind, source, id, payload)
		return
	}
	if seq == -1 {
		if e.peer == "" {
			e.peer = sender
		}
	} else if _, waiting := e.early[seq]; seq >= e.expected && !waiting {
		e.early[seq] = body
		for {
			next, ok := e.early[e.expected]
			if !ok {
				break
			}
			delete(e.early, e.expected)
			e.inbox <- lawSpecInbound{body: next}
			e.expected++
		}
	}
	e.mu.Unlock()
	node.send(sender, "ack", lsInt64Bytes(nil, seq), 0)
}

func lsPutTexts(out []byte, texts []string) []byte {
	out = lsPutUvarint(out, uint64(len(texts)))
	for _, t := range texts {
		out = lsPutText(out, t)
	}
	return out
}

func lsGetTexts(buf []byte, pos int) ([]string, int, error) {
	count, pos, err := lsGetUvarint(buf, pos)
	if err != nil {
		return nil, pos, err
	}
	texts := []string{}
	for i := uint64(0); i < count; i++ {
		var t string
		if t, pos, err = lsGetText(buf, pos); err != nil {
			return nil, pos, err
		}
		texts = append(texts, t)
	}
	return texts, pos, nil
}

type lsNumbered struct {
	seq  int64
	body []byte
}

func lsPutNumbered(out []byte, items []lsNumbered) []byte {
	sort.Slice(items, func(i, j int) bool { return items[i].seq < items[j].seq })
	out = lsPutUvarint(out, uint64(len(items)))
	for _, item := range items {
		out = lsInt64Bytes(out, item.seq)
		out = lsPutUvarint(out, uint64(len(item.body)))
		out = append(out, item.body...)
	}
	return out
}

func lsGetNumbered(buf []byte, pos int) ([]lsNumbered, int, error) {
	count, pos, err := lsGetUvarint(buf, pos)
	if err != nil {
		return nil, pos, err
	}
	items := []lsNumbered{}
	for i := uint64(0); i < count; i++ {
		var seq int64
		var body []byte
		if seq, pos, err = lsGetInt64(buf, pos); err != nil {
			return nil, pos, err
		}
		if body, pos, err = lsGetRaw(buf, pos); err != nil {
			return nil, pos, err
		}
		items = append(items, lsNumbered{seq, append([]byte{}, body...)})
	}
	return items, pos, nil
}

// offer is the address another node takes this unused end over from.
func (e *LawSpecNetEndpoint) offer() string {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.token == "" {
		e.token = LawSpecSecureToken()
	}
	return e.address + "?take=" + e.token
}

// give answers a take frame: it hands the state over once, to the first
// taker with the token, and answers that taker's repeats with the same
// state.
func (e *LawSpecNetEndpoint) give(payload []byte) {
	token, pos, err := lsGetText(payload, 0)
	if err != nil {
		return
	}
	taker, _, err := lsGetText(payload, pos)
	if err != nil {
		return
	}
	e.mu.Lock()
	if e.token == "" || token != e.token {
		e.mu.Unlock()
		return
	}
	if e.movedTo == "" {
		// The end is unused, so nothing else takes from its inbox.
		received := [][]byte{}
	drain:
		for {
			select {
			case inbound := <-e.inbox:
				if inbound.failure == "" {
					received = append(received, inbound.body)
				}
			default:
				break drain
			}
		}
		state := lsPutText(nil, token)
		state = lsPutText(state, e.failure)
		state = lsPutText(state, e.peer)
		state = lsPutTexts(state, append(append([]string{}, e.history...), e.address))
		state = lsInt64Bytes(state, e.out)
		state = lsInt64Bytes(state, e.expected)
		unacked := []lsNumbered{}
		for seq, entry := range e.unacked {
			unacked = append(unacked, lsNumbered{seq, entry.body})
		}
		state = lsPutNumbered(state, unacked)
		early := []lsNumbered{}
		for seq, body := range e.early {
			early = append(early, lsNumbered{seq, body})
		}
		state = lsPutNumbered(state, early)
		state = lsPutUvarint(state, uint64(len(received)))
		for _, body := range received {
			state = lsPutUvarint(state, uint64(len(body)))
			state = append(state, body...)
		}
		e.movedTo, e.state = taker, state
		e.unacked = map[int64]*lawSpecUnacked{}
		e.early = map[int64][]byte{}
	} else if e.movedTo != taker {
		e.mu.Unlock()
		return
	}
	state := e.state
	e.mu.Unlock()
	e.node.send(taker, "state", state, 0)
}

// takeOver takes over the end offered at address (<old address>?take=<token>):
// it asks for the end's state until it comes, then tells the peer where the
// end is now. It returns once the peer knows, or after the deadline (the old
// node then keeps forwarding to this end, as a relay would).
func (e *LawSpecNetEndpoint) takeOver(address string) {
	old, query, _ := strings.Cut(address, "?")
	token := strings.TrimPrefix(query, "take=")
	e.mu.Lock()
	e.taking, e.takeToken = true, token
	e.mu.Unlock()
	request := lsPutText(nil, token)
	request = lsPutText(request, e.address)
	giveUp := time.NewTimer(e.deadline)
	defer giveUp.Stop()
	for {
		e.node.send(old, "take", request, 0)
		select {
		case <-e.taken:
		case <-time.After(50 * time.Millisecond):
			select {
			case <-giveUp.C:
				e.mu.Lock()
				e.taking = false
				e.mu.Unlock()
				e.fail("the node the end came from did not hand it over in time (unreachable)")
				return
			default:
			}
			continue
		}
		break
	}
	select {
	case <-e.confirmed:
	case <-giveUp.C:
	}
}

func (e *LawSpecNetEndpoint) install(payload []byte) {
	token, pos, err := lsGetText(payload, 0)
	if err != nil {
		return
	}
	var failure, peer string
	var history []string
	var out, expected int64
	var unacked, early []lsNumbered
	var count uint64
	if failure, pos, err = lsGetText(payload, pos); err != nil {
		return
	}
	if peer, pos, err = lsGetText(payload, pos); err != nil {
		return
	}
	if history, pos, err = lsGetTexts(payload, pos); err != nil {
		return
	}
	if out, pos, err = lsGetInt64(payload, pos); err != nil {
		return
	}
	if expected, pos, err = lsGetInt64(payload, pos); err != nil {
		return
	}
	if unacked, pos, err = lsGetNumbered(payload, pos); err != nil {
		return
	}
	if early, pos, err = lsGetNumbered(payload, pos); err != nil {
		return
	}
	if count, pos, err = lsGetUvarint(payload, pos); err != nil {
		return
	}
	received := [][]byte{}
	for i := uint64(0); i < count; i++ {
		var body []byte
		if body, pos, err = lsGetRaw(payload, pos); err != nil {
			return
		}
		received = append(received, append([]byte{}, body...))
	}
	now := time.Now()
	e.mu.Lock()
	if !e.taking || token != e.takeToken || e.isTaken {
		e.mu.Unlock()
		return
	}
	e.peer = peer
	e.history = history
	e.out, e.expected = out, expected
	// Sent again from here at once, under this end's address.
	for _, item := range unacked {
		e.unacked[item.seq] = &lawSpecUnacked{e.frame(item.seq, item.body), now, time.Time{}, item.body}
	}
	for _, item := range early {
		e.early[item.seq] = item.body
	}
	for _, body := range received {
		e.inbox <- lawSpecInbound{body: body}
	}
	e.announcing = true
	e.isTaken = true
	close(e.taken)
	e.mu.Unlock()
	if failure != "" {
		e.fail(failure)
	}
}

// peerMoved handles a moved frame: from now on this end sends to the
// peer's new address.
func (e *LawSpecNetEndpoint) peerMoved(payload []byte) {
	history, pos, err := lsGetTexts(payload, 0)
	if err != nil {
		return
	}
	to, _, err := lsGetText(payload, pos)
	if err != nil {
		return
	}
	e.mu.Lock()
	if e.peer == "" {
		e.peer = to
	}
	for _, h := range history {
		if h == e.peer {
			e.peer = to
		}
	}
	known := e.peer == to
	e.mu.Unlock()
	if known {
		e.node.send(to, "moved-ack", lsPutText(nil, to), 0)
	}
}

func (e *LawSpecNetEndpoint) stepDescriptor(sends bool) any {
	if e.step >= len(e.steps) {
		panic("lawspec session: this channel's protocol has ended")
	}
	step := e.steps[e.step]
	if step.Sends != sends {
		if step.Sends {
			panic("lawspec session: this step sends")
		}
		panic("lawspec session: this step receives")
	}
	e.step++
	return step.Descriptor
}

// Send sends a LawSpecValue at this end's next step (side is ignored: an
// endpoint is one side).
func (e *LawSpecNetEndpoint) Send(side int, value any) {
	e.mu.Lock()
	gone := e.gone
	e.mu.Unlock()
	if gone {
		panic(LawSpecPeerFailed)
	}
	d := e.stepDescriptor(true)
	body, err := lsWirePut(e.values, d, value.(LawSpecValue), []byte{0})
	if err != nil {
		panic(err)
	}
	e.mu.Lock()
	seq := e.out
	e.out++
	e.mu.Unlock()
	e.transmit(seq, body)
}

// Receive waits for the next value; it panics with LawSpecPeerFailed when
// the other end gave up or did not answer in time.
func (e *LawSpecNetEndpoint) Receive(side int) any {
	value, err := e.ReceiveWithin(0)
	if err != nil {
		panic(err)
	}
	return value
}

// ReceiveWithin is Receive waiting at most timeout (forever when zero); it
// returns LawSpecPeerFailed, or a timeout error.
func (e *LawSpecNetEndpoint) ReceiveWithin(timeout time.Duration) (LawSpecValue, error) {
	d := e.stepDescriptor(false)
	var deadline <-chan time.Time
	if timeout > 0 {
		timer := time.NewTimer(timeout)
		defer timer.Stop()
		deadline = timer.C
	}
	var inbound lawSpecInbound
	select {
	case inbound = <-e.inbox:
	case <-deadline:
		return LawSpecValue{}, errors.New("no message arrived in time")
	}
	if inbound.failure != "" {
		e.inbox <- inbound
		return LawSpecValue{}, LawSpecPeerFailed
	}
	if len(inbound.body) > 0 && inbound.body[0] == 1 {
		e.fail("the other end gave up the conversation")
		return LawSpecValue{}, LawSpecPeerFailed
	}
	return LawSpecWireDecode(e.values, d, inbound.body[1:])
}

// Abandon gives up: the other end's receives fail after what was sent.
func (e *LawSpecNetEndpoint) Abandon(side int) {
	e.mu.Lock()
	seq := e.out
	e.out++
	e.mu.Unlock()
	e.transmit(seq, []byte{1})
}

// Close stops resending.
func (e *LawSpecNetEndpoint) Close() {
	e.mu.Lock()
	defer e.mu.Unlock()
	if !e.gone {
		e.gone = true
		close(e.stop)
	}
}

// lawSpecNativeTransport is a network endpoint seen through native values:
// each step's value is converted to logical before sending and back after
// receiving.
type lawSpecNativeTransport struct {
	endpoint  *LawSpecNetEndpoint
	toLogical []func(any) LawSpecValue
	toNative  []func(LawSpecValue) any
	step      int
}

func (t *lawSpecNativeTransport) Send(side int, value any) {
	k := t.step
	t.step++
	t.endpoint.Send(side, t.toLogical[k](value))
}

func (t *lawSpecNativeTransport) Receive(side int) any {
	k := t.step
	t.step++
	return t.toNative[k](t.endpoint.Receive(side).(LawSpecValue))
}

func (t *lawSpecNativeTransport) Abandon(side int) { t.endpoint.Abandon(side) }

// lsNetEnd is a typed session's start end over a network endpoint.
func lsNetEnd(endpoint *LawSpecNetEndpoint, side int, toLogical []func(any) LawSpecValue, toNative []func(LawSpecValue) any) *LawSpecEnd {
	return &LawSpecEnd{transport: &lawSpecNativeTransport{endpoint: endpoint, toLogical: toLogical, toNative: toNative}, side: side}
}

// LawSpecOfferEnd is the text that gives an unused channel end to another
// node. An end that is itself between nodes moves there (the text is
// <address>?take=<token>); a local end stays here and a relay on node
// listens for the receiver and passes each step between it and the end
// (the text is the relay's address). steps and the conversions are the
// end's protocol from the end itself. A failure on either side of a relay
// gives up the other.
func LawSpecOfferEnd(node *LawSpecNode, end *LawSpecEnd, steps []LawSpecWireStep, toLogical []func(any) LawSpecValue, toNative []func(LawSpecValue) any, values lawSpecValues) LawSpecValue {
	if network, ok := end.transport.(*lawSpecNativeTransport); ok && network.step == 0 {
		return LawSpecValue{"Text", lsTextUnits(network.endpoint.offer())}
	}
	flipped := []LawSpecWireStep{}
	for _, s := range steps {
		flipped = append(flipped, LawSpecWireStep{!s.Sends, s.Descriptor})
	}
	relay, err := node.Listen(fmt.Sprintf("relay-%d", node.newID()), flipped, values, 5*time.Second)
	if err != nil {
		panic(err)
	}
	relayed := &lawSpecNativeTransport{endpoint: relay, toLogical: toLogical, toNative: toNative}
	go func() {
		defer func() {
			if recover() != nil {
				func() {
					defer func() { recover() }()
					end.transport.Abandon(end.side)
				}()
				func() {
					defer func() { recover() }()
					relay.Abandon(0)
				}()
			}
		}()
		for _, s := range steps {
			if s.Sends {
				end.transport.Send(end.side, relayed.Receive(0))
			} else {
				relayed.Send(0, end.transport.Receive(end.side))
			}
		}
	}()
	return LawSpecValue{"Text", lsTextUnits(relay.Address())}
}

// LawSpecAcceptEnd is the channel end offered at address (as
// LawSpecOfferEnd sent it), on node: it takes over a moving end, or dials a
// relay. steps and the conversions are from that end.
func LawSpecAcceptEnd(node *LawSpecNode, address LawSpecValue, steps []LawSpecWireStep, toLogical []func(any) LawSpecValue, toNative []func(LawSpecValue) any, values lawSpecValues) *LawSpecEnd {
	text, _ := lsUnitsText(address.Data.([]int))
	var endpoint *LawSpecNetEndpoint
	var err error
	if strings.Contains(text, "?take=") {
		endpoint, err = node.Take(text, steps, values, 5*time.Second)
	} else {
		endpoint, err = node.Dial(text, steps, values, 5*time.Second)
	}
	if err != nil {
		panic(err)
	}
	return lsNetEnd(endpoint, 0, toLogical, toNative)
}

// lsFlipSteps is the steps seen from the other end.
func lsFlipSteps(steps []LawSpecWireStep) []LawSpecWireStep {
	out := []LawSpecWireStep{}
	for _, s := range steps {
		out = append(out, LawSpecWireStep{!s.Sends, s.Descriptor})
	}
	return out
}

// Abilities (docs/explanation/abilities.md). Handlers travel in the symbols
// map generated code passes to every definition: that map is the evidence of
// evidence-passing compilation. A law installs one handler per ability; an
// operation finds the handler of its ability there. The Fail ability's
// handlers abort, so raise panics with a *LawSpecFailure and attempt
// recovers it.
const lsHandlersKey = "\x00lawspec.handlers"

// LawSpecFailure is a failure raised with the Fail ability.
type LawSpecFailure struct {
	Ability string
	Value   LawSpecValue
}

func (failure *LawSpecFailure) Error() string {
	return fmt.Sprintf("failed with %v (%s)", failure.Value, failure.Ability)
}

func lsInstallHandlers(symbols map[string]*lawSpecSymbol, handlers map[string]any) {
	table := map[string]any{}
	if entry, ok := symbols[lsHandlersKey]; ok {
		for key, handler := range entry.handlers {
			table[key] = handler
		}
	}
	for key, handler := range handlers {
		table[key] = handler
	}
	installed := &lawSpecSymbol{handlers: table}
	// The workflow clock view stays while its handler does.
	if entry, ok := symbols[lsHandlersKey]; ok && entry != nil {
		lsClockLock.Lock()
		installed.clockView = entry.clockView
		lsClockLock.Unlock()
	}
	symbols[lsHandlersKey] = installed
}

func lsHandler(symbols map[string]*lawSpecSymbol, ability string) any {
	if entry, ok := symbols[lsHandlersKey]; ok {
		if handler, found := entry.handlers[ability]; found {
			return handler
		}
	}
	panic("no handler for the ability " + ability + ": a law names one with `using`, or runs under each lawful handler")
}

// lsWithHandlers runs body with these handlers installed (handle e with h
// end), then puts back the ones they replaced.
func lsWithHandlers(symbols map[string]*lawSpecSymbol, handlers map[string]any, body func() LawSpecValue) LawSpecValue {
	previous, had := symbols[lsHandlersKey]
	lsInstallHandlers(symbols, handlers)
	defer func() {
		if had {
			symbols[lsHandlersKey] = previous
		} else {
			delete(symbols, lsHandlersKey)
		}
	}()
	return body()
}

// LawSpecFail is what native code (an adapter, or a production handler)
// panics with to fail with a value of the failure type its signature names:
// panic(LawSpecFail{Value: v}) for `fails with E`, v a native E.
type LawSpecFail struct {
	Value any
}

func (failure LawSpecFail) Error() string {
	return fmt.Sprintf("failed with %v", failure.Value)
}

// lsErrorAs reports whether err is, or wraps, an error of type T.
func lsErrorAs[T error](err error) bool {
	var target T
	return errors.As(err, &target)
}

// lsNativeFailures calls native code that may fail: a LawSpecFail it panics
// with, or an error lawspec.json maps to a failure (mapped tries each), becomes
// a failure of the ability.
func lsNativeFailures(ability string, convert func(any) LawSpecValue, body func() LawSpecValue, mapped ...func(error) (LawSpecValue, bool)) (result LawSpecValue) {
	defer func() {
		if problem := recover(); problem != nil {
			switch failed := problem.(type) {
			case LawSpecFail:
				panic(&LawSpecFailure{Ability: ability, Value: convert(failed.Value)})
			case *LawSpecFail:
				panic(&LawSpecFailure{Ability: ability, Value: convert(failed.Value)})
			case error:
				for _, try := range mapped {
					if value, ok := try(failed); ok {
						panic(&LawSpecFailure{Ability: ability, Value: value})
					}
				}
			}
			panic(problem)
		}
	}()
	return body()
}

func lsRaiseFailure(ability string, value LawSpecValue) LawSpecValue {
	panic(&LawSpecFailure{Ability: ability, Value: value})
}

func lsAttempt(ability string, body func() LawSpecValue, right, left func(LawSpecValue) LawSpecValue) LawSpecValue {
	var value LawSpecValue
	var failure *LawSpecFailure
	func() {
		defer func() {
			if problem := recover(); problem != nil {
				if caught, ok := problem.(*LawSpecFailure); ok && caught.Ability == ability {
					failure = caught
					return
				}
				panic(problem)
			}
		}()
		value = body()
	}()
	if failure != nil {
		return left(failure.Value)
	}
	return right(value)
}

// LawSpecCall is one call a recording handler saw.
type LawSpecCall struct {
	Operation string
	Arguments []LawSpecValue
}

type lawSpecRecording interface{ lawSpecCalls() []LawSpecCall }

func lsCountCalls(recording any, operation string, matches func([]LawSpecValue) bool) LawSpecValue {
	recorded, ok := recording.(lawSpecRecording)
	if !ok {
		panic("calls of needs a recording handler: `using recording`")
	}
	count := int64(0)
	for _, call := range recorded.lawSpecCalls() {
		if call.Operation == operation && (matches == nil || matches(call.Arguments)) {
			count++
		}
	}
	return lsInteger64(count)
}

// lsPairFields is a Pair's two fields: a stateful handler clause's result
// and the state it leaves.
func lsPairFields(pair LawSpecValue) (LawSpecValue, LawSpecValue) {
	data, ok := pair.Data.(lawSpecData)
	if !ok || len(data.fields) != 2 {
		panic("a handler clause must give Pair result state")
	}
	return data.fields[0], data.fields[1]
}
