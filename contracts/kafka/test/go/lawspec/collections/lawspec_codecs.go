// Typed conversion bridges for Go, independent of property-testing frameworks.
//
// Adapters work with their own Go types; each codec converts between them and
// LawSpec's values and checks the result, so a law never sees a native value
// outside its domain. ref:DEC-native-bindings-typed-identity
package collections

import (
	"fmt"
	"math/big"
	"reflect"
	"slices"
	"time"
)

// LawSpecDuration is a Duration: a time.Duration of whole microseconds, from
// 0 to about 146 years.
type LawSpecDuration = time.Duration

type lawSpecCodec[T any] struct {
	typeRef    lawSpecTypeRef
	toNative   func(LawSpecValue) T
	fromNative func(T) LawSpecValue
	encode     func(T, lawSpecPath) LawSpecValue
}

func lsNativeContext[T any](context string, operation func() T) (result T) {
	defer func() {
		if problem := recover(); problem != nil {
			// A Fail ability's failure passes through to its attempt.
			if failure, ok := problem.(*LawSpecFailure); ok {
				panic(failure)
			}
			// So do native failures, and the application's own errors, which
			// lawspec.json may map to failures.
			switch problem.(type) {
			case LawSpecFail, *LawSpecFail:
				panic(problem)
			}
			if _, isError := problem.(error); isError {
				if _, rejected := problem.(lawSpecRefinementViolation); !rejected {
					panic(problem)
				}
			}
			message := fmt.Sprintf("%s: %v", context, problem)
			if _, rejected := problem.(lawSpecRefinementViolation); rejected {
				panic(lawSpecRefinementViolation{message})
			}
			panic(message)
		}
	}()
	return operation()
}

func lsCodec[T any](schema *lawSpecSchema, bits int, typeRef lawSpecTypeRef,
	toNative func(LawSpecValue) T, fromNative func(T, lawSpecPath) LawSpecValue, contexts ...map[string]*lawSpecSymbol) lawSpecCodec[T] {
	symbols := lsSchemaSymbols(contexts)
	schema.check(typeRef, 0)
	if bits != 32 && bits != 64 {
		panic("machineBits must be 32 or 64")
	}
	schema.checkNativeProfile(typeRef, bits)
	encode := func(value T, path lawSpecPath) LawSpecValue {
		return lsSchemaContext(typeRef.key(), func() LawSpecValue {
			return schema.validate(typeRef, fromNative(value, path), bits, symbols)
		})
	}
	return lawSpecCodec[T]{
		typeRef: typeRef,
		toNative: func(value LawSpecValue) T {
			return lsNativeContext(typeRef.key(), func() T {
				return toNative(schema.validate(typeRef, value, bits, symbols))
			})
		},
		fromNative: func(value T) LawSpecValue { return encode(value, lawSpecPath{}) },
		encode:     encode,
	}
}

// Opaque generic payloads retain validation while crossing application hooks.
func lsLogicalCodec(schema *lawSpecSchema, bits int, typeRef lawSpecTypeRef, contexts ...map[string]*lawSpecSymbol) lawSpecCodec[LawSpecValue] {
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) LawSpecValue { return value },
		func(value LawSpecValue, path lawSpecPath) LawSpecValue { return value }, contexts...)
}

// lsHandleCodec passes a handle's native value through both ways.
func lsHandleCodec[T any](schema *lawSpecSchema, bits int, name string, contexts ...map[string]*lawSpecSymbol) lawSpecCodec[T] {
	typeRef := lsNamed(name)
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) T {
			handle, ok := value.Data.(lawSpecHandle)
			if !ok {
				panic("expected a handle for " + name)
			}
			if handle.native == nil {
				var none T
				return none
			}
			native, ok := handle.native.(T)
			if !ok {
				panic(fmt.Sprintf("handle %s holds %T", name, handle.native))
			}
			return native
		},
		func(value T, path lawSpecPath) LawSpecValue {
			return LawSpecValue{typeRef.key(), lawSpecHandle{any(value)}}
		}, contexts...)
}

// lsNativeMaybe is a native result as a Maybe: nil (a nil pointer or
// interface) is Absent, anything else Present, through a pointer when the
// native returns one.
func lsNativeMaybe[T any](value any) LawSpecMaybe[T] {
	if value == nil {
		return LawSpecAbsent[T]()
	}
	reflected := reflect.ValueOf(value)
	switch reflected.Kind() {
	case reflect.Pointer, reflect.Interface, reflect.Map, reflect.Slice, reflect.Func, reflect.Chan:
		if reflected.IsNil() {
			return LawSpecAbsent[T]()
		}
	}
	if native, ok := value.(T); ok {
		return LawSpecPresent(native)
	}
	if reflected.Kind() == reflect.Pointer {
		if native, ok := reflected.Elem().Interface().(T); ok {
			return LawSpecPresent(native)
		}
	}
	panic(fmt.Sprintf("native result %T is not a Maybe value", value))
}

