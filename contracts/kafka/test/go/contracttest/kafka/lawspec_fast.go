// Typed values for generated checked definitions and stateful spec handlers.
//
// A definition whose types all have a typed form runs over these instead of
// the dynamic LawSpecValue: lists are persistent cons lists that share their
// tails, KeyVal and Set persistent AVL trees in the portable order, and
// Integer stays in an int64 until it outgrows it. A handler's state is
// therefore shared, never copied, from one operation to the next. The
// generated lawspec_typed.go converts each typed value to and from
// LawSpecValue where typed and dynamic code meet.
// ref:DEC-typed-native-definitions
package kafka

import (
	"math/big"
	"strconv"
	"strings"
	"sync"
	"unicode/utf8"
)

// lsfLock serializes a typed spec handler's operations.
type lsfLock = sync.Mutex

var _ = strings.Compare

// lsfUnit is Unit's typed value.
type lsfUnit struct{}

// lsfCmpText orders text by code point, which is byte order in UTF-8.
func lsfCmpText(a, b string) int { return strings.Compare(a, b) }

func lsfTextLength(s string) int64 { return int64(utf8.RuneCountInString(s)) }

func lsfCmpBool(a, b bool) int {
	switch {
	case a == b:
		return 0
	case !a:
		return -1
	default:
		return 1
	}
}

func lsfCmpInt64(a, b int64) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	}
	return 0
}

func lsfCmpUint64(a, b uint64) int {
	switch {
	case a < b:
		return -1
	case a > b:
		return 1
	}
	return 0
}

func lsfOutside(t string) { panic("integer outside " + t + " range") }

// lsfNarrow checks a bounded integer against its type's range.
func lsfNarrow(v, lo, hi int64, t string) int64 {
	if v < lo || v > hi {
		lsfOutside(t)
	}
	return v
}

func lsfAdd(a, b, lo, hi int64, t string) int64 {
	r := a + b
	if (a > 0 && b > 0 && r < 0) || (a < 0 && b < 0 && r >= 0) {
		lsfOutside(t)
	}
	return lsfNarrow(r, lo, hi, t)
}

func lsfSub(a, b, lo, hi int64, t string) int64 {
	r := a - b
	if (a >= 0 && b < 0 && r < 0) || (a < 0 && b > 0 && r >= 0) {
		lsfOutside(t)
	}
	return lsfNarrow(r, lo, hi, t)
}

func lsfMul(a, b, lo, hi int64, t string) int64 {
	if a == 0 || b == 0 {
		return lsfNarrow(0, lo, hi, t)
	}
	r := a * b
	if r/b != a || (a == -1 && b == -1<<63) || (b == -1 && a == -1<<63) {
		lsfOutside(t)
	}
	return lsfNarrow(r, lo, hi, t)
}

// lsfInt is an Integer: an int64 while it fits, a big.Int beyond.
type lsfInt struct {
	s int64
	b *big.Int
}

func lsfSmall(v int64) lsfInt { return lsfInt{s: v} }

func lsfFromBig(b *big.Int) lsfInt {
	if b.IsInt64() {
		return lsfInt{s: b.Int64()}
	}
	return lsfInt{b: b}
}

func lsfFromUint(v uint64) lsfInt {
	if v <= 1<<63-1 {
		return lsfInt{s: int64(v)}
	}
	return lsfInt{b: new(big.Int).SetUint64(v)}
}

func (x lsfInt) big() *big.Int {
	if x.b != nil {
		return x.b
	}
	return big.NewInt(x.s)
}

func (x lsfInt) Add(y lsfInt) lsfInt {
	if x.b == nil && y.b == nil {
		r := x.s + y.s
		if !((x.s > 0 && y.s > 0 && r < 0) || (x.s < 0 && y.s < 0 && r >= 0)) {
			return lsfInt{s: r}
		}
	}
	return lsfFromBig(new(big.Int).Add(x.big(), y.big()))
}

