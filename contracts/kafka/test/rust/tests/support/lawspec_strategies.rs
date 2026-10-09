//! Proptest support, separate from the reusable numeric runtime.
//!
//! Generated tests run in Proptest, the framework Rust developers already use,
//! so failures shrink, persist and report as they expect; LawSpec supplies only
//! the strategies for its own types, and shrinking stays within the declared
//! domain. ref:DEC-native-property-frameworks ref:proptest
//! ref:DEC-shrink-within-domain
use crate::lawspec_runtime::{self as ls, IntoValue, Value};
use proptest::prelude::*;

/// A framework-native factory. Child strategies correspond to type parameters,
/// not constructor fields. Mapping the returned strategy retains its ValueTree.
pub type NativeFactory = fn(
    &ls::Schema,
    &ls::TypeRef,
    u32,
    &ls::Context,
    Vec<BoxedStrategy<Value>>,
) -> ls::Result<BoxedStrategy<Value>>;

#[derive(Default)]
pub struct NativeGenerators {
    factories: std::collections::HashMap<&'static str, NativeFactory>,
}

impl NativeGenerators {
    pub fn new(entries: Vec<(&'static str, NativeFactory)>) -> ls::Result<Self> {
        let mut factories = std::collections::HashMap::new();
        for (name, factory) in entries {
            if name.is_empty() || factories.insert(name, factory).is_some() {
                return Err(format!("duplicate or empty native generator: {name}"));
            }
        }
        Ok(Self { factories })
    }

    fn factory(&self, ty: &ls::TypeRef) -> Option<NativeFactory> {
        match ty {
            ls::TypeRef::Named(name, _) => self.factories.get(name).copied(),
            _ => None,
        }
    }
}

pub fn schema_strategy_with_generators(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
    context: &ls::Context,
    native: &NativeGenerators,
) -> ls::Result<BoxedStrategy<Value>> {
    if schema.has_contracts() {
        return Err("constructor contracts require checked_schema_strategy_with_generators".into());
    }
    shape_strategy_with_generators(
        schema,
        ty,
        bits,
        budget,
        Default::default(),
        context,
        native,
    )
}

pub fn strategy(name: &str) -> ls::Result<BoxedStrategy<Value>> {
    strategy_with_profile(name, usize::BITS)
}

fn scalar_strategy(name: &str) -> ls::Result<BoxedStrategy<Value>> {
    if let Some(inner) = name.strip_prefix("Nullable ") {
        return Ok(proptest::option::of(strategy(inner)?)
            .prop_map(|x| Value::Nullable(x.map(Box::new)))
            .boxed());
    }
    if let Some(inner) = name.strip_prefix("Optional ") {
        return Ok(proptest::option::of(strategy(inner)?)
            .prop_map(|x| Value::Optional(x.map(Box::new)))
            .boxed());
    }
    macro_rules! integer {
        ($t:ty) => {
            any::<$t>().prop_map(IntoValue::into_value).boxed()
        };
    }
    Ok(match name {
        "Bool" => any::<bool>().prop_map(Value::Bool).boxed(),
        "Int8" => integer!(i8),
        "Int16" => integer!(i16),
        "Int32" => integer!(i32),
        "Int64" => integer!(i64),
        "UInt8" => integer!(u8),
        "UInt16" => integer!(u16),
        "UInt32" => integer!(u32),
        "UInt64" => integer!(u64),
        "IntSize" => integer!(isize),
        "UIntSize" | "UIntPtr" => integer!(usize),
        "Integer" | "BigInt" => bigint().prop_map(Value::Integer).boxed(),
        "BigUInt" => proptest::collection::vec(any::<u8>(), 0..65)
            .prop_map(|bytes| {
                Value::Integer(ls::BigInt::from_bytes_le(num_bigint::Sign::Plus, &bytes))
            })
            .boxed(),
        "Decimal" => (bigint(), -100i32..101)
            .prop_map(|(c, e)| Value::Decimal(ls::Decimal::new(c, e.into())))
            .boxed(),
        "Rational" => (bigint(), 1u64..=u64::MAX)
            .prop_map(|(n, d)| Value::Rational(ls::BigRational::new(n, d.into())))
            .boxed(),
        "Float32" => any::<u32>()
            .prop_map(|b| Value::Float32(f32::from_bits(b)))
            .boxed(),
        "Float64" => any::<u64>()
            .prop_map(|b| Value::Float64(f64::from_bits(b)))
            .boxed(),
        "Complex64" => (any::<u32>(), any::<u32>())
            .prop_map(|(r, i)| {
                Value::Complex32(ls::Complex32::new(f32::from_bits(r), f32::from_bits(i)))
            })
            .boxed(),
        "Complex128" => (any::<u64>(), any::<u64>())
            .prop_map(|(r, i)| {
                Value::Complex64(ls::Complex64::new(f64::from_bits(r), f64::from_bits(i)))
            })
            .boxed(),
        "Char" => any::<char>().prop_map(Value::Char).boxed(),
        "CodePoint" => (0u32..=0x10ffff).prop_map(Value::CodePoint).boxed(),
        "CodeUnit16" => any::<u16>().prop_map(Value::CodeUnit16).boxed(),
        "Text" => proptest::collection::vec(any::<char>(), 0..65)
            .prop_map(|cs| Value::Text(cs.into_iter().collect()))
            .boxed(),
        "CodePointText" => proptest::collection::vec(0u32..=0x10ffff, 0..65)
            .prop_map(Value::CodePointText)
            .boxed(),
        "Utf16Text" => proptest::collection::vec(any::<u16>(), 0..65)
            .prop_map(Value::Utf16Text)
            .boxed(),
        "Bytes" => proptest::collection::vec(any::<u8>(), 0..65)
            .prop_map(Value::Bytes)
            .boxed(),
        "Symbol" => any::<u64>()
            .prop_map(|id| Value::Symbol(ls::Symbol::new(format!("symbol-{id}"))))
            .boxed(),
        "Unit" => Just(Value::Unit).boxed(),
        "Null" => Just(Value::Null).boxed(),
        "Undefined" => Just(Value::Undefined).boxed(),
        _ => return Err(format!("no Rust generator for {name}")),
    })
}
fn bigint() -> BoxedStrategy<ls::BigInt> {
    (any::<bool>(), proptest::collection::vec(any::<u8>(), 0..65))
        .prop_map(|(negative, bytes)| {
            ls::BigInt::from_bytes_le(
                if negative {
                    num_bigint::Sign::Minus
                } else {
                    num_bigint::Sign::Plus
                },
                &bytes,
            )
        })
        .boxed()
}
pub fn tuple(types: &[&str]) -> ls::Result<BoxedStrategy<Vec<Value>>> {
    let mut result = Just(Vec::new()).boxed();
    for name in types {
        result = (result, strategy(name)?)
            .prop_map(|(mut xs, x)| {
                xs.push(x);
                xs
            })
            .boxed();
    }
    Ok(result)
}

/// Construct the affine integer domain after preceding inputs are known.
/// An empty continuation returns None so the native flat-map/filter combination
/// retries the preceding inputs and recomputes the domain during shrinking.
pub fn bounded_integer(
    name: &str,
    bounds: Vec<(&str, Value)>,
    machine_bits: u32,
) -> ls::Result<Option<BoxedStrategy<Value>>> {
    use num_traits::{One, Signed, Zero};
    let width = match name {
        "Int8" | "UInt8" => Some(8),
        "Int16" | "UInt16" => Some(16),
        "Int32" | "UInt32" => Some(32),
        "Int64" | "UInt64" => Some(64),
        "IntSize" | "UIntSize" | "UIntPtr" => Some(machine_bits),
        _ => None,
    };
    let unsigned = name.starts_with("UInt") || name == "BigUInt";
    let mut lo = if unsigned {
        Some(ls::BigInt::zero())
    } else {
        width.map(|w| -(ls::BigInt::one() << (w - 1)))
    };
    let mut hi = width.map(|w| (ls::BigInt::one() << (if unsigned { w } else { w - 1 })) - 1u8);
    for (op, value) in bounds {
        let r = value.exact()?;
        let floor = r.floor().to_integer();
        let ceil = r.ceil().to_integer();
        let (lower, upper) = match op {
            ">" => (Some(floor + 1u8), None),
            ">=" => (Some(ceil), None),
            "<" => (None, Some(ceil - 1u8)),
            "<=" => (None, Some(floor)),
            "==" => {
                if !r.is_integer() {
                    return Ok(None);
                }
                let n = r.to_integer();
                (Some(n.clone()), Some(n))
            }
            _ => return Err("invalid planned integer bound".into()),
        };
        if let Some(n) = lower {
            lo = Some(lo.map_or(n.clone(), |old| old.max(n)));
        }
        if let Some(n) = upper {
            hi = Some(hi.map_or(n.clone(), |old| old.min(n)));
        }
    }
    let span = ls::BigInt::one() << 512usize;
    let lower = lo.unwrap_or_else(|| hi.as_ref().map_or(-&span, |upper| upper - &span));
    let upper = hi.unwrap_or_else(|| &lower + &span);
    if lower > upper {
        return Ok(None);
    }
    let count = &upper - &lower + 1u8;
    let offset = lower.clone();
    Ok(Some(
        prop_oneof![
            Just(Value::Integer(lower)),
            Just(Value::Integer(upper)),
            bigint().prop_map(move |n| Value::Integer(&offset + n.abs() % &count))
        ]
        .boxed(),
    ))
}

pub fn strategy_with_profile(name: &str, bits: u32) -> ls::Result<BoxedStrategy<Value>> {
    if let Some(inner) = name.strip_prefix("List ") {
        return Ok(
            proptest::collection::vec(strategy_with_profile(inner, bits)?, 0..65)
                .prop_map(Value::List)
                .boxed(),
        );
    }
    if let Some(inner) = name.strip_prefix("Maybe ") {
        return Ok(proptest::option::of(strategy_with_profile(inner, bits)?)
            .prop_map(|x| Value::Maybe(x.map(Box::new)))
            .boxed());
    }
    if let Some(arguments) = name.strip_prefix("Either ") {
        let (left, right) = ls::either_types(arguments)?;
        return Ok(prop_oneof![
            strategy_with_profile(left, bits)?.prop_map(|x| Value::Left(Box::new(x))),
            strategy_with_profile(right, bits)?.prop_map(|x| Value::Right(Box::new(x)))
        ]
        .boxed());
    }
    if let Some(inner) = name.strip_prefix("Nullable ") {
        return Ok(proptest::option::of(strategy_with_profile(inner, bits)?)
            .prop_map(|x| Value::Nullable(x.map(Box::new)))
            .boxed());
    }
    if let Some(inner) = name.strip_prefix("Optional ") {
        return Ok(proptest::option::of(strategy_with_profile(inner, bits)?)
            .prop_map(|x| Value::Optional(x.map(Box::new)))
            .boxed());
    }
    scalar_strategy(match (name, bits) {
        ("IntSize", 32) => "Int32",
        ("IntSize", 64) => "Int64",
        ("UIntSize" | "UIntPtr", 32) => "UInt32",
        ("UIntSize" | "UIntPtr", 64) => "UInt64",
        _ => name,
    })
}

#[derive(Clone, Debug)]
pub struct Case {
    pub context: ls::Context,
    pub values: Vec<Value>,
    pub error: Option<String>,
}
// A case's workflows wait on a virtual clock, as every generated test's do.
impl Default for Case {
    fn default() -> Self {
        Case { context: ls::Context::testing(), values: Vec::new(), error: None }
    }
}
pub fn seeded(strategy: BoxedStrategy<Value>, seeds: Vec<Value>) -> BoxedStrategy<Value> {
    if seeds.is_empty() {
        strategy
    } else {
        prop_oneof![strategy, proptest::sample::select(seeds)].boxed()
    }
}

/// Compose native proptest generators from instantiated Core data schemas.
/// The structural budget limits samples, not the mathematical input domain.
/// Memoization shares strategies for repeated fields in recursive products.
pub fn schema_strategy(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
) -> ls::Result<BoxedStrategy<Value>> {
    if schema.has_contracts() {
        return Err(
            "constructor contracts require checked_schema_strategy and a Symbol context".into(),
        );
    }
    shape_strategy(schema, ty, bits, budget, Default::default())
}

type Witnesses = std::collections::HashMap<ls::TypeRef, Vec<Value>>;

fn value_cost(value: &Value) -> usize {
    match value {
        Value::Data(_, fields) | Value::List(fields) => {
            1 + fields.iter().map(value_cost).sum::<usize>()
        }
        Value::Maybe(Some(value))
        | Value::Nullable(Some(value))
        | Value::Optional(Some(value))
        | Value::Left(value)
        | Value::Right(value) => 1 + value_cost(value),
        _ => 1,
    }
}

// Witnesses are validated before traversal. Keep their instantiated types so a
// Symbol seed cannot leak into unrelated fields with the same representation.
fn collect_witnesses(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    value: &Value,
    seeds: &mut Witnesses,
) -> ls::Result<()> {
    // Builtin containers must use their native structural generators so a
    // witness list can still shrink its length. Seed payloads recursively;
    // retain named values as fallbacks for uneven recursive node allocations.
    if !matches!(
        value,
        Value::List(_)
            | Value::Maybe(_)
            | Value::Nullable(_)
            | Value::Optional(_)
            | Value::Left(_)
            | Value::Right(_)
    ) {
        seeds.entry(ty.clone()).or_default().push(value.clone());
    }
    if let Some(constructors) = schema.constructor_fields(ty)? {
        let Value::Data(tag, fields) = value else {
            return Err("invalid data witness".into());
        };
        let constructor = constructors
            .iter()
            .find(|c| c.tag == tag)
            .ok_or("unknown witness constructor")?;
        let field_types = schema.witnessed_fields(constructor.tag, &constructor.fields, fields)?;
        for (field_type, field) in field_types.iter().zip(fields) {
            collect_witnesses(schema, field_type, field, seeds)?;
        }
    } else if let ls::TypeRef::Named(_, arguments) = ty {
        match value {
            Value::List(values) => {
                for value in values {
                    collect_witnesses(schema, &arguments[0], value, seeds)?;
                }
            }
            Value::Maybe(Some(value))
            | Value::Nullable(Some(value))
            | Value::Optional(Some(value))
            | Value::Left(value) => {
                collect_witnesses(schema, &arguments[0], value, seeds)?;
            }
            Value::Right(value) => collect_witnesses(schema, &arguments[1], value, seeds)?,
            _ => {}
        }
    }
    Ok(())
}

/// Reject only false predicates, across whole candidates rather than individual
/// branches. Native proptest filtering bounds retries and preserves shrinking.
/// Evaluation errors remain Err values that the property must report as failures.
/// Context clones preserve Symbol identity without mutating it during shrinking.
pub fn checked_schema_strategy(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
    context: &ls::Context,
    witnesses: Vec<Value>,
) -> ls::Result<BoxedStrategy<ls::Result<Value>>> {
    checked_schema_strategy_with_generators(
        schema,
        ty,
        bits,
        budget,
        context,
        witnesses,
        &NativeGenerators::default(),
    )
}

pub fn checked_schema_strategy_with_generators(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
    context: &ls::Context,
    witnesses: Vec<Value>,
    native: &NativeGenerators,
) -> ls::Result<BoxedStrategy<ls::Result<Value>>> {
    let mut seeds = Witnesses::new();
    for witness in witnesses {
        let value = schema.validate_with_context(witness, ty, bits, &mut context.clone())?;
        if value_cost(&value) > budget {
            return Err("constructor witness exceeds structural node budget".into());
        }
        collect_witnesses(schema, ty, &value, &mut seeds)?;
    }
    let strategy =
        shape_strategy_with_generators(schema, ty, bits, budget, seeds, context, native)?;
    let schema = schema.clone();
    let ty = ty.clone();
    let context = context.clone();
    Ok(strategy
        .prop_filter_map("constructor field refinement", move |value| {
            match schema.check_with_context(value, &ty, bits, &mut context.clone()) {
                Ok(ls::ValueCheck::Accepted(value)) => Some(Ok(value)),
                Ok(ls::ValueCheck::Rejected(_)) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .boxed())
}

fn shape_strategy(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
    witnesses: Witnesses,
) -> ls::Result<BoxedStrategy<Value>> {
    shape_strategy_with_generators(
        schema,
        ty,
        bits,
        budget,
        witnesses,
        &ls::Context::default(),
        &NativeGenerators::default(),
    )
}

fn shape_strategy_with_generators(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
    witnesses: Witnesses,
    context: &ls::Context,
    native: &NativeGenerators,
) -> ls::Result<BoxedStrategy<Value>> {
    type Request = (ls::TypeRef, usize);
    struct Builder<'a> {
        schema: &'a ls::Schema,
        context: &'a ls::Context,
        native: &'a NativeGenerators,
        bits: u32,
        witnesses: Witnesses,
        inhabited: std::collections::HashMap<Request, bool>,
        generators: std::collections::HashMap<Request, BoxedStrategy<Value>>,
    }

    impl Builder<'_> {
        fn can_generate(&mut self, ty: &ls::TypeRef, budget: usize) -> ls::Result<bool> {
            if budget == 0 {
                return Ok(false);
            }
            let key = (ty.clone(), budget);
            if let Some(result) = self.inhabited.get(&key) {
                return Ok(*result);
            }
            let result = if self.native.factory(ty).is_some() {
                true
            } else if let Some(constructors) = self.schema.constructor_fields(ty)? {
                let mut found = false;
                for constructor in constructors {
                    for (fields, _) in self.schema.witness_choices(&constructor)? {
                        if self.allocate(&fields, budget - 1)?.is_some() {
                            found = true;
                        }
                    }
                }
                found
            } else {
                let ls::TypeRef::Named(name, arguments) = ty else {
                    return Err("uninstantiated generator type".into());
                };
                match (*name, arguments.as_slice()) {
                    ("List" | "Maybe" | "Nullable" | "Optional", [_]) => true,
                    ("Either", [left, right]) => {
                        self.can_generate(left, budget - 1)?
                            || self.can_generate(right, budget - 1)?
                    }
                    (_, []) => true,
                    _ => return Err(format!("unsupported generator type: {ty:?}")),
                }
            };
            self.inhabited.insert(key, result);
            Ok(result)
        }

        fn minimum(&mut self, ty: &ls::TypeRef, limit: usize) -> ls::Result<Option<usize>> {
            for cost in 1..=limit {
                if self.can_generate(ty, cost)? {
                    return Ok(Some(cost));
                }
            }
            Ok(None)
        }

        // Reserve every field's minimum before sharing the remaining nodes.
        // A deep field must not disappear merely because its siblings are small.
        fn allocate(
            &mut self,
            fields: &[ls::TypeRef],
            budget: usize,
        ) -> ls::Result<Option<Vec<usize>>> {
            let mut remaining = budget;
            let mut costs = Vec::new();
            for field in fields {
                let Some(cost) = self.minimum(field, remaining)? else {
                    return Ok(None);
                };
                costs.push(cost);
                remaining -= cost;
            }
            let count = costs.len();
            for (index, cost) in costs.iter_mut().enumerate() {
                *cost += remaining / count + usize::from(index < remaining % count);
            }
            Ok(Some(costs))
        }

        fn generate(
            &mut self,
            ty: &ls::TypeRef,
            budget: usize,
        ) -> ls::Result<BoxedStrategy<Value>> {
            if !self.can_generate(ty, budget)? {
                return Err(format!(
                    "no value within structural node budget {budget}: {ty:?}"
                ));
            }
            let key = (ty.clone(), budget);
            if let Some(cached) = self.generators.get(&key) {
                return Ok(cached.clone());
            }
            let result = if let Some(factory) = self.native.factory(ty) {
                let ls::TypeRef::Named(_, arguments) = ty else {
                    return Err("uninstantiated native generator type".into());
                };
                let mut children = Vec::new();
                for argument in arguments {
                    let remaining = budget.saturating_sub(1).max(1);
                    let child = if self.can_generate(argument, remaining)? {
                        self.generate(argument, remaining)?
                    } else {
                        // Phantom parameters may be uninhabited. Keep a native
                        // strategy argument that rejects if actually sampled.
                        Just(Value::Unit)
                            .prop_filter(
                                format!("no native generator argument within node budget {remaining}: {argument:?}"),
                                |_| false,
                            )
                            .boxed()
                    };
                    children.push(child);
                }
                let strategy = factory(self.schema, ty, self.bits, self.context, children)?;
                let schema = self.schema.clone();
                let ty = ty.clone();
                let bits = self.bits;
                let context = self.context.clone();
                strategy
                    .prop_map(move |value| {
                        schema
                            .validate_with_context(value, &ty, bits, &mut context.clone())
                            .unwrap_or_else(|error| panic!("native generator {ty:?}: {error}"))
                    })
                    .boxed()
            } else if let Some(constructors) = self.schema.constructor_fields(ty)? {
                let mut alternatives = Vec::new();
                for constructor in constructors {
                  // A field-only existential takes each type of the witness pool.
                  for (declared, witnesses) in self.schema.witness_choices(&constructor)? {
                    let Some(costs) = self.allocate(&declared, budget - 1)? else {
                        continue;
                    };
                    let mut fields = Just(Vec::<Value>::new()).boxed();
                    for (field, cost) in declared.iter().zip(costs) {
                        let next = self.generate(field, cost)?;
                        fields = (fields, next)
                            .prop_map(|(mut values, value)| {
                                values.push(value);
                                values
                            })
                            .boxed();
                    }
                    let tag = constructor.tag;
                    alternatives.push(
                        fields
                            .prop_map(move |mut fields| {
                                fields.extend(witnesses.iter().cloned());
                                ls::construct_data(tag, fields)
                            })
                            .boxed(),
                    );
                  }
                }
                let union = proptest::strategy::Union::new(alternatives).boxed();
                // Generated collections are canonicalised rather than filtered.
                match ty {
                    ls::TypeRef::Named(name, _)
                        if name.starts_with(ls::COLLECTIONS) && (name.ends_with("::Set") || name.ends_with("::KeyVal")) =>
                    {
                        let keyed = name.ends_with("::KeyVal");
                        union
                            .prop_map(move |value| match value {
                                Value::Data(tag, mut fields) if fields.len() == 1 => match fields.pop().unwrap() {
                                    Value::List(items) => Value::Data(
                                        tag,
                                        vec![Value::List(ls::canonical_items(items, keyed).expect("keyed values have an order"))],
                                    ),
                                    other => Value::Data(tag, vec![other]),
                                },
                                other => other,
                            })
                            .boxed()
                    }
                    _ => union,
                }
            } else {
                let ls::TypeRef::Named(name, arguments) = ty else {
                    return Err("uninstantiated generator type".into());
                };
                match (*name, arguments.as_slice()) {
                    ("List", [element]) => {
                        let remaining = budget - 1;
                        let maximum = self
                            .minimum(element, remaining)?
                            .map_or(0, |cost| remaining / cost);
                        let mut lengths = vec![Just(Value::List(vec![])).boxed()];
                        for length in 1..=maximum {
                            let elements = self.generate(element, remaining / length)?;
                            lengths.push(
                                proptest::collection::vec(elements, length..=length)
                                    .prop_map(Value::List)
                                    .boxed(),
                            );
                        }
                        // Native dependent shrinking changes length and then
                        // shrinks elements under the recomputed size bound.
                        (0..=maximum)
                            .prop_flat_map(move |length| lengths[length].clone())
                            .boxed()
                    }
                    ("Maybe" | "Nullable" | "Optional", [element]) => {
                        let values = if self.can_generate(element, budget - 1)? {
                            proptest::option::of(self.generate(element, budget - 1)?).boxed()
                        } else {
                            Just(None).boxed()
                        };
                        let name = *name;
                        values
                            .prop_map(move |value| match name {
                                "Maybe" => Value::Maybe(value.map(Box::new)),
                                "Nullable" => Value::Nullable(value.map(Box::new)),
                                _ => Value::Optional(value.map(Box::new)),
                            })
                            .boxed()
                    }
                    ("Either", [left, right]) => {
                        let mut alternatives = Vec::new();
                        if self.can_generate(left, budget - 1)? {
                            alternatives.push(
                                self.generate(left, budget - 1)?
                                    .prop_map(|value| Value::Left(Box::new(value)))
                                    .boxed(),
                            );
                        }
                        if self.can_generate(right, budget - 1)? {
                            alternatives.push(
                                self.generate(right, budget - 1)?
                                    .prop_map(|value| Value::Right(Box::new(value)))
                                    .boxed(),
                            );
                        }
                        proptest::strategy::Union::new(alternatives).boxed()
                    }
                    (_, []) => strategy_with_profile(name, self.bits)?,
                    _ => return Err(format!("unsupported generator type: {ty:?}")),
                }
            };
            let seeds = self
                .witnesses
                .get(ty)
                .into_iter()
                .flatten()
                .filter(|value| value_cost(value) <= budget)
                .cloned()
                .collect();
            let result = if self.native.factory(ty).is_some() {
                result
            } else {
                seeded(result, seeds)
            };
            self.generators.insert(key, result.clone());
            Ok(result)
        }
    }

    if !matches!(bits, 32 | 64) {
        return Err("machineBits must be 32 or 64".into());
    }
    Builder {
        schema,
        context,
        native,
        bits,
        witnesses,
        inhabited: Default::default(),
        generators: Default::default(),
    }
    .generate(ty, budget)
}

const INDEX_SLACK: u64 = 16;
const INDEX_CHOICES: usize = 6;

/// A prefix index term over field indices (`f<i>`), literals (`c<n>`) and the
/// natural operators `+ - * div mod ^`.
#[derive(Clone, Debug)]
enum IndexTerm {
    Constant(u64),
    Field(usize),
    Apply(String, Box<IndexTerm>, Box<IndexTerm>),
}

fn parse_index_term(tokens: &[&str], at: &mut usize) -> ls::Result<IndexTerm> {
    let token = *tokens.get(*at).ok_or("malformed index term")?;
    *at += 1;
    if let Some(digits) = token.strip_prefix('c') {
        return Ok(IndexTerm::Constant(digits.parse().map_err(|_| "malformed index term")?));
    }
    if let Some(digits) = token.strip_prefix('f') {
        return Ok(IndexTerm::Field(digits.parse().map_err(|_| "malformed index term")?));
    }
    if !["+", "-", "*", "div", "mod", "^"].contains(&token) {
        return Err("malformed index term".into());
    }
    let left = parse_index_term(tokens, at)?;
    let right = parse_index_term(tokens, at)?;
    Ok(IndexTerm::Apply(token.to_string(), Box::new(left), Box::new(right)))
}

/// Natural index arithmetic; `None` when an operation has no natural value.
fn eval_index_term(term: &IndexTerm, fields: &[(usize, u64)]) -> Option<u64> {
    match term {
        IndexTerm::Constant(value) => Some(*value),
        IndexTerm::Field(position) => fields.iter().find(|(p, _)| p == position).map(|(_, v)| *v),
        IndexTerm::Apply(op, left, right) => {
            let x = eval_index_term(left, fields)?;
            let y = eval_index_term(right, fields)?;
            match op.as_str() {
                "+" => x.checked_add(y),
                "-" => x.checked_sub(y),
                "*" => x.checked_mul(y),
                "div" => x.checked_div(y),
                "mod" => x.checked_rem(y),
                _ => u32::try_from(y).ok().filter(|e| *e <= 64).and_then(|e| x.checked_pow(e)),
            }
        }
    }
}

fn index_term_fields(term: &IndexTerm, into: &mut Vec<usize>) {
    match term {
        IndexTerm::Constant(_) => {}
        IndexTerm::Field(position) => {
            if !into.contains(position) {
                into.push(*position);
            }
        }
        IndexTerm::Apply(_, left, right) => {
            index_term_fields(left, into);
            index_term_fields(right, into);
        }
    }
}

type IndexAssignment = Vec<(usize, u64)>;

struct IndexVariant {
    tag: &'static str,
    fields: Vec<ls::TypeRef>,
    plain: Vec<Option<BoxedStrategy<Value>>>,
    term: IndexTerm,
    guards: Vec<(bool, IndexTerm, IndexTerm)>,
    positions: Vec<usize>,
}

impl IndexVariant {
    fn ready(&self) -> bool {
        self.plain
            .iter()
            .enumerate()
            .all(|(index, strategy)| self.positions.contains(&index) || strategy.is_some())
    }

    /// Every guard-satisfying assignment of reachable indices to the index
    /// fields, with the index it produces.
    fn assignments(
        &self,
        reach: &std::collections::HashSet<(ls::TypeRef, u64)>,
        limit: u64,
    ) -> Vec<(u64, IndexAssignment)> {
        let mut found = Vec::new();
        let mut current = Vec::new();
        self.extend(reach, limit, 0, &mut current, &mut found);
        found
    }

    fn extend(
        &self,
        reach: &std::collections::HashSet<(ls::TypeRef, u64)>,
        limit: u64,
        at: usize,
        current: &mut IndexAssignment,
        found: &mut Vec<(u64, IndexAssignment)>,
    ) {
        if at == self.positions.len() {
            let holds = self.guards.iter().all(|(equal, left, right)| {
                match (eval_index_term(left, current), eval_index_term(right, current)) {
                    (Some(x), Some(y)) => if *equal { x == y } else { x >= y },
                    _ => false,
                }
            });
            if holds {
                if let Some(value) = eval_index_term(&self.term, current).filter(|v| *v <= limit) {
                    found.push((value, current.clone()));
                }
            }
            return;
        }
        let position = self.positions[at];
        for value in 0..=limit {
            if reach.contains(&(self.fields[position].clone(), value)) {
                current.push((position, value));
                self.extend(reach, limit, at + 1, current, found);
                current.pop();
            }
        }
    }
}

/// Solved index levels: per family and level, each feasible variant with the
/// field-index assignments that produce that level.
struct IndexTable {
    variants: std::collections::HashMap<ls::TypeRef, Vec<IndexVariant>>,
    solutions: std::collections::HashMap<(ls::TypeRef, u64), Vec<(usize, Vec<IndexAssignment>)>>,
}

/// A lazily built strategy for one family at one level, so children may sit
/// at higher levels than their parents.
#[derive(Clone)]
struct IndexedValue {
    table: std::sync::Arc<IndexTable>,
    family: ls::TypeRef,
    level: u64,
}

impl std::fmt::Debug for IndexedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IndexedValue({:?}, {})", self.family, self.level)
    }
}