func lsScalarCodec[T any](schema *lawSpecSchema, bits int, name string) lawSpecCodec[T] {
	lsCheckNativeProfile(name, bits)
	return lsCodec(schema, bits, lsNamed(name),
		func(value LawSpecValue) T {
			var native any
			switch name {
			case "Unit":
				native = LawSpecUnit{}
			case "Null":
				native = LawSpecNull{}
			case "Undefined":
				native = LawSpecUndefined{}
			default:
				native = lsToNative(name, value, bits)
			}
			return native.(T)
		},
		func(value T, path lawSpecPath) LawSpecValue {
			switch name {
			case "Unit":
				_ = any(value).(LawSpecUnit)
				return lsAbsent(name)
			case "Null":
				_ = any(value).(LawSpecNull)
				return lsAbsent(name)
			case "Undefined":
				_ = any(value).(LawSpecUndefined)
				return lsAbsent(name)
			default:
				return lsFromNative(name, value, bits)
			}
		})
}

func lsListCodec[T any](schema *lawSpecSchema, bits int, element lawSpecCodec[T], contexts ...map[string]*lawSpecSymbol) lawSpecCodec[[]T] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("List", element.typeRef)
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) []T {
			values := value.Data.([]LawSpecValue)
			result := make([]T, len(values))
			for index, child := range values {
				result[index] = lsNativeContext(fmt.Sprintf("List[%d]", index), func() T {
					return element.toNative(child)
				})
			}
			return result
		},
		func(value []T, path lawSpecPath) LawSpecValue {
			defer path.enter(value)()
			result := make([]LawSpecValue, len(value))
			for index, child := range value {
				result[index] = lsSchemaContext(fmt.Sprintf("List[%d]", index), func() LawSpecValue {
					return element.encode(child, path)
				})
			}
			return LawSpecValue{typeRef.key(), result}
		}, symbols)
}

// Built-in collections and their single constructors. Natively each is a
// slice of its items (a KeyVal's entries) in canonical order; a Stack's top is
// last, as for append.
var lsCollectionTags = map[string]string{
	"Set": "SetItems", "KeyVal": "KeyValEntries", "Queue": "QueueItems",
	"Stack": "StackItems", "Deque": "DequeItems",
}

func lsCollectionCodec[T any](schema *lawSpecSchema, bits int, name string, arguments []lawSpecTypeRef, item lawSpecCodec[T], contexts ...map[string]*lawSpecSymbol) lawSpecCodec[[]T] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("lawspec.collections::type::"+name, arguments...)
	tag := typeRef.name + "::" + lsCollectionTags[name]
	listKey := lsNamed("List", item.typeRef).key()
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) []T {
			values := value.Data.(lawSpecData).fields[0].Data.([]LawSpecValue)
			result := make([]T, len(values))
			for index, child := range values {
				result[index] = item.toNative(child)
			}
			if name == "Stack" {
				slices.Reverse(result)
			}
			return result
		},
		func(value []T, path lawSpecPath) LawSpecValue {
			items := make([]LawSpecValue, len(value))
			for index, child := range value {
				items[index] = item.encode(child, path)
			}
			switch name {
			case "Stack":
				slices.Reverse(items)
			case "Set":
				items = lsCanonicalItems(items, func(item LawSpecValue) LawSpecValue { return item })
			case "KeyVal":
				items = lsCanonicalItems(items, func(entry LawSpecValue) LawSpecValue { return entry.Data.(lawSpecData).fields[0] })
			}
			return LawSpecValue{typeRef.key(), lawSpecData{tag, []LawSpecValue{{listKey, items}}}}
		}, symbols)
}

// lsCanonicalItems sorts by key, keeping the last of equal keys.
func lsCanonicalItems(items []LawSpecValue, key func(LawSpecValue) LawSpecValue) []LawSpecValue {
	ordered := slices.Clone(items)
	slices.SortStableFunc(ordered, func(a, b LawSpecValue) int { return lsCompareValues(key(a), key(b)) })
	result := []LawSpecValue{}
	for _, item := range ordered {
		if len(result) > 0 && lsCompareValues(key(result[len(result)-1]), key(item)) == 0 {
			result[len(result)-1] = item
		} else {
			result = append(result, item)
		}
	}
	return result
}