func (x lsfInt) Sub(y lsfInt) lsfInt {
	if x.b == nil && y.b == nil {
		r := x.s - y.s
		if !((x.s >= 0 && y.s < 0 && r < 0) || (x.s < 0 && y.s > 0 && r >= 0)) {
			return lsfInt{s: r}
		}
	}
	return lsfFromBig(new(big.Int).Sub(x.big(), y.big()))
}

func (x lsfInt) Mul(y lsfInt) lsfInt {
	return lsfFromBig(new(big.Int).Mul(x.big(), y.big()))
}

func (x lsfInt) Neg() lsfInt {
	return lsfFromBig(new(big.Int).Neg(x.big()))
}

// Quot truncates, as big.Int's Quo does; Rem takes the dividend's sign.
func (x lsfInt) Quot(y lsfInt) lsfInt {
	if y.b == nil && y.s == 0 {
		panic("exact division by zero")
	}
	return lsfFromBig(new(big.Int).Quo(x.big(), y.big()))
}

func (x lsfInt) Rem(y lsfInt) lsfInt {
	if y.b == nil && y.s == 0 {
		panic("exact division by zero")
	}
	return lsfFromBig(new(big.Int).Rem(x.big(), y.big()))
}

func (x lsfInt) Pow(y lsfInt) lsfInt {
	e := y.big()
	if e.Sign() < 0 {
		panic("negative exponent")
	}
	if !e.IsInt64() || e.Int64() > 1<<31 {
		panic("exponent too large")
	}
	return lsfFromBig(new(big.Int).Exp(x.big(), e, nil))
}

func lsfCmpInt(x, y lsfInt) int {
	if x.b == nil && y.b == nil {
		return lsfCmpInt64(x.s, y.s)
	}
	return x.big().Cmp(y.big())
}

// lsfNarrowInt converts an Integer to a bounded type.
func lsfNarrowInt(x lsfInt, lo, hi int64, t string) int64 {
	if x.b != nil {
		lsfOutside(t)
	}
	return lsfNarrow(x.s, lo, hi, t)
}

func lsfNarrowUint64(x lsfInt) uint64 {
	b := x.big()
	if b.Sign() < 0 || !b.IsUint64() {
		lsfOutside("UInt64")
	}
	return b.Uint64()
}

// lsfMaybe is Maybe: Nothing before Just.
type lsfMaybe[T any] struct {
	ok bool
	v  T
}

func lsfJust[T any](v T) lsfMaybe[T] { return lsfMaybe[T]{true, v} }
func lsfNothing[T any]() lsfMaybe[T] { return lsfMaybe[T]{} }

// lsfEither is Either: Left before Right.
type lsfEither[L any, R any] struct {
	right bool
	l     L
	r     R
}

func lsfLeft[L any, R any](v L) lsfEither[L, R]  { return lsfEither[L, R]{l: v} }
func lsfRight[L any, R any](v R) lsfEither[L, R] { return lsfEither[L, R]{right: true, r: v} }

// lsfList is a persistent list: nil is empty, cons shares its tail, and a
// cell knows the length from it on.
type lsfList[T any] struct {
	head T
	tail *lsfList[T]
	n    int
}

func lsfCons[T any](h T, t *lsfList[T]) *lsfList[T] {
	return &lsfList[T]{h, t, t.length() + 1}
}

func (l *lsfList[T]) length() int {
	if l == nil {
		return 0
	}
	return l.n
}

func lsfListOf[T any](items []T) *lsfList[T] {
	var l *lsfList[T]
	for i := len(items) - 1; i >= 0; i-- {
		l = lsfCons(items[i], l)
	}
	return l
}

func (l *lsfList[T]) slice() []T {
	out := make([]T, 0, l.length())
	for c := l; c != nil; c = c.tail {
		out = append(out, c.head)
	}
	return out
}

func lsfEqList[T any](a, b *lsfList[T], eq func(T, T) bool) bool {
	for {
		if a == b {
			return true
		}
		if a == nil || b == nil || a.n != b.n || !eq(a.head, b.head) {
			return false
		}
		a, b = a.tail, b.tail
	}
}