impl Strategy for IndexedValue {
    type Tree = Box<dyn proptest::strategy::ValueTree<Value = Value>>;
    type Value = Value;

    fn new_tree(&self, runner: &mut proptest::test_runner::TestRunner) -> proptest::strategy::NewTree<Self> {
        let variants = &self.table.variants[&self.family];
        let alternatives: Vec<BoxedStrategy<Value>> = self.table.solutions
            [&(self.family.clone(), self.level)]
            .iter()
            .map(|(index, choices)| {
                let variant = &variants[*index];
                let table = self.table.clone();
                let fields = variant.fields.clone();
                let plain = variant.plain.clone();
                let tag = variant.tag;
                proptest::sample::select(choices.clone())
                    .prop_flat_map(move |assignment| {
                        let children: Vec<BoxedStrategy<Value>> = fields
                            .iter()
                            .enumerate()
                            .map(|(index, field)| {
                                match assignment.iter().find(|(p, _)| *p == index) {
                                    Some((_, level)) => IndexedValue {
                                        table: table.clone(),
                                        family: field.clone(),
                                        level: *level,
                                    }
                                    .boxed(),
                                    None => plain[index].clone().unwrap(),
                                }
                            })
                            .collect();
                        children.prop_map(move |values| ls::construct_data(tag, values))
                    })
                    .boxed()
            })
            .collect();
        proptest::strategy::Union::new(alternatives)
            .new_tree(runner)
            .map(|tree| Box::new(tree) as Box<dyn proptest::strategy::ValueTree<Value = Value>>)
    }
}