func lsMaybeCodec[T any](schema *lawSpecSchema, bits int, element lawSpecCodec[T], contexts ...map[string]*lawSpecSymbol) lawSpecCodec[LawSpecMaybe[T]] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("Maybe", element.typeRef)
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) LawSpecMaybe[T] {
			data := value.Data.(lawSpecData)
			if data.tag == "Maybe::Absent" {
				return LawSpecAbsent[T]()
			}
			return LawSpecPresent(element.toNative(data.fields[0]))
		},
		func(value LawSpecMaybe[T], path lawSpecPath) LawSpecValue {
			if !value.present {
				return schema.construct(typeRef, "Maybe::Absent", nil, bits, symbols)
			}
			return schema.construct(typeRef, "Maybe::Present",
				[]LawSpecValue{element.encode(value.value, path)}, bits, symbols)
		}, symbols)
}

func lsEitherCodec[L, R any](schema *lawSpecSchema, bits int, left lawSpecCodec[L], right lawSpecCodec[R], contexts ...map[string]*lawSpecSymbol) lawSpecCodec[LawSpecEither[L, R]] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("Either", left.typeRef, right.typeRef)
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) LawSpecEither[L, R] {
			data := value.Data.(lawSpecData)
			if data.tag == "Either::Left" {
				return LawSpecLeft[L, R](left.toNative(data.fields[0]))
			}
			return LawSpecRight[L, R](right.toNative(data.fields[0]))
		},
		func(value LawSpecEither[L, R], path lawSpecPath) LawSpecValue {
			switch value.tag {
			case 1:
				return schema.construct(typeRef, "Either::Left",
					[]LawSpecValue{left.encode(value.left, path)}, bits, symbols)
			case 2:
				return schema.construct(typeRef, "Either::Right",
					[]LawSpecValue{right.encode(value.right, path)}, bits, symbols)
			default:
				panic("Either requires Left or Right")
			}
		}, symbols)
}

func lsNullableCodec[T any](schema *lawSpecSchema, bits int, element lawSpecCodec[T], contexts ...map[string]*lawSpecSymbol) lawSpecCodec[LawSpecNullable[T]] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("Nullable", element.typeRef)
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) LawSpecNullable[T] {
			presence := value.Data.(lawSpecPresence)
			if presence.value == nil {
				return LawSpecNullable[T]{}
			}
			return LawSpecNullable[T]{true, element.toNative(*presence.value)}
		},
		func(value LawSpecNullable[T], path lawSpecPath) LawSpecValue {
			if !value.Present {
				return lsPresent(typeRef.key(), nil)
			}
			child := element.encode(value.Value, path)
			return lsPresent(typeRef.key(), &child)
		}, symbols)
}

func lsOptionalCodec[T any](schema *lawSpecSchema, bits int, element lawSpecCodec[T], contexts ...map[string]*lawSpecSymbol) lawSpecCodec[LawSpecOptional[T]] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("Optional", element.typeRef)
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) LawSpecOptional[T] {
			presence := value.Data.(lawSpecPresence)
			if presence.value == nil {
				return LawSpecOptional[T]{}
			}
			return LawSpecOptional[T]{true, element.toNative(*presence.value)}
		},
		func(value LawSpecOptional[T], path lawSpecPath) LawSpecValue {
			if !value.Present {
				return lsPresent(typeRef.key(), nil)
			}
			child := element.encode(value.Value, path)
			return lsPresent(typeRef.key(), &child)
		}, symbols)
}

// lsDurationCodec converts a Duration and a time.Duration. A time.Duration
// that is negative or not a whole number of microseconds is not a Duration.
func lsDurationCodec(schema *lawSpecSchema, bits int, contexts ...map[string]*lawSpecSymbol) lawSpecCodec[LawSpecDuration] {
	symbols := lsSchemaSymbols(contexts)
	typeRef := lsNamed("lawspec.time::type::Duration")
	tag := typeRef.name + "::Duration"
	return lsCodec(schema, bits, typeRef,
		func(value LawSpecValue) LawSpecDuration {
			micros := value.Data.(lawSpecData).fields[0].Data.(*big.Int)
			return LawSpecDuration(micros.Int64()) * time.Microsecond
		},
		func(value LawSpecDuration, path lawSpecPath) LawSpecValue {
			if value < 0 || value%time.Microsecond != 0 {
				panic("a Duration is a non-negative whole number of microseconds")
			}
			micros := LawSpecValue{"Integer", big.NewInt(int64(value / time.Microsecond))}
			return LawSpecValue{typeRef.key(), lawSpecData{tag, []LawSpecValue{micros}}}
		}, symbols)
}