func lsfCmpList[T any](a, b *lsfList[T], cmp func(T, T) int) int {
	for {
		if a == b {
			return 0
		}
		if a == nil {
			return -1
		}
		if b == nil {
			return 1
		}
		if c := cmp(a.head, b.head); c != 0 {
			return c
		}
		a, b = a.tail, b.tail
	}
}

// lsfMap is a persistent map in the portable order of its keys: an AVL tree
// (nil is empty) whose updates copy only the path to the key.
type lsfMap[K any, V any] struct {
	key         K
	value       V
	left, right *lsfMap[K, V]
	height, n   int
}

func (m *lsfMap[K, V]) size() int {
	if m == nil {
		return 0
	}
	return m.n
}

func (m *lsfMap[K, V]) h() int {
	if m == nil {
		return 0
	}
	return m.height
}

func lsfNode[K any, V any](k K, v V, l, r *lsfMap[K, V]) *lsfMap[K, V] {
	h := l.h()
	if r.h() > h {
		h = r.h()
	}
	return &lsfMap[K, V]{k, v, l, r, h + 1, l.size() + r.size() + 1}
}

func lsfBalance[K any, V any](k K, v V, l, r *lsfMap[K, V]) *lsfMap[K, V] {
	if l.h() > r.h()+1 {
		if l.left.h() >= l.right.h() {
			return lsfNode(l.key, l.value, l.left, lsfNode(k, v, l.right, r))
		}
		lr := l.right
		return lsfNode(lr.key, lr.value, lsfNode(l.key, l.value, l.left, lr.left), lsfNode(k, v, lr.right, r))
	}
	if r.h() > l.h()+1 {
		if r.right.h() >= r.left.h() {
			return lsfNode(r.key, r.value, lsfNode(k, v, l, r.left), r.right)
		}
		rl := r.left
		return lsfNode(rl.key, rl.value, lsfNode(k, v, l, rl.left), lsfNode(r.key, r.value, rl.right, r.right))
	}
	return lsfNode(k, v, l, r)
}

func lsfGet[K any, V any](m *lsfMap[K, V], k K, cmp func(K, K) int) (V, bool) {
	for m != nil {
		c := cmp(k, m.key)
		if c == 0 {
			return m.value, true
		}
		if c < 0 {
			m = m.left
		} else {
			m = m.right
		}
	}
	var zero V
	return zero, false
}

func lsfLookup[K any, V any](m *lsfMap[K, V], k K, cmp func(K, K) int) lsfMaybe[V] {
	v, ok := lsfGet(m, k, cmp)
	return lsfMaybe[V]{ok, v}
}

func lsfPut[K any, V any](m *lsfMap[K, V], k K, v V, cmp func(K, K) int) *lsfMap[K, V] {
	if m == nil {
		return lsfNode[K, V](k, v, nil, nil)
	}
	c := cmp(k, m.key)
	switch {
	case c < 0:
		return lsfBalance(m.key, m.value, lsfPut(m.left, k, v, cmp), m.right)
	case c > 0:
		return lsfBalance(m.key, m.value, m.left, lsfPut(m.right, k, v, cmp))
	}
	return lsfNode(k, v, m.left, m.right)
}

func lsfWithoutMin[K any, V any](m *lsfMap[K, V]) (K, V, *lsfMap[K, V]) {
	if m.left == nil {
		return m.key, m.value, m.right
	}
	k, v, l := lsfWithoutMin(m.left)
	return k, v, lsfBalance(m.key, m.value, l, m.right)
}

func lsfDelete[K any, V any](m *lsfMap[K, V], k K, cmp func(K, K) int) *lsfMap[K, V] {
	if m == nil {
		return nil
	}
	c := cmp(k, m.key)
	switch {
	case c < 0:
		if m.left == nil {
			return m
		}
		return lsfBalance(m.key, m.value, lsfDelete(m.left, k, cmp), m.right)
	case c > 0:
		if m.right == nil {
			return m
		}
		return lsfBalance(m.key, m.value, m.left, lsfDelete(m.right, k, cmp))
	}
	if m.left == nil {
		return m.right
	}
	if m.right == nil {
		return m.left
	}
	mk, mv, r := lsfWithoutMin(m.right)
	return lsfBalance(mk, mv, m.left, r)
}