/// Values whose structural index equals `target`. Each constructor carries its
/// index term then its guards, in prefix notation over field indices.
/// Reachability is a forward fixpoint over levels 0..target+slack, so a child
/// may exceed its parent's index; the target is then solved backwards, and
/// generation never filters. Assignments shrink toward the first.
pub fn indexed_strategy(
    schema: &ls::Schema,
    ty: &ls::TypeRef,
    bits: u32,
    budget: usize,
    target: &Value,
    equations: &[(&str, &[&str])],
) -> ls::Result<BoxedStrategy<Value>> {
    let Value::Integer(target) = target else {
        return Err("index target must be a natural number".into());
    };
    let requested: Option<u64> = target.to_string().parse().ok();
    let limit = requested.unwrap_or(0).saturating_add(INDEX_SLACK);
    let parse = |tag: &str| -> ls::Result<(IndexTerm, Vec<(bool, IndexTerm, IndexTerm)>, Vec<usize>)> {
        let (_, texts) = equations
            .iter()
            .find(|(name, _)| *name == tag)
            .ok_or_else(|| format!("missing index equation for {tag}"))?;
        let first = texts.first().ok_or_else(|| format!("missing index equation for {tag}"))?;
        let tokens: Vec<&str> = first.split(' ').collect();
        let mut at = 0;
        let term = parse_index_term(&tokens, &mut at)?;
        if at != tokens.len() {
            return Err("malformed index term".into());
        }
        let mut positions = Vec::new();
        index_term_fields(&term, &mut positions);
        let mut guards = Vec::new();
        for text in &texts[1..] {
            let parts: Vec<&str> = text.split(' ').collect();
            let equal = match parts.first() {
                Some(&"==") => true,
                Some(&">=") => false,
                _ => return Err("malformed index guard".into()),
            };
            let mut at = 1;
            let left = parse_index_term(&parts, &mut at)?;
            let right = parse_index_term(&parts, &mut at)?;
            index_term_fields(&left, &mut positions);
            index_term_fields(&right, &mut positions);
            guards.push((equal, left, right));
        }
        Ok((term, guards, positions))
    };
    let mut variants: std::collections::HashMap<ls::TypeRef, Vec<IndexVariant>> =
        std::collections::HashMap::new();
    let mut plain: std::collections::HashMap<ls::TypeRef, Option<BoxedStrategy<Value>>> =
        std::collections::HashMap::new();
    let mut pending = vec![ty.clone()];
    while let Some(next) = pending.pop() {
        if variants.contains_key(&next) {
            continue;
        }
        let constructors = schema
            .constructor_fields(&next)?
            .ok_or("indexed generation requires a data type")?;
        let mut family = Vec::new();
        for constructor in constructors {
            let (term, guards, positions) = parse(constructor.tag)?;
            pending.extend(positions.iter().map(|p| constructor.fields[*p].clone()));
            let mut strategies = Vec::new();
            for (index, field) in constructor.fields.iter().enumerate() {
                if positions.contains(&index) {
                    strategies.push(None);
                    continue;
                }
                if !plain.contains_key(field) {
                    let strategy = shape_strategy(schema, field, bits, budget, Witnesses::new()).ok();
                    plain.insert(field.clone(), strategy);
                }
                strategies.push(plain[field].clone());
            }
            family.push(IndexVariant {
                tag: constructor.tag,
                fields: constructor.fields.clone(),
                plain: strategies,
                term,
                guards,
                positions,
            });
        }
        variants.insert(next, family);
    }
    let mut reach = std::collections::HashSet::new();
    loop {
        let mut next = reach.clone();
        for (family, list) in &variants {
            for variant in list.iter().filter(|variant| variant.ready()) {
                for (value, _) in variant.assignments(&reach, limit) {
                    next.insert((family.clone(), value));
                }
            }
        }
        if next.len() == reach.len() {
            break;
        }
        reach = next;
    }
    // An open target (negative), or one that names no value, generates from the
    // smallest reachable indices; an index claim rejects a mismatch.
    let levels: Vec<u64> = match requested.filter(|k| reach.contains(&(ty.clone(), *k))) {
        Some(k) => vec![k],
        None => (0..=limit)
            .filter(|level| reach.contains(&(ty.clone(), *level)))
            .take(INDEX_CHOICES)
            .collect(),
    };
    if levels.is_empty() {
        return Err(format!("no value of {ty:?} has an index"));
    }
    let mut solutions: std::collections::HashMap<(ls::TypeRef, u64), Vec<(usize, Vec<IndexAssignment>)>> =
        std::collections::HashMap::new();
    for (family, list) in &variants {
        for (index, variant) in list.iter().enumerate().filter(|(_, variant)| variant.ready()) {
            let mut by_level: std::collections::BTreeMap<u64, Vec<IndexAssignment>> =
                std::collections::BTreeMap::new();
            for (value, assignment) in variant.assignments(&reach, limit) {
                by_level.entry(value).or_default().push(assignment);
            }
            for (level, choices) in by_level {
                solutions.entry((family.clone(), level)).or_default().push((index, choices));
            }
        }
    }
    let table = std::sync::Arc::new(IndexTable { variants, solutions });
    let family = ty.clone();
    Ok(proptest::sample::select(levels)
        .prop_flat_map(move |level| IndexedValue { table: table.clone(), family: family.clone(), level })
        .boxed())
}