// lsfEach visits the entries in key order.
func lsfEach[K any, V any](m *lsfMap[K, V], f func(K, V)) {
	var stack []*lsfMap[K, V]
	for c := m; ; {
		for c != nil {
			stack = append(stack, c)
			c = c.left
		}
		if len(stack) == 0 {
			return
		}
		c = stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		f(c.key, c.value)
		c = c.right
	}
}

// lsfToList gives the entries, made by f, as a list in key order.
func lsfToList[K any, V any, E any](m *lsfMap[K, V], f func(K, V) E) *lsfList[E] {
	var out *lsfList[E]
	var stack []*lsfMap[K, V]
	for c := m; ; {
		for c != nil {
			stack = append(stack, c)
			c = c.right
		}
		if len(stack) == 0 {
			return out
		}
		c = stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		out = lsfCons(f(c.key, c.value), out)
		c = c.left
	}
}

func lsfEntries[K any, V any](m *lsfMap[K, V]) ([]K, []V) {
	ks := make([]K, 0, m.size())
	vs := make([]V, 0, m.size())
	lsfEach(m, func(k K, v V) {
		ks = append(ks, k)
		vs = append(vs, v)
	})
	return ks, vs
}

func lsfEqMap[K any, V any](a, b *lsfMap[K, V], ek func(K, K) bool, ev func(V, V) bool) bool {
	if a == b {
		return true
	}
	if a.size() != b.size() {
		return false
	}
	ak, av := lsfEntries(a)
	bk, bv := lsfEntries(b)
	for i := range ak {
		if !ek(ak[i], bk[i]) || !ev(av[i], bv[i]) {
			return false
		}
	}
	return true
}

func lsfCmpMap[K any, V any](a, b *lsfMap[K, V], ck func(K, K) int, cv func(V, V) int) int {
	ak, av := lsfEntries(a)
	bk, bv := lsfEntries(b)
	for i := 0; i < len(ak) && i < len(bk); i++ {
		if c := ck(ak[i], bk[i]); c != 0 {
			return c
		}
		if c := cv(av[i], bv[i]); c != 0 {
			return c
		}
	}
	return lsfCmpInt64(int64(len(ak)), int64(len(bk)))
}

func lsfEqUnit(lsfUnit, lsfUnit) bool { return true }
func lsfCmpUnit(lsfUnit, lsfUnit) int  { return 0 }

// Sets are maps to lsfUnit.

func lsfSetOf[T any](items *lsfList[T], cmp func(T, T) int) *lsfMap[T, lsfUnit] {
	var s *lsfMap[T, lsfUnit]
	for c := items; c != nil; c = c.tail {
		if _, ok := lsfGet(s, c.head, cmp); !ok {
			s = lsfPut(s, c.head, lsfUnit{}, cmp)
		}
	}
	return s
}

func lsfSetInsert[T any](s *lsfMap[T, lsfUnit], x T, cmp func(T, T) int) *lsfMap[T, lsfUnit] {
	if _, ok := lsfGet(s, x, cmp); ok {
		return s
	}
	return lsfPut(s, x, lsfUnit{}, cmp)
}

func lsfSetItems[T any](s *lsfMap[T, lsfUnit]) *lsfList[T] {
	return lsfToList(s, func(k T, _ lsfUnit) T { return k })
}

func lsfSetUnion[T any](a, b *lsfMap[T, lsfUnit], cmp func(T, T) int) *lsfMap[T, lsfUnit] {
	small, large := a, b
	if a.size() > b.size() {
		small, large = b, a
	}
	out := large
	lsfEach(small, func(k T, _ lsfUnit) { out = lsfSetInsert(out, k, cmp) })
	return out
}

func lsfSetIntersection[T any](a, b *lsfMap[T, lsfUnit], cmp func(T, T) int) *lsfMap[T, lsfUnit] {
	var out *lsfMap[T, lsfUnit]
	lsfEach(a, func(k T, _ lsfUnit) {
		if _, ok := lsfGet(b, k, cmp); ok {
			out = lsfPut(out, k, lsfUnit{}, cmp)
		}
	})
	return out
}

func lsfSetDifference[T any](a, b *lsfMap[T, lsfUnit], cmp func(T, T) int) *lsfMap[T, lsfUnit] {
	var out *lsfMap[T, lsfUnit]
	lsfEach(a, func(k T, _ lsfUnit) {
		if _, ok := lsfGet(b, k, cmp); !ok {
			out = lsfPut(out, k, lsfUnit{}, cmp)
		}
	})
	return out
}

func lsfMember[T any](s *lsfMap[T, lsfUnit], x T, cmp func(T, T) int) bool {
	_, ok := lsfGet(s, x, cmp)
	return ok
}

func lsfRegexMatches(pattern, text string) bool {
	units := make([]int, 0, len(text))
	for _, r := range text {
		units = append(units, int(r))
	}
	return LsRegexMatches(pattern, units)
}

// Conversions from the dynamic LawSpecValue.

func lsfValueInt64(v LawSpecValue, lo, hi int64, t string) int64 {
	switch x := v.Data.(type) {
	case *big.Int:
		if !x.IsInt64() {
			lsfOutside(t)
		}
		return lsfNarrow(x.Int64(), lo, hi, t)
	case int64:
		return lsfNarrow(x, lo, hi, t)
	case int:
		return lsfNarrow(int64(x), lo, hi, t)
	}
	panic("expected an integer for " + t)
}

func lsfValueUint64(v LawSpecValue) uint64 {
	switch x := v.Data.(type) {
	case *big.Int:
		if x.Sign() < 0 || !x.IsUint64() {
			lsfOutside("UInt64")
		}
		return x.Uint64()
	}
	panic("expected an integer for UInt64")
}

func lsfValueInt(v LawSpecValue) lsfInt {
	switch x := v.Data.(type) {
	case *big.Int:
		return lsfFromBig(new(big.Int).Set(x))
	case int64:
		return lsfSmall(x)
	case int:
		return lsfSmall(int64(x))
	}
	panic("expected an Integer")
}

func lsfUnits(v LawSpecValue) []int {
	switch x := v.Data.(type) {
	case []int:
		return x
	case string:
		var out []int
		for _, r := range x {
			out = append(out, int(r))
		}
		return out
	case []byte:
		out := make([]int, len(x))
		for i, b := range x {
			out[i] = int(b)
		}
		return out
	case []LawSpecValue:
		out := make([]int, len(x))
		for i, item := range x {
			out[i] = int(lsfValueInt64(item, 0, 1<<31-1, "code point"))
		}
		return out
	}
	panic("expected a sequence")
}

func lsfValueText(v LawSpecValue) string {
	var b strings.Builder
	for _, u := range lsfUnits(v) {
		b.WriteRune(rune(u))
	}
	return b.String()
}

func lsfValueBytes(v LawSpecValue) string {
	units := lsfUnits(v)
	out := make([]byte, len(units))
	for i, u := range units {
		out[i] = byte(u)
	}
	return string(out)
}

func lsfTextValue(s string) LawSpecValue {
	units := make([]int, 0, len(s))
	for _, r := range s {
		units = append(units, int(r))
	}
	return LawSpecValue{"Text", units}
}

func lsfBytesValue(s string) LawSpecValue {
	units := make([]int, len(s))
	for i := 0; i < len(s); i++ {
		units[i] = int(s[i])
	}
	return LawSpecValue{"Bytes", units}
}

// lsfData gives a data value's tag and fields.
func lsfData(v LawSpecValue) (string, []LawSpecValue) {
	switch d := v.Data.(type) {
	case lawSpecData:
		return d.tag, d.fields
	case *lawSpecData:
		return d.tag, d.fields
	}
	panic("expected a data value of " + v.Type)
}

// lsfItems gives a collection's items: its one field's list, or a plain list.
func lsfItems(v LawSpecValue) []LawSpecValue {
	switch x := v.Data.(type) {
	case []LawSpecValue:
		return x
	}
	_, fields := lsfData(v)
	return fields[0].Data.([]LawSpecValue)
}

func lsfDataValue(key, tag string, fields ...LawSpecValue) LawSpecValue {
	return LawSpecValue{key, lawSpecData{tag, fields}}
}

// Sequences: Text (by code point), Bytes and List. A slice is clamped to
// the sequence; an index outside it is Nothing.

// lsfIndex is an Integer index, clamped to an int64.
func lsfIndex(x lsfInt) int64 {
	if x.b == nil {
		return x.s
	}
	if x.b.Sign() < 0 {
		return -1 << 63
	}
	return 1<<63 - 1
}

func lsfClamp(i int64, count int) int {
	if i < 0 {
		return 0
	}
	if i > int64(count) {
		return count
	}
	return int(i)
}

func lsfBounds(from, to int64, count int) (int, int) {
	i := lsfClamp(from, count)
	j := lsfClamp(to, count)
	if j < i {
		j = i
	}
	return i, j
}

func lsfSliceText(s string, from, to int64) string {
	ascii := true
	for k := 0; k < len(s); k++ {
		if s[k] >= utf8.RuneSelf {
			ascii = false
			break
		}
	}
	if ascii {
		i, j := lsfBounds(from, to, len(s))
		return s[i:j]
	}
	runes := []rune(s)
	i, j := lsfBounds(from, to, len(runes))
	return string(runes[i:j])
}

func lsfSliceBytes(s string, from, to int64) string {
	i, j := lsfBounds(from, to, len(s))
	return s[i:j]
}

// lsfSliceList shares the list's tail when the slice runs to its end.
func lsfSliceList[T any](xs *lsfList[T], from, to int64) *lsfList[T] {
	count := xs.length()
	i, j := lsfBounds(from, to, count)
	rest := xs
	for k := 0; k < i; k++ {
		rest = rest.tail
	}
	if j == count {
		return rest
	}
	items := make([]T, 0, j-i)
	for c := rest; len(items) < j-i; c = c.tail {
		items = append(items, c.head)
	}
	return lsfListOf(items)
}

// lsfAppend shares b.
func lsfAppend[T any](a, b *lsfList[T]) *lsfList[T] {
	items := a.slice()
	out := b
	for k := len(items) - 1; k >= 0; k-- {
		out = lsfCons(items[k], out)
	}
	return out
}

func lsfAtList[T any](xs *lsfList[T], i int64) lsfMaybe[T] {
	if i < 0 || i >= int64(xs.length()) {
		return lsfNothing[T]()
	}
	c := xs
	for k := int64(0); k < i; k++ {
		c = c.tail
	}
	return lsfJust(c.head)
}

func lsfAtBytes(s string, i int64) lsfMaybe[uint8] {
	if i < 0 || i >= int64(len(s)) {
		return lsfNothing[uint8]()
	}
	return lsfJust(s[i])
}

func lsfRange[T int8 | int16 | int32 | int64 | uint8 | uint16 | uint32 | uint64](lo, hi T) *lsfList[T] {
	var out *lsfList[T]
	for k := hi; k > lo; k-- {
		out = lsfCons(k-1, out)
	}
	return out
}

func lsfRangeInt(lo, hi lsfInt) *lsfList[lsfInt] {
	var out *lsfList[lsfInt]
	one := lsfSmall(1)
	for k := hi.Sub(one); lsfCmpInt(k, lo) >= 0; k = k.Sub(one) {
		out = lsfCons(k, out)
	}
	return out
}

func lsfOctets(s string) *lsfList[uint8] {
	return lsfListOf([]uint8(s))
}

func lsfBytesOf(xs *lsfList[uint8]) string {
	return string(xs.slice())
}

func lsfTextOfLong(v int64) string { return strconv.FormatInt(v, 10) }

func lsfTextOfUint(v uint64) string { return strconv.FormatUint(v, 10) }

func lsfTextOfInt(x lsfInt) string {
	if x.b == nil {
		return strconv.FormatInt(x.s, 10)
	}
	return x.b.String()
}

func lsfJoin(separator string, parts *lsfList[string]) string {
	return strings.Join(parts.slice(), separator)
}
