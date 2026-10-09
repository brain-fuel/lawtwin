//! Portable scalar semantics for Rust, with no dependency on a test framework.
//!
//! A law must mean the same thing on every target, so this crate module
//! implements LawSpec's own arithmetic, equality and conversions instead of
//! Rust's machine integers and floats: integers are exact and reach a bounded
//! type only through a checked conversion, exact division gives a rational,
//! decimals are exact, and floats follow IEEE 754 at their declared precision.
//! ref:DEC-portable-exact-arithmetic ref:ieee-754 ref:decimal-arithmetic
//!
//! It is emitted into every generated Rust crate, so the generated tests and
//! the adapters share one definition of the domain. ref:DEC-typed-core-boundary
pub use num_bigint::{BigInt, BigUint};
pub use num_complex::{Complex32, Complex64};
pub use num_rational::BigRational;
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};
use std::collections::HashMap;
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, String>;

/// Runs a generated test on a thread with a large stack: generated values of
/// deep data, in debug builds, can exceed the default test thread's stack.
pub fn with_stack<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(body)
        .expect("test thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// A logical Integer result accepts any native integer without losing bits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Integer(pub BigInt);
macro_rules! integer_from {
    ($($t:ty),*) => {$ (impl From<$t> for Integer {
        fn from(value: $t) -> Self { Self(BigInt::from(value)) }
    })*};
}
integer_from!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, BigInt, BigUint
);

/// Finite base-ten value; arithmetic is independent of ambient rounding modes.
#[derive(Clone, Debug)]
pub struct Decimal {
    pub coefficient: BigInt,
    pub exponent: BigInt,
}
// Decimals compare by value, so 1.0 and 1.00 are one key.
impl Eq for Decimal {}
impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.ratio(), other.ratio()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => (&self.coefficient, &self.exponent).cmp(&(&other.coefficient, &other.exponent)),
        }
    }
}

impl Decimal {
    pub fn new(coefficient: BigInt, exponent: BigInt) -> Self {
        Self {
            coefficient,
            exponent,
        }
    }
    pub fn ratio(&self) -> Result<BigRational> {
        let exponent = self
            .exponent
            .abs()
            .to_u32()
            .ok_or("decimal exponent exceeds runtime capacity")?;
        let power = BigInt::from(10u8).pow(exponent);
        Ok(if self.exponent.is_negative() {
            BigRational::new(self.coefficient.clone(), power)
        } else {
            BigRational::from_integer(&self.coefficient * power)
        })
    }
    pub fn from_ratio(value: BigRational) -> Result<Self> {
        let mut denominator = value.denom().clone();
        let mut twos = 0u32;
        let mut fives = 0u32;
        while (&denominator % 2u8).is_zero() {
            denominator /= 2u8;
            twos += 1;
        }
        while (&denominator % 5u8).is_zero() {
            denominator /= 5u8;
            fives += 1;
        }
        if !denominator.is_one() {
            return Err("conversion to Decimal is not finite; use prelude.round".into());
        }
        let scale = twos.max(fives);
        Ok(Self::new(
            value.numer()
                * BigInt::from(2u8).pow(scale - twos)
                * BigInt::from(5u8).pow(scale - fives),
            -BigInt::from(scale),
        ))
    }
    pub fn add(&self, other: &Self) -> Result<Self> {
        Self::from_ratio(self.ratio()? + other.ratio()?)
    }
    pub fn sub(&self, other: &Self) -> Result<Self> {
        Self::from_ratio(self.ratio()? - other.ratio()?)
    }
    pub fn mul(&self, other: &Self) -> Result<Self> {
        Self::from_ratio(self.ratio()? * other.ratio()?)
    }
    pub fn round(value: BigRational, scale: i32) -> Result<Self> {
        let power = BigInt::from(10u8).pow(scale.unsigned_abs());
        let factor = if scale < 0 {
            BigRational::new(BigInt::one(), power)
        } else {
            BigRational::from_integer(power)
        };
        let scaled = &value * &factor;
        let quotient = scaled.numer() / scaled.denom();
        let remainder = scaled.numer() % scaled.denom();
        let twice = remainder.abs() * 2u8;
        let rounded = if twice > *scaled.denom()
            || twice == *scaled.denom() && !(&quotient % 2u8).is_zero()
        {
            quotient
                + if scaled.is_negative() {
                    -BigInt::one()
                } else {
                    BigInt::one()
                }
        } else {
            quotient
        };
        Self::from_ratio(BigRational::from_integer(rounded) / factor)
    }
}
impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        matches!((self.ratio(),other.ratio()), (Ok(a),Ok(b)) if a==b)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodePoint(pub u32);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodePointText(pub Vec<u32>);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Utf16Text(pub Vec<u16>);
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Null;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Undefined;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Nullable<T> {
    Null,
    Present(T),
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Optional<T> {
    Undefined,
    Present(T),
}
#[derive(Clone, Debug)]
pub struct Symbol(Arc<String>);
impl Symbol {
    pub fn new(description: String) -> Self {
        Self(Arc::new(description))
    }
    pub fn description(&self) -> &str {
        &self.0
    }
}
impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Symbol {}
#[derive(Clone, Debug, Default)]
pub struct Context {
    symbols: HashMap<String, Symbol>,
    /// The workflow runtime this call runs under; None is the default one.
    pub workflow: Option<Arc<std::sync::Mutex<WorkflowRuntime>>>,
    /// The handler installed for each ability, by its key (evidence passing).
    /// Copies of a context share it, so a spec handler (which keeps a copy)
    /// sees the handlers installed after it, for the abilities its clauses use.
    pub handlers: Arc<std::sync::RwLock<HashMap<String, Installed>>>,
    // The default runtime's view under the installed Clock handler.
    clock_view: ClockView,
}
impl Context {
    /// A context whose workflows run under the given runtime.
    pub fn with_workflow(runtime: WorkflowRuntime) -> Context {
        Context { workflow: Some(Arc::new(std::sync::Mutex::new(runtime))), ..Context::default() }
    }
    /// A context for a generated test: workflows wait on a virtual clock, and
    /// gates are off (a workflow law calls the workflow and its composition,
    /// which would see each other's state). Timeouts and hedges stay on: they
    /// count virtual time (see scoped).
    pub fn testing() -> Context {
        let mut runtime = WorkflowRuntime::new(Box::new(VirtualClock::default()), 0);
        runtime.gates = false;
        Context::with_workflow(runtime)
    }
    pub fn symbol(&mut self, id: &str, description: &str) -> Symbol {
        self.symbols
            .entry(id.into())
            .or_insert_with(|| Symbol::new(description.into()))
            .clone()
    }
}

/// A value of a LawSpec type, tagged with that type, because a Rust value alone
/// cannot say whether it is an Int32, a BigInt or a Decimal, and the law's
/// semantics depend on which. ref:DEC-portable-exact-arithmetic
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Integer(BigInt),
    Decimal(Decimal),
    Rational(BigRational),
    Float32(f32),
    Float64(f64),
    Complex32(Complex32),
    Complex64(Complex64),
    Bool(bool),
    Char(char),
    CodePoint(u32),
    CodeUnit16(u16),
    Text(String),
    CodePointText(Vec<u32>),
    Utf16Text(Vec<u16>),
    Bytes(Vec<u8>),
    Symbol(Symbol),
    Unit,
    Null,
    Undefined,
    Nullable(Option<Box<Value>>),
    Optional(Option<Box<Value>>),
    List(Vec<Value>),
    Maybe(Option<Box<Value>>),
    Left(Box<Value>),
    Right(Box<Value>),
    // The compiler supplies unit-qualified constructor identities. Payloads are
    // checked by the generated typed bridges, preserving nested sum states.
    Data(String, Vec<Value>),
    // A value only adapters create, passed along unopened.
    Handle(Handle),
}

/// A handle: a value only adapters create, such as a concurrent queue.
/// LawSpec never builds, generates or looks inside one; two handles are equal
/// only when they are the same, and they have no portable order. Clones share
/// the native value, so a handle stays itself however often it is passed.
#[derive(Clone)]
pub struct Handle(Arc<HandleCell>);

struct HandleCell {
    value: Box<dyn std::any::Any + Send + Sync>,
    // The handle type's name, recorded when a schema first checks it, and
    // its number among that type's handles, given when it is first shown.
    name: std::sync::OnceLock<&'static str>,
    number: std::sync::OnceLock<u64>,
}

impl Handle {
    /// A new handle around a native value.
    pub fn new<T: std::any::Any + Send + Sync>(value: T) -> Self {
        Handle(Arc::new(HandleCell {
            value: Box::new(value),
            name: std::sync::OnceLock::new(),
            number: std::sync::OnceLock::new(),
        }))
    }

    /// The native value, when it is a T.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.value.downcast_ref::<T>()
    }

    /// The native value as a T, or an error naming both.
    pub fn native<T: std::any::Any>(&self) -> Result<&T> {
        self.downcast_ref::<T>()
            .ok_or_else(|| format!("handle {} does not hold a {}", self.label(), std::any::type_name::<T>()))
    }

    /// Whether two handles are the same one.
    pub fn same(&self, other: &Handle) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn named(&self, name: &'static str) {
        let _ = self.0.name.set(name);
    }

    /// A stable label, such as Jobs#1: the type's name, then the handle's
    /// number among that type's handles in order of first appearance.
    pub fn label(&self) -> String {
        use std::sync::{Mutex, OnceLock};
        static COUNTS: OnceLock<Mutex<HashMap<&'static str, u64>>> = OnceLock::new();
        let full = self.0.name.get().copied().unwrap_or("Handle");
        let name = full.rsplit("::").next().unwrap_or(full);
        let number = *self.0.number.get_or_init(|| {
            let mut counts = COUNTS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner());
            let count = counts.entry(name).or_insert(0);
            *count += 1;
            *count
        });
        format!("{name}#{number}")
    }
}

impl PartialEq for Handle {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}
impl Eq for Handle {}
impl std::hash::Hash for Handle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as *const () as usize).hash(state);
    }
}
impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

impl Value {
    pub fn exact(&self) -> Result<BigRational> {
        match self {
            Self::Integer(n) => Ok(BigRational::from_integer(n.clone())),
            Self::Decimal(d) => d.ratio(),
            Self::Rational(r) => Ok(r.clone()),
            _ => Err("requires exact number".into()),
        }
    }
    pub fn integer(&self) -> Result<BigInt> {
        let r = self.exact()?;
        if !r.is_integer() {
            return Err("fractional integer conversion".into());
        }
        Ok(r.to_integer())
    }
    pub fn boolean(&self) -> Result<bool> {
        match self {
            Self::Bool(b) => Ok(*b),
            _ => Err("expected Bool".into()),
        }
    }
    fn real64(&self) -> Result<f64> {
        match self {
            Self::Float32(f) => Ok(f64::from(*f)),
            Self::Float64(f) => Ok(*f),
            _ => Ok(f64::from_bits(exact_float_bits(&self.exact()?, false)?)),
        }
    }
    fn complex64(&self) -> Result<Complex64> {
        match self {
            Self::Complex32(c) => Ok(Complex64::new(f64::from(c.re), f64::from(c.im))),
            Self::Complex64(c) => Ok(*c),
            _ => Ok(Complex64::new(self.real64()?, 0.0)),
        }
    }
    pub fn convert(self, target: &str, machine_bits: u32) -> Result<Self> {
        if target == "Float32" {
            // Rational-to-f32 is direct, avoiding double rounding through f64.
            return Ok(Self::Float32(match &self {
                Self::Float32(x) => *x,
                Self::Float64(x) => *x as f32,
                _ => f32::from_bits(exact_float_bits(&self.exact()?, true)? as u32),
            }));
        }
        if target == "Float64" {
            return Ok(Self::Float64(self.real64()?));
        }
        if target == "Complex64" || target == "Complex128" {
            if target == "Complex128" {
                return Ok(Self::Complex64(self.complex64()?));
            }
            let (real, imag) = match self {
                Self::Complex32(c) => return Ok(Self::Complex32(c)),
                Self::Complex64(c) => (Self::Float64(c.re), Self::Float64(c.im)),
                x => (x, Self::Float32(0.0)),
            };
            return Ok(Self::Complex32(Complex32::new(
                f32::from_value(real.convert("Float32", machine_bits)?)?,
                f32::from_value(imag.convert("Float32", machine_bits)?)?,
            )));
        }
        let exact = match self {
            Self::Float32(f) => {
                Self::Rational(BigRational::from_f32(f).ok_or("non-finite exact conversion")?)
            }
            Self::Float64(f) => {
                Self::Rational(BigRational::from_f64(f).ok_or("non-finite exact conversion")?)
            }
            x => x,
        };
        if target == "Decimal" {
            return Ok(Self::Decimal(Decimal::from_ratio(exact.exact()?)?));
        }
        if target == "Rational" {
            return Ok(Self::Rational(exact.exact()?));
        }
        let (signed, width) = match target {
            "Int8" => (true, 8),
            "Int16" => (true, 16),
            "Int32" => (true, 32),
            "Int64" => (true, 64),
            "UInt8" => (false, 8),
            "UInt16" => (false, 16),
            "UInt32" => (false, 32),
            "UInt64" => (false, 64),
            "IntSize" => (true, machine_bits),
            "UIntSize" | "UIntPtr" => (false, machine_bits),
            "Integer" | "BigInt" => return Ok(Self::Integer(exact.integer()?)),
            "BigUInt" => {
                let n = exact.integer()?;
                if n.is_negative() {
                    return Err("BigUInt cannot be negative".into());
                }
                return Ok(Self::Integer(n));
            }
            _ => return Err(format!("unsupported numeric conversion: {target}")),
        };
        let n = exact.integer()?;
        let (lo, hi) = if signed {
            let bound = BigInt::one() << (width - 1);
            (-&bound, &bound - 1u8)
        } else {
            (BigInt::zero(), (BigInt::one() << width) - 1u8)
        };
        if n < lo || n > hi {
            return Err(format!("integer outside {target} range"));
        }
        Ok(Self::Integer(n))
    }
}

/// Round the exact ratio once, directly into an IEEE significand and exponent.
pub fn exact_float_bits(ratio: &BigRational, single: bool) -> Result<u64> {
    let (precision, bias, sign_shift) = if single {
        (24i64, 127i64, 31)
    } else {
        (53i64, 1023i64, 63)
    };
    let sign = u64::from(ratio.is_negative()) << sign_shift;
    if ratio.is_zero() {
        return Ok(sign);
    }
    let n = ratio.numer().abs();
    let d = ratio.denom().clone();
    let mut exponent = n.bits() as i64 - d.bits() as i64;
    if if exponent >= 0 {
        n < (&d << exponent as usize)
    } else {
        (&n << (-exponent) as usize) < d
    } {
        exponent -= 1;
    }
    let infinity = sign | (((2 * bias + 1) as u64) << (precision - 1));
    if exponent > bias {
        return Ok(infinity);
    }
    let minimum = 1 - bias;
    if exponent < minimum - precision {
        return Ok(sign);
    }
    let scale = exponent.max(minimum) - (precision - 1);
    let num = if scale < 0 { n << (-scale) as usize } else { n };
    let den = if scale > 0 { d << scale as usize } else { d };
    let mut q = &num / &den;
    let twice = (&num % &den) * 2u8;
    if twice > den || twice == den && !(&q % 2u8).is_zero() {
        q += 1u8;
    }
    exponent = exponent.max(minimum);
    if q == (BigInt::one() << precision as usize) {
        q >>= 1usize;
        exponent += 1;
    }
    if exponent > bias {
        return Ok(infinity);
    }
    let hidden = 1u64 << (precision - 1);
    let significand = q.to_u64().ok_or("invalid IEEE significand")?;
    let (exp, mantissa) = if significand < hidden {
        (0, significand)
    } else {
        ((exponent + bias) as u64, significand - hidden)
    };
    Ok(sign | (exp << (precision - 1)) | mantissa)
}

fn numeric_class(value: &Value) -> Option<bool> {
    match value {
        Value::Integer(_) | Value::Decimal(_) | Value::Rational(_) => Some(true),
        Value::Float32(_) | Value::Float64(_) | Value::Complex32(_) | Value::Complex64(_) => {
            Some(false)
        }
        _ => None,
    }
}

/// Equality defined once for all targets: NaN differs from itself, signed zeros
/// are equal, handles and symbols compare by identity, and data compares field
/// by field. ref:DEC-portable-exact-arithmetic
pub fn equal(a: &Value, b: &Value) -> Result<bool> {
    if let (Some(x), Some(y)) = (numeric_class(a), numeric_class(b)) {
        if x != y {
            return Err("exact/inexact mixing requires an explicit conversion".into());
        }
    }
    match (a, b) {
        (
            Value::Integer(_) | Value::Decimal(_) | Value::Rational(_),
            Value::Integer(_) | Value::Decimal(_) | Value::Rational(_),
        ) => Ok(a.exact()? == b.exact()?),
        (Value::Float32(_) | Value::Float64(_), Value::Float32(_) | Value::Float64(_)) => {
            Ok(a.real64()? == b.real64()?)
        }
        (
            Value::Complex32(_) | Value::Complex64(_) | Value::Float32(_) | Value::Float64(_),
            Value::Complex32(_) | Value::Complex64(_) | Value::Float32(_) | Value::Float64(_),
        ) => Ok(a.complex64()? == b.complex64()?),
        (Value::Nullable(Some(x)), Value::Nullable(Some(y)))
        | (Value::Optional(Some(x)), Value::Optional(Some(y)))
        | (Value::Maybe(Some(x)), Value::Maybe(Some(y)))
        | (Value::Left(x), Value::Left(y))
        | (Value::Right(x), Value::Right(y)) => equal(x, y),
        (Value::Data(left_tag, _), Value::Data(right_tag, _)) if left_tag != right_tag => Ok(false),
        (Value::List(xs), Value::List(ys)) | (Value::Data(_, xs), Value::Data(_, ys)) => {
            if xs.len() != ys.len() {
                return Ok(false);
            }
            for (x, y) in xs.iter().zip(ys) {
                if !equal(x, y)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(a == b),
    }
}

/// `domain` is the compiler-resolved arithmetic evidence, never inferred here.
///
/// The operator has LawSpec's semantics rather than Rust's, so a law computes
/// the same result on every target. ref:DEC-portable-exact-arithmetic
/// ref:ieee-754
pub fn binary(op: &str, domain: &str, a: Value, b: Value) -> Result<Value> {
    use Value::*;
    if op == "==" || op == "!=" {
        return Ok(Bool(equal(&a, &b)? == (op == "==")));
    }
    let exact_domain = !matches!(domain, "Float32" | "Float64" | "Complex64" | "Complex128");
    if numeric_class(&a) != Some(exact_domain) || numeric_class(&b) != Some(exact_domain) {
        return Err("numeric operands must match the resolved arithmetic domain; use an explicit conversion".into());
    }
    macro_rules! calculate {
        ($x:expr,$y:expr,$variant:ident) => {{
            let x = $x;
            let y = $y;
            match op {
                "+" => Ok($variant(x + y)),
                "-" => Ok($variant(x - y)),
                "*" => Ok($variant(x * y)),
                "/" => Ok($variant(x / y)),
                "<" => Ok(Bool(x < y)),
                "<=" => Ok(Bool(x <= y)),
                ">" => Ok(Bool(x > y)),
                ">=" => Ok(Bool(x >= y)),
                _ => Err("invalid arithmetic operation".into()),
            }
        }};
    }
    if domain == "Float32" {
        return calculate!(
            f32::from_value(a.convert("Float32", 64)?)?,
            f32::from_value(b.convert("Float32", 64)?)?,
            Float32
        );
    }
    if domain == "Float64" {
        return calculate!(a.real64()?, b.real64()?, Float64);
    }
    macro_rules! complex {
        ($x:expr,$y:expr,$variant:ident) => {{
            let x = $x;
            let y = $y;
            match op {
                "+" => Ok($variant(x + y)),
                "-" => Ok($variant(x - y)),
                "*" => Ok($variant(x * y)),
                "/" => {
                    // Match portable componentwise IEEE operations at declared precision.
                    let d = y.re * y.re + y.im * y.im;
                    Ok($variant(num_complex::Complex::new(
                        (x.re * y.re + x.im * y.im) / d,
                        (x.im * y.re - x.re * y.im) / d,
                    )))
                }
                _ => Err("complex values are not ordered".into()),
            }
        }};
    }
    if domain == "Complex64" {
        return complex!(
            num_complex::Complex32::from_value(a.convert("Complex64", 64)?)?,
            num_complex::Complex32::from_value(b.convert("Complex64", 64)?)?,
            Complex32
        );
    }
    if domain == "Complex128" {
        return complex!(a.complex64()?, b.complex64()?, Complex64);
    }
    let x = a.exact()?;
    let y = b.exact()?;
    if matches!(op, "/" | "quot" | "rem") && y.is_zero() {
        return Err("exact division by zero".into());
    }
    let result = match op {
        "+" => x + y,
        "-" => x - y,
        "*" => x * y,
        "/" => x / y,
        "quot" => return Ok(Integer(a.integer()? / b.integer()?)),
        "rem" => return Ok(Integer(a.integer()? % b.integer()?)),
        "pow" => {
            let exponent = b.integer()?;
            if exponent.is_negative() {
                return Err("negative exponent".into());
            }
            let exponent = exponent.to_u32().ok_or_else(|| "exponent too large".to_string())?;
            return Ok(Integer(num_traits::Pow::pow(a.integer()?, exponent)));
        }
        "<" => return Ok(Bool(x < y)),
        "<=" => return Ok(Bool(x <= y)),
        ">" => return Ok(Bool(x > y)),
        ">=" => return Ok(Bool(x >= y)),
        _ => return Err("invalid arithmetic operation".into()),
    };
    Rational(result).convert(domain, 64)
}

pub fn negate(value: Value) -> Result<Value> {
    Ok(match value {
        Value::Integer(x) => Value::Integer(-x),
        Value::Rational(x) => Value::Rational(-x),
        Value::Decimal(x) => Value::Decimal(Decimal::new(-x.coefficient, x.exponent)),
        Value::Float32(x) => Value::Float32(-x),
        Value::Float64(x) => Value::Float64(-x),
        Value::Complex32(x) => Value::Complex32(-x),
        Value::Complex64(x) => Value::Complex64(-x),
        _ => return Err("negation requires numeric operand".into()),
    })
}

pub trait IntoValue {
    fn into_value(self) -> Value;
}
pub trait FromValue: Sized {
    fn from_value(value: Value) -> Result<Self>;
}
macro_rules! native {
    ($t:ty,$variant:ident) => {
        impl IntoValue for $t {
            fn into_value(self) -> Value {
                Value::$variant(self)
            }
        }
        impl FromValue for $t {
            fn from_value(value: Value) -> Result<Self> {
                if let Value::$variant(x) = value {
                    Ok(x)
                } else {
                    Err(concat!("expected ", stringify!($t)).into())
                }
            }
        }
    };
}
native!(BigInt, Integer);
native!(Decimal, Decimal);
native!(BigRational, Rational);
native!(f32, Float32);
native!(f64, Float64);
native!(Complex32, Complex32);
native!(Complex64, Complex64);
native!(bool, Bool);
native!(char, Char);
native!(String, Text);

native!(Symbol, Symbol);
native!(Handle, Handle);
impl IntoValue for Integer {
    fn into_value(self) -> Value {
        Value::Integer(self.0)
    }
}
impl FromValue for Integer {
    fn from_value(value: Value) -> Result<Self> {
        Ok(Self(value.integer()?))
    }
}
impl IntoValue for BigUint {
    fn into_value(self) -> Value {
        Value::Integer(self.into())
    }
}
impl FromValue for BigUint {
    fn from_value(value: Value) -> Result<Self> {
        value
            .integer()?
            .to_biguint()
            .ok_or_else(|| "BigUInt cannot be negative".into())
    }
}
macro_rules! primitive_int {
    ($($t:ty),*) => {$ (
        impl IntoValue for $t { fn into_value(self)->Value { Value::Integer(BigInt::from(self)) } }
        impl FromValue for $t { fn from_value(value:Value)->Result<Self> {
            Self::try_from(value.integer()?).map_err(|_|concat!("integer outside ",stringify!($t)," range").into())
        } }
    )*};
}
primitive_int!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);
macro_rules! raw {
    ($t:ident,$variant:ident,$valid:expr) => {
        impl IntoValue for $t {
            fn into_value(self) -> Value {
                Value::$variant(self.0)
            }
        }
        impl FromValue for $t {
            fn from_value(value: Value) -> Result<Self> {
                match value {
                    Value::$variant(x) if ($valid)(&x) => Ok(Self(x)),
                    _ => Err(concat!("invalid ", stringify!($t)).into()),
                }
            }
        }
    };
}
raw!(CodePoint, CodePoint, |x: &u32| *x <= 0x10ffff);
raw!(CodePointText, CodePointText, |x: &Vec<u32>| x
    .iter()
    .all(|c| *c <= 0x10ffff));
raw!(Utf16Text, Utf16Text, |_: &Vec<u16>| true);
macro_rules! absence {
    ($t:ty,$variant:ident,$value:expr) => {
        impl IntoValue for $t {
            fn into_value(self) -> Value {
                Value::$variant
            }
        }
        impl FromValue for $t {
            fn from_value(value: Value) -> Result<Self> {
                if matches!(value, Value::$variant) {
                    Ok($value)
                } else {
                    Err(concat!("expected ", stringify!($t)).into())
                }
            }
        }
    };
}
absence!((), Unit, ());
absence!(Null, Null, Null);
absence!(Undefined, Undefined, Undefined);
macro_rules! presence {
    ($t:ident,$absent:ident) => {
        impl<T: IntoValue> IntoValue for $t<T> {
            fn into_value(self) -> Value {
                Value::$t(match self {
                    Self::$absent => None,
                    Self::Present(x) => Some(Box::new(x.into_value())),
                })
            }
        }
        impl<T: FromValue> FromValue for $t<T> {
            fn from_value(value: Value) -> Result<Self> {
                match value {
                    Value::$t(None) => Ok(Self::$absent),
                    Value::$t(Some(x)) => Ok(Self::Present(T::from_value(*x)?)),
                    _ => Err("presence type mismatch".into()),
                }
            }
        }
    };
}
presence!(Nullable, Null);
presence!(Optional, Undefined);

/// Algebraic Either is independent of interoperability absence wrappers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Either<L, R> {
    Left(L),
    Right(R),
}
impl<T: IntoValue> IntoValue for Box<T> {
    fn into_value(self) -> Value {
        (*self).into_value()
    }
}
impl<T: FromValue> FromValue for Box<T> {
    fn from_value(value: Value) -> Result<Self> {
        T::from_value(value).map(Box::new)
    }
}
impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        Value::List(self.into_iter().map(IntoValue::into_value).collect())
    }
}
impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: Value) -> Result<Self> {
        let values = match value {
            Value::List(values) => values,
            // A Stack, whose top is last natively.
            value @ Value::Data(..) => collection_items(value)?,
            Value::Bytes(values) => values
                .into_iter()
                .map(|v| Value::Integer(v.into()))
                .collect(),
            _ => return Err("expected List or Bytes".into()),
        };
        values.into_iter().map(T::from_value).collect()
    }
}
impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        Value::Maybe(self.map(|x| Box::new(x.into_value())))
    }
}
impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Maybe(value) => value.map(|x| T::from_value(*x)).transpose(),
            _ => Err("expected Maybe".into()),
        }
    }
}
impl<L: IntoValue, R: IntoValue> IntoValue for Either<L, R> {
    fn into_value(self) -> Value {
        match self {
            Self::Left(x) => Value::Left(Box::new(x.into_value())),
            Self::Right(x) => Value::Right(Box::new(x.into_value())),
        }
    }
}
impl<L: FromValue, R: FromValue> FromValue for Either<L, R> {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Left(x) => Ok(Self::Left(L::from_value(*x)?)),
            Value::Right(x) => Ok(Self::Right(R::from_value(*x)?)),
            _ => Err("expected Either".into()),
        }
    }
}

// A GADT case's existential fields hold checked dynamic values.
impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}
impl FromValue for Value {
    fn from_value(value: Value) -> Result<Self> {
        Ok(value)
    }
}

/// A fully applied type, or a parameter in a constructor's field schema.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TypeRef {
    Named(&'static str, Vec<TypeRef>),
    Parameter(usize),
}

impl TypeRef {
    pub fn named(name: &'static str, arguments: Vec<TypeRef>) -> Self {
        Self::Named(name, arguments)
    }

    /// Expression runtime spelling, after schema type arguments are substituted.
    pub fn expression_key(&self) -> Result<String> {
        match self {
            Self::Parameter(index) => Err(format!("unbound expression type parameter {index}")),
            Self::Named(name, arguments) => {
                let mut parts = vec![(*name).to_owned()];
                for argument in arguments {
                    parts.push(argument.expression_key()?);
                }
                Ok(parts.join(" "))
            }
        }
    }

    pub fn instantiate(&self, arguments: &[TypeRef]) -> Result<Self> {
        match self {
            Self::Parameter(index) => arguments
                .get(*index)
                .cloned()
                .ok_or_else(|| format!("unbound data type parameter {index}")),
            Self::Named(name, fields) => Ok(Self::Named(
                name,
                fields
                    .iter()
                    .map(|field| field.instantiate(arguments))
                    .collect::<Result<_>>()?,
            )),
        }
    }
}

/// Substitute known types, leaving open parameters in place.
fn substitute_open(ty: &TypeRef, known: &[TypeRef]) -> Result<TypeRef> {
    Ok(match ty {
        TypeRef::Parameter(index) => known.get(*index).cloned().unwrap_or(TypeRef::Parameter(*index)),
        TypeRef::Named(name, arguments) => TypeRef::Named(
            name,
            arguments.iter().map(|argument| substitute_open(argument, known)).collect::<Result<_>>()?,
        ),
    })
}

#[derive(Clone, Debug)]
pub struct ConstructorSchema {
    pub tag: &'static str,
    pub fields: Vec<TypeRef>,
}

#[derive(Clone, Debug)]
pub struct DataSchema {
    pub name: &'static str,
    pub parameters: usize,
    pub constructors: Vec<ConstructorSchema>,
}

/// Typed constructor predicates receive instantiated type arguments and logical
/// fields. A false result rejects a candidate; an error is an evaluation failure.
pub type FieldPredicate = fn(&Schema, &[TypeRef], &[Value], u32, &mut Context) -> Result<bool>;

#[derive(Clone, Debug)]
pub struct ConstructorContract {
    pub tag: &'static str,
    pub predicates: Vec<FieldPredicate>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueCheck {
    Accepted(Value),
    Rejected(String),
}

#[derive(Debug)]
enum ValidationFailure {
    Rejected(String),
    Error(String),
}

impl ValidationFailure {
    fn context(self, prefix: String) -> Self {
        match self {
            Self::Rejected(message) => Self::Rejected(format!("{prefix}: {message}")),
            Self::Error(message) => Self::Error(format!("{prefix}: {message}")),
        }
    }

    fn message(self) -> String {
        match self {
            Self::Rejected(message) | Self::Error(message) => message,
        }
    }
}

impl From<String> for ValidationFailure {
    fn from(message: String) -> Self {
        Self::Error(message)
    }
}

/// Compiler-provided data definitions. Validation is framework-independent and
/// follows instantiated field types, rather than guessing from Rust payloads.
#[derive(Clone, Debug)]
pub struct Schema {
    definitions: HashMap<&'static str, DataSchema>,
    contracts: HashMap<&'static str, Vec<FieldPredicate>>,
    // Indexed families: per constructor, index terms then guards, in prefix
    // notation over field indices. Validation checks the guards.
    indices: HashMap<&'static str, &'static [&'static str]>,
    // GADT constructors: refinements fixing parameters to patterns, and the
    // number of existentials (parameters numbered after the definition's own).
    refinements: HashMap<&'static str, (Vec<(usize, TypeRef)>, usize)>,
    // Existentials only a value determines (parameter numbers), by tag; their
    // types travel as the trailing Text witness fields.
    witnesses: HashMap<&'static str, &'static [usize]>,
    // Handle types: their values are handles, passed along unopened.
    handles: std::collections::HashSet<&'static str>,
}

/// Types a generator may choose for an existential that only a value fixes.
pub fn witness_pool() -> Vec<TypeRef> {
    vec![TypeRef::Named("Bool", vec![]), TypeRef::Named("Int32", vec![])]
}

/// A witness spells a type as its name, or a parenthesized application.
pub fn witness_key(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Named(name, arguments) if arguments.is_empty() => (*name).to_owned(),
        TypeRef::Named(name, arguments) => {
            let mut parts = vec![(*name).to_owned()];
            parts.extend(arguments.iter().map(witness_key));
            format!("({})", parts.join(" "))
        }
        TypeRef::Parameter(index) => format!("?{index}"),
    }
}

/// Reads a witness key back into a type reference. Type names are interned:
/// a name the schema does not know is rejected by the caller's type check.
pub fn parse_witness(text: &str) -> Result<TypeRef> {
    let spaced = text.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    fn read(tokens: &[&str], at: &mut usize) -> Result<TypeRef> {
        let token = *tokens.get(*at).ok_or("malformed type witness")?;
        *at += 1;
        if token != "(" {
            return Ok(TypeRef::Named(intern(token), vec![]));
        }
        let name = *tokens.get(*at).ok_or("malformed type witness")?;
        *at += 1;
        let mut arguments = Vec::new();
        while *tokens.get(*at).ok_or("malformed type witness")? != ")" {
            arguments.push(read(tokens, at)?);
        }
        *at += 1;
        Ok(TypeRef::Named(intern(name), arguments))
    }
    let mut at = 0;
    let result = read(&tokens, &mut at)?;
    if at != tokens.len() {
        return Err("malformed type witness".into());
    }
    Ok(result)
}

/// Witness type names are few; leaking each distinct name once is bounded.
fn intern(name: &str) -> &'static str {
    use std::sync::{Mutex, OnceLock};
    static NAMES: OnceLock<Mutex<std::collections::HashSet<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(|| Mutex::new(std::collections::HashSet::new())).lock().unwrap();
    if let Some(found) = names.get(name) {
        return found;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.insert(leaked);
    leaked
}

/// A GADT constructor's refinements and existential count, by tag.
pub type Refinements = Vec<(&'static str, Vec<(usize, TypeRef)>, usize)>;

/// A prefix index term: `c<n>`, `f<field>[.<index>]` or an operator.
enum SchemaIndexTerm {
    Constant(BigInt),
    Field(usize, usize),
    Apply(&'static str, Box<SchemaIndexTerm>, Box<SchemaIndexTerm>),
}

fn parse_schema_index(tokens: &[&'static str], at: &mut usize) -> Result<SchemaIndexTerm> {
    let token = *tokens.get(*at).ok_or("malformed index term")?;
    *at += 1;
    if let Some(digits) = token.strip_prefix('c') {
        return Ok(SchemaIndexTerm::Constant(
            digits.parse().map_err(|_| "malformed index term".to_string())?,
        ));
    }
    if let Some(reference) = token.strip_prefix('f') {
        let mut parts = reference.splitn(2, '.');
        let position = parts.next().unwrap_or("").parse().map_err(|_| "malformed index term".to_string())?;
        let index = match parts.next() {
            Some(index) => index.parse().map_err(|_| "malformed index term".to_string())?,
            None => 0,
        };
        return Ok(SchemaIndexTerm::Field(position, index));
    }
    let left = parse_schema_index(tokens, at)?;
    let right = parse_schema_index(tokens, at)?;
    Ok(SchemaIndexTerm::Apply(token, Box::new(left), Box::new(right)))
}

/// Natural index arithmetic; `None` when an operation has no natural value.
fn eval_schema_index(
    term: &SchemaIndexTerm,
    field: &mut dyn FnMut(usize, usize) -> Result<BigInt>,
) -> Result<Option<BigInt>> {
    Ok(match term {
        SchemaIndexTerm::Constant(value) => Some(value.clone()),
        SchemaIndexTerm::Field(position, index) => Some(field(*position, *index)?),
        SchemaIndexTerm::Apply(op, left, right) => {
            let (Some(x), Some(y)) = (eval_schema_index(left, field)?, eval_schema_index(right, field)?) else {
                return Ok(None);
            };
            match *op {
                "+" => Some(x + y),
                "-" => (x >= y).then(|| x - y),
                "*" => Some(x * y),
                "div" => (y.is_positive()).then(|| x / y),
                "mod" => (y.is_positive()).then(|| x % y),
                "^" => y.to_u32().filter(|e| *e <= 64).map(|e| num_traits::Pow::pow(x, e)),
                _ => return Err("malformed index term".into()),
            }
        }
    })
}

fn is_index_guard(text: &str) -> bool {
    text.starts_with("== ") || text.starts_with(">= ")
}

fn builtin_arity(name: &str) -> Option<usize> {
    match name {
        "List" | "Maybe" | "Nullable" | "Optional" => Some(1),
        "Either" => Some(2),
        "Bool" | "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32"
        | "UInt64" | "IntSize" | "UIntSize" | "UIntPtr" | "Integer" | "BigInt" | "BigUInt"
        | "Decimal" | "Rational" | "Float32" | "Float64" | "Complex64" | "Complex128" | "Char"
        | "CodePoint" | "CodeUnit16" | "Text" | "CodePointText" | "Utf16Text" | "Bytes"
        | "Symbol" | "Unit" | "Null" | "Undefined" => Some(0),
        _ => None,
    }
}

// Recipes describe parameter positions rather than instantiated field types.
#[derive(Clone, Debug)]
enum PayloadPlan {
    Ignore,
    Parameter(usize),
    Applied(&'static str, Vec<PayloadPlan>),
}

impl PayloadPlan {
    fn field(ty: &TypeRef, arguments: &[Self]) -> Result<Self> {
        match ty {
            TypeRef::Parameter(index) => arguments
                .get(*index)
                .cloned()
                .ok_or_else(|| "unbound payload parameter".into()),
            TypeRef::Named(name, fields) => {
                let children = fields
                    .iter()
                    .map(|field| Self::field(field, arguments))
                    .collect::<Result<Vec<_>>>()?;
                Ok(
                    if children.iter().all(|child| matches!(child, Self::Ignore)) {
                        Self::Ignore
                    } else {
                        Self::Applied(name, children)
                    },
                )
            }
        }
    }
}

impl Schema {
    pub fn has_contracts(&self) -> bool {
        self.contracts
            .values()
            .any(|predicates| !predicates.is_empty())
    }

    pub fn new(definitions: Vec<DataSchema>) -> Result<Self> {
        Self::with_contracts(definitions, vec![])
    }

    /// Attach indexed families' index terms and guards, keyed by constructor.
    pub fn with_indices(mut self, indices: &[(&'static str, &'static [&'static str])]) -> Self {
        self.indices.extend(indices.iter().copied());
        self
    }

    /// Mark handle types, whose values are handles rather than data.
    pub fn with_handles(mut self, handles: &[&'static str]) -> Self {
        for name in handles {
            if self.definitions.get(name).is_some_and(|d| d.constructors.is_empty()) {
                self.handles.insert(*name);
            }
        }
        self
    }

    /// Attach constructors' witnessed existentials (parameter numbers).
    pub fn with_witnesses(mut self, witnesses: &[(&'static str, &'static [usize])]) -> Self {
        self.witnesses.extend(witnesses.iter().copied());
        self
    }

    /// Field types with witnessed existentials read from the trailing witness
    /// fields of a value.
    pub fn witnessed_fields(&self, tag: &str, fields: &[TypeRef], values: &[Value]) -> Result<Vec<TypeRef>> {
        let Some(witnesses) = self.witnesses.get(tag) else {
            return Ok(fields.to_vec());
        };
        if values.len() < witnesses.len() {
            return Err("missing type witness".into());
        }
        let mut keys = Vec::new();
        for value in &values[values.len() - witnesses.len()..] {
            let Value::Text(text) = value else {
                return Err("type witness must be text".into());
            };
            keys.push(text.clone());
        }
        self.fields_at_witnesses(tag, fields, &keys)
    }

    /// Field types at the given witness keys; generated codecs use this too.
    pub fn fields_at_witnesses(&self, tag: &str, fields: &[TypeRef], keys: &[String]) -> Result<Vec<TypeRef>> {
        let witnesses = self.witnesses.get(tag).copied().unwrap_or(&[]);
        let size = witnesses.iter().copied().max().map_or(0, |m| m + 1);
        let mut known: Vec<TypeRef> = (0..size).map(TypeRef::Parameter).collect();
        for (index, key) in witnesses.iter().zip(keys) {
            let ty = parse_witness(key)?;
            self.check_type(&ty, 0)?;
            known[*index] = ty;
        }
        fields.iter().map(|field| substitute_open(field, &known)).collect()
    }

    /// Each choice of witnesses from the pool: the declared fields at it and
    /// the witness values.
    pub fn witness_choices(&self, constructor: &ConstructorSchema) -> Result<Vec<(Vec<TypeRef>, Vec<Value>)>> {
        let Some(witnesses) = self.witnesses.get(constructor.tag) else {
            return Ok(vec![(constructor.fields.clone(), vec![])]);
        };
        let declared = &constructor.fields[..constructor.fields.len() - witnesses.len()];
        let mut choices: Vec<Vec<String>> = vec![vec![]];
        for _ in 0..witnesses.len() {
            choices = choices
                .into_iter()
                .flat_map(|choice| {
                    witness_pool().into_iter().map(move |ty| {
                        let mut next = choice.clone();
                        next.push(witness_key(&ty));
                        next
                    })
                })
                .collect();
        }
        choices
            .into_iter()
            .map(|keys| {
                let fields = self.fields_at_witnesses(constructor.tag, declared, &keys)?;
                Ok((fields, keys.into_iter().map(Value::Text).collect()))
            })
            .collect()
    }

    /// The index of a checked value, computed from its constructor's term.
    fn index_of(&self, ty: &TypeRef, value: &Value, index: usize) -> Result<BigInt> {
        let (TypeRef::Named(name, arguments), Value::Data(tag, fields)) = (ty, value) else {
            return Err("no index for this value".into());
        };
        let definition = self.definitions.get(name).ok_or("no index for this type")?;
        let constructor = definition
            .constructors
            .iter()
            .find(|constructor| constructor.tag == tag)
            .ok_or("unknown constructor")?;
        let texts = self.indices.get(constructor.tag).copied().unwrap_or(&[]);
        let text = texts
            .iter()
            .filter(|text| !is_index_guard(text))
            .nth(index)
            .ok_or_else(|| format!("no index for {name}"))?;
        let tokens: Vec<&'static str> = text.split(' ').collect();
        let term = parse_schema_index(&tokens, &mut 0)?;
        let mut field = |position: usize, child: usize| -> Result<BigInt> {
            let field_type = constructor.fields.get(position).ok_or("malformed index term")?.instantiate(arguments)?;
            self.index_of(&field_type, &fields[position], child)
        };
        eval_schema_index(&term, &mut field)?.ok_or_else(|| format!("index of {tag} has no natural value"))
    }

    pub fn with_contracts(
        definitions: Vec<DataSchema>,
        contracts: Vec<ConstructorContract>,
    ) -> Result<Self> {
        Self::with_refinements(definitions, contracts, vec![])
    }

    pub fn with_refinements(
        definitions: Vec<DataSchema>,
        contracts: Vec<ConstructorContract>,
        refinements: Refinements,
    ) -> Result<Self> {
        let mut types = HashMap::new();
        let mut tags = std::collections::HashSet::new();
        for definition in definitions {
            let name = definition.name;
            if builtin_arity(name).is_some() || types.contains_key(name) {
                return Err(format!("duplicate or reserved data type: {name}"));
            }
            for constructor in &definition.constructors {
                if !tags.insert(constructor.tag) {
                    return Err(format!("duplicate constructor: {}", constructor.tag));
                }
            }
            types.insert(name, definition);
        }
        let mut predicates = HashMap::new();
        for contract in contracts {
            if !tags.contains(contract.tag) {
                return Err(format!("unknown constructor contract: {}", contract.tag));
            }
            if predicates
                .insert(contract.tag, contract.predicates)
                .is_some()
            {
                return Err(format!("duplicate constructor contract: {}", contract.tag));
            }
        }
        let schema = Self {
            definitions: types,
            contracts: predicates,
            indices: HashMap::new(),
            witnesses: HashMap::new(),
            handles: std::collections::HashSet::new(),
            refinements: refinements
                .into_iter()
                .map(|(tag, patterns, existentials)| (tag, (patterns, existentials)))
                .collect(),
        };
        for definition in schema.definitions.values() {
            for constructor in &definition.constructors {
                let existentials = schema.refinements.get(constructor.tag).map_or(0, |(_, count)| *count);
                for field in &constructor.fields {
                    schema.check_type(field, definition.parameters + existentials)?;
                }
            }
        }
        Ok(schema)
    }

    fn check_type(&self, ty: &TypeRef, parameters: usize) -> Result<()> {
        match ty {
            TypeRef::Parameter(index) if *index < parameters => Ok(()),
            TypeRef::Parameter(index) => Err(format!("unbound data type parameter {index}")),
            TypeRef::Named(name, arguments) => {
                let arity = builtin_arity(name)
                    .or_else(|| self.definitions.get(name).map(|d| d.parameters))
                    .ok_or_else(|| format!("unknown data type: {name}"))?;
                if arity != arguments.len() {
                    return Err(format!("expected {arity} type arguments for {name}"));
                }
                for argument in arguments {
                    self.check_type(argument, parameters)?;
                }
                Ok(())
            }
        }
    }

    /// Instantiated fields for composing native generators. None denotes a
    /// built-in type; an empty list denotes a user type with no constructors.
    pub fn constructor_fields(&self, ty: &TypeRef) -> Result<Option<Vec<ConstructorSchema>>> {
        self.check_type(ty, 0)?;
        let TypeRef::Named(name, arguments) = ty else {
            return Err("uninstantiated generator type".into());
        };
        self.definitions
            .get(name)
            .map(|definition| {
                definition
                    .constructors
                    .iter()
                    // A GADT constructor whose refinements do not match these
                    // arguments builds no value of this type.
                    .filter_map(|constructor| {
                        self.refine(constructor, arguments, definition.parameters)
                            .map(|extended| (constructor, extended))
                    })
                    .map(|(constructor, extended)| {
                        Ok(ConstructorSchema {
                            tag: constructor.tag,
                            fields: constructor
                                .fields
                                .iter()
                                .map(|field| field.instantiate(&extended))
                                .collect::<Result<_>>()?,
                        })
                    })
                    .collect::<Result<_>>()
            })
            .transpose()
    }

    /// Arguments extended with the existentials a constructor's refinements
    /// bind; None when the refinements do not match.
    fn refine(&self, constructor: &ConstructorSchema, arguments: &[TypeRef], parameters: usize) -> Option<Vec<TypeRef>> {
        let Some((patterns, existentials)) = self.refinements.get(constructor.tag) else {
            return Some(arguments.to_vec());
        };
        fn matches(
            pattern: &TypeRef,
            actual: &TypeRef,
            arguments: &[TypeRef],
            parameters: usize,
            bound: &mut HashMap<usize, TypeRef>,
        ) -> bool {
            match (pattern, actual) {
                (TypeRef::Parameter(index), _) if *index < parameters => arguments.get(*index) == Some(actual),
                (TypeRef::Parameter(index), _) => bound.entry(*index).or_insert_with(|| actual.clone()) == actual,
                (TypeRef::Named(name, children), TypeRef::Named(other, values)) => {
                    name == other
                        && children.len() == values.len()
                        && children.iter().zip(values).all(|(child, value)| matches(child, value, arguments, parameters, bound))
                }
                _ => false,
            }
        }
        let mut bound = HashMap::new();
        for (index, pattern) in patterns {
            if !matches(pattern, arguments.get(*index)?, arguments, parameters, &mut bound) {
                return None;
            }
        }
        let mut extended = arguments.to_vec();
        for k in 0..*existentials {
            // A witnessed existential stays open until a value supplies it.
            extended.push(bound.get(&(parameters + k)).cloned().unwrap_or(TypeRef::Parameter(parameters + k)));
        }
        Some(extended)
    }

    /// Validate first, then check only stored occurrences of type arguments.
    /// The dispatcher shares one mutable context without aliasing callback borrows.
    pub fn all_payloads_with_context(
        &self,
        value: Value,
        ty: &TypeRef,
        predicate_count: usize,
        bits: u32,
        context: &mut Context,
        mut predicate: impl FnMut(usize, Value, &mut Context) -> Result<Value>,
    ) -> Result<Value> {
        self.check_type(ty, 0)?;
        let TypeRef::Named(name, arguments) = ty else {
            return Err("payload predicates require an instantiated data type".into());
        };
        if !self.definitions.contains_key(name)
            && !matches!(*name, "List" | "Maybe" | "Either" | "Nullable" | "Optional")
        {
            return Err("payload predicates require a data type".into());
        }
        if arguments.len() != predicate_count {
            return Err("payload predicate arity mismatch".into());
        }
        let checked = self.validate_with_context(value, ty, bits, context)?;
        let plan = PayloadPlan::Applied(
            name,
            (0..predicate_count).map(PayloadPlan::Parameter).collect(),
        );
        self.walk_payload(&plan, checked, context, &mut predicate)
            .map(Value::Bool)
    }

    fn walk_payload(
        &self,
        plan: &PayloadPlan,
        value: Value,
        context: &mut Context,
        predicate: &mut impl FnMut(usize, Value, &mut Context) -> Result<Value>,
    ) -> Result<bool> {
        let (name, arguments) = match plan {
            PayloadPlan::Ignore => return Ok(true),
            PayloadPlan::Parameter(index) => return predicate(*index, value, context)?.boolean(),
            PayloadPlan::Applied(name, arguments) => (*name, arguments),
        };
        match (name, value) {
            ("List", Value::List(values)) => {
                for (index, value) in values.into_iter().enumerate() {
                    if !self
                        .walk_payload(&arguments[0], value, context, predicate)
                        .map_err(|error| format!("List element {index}: {error}"))?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            ("Maybe", Value::Maybe(value))
            | ("Nullable", Value::Nullable(value))
            | ("Optional", Value::Optional(value)) => match value {
                None => Ok(true),
                Some(value) => self
                    .walk_payload(&arguments[0], *value, context, predicate)
                    .map_err(|error| format!("{name} payload: {error}")),
            },
            ("Either", Value::Left(value)) => self
                .walk_payload(&arguments[0], *value, context, predicate)
                .map_err(|error| format!("Either Left payload: {error}")),
            ("Either", Value::Right(value)) => self
                .walk_payload(&arguments[1], *value, context, predicate)
                .map_err(|error| format!("Either Right payload: {error}")),
            (_, Value::Data(tag, values)) => {
                let definition = self
                    .definitions
                    .get(name)
                    .ok_or_else(|| format!("unknown payload data type: {name}"))?;
                let constructor = definition
                    .constructors
                    .iter()
                    .find(|item| item.tag == tag)
                    .ok_or_else(|| format!("unknown payload constructor: {tag}"))?;
                for (index, (field, value)) in constructor.fields.iter().zip(values).enumerate() {
                    let plan = PayloadPlan::field(field, arguments)?;
                    if !self
                        .walk_payload(&plan, value, context, predicate)
                        .map_err(|error| format!("{tag} field {index}: {error}"))?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Err("payload traversal requires a matching structural value".into()),
        }
    }

    pub fn validate(&self, value: Value, ty: &TypeRef, bits: u32) -> Result<Value> {
        self.validate_with_context(value, ty, bits, &mut Context::default())
    }

    pub fn check_with_context(
        &self,
        value: Value,
        ty: &TypeRef,
        bits: u32,
        context: &mut Context,
    ) -> Result<ValueCheck> {
        self.check_type(ty, 0)?;
        if !matches!(bits, 32 | 64) {
            return Err("machineBits must be 32 or 64".into());
        }
        match self.walk(value, ty, bits, false, context) {
            Ok(value) => Ok(ValueCheck::Accepted(value)),
            Err(ValidationFailure::Rejected(message)) => Ok(ValueCheck::Rejected(message)),
            Err(ValidationFailure::Error(message)) => Err(message),
        }
    }

    pub fn validate_with_context(
        &self,
        value: Value,
        ty: &TypeRef,
        bits: u32,
        context: &mut Context,
    ) -> Result<Value> {
        match self.check_with_context(value, ty, bits, context)? {
            ValueCheck::Accepted(value) => Ok(value),
            ValueCheck::Rejected(message) => Err(message),
        }
    }

    /// Check logical fields before adapting raw code units to native integers.
    pub fn native_value(&self, value: Value, ty: &TypeRef, bits: u32) -> Result<Value> {
        self.native_value_with_context(value, ty, bits, &mut Context::default())
    }

    pub fn native_value_with_context(
        &self,
        value: Value,
        ty: &TypeRef,
        bits: u32,
        context: &mut Context,
    ) -> Result<Value> {
        let checked = self.validate_with_context(value, ty, bits, context)?;
        self.walk(checked, ty, bits, true, context)
            .map_err(ValidationFailure::message)
    }

    fn walk(
        &self,
        value: Value,
        ty: &TypeRef,
        bits: u32,
        native: bool,
        context: &mut Context,
    ) -> std::result::Result<Value, ValidationFailure> {
        let TypeRef::Named(name, arguments) = ty else {
            return Err("uninstantiated data type parameter".to_string().into());
        };
        // A native collection arrives as a plain list: retag it, put a Set
        // or KeyVal in canonical order, and a Stack's top first.
        let value = match (collection_tag(name), value) {
            (Some(tag), Value::List(mut items)) => {
                if name.ends_with("::Set") || name.ends_with("::KeyVal") {
                    items = canonical_items(items, name.ends_with("::KeyVal"))?;
                } else if name.ends_with("::Stack") {
                    items.reverse();
                }
                Value::Data(tag.into(), vec![Value::List(items)])
            }
            (_, value) => value,
        };
        if self.handles.contains(name) {
            let Value::Handle(handle) = value else {
                return Err(format!("expected a handle of type {name}").into());
            };
            handle.named(name);
            return Ok(Value::Handle(handle));
        }
        if let Some(definition) = self.definitions.get(name) {
            let Value::Data(tag, fields) = value else {
                return Err(format!("expected data value of type {name}").into());
            };
            let constructor = definition
                .constructors
                .iter()
                .find(|constructor| constructor.tag == tag)
                .ok_or_else(|| format!("constructor {tag} does not belong to {name}"))?;
            if fields.len() != constructor.fields.len() {
                return Err(format!("wrong field count for {tag}").into());
            }
            let arguments = &self
                .refine(constructor, arguments, definition.parameters)
                .ok_or_else(|| format!("constructor {tag} is not a value of {name}"))?;
            let instantiated = constructor
                .fields
                .iter()
                .map(|field| field.instantiate(arguments))
                .collect::<Result<Vec<_>>>()?;
            let field_types = self.witnessed_fields(&tag, &instantiated, &fields)?;
            let checked: Vec<Value> = fields
                .into_iter()
                .zip(&field_types)
                .enumerate()
                .map(|(index, (field, field_type))| {
                    self.walk(
                        field,
                        field_type,
                        bits,
                        native,
                        context,
                    )
                    .map_err(|error| error.context(format!("{tag} field {index}")))
                })
                .collect::<std::result::Result<_, ValidationFailure>>()?;
            if !native {
                if let Some(predicates) = self.contracts.get(constructor.tag) {
                    for (index, predicate) in predicates.iter().enumerate() {
                        let label = format!("{tag}: field refinement {}", index + 1);
                        let accepted = predicate(self, arguments, &checked, bits, context)
                            .map_err(|message| {
                                ValidationFailure::Error(format!("{label}: {message}"))
                            })?;
                        if !accepted {
                            return Err(ValidationFailure::Rejected(format!("{label} failed")));
                        }
                    }
                }
                for text in self.indices.get(constructor.tag).copied().unwrap_or(&[]) {
                    if !is_index_guard(text) {
                        continue;
                    }
                    let tokens: Vec<&'static str> = text.split(' ').collect();
                    let mut at = 1;
                    let left = parse_schema_index(&tokens, &mut at)?;
                    let right = parse_schema_index(&tokens, &mut at)?;
                    let mut field = |position: usize, child: usize| -> Result<BigInt> {
                        let field_type = constructor.fields.get(position).ok_or("malformed index term")?.instantiate(arguments)?;
                        self.index_of(&field_type, &checked[position], child)
                    };
                    let x = eval_schema_index(&left, &mut field)?;
                    let y = eval_schema_index(&right, &mut field)?;
                    let holds = match (x, y) {
                        (Some(x), Some(y)) => if tokens[0] == "==" { x == y } else { x >= y },
                        _ => false,
                    };
                    if !holds {
                        return Err(ValidationFailure::Rejected(format!("{tag}: index guard {text} failed")));
                    }
                }
            }
            return Ok(Value::Data(tag, checked));
        }
        if arguments.is_empty() {
            let checked = validate(value, name, bits)?;
            return if native {
                native_value(checked, name).map_err(ValidationFailure::Error)
            } else {
                Ok(checked)
            };
        }
        match (*name, arguments.as_slice(), value) {
            ("List", [element], Value::List(values)) => Ok(Value::List(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        self.walk(value, element, bits, native, context)
                            .map_err(|error| error.context(format!("List element {index}")))
                    })
                    .collect::<std::result::Result<_, ValidationFailure>>()?,
            )),
            ("Maybe", [element], Value::Maybe(value)) => Ok(Value::Maybe(
                value
                    .map(|value| {
                        self.walk(*value, element, bits, native, context)
                            .map(Box::new)
                    })
                    .transpose()?,
            )),
            ("Nullable", [element], Value::Nullable(value)) => Ok(Value::Nullable(
                value
                    .map(|value| {
                        self.walk(*value, element, bits, native, context)
                            .map(Box::new)
                    })
                    .transpose()?,
            )),
            ("Optional", [element], Value::Optional(value)) => Ok(Value::Optional(
                value
                    .map(|value| {
                        self.walk(*value, element, bits, native, context)
                            .map(Box::new)
                    })
                    .transpose()?,
            )),
            ("Either", [left, _], Value::Left(value)) => Ok(Value::Left(Box::new(
                self.walk(*value, left, bits, native, context)?,
            ))),
            ("Either", [_, right], Value::Right(value)) => Ok(Value::Right(Box::new(
                self.walk(*value, right, bits, native, context)?,
            ))),
            _ => Err(format!("invalid value for {name}").into()),
        }
    }
}

/// Build a user constructor selected and typed by the compiler. This does not
/// validate its payload; generated bridges check each field at native boundaries.
pub fn construct_data(tag: &str, fields: Vec<Value>) -> Value {
    Value::Data(tag.into(), fields)
}

/// Extract exactly one constructor's fields before typed field conversion.
/// Tag and arity checks prevent cross-constructor coercion and dropped fields.
pub fn data_fields(value: Value, tag: &str, arity: usize) -> Result<Vec<Value>> {
    match value {
        Value::Data(actual, fields) if actual == tag && fields.len() == arity => Ok(fields),
        Value::Data(actual, fields) => Err(format!(
            "expected {tag} with {arity} fields, found {actual} with {} fields",
            fields.len()
        )),
        _ => Err(format!("expected data constructor {tag}")),
    }
}

pub fn all_elements(
    value: Value,
    mut predicate: impl FnMut(Value) -> Result<Value>,
) -> Result<Value> {
    let Value::List(values) = value else {
        return Err("expected List in element predicate".into());
    };
    for (index, item) in values.into_iter().enumerate() {
        let accepted = predicate(item)
            .and_then(|result| result.boolean())
            .map_err(|error| format!("List element {index}: {error}"))?;
        if !accepted {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

pub fn construct(tag: &str, mut fields: Vec<Value>) -> Result<Value> {
    Ok(match (tag, fields.len()) {
        ("List::Nil", 0) => Value::List(vec![]),
        ("List::Cons", 2) => {
            let Value::List(mut tail) = fields.pop().unwrap() else {
                return Err("expected List tail".into());
            };
            tail.insert(0, fields.pop().unwrap());
            Value::List(tail)
        }
        ("Maybe::Nothing", 0) => Value::Maybe(None),
        ("Maybe::Just", 1) => Value::Maybe(Some(Box::new(fields.pop().unwrap()))),
        ("Either::Left", 1) => Value::Left(Box::new(fields.pop().unwrap())),
        ("Either::Right", 1) => Value::Right(Box::new(fields.pop().unwrap())),
        _ => return Err(format!("invalid constructor or arity: {tag}")),
    })
}

// Read one saturated type from the compiler's prefix encoding. Both arguments
// of Either may themselves contain binary and unary type constructors.
fn split_type(input: &str) -> Result<(&str, &str)> {
    let (head, mut remaining) = input.split_once(' ').unwrap_or((input, ""));
    if head.is_empty() {
        return Err("missing type argument".into());
    }
    let arity = match head {
        "Either" => 2,
        "List" | "Maybe" | "Nullable" | "Optional" => 1,
        _ => 0,
    };
    for _ in 0..arity {
        remaining = split_type(remaining)?.1;
    }
    Ok((input[..input.len() - remaining.len()].trim_end(), remaining))
}
pub fn either_types(arguments: &str) -> Result<(&str, &str)> {
    let (left, rest) = split_type(arguments)?;
    let (right, extra) = split_type(rest)?;
    if !extra.is_empty() {
        return Err("extra Either type arguments".into());
    }
    Ok((left, right))
}

pub fn require_architecture(machine_bits: u32) -> Result<()> {
    if usize::BITS == machine_bits {
        Ok(())
    } else {
        Err(format!(
            "machineBits does not match native architecture: profile {machine_bits}, native {}",
            usize::BITS
        ))
    }
}

/// Run an async adapter's future to completion on this thread. A waker that
/// unparks the thread is enough: adapters own whatever executor they need.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => return value,
            std::task::Poll::Pending => std::thread::park(),
        }
    }
}

/// The portable total order. Exact numbers by value, text by code point,
/// sequences by unit, false before true, absence before presence, lists
/// element by element, Nothing before Just, Left before Right, and other data
/// by constructor identity, then fields left to right.
pub fn compare_values(a: &Value, b: &Value) -> Result<std::cmp::Ordering> {
    use std::cmp::Ordering::*;
    use Value::*;
    fn items(a: &[Value], b: &[Value]) -> Result<std::cmp::Ordering> {
        for (x, y) in a.iter().zip(b) {
            let order = compare_values(x, y)?;
            if order != Equal {
                return Ok(order);
            }
        }
        Ok(a.len().cmp(&b.len()))
    }
    fn optional(a: &Option<Box<Value>>, b: &Option<Box<Value>>) -> Result<std::cmp::Ordering> {
        match (a, b) {
            (None, None) => Ok(Equal),
            (None, Some(_)) => Ok(Less),
            (Some(_), None) => Ok(Greater),
            (Some(x), Some(y)) => compare_values(x, y),
        }
    }
    Ok(match (a, b) {
        (Bool(x), Bool(y)) => x.cmp(y),
        (Integer(_) | Decimal(_) | Rational(_), Integer(_) | Decimal(_) | Rational(_)) => a.exact()?.cmp(&b.exact()?),
        (Text(x), Text(y)) => x.chars().cmp(y.chars()),
        (Char(x), Char(y)) => x.cmp(y),
        (CodePoint(x), CodePoint(y)) => x.cmp(y),
        (CodeUnit16(x), CodeUnit16(y)) => x.cmp(y),
        (CodePointText(x), CodePointText(y)) => x.cmp(y),
        (Utf16Text(x), Utf16Text(y)) => x.cmp(y),
        (Bytes(x), Bytes(y)) => x.cmp(y),
        (Unit, Unit) | (Null, Null) | (Undefined, Undefined) => Equal,
        (Nullable(x), Nullable(y)) | (Optional(x), Optional(y)) | (Maybe(x), Maybe(y)) => optional(x, y)?,
        (Left(_), Right(_)) => Less,
        (Right(_), Left(_)) => Greater,
        (Left(x), Left(y)) | (Right(x), Right(y)) => compare_values(x, y)?,
        (List(x), List(y)) => items(x, y)?,
        (Data(s, x), Data(t, y)) if s == t => items(x, y)?,
        (Data(s, _), Data(t, _)) => s.cmp(t),
        (Handle(x), Handle(y)) if x == y => Equal,
        (Handle(_), Handle(_)) => return Err("handles have no portable order".into()),
        _ => return Err("values have no portable order".into()),
    })
}

/// A Set's items or a KeyVal's entries sorted by key, keeping the last of
/// equal keys.
pub fn canonical_items(items: Vec<Value>, keyed: bool) -> Result<Vec<Value>> {
    let key = |item: &Value| -> Value {
        match (keyed, item) {
            (true, Value::Data(_, fields)) if !fields.is_empty() => fields[0].clone(),
            _ => item.clone(),
        }
    };
    let mut ordered = items;
    let mut failure = None;
    ordered.sort_by(|a, b| match compare_values(&key(a), &key(b)) {
        Ok(order) => order,
        Err(error) => {
            failure = Some(error);
            std::cmp::Ordering::Equal
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }
    let mut result: Vec<Value> = Vec::new();
    for item in ordered {
        if let Some(last) = result.last() {
            if compare_values(&key(last), &key(&item))? == std::cmp::Ordering::Equal {
                *result.last_mut().unwrap() = item;
                continue;
            }
        }
        result.push(item);
    }
    Ok(result)
}

// Workflow runtime. A workflow runs under a runtime: a clock, a seeded
// random source, a trace of what happened, and the state of stateful stages.
// The runtime travels in the Context every generated function takes; without
// one, the default runtime applies (real time). Durations are i64
// microseconds.

/// Tells the time and waits, in microseconds.
pub trait Clock: Send {
    fn now(&self) -> i64;
    fn sleep(&mut self, micros: i64);
    /// A virtual clock's waits pass at once, and timeouts and hedges count
    /// only the time it reports (see scoped). Real time unless it says so.
    fn is_virtual(&self) -> bool {
        false
    }
}

/// Monotonic wall time.
pub struct RealClock(std::time::Instant);
impl Default for RealClock {
    fn default() -> Self {
        RealClock(std::time::Instant::now())
    }
}
impl Clock for RealClock {
    fn now(&self) -> i64 {
        self.0.elapsed().as_micros() as i64
    }
    fn sleep(&mut self, micros: i64) {
        std::thread::sleep(std::time::Duration::from_micros(micros.max(0) as u64));
    }
}

/// Advances when slept on and returns at once.
#[derive(Default)]
pub struct VirtualClock {
    pub time: i64,
}
impl Clock for VirtualClock {
    fn now(&self) -> i64 {
        self.time
    }
    fn sleep(&mut self, micros: i64) {
        self.time += micros;
    }
    fn is_virtual(&self) -> bool {
        true
    }
}

/// The Clock ability (lawspec.time's Clock) as the runtime reads it: the
/// handler a law installs, in microseconds. real_time is true only for the
/// default handler (the system clock, installed with installed_native); any
/// other handler is virtual.
pub trait ClockAbility: Send + Sync {
    fn now_micros(&self) -> i64;
    fn sleep_micros(&self, micros: i64);
    fn real_time(&self) -> bool;
}

/// The key the Clock ability's handler is installed under.
pub const CLOCK_KEY: &str = "lawspec.time::ability::Clock";

/// Turns the handler installed for the Clock ability into a ClockAbility.
/// The runtime cannot name the generated Clock trait, so lawspec.time's
/// default handlers module registers one (lawspec_time::
/// register_clock_ability, which the generated tests of a unit whose laws
/// install a Clock handler call); without it, workflow time is the
/// runtime's own.
pub type ClockAbilityReader = fn(&Installed) -> Option<Arc<dyn ClockAbility>>;

static CLOCK_READER: std::sync::OnceLock<ClockAbilityReader> = std::sync::OnceLock::new();

/// Registers how to read the Clock ability's handler; the first one stays.
pub fn register_clock_ability(reader: ClockAbilityReader) {
    let _ = CLOCK_READER.set(reader);
}

/// The Clock handler installed in ctx, as the runtime reads it, if any.
pub fn clock_ability(ctx: &Context) -> Option<Arc<dyn ClockAbility>> {
    let reader = CLOCK_READER.get()?;
    reader(&ctx.installed_for(CLOCK_KEY)?)
}

/// A workflow runtime's clock read through the Clock ability: the handler a
/// law installs (the virtual clock, or the default real one). A handler that
/// is not real time is virtual: waits pass at once, and timeouts and hedges
/// count only the time it reports.
pub struct AbilityClock(pub Arc<dyn ClockAbility>);
impl Clock for AbilityClock {
    fn now(&self) -> i64 {
        self.0.now_micros()
    }
    fn sleep(&mut self, micros: i64) {
        self.0.sleep_micros(micros)
    }
    fn is_virtual(&self) -> bool {
        !self.0.real_time()
    }
}

/// The same sequence on every target for the same seed.
///
/// SplitMix64 is small, fast and specified exactly, so every runtime implements
/// the same generator and a seed names the same case everywhere. ref:splitmix
/// ref:DEC-portable-seeded-generation
pub struct SplitMix64 {
    state: u64,
}
impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }
    pub fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, bound); 0 when bound is 0.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 { 0 } else { self.next() % bound }
    }
}

/// A stage starting or finishing an attempt, or a wait.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceEvent {
    pub kind: &'static str,
    pub stage: String,
    pub number: i64,
    pub succeeded: bool,
}

pub struct WorkflowRuntime {
    pub clock: Box<dyn Clock>,
    pub random: SplitMix64,
    pub trace: Vec<TraceEvent>,
    pub state: HashMap<String, Value>,
    /// Whether rate limits, breakers, bulkheads and caches apply.
    pub gates: bool,
    cache: HashMap<String, Vec<(Value, Value, i64)>>,
    // A frame per running workflow: the undos of its completed stages.
    frames: Vec<Vec<(&'static str, Value, fn(&mut Context, Value) -> Result<Value>)>>,
    // When the running attempt of a stage with a timeout must end.
    deadline: Option<std::time::Instant>,
    // The running attempt's stage and hedge (delay, most).
    hedge: Option<(&'static str, (i64, i64))>,
    // The same, on a virtual clock: attempts run one after another.
    virtual_hedge: Option<(&'static str, (i64, i64))>,
}
impl std::fmt::Debug for WorkflowRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WorkflowRuntime {{ trace: {:?} }}", self.trace)
    }
}
impl WorkflowRuntime {
    pub fn new(clock: Box<dyn Clock>, seed: u64) -> Self {
        WorkflowRuntime { clock, random: SplitMix64::new(seed), trace: Vec::new(), state: HashMap::new(), gates: true, cache: HashMap::new(), frames: Vec::new(), deadline: None, hedge: None, virtual_hedge: None }
    }
}

static DEFAULT_RUNTIME: std::sync::OnceLock<Arc<std::sync::Mutex<WorkflowRuntime>>> = std::sync::OnceLock::new();

// A runtime attached to the context keeps its own clock. Otherwise workflow
// time is the Clock ability's: where a law has installed a Clock handler,
// workflows wait and time out on it, under a view of the default runtime
// (its gates; its own trace and state) kept per context and handler, so
// run_stage and await_step share it.
fn workflow_runtime(ctx: &Context) -> Arc<std::sync::Mutex<WorkflowRuntime>> {
    if let Some(runtime) = &ctx.workflow {
        return runtime.clone();
    }
    let runtime = DEFAULT_RUNTIME
        .get_or_init(|| Arc::new(std::sync::Mutex::new(WorkflowRuntime::new(Box::new(RealClock::default()), 0))))
        .clone();
    let Some(installed) = ctx.installed_for(CLOCK_KEY) else { return runtime };
    let Some(reader) = CLOCK_READER.get() else { return runtime };
    let mut view = ctx.clock_view.0.lock().unwrap();
    if let Some((handler, under)) = view.as_ref() {
        if Arc::ptr_eq(handler, &installed.handler) {
            return under.clone();
        }
    }
    let Some(clock) = reader(&installed) else { return runtime };
    let mut under = WorkflowRuntime::new(Box::new(AbilityClock(clock)), 0);
    under.gates = runtime.lock().unwrap().gates;
    let under = Arc::new(std::sync::Mutex::new(under));
    *view = Some((installed.handler.clone(), under.clone()));
    under
}

// The workflow runtime a context's Clock handler gives (workflow_runtime).
#[derive(Clone, Default)]
struct ClockView(Arc<std::sync::Mutex<Option<(Arc<dyn std::any::Any + Send + Sync>, Arc<std::sync::Mutex<WorkflowRuntime>>)>>>);
impl std::fmt::Debug for ClockView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClockView")
    }
}

/// strategy is immediate, fixed, linear, exponential, fibonacci or custom;
/// delay, step, factor and cap (negative for none) are its parameters.
pub struct Retry {
    pub strategy: &'static str,
    pub delay: i64,
    pub step: i64,
    pub factor: i64,
    pub cap: i64,
    pub attempts: i64,
    pub jitter: &'static str,
    pub when: Option<fn(&mut Context, Value) -> Result<bool>>,
    pub decide: Option<fn(&mut Context, i64, Value, i64) -> Result<Option<i64>>>,
}

/// A stateful policy: start gives its state, admit a Step of the next state
/// and a Gate (Admit, WaitFor or Reject), and finish (when set) the state
/// after the call. wait is -2 to fail at once when not admitted, -1 to wait
/// without bound, or the most it waits.
pub struct Gate {
    pub kind: &'static str,
    pub start: fn(&mut Context, i64) -> Result<Value>,
    pub admit: fn(&mut Context, Value, i64) -> Result<Value>,
    pub finish: Option<fn(&mut Context, Value, i64, bool) -> Result<Value>>,
    pub wait: i64,
}

/// key names the stage's state; cache is how long a success is reused (0
/// or less for none); wraps says failures are StageFailures.
pub struct StagePolicy {
    pub stage: &'static str,
    pub retry: Option<Retry>,
    pub timeout: i64,
    pub key: &'static str,
    pub gates: Vec<Gate>,
    pub cache: i64,
    pub wraps: bool,
    /// Undoes the stage's success value when its workflow fails.
    pub compensate: Option<fn(&mut Context, Value) -> Result<Value>>,
    /// (delay, most): when an attempt has not succeeded after delay, another
    /// starts beside it, up to most in all; the first success wins.
    pub hedge: Option<(i64, i64)>,
}

/// Runs a workflow whose stages compensate: when it fails, the undos of its
/// completed stages run, last first.
pub fn run_workflow(ctx: &mut Context, mut attempt: impl FnMut(&mut Context) -> Result<Value>) -> Result<Value> {
    let runtime = workflow_runtime(ctx);
    runtime.lock().unwrap().frames.push(Vec::new());
    let result = attempt(ctx);
    let frame = runtime.lock().unwrap().frames.pop().unwrap_or_default();
    let result = result?;
    if matches!(result, Value::Left(_)) {
        for (stage, value, undo) in frame.into_iter().rev() {
            runtime.lock().unwrap().trace.push(TraceEvent { kind: "compensate", stage: stage.into(), number: 0, succeeded: true });
            undo(ctx, value)?;
        }
    }
    Ok(result)
}

const STAGE_FAILURE: &str = "lawspec.resilience::type::StageFailure::";
const GATE: &str = "lawspec.resilience::type::Gate::";

fn stage_failure(kind: &str) -> Value {
    Value::Left(Box::new(Value::Data(format!("{STAGE_FAILURE}{kind}"), vec![])))
}

fn gate_key(policy: &StagePolicy, gate: &Gate) -> String {
    format!("{}/{}", if policy.key.is_empty() { policy.stage } else { policy.key }, gate.kind)
}

/// Admits the call (None) or gives the failure to return instead.
fn pass_gate(ctx: &mut Context, runtime: &Arc<std::sync::Mutex<WorkflowRuntime>>, policy: &StagePolicy, gate: &Gate) -> Result<Option<&'static str>> {
    let key = gate_key(policy, gate);
    let failure = match gate.kind { "breaker" => "CircuitOpen", "limit" => "RateLimited", _ => "Saturated" };
    let mut waited = 0i64;
    loop {
        let (now, state) = {
            let guard = runtime.lock().unwrap();
            (guard.clock.now(), guard.state.get(&key).cloned())
        };
        let state = match state { Some(state) => state, None => (gate.start)(ctx, now)? };
        let Value::Data(_, mut step) = (gate.admit)(ctx, state, now)? else { return Err("expected a Step".into()) };
        let decision = step.pop().unwrap();
        runtime.lock().unwrap().state.insert(key.clone(), step.pop().unwrap());
        let Value::Data(tag, fields) = decision else { return Err("expected a Gate".into()) };
        if tag == format!("{GATE}Admit") {
            return Ok(None);
        }
        if tag == format!("{GATE}Reject") || gate.wait == -2 {
            return Ok(Some(failure));
        }
        let delay = match fields.first() {
            Some(Value::Integer(n)) => n.to_i64().ok_or("wait outside i64")?,
            _ => return Err("expected a wait".into()),
        };
        if gate.wait >= 0 && waited + delay > gate.wait {
            return Ok(Some(failure));
        }
        let mut guard = runtime.lock().unwrap();
        guard.trace.push(TraceEvent { kind: "wait", stage: policy.stage.into(), number: delay, succeeded: true });
        guard.clock.sleep(delay);
        waited += delay;
    }
}

fn finish_gate(ctx: &mut Context, runtime: &Arc<std::sync::Mutex<WorkflowRuntime>>, policy: &StagePolicy, gate: &Gate, succeeded: bool) -> Result<()> {
    if let Some(finish) = gate.finish {
        let key = gate_key(policy, gate);
        let (now, state) = {
            let guard = runtime.lock().unwrap();
            (guard.clock.now(), guard.state.get(&key).cloned().ok_or("missing gate state")?)
        };
        let next = finish(ctx, state, now, succeeded)?;
        runtime.lock().unwrap().state.insert(key, next);
    }
    Ok(())
}

fn fibonacci(n: i64) -> i64 {
    let (mut a, mut b) = (1i64, 1i64);
    for _ in 1..n {
        let next = a + b;
        a = b;
        b = next;
    }
    a
}

/// The delay before attempt (2 or more), before jitter.
pub fn retry_delay(retry: &Retry, attempt: i64) -> i64 {
    let n = attempt - 1;
    match retry.strategy {
        "immediate" => 0,
        "fixed" => retry.delay,
        "linear" => retry.delay + retry.step * (n - 1),
        "exponential" => {
            let mut delay = retry.delay;
            for _ in 1..n {
                delay *= retry.factor;
                if retry.cap >= 0 && delay >= retry.cap {
                    return retry.cap;
                }
            }
            if retry.cap >= 0 && delay > retry.cap { retry.cap } else { delay }
        }
        "fibonacci" => retry.delay * fibonacci(n),
        other => panic!("unknown retry strategy: {other}"),
    }
}

/// Full: [0, delay]; equal: delay/2 + [0, delay/2]; decorrelated:
/// [base, previous * 3], capped at delay.
pub fn jittered(jitter: &str, delay: i64, previous: i64, base: i64, random: &mut SplitMix64) -> i64 {
    match jitter {
        "full" => random.below((delay + 1) as u64) as i64,
        "equal" => {
            let half = delay / 2;
            half + random.below((delay - half + 1) as u64) as i64
        }
        "decorrelated" => {
            let high = base.max(previous * 3);
            delay.min(base + random.below((high - base + 1) as u64) as i64)
        }
        _ => delay,
    }
}

/// Runs a stage's attempts under its policy; a Left is a failure. key is the
/// stage's input, for the cache.
pub fn run_stage(
    ctx: &mut Context,
    policy: &StagePolicy,
    attempt: impl FnMut(&mut Context) -> Result<Value>,
    key: Value,
) -> Result<Value> {
    let runtime = workflow_runtime(ctx);
    let gating = runtime.lock().unwrap().gates;
    let cache_key = format!("{}/cache", if policy.key.is_empty() { policy.stage } else { policy.key });
    let caching = policy.cache > 0 && gating;
    if caching {
        let mut guard = runtime.lock().unwrap();
        let now = guard.clock.now();
        let hit = guard.cache.get(&cache_key).and_then(|entries| {
            entries.iter().find(|(entry, _, expires)| now < *expires && equal(entry, &key).unwrap_or(false)).map(|(_, value, _)| value.clone())
        });
        if let Some(value) = hit {
            guard.trace.push(TraceEvent { kind: "cached", stage: policy.stage.into(), number: 0, succeeded: true });
            return Ok(value);
        }
    }
    let gates: &[Gate] = if gating { &policy.gates } else { &[] };
    for (i, gate) in gates.iter().enumerate() {
        if let Some(failure) = pass_gate(ctx, &runtime, policy, gate)? {
            for passed in &gates[..i] {
                finish_gate(ctx, &runtime, policy, passed, false)?;
            }
            return Ok(stage_failure(failure));
        }
    }
    let result = attempts(ctx, &runtime, policy, attempt)?;
    let succeeded = !matches!(result, Value::Left(_));
    for gate in gates {
        finish_gate(ctx, &runtime, policy, gate, succeeded)?;
    }
    if let (Value::Right(value), Some(undo)) = (&result, policy.compensate) {
        if let Some(frame) = runtime.lock().unwrap().frames.last_mut() {
            frame.push((policy.stage, (**value).clone(), undo));
        }
    }
    if caching && succeeded {
        let mut guard = runtime.lock().unwrap();
        let expires = guard.clock.now() + policy.cache;
        let entries = guard.cache.entry(cache_key).or_default();
        entries.retain(|(entry, _, _)| !equal(entry, &key).unwrap_or(false));
        entries.push((key, result.clone(), expires));
    }
    Ok(result)
}

// The error block_on_within gives when an attempt outlives its stage's
// timeout; the stage turns it into TimedOut.
const TIMED_OUT: &str = "\0lawspec: timed out";

/// The Async ability's default handler: native threads. LawSpec code
/// performs pause; workflows reach the rest natively: spawn starts a function
/// as a task (a thread), wait gives a task's result, wait_future runs a
/// future to completion on this thread, and all runs functions side by side
/// and gives their results in order. An `async` adapter is an adapter that
/// uses Async, so its future is awaited with wait_future.
/// (lawspec.concurrent's default AsyncHandler delegates to it.)
#[derive(Clone, Copy, Debug, Default)]
pub struct NativeAsync;

/// The runtime's NativeAsync.
pub static ASYNC: NativeAsync = NativeAsync;

impl NativeAsync {
    /// Lets other threads run.
    pub fn pause(&self) {
        std::thread::yield_now();
    }

    /// Starts body as a task of its own.
    pub fn spawn<T: Send + 'static>(&self, body: impl FnOnce() -> T + Send + 'static) -> std::thread::JoinHandle<T> {
        std::thread::spawn(body)
    }

    /// The task's result; a panic in it is resumed here.
    pub fn wait<T>(&self, task: std::thread::JoinHandle<T>) -> T {
        task.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    }

    /// A future's output, polled on this thread (see block_on).
    pub fn wait_future<F: std::future::Future>(&self, future: F) -> F::Output {
        block_on(future)
    }

    /// Every function's result, in order: they run side by side, each on a
    /// scoped thread, and all finish before the first panic (in order) is
    /// resumed.
    pub fn all<'a, T: Send + 'a>(&self, bodies: Vec<Box<dyn FnOnce() -> T + Send + 'a>>) -> Vec<T> {
        let results: Vec<std::thread::Result<T>> = std::thread::scope(|scope| {
            let tasks: Vec<_> = bodies.into_iter().map(|body| scope.spawn(body)).collect();
            tasks.into_iter().map(|task| task.join()).collect()
        });
        results.into_iter().map(|result| result.unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    }
}

/// An all group's step results, in declaration order, as the runtime's
/// NativeAsync runs them: every step finishes before the first failure (or
/// panic), in declaration order, is the group's.
pub fn concurrently<'a>(steps: Vec<Box<dyn FnOnce() -> Result<Value> + Send + 'a>>) -> Result<Vec<Value>> {
    ASYNC.all(steps).into_iter().collect()
}

/// An all group's step results, joined from their scoped threads, in
/// declaration order. Every step has finished; the first failure (or panic),
/// in declaration order, is the group's.
pub fn joined(results: Vec<std::thread::Result<Result<Value>>>) -> Result<Vec<Value>> {
    let mut values = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(value) => values.push(value?),
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
    Ok(values)
}

/// An async step's logical result: start begins the step and convert turns
/// its native result into a logical value. The step runs within its stage's
/// timeout and hedge, if any, polling its attempts on this thread.
pub fn await_step<F: std::future::Future>(
    ctx: &Context,
    mut start: impl FnMut() -> F,
    convert: impl Fn(F::Output) -> Value,
) -> Result<Value> {
    let runtime = workflow_runtime(ctx);
    let (deadline, hedge, virtual_hedge) = {
        let guard = runtime.lock().unwrap();
        (guard.deadline, guard.hedge, guard.virtual_hedge)
    };
    if let Some((stage, (_, most))) = virtual_hedge {
        return Ok(virtually_hedged(&runtime, stage, most, start, convert));
    }
    if deadline.is_none() && hedge.is_none() {
        return Ok(convert(ASYNC.wait_future(start())));
    }
    let (stage, delay, most) = match hedge {
        Some((stage, (delay, most))) => (stage, std::time::Duration::from_micros(delay as u64), most),
        None => ("", std::time::Duration::ZERO, 1),
    };
    struct Unpark(std::thread::Thread);
    impl std::task::Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = std::task::Context::from_waker(&waker);
    let mut pending: Vec<std::pin::Pin<Box<F>>> = Vec::new();
    let (mut started, mut next, mut launch) = (0i64, None, true);
    loop {
        if launch {
            started += 1;
            if started > 1 {
                runtime.lock().unwrap().trace.push(TraceEvent { kind: "hedge", stage: stage.into(), number: started, succeeded: true });
            }
            pending.push(Box::pin(start()));
            next = if started < most { Some(std::time::Instant::now() + delay) } else { None };
            launch = false;
        }
        let mut last = None;
        let mut i = 0;
        while i < pending.len() {
            match pending[i].as_mut().poll(&mut context) {
                std::task::Poll::Ready(output) => {
                    pending.remove(i);
                    let value = convert(output);
                    if !matches!(value, Value::Left(_)) {
                        return Ok(value);
                    }
                    last = Some(value);
                }
                std::task::Poll::Pending => i += 1,
            }
        }
        let now = std::time::Instant::now();
        if deadline.is_some_and(|deadline| now >= deadline) {
            return Err(TIMED_OUT.into());
        }
        if pending.is_empty() {
            if started >= most {
                return Ok(last.expect("the last attempt failed"));
            }
            launch = true;
        } else if next.is_some_and(|next| now >= next) {
            launch = true;
        } else {
            match [deadline, next].into_iter().flatten().min() {
                Some(until) => std::thread::park_timeout(until - now),
                None => std::thread::park(),
            }
        }
    }
}

/// A hedge on a virtual clock: attempts run one after another, and the next
/// starts when one fails, so the first success wins as it would in real time
/// when no attempt outlives the delay.
fn virtually_hedged<F: std::future::Future>(
    runtime: &Arc<std::sync::Mutex<WorkflowRuntime>>,
    stage: &'static str,
    most: i64,
    mut start: impl FnMut() -> F,
    convert: impl Fn(F::Output) -> Value,
) -> Value {
    let mut started = 1i64;
    let mut value = convert(ASYNC.wait_future(start()));
    while matches!(value, Value::Left(_)) && started < most {
        started += 1;
        runtime.lock().unwrap().trace.push(TraceEvent { kind: "hedge", stage: stage.into(), number: started, succeeded: true });
        value = convert(ASYNC.wait_future(start()));
    }
    value
}

/// An attempt under its stage's timeout (failing with TimedOut when it
/// outlives it) and hedge: the Timeout and Hedge transformers of the Async
/// ability, measured on the runtime's Clock. On a virtual clock (generated
/// tests, or a law using virtual clock) an attempt takes the virtual time
/// that passes while it runs, so both are deterministic. On a real clock
/// with gates off, both are off.
fn scoped(
    ctx: &mut Context,
    runtime: &Arc<std::sync::Mutex<WorkflowRuntime>>,
    policy: &StagePolicy,
    attempt: &mut impl FnMut(&mut Context) -> Result<Value>,
) -> Result<Value> {
    if policy.timeout <= 0 && policy.hedge.is_none() {
        return attempt(ctx);
    }
    let (is_virtual, gates) = {
        let guard = runtime.lock().unwrap();
        (guard.clock.is_virtual(), guard.gates)
    };
    if is_virtual {
        return virtually_scoped(ctx, runtime, policy, attempt);
    }
    if !gates {
        return attempt(ctx);
    }
    let outer = {
        let mut guard = runtime.lock().unwrap();
        let outer = (guard.deadline, guard.hedge, guard.virtual_hedge);
        if policy.timeout > 0 {
            guard.deadline = Some(std::time::Instant::now() + std::time::Duration::from_micros(policy.timeout as u64));
        }
        guard.hedge = policy.hedge.map(|hedge| (policy.stage, hedge));
        guard.virtual_hedge = None;
        outer
    };
    let result = attempt(ctx);
    {
        let mut guard = runtime.lock().unwrap();
        (guard.deadline, guard.hedge, guard.virtual_hedge) = outer;
    }
    match result {
        // Callers may have added context to the marker.
        Err(error) if error.contains(TIMED_OUT) => Ok(stage_failure("TimedOut")),
        other => other,
    }
}

fn virtually_scoped(
    ctx: &mut Context,
    runtime: &Arc<std::sync::Mutex<WorkflowRuntime>>,
    policy: &StagePolicy,
    attempt: &mut impl FnMut(&mut Context) -> Result<Value>,
) -> Result<Value> {
    let (outer, began) = {
        let mut guard = runtime.lock().unwrap();
        let outer = (guard.deadline, guard.hedge, guard.virtual_hedge);
        let began = guard.clock.now();
        guard.deadline = None;
        guard.hedge = None;
        guard.virtual_hedge = policy.hedge.map(|hedge| (policy.stage, hedge));
        (outer, began)
    };
    let result = attempt(ctx);
    let ended = {
        let mut guard = runtime.lock().unwrap();
        (guard.deadline, guard.hedge, guard.virtual_hedge) = outer;
        guard.clock.now()
    };
    match result {
        Err(error) if error.contains(TIMED_OUT) => Ok(stage_failure("TimedOut")),
        Ok(_) if policy.timeout > 0 && ended - began > policy.timeout => Ok(stage_failure("TimedOut")),
        other => other,
    }
}

fn attempts(
    ctx: &mut Context,
    runtime: &Arc<std::sync::Mutex<WorkflowRuntime>>,
    policy: &StagePolicy,
    mut attempt: impl FnMut(&mut Context) -> Result<Value>,
) -> Result<Value> {
    let event = |kind: &'static str, number: i64, succeeded: bool| TraceEvent { kind, stage: policy.stage.into(), number, succeeded };
    let (mut number, mut previous) = (1i64, 0i64);
    loop {
        runtime.lock().unwrap().trace.push(event("start", number, false));
        let result = scoped(ctx, runtime, policy, &mut attempt)?;
        let failure = match &result {
            Value::Left(error) => Some((**error).clone()),
            _ => None,
        };
        runtime.lock().unwrap().trace.push(event("finish", number, failure.is_none()));
        let (Some(mut failure), Some(retry)) = (failure, &policy.retry) else { return Ok(result) };
        if retry.attempts > 0 && number >= retry.attempts {
            return Ok(result);
        }
        if policy.wraps {
            // Only the step's own failures and timeouts are retried.
            match failure {
                Value::Data(ref tag, ref mut fields) if *tag == format!("{STAGE_FAILURE}StepFailed") && fields.len() == 1 => {
                    failure = fields.pop().unwrap();
                }
                Value::Data(ref tag, _) if *tag == format!("{STAGE_FAILURE}TimedOut") => {}
                _ => return Ok(result),
            }
        }
        if let Some(when) = retry.when {
            if !when(ctx, failure.clone())? {
                return Ok(result);
            }
        }
        number += 1;
        let delay = if retry.strategy == "custom" {
            match (retry.decide.expect("a custom strategy decides"))(ctx, number, failure, previous)? {
                Some(delay) => delay,
                None => return Ok(result),
            }
        } else {
            let base = if retry.strategy == "immediate" { 0 } else { retry_delay(retry, 2) };
            let mut guard = runtime.lock().unwrap();
            jittered(retry.jitter, retry_delay(retry, number), previous, base, &mut guard.random)
        };
        let mut guard = runtime.lock().unwrap();
        guard.trace.push(event("sleep", delay, true));
        guard.clock.sleep(delay);
        previous = delay;
    }
}

/// A logical Duration of whole microseconds.
pub fn duration(micros: i64) -> Value {
    Value::Data(DURATION_TAG.into(), vec![Value::Integer(BigInt::from(micros))])
}

/// A RetryDecision's delay, or None to stop.
pub fn retry_decision(decision: Value) -> Result<Option<i64>> {
    match decision {
        Value::Data(tag, mut fields) if tag == "lawspec.time::type::RetryDecision::RetryAfter" && fields.len() == 1 => {
            match fields.pop().unwrap() {
                Value::Data(_, mut micros) if micros.len() == 1 => match micros.pop().unwrap() {
                    Value::Integer(n) => n.to_i64().map(Some).ok_or_else(|| "delay outside i64".into()),
                    _ => Err("expected Duration microseconds".into()),
                },
                _ => Err("expected a Duration".into()),
            }
        }
        Value::Data(..) => Ok(None),
        _ => Err("expected a RetryDecision".into()),
    }
}

/// A Duration, whole microseconds from 0 to about 146 years, is natively a
/// std::time::Duration. One with a fraction of a microsecond is not a
/// Duration; IntoValue cannot fail, so converting one panics.
pub const DURATION_TAG: &str = "lawspec.time::type::Duration::Duration";

impl IntoValue for std::time::Duration {
    fn into_value(self) -> Value {
        assert!(
            self.subsec_nanos() % 1000 == 0,
            "a Duration is a non-negative whole number of microseconds"
        );
        Value::Data(DURATION_TAG.into(), vec![Value::Integer(BigInt::from(self.as_micros()))])
    }
}
impl FromValue for std::time::Duration {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Data(tag, fields) if tag == DURATION_TAG && fields.len() == 1 => match &fields[0] {
                Value::Integer(micros) => u64::try_from(micros)
                    .map(std::time::Duration::from_micros)
                    .map_err(|_| "Duration outside the native range".into()),
                _ => Err("expected Duration microseconds".into()),
            },
            _ => Err("expected Duration".into()),
        }
    }
}

/// Built-in collections: natively a BTreeSet, a BTreeMap, a VecDeque (Queue
/// and Deque) or a Vec whose top is last (Stack). Natives convert to plain
/// lists; the schema retags them by the expected type.
pub const COLLECTIONS: &str = "lawspec.collections::type::";

pub fn collection_tag(name: &str) -> Option<&'static str> {
    Some(match name.strip_prefix(COLLECTIONS)? {
        "Set" => "lawspec.collections::type::Set::SetItems",
        "KeyVal" => "lawspec.collections::type::KeyVal::KeyValEntries",
        "Queue" => "lawspec.collections::type::Queue::QueueItems",
        "Stack" => "lawspec.collections::type::Stack::StackItems",
        "Deque" => "lawspec.collections::type::Deque::DequeItems",
        _ => return None,
    })
}

fn collection_items(value: Value) -> Result<Vec<Value>> {
    match value {
        Value::List(items) => Ok(items),
        Value::Data(tag, mut fields) if tag.starts_with(COLLECTIONS) && fields.len() == 1 => {
            let reverse = tag.ends_with("::StackItems");
            match fields.pop().unwrap() {
                Value::List(mut items) => {
                    if reverse {
                        items.reverse();
                    }
                    Ok(items)
                }
                _ => Err("expected collection items".into()),
            }
        }
        _ => Err("expected a collection".into()),
    }
}

impl<T: IntoValue> IntoValue for std::collections::BTreeSet<T> {
    fn into_value(self) -> Value {
        Value::List(self.into_iter().map(IntoValue::into_value).collect())
    }
}
impl<T: FromValue + Ord> FromValue for std::collections::BTreeSet<T> {
    fn from_value(value: Value) -> Result<Self> {
        collection_items(value)?.into_iter().map(T::from_value).collect()
    }
}
impl<K: IntoValue, V: IntoValue> IntoValue for std::collections::BTreeMap<K, V> {
    fn into_value(self) -> Value {
        Value::List(
            self.into_iter()
                .map(|(k, v)| Value::Data("lawspec.collections::type::Entry::Entry".into(), vec![k.into_value(), v.into_value()]))
                .collect(),
        )
    }
}
impl<K: FromValue + Ord, V: FromValue> FromValue for std::collections::BTreeMap<K, V> {
    fn from_value(value: Value) -> Result<Self> {
        collection_items(value)?
            .into_iter()
            .map(|entry| match entry {
                Value::Data(_, fields) if fields.len() == 2 => {
                    let mut fields = fields.into_iter();
                    Ok((K::from_value(fields.next().unwrap())?, V::from_value(fields.next().unwrap())?))
                }
                _ => Err("expected a KeyVal entry".into()),
            })
            .collect()
    }
}
impl<T: IntoValue> IntoValue for std::collections::VecDeque<T> {
    fn into_value(self) -> Value {
        Value::List(self.into_iter().map(IntoValue::into_value).collect())
    }
}
impl<T: FromValue> FromValue for std::collections::VecDeque<T> {
    fn from_value(value: Value) -> Result<Self> {
        collection_items(value)?.into_iter().map(T::from_value).collect()
    }
}

pub fn helper(name: &str, mut args: Vec<Value>) -> Result<Value> {
    use Value::*;
    if let Some(value) = matcher_helper(name, &args)? {
        return Ok(value);
    }
    if name == "select" && args.len() == 3 {
        let other = args.pop().unwrap();
        let chosen = args.pop().unwrap();
        return Ok(if matches!(args[0], Bool(true)) { chosen } else { other });
    }
    if name == "compare" && args.len() == 2 {
        let tag = match compare_values(&args[0], &args[1])? {
            std::cmp::Ordering::Less => "Less",
            std::cmp::Ordering::Equal => "Equal",
            std::cmp::Ordering::Greater => "Greater",
        };
        return Ok(Data(format!("lawspec.collections::type::Ordering::{tag}"), vec![]));
    }
    if name == "round" && args.len() == 2 {
        let scale = i32::from_value(args.pop().unwrap())?;
        return Ok(Decimal(self::Decimal::round(args[0].exact()?, scale)?));
    }
    if args.len() != 1 {
        return Err(format!("invalid {name} arguments"));
    }
    let value = args.pop().unwrap();
    Ok(match name {
        "checked" => Bool(true),
        "length" => Integer(BigInt::from(match value {
            Text(s) => s.chars().count(),
            CodePointText(s) => s.len(),
            Utf16Text(s) => s.len(),
            Bytes(s) => s.len(),
            List(s) => s.len(),
            _ => return Err("length requires sequence".into()),
        })),
        "isPresent" => Bool(match value {
            Nullable(v) | Optional(v) => v.is_some(),
            _ => return Err("expected presence".into()),
        }),
        "presentValue" => match value {
            Nullable(Some(v)) | Optional(Some(v)) => *v,
            _ => return Err("absent value has no payload".into()),
        },
        "real" => match value {
            Complex32(c) => Float32(c.re),
            Complex64(c) => Float64(c.re),
            _ => return Err("expected complex value".into()),
        },
        "imag" => match value {
            Complex32(c) => Float32(c.im),
            Complex64(c) => Float64(c.im),
            _ => return Err("expected complex value".into()),
        },
        "isNaN" => Bool(value.real64()?.is_nan()),
        "isInfinite" => Bool(value.real64()?.is_infinite()),
        "isFinite" => Bool(value.real64()?.is_finite()),
        "isNegativeZero" => {
            let x = value.real64()?;
            Bool(x == 0.0 && x.is_sign_negative())
        }
        _ => return Err(format!("unknown helper: {name}")),
    })
}

/// Where a structured actual value first differs from the expected one, by
/// the portable rendering: "" when they agree or neither has parts.
pub fn difference(actual: &Value, expected: &Value) -> String {
    fn parts(v: &Value) -> Option<(Option<String>, Vec<&Value>)> {
        match v {
            Value::List(xs) => Some((None, xs.iter().collect())),
            Value::Data(tag, fields) => Some((Some(tag.clone()), fields.iter().collect())),
            Value::Maybe(Some(x)) => Some((Some("Just".into()), vec![x.as_ref()])),
            Value::Left(x) => Some((Some("Left".into()), vec![x.as_ref()])),
            Value::Right(x) => Some((Some("Right".into()), vec![x.as_ref()])),
            _ => None,
        }
    }
    let mut path = vec![];
    let (mut a, mut b) = (actual, expected);
    let mut lengths = None;
    while let (Some((ta, xs)), Some((tb, ys))) = (parts(a), parts(b)) {
        if ta != tb || (ta.is_some() && xs.len() != ys.len()) {
            break;
        }
        match xs.iter().zip(ys.iter()).position(|(x, y)| render(x) != render(y)) {
            Some(i) => {
                path.push(match &ta {
                    None => format!("item {}", i + 1),
                    Some(tag) => format!("field {} of {}", i + 1, tag.rsplit("::").next().unwrap_or(tag)),
                });
                (a, b) = (xs[i], ys[i]);
            }
            None if xs.len() == ys.len() => return String::new(),
            None => {
                path.push("length".to_string());
                lengths = Some((xs.len(), ys.len()));
                break;
            }
        }
    }
    if path.is_empty() {
        return String::new();
    }
    let (shown_a, shown_b) = match lengths {
        Some((x, y)) => (x.to_string(), y.to_string()),
        None => (render(a), render(b)),
    };
    format!(" | first difference at {}: expected {shown_b}, actual {shown_a}", path.join(", "))
}

/// The matcher, recording and resource helpers, or None for another helper.
fn matcher_helper(name: &str, args: &[Value]) -> Result<Option<Value>> {
    let text = |i: usize| -> Result<&str> {
        match args.get(i) {
            Some(Value::Text(s)) => Ok(s.as_str()),
            _ => Err(format!("{name} expects text")),
        }
    };
    Ok(Some(match name {
        "startsWith" => Value::Bool(text(0)?.starts_with(text(1)?)),
        "endsWith" => Value::Bool(text(0)?.ends_with(text(1)?)),
        "textContains" => Value::Bool(text(0)?.contains(text(1)?)),
        "regexMatches" => Value::Bool(regex_matches(text(0)?, text(1)?)?),
        "recorded" => Value::Bool(recorded(text(0)?, args.get(1).ok_or("recorded expects a value")?)?),
        "acquireResource" => Value::Text(acquire_resource(text(0)?)?),
        "releaseResource" => {
            release_resource(text(0)?, text(1)?)?;
            Value::Bool(true)
        }
        "freePort" => Value::Integer(BigInt::from(free_port()?)),
        _ => return Ok(None),
    }))
}

// Portable regular expressions (see LawSpec.Regex): the subset of RE2 and
// ECMAScript that means the same in both, matched against a whole text, code
// point by code point. The compiler has checked every pattern; a pattern
// that is not portable is an error here too.
#[derive(Clone, Debug)]
struct RegexItem {
    negated: bool,
    ranges: Vec<(u32, u32)>,
}

#[derive(Clone, Debug)]
enum RegexNode {
    Set(bool, Vec<RegexItem>),
    Sequence(Vec<RegexNode>),
    Alternatives(Vec<RegexNode>),
    Repeat(Box<RegexNode>, usize, Option<usize>),
}

const REGEX_DIGITS: &[(u32, u32)] = &[(48, 57)];
const REGEX_WORD: &[(u32, u32)] = &[(48, 57), (65, 90), (95, 95), (97, 122)];
const REGEX_SPACE: &[(u32, u32)] = &[(9, 13), (32, 32)];

struct RegexParser<'a> {
    pattern: &'a str,
    cs: Vec<u32>,
    pos: usize,
}

impl RegexParser<'_> {
    fn peek(&self) -> Option<u32> {
        self.cs.get(self.pos).copied()
    }

    fn fail<T>(&self, message: &str) -> Result<T> {
        Err(format!("regex {:?} is not portable: {message}", self.pattern))
    }

    fn escape(&mut self) -> Result<RegexItem> {
        self.pos += 1;
        let Some(c) = self.peek() else { return self.fail("the regex ends with a lone \\") };
        self.pos += 1;
        let item = |negated: bool, ranges: &[(u32, u32)]| RegexItem { negated, ranges: ranges.to_vec() };
        Ok(match char::from_u32(c).unwrap_or('\0') {
            'd' => item(false, REGEX_DIGITS),
            'D' => item(true, REGEX_DIGITS),
            'w' => item(false, REGEX_WORD),
            'W' => item(true, REGEX_WORD),
            's' => item(false, REGEX_SPACE),
            'S' => item(true, REGEX_SPACE),
            'n' => item(false, &[(10, 10)]),
            't' => item(false, &[(9, 9)]),
            'r' => item(false, &[(13, 13)]),
            'f' => item(false, &[(12, 12)]),
            'v' => item(false, &[(11, 11)]),
            ch if "\\.^$|?*+()[]{}-/".contains(ch) => item(false, &[(c, c)]),
            ch => return self.fail(&format!("\\{ch} is not a portable escape")),
        })
    }

    fn literal(&mut self) -> Result<RegexItem> {
        let c = self.peek().unwrap_or(0);
        if c == '[' as u32 {
            return self.fail("write \\[ for the character inside a class");
        }
        self.pos += 1;
        Ok(RegexItem { negated: false, ranges: vec![(c, c)] })
    }

    fn single(item: &RegexItem) -> bool {
        !item.negated && item.ranges.len() == 1 && item.ranges[0].0 == item.ranges[0].1
    }

    fn class(&mut self) -> Result<RegexNode> {
        self.pos += 1;
        let negated = self.peek() == Some('^' as u32);
        if negated {
            self.pos += 1;
        }
        let mut items = vec![];
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return self.fail("a [ is never closed") };
            if c == ']' as u32 {
                if first {
                    return self.fail("an empty class is not portable");
                }
                self.pos += 1;
                return Ok(RegexNode::Set(negated, items));
            }
            first = false;
            let item = if c == '\\' as u32 { self.escape()? } else { self.literal()? };
            if Self::single(&item)
                && self.peek() == Some('-' as u32)
                && self.cs.get(self.pos + 1).is_some_and(|&next| next != ']' as u32)
            {
                self.pos += 1;
                let high = if self.peek() == Some('\\' as u32) { self.escape()? } else { self.literal()? };
                if !Self::single(&high) {
                    return self.fail("a range ends with one character");
                }
                if high.ranges[0].0 < item.ranges[0].0 {
                    return self.fail("a range must run from low to high");
                }
                items.push(RegexItem { negated: false, ranges: vec![(item.ranges[0].0, high.ranges[0].0)] });
            } else {
                items.push(item);
            }
        }
    }

    fn digits(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| ('0' as u32..='9' as u32).contains(&c)) {
            self.pos += 1;
        }
        self.cs[start..self.pos].iter().filter_map(|&c| char::from_u32(c)).collect()
    }

    fn atom(&mut self) -> Result<RegexNode> {
        let c = self.peek().unwrap_or(0);
        let ch = char::from_u32(c).unwrap_or('\0');
        match ch {
            '(' => {
                self.pos += 1;
                if self.peek() == Some('?' as u32) {
                    if self.cs.get(self.pos + 1) == Some(&(':' as u32)) {
                        self.pos += 2;
                    } else {
                        return self.fail("only (?: ...) groups are portable");
                    }
                }
                let node = self.alternatives()?;
                if self.peek() != Some(')' as u32) {
                    return self.fail("a ( is never closed");
                }
                self.pos += 1;
                Ok(node)
            }
            '[' => self.class(),
            '.' => {
                self.pos += 1;
                Ok(RegexNode::Set(true, vec![RegexItem { negated: false, ranges: vec![(10, 10)] }]))
            }
            '\\' => Ok(RegexNode::Set(false, vec![self.escape()?])),
            _ if "*+?{^$]}".contains(ch) => self.fail(&format!("unexpected {ch}")),
            _ => {
                self.pos += 1;
                Ok(RegexNode::Set(false, vec![RegexItem { negated: false, ranges: vec![(c, c)] }]))
            }
        }
    }

    fn quantifier(c: Option<u32>) -> bool {
        c.and_then(char::from_u32).is_some_and(|ch| "*+?{".contains(ch))
    }

    fn quantified(&mut self, node: RegexNode) -> Result<RegexNode> {
        let c = self.peek();
        if !Self::quantifier(c) {
            return Ok(node);
        }
        self.pos += 1;
        let repeat = |low, high| RegexNode::Repeat(Box::new(node.clone()), low, high);
        let result = match char::from_u32(c.unwrap_or(0)).unwrap_or('\0') {
            '*' => repeat(0, None),
            '+' => repeat(1, None),
            '?' => repeat(0, Some(1)),
            _ => {
                let low_text = self.digits();
                let high_text;
                if self.peek() == Some('}' as u32) {
                    high_text = Some(low_text.clone());
                } else if self.peek() == Some(',' as u32) {
                    self.pos += 1;
                    let text = self.digits();
                    high_text = if text.is_empty() { None } else { Some(text) };
                    if self.peek() != Some('}' as u32) {
                        return self.fail("a repetition is {n}, {n,} or {n,m}");
                    }
                } else {
                    return self.fail("a repetition is {n}, {n,} or {n,m}");
                }
                self.pos += 1;
                if low_text.is_empty() {
                    return self.fail("a repetition is {n}, {n,} or {n,m}");
                }
                let low: usize = low_text.parse().unwrap_or(usize::MAX);
                let high: Option<usize> = high_text.map(|t| t.parse().unwrap_or(usize::MAX));
                if low > 1000 || high.is_some_and(|h| h > 1000 || h < low) {
                    return self.fail("a repetition count is at most 1000, and n must not exceed m");
                }
                repeat(low, high)
            }
        };
        if Self::quantifier(self.peek()) {
            return self.fail("a repetition cannot itself be repeated");
        }
        Ok(result)
    }

    fn sequence(&mut self) -> Result<RegexNode> {
        let mut items = vec![];
        while let Some(c) = self.peek() {
            if c == '|' as u32 || c == ')' as u32 {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(RegexNode::Sequence(items))
    }

    fn alternatives(&mut self) -> Result<RegexNode> {
        let mut branches = vec![self.sequence()?];
        while self.peek() == Some('|' as u32) {
            self.pos += 1;
            branches.push(self.sequence()?);
        }
        Ok(if branches.len() == 1 { branches.pop().unwrap() } else { RegexNode::Alternatives(branches) })
    }
}

fn regex_parse(pattern: &str) -> Result<RegexNode> {
    let mut parser = RegexParser { pattern, cs: pattern.chars().map(|c| c as u32).collect(), pos: 0 };
    let node = parser.alternatives()?;
    if parser.pos != parser.cs.len() {
        return parser.fail("a ) has no ( before it");
    }
    Ok(node)
}

fn regex_reach(node: &RegexNode, text: &[u32], positions: Vec<bool>) -> Vec<bool> {
    let mut next = vec![false; text.len() + 1];
    match node {
        RegexNode::Set(negated, items) => {
            for (p, &at) in positions.iter().enumerate() {
                if !at || p >= text.len() {
                    continue;
                }
                let c = text[p];
                let inside = items
                    .iter()
                    .any(|item| item.negated != item.ranges.iter().any(|&(lo, hi)| lo <= c && c <= hi));
                if inside != *negated {
                    next[p + 1] = true;
                }
            }
            next
        }
        RegexNode::Sequence(items) => items.iter().fold(positions, |at, item| regex_reach(item, text, at)),
        RegexNode::Alternatives(branches) => {
            for branch in branches {
                for (p, at) in regex_reach(branch, text, positions.clone()).into_iter().enumerate() {
                    next[p] |= at;
                }
            }
            next
        }
        RegexNode::Repeat(body, low, high) => {
            let mut positions = positions;
            for _ in 0..*low {
                positions = regex_reach(body, text, positions);
            }
            let mut seen = positions.clone();
            let mut frontier = positions;
            let mut limit = high.map(|h| h - low);
            while limit != Some(0) {
                let reached = regex_reach(body, text, frontier);
                frontier = vec![false; text.len() + 1];
                let mut any = false;
                for (p, at) in reached.into_iter().enumerate() {
                    if at && !seen[p] {
                        frontier[p] = true;
                        seen[p] = true;
                        any = true;
                    }
                }
                if !any {
                    break;
                }
                limit = limit.map(|l| l - 1);
            }
            seen
        }
    }
}

static REGEX_CACHE: std::sync::LazyLock<std::sync::Mutex<HashMap<String, RegexNode>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Whether the portable regex matches the whole text.
pub fn regex_matches(pattern: &str, text: &str) -> Result<bool> {
    let cached = REGEX_CACHE.lock().unwrap().get(pattern).cloned();
    let node = match cached {
        Some(node) => node,
        None => {
            let node = regex_parse(pattern)?;
            REGEX_CACHE.lock().unwrap().insert(pattern.to_string(), node.clone());
            node
        }
    };
    let points: Vec<u32> = text.chars().map(|c| c as u32).collect();
    let mut start = vec![false; points.len() + 1];
    start[0] = true;
    Ok(regex_reach(&node, &points, start)[points.len()])
}

// Recorded values: a law compares a value's portable rendering with the
// text stored under recorded/<unit>/<name> in the project. LAWSPEC_RECORDED
// names the folder; otherwise it is recorded/ in the nearest folder, from
// the working one up, that holds lawspec.json or recorded/. With
// LAWSPEC_UPDATE_RECORDED=1 (lawspec test --update-recorded) a law records
// the value instead.
pub fn recorded_root() -> std::path::PathBuf {
    if let Ok(given) = std::env::var("LAWSPEC_RECORDED") {
        if !given.is_empty() {
            return given.into();
        }
    }
    let start = std::env::current_dir().unwrap_or_default();
    let mut folder = start.as_path();
    loop {
        if folder.join("lawspec.json").exists() || folder.join("recorded").is_dir() {
            return folder.join("recorded");
        }
        match folder.parent() {
            Some(parent) => folder = parent,
            None => return start.join("recorded"),
        }
    }
}

pub fn recorded(key: &str, value: &Value) -> Result<bool> {
    let text = render(value);
    let mut path = recorded_root();
    for part in key.split('/') {
        path.push(part);
    }
    if std::env::var("LAWSPEC_UPDATE_RECORDED").as_deref() == Ok("1") {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, format!("{text}\n")).map_err(|e| e.to_string())?;
        return Ok(true);
    }
    let Ok(stored) = std::fs::read_to_string(&path) else {
        return Err(format!("no recording recorded/{key}; run lawspec test --update-recorded to record {text}"));
    };
    let stored = stored.strip_suffix('\n').unwrap_or(&stored);
    if stored != text {
        return Err(format!(
            "recorded/{key} differs: expected {stored}, actual {text} (lawspec test --update-recorded records the new value)"
        ));
    }
    Ok(true)
}

// Shared resources (share R per group | unit | run, in a harness): one value
// per scope key. The first use acquires it; every later use resets it first,
// so no case sees what another left. One case holds it at a time (until its
// SharedGuard drops, even on a panic), and it is released when the test
// process exits.
struct SharedEntry {
    busy: bool,
    users: usize,
    concurrent: bool,
    value: Option<Value>,
}
static SHARED: std::sync::Mutex<Option<HashMap<String, SharedEntry>>> = std::sync::Mutex::new(None);
static SHARED_FREE: std::sync::Condvar = std::sync::Condvar::new();
type SharedRelease = Box<dyn FnOnce() + Send>;
static SHARED_RELEASES: std::sync::Mutex<Vec<SharedRelease>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> std::os::raw::c_int;
}

extern "C" fn release_shared() {
    let releases: Vec<SharedRelease> = match SHARED_RELEASES.lock() {
        Ok(mut list) => list.drain(..).rev().collect(),
        Err(_) => return,
    };
    for release in releases {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(release));
    }
}

pub struct SharedGuard(String);

impl Drop for SharedGuard {
    fn drop(&mut self) {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = shared.get_or_insert_with(HashMap::new).get_mut(&self.0) {
            if entry.concurrent {
                entry.users = entry.users.saturating_sub(1);
                if entry.users == 0 {
                    entry.busy = false;
                }
            } else {
                entry.busy = false;
            }
        }
        SHARED_FREE.notify_all();
    }
}

/// A concurrent resource (resource T is concurrent) is held by any number of
/// cases at once, and reset only when none holds it.
pub fn share<A, R, F>(key: &str, ctx: &mut Context, acquire: A, reset: R, release: F, concurrent: bool) -> Result<(Value, SharedGuard)>
where
    A: FnOnce(&mut Context) -> Result<Value>,
    R: FnOnce(&mut Context, Value) -> Result<Value>,
    F: FnOnce(&mut Context, Value) -> Result<Value> + Send + 'static,
{
    if concurrent {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        let entry = shared.get_or_insert_with(HashMap::new).entry(key.to_string())
            .or_insert(SharedEntry { busy: false, users: 0, concurrent: true, value: None });
        let value = match entry.value.clone() {
            Some(value) => {
                if entry.users == 0 {
                    reset(ctx, value.clone())?;
                }
                value
            }
            None => {
                let value = acquire(ctx)?;
                entry.value = Some(value.clone());
                let kept = value.clone();
                let mut releases = SHARED_RELEASES.lock().unwrap_or_else(|e| e.into_inner());
                if releases.is_empty() {
                    unsafe { atexit(release_shared); }
                }
                releases.push(Box::new(move || {
                    let ctx = &mut Context::testing();
                    let _ = release(ctx, kept);
                }));
                value
            }
        };
        entry.users += 1;
        entry.busy = true;
        return Ok((value, SharedGuard(key.to_string())));
    }
    let held = {
        let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            let entry = shared.get_or_insert_with(HashMap::new).entry(key.to_string())
                .or_insert(SharedEntry { busy: false, users: 0, concurrent: false, value: None });
            if !entry.busy {
                entry.busy = true;
                break entry.value.clone();
            }
            shared = SHARED_FREE.wait(shared).unwrap_or_else(|e| e.into_inner());
        }
    };
    let guard = SharedGuard(key.to_string());
    match held {
        Some(value) => {
            reset(ctx, value.clone())?;
            Ok((value, guard))
        }
        None => {
            let value = acquire(ctx)?;
            {
                let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = shared.get_or_insert_with(HashMap::new).get_mut(key) {
                    entry.value = Some(value.clone());
                }
            }
            let kept = value.clone();
            let mut releases = SHARED_RELEASES.lock().unwrap_or_else(|e| e.into_inner());
            if releases.is_empty() {
                unsafe { atexit(release_shared); }
            }
            releases.push(Box::new(move || {
                let ctx = &mut Context::testing();
                let _ = release(ctx, kept);
            }));
            Ok((value, guard))
        }
    }
}

// Built-in resources (see LawSpec.Resources): a law acquires them before
// each case and releases them after it. Tests run on several threads, so a
// saved environment also holds the environment for its case alone.
static ENVIRONMENT_HELD: std::sync::Mutex<bool> = std::sync::Mutex::new(false);
static ENVIRONMENT_FREE: std::sync::Condvar = std::sync::Condvar::new();
static TEMPORARY_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn acquire_resource(kind: &str) -> Result<String> {
    match kind {
        "temporaryDirectory" | "temporaryFile" => {
            let unique = TEMPORARY_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("lawspec-{}-{nanos}-{unique}", std::process::id()));
            if kind == "temporaryDirectory" {
                std::fs::create_dir(&path).map_err(|e| e.to_string())?;
            } else {
                std::fs::write(&path, "").map_err(|e| e.to_string())?;
            }
            Ok(path.to_string_lossy().into_owned())
        }
        "environment" => {
            let mut held = ENVIRONMENT_HELD.lock().unwrap();
            while *held {
                held = ENVIRONMENT_FREE.wait(held).unwrap();
            }
            *held = true;
            let mut saved: Vec<(String, String)> = std::env::vars().collect();
            saved.sort();
            Ok(saved.into_iter().map(|(k, v)| format!("{k}\0{v}\0")).collect())
        }
        _ => Err(format!("unknown resource kind {kind}")),
    }
}

pub fn release_resource(kind: &str, value: &str) -> Result<()> {
    match kind {
        "temporaryDirectory" => {
            let _ = std::fs::remove_dir_all(value);
        }
        "temporaryFile" => {
            let _ = std::fs::remove_file(value);
        }
        "environment" => {
            let parts: Vec<&str> = value.split('\0').collect();
            let saved: HashMap<&str, &str> = parts.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).collect();
            for (name, _) in std::env::vars() {
                if !saved.contains_key(name.as_str()) {
                    // SAFETY: the saved environment holds the environment for
                    // this case alone (ENVIRONMENT_HELD).
                    unsafe { std::env::remove_var(&name) };
                }
            }
            for (name, text) in &saved {
                if std::env::var(name).as_deref() != Ok(*text) {
                    // SAFETY: as above.
                    unsafe { std::env::set_var(name, text) };
                }
            }
            *ENVIRONMENT_HELD.lock().unwrap() = false;
            ENVIRONMENT_FREE.notify_one();
        }
        _ => return Err(format!("unknown resource kind {kind}")),
    }
    Ok(())
}

/// A TCP port on the local host that is free now.
pub fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    listener.local_addr().map(|a| a.port()).map_err(|e| e.to_string())
}

// Portable generation for stateful models. A type descriptor is an
// s-expression: (int T lo hi) with _ for no bound, (bool), (text), (unit),
// (list D), (maybe D), (either L R), (data NAME (ctor TAG D...) ...) and
// (ref NAME) for a data type declared in the model's table. Every target
// generates, shrinks and renders the same values for the same seed, so the
// draws, candidate order and rendering below follow the Python reference.

/// A descriptor form. Quoted strings and symbols are one atom: the reference
/// compares them as equal strings.
#[derive(Clone, Debug, PartialEq)]
pub enum Sexp {
    List(Vec<Sexp>),
    Int(BigInt),
    Atom(String),
    Blank,
}

impl Sexp {
    fn items(&self) -> &[Sexp] {
        match self {
            Sexp::List(items) => items,
            other => panic!("expected a descriptor form, got {other:?}"),
        }
    }

    fn kind(&self) -> &str {
        match self.items().first() {
            Some(Sexp::Atom(kind)) => kind,
            _ => "",
        }
    }

    fn name(&self) -> String {
        match self {
            Sexp::Atom(s) => s.clone(),
            Sexp::Int(n) => n.to_string(),
            Sexp::Blank => "None".into(),
            Sexp::List(_) => format!("{self:?}"),
        }
    }

    fn bound(&self) -> Option<BigInt> {
        match self {
            Sexp::Int(n) => Some(n.clone()),
            Sexp::Blank => None,
            other => panic!("expected an integer bound, got {other:?}"),
        }
    }
}

/// Parses s-expressions: lists, integers, strings, symbols and _ (Blank).
pub fn read_descriptor(text: &str) -> Vec<Sexp> {
    let chars: Vec<char> = text.chars().collect();
    let space = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    fn skip(chars: &[char], at: &mut usize, space: &dyn Fn(char) -> bool) {
        while *at < chars.len() && space(chars[*at]) {
            *at += 1;
        }
    }
    fn item(chars: &[char], at: &mut usize, space: &dyn Fn(char) -> bool) -> Sexp {
        skip(chars, at, space);
        match chars[*at] {
            '(' => {
                *at += 1;
                let mut items = Vec::new();
                skip(chars, at, space);
                while chars[*at] != ')' {
                    items.push(item(chars, at, space));
                    skip(chars, at, space);
                }
                *at += 1;
                Sexp::List(items)
            }
            '"' => {
                *at += 1;
                let mut out = String::new();
                while chars[*at] != '"' {
                    if chars[*at] == '\\' {
                        *at += 1;
                    }
                    out.push(chars[*at]);
                    *at += 1;
                }
                *at += 1;
                Sexp::Atom(out)
            }
            _ => {
                let start = *at;
                while *at < chars.len() && !space(chars[*at]) && chars[*at] != '(' && chars[*at] != ')' {
                    *at += 1;
                }
                let atom: String = chars[start..*at].iter().collect();
                let digits = atom.trim_start_matches('-');
                if atom == "_" {
                    Sexp::Blank
                } else if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                    atom.parse::<BigInt>().map(Sexp::Int).unwrap_or(Sexp::Atom(atom))
                } else {
                    Sexp::Atom(atom)
                }
            }
        }
    }
    let mut at = 0;
    let mut forms = Vec::new();
    skip(&chars, &mut at, &space);
    while at < chars.len() {
        forms.push(item(&chars, &mut at, &space));
        skip(&chars, &mut at, &space);
    }
    forms
}

const UNBOUNDED: i64 = 1_000_000;

// Uniform in [0, bound) like SplitMix64::below, but for any bound: a range
// such as UInt64's is 2^64 wide.
fn below_big(random: &mut SplitMix64, bound: &BigInt) -> BigInt {
    if bound.is_positive() { BigInt::from(random.next()) % bound } else { BigInt::zero() }
}

fn below_len(random: &mut SplitMix64, bound: i64) -> usize {
    if bound > 0 { random.below(bound as u64) as usize } else { 0 }
}

/// Generation, shrinking and rendering over a table of data types.
#[derive(Clone, Debug)]
pub struct Values {
    pub table: HashMap<String, Sexp>,
}

impl Values {
    pub fn new(table: HashMap<String, Sexp>) -> Self {
        Values { table }
    }

    pub fn resolve<'a>(&'a self, d: &'a Sexp) -> &'a Sexp {
        if d.kind() == "ref" {
            let name = d.items()[1].name();
            self.table.get(&name).unwrap_or_else(|| panic!("unknown data type {name}"))
        } else {
            d
        }
    }

    /// An integer's range: a missing bound is 1,000,000 from zero, or
    /// 2,000,000 from the other bound when that is beyond it.
    pub fn bounds(&self, d: &Sexp) -> (BigInt, BigInt) {
        let items = d.items();
        let far = BigInt::from(UNBOUNDED);
        match (items[2].bound(), items[3].bound()) {
            (None, None) => (-far.clone(), far),
            (None, Some(hi)) => ((-far.clone()).min(&hi - 2 * &far), hi),
            (Some(lo), None) => (lo.clone(), far.clone().max(&lo + 2 * &far)),
            (Some(lo), Some(hi)) => (lo, hi),
        }
    }

    /// The constructors whose fields mention no data type.
    pub fn base<'a>(&self, d: &'a Sexp) -> Vec<&'a Sexp> {
        let ctors = &d.items()[2..];
        let found: Vec<&Sexp> = ctors.iter().filter(|c| !c.items()[2..].iter().any(mentions_data)).collect();
        if found.is_empty() { ctors.iter().collect() } else { found }
    }

    pub fn generate(&self, d: &Sexp, random: &mut SplitMix64, size: i64) -> Value {
        let d = self.resolve(d);
        let items = d.items();
        match d.kind() {
            "int" => {
                let (lo, hi) = self.bounds(d);
                if random.below(10) < 2 {
                    let specials = [lo.clone(), hi.clone(), clamp(0, &lo, &hi), clamp(1, &lo, &hi)];
                    return Value::Integer(specials[random.below(4) as usize].clone());
                }
                let offset = below_big(random, &(&hi - &lo + 1));
                Value::Integer(lo + offset)
            }
            "bool" => Value::Bool(random.below(2) == 1),
            "text" => {
                let n = below_len(random, size + 1);
                Value::Text((0..n).map(|_| char::from(32 + random.below(95) as u8)).collect())
            }
            "unit" => Value::Unit,
            "list" => {
                let n = below_len(random, size + 1);
                Value::List((0..n).map(|_| self.generate(&items[1], random, size)).collect())
            }
            "maybe" => {
                if random.below(4) == 0 {
                    return Value::Maybe(None);
                }
                Value::Maybe(Some(Box::new(self.generate(&items[1], random, size))))
            }
            "either" => {
                if random.below(2) == 0 {
                    return Value::Left(Box::new(self.generate(&items[1], random, size)));
                }
                Value::Right(Box::new(self.generate(&items[2], random, size)))
            }
            "data" => {
                let choices: Vec<&Sexp> = if size <= 0 { self.base(d) } else { items[2..].iter().collect() };
                let ctor = choices[random.below(choices.len() as u64) as usize].items();
                let fields = ctor[2..].iter().map(|f| self.generate(f, random, (size - 1).max(0))).collect();
                Value::Data(ctor[1].name(), fields)
            }
            _ => panic!("unknown descriptor {d:?}"),
        }
    }

    pub fn minimal(&self, d: &Sexp) -> Value {
        let d = self.resolve(d);
        let items = d.items();
        match d.kind() {
            "int" => {
                let (lo, hi) = self.bounds(d);
                Value::Integer(clamp(0, &lo, &hi))
            }
            "bool" => Value::Bool(false),
            "text" => Value::Text(String::new()),
            "unit" => Value::Unit,
            "list" => Value::List(Vec::new()),
            "maybe" => Value::Maybe(None),
            "either" => Value::Left(Box::new(self.minimal(&items[1]))),
            _ => {
                let ctor = self.base(d)[0].items();
                Value::Data(ctor[1].name(), ctor[2..].iter().map(|f| self.minimal(f)).collect())
            }
        }
    }

    /// Smaller candidates for v, most aggressive first.
    pub fn shrink(&self, d: &Sexp, v: &Value) -> Vec<Value> {
        let d = self.resolve(d);
        let items = d.items();
        let mut out: Vec<Value> = Vec::new();
        match (d.kind(), v) {
            ("int", Value::Integer(n)) => {
                let target = match self.minimal(d) {
                    Value::Integer(t) => t,
                    _ => unreachable!(),
                };
                if *n != target {
                    let step = if *n > target { BigInt::one() } else { -BigInt::one() };
                    out = vec![
                        Value::Integer(target.clone()),
                        Value::Integer(n - toward_zero(&(n - &target), 2)),
                        Value::Integer(n - step),
                    ];
                }
            }
            ("bool", Value::Bool(b)) => {
                if *b {
                    out = vec![Value::Bool(false)];
                }
            }
            ("text", Value::Text(s)) => {
                let cs: Vec<char> = s.chars().collect();
                if !cs.is_empty() {
                    out.push(Value::Text(String::new()));
                    out.push(Value::Text(cs[..cs.len() / 2].iter().collect()));
                    for i in 0..cs.len() {
                        out.push(Value::Text(cs[..i].iter().chain(&cs[i + 1..]).collect()));
                    }
                }
            }
            ("list", Value::List(xs)) => {
                if !xs.is_empty() {
                    out.push(Value::List(Vec::new()));
                    out.push(Value::List(xs[..xs.len() / 2].to_vec()));
                    for i in 0..xs.len() {
                        out.push(Value::List(xs[..i].iter().chain(&xs[i + 1..]).cloned().collect()));
                    }
                    for (i, x) in xs.iter().enumerate() {
                        for c in self.shrink(&items[1], x) {
                            let mut ys = xs.clone();
                            ys[i] = c;
                            out.push(Value::List(ys));
                        }
                    }
                }
            }
            ("maybe", Value::Maybe(Some(x))) => {
                out.push(Value::Maybe(None));
                out.extend(self.shrink(&items[1], x).into_iter().map(|c| Value::Maybe(Some(Box::new(c)))));
            }
            ("either", Value::Left(x)) => {
                out = self.shrink(&items[1], x).into_iter().map(|c| Value::Left(Box::new(c))).collect();
            }
            ("either", Value::Right(x)) => {
                out = self.shrink(&items[2], x).into_iter().map(|c| Value::Right(Box::new(c))).collect();
            }
            ("data", Value::Data(tag, fields)) => {
                let ctor = items[2..]
                    .iter()
                    .find(|c| c.items()[1].name() == *tag)
                    .unwrap_or_else(|| panic!("no constructor {tag}"))
                    .items();
                let own = Sexp::List(vec![Sexp::Atom("ref".into()), items[1].clone()]);
                out.push(self.minimal(d));
                // A field of the same type is a smaller value of it.
                out.extend(fields.iter().zip(&ctor[2..]).filter(|(_, fd)| **fd == own).map(|(f, _)| f.clone()));
                for (i, (field, fd)) in fields.iter().zip(&ctor[2..]).enumerate() {
                    for c in self.shrink(fd, field) {
                        let mut fs = fields.clone();
                        fs[i] = c;
                        out.push(Value::Data(tag.clone(), fs));
                    }
                }
            }
            _ => {}
        }
        let own = render(v);
        let mut seen: Vec<String> = Vec::new();
        let mut unique = Vec::new();
        for c in out {
            let text = render(&c);
            if text != own && !seen.contains(&text) {
                seen.push(text);
                unique.push(c);
            }
        }
        unique
    }
}

fn clamp(n: i64, lo: &BigInt, hi: &BigInt) -> BigInt {
    BigInt::from(n).max(lo.clone()).min(hi.clone())
}

fn toward_zero(n: &BigInt, d: i64) -> BigInt {
    let q = n.abs() / d;
    if n.is_negative() { -q } else { q }
}

fn mentions_data(d: &Sexp) -> bool {
    match d {
        Sexp::List(items) => {
            matches!(items.first(), Some(Sexp::Atom(k)) if k == "ref" || k == "data")
                || items.iter().skip(1).any(mentions_data)
        }
        _ => false,
    }
}

/// A value's canonical text, the same on every target.
pub fn render(v: &Value) -> String {
    let all = |xs: &[Value]| xs.iter().map(render).collect::<Vec<_>>().join(", ");
    match v {
        Value::Bool(b) => (if *b { "true" } else { "false" }).into(),
        Value::Integer(n) => n.to_string(),
        Value::Text(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Unit => "()".into(),
        Value::List(xs) => format!("[{}]", all(xs)),
        Value::Maybe(None) => "Nothing".into(),
        Value::Maybe(Some(x)) => format!("Just({})", render(x)),
        Value::Left(x) => format!("Left({})", render(x)),
        Value::Right(x) => format!("Right({})", render(x)),
        Value::Data(tag, fields) => {
            let name = tag.rsplit("::").next().unwrap_or(tag);
            if fields.is_empty() { name.into() } else { format!("{name}({})", all(fields)) }
        }
        Value::Handle(h) => h.label(),
        other => format!("{other:?}"),
    }
}

/// A descriptor text's data types and its last form, the one generated.
pub fn values_from(text: &str) -> (Values, Sexp) {
    let mut forms = read_descriptor(text);
    let table = forms
        .iter()
        .filter(|f| matches!(f, Sexp::List(_)) && f.kind() == "data")
        .map(|f| (f.items()[1].name(), f.clone()))
        .collect();
    let last = forms.pop().expect("a descriptor");
    (Values::new(table), last)
}

// LawSpec's own search over a law's inputs (see LawSpec.Search). The
// failure database keeps a failing case's inputs in the wire encoding, under
// LAWSPEC_FAILURES/inputs, and they are replayed before the law's next
// generated cases. target maximize climbs: it generates inputs from seeds,
// keeps the best-scoring case, and moves its integers (one step, doubling,
// halving the distance to a bound), keeping the best move while the score
// rises and drawing afresh when it does not, checking the law on every case
// it tries (at most 400). A case gives its score, or None when its inputs
// fall outside the law's refinements.
fn search_file(law: &str) -> Option<std::path::PathBuf> {
    let directory = std::env::var("LAWSPEC_FAILURES").ok().filter(|d| !d.is_empty())?;
    let safe: String = law.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    Some(std::path::Path::new(&directory).join("inputs").join(safe + ".json"))
}

pub fn search_remember(law: &str, descriptors: &[&str], values: &[Value]) {
    let Some(file) = search_file(law) else { return };
    let mut inputs = Vec::new();
    for (text, value) in descriptors.iter().zip(values) {
        let (table, d) = values_from(text);
        match net::wire_encode(&table, &d, value) {
            Ok(bytes) => inputs.push(format!("\"{}\"", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())),
            Err(_) => return,
        }
    }
    let quoted = law.replace('\\', "\\\\").replace('"', "\\\"");
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&file, format!("{{\"law\": \"{quoted}\", \"inputs\": [{}]}}", inputs.join(", ")));
}

pub fn search_replay(law: &str, descriptors: &[&str], mut check: impl FnMut(&[Value]) -> Result<Option<f64>>) -> Result<()> {
    let Some(file) = search_file(law) else { return Ok(()) };
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(()) };
    let after = text.find("\"inputs\"").map(|i| &text[i + 8..]).unwrap_or("");
    let encoded: Vec<&str> = after.split('"').skip(1).step_by(2).collect();
    let mut values = Vec::new();
    for (i, d) in descriptors.iter().enumerate() {
        let Some(hex) = encoded.get(i) else { break };
        let bytes: Option<Vec<u8>> = (0..hex.len() / 2).map(|k| u8::from_str_radix(&hex[2 * k..2 * k + 2], 16).ok()).collect();
        let (table, descriptor) = values_from(d);
        match bytes.map(|b| net::wire_decode(&table, &descriptor, &b)) {
            Some(Ok(value)) => values.push(value),
            _ => break,
        }
    }
    if values.len() == descriptors.len() {
        println!("{law}: replaying the failing inputs kept in .lawspec/failures");
        check(&values)?;
    }
    let _ = std::fs::remove_file(&file);
    Ok(())
}

// Each value with one integer moved, wherever it sits in the value.
fn search_moves(table: &Values, d: &Sexp, v: &Value) -> Vec<Value> {
    let d = table.resolve(d);
    let items = d.items();
    match (d.kind(), v) {
        ("int", Value::Integer(x)) => {
            let (lo, hi) = table.bounds(d);
            let two = BigInt::from(2);
            let candidates = [x + 1, x - 1, x * &two, x / &two, x + (&hi - x) / &two, x - (x - &lo) / &two];
            let mut out: Vec<Value> = Vec::new();
            for c in candidates {
                if c >= lo && c <= hi && &c != x && !out.contains(&Value::Integer(c.clone())) {
                    out.push(Value::Integer(c));
                }
            }
            out
        }
        ("list", Value::List(xs)) => (0..xs.len())
            .flat_map(|i| search_moves(table, &items[1], &xs[i]).into_iter().map(move |m| (i, m)))
            .map(|(i, m)| { let mut changed = xs.clone(); changed[i] = m; Value::List(changed) })
            .collect(),
        ("maybe", Value::Maybe(Some(inner))) => search_moves(table, &items[1], inner).into_iter().map(|m| Value::Maybe(Some(Box::new(m)))).collect(),
        ("either", Value::Left(inner)) => search_moves(table, &items[1], inner).into_iter().map(|m| Value::Left(Box::new(m))).collect(),
        ("either", Value::Right(inner)) => search_moves(table, &items[2], inner).into_iter().map(|m| Value::Right(Box::new(m))).collect(),
        ("data", Value::Data(tag, fields)) => {
            let Some(ctor) = items[2..].iter().find(|c| &c.items()[1].name() == tag) else { return Vec::new() };
            let field_forms = &ctor.items()[2..];
            (0..fields.len().min(field_forms.len()))
                .flat_map(|i| search_moves(table, &field_forms[i], &fields[i]).into_iter().map(move |m| (i, m)))
                .map(|(i, m)| { let mut changed = fields.clone(); changed[i] = m; Value::Data(tag.clone(), changed) })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub fn search_climb(law: &str, descriptors: &[&str], mut check: impl FnMut(&[Value]) -> Result<Option<f64>>) -> Result<()> {
    let tables: Vec<(Values, Sexp)> = descriptors.iter().map(|d| values_from(d)).collect();
    let base: u64 = std::env::var("LAWSPEC_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    let generate = |seed: u64| -> Vec<Value> {
        let mut random = SplitMix64::new(seed);
        tables.iter().map(|(table, d)| table.generate(d, &mut random, 8)).collect()
    };
    let mut best: Option<(f64, Vec<Value>)> = None;
    let mut tried = 0;
    let mut attempt = |values: Vec<Value>, best: &mut Option<(f64, Vec<Value>)>, tried: &mut usize| -> Result<bool> {
        if *tried >= 400 {
            return Ok(false);
        }
        *tried += 1;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&values)));
        let score = match outcome {
            Ok(Ok(score)) => score,
            Ok(Err(error)) => { search_remember(law, descriptors, &values); return Err(error) }
            Err(panic) => { search_remember(law, descriptors, &values); std::panic::resume_unwind(panic) }
        };
        match (score, best.as_ref()) {
            (Some(s), Some((b, _))) if !(s > *b) => Ok(false),
            (Some(s), _) => { *best = Some((s, values)); Ok(true) }
            (None, _) => Ok(false),
        }
    };
    for k in 0..10u64 {
        attempt(generate(base.wrapping_add(k)), &mut best, &mut tried)?;
    }
    for step in 0..60u64 {
        if tried >= 400 {
            break;
        }
        let mut rose = false;
        if let Some((_, current)) = best.clone() {
            let mut moves = Vec::new();
            for (i, v) in current.iter().enumerate() {
                for m in search_moves(&tables[i].0, &tables[i].1, v) {
                    let mut changed = current.clone();
                    changed[i] = m;
                    moves.push(changed);
                }
            }
            moves.truncate(24);
            for candidate in moves {
                if attempt(candidate, &mut best, &mut tried)? {
                    rose = true;
                }
            }
        }
        if !rose {
            attempt(generate(base.wrapping_add(1000 + step)), &mut best, &mut tried)?;
        }
    }
    match &best {
        Some((score, _)) => println!("{law}: targeted search tried {tried} case(s); best score {score}"),
        None => println!("{law}: targeted search tried {tried} case(s)"),
    }
    Ok(())
}

/// A score as a float: the targeted search maximizes it.
pub fn search_number(v: &Value) -> f64 {
    match v {
        Value::Integer(n) => n.to_f64().unwrap_or(f64::NAN),
        Value::Float32(x) => *x as f64,
        Value::Float64(x) => *x,
        Value::Rational(r) => r.to_f64().unwrap_or(f64::NAN),
        Value::Decimal(d) => format!("{d:?}").parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// count values generated from one SplitMix64 seed, rendered.
pub fn generated(text: &str, seed: u64, size: i64, count: i64) -> Vec<String> {
    let (values, d) = values_from(text);
    let mut random = SplitMix64::new(seed);
    (0..count).map(|_| render(&values.generate(&d, &mut random, size))).collect()
}

/// The shrink candidates of the first value generated, rendered.
pub fn shrunk(text: &str, seed: u64, size: i64) -> Vec<String> {
    let (values, d) = values_from(text);
    let first = values.generate(&d, &mut SplitMix64::new(seed), size);
    values.shrink(&d, &first).iter().map(render).collect()
}

// Stateful models. A model's spec (see LawSpec.MachineSpec) lists its data
// types, start and commands; the callbacks beside it are the generated
// definitions that call the adapters, the references over the model state,
// preconditions, the abstraction and invariants. A run is generated by
// simulating the pure model, so every command in it is allowed by typestate,
// its precondition and its reference; it is then executed against the
// adapters and every result, abstracted state and invariant is checked. A
// failing run is shrunk by dropping commands and shrinking arguments,
// replaying the model to keep each candidate valid. The draws, candidate
// order and messages follow the Python reference.

/// A generated definition: the context, then its arguments.
pub type ModelCallback = fn(&mut Context, Vec<Value>) -> Result<Value>;

/// A stateful model: its spec and the callbacks in the order the spec lists
/// them. start is [run, model]; each command is (run, reference, when).
pub struct Model {
    pub spec: &'static str,
    pub start: [ModelCallback; 2],
    pub commands: Vec<(ModelCallback, ModelCallback, Option<ModelCallback>)>,
    pub abstract_state: Option<ModelCallback>,
    pub invariants: Vec<ModelCallback>,
}

enum ModelNeed {
    AtLeast(i64),
    Exactly(i64),
}

enum ModelShift {
    By(i64),
    To(i64),
}

struct ModelCommand {
    name: String,
    arguments: Vec<Sexp>,
    state: usize,
    unit: bool,
    needs: Vec<ModelNeed>,
    shifts: Vec<ModelShift>,
    // The argument naming the key the command touches, for per-key checks.
    key: Option<usize>,
    // An actor's restart (restart from): never generated as a step; the
    // injected crash runs it.
    restart: bool,
    run: ModelCallback,
    reference: ModelCallback,
    when: Option<ModelCallback>,
}

fn sexp_i64(s: &Sexp) -> i64 {
    match s {
        Sexp::Int(n) => n.to_i64().unwrap_or_else(|| panic!("index {n} out of range")),
        other => panic!("expected an integer, got {other:?}"),
    }
}

// The fields of a form, by their first atom.
fn form_fields(items: &[Sexp]) -> HashMap<String, Vec<Sexp>> {
    items.iter().map(|f| (f.kind().to_string(), f.items()[1..].to_vec())).collect()
}

impl ModelCommand {
    fn new(form: &Sexp, callbacks: &(ModelCallback, ModelCallback, Option<ModelCallback>)) -> Self {
        let items = form.items();
        let fields = form_fields(&items[2..]);
        let field = |name: &str| fields.get(name).unwrap_or_else(|| panic!("command without {name}")).clone();
        let pair = |f: &Sexp| (f.kind().to_string(), sexp_i64(&f.items()[1]));
        ModelCommand {
            name: items[1].name(),
            arguments: field("arguments"),
            state: sexp_i64(&field("state")[0]) as usize,
            unit: field("unit")[0].name() == "true",
            needs: field("needs")
                .iter()
                .map(|n| match pair(n) {
                    (k, i) if k == "atleast" => ModelNeed::AtLeast(i),
                    (_, i) => ModelNeed::Exactly(i),
                })
                .collect(),
            shifts: field("shifts")
                .iter()
                .map(|s| match pair(s) {
                    (k, d) if k == "by" => ModelShift::By(d),
                    (_, k) => ModelShift::To(k),
                })
                .collect(),
            key: match fields.get("key").and_then(|k| k.first()) {
                Some(Sexp::Int(n)) => Some(n.to_usize().unwrap_or_else(|| panic!("key {n} out of range"))),
                _ => None,
            },
            restart: fields.get("restart").and_then(|r| r.first()).map(Sexp::name).as_deref() == Some("true"),
            run: callbacks.0,
            reference: callbacks.1,
            when: callbacks.2,
        }
    }

    fn admits(&self, indices: &[i64]) -> bool {
        self.needs.iter().zip(indices).all(|(n, i)| match n {
            ModelNeed::AtLeast(k) => i >= k,
            ModelNeed::Exactly(k) => i == k,
        })
    }

    fn shifted(&self, indices: &[i64]) -> Vec<i64> {
        self.shifts
            .iter()
            .zip(indices)
            .map(|(s, i)| match s {
                ModelShift::By(d) => i + d,
                ModelShift::To(k) => *k,
            })
            .collect()
    }
}

// Why a model step did not go through: the model does not allow it, or a
// callback failed.
enum ModelFault {
    Invalid,
    Error(String),
}

// A run: the start's arguments, then each command's index and arguments.
type ModelRun = (Vec<Value>, Vec<(usize, Vec<Value>)>);

struct Machine<'a> {
    model: &'a Model,
    name: String,
    shared: bool,
    values: Values,
    start_indices: Vec<i64>,
    start_arguments: Vec<Sexp>,
    commands: Vec<ModelCommand>,
    invariants: Vec<(String, ModelCallback)>,
    per_key: bool,
    // An actor model: the start's state runs inside an actor, and each
    // command's handler bridge runs inside it in turn.
    actor: bool,
    // An actor's restart command (restart from), whose run and reference an
    // injected crash uses; without one, a crash restarts from the start.
    restart: Option<(ModelCallback, ModelCallback)>,
    // What histories must agree with the model by: linearizable,
    // sequential, causal or eventual.
    consistency: String,
}

/// How a history that is not consistent is described.
fn consistent_words(consistency: &str) -> &'static str {
    match consistency {
        "sequential" => "sequentially consistent",
        "causal" => "causally consistent",
        "eventual" => "eventually consistent",
        _ => "linearizable",
    }
}

// Calls a command: on an actor, its handler bridge (the state first,
// returning Pair reply state, or the state alone for a Unit reply) runs in
// the actor's turn, on this thread, so it can use this thread's Context.
fn call_command(actor: bool, run: ModelCallback, unit: bool, ctx: &mut Context, mut full: Vec<Value>) -> Result<Value> {
    if !actor {
        return run(ctx, full);
    }
    let Some(Value::Handle(handle)) = full.first().cloned() else {
        return Err("an actor's command was not given its actor".into());
    };
    handle.native::<actors::Actor<Value>>()?.call(|state| {
        full[0] = state;
        let out = run(ctx, full)?;
        if unit {
            return Ok((Value::Unit, out));
        }
        match out {
            Value::Data(_, fields) if fields.len() == 2 => {
                let mut fields = fields.into_iter();
                let reply = fields.next().unwrap();
                Ok((reply, fields.next().unwrap()))
            }
            other => Err(format!("a handler returned {}, not a reply and a state", render(&other))),
        }
    })
}

impl<'a> Machine<'a> {
    fn new(model: &'a Model) -> Self {
        let forms = read_descriptor(model.spec);
        let head = forms[0].items();
        let table = forms
            .iter()
            .filter(|f| f.kind() == "data")
            .map(|f| (f.items()[1].name(), f.clone()))
            .collect();
        let start = forms.iter().find(|f| f.kind() == "start").expect("a model start");
        let start_fields = form_fields(&start.items()[1..]);
        let kinds = forms.iter().find(|f| f.kind() == "invariants").expect("model invariants").items()[1..].to_vec();
        let (restarts, commands): (Vec<ModelCommand>, Vec<ModelCommand>) = forms
            .iter()
            .filter(|f| f.kind() == "command")
            .zip(&model.commands)
            .map(|(f, c)| ModelCommand::new(f, c))
            .partition(|c| c.restart);
        Machine {
            model,
            name: head[1].name(),
            shared: head[2].name() == "shared",
            values: Values::new(table),
            start_indices: start_fields.get("indices").map(|is| is.iter().map(sexp_i64).collect()).unwrap_or_default(),
            start_arguments: start_fields.get("arguments").cloned().unwrap_or_default(),
            commands,
            restart: restarts.first().map(|c| (c.run, c.reference)),
            invariants: kinds.iter().map(Sexp::name).zip(model.invariants.iter().copied()).collect(),
            per_key: forms.iter().any(|f| f.kind() == "perkey" && f.items().get(1).map(Sexp::name).as_deref() == Some("true")),
            actor: forms.iter().any(|f| f.kind() == "actor" && f.items().get(1).map(Sexp::name).as_deref() == Some("true")),
            consistency: forms
                .iter()
                .find(|f| f.kind() == "consistency")
                .and_then(|f| f.items().get(1).map(Sexp::name))
                .unwrap_or_else(|| "linearizable".into()),
        }
    }

    /// Starts the system: an actor model's start state runs in a new actor.
    fn start_system(&self, ctx: &mut Context, args: Vec<Value>) -> Result<Value> {
        let state = (self.model.start[0])(ctx, args)?;
        Ok(if self.actor { Value::Handle(Handle::new(actors::Actor::new(state))) } else { state })
    }

    fn run_command(&self, command: &ModelCommand, ctx: &mut Context, full: Vec<Value>) -> Result<Value> {
        call_command(self.actor, command.run, command.unit, ctx, full)
    }

    /// The system state the abstraction and invariants see: an actor's own
    /// state, read between messages.
    fn system_state(&self, state: &Value) -> Result<Value> {
        match (self.actor, state) {
            (true, Value::Handle(handle)) => handle.native::<actors::Actor<Value>>()?.state(),
            _ => Ok(state.clone()),
        }
    }

    /// Whether the model allows every step of a run.
    fn simulate(&self, run: &ModelRun) -> bool {
        let mut ctx = Context::testing();
        let Ok(mut state) = (self.model.start[1])(&mut ctx, run.0.clone()) else {
            return false;
        };
        let mut indices = self.start_indices.clone();
        for (index, args) in &run.1 {
            if *index == self.commands.len() {
                match self.crash_model(&mut ctx, &run.0, state) {
                    Ok(next) => state = next,
                    Err(_) => return false,
                }
                continue;
            }
            let command = &self.commands[*index];
            if !command.admits(&indices) {
                return false;
            }
            match step_model(command, &mut ctx, args, state) {
                Ok((next, _)) => state = next,
                Err(ModelFault::Invalid) => return false,
                Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
            }
            indices = command.shifted(&indices);
        }
        true
    }

    /// An injected crash, on the model: the restart's reference, or the
    /// start's model state.
    fn crash_model(&self, ctx: &mut Context, start_args: &[Value], state: Value) -> Result<Value> {
        match self.restart {
            Some((_, reference)) => reference(ctx, vec![state]),
            None => (self.model.start[1])(ctx, start_args.to_vec()),
        }
    }

    /// An injected crash, on the system: the actor's state is replaced
    /// between messages by the restart's run, or the start's run.
    fn crash_system(&self, ctx: &mut Context, start_args: &[Value], state: &Value) -> Result<()> {
        let Value::Handle(handle) = state else {
            return Err("a crash was injected into a model that is not an actor".into());
        };
        let actor = handle.native::<actors::Actor<Value>>()?;
        match self.restart {
            Some((run, _)) => actor.restart(|s| run(ctx, vec![s])),
            None => {
                let begin = self.model.start[0];
                actor.restart(|_| begin(ctx, start_args.to_vec()))
            }
        }
    }

    fn generate_run(&self, random: &mut SplitMix64, length: u64, size: i64, crashes: bool) -> ModelRun {
        let mut ctx = Context::testing();
        let start_args: Vec<Value> = self.start_arguments.iter().map(|d| self.values.generate(d, random, size)).collect();
        let Ok(mut state) = (self.model.start[1])(&mut ctx, start_args.clone()) else {
            return (start_args, Vec::new());
        };
        let mut indices = self.start_indices.clone();
        let mut steps = Vec::new();
        for _ in 0..length {
            let allowed: Vec<usize> = (0..self.commands.len()).filter(|&i| self.commands[i].admits(&indices)).collect();
            if allowed.is_empty() {
                break;
            }
            let index = allowed[random.below(allowed.len() as u64) as usize];
            // One step in eight of an actor's run is a crash.
            if crashes && self.actor && random.below(8) == 0 {
                match self.crash_model(&mut ctx, &start_args, state.clone()) {
                    Ok(next) => state = next,
                    Err(_) => continue,
                }
                steps.push((self.commands.len(), Vec::new()));
                continue;
            }
            let command = &self.commands[index];
            let args: Vec<Value> = command.arguments.iter().map(|d| self.values.generate(d, random, size)).collect();
            match step_model(command, &mut ctx, &args, state.clone()) {
                Ok((next, _)) => state = next,
                Err(ModelFault::Invalid) => continue,
                Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
            }
            steps.push((index, args));
            indices = command.shifted(&indices);
        }
        (start_args, steps)
    }

    /// None when the system agrees with the model along the run; otherwise
    /// the failing step's number and what went wrong.
    fn execute(&self, run: &ModelRun) -> Option<(usize, String)> {
        let mut step = 0;
        match self.execute_steps(run, &mut step) {
            Ok(None) => None,
            Ok(Some(message)) => Some((step, message)),
            Err(ModelFault::Invalid) => Some((step, "the model does not allow this step".into())),
            Err(ModelFault::Error(e)) => Some((step, format!("raised error: {e}"))),
        }
    }

    fn execute_steps(&self, run: &ModelRun, step: &mut usize) -> std::result::Result<Option<String>, ModelFault> {
        let mut ctx = Context::testing();
        let mut state = self.start_system(&mut ctx, run.0.clone()).map_err(ModelFault::Error)?;
        let mut expected = (self.model.start[1])(&mut ctx, run.0.clone()).map_err(ModelFault::Error)?;
        if let Some(failure) = self.check_state(&mut ctx, &state, &expected)? {
            return Ok(Some(failure));
        }
        for (index, args) in &run.1 {
            *step += 1;
            if *index == self.commands.len() {
                self.crash_system(&mut ctx, &run.0, &state).map_err(ModelFault::Error)?;
                expected = self.crash_model(&mut ctx, &run.0, expected).map_err(|_| ModelFault::Invalid)?;
                if let Some(failure) = self.check_state(&mut ctx, &state, &expected)? {
                    return Ok(Some(failure));
                }
                continue;
            }
            let command = &self.commands[*index];
            let mut full = args.clone();
            full.insert(command.state, state.clone());
            let out = self.run_command(command, &mut ctx, full).map_err(ModelFault::Error)?;
            let result = if self.shared {
                out
            } else {
                let Value::Data(_, mut fields) = out else {
                    return Err(ModelFault::Error(format!("command {} did not return its state", command.name)));
                };
                let Some(last) = fields.pop() else {
                    return Err(ModelFault::Error(format!("command {} did not return its state", command.name)));
                };
                let result = if command.unit { Value::Unit } else { fields.into_iter().next().unwrap_or(last.clone()) };
                state = last;
                result
            };
            let (next, wanted) = step_model(command, &mut ctx, args, expected)?;
            expected = next;
            if !command.unit
                && compare_values(&result, &wanted).map_err(ModelFault::Error)? != std::cmp::Ordering::Equal
            {
                return Ok(Some(format!("returned {}; the model returns {}", render(&result), render(&wanted))));
            }
            if let Some(failure) = self.check_state(&mut ctx, &state, &expected)? {
                return Ok(Some(failure));
            }
        }
        Ok(None)
    }

    fn check_state(
        &self,
        ctx: &mut Context,
        state: &Value,
        expected: &Value,
    ) -> std::result::Result<Option<String>, ModelFault> {
        let state = &self.system_state(state).map_err(ModelFault::Error)?;
        if let Some(abstraction) = self.model.abstract_state {
            let actual = abstraction(ctx, vec![state.clone()]).map_err(ModelFault::Error)?;
            if compare_values(&actual, expected).map_err(ModelFault::Error)? != std::cmp::Ordering::Equal {
                return Ok(Some(format!("the state is {}; the model is {}", render(&actual), render(expected))));
            }
        }
        for (kind, invariant) in &self.invariants {
            let subject = if kind == "model" { expected } else { state };
            let holds = invariant(ctx, vec![subject.clone()]).and_then(|v| v.boolean()).map_err(ModelFault::Error)?;
            if !holds {
                return Ok(Some(format!("an invariant on the {kind} fails")));
            }
        }
        Ok(None)
    }

    fn shrink_candidates(&self, run: &ModelRun) -> Vec<ModelRun> {
        let (start_args, steps) = run;
        let mut out = Vec::new();
        let n = steps.len();
        let mut size = n / 2;
        while size >= 1 {
            for begin in (0..n).step_by(size) {
                let kept = steps[..begin].iter().chain(steps.get(begin + size..).unwrap_or(&[])).cloned().collect();
                out.push((start_args.clone(), kept));
            }
            size /= 2;
        }
        for (k, (index, args)) in steps.iter().enumerate() {
            let Some(command) = self.commands.get(*index) else {
                continue;
            };
            for (j, (d, arg)) in command.arguments.iter().zip(args).enumerate() {
                for c in self.values.shrink(d, arg) {
                    let mut changed = steps.clone();
                    changed[k].1[j] = c;
                    out.push((start_args.clone(), changed));
                }
            }
        }
        for (j, (d, arg)) in self.start_arguments.iter().zip(start_args).enumerate() {
            for c in self.values.shrink(d, arg) {
                let mut changed = start_args.clone();
                changed[j] = c;
                out.push((changed, steps.clone()));
            }
        }
        out
    }

    fn shrink_run(&self, mut run: ModelRun, mut failure: (usize, String), max_shrinks: i64) -> (ModelRun, (usize, String)) {
        let mut budget = max_shrinks;
        'shrinking: while budget > 0 {
            let mut improved = false;
            for candidate in self.shrink_candidates(&run) {
                budget -= 1;
                if budget <= 0 {
                    break 'shrinking;
                }
                if !self.simulate(&candidate) {
                    continue;
                }
                if let Some(found) = self.execute(&candidate) {
                    run = candidate;
                    failure = found;
                    improved = true;
                    break;
                }
            }
            if !improved {
                break;
            }
        }
        (run, failure)
    }

    fn describe_run(&self, run: &ModelRun) -> String {
        let all = |xs: &[Value]| xs.iter().map(render).collect::<Vec<_>>().join(", ");
        let mut parts = vec![format!("start({})", all(&run.0))];
        parts.extend(run.1.iter().map(|(i, args)| {
            format!("{}({})", self.commands.get(*i).map_or("crash", |c| c.name.as_str()), all(args))
        }));
        parts.join("; ")
    }
}

// A reference step over the model state: the next state and the result.
fn step_model(
    command: &ModelCommand,
    ctx: &mut Context,
    args: &[Value],
    state: Value,
) -> std::result::Result<(Value, Value), ModelFault> {
    if let Some(when) = command.when {
        match when(ctx, vec![state.clone()]).and_then(|v| v.boolean()) {
            Ok(true) => {}
            _ => return Err(ModelFault::Invalid),
        }
    }
    let mut full = args.to_vec();
    full.push(state);
    let out = (command.reference)(ctx, full).map_err(|_| ModelFault::Invalid)?;
    if command.unit {
        return Ok((out, Value::Unit));
    }
    match out {
        Value::Data(_, fields) if fields.len() >= 2 => {
            let mut fields = fields.into_iter();
            let result = fields.next().unwrap();
            Ok((fields.next().unwrap(), result))
        }
        other => Err(ModelFault::Error(format!(
            "the reference for {} returned {}, not a pair",
            command.name,
            render(&other)
        ))),
    }
}

/// Checks the system against its model on generated runs (100 cases of up
/// to 20 commands, seeded by LAWSPEC_SEED); a failure names the shortest
/// failing run found.
///
/// A stateful model is checked by running generated command sequences against
/// the system and the model side by side, then shrinking a failing run.
/// ref:DEC-stateful-models-linearizability
pub fn check_model(model: &Model) -> std::result::Result<(), String> {
    let (cases, max_length, max_shrinks) = (100u64, 20u64, 2000i64);
    let seed = std::env::var("LAWSPEC_SEED").ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    let machine = Machine::new(model);
    let mut random = SplitMix64::new(seed);
    for case in 0..cases {
        let length = random.below(max_length + 1);
        let run = machine.generate_run(&mut random, length, 1 + (case % 8) as i64, true);
        if let Some(failure) = machine.execute(&run) {
            let (run, (step, message)) = machine.shrink_run(run, failure, max_shrinks);
            return Err(format!(
                "model {} fails at step {} of {}: {}",
                machine.name,
                step,
                machine.describe_run(&run),
                message
            ));
        }
    }
    Ok(())
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
// positions and model state). Each case runs several times. The draws,
// candidate order and messages follow the Python reference.

const PARALLEL_THREADS: usize = 3;
const PARALLEL_BRANCH: u64 = 5;

// A branch: each command's index and arguments.
type ModelBranch = Vec<(usize, Vec<Value>)>;
type ParallelCase = (ModelRun, Vec<ModelBranch>);
// One call of a branch: when it started, when it returned, and its result.
type ParallelCall = (u64, u64, Value);

fn branch_name(i: usize) -> char {
    (b'A' + i as u8) as char
}

// Positions with thread i's moved one step on.
fn advanced(positions: &[usize], i: usize) -> Vec<usize> {
    let mut next = positions.to_vec();
    next[i] += 1;
    next
}

/// Nothing, a yield, or a sleep of 10 or 100 microseconds.
fn perturb(random: &mut SplitMix64) {
    match random.below(4) {
        0 => {}
        1 => std::thread::yield_now(),
        2 => std::thread::sleep(std::time::Duration::from_micros(10)),
        _ => std::thread::sleep(std::time::Duration::from_micros(100)),
    }
}

impl<'a> Machine<'a> {
    /// The model state after a run, or None when the model does not allow it.
    fn simulate_state(&self, ctx: &mut Context, run: &ModelRun) -> Option<Value> {
        let mut state = (self.model.start[1])(ctx, run.0.clone()).ok()?;
        let mut indices = self.start_indices.clone();
        for (index, args) in &run.1 {
            let command = &self.commands[*index];
            if !command.admits(&indices) {
                return None;
            }
            match step_model(command, ctx, args, state) {
                Ok((next, _)) => state = next,
                Err(ModelFault::Invalid) => return None,
                Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
            }
            indices = command.shifted(&indices);
        }
        Some(state)
    }

    /// Whether the model allows the prefix then every interleaving.
    fn parallel_allowed(&self, prefix: &ModelRun, branches: &[ModelBranch]) -> bool {
        let mut ctx = Context::testing();
        let Some(start) = self.simulate_state(&mut ctx, prefix) else {
            return false;
        };
        let mut seen = std::collections::HashSet::new();
        self.allowed_from(&mut ctx, branches, &mut seen, vec![0; branches.len()], start)
    }

    fn allowed_from(
        &self,
        ctx: &mut Context,
        branches: &[ModelBranch],
        seen: &mut std::collections::HashSet<(Vec<usize>, String)>,
        positions: Vec<usize>,
        state: Value,
    ) -> bool {
        if !seen.insert((positions.clone(), render(&state))) {
            return true;
        }
        for (i, branch) in branches.iter().enumerate() {
            let k = positions[i];
            if k < branch.len() {
                let (index, args) = &branch[k];
                let after = match step_model(&self.commands[*index], ctx, args, state.clone()) {
                    Ok((next, _)) => next,
                    Err(ModelFault::Invalid) => return false,
                    Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
                };
                if !self.allowed_from(ctx, branches, seen, advanced(&positions, i), after) {
                    return false;
                }
            }
        }
        true
    }

    fn generate_branch(&self, random: &mut SplitMix64, mut state: Value, length: u64, size: i64) -> ModelBranch {
        let mut ctx = Context::testing();
        let mut steps = Vec::new();
        for _ in 0..length {
            let index = random.below(self.commands.len() as u64) as usize;
            let command = &self.commands[index];
            let args: Vec<Value> = command.arguments.iter().map(|d| self.values.generate(d, random, size)).collect();
            match step_model(command, &mut ctx, &args, state.clone()) {
                Ok((next, _)) => state = next,
                Err(ModelFault::Invalid) => continue,
                Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
            }
            steps.push((index, args));
        }
        steps
    }

    fn generate_parallel(&self, random: &mut SplitMix64, size: i64, threads: usize, branch_length: u64) -> ParallelCase {
        let length = random.below(4);
        let prefix = self.generate_run(random, length, size, false);
        let Some(state) = self.simulate_state(&mut Context::testing(), &prefix) else {
            return (prefix, vec![Vec::new(); threads]);
        };
        let mut branches = Vec::new();
        for _ in 0..threads {
            let length = 1 + random.below(branch_length);
            branches.push(self.generate_branch(random, state.clone(), length, size));
        }
        // Drop the last step of the longest branch (the first, among equals)
        // until every interleaving is allowed.
        while !self.parallel_allowed(&prefix, &branches) {
            let mut longest = 0;
            for i in 1..threads {
                if branches[i].len() > branches[longest].len() {
                    longest = i;
                }
            }
            branches[longest].pop();
        }
        (prefix, branches)
    }

    /// None when the history is linearizable; otherwise what went wrong.
    fn execute_parallel(&self, case: &ParallelCase, shake: u64) -> Option<String> {
        let (prefix, branches) = case;
        let mut ctx = Context::testing();
        let state = match self.run_prefix(&mut ctx, prefix) {
            Ok(state) => state,
            Err(e) => return Some(format!("the prefix raised error: {e}")),
        };
        // What each thread needs: the call, the command's name and the full
        // arguments, all Send.
        let calls: Vec<Vec<(ModelCallback, bool, String, Vec<Value>)>> = branches
            .iter()
            .map(|branch| {
                branch
                    .iter()
                    .map(|(index, args)| {
                        let command = &self.commands[*index];
                        let mut full = args.clone();
                        full.insert(command.state, state.clone());
                        (command.run, command.unit, command.name.clone(), full)
                    })
                    .collect()
            })
            .collect();
        let clock = std::sync::atomic::AtomicU64::new(0);
        let errors = std::sync::Mutex::new(Vec::<String>::new());
        let actor = self.actor;
        let tick = || clock.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let history: Vec<Vec<ParallelCall>> = std::thread::scope(|scope| {
            let handles: Vec<_> = calls
                .into_iter()
                .enumerate()
                .map(|(i, branch)| {
                    let (tick, errors) = (&tick, &errors);
                    scope.spawn(move || {
                        let mut own = Context::testing();
                        let mut random = SplitMix64::new(shake ^ ((i as u64 + 1).wrapping_mul(0x9E3779B97F4A7C15)));
                        let mut out = Vec::new();
                        for (run, unit, name, full) in branch {
                            perturb(&mut random);
                            let called = tick();
                            let result = match call_command(actor, run, unit, &mut own, full) {
                                Ok(result) => result,
                                Err(e) => {
                                    errors.lock().unwrap_or_else(|p| p.into_inner()).push(format!("{name} raised error: {e}"));
                                    Value::Unit
                                }
                            };
                            out.push((called, tick(), result));
                            perturb(&mut random);
                        }
                        out
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p))).collect()
        });
        if let Some(error) = errors.into_inner().unwrap_or_else(|p| p.into_inner()).into_iter().next() {
            return Some(error);
        }
        let expected = self.simulate_state(&mut ctx, prefix).expect("the model allows the prefix");
        let state = match self.system_state(&state) {
            Ok(state) => state,
            Err(e) => return Some(format!("raised error: {e}")),
        };
        let fin = match self.model.abstract_state {
            Some(abstraction) => match abstraction(&mut ctx, vec![state.clone()]) {
                Ok(v) => Some(v),
                Err(e) => return Some(format!("raised error: {e}")),
            },
            None => None,
        };
        if self.linearizable(&mut ctx, branches, &history, expected, fin.as_ref(), &state) {
            return None;
        }
        let mut observed = Vec::new();
        for (i, branch) in branches.iter().enumerate() {
            for (k, (index, _)) in branch.iter().enumerate() {
                observed.push(format!(
                    "{}: {}() returned {}",
                    branch_name(i),
                    self.commands[*index].name,
                    render(&history[i][k].2)
                ));
            }
        }
        Some(format!("no order of the parallel calls agrees with the model ({})", observed.join("; ")))
    }

    // Starts the system and runs the prefix's commands, giving the state.
    fn run_prefix(&self, ctx: &mut Context, prefix: &ModelRun) -> Result<Value> {
        let state = self.start_system(ctx, prefix.0.clone())?;
        for (index, args) in &prefix.1 {
            let command = &self.commands[*index];
            let mut full = args.clone();
            full.insert(command.state, state.clone());
            self.run_command(command, ctx, full)?;
        }
        Ok(state)
    }

    /// Whether the history linearizes, with the final state and invariants
    /// the model gives. For a set or map whose every call touches one key,
    /// each key's calls are linearized separately (the keys are
    /// independent), one group after another; otherwise all calls at once.
    fn linearizable(
        &self,
        ctx: &mut Context,
        branches: &[ModelBranch],
        history: &[Vec<ParallelCall>],
        expected: Value,
        fin: Option<&Value>,
        state: &Value,
    ) -> bool {
        let finish = |ctx: &mut Context, model_state: &Value| -> bool {
            if let Some(fin) = fin {
                if !matches!(compare_values(fin, model_state), Ok(std::cmp::Ordering::Equal)) {
                    return false;
                }
            }
            for (kind, invariant) in &self.invariants {
                let subject = if kind == "model" { model_state } else { state };
                if !matches!(invariant(ctx, vec![subject.clone()]).and_then(|v| v.boolean()), Ok(true)) {
                    return false;
                }
            }
            true
        };
        if !self.per_key {
            return self.linearize(ctx, branches, history, expected, &mut |c, s| finish(c, s));
        }
        // Each key's calls, thread by thread, ordered by the rendered key.
        let mut groups: std::collections::BTreeMap<String, Vec<(ModelBranch, Vec<ParallelCall>)>> =
            std::collections::BTreeMap::new();
        for (i, branch) in branches.iter().enumerate() {
            for (k, (index, args)) in branch.iter().enumerate() {
                let position = self.commands[*index].key.expect("a per-key command names its key");
                let parts = groups
                    .entry(render(&args[position]))
                    .or_insert_with(|| vec![(Vec::new(), Vec::new()); branches.len()]);
                parts[i].0.push(branch[k].clone());
                parts[i].1.push(history[i][k].clone());
            }
        }
        let mut model_state = expected;
        for parts in groups.values() {
            let (steps, calls): (Vec<ModelBranch>, Vec<Vec<ParallelCall>>) = parts.iter().cloned().unzip();
            let mut ends = Vec::new();
            if !self.linearize(ctx, &steps, &calls, model_state, &mut |_, end| {
                ends.push(end.clone());
                true
            }) {
                return false;
            }
            model_state = ends.swap_remove(0);
        }
        finish(ctx, &model_state)
    }

    /// A Wing-Gong search: linearize, next, a call no pending call on
    /// another thread returned before; memoized on positions and the model
    /// state. finish judges each complete order's final model state.
    ///
    /// ref:wing-gong-linearizability
    fn linearize(
        &self,
        ctx: &mut Context,
        branches: &[ModelBranch],
        history: &[Vec<ParallelCall>],
        expected: Value,
        finish: &mut dyn FnMut(&mut Context, &Value) -> bool,
    ) -> bool {
        // Causal: threads that never message each other see only their own
        // calls, so each thread's results replay alone.
        if self.consistency == "causal" {
            for (i, branch) in branches.iter().enumerate() {
                let mut state = expected.clone();
                for (k, (index, args)) in branch.iter().enumerate() {
                    let command = &self.commands[*index];
                    let (after, wanted) = match step_model(command, ctx, args, state) {
                        Ok(stepped) => stepped,
                        Err(ModelFault::Invalid) => return false,
                        Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
                    };
                    if !command.unit && !matches!(compare_values(&history[i][k].2, &wanted), Ok(std::cmp::Ordering::Equal)) {
                        return false;
                    }
                    state = after;
                }
            }
            return true;
        }
        let mut seen = std::collections::HashSet::new();
        self.visit(ctx, branches, history, &mut seen, vec![0; branches.len()], expected, finish)
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        ctx: &mut Context,
        branches: &[ModelBranch],
        history: &[Vec<ParallelCall>],
        seen: &mut std::collections::HashSet<(Vec<usize>, String)>,
        positions: Vec<usize>,
        model_state: Value,
        finish: &mut dyn FnMut(&mut Context, &Value) -> bool,
    ) -> bool {
        if !seen.insert((positions.clone(), render(&model_state))) {
            return false;
        }
        if positions.iter().zip(branches).all(|(k, b)| *k == b.len()) {
            return finish(ctx, &model_state);
        }
        for (i, branch) in branches.iter().enumerate() {
            let k = positions[i];
            if k == branch.len() {
                continue;
            }
            let called = history[i][k].0;
            // Sequential and eventual drop real time; each thread's own
            // order remains.
            if self.consistency == "linearizable"
                && (0..branches.len()).any(|j| j != i && positions[j] < branches[j].len() && history[j][positions[j]].1 < called)
            {
                continue;
            }
            let (index, args) = &branch[k];
            let command = &self.commands[*index];
            let (after, wanted) = match step_model(command, ctx, args, model_state.clone()) {
                Ok(stepped) => stepped,
                Err(ModelFault::Invalid) => continue,
                Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
            };
            if self.consistency != "eventual"
                && !command.unit
                && !matches!(compare_values(&history[i][k].2, &wanted), Ok(std::cmp::Ordering::Equal))
            {
                continue;
            }
            if self.visit(ctx, branches, history, seen, advanced(&positions, i), after, finish) {
                return true;
            }
        }
        false
    }

    fn parallel_fails(&self, case: &ParallelCase, repeats: u64, shake: u64) -> Option<String> {
        (0..repeats).find_map(|attempt| self.execute_parallel(case, shake.wrapping_add(attempt)))
    }

    fn shrink_parallel(
        &self,
        mut case: ParallelCase,
        mut failure: String,
        repeats: u64,
        mut budget: i64,
        shake: u64,
    ) -> (ParallelCase, String) {
        'shrinking: while budget > 0 {
            let (prefix, branches) = &case;
            let mut candidates: Vec<ParallelCase> = Vec::new();
            for k in 0..prefix.1.len() {
                let mut steps = prefix.1.clone();
                steps.remove(k);
                candidates.push(((prefix.0.clone(), steps), branches.clone()));
            }
            for i in 0..branches.len() {
                for k in 0..branches[i].len() {
                    let mut shorter = branches.clone();
                    shorter[i].remove(k);
                    candidates.push((prefix.clone(), shorter));
                }
            }
            // Then smaller arguments, branch by branch, step by step.
            for i in 0..branches.len() {
                for (k, (index, args)) in branches[i].iter().enumerate() {
                    let command = &self.commands[*index];
                    for (a, (d, arg)) in command.arguments.iter().zip(args).enumerate() {
                        for c in self.values.shrink(d, arg) {
                            let mut changed = branches.clone();
                            changed[i][k].1[a] = c;
                            candidates.push((prefix.clone(), changed));
                        }
                    }
                }
            }
            let mut improved = false;
            for candidate in candidates {
                budget -= 1;
                if budget <= 0 {
                    break 'shrinking;
                }
                if !self.parallel_allowed(&candidate.0, &candidate.1) {
                    continue;
                }
                if let Some(found) = self.parallel_fails(&candidate, repeats, shake) {
                    case = candidate;
                    failure = found;
                    improved = true;
                    break;
                }
            }
            if !improved {
                break;
            }
        }
        (case, failure)
    }

    fn describe_parallel(&self, case: &ParallelCase) -> String {
        let describe = |steps: &ModelBranch| {
            let text = steps
                .iter()
                .map(|(i, args)| {
                    format!("{}({})", self.commands[*i].name, args.iter().map(render).collect::<Vec<_>>().join(", "))
                })
                .collect::<Vec<_>>()
                .join("; ");
            if text.is_empty() { "nothing".to_string() } else { text }
        };
        let parts: Vec<String> =
            case.1.iter().enumerate().map(|(i, b)| format!("{}: {}", branch_name(i), describe(b))).collect();
        let (last, rest) = parts.split_last().map(|(l, r)| (l.as_str(), r)).unwrap_or(("", &[]));
        format!("{}, then {} and {} at the same time", self.describe_run(&case.0), rest.join(", "), last)
    }
}

/// Checks a shared model's histories under concurrency (50 cases of three
/// branches, each case run 10 times, seeded by LAWSPEC_SEED); a failure
/// names the smallest failing case found.
///
/// A shared model promises linearizability unless it names a weaker
/// consistency, so concurrent histories are judged against some sequential
/// order of the calls. ref:herlihy-wing-linearizability
/// ref:DEC-stateful-models-linearizability
pub fn check_model_parallel(model: &Model) -> std::result::Result<(), String> {
    let seed = std::env::var("LAWSPEC_SEED").ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    check_model_parallel_with(model, 50, 10, 300, seed, PARALLEL_THREADS, PARALLEL_BRANCH)
}

/// check_model_parallel with every parameter given.
pub fn check_model_parallel_with(
    model: &Model,
    cases: u64,
    repeats: u64,
    max_shrinks: i64,
    seed: u64,
    threads: usize,
    branch_length: u64,
) -> std::result::Result<(), String> {
    let machine = Machine::new(model);
    let mut random = SplitMix64::new(seed ^ 0x5BD1E995);
    for case_number in 0..cases {
        let case = machine.generate_parallel(&mut random, 1 + (case_number % 8) as i64, threads, branch_length);
        let shake = random.next();
        if let Some(failure) = machine.parallel_fails(&case, repeats, shake) {
            let (case, failure) = machine.shrink_parallel(case, failure, (repeats / 2).max(2), max_shrinks, shake);
            return Err(format!(
                "model {} is not {}: {}: {}",
                machine.name,
                consistent_words(&machine.consistency),
                machine.describe_parallel(&case),
                failure
            ));
        }
    }
    Ok(())
}

// Scenarios: processes that drive a shared model's commands at the same time
// and talk over channels (see LawSpec.Core.Program for the spec). Each
// channel has a queue per direction; a process holds an end of a channel as
// (channel, side), the first branch of a par to use a channel taking side 0.
// A channel end sent over a channel moves to the receiver. Every command's
// call and return are stamped on one counter; the history must linearize
// against the model, and every expect must hold, on each of many schedules.
// The draws, search order and messages follow the Python reference.

// What travels over a channel: a value, or a channel end (channel, side).
enum Carried {
    Value(Value),
    End(usize, usize),
    // The sending side's process has ended.
    Gone,
}

// A queue per direction: side s sends on queues[s] and receives on
// queues[1 - s]. ended[s] once side s's process has ended.
struct ScenarioQueues {
    queues: [std::collections::VecDeque<Carried>; 2],
    ended: [bool; 2],
}

struct ScenarioChannel {
    state: std::sync::Mutex<ScenarioQueues>,
    ready: std::sync::Condvar,
    // In a network run: the channel's two sides as ends on two nodes of a
    // faulty in-memory network, instead of the queues.
    net: Option<NetChannel>,
}

struct NetChannel {
    nodes: [net::Node; 2],
    ends: [net::NetEndpoint; 2],
    done: [std::sync::atomic::AtomicBool; 2],
}

// A vector clock: each process's count of its own events.
type VectorClock = HashMap<usize, u64>;

// A scenario's mailbox: any process sends, one receives. expected is how
// many sends the scenario makes; a process that ends gives up the sends it
// did not make, and a receive with nothing left to come finds Gone instead
// of waiting. Over a network, messages go from a sender node to the
// receiver's node, each send waiting until it is delivered; the senders'
// clocks travel beside the network, in send order.
struct ScenarioMailbox {
    expected: usize,
    state: std::sync::Mutex<MailboxState>,
    ready: std::sync::Condvar,
    net: Option<NetMailbox>,
}

#[derive(Default)]
struct MailboxState {
    items: std::collections::VecDeque<(Carried, VectorClock)>,
    clocks: std::collections::VecDeque<VectorClock>,
    received: usize,
    abandoned: usize,
}

struct NetMailbox {
    nodes: [net::Node; 2],
    inbox: actors::Mailbox<Value>,
    remote: net::RemoteMailbox,
}

/// How many times these acts (not nested pars) send to name.
fn scenario_sends(acts: &[Sexp], name: &str) -> usize {
    acts.iter().filter(|a| a.kind() == "send" && a.items()[1].name() == name).count()
}

/// How many sends to name the whole program makes.
fn all_sends(acts: &[Sexp], name: &str) -> usize {
    let mut total = 0;
    for act in acts {
        if act.kind() == "send" && act.items()[1].name() == name {
            total += 1;
        } else if act.kind() == "par" {
            total += act.items()[1..].iter().map(|b| all_sends(&b.items()[1..], name)).sum::<usize>();
        }
    }
    total
}

// A channel end a process holds: the channel's number and its side.
type ScenarioEnds = HashMap<String, (usize, usize)>;
// One call: the command, its arguments, its result, when it started and
// when it returned, its process, and its process's clock at the call and
// at the return.
type ScenarioCall = (usize, Vec<Value>, Value, u64, u64, usize, VectorClock, VectorClock);

struct ScenarioRun<'m> {
    machine: &'m Machine<'m>,
    commands: HashMap<String, usize>,
    channels: Vec<ScenarioChannel>,
    channel_numbers: HashMap<String, usize>,
    channel_names: Vec<String>,
    // Each value sent carries its sender's clock, kept here in order per
    // channel and sending side.
    stamps: std::sync::Mutex<HashMap<(usize, usize), std::collections::VecDeque<VectorClock>>>,
    state: Value,
    shake: u64,
    clock: std::sync::atomic::AtomicU64,
    history: std::sync::Mutex<Vec<ScenarioCall>>,
    failures: std::sync::Mutex<Vec<String>>,
    // The crashed process (a par's branch) and the act it crashes before.
    victim: Option<(usize, usize)>,
    mailboxes: HashMap<String, ScenarioMailbox>,
}

/// Every process of a par, outermost and first first (not or else).
fn scenario_processes<'s>(acts: &'s [Sexp], found: &mut Vec<&'s Sexp>) {
    for act in acts {
        if act.kind() == "par" {
            for branch in &act.items()[1..] {
                found.push(branch);
                scenario_processes(&branch.items()[1..], found);
            }
        }
    }
}

/// The names an act list sends, receives or sends away, with nested pars.
fn scenario_channels(acts: &[Sexp]) -> Vec<String> {
    let mut names = Vec::new();
    for act in acts {
        let items = act.items();
        match act.kind() {
            "send" => {
                names.push(items[1].name());
                if items[2].kind() == "var" {
                    names.push(items[2].items()[1].name());
                }
            }
            "receive" => names.push(items[1].name()),
            "receiveor" => {
                names.push(items[1].name());
                names.extend(scenario_channels(&items[3].items()[1..]));
            }
            "par" => {
                for branch in &items[1..] {
                    names.extend(scenario_channels(&branch.items()[1..]));
                }
            }
            _ => {}
        }
    }
    names
}

fn scenario_constant(form: &Sexp) -> Value {
    let items = form.items();
    match form.kind() {
        "int" => match &items[1] {
            Sexp::Int(n) => Value::Integer(n.clone()),
            other => Value::Integer(other.name().parse().unwrap_or_else(|_| panic!("expected an integer, got {other:?}"))),
        },
        "text" => Value::Text(items[1].name()),
        "bool" => Value::Bool(items[1].name() == "true"),
        _ => Value::Data(items[1].name(), Vec::new()),
    }
}

impl<'m> ScenarioRun<'m> {
    fn fail(&self, message: String) {
        self.failures.lock().unwrap_or_else(|p| p.into_inner()).push(message);
    }

    fn failed(&self) -> bool {
        !self.failures.lock().unwrap_or_else(|p| p.into_inner()).is_empty()
    }

    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    fn operand(&self, operand: &Sexp, env: &HashMap<String, Value>) -> Option<Value> {
        if operand.kind() == "var" {
            env.get(&operand.items()[1].name()).cloned()
        } else {
            Some(scenario_constant(operand))
        }
    }

    fn end(&self, name: &str, ends: &ScenarioEnds) -> Option<(usize, usize)> {
        let found = ends.get(name).copied();
        if found.is_none() {
            self.fail(format!("{name} is not a channel end this process holds"));
        }
        found
    }

    fn queues(&self, channel: usize) -> std::sync::MutexGuard<'_, ScenarioQueues> {
        self.channels[channel].state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Sends from side; a channel end sent to a process that has ended is
    /// given up.
    fn send_on(&self, channel: usize, side: usize, value: Carried) {
        if let Some(net) = &self.channels[channel].net {
            let value = match value {
                Carried::End(c, s) => Value::Text(format!("{}#{s}", self.channel_names[c])),
                Carried::Value(v) => v,
                Carried::Gone => return,
            };
            if let Err(e) = net.ends[side].send(&value) {
                if !net::is_peer_failed(&e) {
                    self.fail(format!("a send failed: {e}"));
                }
            }
            return;
        }
        let mut queues = self.queues(channel);
        if let (true, Carried::End(c, s)) = (queues.ended[1 - side], &value) {
            let (c, s) = (*c, *s);
            drop(queues);
            return self.gone(c, s);
        }
        queues.queues[side].push_back(value);
        self.channels[channel].ready.notify_all();
    }

    /// side's process has ended: the other side's receives that find
    /// nothing more fail instead of waiting, and channel ends on their way
    /// to side are given up too.
    fn gone(&self, channel: usize, side: usize) {
        if let Some(net) = &self.channels[channel].net {
            if !net.done[side].swap(true, std::sync::atomic::Ordering::SeqCst) {
                net.ends[side].abandon();
            }
            return;
        }
        let mut stranded = Vec::new();
        {
            let mut queues = self.queues(channel);
            if queues.ended[side] {
                return;
            }
            queues.ended[side] = true;
            queues.queues[side].push_back(Carried::Gone);
            while let Some(value) = queues.queues[1 - side].pop_front() {
                match value {
                    Carried::End(c, s) => stranded.push((c, s)),
                    Carried::Gone => {
                        queues.queues[1 - side].push_back(Carried::Gone);
                        break;
                    }
                    Carried::Value(_) => {}
                }
            }
            self.channels[channel].ready.notify_all();
        }
        for (c, s) in stranded {
            self.gone(c, s);
        }
    }

    /// The next thing side receives on channel: None when nothing came in
    /// time.
    fn receive_on(&self, channel: usize, side: usize) -> Option<Carried> {
        if let Some(net) = &self.channels[channel].net {
            return match net.ends[side].receive(Some(std::time::Duration::from_secs(5))) {
                Ok(Value::Text(t)) => match t.rsplit_once('#').and_then(|(c, s)| Some((self.channel_numbers.get(c)?, s.parse::<usize>().ok()?))) {
                    Some((c, s)) => Some(Carried::End(*c, s)),
                    None => Some(Carried::Value(Value::Text(t))),
                },
                Ok(v) => Some(Carried::Value(v)),
                Err(e) if net::is_peer_failed(&e) => Some(Carried::Gone),
                Err(_) => None,
            };
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut waiting = self.queues(channel);
        loop {
            if let Some(v) = waiting.queues[1 - side].pop_front() {
                if let Carried::Gone = v {
                    waiting.queues[1 - side].push_back(Carried::Gone);
                }
                return Some(v);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            waiting = self.channels[channel].ready.wait_timeout(waiting, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    fn mailbox_send(&self, name: &str, value: Carried, clock: VectorClock) -> bool {
        let mailbox = &self.mailboxes[name];
        match &mailbox.net {
            None => {
                mailbox.state.lock().unwrap_or_else(|p| p.into_inner()).items.push_back((value, clock));
                mailbox.ready.notify_all();
                true
            }
            Some(net) => {
                let value = match value {
                    Carried::End(c, s) => Value::Text(format!("{}#{s}", self.channel_names[c])),
                    Carried::Value(v) => v,
                    Carried::Gone => return true,
                };
                mailbox.state.lock().unwrap_or_else(|p| p.into_inner()).clocks.push_back(clock);
                if let Err(e) = net.remote.send(&value) {
                    self.fail(format!("a send to mailbox {name} failed: {e}"));
                    return false;
                }
                mailbox.ready.notify_all();
                true
            }
        }
    }

    fn mailbox_give_up(&self, name: &str, count: usize) {
        let mailbox = &self.mailboxes[name];
        mailbox.state.lock().unwrap_or_else(|p| p.into_inner()).abandoned += count;
        mailbox.ready.notify_all();
    }

    /// The next message and its sender's clock, Gone when no message is
    /// left to come, or None when nothing came in time.
    fn mailbox_receive(&self, name: &str) -> Option<(Carried, VectorClock)> {
        let mailbox = &self.mailboxes[name];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            {
                let mut state = mailbox.state.lock().unwrap_or_else(|p| p.into_inner());
                if mailbox.net.is_none() {
                    if let Some(item) = state.items.pop_front() {
                        state.received += 1;
                        return Some(item);
                    }
                }
                if state.received + state.abandoned >= mailbox.expected {
                    return Some((Carried::Gone, VectorClock::new()));
                }
                if mailbox.net.is_none() {
                    let now = std::time::Instant::now();
                    if now >= deadline {
                        return None;
                    }
                    drop(mailbox.ready.wait_timeout(state, (deadline - now).min(std::time::Duration::from_millis(50))));
                    continue;
                }
            }
            let net = mailbox.net.as_ref().expect("a network mailbox");
            match net.inbox.receive(Some(std::time::Duration::from_millis(20))) {
                Ok(value) => {
                    let clock = {
                        let mut state = mailbox.state.lock().unwrap_or_else(|p| p.into_inner());
                        state.received += 1;
                        state.clocks.pop_front().unwrap_or_default()
                    };
                    let carried = match value {
                        Value::Text(t) => match t.rsplit_once('#').and_then(|(c, s)| Some((self.channel_numbers.get(c)?, s.parse::<usize>().ok()?))) {
                            Some((c, s)) => Carried::End(*c, s),
                            None => Carried::Value(Value::Text(t)),
                        },
                        other => Carried::Value(other),
                    };
                    return Some((carried, clock));
                }
                Err(_) if std::time::Instant::now() >= deadline => return None,
                Err(_) => {}
            }
        }
    }

    fn stamp(&self, channel: usize, side: usize, clock: &VectorClock) {
        self.stamps.lock().unwrap_or_else(|p| p.into_inner()).entry((channel, side)).or_default().push_back(clock.clone());
    }

    fn unstamp(&self, channel: usize, side: usize, clock: &mut VectorClock, me: usize) {
        let sent = self
            .stamps
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&(channel, 1 - side))
            .and_then(|q| q.pop_front())
            .unwrap_or_default();
        for (p, n) in sent {
            let mine = clock.entry(p).or_insert(0);
            *mine = (*mine).max(n);
        }
        *clock.entry(me).or_insert(0) += 1;
    }

    /// Runs a process: true when it finished, false when it failed; either
    /// way, the ends it still holds are given up.
    fn process(
        &self,
        acts: &[Sexp],
        env: &mut HashMap<String, Value>,
        ends: &mut ScenarioEnds,
        random: &mut SplitMix64,
        identity: usize,
        clock: &mut VectorClock,
    ) -> bool {
        let mut sent: HashMap<String, usize> = self.mailboxes.keys().map(|m| (m.clone(), 0)).collect();
        let done = self.steps(acts, env, ends, random, identity, clock, identity, &mut sent);
        for (channel, side) in ends.drain().map(|(_, end)| end).collect::<Vec<_>>() {
            self.gone(channel, side);
        }
        // Sends this process will never make.
        for (name, count) in &sent {
            let missing = scenario_sends(acts, name).saturating_sub(*count);
            if missing > 0 {
                self.mailbox_give_up(name, missing);
            }
        }
        done
    }

    fn steps(
        &self,
        acts: &[Sexp],
        env: &mut HashMap<String, Value>,
        ends: &mut ScenarioEnds,
        random: &mut SplitMix64,
        identity: usize,
        clock: &mut VectorClock,
        me: usize,
        sent: &mut HashMap<String, usize>,
    ) -> bool {
        let mut own = Context::testing();
        for (index, act) in acts.iter().enumerate() {
            if self.failed() {
                return false;
            }
            if self.victim == Some((identity, index)) {
                return false;
            }
            let items = act.items();
            match act.kind() {
                "call" => {
                    let name = items[1].name();
                    let command_index = *self.commands.get(&name).unwrap_or_else(|| panic!("no command {name}"));
                    let command = &self.machine.commands[command_index];
                    let mut args = Vec::new();
                    for o in &items[3..] {
                        match self.operand(o, env) {
                            Some(v) => args.push(v),
                            None => {
                                self.fail(format!("{} is not bound", o.items()[1].name()));
                                return false;
                            }
                        }
                    }
                    let mut full = args.clone();
                    full.insert(command.state, self.state.clone());
                    perturb(random);
                    *clock.entry(me).or_insert(0) += 1;
                    let at_call = clock.clone();
                    let called = self.tick();
                    let result = match self.machine.run_command(command, &mut own, full) {
                        Ok(result) => result,
                        Err(e) => {
                            self.fail(format!("{} raised error: {e}", command.name));
                            return false;
                        }
                    };
                    let returned = self.tick();
                    *clock.entry(me).or_insert(0) += 1;
                    self.history.lock().unwrap_or_else(|p| p.into_inner()).push((
                        command_index,
                        args,
                        result.clone(),
                        called,
                        returned,
                        me,
                        at_call,
                        clock.clone(),
                    ));
                    if items[2] != Sexp::Blank {
                        env.insert(items[2].name(), result);
                    }
                }
                "send" if self.mailboxes.contains_key(&items[1].name()) => {
                    let name = items[1].name();
                    let operand = &items[2];
                    let held = if operand.kind() == "var" { ends.remove(&operand.items()[1].name()) } else { None };
                    let value = match held {
                        Some((c, s)) => Carried::End(c, s),
                        None => match self.operand(operand, env) {
                            Some(v) => Carried::Value(v),
                            None => {
                                self.fail(format!("{} is not bound", operand.items()[1].name()));
                                return false;
                            }
                        },
                    };
                    perturb(random);
                    *clock.entry(me).or_insert(0) += 1;
                    if !self.mailbox_send(&name, value, clock.clone()) {
                        return false;
                    }
                    *sent.entry(name).or_insert(0) += 1;
                }
                "receive" | "receiveor" if self.mailboxes.contains_key(&items[1].name()) => {
                    let name = items[1].name();
                    match self.mailbox_receive(&name) {
                        None => {
                            self.fail(format!("a receive on mailbox {name} waited too long: the processes are blocked"));
                            return false;
                        }
                        Some((Carried::Gone, _)) => {
                            if act.kind() == "receive" {
                                return false;
                            }
                            return self.steps(&items[3].items()[1..], env, ends, random, usize::MAX, clock, me, sent);
                        }
                        Some((value, carried)) => {
                            for (p, n) in carried {
                                let mine = clock.entry(p).or_insert(0);
                                *mine = (*mine).max(n);
                            }
                            *clock.entry(me).or_insert(0) += 1;
                            match value {
                                Carried::End(c, s) => {
                                    ends.insert(items[2].name(), (c, s));
                                }
                                Carried::Value(v) => {
                                    env.insert(items[2].name(), v);
                                }
                                Carried::Gone => {}
                            }
                        }
                    }
                }
                "send" => {
                    let Some((channel, side)) = self.end(&items[1].name(), ends) else { return false };
                    let operand = &items[2];
                    let held = if operand.kind() == "var" { ends.remove(&operand.items()[1].name()) } else { None };
                    let value = match held {
                        Some((c, s)) => Carried::End(c, s),
                        None => match self.operand(operand, env) {
                            Some(v) => Carried::Value(v),
                            None => {
                                self.fail(format!("{} is not bound", operand.items()[1].name()));
                                return false;
                            }
                        },
                    };
                    perturb(random);
                    *clock.entry(me).or_insert(0) += 1;
                    self.stamp(channel, side, clock);
                    self.send_on(channel, side, value);
                }
                "receive" | "receiveor" => {
                    let name = items[1].name();
                    let Some((channel, side)) = self.end(&name, ends) else { return false };
                    let value = self.receive_on(channel, side);
                    if !matches!(value, None | Some(Carried::Gone)) {
                        self.unstamp(channel, side, clock, me);
                    }
                    match value {
                        None => {
                            self.fail(format!("a receive on {name} waited too long: the processes are blocked"));
                            return false;
                        }
                        // The other process ended: or else runs instead of
                        // the rest; without it, this process fails too.
                        Some(Carried::Gone) => {
                            if act.kind() == "receive" {
                                return false;
                            }
                            ends.remove(&name);
                            return self.steps(&items[3].items()[1..], env, ends, random, usize::MAX, clock, me, sent);
                        }
                        Some(Carried::End(c, s)) => {
                            ends.insert(items[2].name(), (c, s));
                        }
                        Some(Carried::Value(v)) => {
                            env.insert(items[2].name(), v);
                        }
                    }
                }
                "par" => {
                    let branches: Vec<&[Sexp]> = items[1..].iter().map(|b| &b.items()[1..]).collect();
                    // Each name's users, in branch order.
                    let mut owned: Vec<(String, Vec<usize>)> = Vec::new();
                    for (i, branch) in branches.iter().enumerate() {
                        for name in scenario_channels(branch) {
                            let at = match owned.iter().position(|(n, _)| *n == name) {
                                Some(at) => at,
                                None => {
                                    owned.push((name, Vec::new()));
                                    owned.len() - 1
                                }
                            };
                            if !owned[at].1.contains(&i) {
                                owned[at].1.push(i);
                            }
                        }
                    }
                    let mut handed = Vec::new();
                    for i in 0..branches.len() {
                        let mut mine = ScenarioEnds::new();
                        for (name, users) in &owned {
                            if let Some(side) = users.iter().position(|u| *u == i) {
                                if let Some(end) = ends.remove(name) {
                                    mine.insert(name.clone(), end);
                                } else if let Some(channel) = self.channel_numbers.get(name) {
                                    mine.insert(name.clone(), (*channel, side));
                                }
                            }
                        }
                        handed.push(mine);
                    }
                    let parent = clock.clone();
                    let outcomes: Vec<(bool, VectorClock)> = std::thread::scope(|scope| {
                        let running: Vec<_> = items[1..]
                            .iter()
                            .zip(handed)
                            .enumerate()
                            .map(|(i, (branch, mut mine))| {
                                let mut env = env.clone();
                                let mut child = parent.clone();
                                let mut random =
                                    SplitMix64::new(self.shake ^ ((i as u64 + 1).wrapping_mul(0x9E3779B97F4A7C15)));
                                let identity = branch as *const Sexp as usize;
                                scope.spawn(move || {
                                    let done = self.process(&branch.items()[1..], &mut env, &mut mine, &mut random, identity, &mut child);
                                    (done, child)
                                })
                            })
                            .collect();
                        running.into_iter().map(|t| t.join().unwrap_or((false, VectorClock::new()))).collect()
                    });
                    let mut finished = Vec::new();
                    for (done, child) in outcomes {
                        finished.push(done);
                        for (p, n) in child {
                            let mine = clock.entry(p).or_insert(0);
                            *mine = (*mine).max(n);
                        }
                    }
                    *clock.entry(me).or_insert(0) += 1;
                    // A failed branch fails the process that ran the par.
                    if finished.contains(&false) {
                        return false;
                    }
                }
                "expect" => {
                    let name = items[1].name();
                    let wanted = scenario_constant(&items[2]);
                    let actual = env.get(&name);
                    let agrees = actual
                        .map(|a| matches!(compare_values(a, &wanted), Ok(std::cmp::Ordering::Equal)))
                        .unwrap_or(false);
                    if !agrees {
                        let shown = actual.map(render).unwrap_or_else(|| "None".into());
                        self.fail(format!("expect {name} = {} failed: {name} is {shown}", render(&wanted)));
                        return false;
                    }
                }
                other => panic!("unknown scenario act {other}"),
            }
        }
        self.victim != Some((identity, acts.len()))
    }
}

// One run of a scenario: its title, and what went wrong if anything did.
fn run_scenario(machine: &Machine, spec: &str, shake: u64, crash: bool, network: bool) -> (String, Option<String>) {
    let forms = read_descriptor(spec);
    let title = forms[0].items()[1].name();
    let names: Vec<String> = forms
        .iter()
        .find(|f| f.kind() == "channels")
        .map(|f| f.items()[1..].iter().map(Sexp::name).collect())
        .unwrap_or_default();
    let body = forms.iter().find(|f| f.kind() == "process").map(|f| &f.items()[1..]).unwrap_or(&[]);
    let mut symbols = Context::testing();
    let start_args: Vec<Value> = machine.start_arguments.iter().map(|d| machine.values.minimal(d)).collect();
    let state = match machine.start_system(&mut symbols, start_args.clone()) {
        Ok(state) => state,
        Err(e) => return (title, Some(format!("the start raised error: {e}"))),
    };
    let expected = match (machine.model.start[1])(&mut symbols, start_args) {
        Ok(expected) => expected,
        Err(e) => return (title, Some(format!("the start raised error: {e}"))),
    };
    // Some runs crash one process of a par before a random act.
    let mut processes = Vec::new();
    scenario_processes(body, &mut processes);
    let victim = if crash && !processes.is_empty() {
        let mut chooser = SplitMix64::new(shake ^ 0xC3A5C85C97CB3127);
        let branch = processes[chooser.below(processes.len() as u64) as usize];
        let acts = branch.items().len() as u64 - 1;
        Some((branch as *const Sexp as usize, chooser.below(acts + 1) as usize))
    } else {
        None
    };
    // Network runs: loss, duplication and delay (which reorders); the
    // channels' numbered, acknowledged frames must hide them all.
    let wire = forms.iter().find(|f| f.kind() == "wire").filter(|_| network);
    let faulty = wire.map(|_| net::MemoryNetwork::new(shake ^ 0x7F4A7C159E3779B9, 0.1, 0.1, std::time::Duration::from_millis(2)));
    let types = Values::new(
        wire.map(|w| w.items()[1..].iter().filter(|f| f.kind() == "data").map(|f| (f.items()[1].name(), f.clone())).collect())
            .unwrap_or_default(),
    );
    let new_channel = |name: &String| {
        let net = match (&faulty, wire) {
            (Some(network), Some(w)) => w.items()[1..].iter().find(|f| f.kind() == "channel" && f.items()[1].name() == *name).map(|f| {
                let steps: Vec<(bool, Sexp)> = f.items()[2..].iter().map(|s| (s.kind() == "send", s.items()[1].clone())).collect();
                let nodes = [net::Node::new(network.insecure_transport_for_tests(&format!("{name}-0"))), net::Node::new(network.insecure_transport_for_tests(&format!("{name}-1")))];
                let deadline = std::time::Duration::from_secs(5);
                let first = nodes[0].listen(name, steps.clone(), types.clone(), deadline).expect("a fresh node");
                let second = nodes[1]
                    .dial(&first.address(), steps.into_iter().map(|(s, d)| (!s, d)).collect(), types.clone(), deadline)
                    .expect("a fresh node");
                NetChannel {
                    nodes,
                    ends: [first, second],
                    done: [std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false)],
                }
            }),
            _ => None,
        };
        ScenarioChannel {
            state: std::sync::Mutex::new(ScenarioQueues {
                queues: [std::collections::VecDeque::new(), std::collections::VecDeque::new()],
                ended: [false, false],
            }),
            ready: std::sync::Condvar::new(),
            net,
        }
    };
    let boxes: Vec<String> = forms
        .iter()
        .filter(|f| f.kind() == "mailboxes")
        .flat_map(|f| f.items()[1..].iter().map(Sexp::name).collect::<Vec<_>>())
        .collect();
    let new_mailbox = |name: &String| {
        let net = match (&faulty, wire) {
            (Some(network), Some(w)) => w.items()[1..].iter().find(|f| f.kind() == "mailbox" && f.items()[1].name() == *name).map(|f| {
                let d = f.items()[2].clone();
                let d = if d.kind() == "end" { descriptor("(text)") } else { d };
                let nodes = [
                    net::Node::new(network.insecure_transport_for_tests(&format!("{name}-owner"))),
                    net::Node::new(network.insecure_transport_for_tests(&format!("{name}-senders"))),
                ];
                let inbox = nodes[0].mailbox(name, d.clone(), types.clone()).expect("a fresh node");
                let remote = nodes[1].remote_mailbox_within(
                    &format!("{}/{name}", nodes[0].address()),
                    d,
                    types.clone(),
                    std::time::Duration::from_secs(5),
                );
                NetMailbox { nodes, inbox, remote }
            }),
            _ => None,
        };
        ScenarioMailbox {
            expected: all_sends(body, name),
            state: std::sync::Mutex::new(MailboxState::default()),
            ready: std::sync::Condvar::new(),
            net,
        }
    };
    let run = ScenarioRun {
        mailboxes: boxes.iter().map(|m| (m.clone(), new_mailbox(m))).collect(),
        machine,
        commands: machine.commands.iter().enumerate().map(|(i, c)| (c.name.clone(), i)).collect(),
        channels: names.iter().map(new_channel).collect(),
        channel_numbers: names.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect(),
        channel_names: names.clone(),
        stamps: std::sync::Mutex::new(HashMap::new()),
        state: state.clone(),
        shake,
        clock: std::sync::atomic::AtomicU64::new(0),
        history: std::sync::Mutex::new(Vec::new()),
        failures: std::sync::Mutex::new(Vec::new()),
        victim,
    };
    let finished = run.process(body, &mut HashMap::new(), &mut ScenarioEnds::new(), &mut SplitMix64::new(shake), 0, &mut VectorClock::new());
    for channel in &run.channels {
        if let Some(net) = &channel.net {
            for node in &net.nodes {
                node.close();
            }
        }
    }
    for mailbox in run.mailboxes.values() {
        if let Some(net) = &mailbox.net {
            for node in &net.nodes {
                node.close();
            }
        }
    }
    if let Some(failure) = run.failures.into_inner().unwrap_or_else(|p| p.into_inner()).into_iter().next() {
        let crashed = if victim.is_some() { " (with a process crashed)" } else { "" };
        return (title, Some(format!("{failure}{crashed}")));
    }
    if !finished && victim.is_none() {
        return (title, Some("a process failed".into()));
    }
    let history = run.history.into_inner().unwrap_or_else(|p| p.into_inner());
    let state = match machine.system_state(&state) {
        Ok(state) => state,
        Err(e) => return (title, Some(format!("raised error: {e}"))),
    };
    let fin = match machine.model.abstract_state {
        Some(abstraction) => match abstraction(&mut symbols, vec![state.clone()]) {
            Ok(v) => Some(v),
            Err(e) => return (title, Some(format!("raised error: {e}"))),
        },
        None => None,
    };
    if !machine.scenario_linearizes(&mut symbols, &history, expected, fin.as_ref(), &state) {
        let mut ordered: Vec<&ScenarioCall> = history.iter().collect();
        ordered.sort_by_key(|h| h.3);
        let observed: Vec<String> = ordered
            .iter()
            .map(|(index, args, result, ..)| {
                format!(
                    "{}({}) returned {}",
                    machine.commands[*index].name,
                    args.iter().map(render).collect::<Vec<_>>().join(", "),
                    render(result)
                )
            })
            .collect();
        return (
            title,
            Some(format!(
                "the calls are not {} with the model ({})",
                consistent_words(&machine.consistency),
                observed.join("; ")
            )),
        );
    }
    (title, None)
}

// Whether call a returned before call b began, as far as messages tell:
// a's return clock is at or below b's call clock everywhere.
fn happened_before(a: &ScenarioCall, b: &ScenarioCall) -> bool {
    a.7.iter().all(|(p, n)| b.6.get(p).copied().unwrap_or(0) >= *n)
}

impl<'a> Machine<'a> {
    /// A Wing-Gong search over the scenario's calls, memoized on the calls
    /// done and the state. Linearizable: next, a call no pending call
    /// returned before (real time). Sequential: next, a call every call that
    /// happened before it (its process's order, and messages) is done.
    /// Causal: each process's results from an order of what happened before
    /// them. Eventual: no results, only the final state.
    fn scenario_linearizes(
        &self,
        ctx: &mut Context,
        history: &[ScenarioCall],
        expected: Value,
        fin: Option<&Value>,
        state: &Value,
    ) -> bool {
        let everything: Vec<usize> = (0..history.len()).collect();
        if self.consistency == "causal" {
            let mut processes: Vec<usize> = history.iter().map(|h| h.5).collect();
            processes.sort_unstable();
            processes.dedup();
            for process in processes {
                let own: Vec<usize> = everything.iter().copied().filter(|i| history[*i].5 == process).collect();
                let members: Vec<usize> = everything
                    .iter()
                    .copied()
                    .filter(|j| own.contains(j) || own.iter().any(|i| i != j && happened_before(&history[*j], &history[*i])))
                    .collect();
                let checked: std::collections::HashSet<usize> = own.into_iter().collect();
                let mut seen = std::collections::HashSet::new();
                if !self.scenario_visit(ctx, history, &members, &checked, false, &mut seen, Vec::new(), expected.clone(), fin, state) {
                    return false;
                }
            }
            return true;
        }
        let checked: std::collections::HashSet<usize> =
            if self.consistency == "eventual" { Default::default() } else { everything.iter().copied().collect() };
        let mut seen = std::collections::HashSet::new();
        self.scenario_visit(ctx, history, &everything, &checked, true, &mut seen, Vec::new(), expected, fin, state)
    }

    fn before(&self, history: &[ScenarioCall], j: usize, i: usize) -> bool {
        if self.consistency == "linearizable" {
            history[j].4 < history[i].3
        } else {
            happened_before(&history[j], &history[i])
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scenario_visit(
        &self,
        ctx: &mut Context,
        history: &[ScenarioCall],
        members: &[usize],
        checked: &std::collections::HashSet<usize>,
        judge_final: bool,
        seen: &mut std::collections::HashSet<(Vec<usize>, String)>,
        done: Vec<usize>,
        model_state: Value,
        fin: Option<&Value>,
        state: &Value,
    ) -> bool {
        let mut key = done.clone();
        key.sort_unstable();
        if !seen.insert((key, render(&model_state))) {
            return false;
        }
        if done.len() == members.len() {
            if !judge_final {
                return true;
            }
            if let Some(fin) = fin {
                if !matches!(compare_values(fin, &model_state), Ok(std::cmp::Ordering::Equal)) {
                    return false;
                }
            }
            return self.invariants.iter().all(|(kind, invariant)| {
                let subject = if kind == "model" { &model_state } else { state };
                matches!(invariant(ctx, vec![subject.clone()]).and_then(|v| v.boolean()), Ok(true))
            });
        }
        for &i in members {
            if done.contains(&i) {
                continue;
            }
            if members.iter().any(|&j| j != i && !done.contains(&j) && self.before(history, j, i)) {
                continue;
            }
            let (index, args, result) = (&history[i].0, &history[i].1, &history[i].2);
            let command = &self.commands[*index];
            let (after, wanted) = match step_model(command, ctx, args, model_state.clone()) {
                Ok(stepped) => stepped,
                Err(ModelFault::Invalid) => continue,
                Err(ModelFault::Error(e)) => panic!("model {}: {e}", self.name),
            };
            if checked.contains(&i) && !command.unit && !matches!(compare_values(result, &wanted), Ok(std::cmp::Ordering::Equal)) {
                continue;
            }
            let mut next = done.clone();
            next.push(i);
            if self.scenario_visit(ctx, history, members, checked, judge_final, seen, next, after, fin, state) {
                return true;
            }
        }
        false
    }
}

/// Runs a scenario on many schedules (30 runs, seeded by LAWSPEC_SEED);
/// a failure names the scenario and what went wrong.
pub fn check_scenario(model: &Model, spec: &str) -> std::result::Result<(), String> {
    let seed = std::env::var("LAWSPEC_SEED").ok().and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
    let machine = Machine::new(model);
    let mut random = SplitMix64::new(seed ^ 0x2545F4914F6CDD1D);
    for run in 0..30 {
        // Every third run crashes one process of a par at a random point, and
        // every third other one sends each channel over a faulty network.
        let (title, failure) = run_scenario(&machine, spec, random.next(), run % 3 == 2, run % 3 == 1);
        if let Some(failure) = failure {
            return Err(format!("scenario {title} fails: {failure}"));
        }
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    fn int(n: i64) -> Value {
        Value::Integer(n.into())
    }
    fn decimal(c: i64, e: i64) -> Value {
        Value::Decimal(Decimal::new(c.into(), e.into()))
    }
    #[test]
    fn data_bridges_check_constructor_identity_arity_and_typed_fields() {
        let tag = "example::type::Pair::Pair";
        let value = construct_data(tag, vec![int(127), Value::Bool(true)]);
        let fields = data_fields(value.clone(), tag, 2).unwrap();
        assert_eq!(i8::from_value(fields[0].clone()).unwrap(), 127);
        assert!(data_fields(value.clone(), "other::type::Pair::Pair", 2).is_err());
        assert!(data_fields(value.clone(), tag, 1).is_err());
        assert!(data_fields(value, tag, 3).is_err());
        assert!(data_fields(Value::List(vec![]), tag, 0).is_err());
        let overflow = data_fields(construct_data(tag, vec![int(128)]), tag, 1).unwrap();
        assert!(i8::from_value(overflow[0].clone()).is_err());
    }

    #[test]
    fn data_equality_preserves_ieee_and_constructor_semantics() {
        let tag = "example::type::Box::Box";
        let boxed = |value| construct_data(tag, vec![value]);
        let nan = boxed(Value::List(vec![Value::Float32(f32::NAN)]));
        assert!(!equal(&nan, &nan).unwrap());
        assert!(equal(&boxed(Value::Float64(-0.0)), &boxed(Value::Float64(0.0))).unwrap());
        assert!(!equal(&boxed(int(1)), &construct_data(tag, vec![int(1), int(2)])).unwrap());
        assert!(
            !equal(
                &boxed(int(1)),
                &construct_data("example::type::Other::Box", vec![int(1)])
            )
            .unwrap()
        );
        assert!(equal(&boxed(int(1)), &boxed(Value::Float64(1.0))).is_err());
    }

    #[test]
    fn nested_data_keeps_absence_raw_units_and_symbol_identity() {
        let tag = "example::type::Record::Record";
        let symbol = Symbol::new("same description".into());
        let record = |symbol| {
            construct_data(
                tag,
                vec![
                    Value::Maybe(Some(Box::new(Value::Maybe(None)))),
                    Value::Utf16Text(vec![0xd800, 0, 0xdfff]),
                    Value::Bytes(vec![0, 0xff]),
                    Value::Symbol(symbol),
                ],
            )
        };
        let original = record(symbol.clone());
        assert!(equal(&original, &record(symbol)).unwrap());
        assert!(!equal(&original, &record(Symbol::new("same description".into()))).unwrap());
        let mut fields = data_fields(original.clone(), tag, 4).unwrap();
        fields[0] = Value::Maybe(None);
        assert!(!equal(&original, &construct_data(tag, fields)).unwrap());
    }

    #[test]
    fn integer_promotion_and_checked_bridges() {
        let result = binary("+", "Integer", 127i8.into_value(), int(1)).unwrap();
        assert_eq!(result, int(128));
        assert!(i8::from_value(result).is_err());
        assert_eq!(
            u64::from_value(Value::Integer(u64::MAX.into())).unwrap(),
            u64::MAX
        );
        assert_eq!(Integer::from(u128::MAX).0, BigInt::from(u128::MAX));
        assert!(u64::from_value(int(-1)).is_err());
        assert!(i32::from_value(Value::Rational(BigRational::new(1.into(), 2.into()))).is_err());
    }
    #[test]
    fn exact_decimal_and_rational_arithmetic() {
        assert!(
            equal(
                &binary("+", "Decimal", decimal(1, -1), decimal(2, -1)).unwrap(),
                &decimal(3, -1)
            )
            .unwrap()
        );
        let half = binary("/", "Rational", int(1), int(2)).unwrap();
        assert_eq!(half, Value::Rational(BigRational::new(1.into(), 2.into())));
        assert!(binary("/", "Rational", int(1), int(0)).is_err());
        assert!(
            Value::Rational(BigRational::new(1.into(), 3.into()))
                .convert("Decimal", 64)
                .is_err()
        );
        assert_eq!(binary("quot", "Integer", int(-7), int(3)).unwrap(), int(-2));
        assert_eq!(binary("rem", "Integer", int(-7), int(3)).unwrap(), int(-1));
    }
    #[test]
    fn decimal_rounds_half_even_at_requested_scale() {
        for (numerator, expected) in [(125, 12), (135, 14), (-125, -12), (-135, -14)] {
            let d = Decimal::round(BigRational::new(numerator.into(), 100.into()), 1).unwrap();
            assert!(equal(&Value::Decimal(d), &decimal(expected, -1)).unwrap());
        }
        let hundred = Decimal::round(BigRational::from_integer(250.into()), -2).unwrap();
        assert!(equal(&Value::Decimal(hundred), &int(200)).unwrap());
    }
    #[test]
    fn ieee_equality_classification_and_precision() {
        assert!(binary("+", "Float32", int(1), Value::Float32(1.0)).is_err());
        assert!(equal(&int(1), &Value::Float32(1.0)).is_err());
        assert!(!equal(&Value::Float32(f32::NAN), &Value::Float32(f32::NAN)).unwrap());
        assert!(equal(&Value::Float64(0.0), &Value::Float64(-0.0)).unwrap());
        assert_eq!(
            helper("isNegativeZero", vec![Value::Float32(-0.0)]).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            binary(
                "+",
                "Float32",
                Value::Float32(16777216.0),
                Value::Float32(1.0)
            )
            .unwrap(),
            Value::Float32(16777216.0)
        );
        assert_eq!(
            binary("/", "Float64", Value::Float64(1.0), Value::Float64(0.0)).unwrap(),
            Value::Float64(f64::INFINITY)
        );
        assert!(
            Value::Float64(f64::INFINITY)
                .convert("Integer", 64)
                .is_err()
        );
        assert_eq!(
            Value::Float64(0.5).convert("Rational", 64).unwrap(),
            Value::Rational(BigRational::new(1.into(), 2.into()))
        );
    }
    #[test]
    fn nested_code_units_cross_native_bridges_without_losing_the_domain() {
        let native: Optional<Nullable<u16>> = Optional::Present(Nullable::Present(0xd800));
        let value = validate(
            native.clone().into_value(),
            "Optional Nullable CodeUnit16",
            64,
        )
        .unwrap();
        assert_eq!(
            value,
            Value::Optional(Some(Box::new(Value::Nullable(Some(Box::new(
                Value::CodeUnit16(0xd800)
            ))))))
        );
        let round_trip = Optional::<Nullable<u16>>::from_value(
            native_value(value, "Optional Nullable CodeUnit16").unwrap(),
        )
        .unwrap();
        assert_eq!(round_trip, native);
        assert!(validate(Value::CodePoint(0x110000), "CodePoint", 64).is_err());
    }
    #[test]
    fn exact_float_conversion_avoids_double_rounding() {
        let n = BigInt::parse_bytes(b"1208925891672223212634113", 10).unwrap();
        let d = BigInt::parse_bytes(b"1208925819614629174706176", 10).unwrap();
        assert_eq!(
            exact_float_bits(&BigRational::new(n, d), true).unwrap(),
            0x3f800001
        );
        assert_eq!(
            exact_float_bits(&BigRational::new(16777217.into(), 16777216.into()), true).unwrap(),
            0x3f800000
        );
        assert_eq!(
            exact_float_bits(&BigRational::new(1.into(), BigInt::one() << 150usize), true).unwrap(),
            0
        );
        assert_eq!(
            exact_float_bits(
                &BigRational::new((-1).into(), BigInt::one() << 150usize),
                true
            )
            .unwrap(),
            0x80000000
        );
        assert_eq!(
            exact_float_bits(
                &BigRational::from_integer(BigInt::one() << 1024usize),
                false
            )
            .unwrap(),
            f64::INFINITY.to_bits()
        );
    }
    #[test]
    fn raw_text_symbols_and_nested_presence_remain_distinct() {
        let raw = Utf16Text(vec![0xd800, 0, 0xdc00]);
        assert_eq!(
            Utf16Text::from_value(raw.clone().into_value()).unwrap(),
            raw
        );
        assert!(CodePoint::from_value(Value::CodePoint(0xd800)).is_ok());
        assert!(CodePoint::from_value(Value::CodePoint(0x110000)).is_err());
        let mut context = Context::default();
        assert_eq!(context.symbol("a", "same"), context.symbol("a", "same"));
        assert_ne!(context.symbol("a", "same"), context.symbol("b", "same"));
        let absent: Optional<Nullable<i8>> = Optional::Undefined;
        let null: Optional<Nullable<i8>> = Optional::Present(Nullable::Null);
        let some: Optional<Nullable<i8>> = Optional::Present(Nullable::Present(0));
        assert_ne!(absent.clone().into_value(), null.clone().into_value());
        assert_ne!(null.clone().into_value(), some.clone().into_value());
        for x in [absent, null, some] {
            assert_eq!(
                Optional::<Nullable<i8>>::from_value(x.clone().into_value()).unwrap(),
                x
            );
        }
    }
}

/// Checks a value an adapter produced against its declared domain: native code
/// may return anything its own type allows, and a law quantifies only over the
/// declared domain. ref:DEC-portable-exact-arithmetic
pub fn validate(value: Value, name: &str, bits: u32) -> Result<Value> {
    if let Some(inner) = name.strip_prefix("List ") {
        return match value {
            Value::List(values) => Ok(Value::List(
                values
                    .into_iter()
                    .map(|x| validate(x, inner, bits))
                    .collect::<Result<_>>()?,
            )),
            _ => Err("expected List".into()),
        };
    }
    if let Some(inner) = name.strip_prefix("Maybe ") {
        return match value {
            Value::Maybe(value) => Ok(Value::Maybe(
                value
                    .map(|x| validate(*x, inner, bits).map(Box::new))
                    .transpose()?,
            )),
            _ => Err("expected Maybe".into()),
        };
    }
    if let Some(arguments) = name.strip_prefix("Either ") {
        let (left, right) = either_types(arguments)?;
        return match value {
            Value::Left(x) => Ok(Value::Left(Box::new(validate(*x, left, bits)?))),
            Value::Right(x) => Ok(Value::Right(Box::new(validate(*x, right, bits)?))),
            _ => Err("expected Either".into()),
        };
    }
    if let Some(inner) = name.strip_prefix("Nullable ") {
        return match value {
            Value::Nullable(None) => Ok(Value::Nullable(None)),
            Value::Nullable(Some(x)) => {
                Ok(Value::Nullable(Some(Box::new(validate(*x, inner, bits)?))))
            }
            _ => Err("expected Nullable".into()),
        };
    }
    if let Some(inner) = name.strip_prefix("Optional ") {
        return match value {
            Value::Optional(None) => Ok(Value::Optional(None)),
            Value::Optional(Some(x)) => {
                Ok(Value::Optional(Some(Box::new(validate(*x, inner, bits)?))))
            }
            _ => Err("expected Optional".into()),
        };
    }
    if name == "Bytes" {
        if let Value::List(values) = value {
            return Ok(Value::Bytes(
                values
                    .into_iter()
                    .map(u8::from_value)
                    .collect::<Result<_>>()?,
            ));
        }
    }
    if name == "CodeUnit16" {
        return match value {
            Value::CodeUnit16(x) => Ok(Value::CodeUnit16(x)),
            x => Ok(Value::CodeUnit16(u16::from_value(x)?)),
        };
    }
    let valid = match (&value, name) {
        (Value::Integer(_), _) => return value.convert(name, bits),
        (Value::Decimal(d), "Decimal") => {
            d.ratio()?;
            true
        }
        (Value::Rational(_), "Rational")
        | (Value::Float32(_), "Float32")
        | (Value::Float64(_), "Float64")
        | (Value::Complex32(_), "Complex64")
        | (Value::Complex64(_), "Complex128")
        | (Value::Bool(_), "Bool")
        | (Value::Char(_), "Char")
        | (Value::CodeUnit16(_), "CodeUnit16")
        | (Value::Text(_), "Text")
        | (Value::Bytes(_), "Bytes")
        | (Value::Utf16Text(_), "Utf16Text")
        | (Value::Symbol(_), "Symbol")
        | (Value::Unit, "Unit")
        | (Value::Null, "Null")
        | (Value::Undefined, "Undefined") => true,
        (Value::CodePoint(c), "CodePoint") => *c <= 0x10ffff,
        (Value::CodePointText(cs), "CodePointText") => cs.iter().all(|c| *c <= 0x10ffff),
        _ => false,
    };
    if valid {
        Ok(value)
    } else {
        Err(format!("invalid {name} representation returned by adapter"))
    }
}

/// Type-directed native bridge for nested domains that share a Rust primitive.
pub fn native_value(value: Value, name: &str) -> Result<Value> {
    if let Some(inner) = name.strip_prefix("List ") {
        return match value {
            Value::List(values) => Ok(Value::List(
                values
                    .into_iter()
                    .map(|x| native_value(x, inner))
                    .collect::<Result<_>>()?,
            )),
            _ => Err("expected List".into()),
        };
    }
    if let Some(inner) = name.strip_prefix("Maybe ") {
        return match value {
            Value::Maybe(value) => Ok(Value::Maybe(
                value
                    .map(|x| native_value(*x, inner).map(Box::new))
                    .transpose()?,
            )),
            _ => Err("expected Maybe".into()),
        };
    }
    if let Some(arguments) = name.strip_prefix("Either ") {
        let (left, right) = either_types(arguments)?;
        return match value {
            Value::Left(x) => Ok(Value::Left(Box::new(native_value(*x, left)?))),
            Value::Right(x) => Ok(Value::Right(Box::new(native_value(*x, right)?))),
            _ => Err("expected Either".into()),
        };
    }
    if let Some(inner) = name.strip_prefix("Nullable ") {
        return match value {
            Value::Nullable(Some(x)) => {
                Ok(Value::Nullable(Some(Box::new(native_value(*x, inner)?))))
            }
            Value::Nullable(None) => Ok(Value::Nullable(None)),
            _ => Err("expected Nullable".into()),
        };
    }
    if let Some(inner) = name.strip_prefix("Optional ") {
        return match value {
            Value::Optional(Some(x)) => {
                Ok(Value::Optional(Some(Box::new(native_value(*x, inner)?))))
            }
            Value::Optional(None) => Ok(Value::Optional(None)),
            _ => Err("expected Optional".into()),
        };
    }
    match (value, name) {
        (Value::CodeUnit16(x), "CodeUnit16") => Ok(Value::Integer(x.into())),
        (v, _) => Ok(v),
    }
}

#[cfg(test)]
mod handle_tests {
    use super::*;

    #[test]
    fn handles_are_equal_by_identity_and_have_no_order() {
        let a = Value::Handle(Handle::new(std::sync::Mutex::new(vec![1i32])));
        let b = Value::Handle(Handle::new(std::sync::Mutex::new(vec![1i32])));
        assert!(equal(&a, &a.clone()).unwrap());
        assert!(!equal(&a, &b).unwrap());
        assert_eq!(compare_values(&a, &a.clone()).unwrap(), std::cmp::Ordering::Equal);
        assert!(compare_values(&a, &b).unwrap_err().contains("no portable order"));
        fn send<T: Send + Sync>(_: &T) {}
        send(&a);
    }

    #[test]
    fn a_schema_names_and_passes_handles() {
        let schema = Schema::new(vec![DataSchema { name: "unit::type::Jobs", parameters: 0, constructors: vec![] }])
            .unwrap()
            .with_handles(&["unit::type::Jobs"]);
        let ty = TypeRef::named("unit::type::Jobs", vec![]);
        let handle = Handle::new(7i32);
        let checked = schema.validate(Value::Handle(handle.clone()), &ty, 64).unwrap();
        assert_eq!(checked, Value::Handle(handle.clone()));
        assert!(render(&checked).starts_with("Jobs#"));
        assert_eq!(render(&checked), render(&Value::Handle(handle.clone())));
        assert_eq!(*handle.native::<i32>().unwrap(), 7);
        assert!(handle.native::<String>().is_err());
        assert!(schema.validate(Value::Unit, &ty, 64).is_err());
    }
}

#[cfg(test)]
mod collection_tests {
    use super::*;

    #[test]
    fn nested_collections_preserve_code_units_and_bytes() {
        let native: Vec<Option<Either<u16, Vec<u8>>>> = vec![
            None,
            Some(Either::Left(0xd800)),
            Some(Either::Right(vec![0, 255])),
        ];
        let domain = "List Maybe Either CodeUnit16 Bytes";
        let value = validate(native.clone().into_value(), domain, 64).unwrap();
        let Value::List(ref items) = value else {
            panic!("expected List");
        };
        assert_eq!(items[0], Value::Maybe(None));
        assert_eq!(
            items[1],
            Value::Maybe(Some(Box::new(Value::Left(Box::new(Value::CodeUnit16(
                0xd800
            ))))))
        );
        assert_eq!(
            items[2],
            Value::Maybe(Some(Box::new(Value::Right(Box::new(Value::Bytes(vec![
                0, 255
            ]))))))
        );
        let actual =
            Vec::<Option<Either<u16, Vec<u8>>>>::from_value(native_value(value, domain).unwrap())
                .unwrap();
        assert_eq!(actual, native);
    }

    #[test]
    fn structural_equality_preserves_tags_ieee_rules_and_length() {
        let nan = Value::List(vec![Value::Float32(f32::NAN)]);
        assert!(!equal(&nan, &nan).unwrap());
        assert!(
            equal(
                &Value::List(vec![Value::Float64(-0.0)]),
                &Value::List(vec![Value::Float64(0.0)])
            )
            .unwrap()
        );
        assert!(
            !equal(
                &Value::Left(Box::new(Value::Bool(true))),
                &Value::Right(Box::new(Value::Bool(true)))
            )
            .unwrap()
        );
        assert!(!equal(&Value::List(vec![]), &Value::List(vec![Value::Unit])).unwrap());
        assert!(
            !equal(
                &Value::Maybe(None),
                &Value::Maybe(Some(Box::new(Value::Maybe(None))))
            )
            .unwrap()
        );
    }

    #[test]
    fn collection_domains_reject_bad_payloads_and_do_not_collapse_presence() {
        assert!(
            validate(
                Value::List(vec![Value::Integer(128.into())]),
                "List Int8",
                64
            )
            .is_err()
        );
        assert!(validate(Value::Nullable(None), "Maybe Bool", 64).is_err());
        assert!(validate(Value::Maybe(None), "Nullable Bool", 64).is_err());
        assert!(
            validate(
                Value::Left(Box::new(Value::Bool(true))),
                "Either Int8 Bool",
                64
            )
            .is_err()
        );
        assert!(construct("List::Cons", vec![Value::Bool(true), Value::Bool(false)]).is_err());
        assert_eq!(
            validate(vec![0u8, 255].into_value(), "Bytes", 64).unwrap(),
            Value::Bytes(vec![0, 255])
        );
        assert!(matches!(
            validate(vec![0u8, 255].into_value(), "List UInt8", 64).unwrap(),
            Value::List(_)
        ));
        assert_eq!(
            either_types("Either Bool Unit List Maybe Int8").unwrap(),
            ("Either Bool Unit", "List Maybe Int8")
        );
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    fn ty(name: &'static str) -> TypeRef {
        TypeRef::named(name, vec![])
    }

    fn box_type(element: TypeRef) -> TypeRef {
        TypeRef::named("example::Box", vec![element])
    }

    fn schema() -> Schema {
        Schema::new(vec![
            DataSchema {
                name: "example::Box",
                parameters: 1,
                constructors: vec![ConstructorSchema {
                    tag: "example::Box::Box",
                    fields: vec![TypeRef::Parameter(0)],
                }],
            },
            DataSchema {
                name: "example::Tree",
                parameters: 1,
                constructors: vec![
                    ConstructorSchema {
                        tag: "example::Tree::Leaf",
                        fields: vec![TypeRef::Parameter(0)],
                    },
                    ConstructorSchema {
                        tag: "example::Tree::Branch",
                        fields: vec![TypeRef::named(
                            "List",
                            vec![TypeRef::named("example::Tree", vec![TypeRef::Parameter(0)])],
                        )],
                    },
                ],
            },
        ])
        .unwrap()
    }

    #[test]
    fn validates_recursive_instantiated_fields_and_reports_the_payload_path() {
        let schema = schema();
        let tree = TypeRef::named("example::Tree", vec![ty("Int8")]);
        let value = |integer: i64| {
            construct_data(
                "example::Tree::Branch",
                vec![Value::List(vec![construct_data(
                    "example::Tree::Leaf",
                    vec![Value::Integer(integer.into())],
                )])],
            )
        };
        assert_eq!(schema.validate(value(127), &tree, 64).unwrap(), value(127));
        let error = schema.validate(value(128), &tree, 64).unwrap_err();
        assert!(error.contains("example::Tree::Branch field 0"));
        assert!(error.contains("List element 0"));
        assert!(error.contains("example::Tree::Leaf field 0"));
        assert!(
            schema
                .validate(
                    construct_data("example::Box::Box", vec![Value::Integer(1.into())]),
                    &tree,
                    64,
                )
                .is_err()
        );
        assert!(
            schema
                .validate(construct_data("example::Tree::Leaf", vec![]), &tree, 64)
                .is_err()
        );
    }

    #[test]
    fn preserves_nested_raw_domains_across_native_bridges() {
        let schema = schema();
        let element = box_type(ty("CodeUnit16"));
        let logical = TypeRef::named("Maybe", vec![TypeRef::named("List", vec![element])]);
        let value = Value::Maybe(Some(Box::new(Value::List(vec![construct_data(
            "example::Box::Box",
            vec![Value::CodeUnit16(0xd800)],
        )]))));
        let native = schema.native_value(value.clone(), &logical, 64).unwrap();
        assert_ne!(value, native);
        assert_eq!(schema.validate(native, &logical, 64).unwrap(), value);
        let bytes = box_type(ty("Bytes"));
        let native_bytes = construct_data("example::Box::Box", vec![vec![0u8, 255u8].into_value()]);
        assert_eq!(
            schema.validate(native_bytes, &bytes, 64).unwrap(),
            construct_data("example::Box::Box", vec![Value::Bytes(vec![0, 255])])
        );
        assert!(schema.validate(Value::Maybe(None), &logical, 64).is_ok());
    }

    #[test]
    fn checks_machine_width_and_scalar_domains_inside_user_values() {
        let schema = schema();
        let value = construct_data(
            "example::Box::Box",
            vec![Value::Integer((1u64 << 32).into())],
        );
        let target = box_type(ty("UIntSize"));
        assert!(schema.validate(value.clone(), &target, 32).is_err());
        assert!(schema.validate(value.clone(), &target, 64).is_ok());
        assert!(schema.validate(value.clone(), &target, 16).is_err());
        assert!(schema.native_value(value, &target, 16).is_err());
        assert!(
            schema
                .validate(
                    construct_data("example::Box::Box", vec![Value::Float32(1.0)]),
                    &box_type(ty("Int8")),
                    64,
                )
                .is_err()
        );
    }

    #[test]
    fn rejects_corrupt_schemas_and_unapplied_types() {
        let definition = |name, field| DataSchema {
            name,
            parameters: 1,
            constructors: vec![ConstructorSchema {
                tag: "unique::Tag",
                fields: vec![field],
            }],
        };
        assert!(Schema::new(vec![definition("Bad", TypeRef::Parameter(1))]).is_err());
        assert!(Schema::new(vec![definition("Bad", ty("Unknown"))]).is_err());
        assert!(Schema::new(vec![definition("Bad", ty("Maybe"))]).is_err());
        assert!(Schema::new(vec![definition("Bool", TypeRef::Parameter(0))]).is_err());
        assert!(
            Schema::new(vec![
                definition("First", TypeRef::Parameter(0)),
                definition("Second", TypeRef::Parameter(0)),
            ])
            .is_err()
        );
        let schema = schema();
        let value = construct_data("example::Box::Box", vec![Value::Bool(true)]);
        assert!(
            schema
                .validate(value.clone(), &ty("example::Box"), 64)
                .is_err()
        );
        assert!(schema.validate(value, &TypeRef::Parameter(0), 64).is_err());
    }
}

#[cfg(test)]
mod constructor_contract_tests {
    use super::*;

    fn ty(name: &'static str) -> TypeRef {
        TypeRef::named(name, vec![])
    }

    fn schema(predicates: Vec<FieldPredicate>, element: &'static str) -> Schema {
        Schema::with_contracts(
            vec![DataSchema {
                name: "Box",
                parameters: 1,
                constructors: vec![ConstructorSchema {
                    tag: "Box::Box",
                    fields: vec![TypeRef::Parameter(0)],
                }],
            }],
            vec![ConstructorContract {
                tag: "Box::Box",
                predicates,
            }],
        )
        .and_then(|schema| {
            schema.check_type(&TypeRef::named("Box", vec![ty(element)]), 0)?;
            Ok(schema)
        })
        .unwrap()
    }

    fn value(field: Value) -> Value {
        construct_data("Box::Box", vec![field])
    }

    fn positive(
        _: &Schema,
        arguments: &[TypeRef],
        fields: &[Value],
        bits: u32,
        _: &mut Context,
    ) -> Result<bool> {
        assert_eq!(arguments, &[ty("IntSize")]);
        assert!(matches!(bits, 32 | 64));
        let [Value::Integer(n)] = fields else {
            return Err("expected logical integer".into());
        };
        Ok(n > &BigInt::zero())
    }

    #[test]
    fn contracts_check_native_and_logical_boundaries_at_both_widths() {
        let schema = schema(vec![positive], "IntSize");
        let ty = TypeRef::named("Box", vec![ty("IntSize")]);
        for bits in [32, 64] {
            let good = value(Value::Integer(1.into()));
            let bad = value(Value::Integer(0.into()));
            assert_eq!(schema.validate(good.clone(), &ty, bits).unwrap(), good);
            assert_eq!(schema.native_value(good.clone(), &ty, bits).unwrap(), good);
            assert!(schema.validate(bad.clone(), &ty, bits).is_err());
            assert!(schema.native_value(bad.clone(), &ty, bits).is_err());
            assert!(matches!(
                schema
                    .check_with_context(bad, &ty, bits, &mut Context::default())
                    .unwrap(),
                ValueCheck::Rejected(_)
            ));
        }
        let large = value(Value::Integer(BigInt::from(1u64 << 40)));
        assert!(schema.validate(large.clone(), &ty, 32).is_err());
        assert!(schema.validate(large, &ty, 64).is_ok());
    }

    #[test]
    fn predicates_short_circuit_and_do_not_hide_evaluator_errors() {
        fn reject(_: &Schema, _: &[TypeRef], _: &[Value], _: u32, _: &mut Context) -> Result<bool> {
            Ok(false)
        }
        fn broken(_: &Schema, _: &[TypeRef], _: &[Value], _: u32, _: &mut Context) -> Result<bool> {
            Err("broken predicate".into())
        }
        let ty = TypeRef::named("Box", vec![ty("Bool")]);
        let good = value(Value::Bool(true));
        assert!(matches!(
            schema(vec![reject, broken], "Bool")
                .check_with_context(good.clone(), &ty, 64, &mut Context::default())
                .unwrap(),
            ValueCheck::Rejected(_)
        ));
        let error = schema(vec![broken], "Bool")
            .check_with_context(good, &ty, 64, &mut Context::default())
            .unwrap_err();
        assert!(error.contains("field refinement 1: broken predicate"));
        assert!(
            schema(vec![reject], "Bool")
                .check_with_context(
                    value(Value::Text("wrong".into())),
                    &ty,
                    64,
                    &mut Context::default()
                )
                .is_err()
        );
    }

    #[test]
    fn native_code_unit_conversion_follows_contract_validation() {
        fn surrogate(
            _: &Schema,
            _: &[TypeRef],
            fields: &[Value],
            _: u32,
            _: &mut Context,
        ) -> Result<bool> {
            match fields {
                [Value::CodeUnit16(value)] => Ok(*value == 0xd800),
                _ => Err("predicate must receive a logical code unit".into()),
            }
        }
        let schema = schema(vec![surrogate], "CodeUnit16");
        let ty = TypeRef::named("Box", vec![ty("CodeUnit16")]);
        let logical = value(Value::CodeUnit16(0xd800));
        let native = schema.native_value(logical.clone(), &ty, 64).unwrap();
        assert_eq!(native, value(Value::Integer(0xd800.into())));
        assert_eq!(schema.validate(native, &ty, 64).unwrap(), logical);
        assert!(
            schema
                .native_value(value(Value::CodeUnit16(0)), &ty, 64)
                .is_err()
        );
    }

    #[test]
    fn nested_symbols_share_context_and_keep_rejection_paths() {
        fn fixture(
            _: &Schema,
            _: &[TypeRef],
            fields: &[Value],
            _: u32,
            context: &mut Context,
        ) -> Result<bool> {
            equal(
                &fields[0],
                &Value::Symbol(context.symbol("fixture", "same")),
            )
        }
        let schema = schema(vec![fixture], "Symbol");
        let mut context = Context::default();
        let box_type = TypeRef::named("Box", vec![ty("Symbol")]);
        let expected = value(Value::Symbol(context.symbol("fixture", "same")));
        let cases = vec![
            (
                TypeRef::named("List", vec![box_type.clone()]),
                Value::List(vec![expected.clone()]),
            ),
            (
                TypeRef::named("Maybe", vec![box_type.clone()]),
                Value::Maybe(Some(Box::new(expected.clone()))),
            ),
            (
                TypeRef::named("Nullable", vec![box_type.clone()]),
                Value::Nullable(Some(Box::new(expected.clone()))),
            ),
            (
                TypeRef::named("Optional", vec![box_type.clone()]),
                Value::Optional(Some(Box::new(expected.clone()))),
            ),
            (
                TypeRef::named("Either", vec![box_type.clone(), box_type.clone()]),
                Value::Right(Box::new(expected.clone())),
            ),
        ];
        for (ty, logical) in cases {
            let native = schema
                .native_value_with_context(logical.clone(), &ty, 64, &mut context)
                .unwrap();
            let restored = schema
                .validate_with_context(native, &ty, 64, &mut context)
                .unwrap();
            assert!(equal(&logical, &restored).unwrap());
            assert!(matches!(
                schema
                    .check_with_context(logical, &ty, 64, &mut Context::default())
                    .unwrap(),
                ValueCheck::Rejected(_)
            ));
        }
        let list = TypeRef::named("List", vec![box_type.clone()]);
        let outcome = schema
            .check_with_context(
                Value::List(vec![expected]),
                &list,
                64,
                &mut Context::default(),
            )
            .unwrap();
        let ValueCheck::Rejected(message) = outcome else {
            panic!("expected rejection")
        };
        assert!(message.contains("List element 0"));
        assert!(matches!(
            schema
                .check_with_context(
                    value(Value::Symbol(Symbol::new("same".into()))),
                    &box_type,
                    64,
                    &mut context
                )
                .unwrap(),
            ValueCheck::Rejected(_)
        ));
    }

    #[test]
    fn contract_metadata_rejects_unknown_and_duplicate_constructor_tags() {
        assert!(
            Schema::with_contracts(
                vec![],
                vec![ConstructorContract {
                    tag: "Unknown",
                    predicates: vec![positive],
                }]
            )
            .is_err()
        );
        let definitions = vec![DataSchema {
            name: "Box",
            parameters: 0,
            constructors: vec![ConstructorSchema {
                tag: "Box",
                fields: vec![],
            }],
        }];
        assert!(
            Schema::with_contracts(
                definitions,
                vec![
                    ConstructorContract {
                        tag: "Box",
                        predicates: vec![]
                    },
                    ConstructorContract {
                        tag: "Box",
                        predicates: vec![]
                    },
                ]
            )
            .is_err()
        );
    }
}

/// Channels and processes for the typed channel ends LawSpec generates from
/// protocols (lawspec_sessions). Generated ends move one Endpoint from step
/// to step; implementation code only opens, spawns and joins.
pub mod sessions {
    use std::any::Any;
    use std::collections::VecDeque;
    use std::sync::{Arc, Condvar, Mutex};

    /// A value in flight. Generated ends fix each step's type, so a receive
    /// knows what it takes off the channel.
    pub type Message = Box<dyn Any + Send>;

    /// One end of a two-ended channel.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Side {
        First,
        Second,
    }

    impl Side {
        fn index(self) -> usize {
            match self {
                Side::First => 0,
                Side::Second => 1,
            }
        }

        fn other(self) -> Side {
            match self {
                Side::First => Side::Second,
                Side::Second => Side::First,
            }
        }
    }

    /// A receive whose other end gave up: its process failed (a panic drops
    /// its ends), it was dropped, or abandon was called. try_receive returns
    /// it; receive panics with it, failing this process too.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct PeerFailed;

    impl std::fmt::Display for PeerFailed {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("the other end gave up the conversation (its process failed or abandoned it)")
        }
    }

    impl std::error::Error for PeerFailed {}

    /// What carries messages between a channel's two ends. The local channel
    /// is in memory; a networked transport can implement the same interface.
    pub trait Transport: Send + Sync {
        /// Sends a message from `side` to the other end.
        fn send(&self, side: Side, message: Message);
        /// Takes the next message sent to `side`, waiting for one; PeerFailed
        /// once the other end has closed and nothing it sent is left.
        fn receive(&self, side: Side) -> Result<Message, PeerFailed>;
        /// Closes `side`: the other end's pending receives fail.
        fn close(&self, side: Side);
        /// For an unused network channel end, the address another node
        /// takes it over from (closing it afterwards does not give it up);
        /// None for a local channel.
        fn hand_over(&self) -> Option<String> {
            None
        }
    }

    /// An in-memory channel: one queue per direction.
    #[derive(Default)]
    pub struct LocalChannel {
        state: Mutex<LocalState>,
        changed: Condvar,
    }

    #[derive(Default)]
    struct LocalState {
        // queues[i] holds messages sent to side i.
        queues: [VecDeque<Message>; 2],
        closed: [bool; 2],
    }

    impl Transport for LocalChannel {
        fn send(&self, side: Side, message: Message) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.queues[side.other().index()].push_back(message);
            self.changed.notify_all();
        }

        fn receive(&self, side: Side) -> Result<Message, PeerFailed> {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(message) = state.queues[side.index()].pop_front() {
                    return Ok(message);
                }
                if state.closed[side.other().index()] {
                    return Err(PeerFailed);
                }
                state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        }

        fn close(&self, side: Side) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed[side.index()] = true;
            self.changed.notify_all();
        }
    }

    /// One side of a channel, untyped: generated ends wrap it, and closing
    /// it (by dropping) wakes a peer waiting on it.
    pub struct Endpoint {
        transport: Arc<dyn Transport>,
        side: Side,
    }

    impl Endpoint {
        /// The ends of a channel over the given transport.
        pub fn pair(transport: Arc<dyn Transport>) -> (Endpoint, Endpoint) {
            (
                Endpoint { transport: transport.clone(), side: Side::First },
                Endpoint { transport, side: Side::Second },
            )
        }

        /// One end of a channel over the given transport, such as a
        /// network channel end's (see net::NetSession).
        pub fn on(transport: Arc<dyn Transport>, side: Side) -> Endpoint {
            Endpoint { transport, side }
        }

        pub fn send<T: Any + Send>(&self, value: T) {
            self.transport.send(self.side, Box::new(value));
        }

        /// The next value; panics with PeerFailed when the other end gave up.
        pub fn receive<T: Any + Send>(&self) -> T {
            self.try_receive().unwrap_or_else(|failed| panic!("{failed}"))
        }

        /// The next value, or PeerFailed once the other end gave up and
        /// nothing it sent is left.
        pub fn try_receive<T: Any + Send>(&self) -> Result<T, PeerFailed> {
            match self.transport.receive(self.side)?.downcast::<T>() {
                Ok(value) => Ok(*value),
                Err(_) => panic!("a session received a value of an unexpected type"),
            }
        }

        /// Gives up the conversation: the other end's receives fail with
        /// PeerFailed once it has received what was already sent. Dropping
        /// an end does the same.
        pub fn abandon(self) {}

        /// Sends a value already boxed (a relay passing one step on).
        pub fn send_message(&self, message: Message) {
            self.transport.send(self.side, message);
        }

        /// The next value, still boxed (a relay passing one step on).
        pub fn receive_message(&self) -> Result<Message, PeerFailed> {
            self.transport.receive(self.side)
        }

        /// For an unused end between nodes, the address another node takes
        /// it over from; None for a local end.
        pub fn hand_over(&self) -> Option<String> {
            self.transport.hand_over()
        }
    }

    impl Drop for Endpoint {
        fn drop(&mut self) {
            self.transport.close(self.side);
        }
    }

    /// The ends of a fresh in-memory channel.
    pub fn channel() -> (Endpoint, Endpoint) {
        Endpoint::pair(Arc::new(LocalChannel::default()))
    }

    /// A running process: a thread whose result join returns.
    pub struct Process<T>(std::thread::JoinHandle<T>);

    impl<T> Process<T> {
        /// Waits for the process; its panic, if any, continues here.
        pub fn join(self) -> T {
            self.0.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }
    }

    /// Runs `body` as a new process.
    pub fn spawn<T: Send + 'static>(body: impl FnOnce() -> T + Send + 'static) -> Process<T> {
        Process(std::thread::spawn(body))
    }

    /// Runs two closures in parallel and returns both results; a panic in
    /// either continues here.
    pub fn par<A: Send, B: Send>(
        left: impl FnOnce() -> A + Send,
        right: impl FnOnce() -> B + Send,
    ) -> (A, B) {
        std::thread::scope(|scope| {
            let right = scope.spawn(right);
            let left = left();
            let right = right.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            (left, right)
        })
    }
}

// Actors. An actor owns a state and handles one message at a time, in the
// order they arrive. It is not a thread: each message takes a ticket when it
// is sent, and runs when the actor's turn reaches it. A call runs its handler
// on the caller's own thread once its turn comes, so it may borrow from the
// caller (the model runner's Context, say) and costs no thread handoff; a
// tell is queued, and a worker thread runs while there are queued tells and
// stops when there are none, so an idle actor costs only its state.
//
// A handler that fails (an Err or a panic) crashes the actor: the caller
// gets an error starting "the actor crashed: ". A supervised actor with a
// restart function restarts in place, keeping its address and the messages
// waiting for it; any other actor stops, and later messages fail with "the
// actor has stopped". Monitors hear of each crash and of the stop; links
// carry a crash to the linked actor, crossing each link once.
pub mod actors {
    use std::collections::{HashSet, VecDeque};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
    use std::time::{Duration, Instant};

    /// The start of the error a call returns when its handler crashed the actor.
    pub const CRASHED: &str = "the actor crashed: ";
    /// The error of a message sent to an actor that has stopped.
    pub const STOPPED: &str = "the actor has stopped";

    /// Whether an error says the handler crashed the actor.
    pub fn is_crashed(error: &str) -> bool {
        error.starts_with(CRASHED)
    }

    /// Whether an error says the actor had stopped.
    pub fn is_stopped(error: &str) -> bool {
        error == STOPPED
    }

    /// What a monitor hears: a crash and its cause, or the stop.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Exit {
        Crashed(String),
        Stopped,
    }

    static CRASHES: AtomicU64 = AtomicU64::new(1);

    // Each first crash is numbered, so a crash crosses each link once.
    fn next_crash() -> u64 {
        CRASHES.fetch_add(1, Ordering::Relaxed)
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = panic.downcast_ref::<&str>() {
            format!("a handler panicked: {s}")
        } else if let Some(s) = panic.downcast_ref::<String>() {
            format!("a handler panicked: {s}")
        } else {
            "a handler panicked".into()
        }
    }

    type Tell<S> = Box<dyn FnOnce(S) -> super::Result<S> + Send>;
    type Restart<S> = Arc<dyn Fn(S) -> super::Result<S> + Send + Sync>;
    type Keep<S> = Arc<dyn Fn(&S) -> S + Send + Sync>;
    type Notify = Arc<dyn Fn(Exit) + Send + Sync>;
    type LinkCrash = Arc<dyn Fn(&str, u64) + Send + Sync>;

    enum Message<S> {
        Run(Tell<S>),
        // A crash on purpose (crash, a link), with its first crash's number.
        Crash(String, u64),
        // A restart a supervisor asks of a sibling.
        Restart,
    }

    /// An actor over a state S. Clones share the actor.
    pub struct Actor<S> {
        inner: Arc<Inner<S>>,
    }

    impl<S> Clone for Actor<S> {
        fn clone(&self) -> Self {
            Actor { inner: self.inner.clone() }
        }
    }

    struct Inner<S> {
        lock: Mutex<Turns<S>>,
        changed: Condvar,
        restart: Option<Restart<S>>,
        // Copies the state before each handler, so a restart can start from
        // the last state even when the handler failed.
        keep: Option<Keep<S>>,
        links: Mutex<Vec<LinkCrash>>,
        monitors: Mutex<Vec<Notify>>,
        supervisor: Mutex<Option<Weak<SupervisorInner>>>,
        seen: Mutex<HashSet<u64>>,
    }

    struct Turns<S> {
        // The state between messages; None while a handler holds it, or
        // once the actor has halted.
        state: Option<S>,
        next_ticket: u64,
        serving: u64,
        tells: VecDeque<(u64, Message<S>)>,
        draining: bool,
        stopped: bool,
        halted: bool,
    }

    impl<S: Send + 'static> Actor<S> {
        /// Starts an actor owning `state`. It cannot restart: a crash stops
        /// it, even under a supervisor.
        pub fn new(state: S) -> Self {
            Self::make(state, None, None)
        }

        fn make(state: S, restart: Option<Restart<S>>, keep: Option<Keep<S>>) -> Self {
            Actor {
                inner: Arc::new(Inner {
                    lock: Mutex::new(Turns {
                        state: Some(state),
                        next_ticket: 0,
                        serving: 0,
                        tells: VecDeque::new(),
                        draining: false,
                        stopped: false,
                        halted: false,
                    }),
                    changed: Condvar::new(),
                    restart,
                    keep,
                    links: Mutex::new(Vec::new()),
                    monitors: Mutex::new(Vec::new()),
                    supervisor: Mutex::new(None),
                    seen: Mutex::new(HashSet::new()),
                }),
            }
        }

        fn id(&self) -> usize {
            Arc::as_ptr(&self.inner) as *const () as usize
        }

        fn turns(&self) -> MutexGuard<'_, Turns<S>> {
            lock(&self.inner.lock)
        }

        fn ticket(&self) -> super::Result<u64> {
            let mut turns = self.turns();
            if turns.stopped {
                return Err(STOPPED.into());
            }
            let ticket = turns.next_ticket;
            turns.next_ticket += 1;
            Ok(ticket)
        }

        /// Waits for `ticket`'s turn and takes the state.
        fn take(&self, ticket: u64) -> std::result::Result<S, String> {
            let mut turns = self.turns();
            while turns.serving != ticket {
                turns = self.inner.changed.wait(turns).unwrap_or_else(|e| e.into_inner());
            }
            if turns.halted {
                return Err(STOPPED.into());
            }
            turns.state.take().ok_or_else(|| STOPPED.to_string())
        }

        /// Ends the current turn, leaving `state` unless the actor halted.
        fn finish(&self, state: Option<S>) {
            let mut turns = self.turns();
            if !turns.halted {
                if let Some(state) = state {
                    turns.state = Some(state);
                }
            }
            turns.serving += 1;
            self.inner.changed.notify_all();
        }

        fn queue(&self, message: Message<S>) -> super::Result<()> {
            let start = {
                let mut turns = self.turns();
                if turns.stopped {
                    return Err(STOPPED.into());
                }
                let ticket = turns.next_ticket;
                turns.next_ticket += 1;
                turns.tells.push_back((ticket, message));
                !std::mem::replace(&mut turns.draining, true)
            };
            if start {
                let actor = self.clone();
                std::thread::spawn(move || actor.drain());
            }
            Ok(())
        }

        /// Stops at once: the state is dropped, and messages still waiting
        /// fail with "the actor has stopped".
        fn halt_now(&self) {
            let mut turns = self.turns();
            turns.halted = true;
            turns.stopped = true;
            turns.state = None;
            self.inner.changed.notify_all();
        }

        /// On the actor's turn, with the last state if it is known: restart
        /// or halt, end the turn, then tell monitors and links.
        fn crashed(&self, cause: String, origin: u64, last: Option<S>) {
            lock(&self.inner.seen).insert(origin);
            let supervisor = lock(&self.inner.supervisor).clone().and_then(|w| w.upgrade());
            let mut next = None;
            if let (Some(supervisor), Some(restart), Some(last)) = (supervisor, self.inner.restart.clone(), last) {
                if supervisor.child_crashed(self.id(), &cause) {
                    next = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| restart(last))).ok().and_then(|r| r.ok());
                }
            }
            if next.is_none() {
                self.halt_now();
            }
            self.finish(next);
            for monitor in lock(&self.inner.monitors).clone() {
                monitor(Exit::Crashed(cause.clone()));
            }
            for link in lock(&self.inner.links).clone() {
                link(&cause, origin);
            }
        }

        /// Runs one turn's body over the state; a failure crashes the actor.
        fn turn<R>(&self, state: S, body: impl FnOnce(S) -> super::Result<(R, S)>) -> super::Result<R> {
            let snapshot = self.inner.keep.as_ref().map(|keep| keep(&state));
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(state)))
                .unwrap_or_else(|panic| Err(panic_text(panic)));
            match outcome {
                Ok((reply, next)) => {
                    self.finish(Some(next));
                    Ok(reply)
                }
                Err(cause) => {
                    self.crashed(cause.clone(), next_crash(), snapshot);
                    Err(format!("{CRASHED}{cause}"))
                }
            }
        }

        /// Runs handler(state) -> (reply, next state) in turn and returns the
        /// reply. A handler that fails or panics crashes the actor, and call
        /// returns an error starting "the actor crashed: ".
        pub fn call<R>(&self, handler: impl FnOnce(S) -> super::Result<(R, S)>) -> super::Result<R> {
            let ticket = self.ticket()?;
            match self.take(ticket) {
                Ok(state) => self.turn(state, handler),
                Err(e) => {
                    self.finish(None);
                    Err(e)
                }
            }
        }

        /// Queues handler(state) -> (reply, next state) without waiting; the
        /// reply is dropped.
        pub fn tell<R>(&self, handler: impl FnOnce(S) -> super::Result<(R, S)> + Send + 'static) -> super::Result<()> {
            self.queue(Message::Run(Box::new(move |s| handler(s).map(|(_, next)| next))))
        }

        fn drain(&self) {
            loop {
                let (ticket, message) = {
                    let mut turns = self.turns();
                    match turns.tells.pop_front() {
                        Some(next) => next,
                        None => {
                            turns.draining = false;
                            return;
                        }
                    }
                };
                let state = match self.take(ticket) {
                    Ok(state) => state,
                    Err(_) => {
                        self.finish(None);
                        continue;
                    }
                };
                match message {
                    Message::Run(handler) => {
                        let _ = self.turn(state, |s| handler(s).map(|next| ((), next)));
                    }
                    // A crash that already reached this actor by another
                    // link is not repeated.
                    Message::Crash(_, origin) if lock(&self.inner.seen).contains(&origin) => self.finish(Some(state)),
                    Message::Crash(cause, origin) => self.crashed(cause, origin, Some(state)),
                    Message::Restart => match &self.inner.restart {
                        Some(restart) => {
                            let restart = restart.clone();
                            let _ = self.turn(state, |s| restart(s).map(|next| ((), next)));
                        }
                        None => self.finish(Some(state)),
                    },
                }
            }
        }

        /// Crashes the actor once the messages sent before are handled, as a
        /// failing handler would: for testing supervision.
        pub fn crash(&self, cause: &str) -> super::Result<()> {
            let ticket = self.ticket()?;
            match self.take(ticket) {
                Ok(state) => {
                    self.crashed(cause.to_string(), next_crash(), Some(state));
                    Ok(())
                }
                Err(e) => {
                    self.finish(None);
                    Err(e)
                }
            }
        }

        /// Replaces the state by restart(last state) between messages, as a
        /// supervised restart does (crash injection in model runs).
        pub fn restart(&self, restart: impl FnOnce(S) -> super::Result<S>) -> super::Result<()> {
            self.call(|s| restart(s).map(|next| ((), next)))
        }

        /// notify(Exit::Crashed(cause)) after each crash, and
        /// notify(Exit::Stopped) once it stops.
        pub fn monitor(&self, notify: impl Fn(Exit) + Send + Sync + 'static) {
            lock(&self.inner.monitors).push(Arc::new(notify));
        }

        /// Links two actors: when either crashes, the other crashes too.
        pub fn link<T: Send + 'static>(&self, other: &Actor<T>) {
            lock(&self.inner.links).push(other.link_target());
            lock(&other.inner.links).push(self.link_target());
        }

        fn link_target(&self) -> LinkCrash {
            let weak = Arc::downgrade(&self.inner);
            Arc::new(move |cause: &str, origin: u64| {
                if let Some(inner) = weak.upgrade() {
                    let actor = Actor { inner };
                    if !lock(&actor.inner.seen).contains(&origin) {
                        let _ = actor.queue(Message::Crash(cause.to_string(), origin));
                    }
                }
            })
        }

        /// Refuses further messages; those already sent are still handled. A
        /// permanent child of a supervisor restarts instead.
        pub fn stop(&self) {
            let supervisor = lock(&self.inner.supervisor).clone().and_then(|w| w.upgrade());
            if let Some(supervisor) = supervisor {
                if supervisor.child_stopped(self.id()) {
                    return;
                }
            }
            let already = std::mem::replace(&mut self.turns().stopped, true);
            if !already {
                for monitor in lock(&self.inner.monitors).clone() {
                    monitor(Exit::Stopped);
                }
            }
        }
    }

    impl<S: Clone + Send + 'static> Actor<S> {
        /// Starts an actor owning `state` that restarts after a crash, under
        /// a supervisor, with restart(last state).
        pub fn with_restart(state: S, restart: impl Fn(S) -> super::Result<S> + Send + Sync + 'static) -> Self {
            Self::make(state, Some(Arc::new(restart)), Some(Arc::new(S::clone)))
        }

        /// The state after every message sent before this call.
        pub fn state(&self) -> super::Result<S> {
            self.call(|s: S| Ok((s.clone(), s)))
        }
    }

    /// A child a supervisor can hold: an actor or another supervisor.
    pub trait Supervised: Send + Sync {
        /// Identifies the child: clones share it.
        fn child_id(&self) -> usize;
        /// Restarts in mailbox order (an actor) or restarts every child (a
        /// supervisor).
        fn restart_later(&self);
        /// Stops at once.
        fn halt(&self);
        /// Stops as asked (see stop).
        fn stop_child(&self);
        /// Sets or clears the child's supervisor.
        fn attach(&self, supervisor: Option<&Supervisor>);
    }

    impl<S: Send + 'static> Supervised for Actor<S> {
        fn child_id(&self) -> usize {
            self.id()
        }

        fn restart_later(&self) {
            let _ = self.queue(Message::Restart);
        }

        fn halt(&self) {
            self.halt_now();
        }

        fn stop_child(&self) {
            self.stop();
        }

        fn attach(&self, supervisor: Option<&Supervisor>) {
            *lock(&self.inner.supervisor) = supervisor.map(|s| Arc::downgrade(&s.inner));
        }
    }

    /// Which children restart after one crashes.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Strategy {
        /// Only the child that crashed.
        OneForOne,
        /// Every child.
        OneForAll,
        /// The child that crashed and those added after it.
        RestForOne,
    }

    /// Whether a child restarts.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Lifetime {
        /// After a crash or a stop.
        Permanent,
        /// Only after a crash.
        Transient,
        /// Never.
        Temporary,
    }

    struct Child {
        child: Arc<dyn Supervised>,
        lifetime: Lifetime,
    }

    struct SupervisorState {
        children: Vec<Child>,
        restarts: VecDeque<Instant>,
        stopped: bool,
    }

    struct SupervisorInner {
        strategy: Strategy,
        max_restarts: usize,
        period: Duration,
        state: Mutex<SupervisorState>,
        parent: Mutex<Option<Weak<SupervisorInner>>>,
        monitors: Mutex<Vec<Notify>>,
    }

    /// Starts nothing itself: children (actors or supervisors) are added
    /// with supervise, and restarted after a crash by the strategy and each
    /// child's lifetime. More than max_restarts within period is the
    /// supervisor's own crash: its supervisor restarts all of its children,
    /// or, at the top, every child stops. Clones share the supervisor.
    #[derive(Clone)]
    pub struct Supervisor {
        inner: Arc<SupervisorInner>,
    }

    impl Supervisor {
        pub fn new(strategy: Strategy, max_restarts: usize, period: Duration) -> Self {
            Supervisor {
                inner: Arc::new(SupervisorInner {
                    strategy,
                    max_restarts,
                    period,
                    state: Mutex::new(SupervisorState { children: Vec::new(), restarts: VecDeque::new(), stopped: false }),
                    parent: Mutex::new(None),
                    monitors: Mutex::new(Vec::new()),
                }),
            }
        }

        /// Adds a started child (a clone of an actor or a supervisor).
        pub fn supervise(&self, child: impl Supervised + 'static, lifetime: Lifetime) {
            child.attach(Some(self));
            lock(&self.inner.state).children.push(Child { child: Arc::new(child), lifetime });
        }

        /// notify(Exit::Crashed(cause)) when it passes its restart limit, and
        /// notify(Exit::Stopped) once stopped.
        pub fn monitor(&self, notify: impl Fn(Exit) + Send + Sync + 'static) {
            lock(&self.inner.monitors).push(Arc::new(notify));
        }

        /// Stops every child, last added first, without restarting them.
        pub fn stop(&self) {
            self.inner.stop();
        }
    }

    impl SupervisorInner {
        fn id(self: &Arc<Self>) -> usize {
            Arc::as_ptr(self) as *const () as usize
        }

        fn allow_restart(&self, state: &mut SupervisorState) -> bool {
            let now = Instant::now();
            while state.restarts.front().is_some_and(|at| now.duration_since(*at) > self.period) {
                state.restarts.pop_front();
            }
            if state.restarts.len() >= self.max_restarts {
                return false;
            }
            state.restarts.push_back(now);
            true
        }

        /// Under the lock: the children to restart for the crash of the
        /// child at `index`, or None when the supervisor gives up.
        fn restarting(
            self: &Arc<Self>,
            state: &mut SupervisorState,
            index: usize,
            cause: &str,
            crashed: usize,
        ) -> Option<Vec<Arc<dyn Supervised>>> {
            if self.allow_restart(state) {
                let group: Vec<&Child> = match self.strategy {
                    Strategy::OneForOne => vec![&state.children[index]],
                    Strategy::OneForAll => state.children.iter().collect(),
                    Strategy::RestForOne => state.children[index..].iter().collect(),
                };
                return Some(group.into_iter().map(|c| c.child.clone()).collect());
            }
            let parent = lock(&self.parent).clone().and_then(|w| w.upgrade());
            if let Some(parent) = parent {
                if parent.child_failed(self.id(), cause) {
                    state.restarts.clear();
                    return Some(state.children.iter().map(|c| c.child.clone()).collect());
                }
            }
            self.fail(state, crashed, cause);
            None
        }

        fn entry(state: &SupervisorState, id: usize) -> Option<usize> {
            state.children.iter().position(|c| c.child.child_id() == id)
        }

        /// On the child's turn: whether it restarts now.
        fn child_crashed(self: &Arc<Self>, id: usize, cause: &str) -> bool {
            let group = {
                let mut state = lock(&self.state);
                let Some(index) = Self::entry(&state, id) else {
                    return false;
                };
                if state.stopped {
                    return false;
                }
                if state.children[index].lifetime == Lifetime::Temporary {
                    state.children.remove(index);
                    return false;
                }
                match self.restarting(&mut state, index, cause, id) {
                    Some(group) => group,
                    None => return false,
                }
            };
            for other in group {
                if other.child_id() != id {
                    other.restart_later();
                }
            }
            true
        }

        /// A child supervisor gave up: whether it may restart its children.
        fn child_failed(self: &Arc<Self>, id: usize, cause: &str) -> bool {
            let group = {
                let mut state = lock(&self.state);
                let Some(index) = Self::entry(&state, id) else {
                    return false;
                };
                if state.stopped {
                    return false;
                }
                if state.children[index].lifetime == Lifetime::Temporary {
                    state.children.remove(index);
                    return false;
                }
                match self.restarting(&mut state, index, cause, id) {
                    Some(group) => group,
                    None => return false,
                }
            };
            for other in group {
                if other.child_id() != id {
                    other.restart_later();
                }
            }
            true
        }

        /// Whether a stopped child is permanent and restarts instead.
        fn child_stopped(self: &Arc<Self>, id: usize) -> bool {
            let group = {
                let mut state = lock(&self.state);
                let Some(index) = Self::entry(&state, id) else {
                    return false;
                };
                if state.stopped {
                    return false;
                }
                if state.children[index].lifetime != Lifetime::Permanent {
                    state.children.remove(index);
                    return false;
                }
                match self.restarting(&mut state, index, "stopped", id) {
                    Some(group) => group,
                    None => return false,
                }
            };
            for other in group {
                other.restart_later();
            }
            true
        }

        /// Under the lock: every child but the one crashing (which halts
        /// itself) stops, and so does the supervisor.
        fn fail(&self, state: &mut SupervisorState, crashed: usize, cause: &str) {
            let children = std::mem::take(&mut state.children);
            state.stopped = true;
            for c in children.iter().rev() {
                c.child.attach(None);
                if c.child.child_id() != crashed {
                    c.child.halt();
                }
            }
            for monitor in lock(&self.monitors).clone() {
                monitor(Exit::Crashed(cause.to_string()));
            }
        }

        fn stop(&self) {
            let children = {
                let mut state = lock(&self.state);
                if state.stopped {
                    return;
                }
                state.stopped = true;
                std::mem::take(&mut state.children)
            };
            for c in children.iter().rev() {
                c.child.attach(None);
                c.child.stop_child();
            }
            for monitor in lock(&self.monitors).clone() {
                monitor(Exit::Stopped);
            }
        }
    }

    impl Supervised for Supervisor {
        fn child_id(&self) -> usize {
            self.inner.id()
        }

        /// Restarted by its own supervisor: every child restarts.
        fn restart_later(&self) {
            let children: Vec<Arc<dyn Supervised>> = {
                let mut state = lock(&self.inner.state);
                state.restarts.clear();
                state.children.iter().map(|c| c.child.clone()).collect()
            };
            for child in children {
                child.restart_later();
            }
        }

        fn halt(&self) {
            self.inner.stop();
        }

        fn stop_child(&self) {
            self.inner.stop();
        }

        fn attach(&self, supervisor: Option<&Supervisor>) {
            *lock(&self.inner.parent) = supervisor.map(|s| Arc::downgrade(&s.inner));
        }
    }

    /// The runtime's own check of crashes, links, monitors and supervision:
    /// every strategy, lifetime, the restart limit and escalation. Fails
    /// naming the first behaviour that differs.
    pub fn check_supervision() -> super::Result<()> {
        fn counter() -> Actor<i64> {
            Actor::with_restart(0, |_| Ok(0))
        }
        fn bump(a: &Actor<i64>) -> super::Result<i64> {
            a.call(|s| Ok((s + 1, s + 1)))
        }
        fn fail(a: &Actor<i64>) -> super::Result<()> {
            match a.call(|_| -> super::Result<((), i64)> { Err("division by zero".into()) }) {
                Err(e) if is_crashed(&e) => Ok(()),
                _ => Err("a failing handler did not crash the actor".into()),
            }
        }
        fn stopped(a: &Actor<i64>, what: &str) -> super::Result<()> {
            match a.state() {
                Err(e) if is_stopped(&e) => Ok(()),
                _ => Err(format!("{what} should have stopped")),
            }
        }
        fn expect<T: PartialEq + std::fmt::Debug>(actual: T, wanted: T, what: &str) -> super::Result<()> {
            if actual != wanted {
                return Err(format!("{what}: got {actual:?}, expected {wanted:?}"));
            }
            Ok(())
        }
        fn wait_until(mut done: impl FnMut() -> bool) {
            for _ in 0..100 {
                if done() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        let long = Duration::from_secs(10);
        let a = counter();
        bump(&a)?;
        fail(&a)?;
        stopped(&a, "an unsupervised actor that crashed")?;
        let sup = Supervisor::new(Strategy::OneForOne, 3, Duration::from_secs(5));
        let (x, y) = (counter(), counter());
        sup.supervise(x.clone(), Lifetime::Permanent);
        sup.supervise(y.clone(), Lifetime::Permanent);
        bump(&x)?;
        bump(&y)?;
        bump(&y)?;
        fail(&x)?;
        expect((x.state()?, y.state()?), (0, 2), "one for one restarts only the crashed child")?;
        let sup = Supervisor::new(Strategy::OneForAll, 3, Duration::from_secs(5));
        let (x, y) = (counter(), counter());
        sup.supervise(x.clone(), Lifetime::Permanent);
        sup.supervise(y.clone(), Lifetime::Permanent);
        bump(&x)?;
        bump(&y)?;
        fail(&x)?;
        expect((x.state()?, y.state()?), (0, 0), "one for all restarts every child")?;
        let sup = Supervisor::new(Strategy::RestForOne, 3, Duration::from_secs(5));
        let (x, y, z) = (counter(), counter(), counter());
        for c in [&x, &y, &z] {
            sup.supervise(c.clone(), Lifetime::Permanent);
            bump(c)?;
        }
        fail(&y)?;
        expect((x.state()?, y.state()?, z.state()?), (1, 0, 0), "rest for one restarts the child and later ones")?;
        let sup = Supervisor::new(Strategy::OneForOne, 3, Duration::from_secs(5));
        let t = counter();
        sup.supervise(t.clone(), Lifetime::Temporary);
        fail(&t)?;
        stopped(&t, "a temporary child that crashed")?;
        let sup = Supervisor::new(Strategy::OneForOne, 3, Duration::from_secs(5));
        let (p, q) = (counter(), counter());
        sup.supervise(p.clone(), Lifetime::Permanent);
        sup.supervise(q.clone(), Lifetime::Transient);
        bump(&p)?;
        p.stop();
        expect(p.state()?, 0, "a permanent child restarts after a stop")?;
        q.stop();
        stopped(&q, "a transient child that was stopped")?;
        let events = Arc::new(Mutex::new(Vec::new()));
        let sup = Supervisor::new(Strategy::OneForOne, 2, long);
        let heard = events.clone();
        sup.monitor(move |e| lock(&heard).push(e));
        let (x, y) = (counter(), counter());
        sup.supervise(x.clone(), Lifetime::Permanent);
        sup.supervise(y.clone(), Lifetime::Permanent);
        fail(&x)?;
        fail(&x)?;
        fail(&x)?;
        stopped(&y, "a child of a supervisor past its restart limit")?;
        expect(
            lock(&events).iter().map(|e| matches!(e, Exit::Crashed(_))).collect::<Vec<_>>(),
            vec![true],
            "a supervisor past its limit tells its monitors",
        )?;
        let outer = Supervisor::new(Strategy::OneForOne, 5, long);
        let inner = Supervisor::new(Strategy::OneForOne, 1, long);
        outer.supervise(inner.clone(), Lifetime::Permanent);
        let (x, y) = (counter(), counter());
        inner.supervise(x.clone(), Lifetime::Permanent);
        inner.supervise(y.clone(), Lifetime::Permanent);
        bump(&y)?;
        fail(&x)?;
        fail(&x)?;
        expect((x.state()?, y.state()?), (0, 0), "a supervisor past its limit is restarted by its own")?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (counter(), counter());
        a.link(&b);
        let heard = seen.clone();
        b.monitor(move |e| lock(&heard).push(e));
        fail(&a)?;
        wait_until(|| !lock(&seen).is_empty());
        stopped(&b, "an unsupervised actor linked to one that crashed")?;
        expect(
            lock(&seen).iter().map(|e| matches!(e, Exit::Crashed(_))).collect::<Vec<_>>(),
            vec![true],
            "a monitor hears of a crash",
        )?;
        let sup = Supervisor::new(Strategy::OneForOne, 10, Duration::from_secs(5));
        let (a, b, c) = (counter(), counter(), counter());
        for x in [&a, &b, &c] {
            sup.supervise(x.clone(), Lifetime::Permanent);
        }
        a.link(&b);
        b.link(&c);
        c.link(&a);
        bump(&a)?;
        bump(&b)?;
        bump(&c)?;
        fail(&a)?;
        wait_until(|| lock(&sup.inner.state).restarts.len() >= 3);
        std::thread::sleep(Duration::from_millis(50));
        expect(
            (a.state()?, b.state()?, c.state()?, lock(&sup.inner.state).restarts.len()),
            (0, 0, 0, 3),
            "a crash crosses each link once",
        )?;
        Ok(())
    }

    /// A queue with many senders and one receiver: the channel form of an
    /// actor. A process that loops over receive and answers each message is
    /// an actor written by hand; send never waits. Clones share the mailbox.
    pub struct Mailbox<T> {
        inner: Arc<(Mutex<(VecDeque<T>, bool)>, Condvar)>,
    }

    impl<T> Clone for Mailbox<T> {
        fn clone(&self) -> Self {
            Mailbox { inner: self.inner.clone() }
        }
    }

    impl<T> Default for Mailbox<T> {
        fn default() -> Self {
            Mailbox { inner: Arc::new((Mutex::new((VecDeque::new(), false)), Condvar::new())) }
        }
    }

    impl<T> Mailbox<T> {
        pub fn new() -> Self {
            Self::default()
        }

        /// Sends a message; fails once the mailbox is closed.
        pub fn send(&self, value: T) -> super::Result<()> {
            let mut items = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            if items.1 {
                return Err("the mailbox is closed".into());
            }
            items.0.push_back(value);
            self.inner.1.notify_one();
            Ok(())
        }

        /// The next message, waiting up to `timeout` (forever when None);
        /// fails on timing out, or once closed and empty.
        pub fn receive(&self, timeout: Option<std::time::Duration>) -> super::Result<T> {
            let deadline = timeout.map(|t| std::time::Instant::now() + t);
            let mut items = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(value) = items.0.pop_front() {
                    return Ok(value);
                }
                if items.1 {
                    return Err("the mailbox is closed".into());
                }
                items = match deadline {
                    None => self.inner.1.wait(items).unwrap_or_else(|e| e.into_inner()),
                    Some(at) => {
                        let now = std::time::Instant::now();
                        if now >= at {
                            return Err("no message arrived in time".into());
                        }
                        self.inner.1.wait_timeout(items, at - now).unwrap_or_else(|e| e.into_inner()).0
                    }
                };
            }
        }

        /// The Mailbox ability's receive ... within d: the next message, or
        /// None when none arrives within `within`. On a virtual clock (a
        /// Clock handler that is not real time) it waits no real time: it
        /// takes a message already sent, or lets the time pass on that clock
        /// and gives None. With no clock, or the real one, it waits real
        /// time. Fails once closed and empty.
        pub fn receive_within(
            &self,
            within: std::time::Duration,
            clock: Option<&dyn super::ClockAbility>,
        ) -> super::Result<Option<T>> {
            if let Some(clock) = clock.filter(|clock| !clock.real_time()) {
                {
                    let mut items = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(value) = items.0.pop_front() {
                        return Ok(Some(value));
                    }
                    if items.1 {
                        return Err("the mailbox is closed".into());
                    }
                }
                clock.sleep_micros(i64::try_from(within.as_micros()).unwrap_or(i64::MAX));
                return Ok(None);
            }
            let deadline = std::time::Instant::now().checked_add(within);
            let mut items = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(value) = items.0.pop_front() {
                    return Ok(Some(value));
                }
                if items.1 {
                    return Err("the mailbox is closed".into());
                }
                items = match deadline {
                    None => self.inner.1.wait(items).unwrap_or_else(|e| e.into_inner()),
                    Some(at) => {
                        let now = std::time::Instant::now();
                        if now >= at {
                            return Ok(None);
                        }
                        self.inner.1.wait_timeout(items, at - now).unwrap_or_else(|e| e.into_inner()).0
                    }
                };
            }
        }

        /// Refuses further messages; those already sent can still be received.
        pub fn close(&self) {
            self.inner.0.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
            self.inner.1.notify_all();
        }
    }
}

#[cfg(test)]
mod actor_tests {
    use super::ClockAbility as _;
    use super::actors::{Actor, Mailbox};

    struct Virtual(std::sync::atomic::AtomicI64);
    impl super::ClockAbility for Virtual {
        fn now_micros(&self) -> i64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn sleep_micros(&self, micros: i64) {
            self.0.fetch_add(micros, std::sync::atomic::Ordering::SeqCst);
        }
        fn real_time(&self) -> bool {
            false
        }
    }

    #[test]
    fn receive_within_waits_on_the_clock() {
        let mailbox = Mailbox::new();
        let clock = Virtual(std::sync::atomic::AtomicI64::new(0));
        mailbox.send(7).unwrap();
        let within = std::time::Duration::from_secs(60);
        assert_eq!(mailbox.receive_within(within, Some(&clock)).unwrap(), Some(7));
        assert_eq!(mailbox.receive_within(within, Some(&clock)).unwrap(), None);
        assert_eq!(clock.now_micros(), 60_000_000);
        assert_eq!(mailbox.receive_within(std::time::Duration::from_millis(10), None).unwrap(), None);
        mailbox.close();
        assert!(mailbox.receive_within(within, Some(&clock)).is_err());
    }

    #[test]
    fn calls_run_one_at_a_time_in_order() {
        let actor = Actor::new(0i64);
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let a = actor.clone();
                std::thread::spawn(move || {
                    for _ in 0..100 {
                        a.call(|s| Ok(((), s + 1))).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        actor.tell(|s| Ok(((), s * 2))).unwrap();
        assert_eq!(actor.state().unwrap(), 800);
        actor.stop();
        assert!(actor.call(|s| Ok(((), s))).is_err());
    }

    #[test]
    fn supervision_behaves_as_documented() {
        super::actors::check_supervision().unwrap();
    }

    #[test]
    fn mailboxes_deliver_in_order_and_close() {
        let m = Mailbox::new();
        m.send(1).unwrap();
        m.send(2).unwrap();
        m.close();
        assert_eq!(m.receive(None).unwrap(), 1);
        assert_eq!(m.receive(None).unwrap(), 2);
        assert!(m.receive(None).is_err());
        assert!(m.send(3).is_err());
    }
}

#[cfg(test)]
mod session_tests {
    use super::sessions::{channel, par, spawn};

    #[test]
    fn messages_cross_in_order_both_ways() {
        let (first, second) = channel();
        let process = spawn(move || {
            let a: i32 = first.receive();
            let b: i32 = first.receive();
            first.send(i64::from(a) + i64::from(b));
        });
        second.send(2i32);
        second.send(3i32);
        assert_eq!(second.receive::<i64>(), 5);
        process.join();
    }

    #[test]
    fn a_failed_process_gives_up_its_ends() {
        let (first, second) = channel();
        let worker = spawn(move || {
            first.send(1i32);
            panic!("boom");
        });
        assert_eq!(second.receive::<i32>(), 1);
        assert_eq!(second.try_receive::<i32>(), Err(super::sessions::PeerFailed));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker.join())).is_err());
    }

    #[test]
    fn a_closed_peer_fails_a_waiting_receive() {
        let (first, second) = channel();
        let ((), outcome) = par(
            move || drop(first),
            move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| second.receive::<i32>())),
        );
        assert!(outcome.is_err());
    }
}

/// A descriptor text's data types, for decoding and encoding values whose
/// descriptors name them.
pub fn values_table(text: &str) -> Values {
    Values::new(
        read_descriptor(text)
            .into_iter()
            .filter(|f| matches!(f, Sexp::List(_)) && f.kind() == "data")
            .map(|f| (f.items()[1].name(), f))
            .collect(),
    )
}

/// The first form of a descriptor text.
pub fn descriptor(text: &str) -> Sexp {
    read_descriptor(text).into_iter().next().expect("a descriptor")
}

// Distribution. Values cross the network in a canonical binary encoding
// driven by their type descriptor (the same descriptors as generation), so
// no tags are sent and every target writes the same bytes:
//   int: zigzag LEB128 of the integer (any size)      bool: 0 or 1
//   text, bytes: LEB128 length, then UTF-8 or raw     unit: nothing
//   list: LEB128 count, then items                     maybe: 0, or 1 then the value
//   either: 0 then left, or 1 then right               data: LEB128 constructor index, then fields
// A node sends frames over a Transport (in memory, TCP or HTTP): kind,
// entity name, the sender's address, an id and a payload. The protocol and
// the bytes follow the Python reference, so nodes on different targets
// interoperate.
pub mod net {
    use super::{BigInt, Result, SplitMix64, Sexp, Value, Values, actors, read_descriptor, render, values_from};
    use num_traits::{One, Signed, ToPrimitive, Zero};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
    use std::time::{Duration, Instant};

    /// The start of the error of a node that could not be reached, or did
    /// not answer in time.
    pub const UNREACHABLE: &str = "unreachable: ";
    /// The start of the error of a channel end whose other end failed.
    pub const PEER_FAILED: &str = "peer failed: ";
    /// The start of the error of bytes that are not a value of the type.
    pub const WIRE: &str = "wire: ";

    pub fn is_unreachable(error: &str) -> bool {
        error.starts_with(UNREACHABLE)
    }

    pub fn is_peer_failed(error: &str) -> bool {
        error.starts_with(PEER_FAILED)
    }

    fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wire_error<T>(message: impl std::fmt::Display) -> Result<T> {
        Err(format!("{WIRE}{message}"))
    }

    fn put_varint(out: &mut Vec<u8>, n: &BigInt) {
        let mut n = n.clone();
        let mask = BigInt::from(0x7F);
        loop {
            let byte = (&n & &mask).to_u8().unwrap_or(0);
            n >>= 7;
            if n.is_zero() {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn get_varint(buf: &[u8], pos: &mut usize) -> Result<BigInt> {
        let mut result = BigInt::zero();
        let mut shift = 0usize;
        loop {
            let Some(&byte) = buf.get(*pos) else {
                return wire_error("the bytes end in the middle of a value");
            };
            *pos += 1;
            result |= BigInt::from(byte & 0x7F) << shift;
            if byte < 0x80 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    fn put_len(out: &mut Vec<u8>, n: usize) {
        put_varint(out, &BigInt::from(n));
    }

    fn get_len(buf: &[u8], pos: &mut usize) -> Result<usize> {
        match get_varint(buf, pos)?.to_usize() {
            Some(n) => Ok(n),
            None => wire_error("a length too large"),
        }
    }

    fn kind(d: &Sexp) -> &str {
        d.kind()
    }

    /// Appends the encoding of v, a value of descriptor d.
    pub fn wire_put(values: &Values, d: &Sexp, v: &Value, out: &mut Vec<u8>) -> Result<()> {
        let d = values.resolve(d);
        let items = d.items();
        match (kind(d), v) {
            ("int", Value::Integer(n)) => {
                let below = items[2].bound().is_some_and(|lo| *n < lo);
                let above = items[3].bound().is_some_and(|hi| *n > hi);
                if below || above {
                    return wire_error(format!("{n} is not a {}", items[1].name()));
                }
                let z = if n.is_negative() { -n * 2 - 1 } else { n * 2 };
                put_varint(out, &z);
            }
            ("bool", Value::Bool(b)) => out.push(u8::from(*b)),
            ("text" | "end", Value::Text(s)) => {
                put_len(out, s.len());
                out.extend_from_slice(s.as_bytes());
            }
            ("bytes", Value::Bytes(b)) => {
                put_len(out, b.len());
                out.extend_from_slice(b);
            }
            ("unit", _) => {}
            ("list", Value::List(xs)) => {
                put_len(out, xs.len());
                for x in xs {
                    wire_put(values, &items[1], x, out)?;
                }
            }
            ("maybe", Value::Maybe(None)) => out.push(0),
            ("maybe", Value::Maybe(Some(x))) => {
                out.push(1);
                wire_put(values, &items[1], x, out)?;
            }
            ("either", Value::Left(x)) => {
                out.push(0);
                wire_put(values, &items[1], x, out)?;
            }
            ("either", Value::Right(x)) => {
                out.push(1);
                wire_put(values, &items[2], x, out)?;
            }
            ("data", Value::Data(tag, fields)) => {
                let Some(index) = items[2..].iter().position(|c| c.items()[1].name() == *tag) else {
                    return wire_error(format!("{tag} is not a constructor of {}", items[1].name()));
                };
                put_len(out, index);
                for (field, fd) in fields.iter().zip(&items[2 + index].items()[2..]) {
                    wire_put(values, fd, field, out)?;
                }
            }
            (k, other) => return wire_error(format!("{} is not a {k}", render(other))),
        }
        Ok(())
    }

    /// The value of descriptor d encoded at pos, moving pos past it.
    pub fn wire_get(values: &Values, d: &Sexp, buf: &[u8], pos: &mut usize) -> Result<Value> {
        let d = values.resolve(d);
        let items = d.items();
        let flag = |pos: &mut usize, what: &str| -> Result<bool> {
            match buf.get(*pos) {
                Some(b) if *b <= 1 => {
                    *pos += 1;
                    Ok(*b == 1)
                }
                _ => wire_error(format!("not a {what}")),
            }
        };
        Ok(match kind(d) {
            "int" => {
                let z = get_varint(buf, pos)?;
                let two = BigInt::from(2);
                let v = if (&z % &two).is_zero() { &z / &two } else { -((&z + BigInt::one()) / &two) };
                let below = items[2].bound().is_some_and(|lo| v < lo);
                let above = items[3].bound().is_some_and(|hi| v > hi);
                if below || above {
                    return wire_error(format!("{v} is out of range for {}", items[1].name()));
                }
                Value::Integer(v)
            }
            "bool" => Value::Bool(flag(pos, "Bool")?),
            k @ ("text" | "bytes" | "end") => {
                let n = get_len(buf, pos)?;
                if *pos + n > buf.len() {
                    return wire_error("the bytes end in the middle of a value");
                }
                let raw = buf[*pos..*pos + n].to_vec();
                *pos += n;
                if k == "bytes" {
                    Value::Bytes(raw)
                } else {
                    match String::from_utf8(raw) {
                        Ok(s) => Value::Text(s),
                        Err(_) => return wire_error("text that is not UTF-8"),
                    }
                }
            }
            "unit" => Value::Unit,
            "list" => {
                let n = get_len(buf, pos)?;
                let mut xs = Vec::new();
                for _ in 0..n {
                    xs.push(wire_get(values, &items[1], buf, pos)?);
                }
                Value::List(xs)
            }
            "maybe" => {
                if flag(pos, "Maybe")? {
                    Value::Maybe(Some(Box::new(wire_get(values, &items[1], buf, pos)?)))
                } else {
                    Value::Maybe(None)
                }
            }
            "either" => {
                if flag(pos, "Either")? {
                    Value::Right(Box::new(wire_get(values, &items[2], buf, pos)?))
                } else {
                    Value::Left(Box::new(wire_get(values, &items[1], buf, pos)?))
                }
            }
            "data" => {
                let index = get_len(buf, pos)?;
                let ctors = &items[2..];
                let Some(ctor) = ctors.get(index) else {
                    return wire_error(format!("no constructor {index} in {}", items[1].name()));
                };
                let mut fields = Vec::new();
                for fd in &ctor.items()[2..] {
                    fields.push(wire_get(values, fd, buf, pos)?);
                }
                Value::Data(ctor.items()[1].name(), fields)
            }
            other => return wire_error(format!("unknown descriptor {other}")),
        })
    }

    /// The value's canonical bytes.
    pub fn wire_encode(values: &Values, d: &Sexp, v: &Value) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        wire_put(values, d, v, &mut out)?;
        Ok(out)
    }

    /// The value encoded by exactly these bytes.
    pub fn wire_decode(values: &Values, d: &Sexp, data: &[u8]) -> Result<Value> {
        let mut pos = 0;
        let v = wire_get(values, d, data, &mut pos)?;
        if pos != data.len() {
            return wire_error("extra bytes after the value");
        }
        Ok(v)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// count values generated from one seed, encoded, in hexadecimal.
    pub fn wire_encoded(text: &str, seed: u64, size: i64, count: i64) -> Vec<String> {
        let (values, d) = values_from(text);
        let mut random = SplitMix64::new(seed);
        (0..count)
            .map(|_| {
                let v = values.generate(&d, &mut random, size);
                hex(&wire_encode(&values, &d, &v).unwrap_or_else(|e| panic!("{e}")))
            })
            .collect()
    }

    /// Whether count generated values decode to themselves.
    pub fn wire_round_trips(text: &str, seed: u64, size: i64, count: i64) -> bool {
        let (values, d) = values_from(text);
        let mut random = SplitMix64::new(seed);
        (0..count).all(|_| {
            let v = values.generate(&d, &mut random, size);
            match wire_encode(&values, &d, &v).and_then(|bytes| wire_decode(&values, &d, &bytes)) {
                Ok(back) => render(&back) == render(&v),
                Err(_) => false,
            }
        })
    }

    fn text_d() -> Sexp {
        Sexp::List(vec![Sexp::Atom("text".into())])
    }

    fn seq_d() -> Sexp {
        Sexp::List(vec![Sexp::Atom("int".into()), Sexp::Atom("Int64".into()), Sexp::Blank, Sexp::Blank])
    }

    fn id_d() -> Sexp {
        Sexp::List(vec![Sexp::Atom("int".into()), Sexp::Atom("UInt64".into()), Sexp::Int(BigInt::zero()), Sexp::Blank])
    }

    fn bytes_d() -> Sexp {
        Sexp::List(vec![Sexp::Atom("bytes".into())])
    }

    fn no_types() -> Values {
        Values::new(HashMap::new())
    }

    fn put_text(out: &mut Vec<u8>, s: &str) {
        put_len(out, s.len());
        out.extend_from_slice(s.as_bytes());
    }

    fn get_text(buf: &[u8], pos: &mut usize) -> Result<String> {
        match wire_get(&no_types(), &text_d(), buf, pos)? {
            Value::Text(s) => Ok(s),
            _ => wire_error("not text"),
        }
    }

    fn put_seq(out: &mut Vec<u8>, seq: i64) {
        let _ = wire_put(&no_types(), &seq_d(), &Value::Integer(seq.into()), out);
    }

    fn get_seq(buf: &[u8], pos: &mut usize) -> Result<i64> {
        match wire_get(&no_types(), &seq_d(), buf, pos)? {
            Value::Integer(n) => n.to_i64().map_or_else(|| wire_error("a sequence number too large"), Ok),
            _ => wire_error("not a sequence number"),
        }
    }

    fn frame_encode(kind: &str, to: &str, source: &str, id: u64, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        put_text(&mut out, kind);
        put_text(&mut out, to);
        put_text(&mut out, source);
        let _ = wire_put(&no_types(), &id_d(), &Value::Integer(id.into()), &mut out);
        put_len(&mut out, payload.len());
        out.extend_from_slice(payload);
        out
    }

    struct Frame {
        kind: String,
        to: String,
        source: String,
        id: u64,
        payload: Vec<u8>,
    }

    fn frame_decode(data: &[u8]) -> Result<Frame> {
        let mut pos = 0;
        let kind = get_text(data, &mut pos)?;
        let to = get_text(data, &mut pos)?;
        let source = get_text(data, &mut pos)?;
        let id = match wire_get(&no_types(), &id_d(), data, &mut pos)? {
            Value::Integer(n) => n.to_u64().unwrap_or(0),
            _ => 0,
        };
        let payload = match wire_get(&no_types(), &bytes_d(), data, &mut pos)? {
            Value::Bytes(b) => b,
            _ => Vec::new(),
        };
        if pos != data.len() {
            return wire_error("extra bytes after a frame");
        }
        Ok(Frame { kind, to, source, id, payload })
    }

    /// "tcp://host:port/name" as ("tcp://host:port", "name").
    fn split_address(address: &str) -> Result<(String, String)> {
        match address.rsplit_once('/') {
            Some((node, name)) if node.contains("://") && !node.ends_with(':') && !node.ends_with('/') => {
                Ok((node.to_string(), name.to_string()))
            }
            _ => Err(format!("{address:?} is not an address such as tcp://127.0.0.1:7000/name")),
        }
    }

    /// What a transport calls with each frame that arrives.
    pub type Deliver = Arc<dyn Fn(Vec<u8>) + Send + Sync>;

    /// Moves frames between nodes: start begins calling deliver for every
    /// frame that arrives; send sends one to the node at that address, best
    /// effort (an error starting UNREACHABLE when it cannot); close stops.
    pub trait Transport: Send + Sync {
        fn address(&self) -> String;
        fn start(&self, deliver: Deliver);
        fn send(&self, node: &str, frame: Vec<u8>) -> Result<()>;
        fn close(&self);
        /// Whether a node on this transport skips the handshake and sends
        /// frames in the clear. Only the transport that
        /// MemoryNetwork::insecure_transport_for_tests makes says so: no
        /// other transport can name the marker.
        #[doc(hidden)]
        fn insecure_for_tests(&self) -> Option<sealed::InsecureForTests> {
            None
        }
    }

    mod sealed {
        /// The marker of the in-memory transport made for tests only.
        pub struct InsecureForTests;
    }

    /// Nodes in one process, with faults for testing: each frame may be lost
    /// or duplicated, and is delayed by up to delay (so frames can overtake
    /// each other); partition cuts nodes off until heal. Clones share it.
    #[derive(Clone)]
    pub struct MemoryNetwork {
        inner: Arc<NetworkInner>,
    }

    struct NetworkInner {
        state: Mutex<NetworkState>,
        loss: f64,
        duplicate: f64,
        delay: Duration,
        // With recording, every record sent, as the network saw it.
        recorded: Mutex<Option<Vec<Vec<u8>>>>,
    }

    struct NetworkState {
        random: SplitMix64,
        nodes: HashMap<String, Deliver>,
        groups: Option<Vec<HashSet<String>>>,
    }

    impl MemoryNetwork {
        pub fn new(seed: u64, loss: f64, duplicate: f64, delay: Duration) -> Self {
            MemoryNetwork {
                inner: Arc::new(NetworkInner {
                    state: Mutex::new(NetworkState { random: SplitMix64::new(seed), nodes: HashMap::new(), groups: None }),
                    loss,
                    duplicate,
                    delay,
                    recorded: Mutex::new(None),
                }),
            }
        }

        /// The network, recording every record sent on it from now on, as
        /// the network saw it (see recorded): MemoryNetwork::new(...)
        /// .with_recording().
        pub fn with_recording(self) -> Self {
            lock(&self.inner.recorded).get_or_insert_with(Vec::new);
            self
        }

        /// Every record sent since with_recording, in order.
        pub fn recorded(&self) -> Vec<Vec<u8>> {
            lock(&self.inner.recorded).clone().unwrap_or_default()
        }

        /// A transport whose node skips the handshake and sends frames in
        /// the clear: for tests of the frame layer only. Only an in-memory
        /// network makes one, and no configuration selects it.
        pub fn insecure_transport_for_tests(&self, name: &str) -> Arc<dyn Transport> {
            Arc::new(InsecureMemoryTransport(MemoryTransport { network: self.clone(), address: format!("mem://{name}") }))
        }

        /// A network without faults.
        pub fn reliable() -> Self {
            Self::new(0, 0.0, 0.0, Duration::ZERO)
        }

        /// A transport for a node named name, at mem://name.
        pub fn transport(&self, name: &str) -> Arc<dyn Transport> {
            Arc::new(MemoryTransport { network: self.clone(), address: format!("mem://{name}") })
        }

        /// Only nodes named in the same group reach each other.
        pub fn partition(&self, groups: &[&[&str]]) {
            lock(&self.inner.state).groups =
                Some(groups.iter().map(|g| g.iter().map(|n| format!("mem://{n}")).collect()).collect());
        }

        pub fn heal(&self) {
            lock(&self.inner.state).groups = None;
        }

        fn send(&self, source: &str, node: &str, frame: Vec<u8>) -> Result<()> {
            if let Some(recorded) = lock(&self.inner.recorded).as_mut() {
                recorded.push(frame.clone());
            }
            let (deliver, delays) = {
                let mut state = lock(&self.inner.state);
                let Some(deliver) = state.nodes.get(node).cloned() else {
                    return Err(format!("{UNREACHABLE}no node at {node}"));
                };
                if let Some(groups) = &state.groups {
                    if !groups.iter().any(|g| g.contains(source) && g.contains(node)) {
                        return Ok(());
                    }
                }
                let chance = |random: &mut SplitMix64, p: f64| p > 0.0 && (random.below(1 << 30) as f64) < p * (1u64 << 30) as f64;
                if chance(&mut state.random, self.inner.loss) {
                    return Ok(());
                }
                let copies = if chance(&mut state.random, self.inner.duplicate) { 2 } else { 1 };
                let delays: Vec<Duration> =
                    (0..copies).map(|_| self.inner.delay.mul_f64(state.random.below(1001) as f64 / 1000.0)).collect();
                (deliver, delays)
            };
            for wait in delays {
                let (deliver, frame) = (deliver.clone(), frame.clone());
                std::thread::spawn(move || {
                    if !wait.is_zero() {
                        std::thread::sleep(wait);
                    }
                    deliver(frame);
                });
            }
            Ok(())
        }
    }

    struct MemoryTransport {
        network: MemoryNetwork,
        address: String,
    }

    impl Transport for MemoryTransport {
        fn address(&self) -> String {
            self.address.clone()
        }

        fn start(&self, deliver: Deliver) {
            lock(&self.network.inner.state).nodes.insert(self.address.clone(), deliver);
        }

        fn send(&self, node: &str, frame: Vec<u8>) -> Result<()> {
            self.network.send(&self.address, node, frame)
        }

        fn close(&self) {
            lock(&self.network.inner.state).nodes.remove(&self.address);
        }
    }

    /// In memory, without the handshake: tests only (see
    /// MemoryNetwork::insecure_transport_for_tests).
    pub struct InsecureMemoryTransport(MemoryTransport);

    impl Transport for InsecureMemoryTransport {
        fn address(&self) -> String {
            self.0.address()
        }

        fn start(&self, deliver: Deliver) {
            self.0.start(deliver)
        }

        fn send(&self, node: &str, frame: Vec<u8>) -> Result<()> {
            self.0.send(node, frame)
        }

        fn close(&self) {
            self.0.close()
        }

        fn insecure_for_tests(&self) -> Option<sealed::InsecureForTests> {
            Some(sealed::InsecureForTests)
        }
    }

    // A listening socket polled until closed, so close needs no wake-up.
    struct Listener {
        socket: std::net::TcpListener,
        closed: Arc<AtomicBool>,
        accepted: Arc<Mutex<Vec<std::net::TcpStream>>>,
    }

    impl Listener {
        fn bind(host: &str, port: u16) -> Result<(Listener, u16)> {
            let socket = std::net::TcpListener::bind((host, port)).map_err(|e| format!("cannot listen on {host}:{port}: {e}"))?;
            let port = socket.local_addr().map_err(|e| e.to_string())?.port();
            socket.set_nonblocking(true).map_err(|e| e.to_string())?;
            Ok((Listener { socket, closed: Arc::new(AtomicBool::new(false)), accepted: Arc::new(Mutex::new(Vec::new())) }, port))
        }

        // Accepts connections until closed, handing each to serve on its own thread.
        fn run(&self, serve: Arc<dyn Fn(std::net::TcpStream) + Send + Sync>) {
            let socket = match self.socket.try_clone() {
                Ok(socket) => socket,
                Err(_) => return,
            };
            let (closed, accepted) = (self.closed.clone(), self.accepted.clone());
            std::thread::spawn(move || {
                while !closed.load(Ordering::SeqCst) {
                    match socket.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            if let Ok(copy) = stream.try_clone() {
                                lock(&accepted).push(copy);
                            }
                            let serve = serve.clone();
                            std::thread::spawn(move || serve(stream));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
        }

        fn close(&self) {
            self.closed.store(true, Ordering::SeqCst);
            for stream in lock(&self.accepted).drain(..) {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    fn read_exactly(stream: &mut std::net::TcpStream, n: usize) -> Option<Vec<u8>> {
        let mut data = vec![0u8; n];
        stream.read_exact(&mut data).ok()?;
        Some(data)
    }

    fn host_port(rest: &str) -> Result<(String, u16)> {
        let rest = rest.split('/').next().unwrap_or(rest);
        match rest.rsplit_once(':') {
            Some((host, port)) => Ok((host.to_string(), port.parse().map_err(|_| format!("{UNREACHABLE}bad port in {rest}"))?)),
            None => Err(format!("{UNREACHABLE}no port in {rest}")),
        }
    }

    fn connect(host: &str, port: u16) -> std::io::Result<std::net::TcpStream> {
        use std::net::ToSocketAddrs;
        let mut last = std::io::Error::other("no address");
        for address in (host, port).to_socket_addrs()? {
            match std::net::TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    return Ok(stream);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// Frames over TCP, each a 4-byte big-endian length then the frame.
    /// Port 0 picks a free port; the address is tcp://host:port.
    pub struct TcpTransport {
        listener: Listener,
        address: String,
        connections: Mutex<HashMap<String, std::net::TcpStream>>,
    }

    impl TcpTransport {
        pub fn new(host: &str, port: u16) -> Result<Arc<dyn Transport>> {
            let (listener, port) = Listener::bind(host, port)?;
            Ok(Arc::new(TcpTransport { listener, address: format!("tcp://{host}:{port}"), connections: Mutex::new(HashMap::new()) }))
        }

        /// On 127.0.0.1, at a free port.
        pub fn local() -> Result<Arc<dyn Transport>> {
            Self::new("127.0.0.1", 0)
        }
    }

    impl Transport for TcpTransport {
        fn address(&self) -> String {
            self.address.clone()
        }

        fn start(&self, deliver: Deliver) {
            self.listener.run(Arc::new(move |mut stream: std::net::TcpStream| {
                while let Some(header) = read_exactly(&mut stream, 4) {
                    let n = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
                    let Some(frame) = read_exactly(&mut stream, n) else { return };
                    deliver(frame);
                }
            }));
        }

        fn send(&self, node: &str, frame: Vec<u8>) -> Result<()> {
            let (host, port) = host_port(node.trim_start_matches("tcp://"))?;
            let mut data = (frame.len() as u32).to_be_bytes().to_vec();
            data.extend_from_slice(&frame);
            let mut connections = lock(&self.connections);
            for attempt in 0..2 {
                let stream = match connections.get_mut(node) {
                    Some(stream) => stream,
                    None => match connect(&host, port) {
                        Ok(stream) => connections.entry(node.to_string()).or_insert(stream),
                        Err(e) if attempt == 1 => return Err(format!("{UNREACHABLE}cannot reach {node}: {e}")),
                        Err(_) => continue,
                    },
                };
                match stream.write_all(&data) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        connections.remove(node);
                        if attempt == 1 {
                            return Err(format!("{UNREACHABLE}cannot reach {node}: {e}"));
                        }
                    }
                }
            }
            Ok(())
        }

        fn close(&self) {
            self.listener.close();
            for (_, stream) in lock(&self.connections).drain() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    /// Frames as HTTP POST bodies to /lawspec; the address is http://host:port.
    pub struct HttpTransport {
        listener: Listener,
        address: String,
    }

    impl HttpTransport {
        pub fn new(host: &str, port: u16) -> Result<Arc<dyn Transport>> {
            let (listener, port) = Listener::bind(host, port)?;
            Ok(Arc::new(HttpTransport { listener, address: format!("http://{host}:{port}") }))
        }

        /// On 127.0.0.1, at a free port.
        pub fn local() -> Result<Arc<dyn Transport>> {
            Self::new("127.0.0.1", 0)
        }
    }

    // An HTTP/1.1 message's head (start line and headers) and its body.
    fn read_http(stream: &mut std::net::TcpStream) -> Option<(String, Vec<u8>)> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).ok()? == 0 {
                return None;
            }
            head.push(byte[0]);
            if head.len() > 65536 {
                return None;
            }
        }
        let head = String::from_utf8_lossy(&head).to_string();
        let length = head
            .lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let body = read_exactly(stream, length)?;
        Some((head, body))
    }

    impl Transport for HttpTransport {
        fn address(&self) -> String {
            self.address.clone()
        }

        fn start(&self, deliver: Deliver) {
            self.listener.run(Arc::new(move |mut stream: std::net::TcpStream| {
                while let Some((head, body)) = read_http(&mut stream) {
                    let first = head.lines().next().unwrap_or("").to_string();
                    let ours = first.starts_with("POST /lawspec ");
                    let status = if ours { "204 No Content" } else { "404 Not Found" };
                    if stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes()).is_err() {
                        return;
                    }
                    if ours {
                        deliver(body);
                    }
                    if head.to_ascii_lowercase().contains("connection: close") {
                        return;
                    }
                }
            }));
        }

        fn send(&self, node: &str, frame: Vec<u8>) -> Result<()> {
            let (host, port) = host_port(node.trim_start_matches("http://"))?;
            let unreachable = |e: &dyn std::fmt::Display| format!("{UNREACHABLE}cannot reach {node}: {e}");
            let mut stream = connect(&host, port).map_err(|e| unreachable(&e))?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let request = format!(
                "POST /lawspec HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                frame.len()
            );
            stream.write_all(request.as_bytes()).map_err(|e| unreachable(&e))?;
            stream.write_all(&frame).map_err(|e| unreachable(&e))?;
            let (head, _) = read_http(&mut stream).ok_or_else(|| unreachable(&"no answer"))?;
            let status = head.split_whitespace().nth(1).unwrap_or("");
            if status.starts_with('2') { Ok(()) } else { Err(unreachable(&format!("HTTP {status}"))) }
        }

        fn close(&self) {
            self.listener.close();
        }
    }

    // The secure network handler lives in lawspec_network, which the compiler
    // writes beside this runtime when a program imports lawspec.network: it
    // needs the crypto libraries, which programs without nodes do without.
    // Its install registers it here (register_secure_network); the generated
    // tests of such a program call it.

    /// A node's secure layer (lawspec_network's): send seals a frame for a
    /// peer (or queues it while a handshake runs); receive gives the frame a
    /// record carries, or None for a handshake record or one that fails.
    pub trait SecureLayer: Send + Sync {
        fn send(&self, peer: &str, frame: Vec<u8>) -> Result<()>;
        fn receive(&self, record: &[u8]) -> Option<Vec<u8>>;
        fn as_any(&self) -> &dyn std::any::Any;
    }

    /// The secure network handler: layer makes a node's secure layer (by
    /// default identity and trust), token a one-time take token from its
    /// secure generator.
    pub trait SecureNetwork: Send + Sync {
        fn layer(&self, node: NodeLink) -> Result<Box<dyn SecureLayer>>;
        fn token(&self) -> String;
    }

    static SECURE_NETWORK: std::sync::OnceLock<Arc<dyn SecureNetwork>> = std::sync::OnceLock::new();

    /// Registers the secure network handler; the first one stays.
    pub fn register_secure_network(provider: Arc<dyn SecureNetwork>) {
        let _ = SECURE_NETWORK.set(provider);
    }

    /// The error of a node made without the secure network handler.
    pub const NO_SECURE_NETWORK: &str = "a node needs the secure network handler: add `import lawspec.network` \
        to a unit of the program, and call lawspec_network::install()";

    fn secure_network() -> Result<Arc<dyn SecureNetwork>> {
        SECURE_NETWORK.get().cloned().ok_or_else(|| NO_SECURE_NETWORK.to_string())
    }

    /// What a secure layer holds of its node: its address, sending a record
    /// to a peer node, and whether the node has closed. It does not keep the
    /// node alive.
    #[derive(Clone)]
    pub struct NodeLink {
        node: Weak<NodeInner>,
        address: String,
    }

    impl NodeLink {
        pub fn address(&self) -> &str {
            &self.address
        }

        /// Sends a record as it is (an error starting UNREACHABLE when it
        /// cannot, or once the node is gone).
        pub fn send(&self, peer: &str, record: Vec<u8>) -> Result<()> {
            match self.node.upgrade() {
                Some(node) => node.transport.send(peer, record),
                None => Err(format!("{UNREACHABLE}{} has closed", self.address)),
            }
        }

        pub fn closed(&self) -> bool {
            self.node.upgrade().is_none_or(|node| node.closed.load(Ordering::SeqCst))
        }
    }

    /// A one-time token from a secure generator: 32 bytes as 64 lowercase
    /// hexadecimal digits, as SecureRandom's secureToken gives. The secure
    /// network handler's generator when it is installed, else the operating
    /// system's (/dev/urandom).
    pub fn secure_token() -> String {
        if let Some(provider) = SECURE_NETWORK.get() {
            return provider.token();
        }
        let mut bytes = [0u8; 32];
        match std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)) {
            Ok(()) => hex(&bytes),
            Err(error) => panic!(
                "a take token needs a secure generator: /dev/urandom ({error}); add `import lawspec.network` and call lawspec_network::install()"
            ),
        }
    }

    // Something a node names: it handles the frames sent to it.
    trait Entity: Send + Sync {
        fn receive(&self, node: &Node, kind: &str, source: &str, id: u64, payload: Vec<u8>);
    }

    struct Slot {
        reply: Mutex<Option<Vec<u8>>>,
        ready: Condvar,
    }

    /// A process's presence on a network: it names local mailboxes, actors,
    /// channel ends and definitions, so other nodes can reach them at
    /// <node address>/<name>, and it sends to theirs. Order is kept within one
    /// channel; a mailbox send is best effort, and a request is sent again
    /// until answered (the receiver runs it once), failing with UNREACHABLE
    /// after its timeout. Clones share the node.
    #[derive(Clone)]
    pub struct Node {
        inner: Arc<NodeInner>,
    }

    struct NodeInner {
        transport: Arc<dyn Transport>,
        address: String,
        // None only on the transport made for tests that skips the handshake.
        secure: Option<Box<dyn SecureLayer>>,
        entities: Mutex<HashMap<String, Arc<dyn Entity>>>,
        pending: Mutex<HashMap<u64, Arc<Slot>>>,
        // Requests seen, by sender and id, with their reply once sent.
        seen: Mutex<(HashMap<(String, u64), Option<Vec<u8>>>, VecDeque<(String, u64)>)>,
        ids: AtomicU64,
        closed: AtomicBool,
    }

    impl Node {
        /// A node on transport. Unless it is the transport made for tests
        /// only (MemoryNetwork::insecure_transport_for_tests), the node is
        /// secure: the secure network handler (lawspec_network, installed)
        /// gives it the identity lawspec.json binds, or a fresh one. Panics
        /// without that handler, or when lawspec-network.conf is unreadable;
        /// lawspec_network::node takes an identity and trusted peers.
        pub fn new(transport: Arc<dyn Transport>) -> Node {
            Node::with_secure_layer(transport, |link| {
                secure_network().and_then(|provider| provider.layer(link)).unwrap_or_else(|error| panic!("{error}"))
            })
        }

        /// A node whose secure layer make gives (it is not called for the
        /// transport made for tests only, whose node sends frames in the
        /// clear; no other transport can skip the layer).
        pub fn with_secure_layer(transport: Arc<dyn Transport>, make: impl FnOnce(NodeLink) -> Box<dyn SecureLayer>) -> Node {
            let insecure = transport.insecure_for_tests().is_some();
            let address = transport.address();
            let inner = Arc::new_cyclic(|weak: &Weak<NodeInner>| NodeInner {
                secure: if insecure { None } else { Some(make(NodeLink { node: weak.clone(), address: address.clone() })) },
                address,
                transport: transport.clone(),
                entities: Mutex::new(HashMap::new()),
                pending: Mutex::new(HashMap::new()),
                seen: Mutex::new((HashMap::new(), VecDeque::new())),
                ids: AtomicU64::new(1),
                closed: AtomicBool::new(false),
            });
            let weak: Weak<NodeInner> = Arc::downgrade(&inner);
            transport.start(Arc::new(move |record| {
                if let Some(inner) = weak.upgrade() {
                    Node { inner }.arrive(record);
                }
            }));
            Node { inner }
        }

        pub fn address(&self) -> String {
            self.inner.address.clone()
        }

        /// The node's secure layer; None on the transport made for tests only.
        pub fn secure_layer(&self) -> Option<&dyn SecureLayer> {
            self.inner.secure.as_deref()
        }

        // A record from the transport: a frame in the clear on the transport
        // made for tests, else a handshake or sealed record.
        fn arrive(&self, record: Vec<u8>) {
            match &self.inner.secure {
                None => self.deliver(record),
                Some(secure) => {
                    if let Some(frame) = secure.receive(&record) {
                        self.deliver(frame);
                    }
                }
            }
        }

        fn transmit(&self, node: &str, frame: Vec<u8>) -> Result<()> {
            match &self.inner.secure {
                None => self.inner.transport.send(node, frame),
                Some(secure) => secure.send(node, frame),
            }
        }

        pub fn close(&self) {
            self.inner.closed.store(true, Ordering::SeqCst);
            self.inner.transport.close();
        }

        fn closed(&self) -> bool {
            self.inner.closed.load(Ordering::SeqCst)
        }

        fn next_id(&self) -> u64 {
            self.inner.ids.fetch_add(1, Ordering::SeqCst)
        }

        fn send_frame(&self, address: &str, kind: &str, payload: &[u8], id: u64) -> Result<()> {
            let (node, name) = split_address(address)?;
            self.transmit(&node, frame_encode(kind, &name, &self.inner.address, id, payload))
        }

        fn register(&self, name: &str, entity: Arc<dyn Entity>) -> Result<String> {
            if name.is_empty() || name.contains('/') {
                return Err(format!("{name:?} is not a name: use letters, digits and dashes"));
            }
            let mut entities = lock(&self.inner.entities);
            if entities.contains_key(name) {
                return Err(format!("{name} is already registered on {}", self.inner.address));
            }
            entities.insert(name.to_string(), entity);
            Ok(format!("{}/{name}", self.inner.address))
        }

        fn deliver(&self, frame: Vec<u8>) {
            let Ok(Frame { kind, to, source, id, payload }) = frame_decode(&frame) else { return };
            if kind == "reply" {
                if let Some(slot) = lock(&self.inner.pending).remove(&id) {
                    *lock(&slot.reply) = Some(payload);
                    slot.ready.notify_all();
                }
                return;
            }
            let entity = lock(&self.inner.entities).get(&to).cloned();
            let Some(entity) = entity else {
                if id != 0 {
                    self.reply(&source, id, 3, format!("nothing is registered as {to} on {}", self.inner.address).as_bytes());
                }
                return;
            };
            if id != 0 {
                let key = (source.clone(), id);
                let mut seen = lock(&self.inner.seen);
                if let Some(answer) = seen.0.get(&key) {
                    if let Some(answer) = answer.clone() {
                        let node = self.clone();
                        std::thread::spawn(move || {
                            let _ = node.send_frame(&format!("{source}/"), "reply", &answer, id);
                        });
                    }
                    return;
                }
                seen.0.insert(key.clone(), None);
                seen.1.push_back(key);
                while seen.1.len() > 10000 {
                    if let Some(old) = seen.1.pop_front() {
                        seen.0.remove(&old);
                    }
                }
            }
            // Handled off the transport's thread, so a slow handler does not
            // hold up other frames.
            let node = self.clone();
            std::thread::spawn(move || entity.receive(&node, &kind, &source, id, payload));
        }

        fn reply(&self, source: &str, id: u64, status: u8, body: &[u8]) {
            let mut payload = vec![status];
            payload.extend_from_slice(body);
            {
                let mut seen = lock(&self.inner.seen);
                if let Some(answer) = seen.0.get_mut(&(source.to_string(), id)) {
                    *answer = Some(payload.clone());
                }
            }
            let _ = self.send_frame(&format!("{source}/"), "reply", &payload, id);
        }

        /// Sends a request again until answered: (status, body).
        fn request(&self, address: &str, kind: &str, payload: &[u8], timeout: Duration) -> Result<(u8, Vec<u8>)> {
            let id = self.next_id();
            let slot = Arc::new(Slot { reply: Mutex::new(None), ready: Condvar::new() });
            lock(&self.inner.pending).insert(id, slot.clone());
            let give_up = Instant::now() + timeout;
            loop {
                if let Err(e) = self.send_frame(address, kind, payload, id) {
                    if !is_unreachable(&e) {
                        lock(&self.inner.pending).remove(&id);
                        return Err(e);
                    }
                }
                let now = Instant::now();
                let wait = Duration::from_millis(100).min(give_up.saturating_duration_since(now));
                let mut reply = lock(&slot.reply);
                if reply.is_none() {
                    reply = slot.ready.wait_timeout(reply, wait).unwrap_or_else(|e| e.into_inner()).0;
                }
                if let Some(answer) = reply.take() {
                    if answer.is_empty() {
                        return Err(format!("{UNREACHABLE}an empty reply from {address}"));
                    }
                    return Ok((answer[0], answer[1..].to_vec()));
                }
                drop(reply);
                if Instant::now() >= give_up {
                    lock(&self.inner.pending).remove(&id);
                    return Err(format!("{UNREACHABLE}{address} did not answer within {:?}", timeout));
                }
            }
        }

        // Mailboxes: values of one type sent by any node.

        /// A local mailbox that other nodes send to at <address>/name.
        pub fn mailbox(&self, name: &str, descriptor: Sexp, values: Values) -> Result<actors::Mailbox<Value>> {
            let mailbox = actors::Mailbox::new();
            self.register(name, Arc::new(MailEntity { mailbox: mailbox.clone(), descriptor, values }))?;
            Ok(mailbox)
        }

        pub fn remote_mailbox(&self, address: &str, descriptor: Sexp, values: Values) -> RemoteMailbox {
            self.remote_mailbox_within(address, descriptor, values, Duration::from_secs(5))
        }

        /// A remote mailbox whose sends wait up to timeout for delivery.
        pub fn remote_mailbox_within(&self, address: &str, descriptor: Sexp, values: Values, timeout: Duration) -> RemoteMailbox {
            RemoteMailbox { node: self.clone(), address: address.to_string(), descriptor, values, timeout }
        }

        // Actors: calls by message name, with each message's types.

        /// Lets other nodes call handlers at <address>/name; returns that
        /// address. Each handler takes the message's logical arguments and
        /// gives its logical reply (typically by calling a local actor).
        pub fn serve(&self, name: &str, handlers: Vec<RemoteHandler>, values: Values) -> Result<String> {
            self.register(name, Arc::new(ActorEntity { handlers, values }))
        }

        /// A proxy calling the actor at address.
        pub fn remote_actor(&self, address: &str, signatures: Vec<Signature>, values: Values, timeout: Duration) -> RemoteActor {
            RemoteActor { node: self.clone(), address: address.to_string(), signatures, values, timeout }
        }

        // Definitions, by content hash.

        /// Lets other nodes evaluate definitions, by content hash.
        pub fn serve_definitions(&self, table: Vec<RemoteDefinition>, values: Values, name: &str) -> Result<String> {
            self.register(name, Arc::new(DefinitionEntity { table, values }))
        }

        /// Evaluates the definition with this content hash on another node.
        #[allow(clippy::too_many_arguments)]
        pub fn evaluate(
            &self,
            node: &str,
            digest: &str,
            args: &[Value],
            arguments: &[Sexp],
            result: &Sexp,
            values: &Values,
            timeout: Duration,
            name: &str,
        ) -> Result<Value> {
            let mut payload = Vec::new();
            put_text(&mut payload, digest);
            for (d, v) in arguments.iter().zip(args) {
                wire_put(values, d, v, &mut payload)?;
            }
            let (status, body) = self.request(&format!("{node}/{name}"), "eval", &payload, timeout)?;
            reply_value(status, &body, values, result)
        }

        // Channels: one side here, the other on any node.

        /// The first end of a channel named name here; its other end is
        /// dialed from any node. steps: (sends, descriptor) per step, from
        /// this end's side.
        pub fn listen(&self, name: &str, steps: Vec<(bool, Sexp)>, values: Values, deadline: Duration) -> Result<NetEndpoint> {
            let endpoint = NetEndpoint::new(self, steps, values, deadline);
            let address = self.register(name, endpoint.inner.clone())?;
            *lock(&endpoint.inner.address) = address;
            Ok(endpoint)
        }

        /// The second end of the channel listening at address; steps are
        /// from this end's side.
        pub fn dial(&self, address: &str, steps: Vec<(bool, Sexp)>, values: Values, deadline: Duration) -> Result<NetEndpoint> {
            let endpoint = NetEndpoint::new(self, steps, values, deadline);
            let own = self.register(&format!("end-{}", self.next_id()), endpoint.inner.clone())?;
            *lock(&endpoint.inner.address) = own;
            endpoint.connect(address);
            Ok(endpoint)
        }

        /// Takes over a channel end another node moves here: address is
        /// <old address>?take=<token>, as that node offered it. Returns once
        /// the end's state has arrived and its peer has been told (or after
        /// the deadline; the old node then forwards to the end).
        pub fn take(&self, address: &str, steps: Vec<(bool, Sexp)>, values: Values, deadline: Duration) -> Result<NetEndpoint> {
            let endpoint = NetEndpoint::new(self, steps, values, deadline);
            let own = self.register(&format!("end-{}", self.next_id()), endpoint.inner.clone())?;
            *lock(&endpoint.inner.address) = own;
            endpoint.take_over(address);
            Ok(endpoint)
        }

        /// Passes a frame on to address unchanged, keeping its source.
        fn forward(&self, address: &str, kind: &str, source: &str, id: u64, payload: &[u8]) {
            if let Ok((node, name)) = split_address(address) {
                let _ = self.transmit(&node, frame_encode(kind, &name, source, id, payload));
            }
        }
    }

    fn reply_value(status: u8, body: &[u8], values: &Values, d: &Sexp) -> Result<Value> {
        let message = String::from_utf8_lossy(body).to_string();
        match status {
            0 => wire_decode(values, d, body),
            1 if message.starts_with(actors::CRASHED) => Err(message),
            1 => Err(format!("{}{message}", actors::CRASHED)),
            2 => Err(actors::STOPPED.to_string()),
            _ if is_unreachable(&message) => Err(message),
            _ => Err(format!("{UNREACHABLE}{message}")),
        }
    }

    struct MailEntity {
        mailbox: actors::Mailbox<Value>,
        descriptor: Sexp,
        values: Values,
    }

    impl Entity for MailEntity {
        fn receive(&self, node: &Node, kind: &str, source: &str, id: u64, payload: Vec<u8>) {
            if kind != "mail" {
                return;
            }
            let (status, body) = match wire_decode(&self.values, &self.descriptor, &payload) {
                Ok(value) => match self.mailbox.send(value) {
                    Ok(()) => (0u8, Vec::new()),
                    Err(e) => (2, e.into_bytes()),
                },
                Err(e) => (3, format!("not a message of this mailbox: {e}").into_bytes()),
            };
            if id != 0 {
                node.reply(source, id, status, &body);
            }
        }
    }

    /// Sends to a mailbox on another node. A send waits until the mailbox
    /// has the message (resending a lost one; the mailbox takes it once),
    /// and fails with UNREACHABLE after the timeout, or as stopped if the
    /// mailbox is closed.
    pub struct RemoteMailbox {
        node: Node,
        pub address: String,
        descriptor: Sexp,
        values: Values,
        pub timeout: Duration,
    }

    impl RemoteMailbox {
        pub fn send(&self, value: &Value) -> Result<()> {
            let bytes = wire_encode(&self.values, &self.descriptor, value)?;
            let (status, body) = self.node.request(&self.address, "mail", &bytes, self.timeout)?;
            if status == 0 {
                Ok(())
            } else {
                reply_value(status, &body, &self.values, &Sexp::List(vec![Sexp::Atom("unit".into())])).map(|_| ())
            }
        }
    }

    /// A served message: its name, argument and reply descriptors, and
    /// what handles it (logical arguments in, logical reply out).
    pub type RemoteHandler = (String, Vec<Sexp>, Sexp, Arc<dyn Fn(Vec<Value>) -> Result<Value> + Send + Sync>);
    /// A message's name, argument descriptors and reply descriptor.
    pub type Signature = (String, Vec<Sexp>, Sexp);
    /// A definition other nodes evaluate: its content hash, the function
    /// (logical arguments in, logical result out), and its descriptors.
    pub type RemoteDefinition = (String, Arc<dyn Fn(Vec<Value>) -> Result<Value> + Send + Sync>, Vec<Sexp>, Sexp);

    struct ActorEntity {
        handlers: Vec<RemoteHandler>,
        values: Values,
    }

    impl Entity for ActorEntity {
        fn receive(&self, node: &Node, kind: &str, source: &str, id: u64, payload: Vec<u8>) {
            if kind != "call" {
                return;
            }
            let mut pos = 0;
            let decoded = get_text(&payload, &mut pos).and_then(|message| {
                let handler = self.handlers.iter().find(|h| h.0 == message).ok_or_else(|| format!("no message {message}"))?;
                let mut args = Vec::new();
                for d in &handler.1 {
                    args.push(wire_get(&self.values, d, &payload, &mut pos)?);
                }
                if pos != payload.len() {
                    return wire_error("extra bytes after the arguments");
                }
                Ok((handler, args))
            });
            let (handler, args) = match decoded {
                Ok(found) => found,
                Err(e) => return node.reply(source, id, 3, format!("not a message this actor handles: {e}").as_bytes()),
            };
            match (handler.3)(args).and_then(|reply| wire_encode(&self.values, &handler.2, &reply)) {
                Ok(bytes) => node.reply(source, id, 0, &bytes),
                Err(e) if actors::is_stopped(&e) => node.reply(source, id, 2, e.as_bytes()),
                Err(e) if actors::is_crashed(&e) => node.reply(source, id, 1, e.as_bytes()),
                Err(e) => node.reply(source, id, 1, format!("{}{e}", actors::CRASHED).as_bytes()),
            }
        }
    }

    /// Calls an actor on another node: call sends the message and waits for
    /// the reply, failing with UNREACHABLE after the timeout, or as the
    /// actor's call failed (crashed, stopped).
    pub struct RemoteActor {
        node: Node,
        pub address: String,
        signatures: Vec<Signature>,
        values: Values,
        pub timeout: Duration,
    }

    impl RemoteActor {
        pub fn call(&self, message: &str, args: &[Value]) -> Result<Value> {
            let (_, arguments, reply) =
                self.signatures.iter().find(|s| s.0 == message).ok_or_else(|| format!("no message {message}"))?;
            let mut payload = Vec::new();
            put_text(&mut payload, message);
            for (d, v) in arguments.iter().zip(args) {
                wire_put(&self.values, d, v, &mut payload)?;
            }
            let (status, body) = self.node.request(&self.address, "call", &payload, self.timeout)?;
            reply_value(status, &body, &self.values, reply)
        }
    }

    struct DefinitionEntity {
        table: Vec<RemoteDefinition>,
        values: Values,
    }

    impl Entity for DefinitionEntity {
        fn receive(&self, node: &Node, kind: &str, source: &str, id: u64, payload: Vec<u8>) {
            if kind != "eval" {
                return;
            }
            let mut pos = 0;
            let decoded = get_text(&payload, &mut pos).and_then(|digest| {
                let entry = self.table.iter().find(|e| e.0 == digest).ok_or_else(|| "no such definition".to_string())?;
                let mut args = Vec::new();
                for d in &entry.2 {
                    args.push(wire_get(&self.values, d, &payload, &mut pos)?);
                }
                Ok((entry, args))
            });
            let (entry, args) = match decoded {
                Ok(found) => found,
                Err(_) => return node.reply(source, id, 3, b"this node has no definition with that content hash"),
            };
            match (entry.1)(args).and_then(|result| wire_encode(&self.values, &entry.3, &result)) {
                Ok(bytes) => node.reply(source, id, 0, &bytes),
                Err(e) => node.reply(source, id, 1, e.as_bytes()),
            }
        }
    }

    enum Inbox {
        Value(Vec<u8>),
        Failed(String),
    }

    struct Unacked {
        payload: Vec<u8>,
        first: Instant,
        last: Instant,
        body: Vec<u8>,
    }

    struct EndpointState {
        peer: Option<String>,
        out: i64,
        unacked: HashMap<i64, Unacked>,
        expected: i64,
        early: HashMap<i64, Vec<u8>>,
        step: usize,
        gone: bool,
        failure: String,
        // Moving: the addresses this end had before (oldest first), the
        // token a taker must show, where the end went and the state frame it
        // was given, and, on the new node, the takeover in progress.
        history: Vec<String>,
        token: Option<String>,
        moved_to: Option<String>,
        handed: Vec<u8>,
        taking: Option<String>,
        taken: bool,
        announcing: bool,
        announced_at: Option<Instant>,
        confirmed: bool,
    }

    struct EndpointInner {
        node: Node,
        steps: Vec<(bool, Sexp)>,
        values: Values,
        deadline: Duration,
        address: Mutex<String>,
        state: Mutex<EndpointState>,
        changed: Condvar,
        inbox: Mutex<VecDeque<Inbox>>,
        arrived: Condvar,
    }

    /// One end of a channel between nodes. Each value travels in a numbered
    /// frame that is sent again until acknowledged, so loss, duplication
    /// and reordering are repaired; a peer silent past the deadline fails
    /// the end (PEER_FAILED). Order is kept within the channel. Clones share
    /// the end.
    ///
    /// An unused end can move to another node: offer gives the address the
    /// new node takes it over from (<address>?take=<token>). On a take frame
    /// with that token, this end hands its state over (a state frame) and
    /// from then on forwards every frame it gets to the new end; the new end
    /// tells the peer (a moved frame) so the peer sends to it directly.
    #[derive(Clone)]
    pub struct NetEndpoint {
        inner: Arc<EndpointInner>,
    }

    impl NetEndpoint {
        fn new(node: &Node, steps: Vec<(bool, Sexp)>, values: Values, deadline: Duration) -> NetEndpoint {
            let inner = Arc::new(EndpointInner {
                node: node.clone(),
                steps,
                values,
                deadline,
                address: Mutex::new(String::new()),
                state: Mutex::new(EndpointState {
                    peer: None,
                    out: 0,
                    unacked: HashMap::new(),
                    expected: 0,
                    early: HashMap::new(),
                    step: 0,
                    gone: false,
                    failure: String::new(),
                    history: Vec::new(),
                    token: None,
                    moved_to: None,
                    handed: Vec::new(),
                    taking: None,
                    taken: false,
                    announcing: false,
                    announced_at: None,
                    confirmed: false,
                }),
                changed: Condvar::new(),
                inbox: Mutex::new(VecDeque::new()),
                arrived: Condvar::new(),
            });
            let weak = Arc::downgrade(&inner);
            std::thread::spawn(move || resend(weak));
            NetEndpoint { inner }
        }

        /// This end's address.
        pub fn address(&self) -> String {
            lock(&self.inner.address).clone()
        }

        fn connect(&self, address: &str) {
            lock(&self.inner.state).peer = Some(address.to_string());
            self.inner.transmit(-1, b"hello".to_vec());
        }

        fn step_descriptor(&self, sends: bool) -> Result<Sexp> {
            let mut state = lock(&self.inner.state);
            let Some((step_sends, d)) = self.inner.steps.get(state.step) else {
                return Err("this channel's protocol has ended".into());
            };
            if *step_sends != sends {
                return Err(format!("this step {}", if *step_sends { "sends" } else { "receives" }));
            }
            state.step += 1;
            Ok(d.clone())
        }

        /// Sends the next step's value.
        pub fn send(&self, value: &Value) -> Result<()> {
            if lock(&self.inner.state).gone {
                return Err(format!("{PEER_FAILED}the other end has failed"));
            }
            let d = self.step_descriptor(true)?;
            let mut body = vec![0u8];
            wire_put(&self.inner.values, &d, value, &mut body)?;
            let seq = {
                let mut state = lock(&self.inner.state);
                state.out += 1;
                state.out - 1
            };
            self.inner.transmit(seq, body);
            Ok(())
        }

        /// The next step's value, waiting up to timeout (forever when None);
        /// fails with PEER_FAILED once the other end gave up or failed.
        pub fn receive(&self, timeout: Option<Duration>) -> Result<Value> {
            let d = self.step_descriptor(false)?;
            let until = timeout.map(|t| Instant::now() + t);
            let mut inbox = lock(&self.inner.inbox);
            let item = loop {
                if let Some(item) = inbox.pop_front() {
                    break item;
                }
                inbox = match until {
                    None => self.inner.arrived.wait(inbox).unwrap_or_else(|e| e.into_inner()),
                    Some(at) => {
                        let now = Instant::now();
                        if now >= at {
                            return Err("no message arrived in time".into());
                        }
                        self.inner.arrived.wait_timeout(inbox, at - now).unwrap_or_else(|e| e.into_inner()).0
                    }
                };
            };
            match item {
                Inbox::Failed(reason) => {
                    inbox.push_front(Inbox::Failed(reason.clone()));
                    Err(format!("{PEER_FAILED}{reason}"))
                }
                Inbox::Value(body) => {
                    drop(inbox);
                    if body.first() == Some(&1) {
                        self.inner.fail("the other end gave up the conversation");
                        return Err(format!(
                            "{PEER_FAILED}the other end gave up the conversation (its process failed or abandoned it)"
                        ));
                    }
                    wire_decode(&self.inner.values, &d, body.get(1..).unwrap_or(&[]))
                }
            }
        }

        /// Gives up: the other end's receives fail after what was sent.
        pub fn abandon(&self) {
            let seq = {
                let mut state = lock(&self.inner.state);
                state.out += 1;
                state.out - 1
            };
            self.inner.transmit(seq, vec![1]);
        }

        /// The address another node takes this unused end over from.
        fn offer(&self) -> String {
            let mut state = lock(&self.inner.state);
            let token = state.token.get_or_insert_with(secure_token);
            format!("{}?take={token}", lock(&self.inner.address))
        }

        /// Takes over the end offered at address (<old address>?take=<token>):
        /// asks for its state until it comes, then tells the peer where the
        /// end is now. Returns once the peer knows, or after the deadline (the
        /// old node then keeps forwarding to this end, as a relay would).
        fn take_over(&self, address: &str) {
            let (old, query) = address.split_once('?').unwrap_or((address, ""));
            let token = query.strip_prefix("take=").unwrap_or(query).to_string();
            lock(&self.inner.state).taking = Some(token.clone());
            let mut request = Vec::new();
            put_text(&mut request, &token);
            put_text(&mut request, &self.address());
            let give_up = Instant::now() + self.inner.deadline;
            loop {
                let _ = self.inner.node.send_frame(old, "take", &request, 0);
                let state = lock(&self.inner.state);
                let (state, _) = self
                    .inner
                    .changed
                    .wait_timeout_while(state, Duration::from_millis(50), |s| !s.taken)
                    .unwrap_or_else(|e| e.into_inner());
                if state.taken {
                    break;
                }
                drop(state);
                if Instant::now() >= give_up {
                    lock(&self.inner.state).taking = None;
                    self.inner.fail("the node the end came from did not hand it over in time (unreachable)");
                    return;
                }
            }
            let state = lock(&self.inner.state);
            let _ = self
                .inner
                .changed
                .wait_timeout_while(state, give_up.saturating_duration_since(Instant::now()), |s| !s.confirmed)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    fn put_texts(out: &mut Vec<u8>, texts: &[String]) {
        put_len(out, texts.len());
        for t in texts {
            put_text(out, t);
        }
    }

    fn get_texts(buf: &[u8], pos: &mut usize) -> Result<Vec<String>> {
        let count = get_len(buf, pos)?;
        (0..count).map(|_| get_text(buf, pos)).collect()
    }

    fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
        put_len(out, bytes.len());
        out.extend_from_slice(bytes);
    }

    fn get_bytes(buf: &[u8], pos: &mut usize) -> Result<Vec<u8>> {
        match wire_get(&no_types(), &bytes_d(), buf, pos)? {
            Value::Bytes(b) => Ok(b),
            _ => wire_error("not bytes"),
        }
    }

    fn put_numbered(out: &mut Vec<u8>, mut items: Vec<(i64, Vec<u8>)>) {
        items.sort_by_key(|(seq, _)| *seq);
        put_len(out, items.len());
        for (seq, body) in items {
            put_seq(out, seq);
            put_bytes(out, &body);
        }
    }

    fn get_numbered(buf: &[u8], pos: &mut usize) -> Result<Vec<(i64, Vec<u8>)>> {
        let count = get_len(buf, pos)?;
        (0..count).map(|_| Ok((get_seq(buf, pos)?, get_bytes(buf, pos)?))).collect()
    }

    impl EndpointInner {
        fn frame(&self, seq: i64, body: &[u8]) -> Vec<u8> {
            let mut payload = Vec::new();
            put_seq(&mut payload, seq);
            put_text(&mut payload, &lock(&self.address));
            payload.extend_from_slice(body);
            payload
        }

        fn transmit(&self, seq: i64, body: Vec<u8>) {
            let payload = self.frame(seq, &body);
            let peer = {
                let mut state = lock(&self.state);
                let now = Instant::now();
                state.unacked.insert(seq, Unacked { payload: payload.clone(), first: now, last: now, body });
                state.peer.clone()
            };
            if let Some(peer) = peer {
                let _ = self.node.send_frame(&peer, "chan", &payload, 0);
            }
        }

        fn fail(&self, reason: &str) {
            {
                let mut state = lock(&self.state);
                if state.gone {
                    return;
                }
                state.gone = true;
                state.failure = reason.to_string();
                state.unacked.clear();
                lock(&self.inbox).push_back(Inbox::Failed(reason.to_string()));
            }
            self.arrived.notify_all();
        }

        /// A take frame: hands the state over once, to the first taker with
        /// the token, and answers that taker's repeats with the same state.
        fn give(&self, payload: &[u8]) {
            let mut pos = 0;
            let Ok(token) = get_text(payload, &mut pos) else { return };
            let Ok(taker) = get_text(payload, &mut pos) else { return };
            let handed = {
                let mut state = lock(&self.state);
                if state.token.as_deref() != Some(token.as_str()) {
                    return;
                }
                match state.moved_to.clone() {
                    Some(to) if to != taker => return,
                    Some(_) => {}
                    None => {
                        let address = lock(&self.address).clone();
                        let received: Vec<Vec<u8>> = lock(&self.inbox)
                            .iter()
                            .filter_map(|item| match item {
                                Inbox::Value(body) => Some(body.clone()),
                                Inbox::Failed(_) => None,
                            })
                            .collect();
                        let mut out = Vec::new();
                        put_text(&mut out, &token);
                        put_text(&mut out, &state.failure);
                        put_text(&mut out, state.peer.as_deref().unwrap_or(""));
                        let mut former = state.history.clone();
                        former.push(address);
                        put_texts(&mut out, &former);
                        put_seq(&mut out, state.out);
                        put_seq(&mut out, state.expected);
                        put_numbered(&mut out, state.unacked.iter().map(|(seq, u)| (*seq, u.body.clone())).collect());
                        put_numbered(&mut out, state.early.iter().map(|(seq, b)| (*seq, b.clone())).collect());
                        put_len(&mut out, received.len());
                        for body in &received {
                            put_bytes(&mut out, body);
                        }
                        state.moved_to = Some(taker.clone());
                        state.handed = out;
                        state.unacked.clear();
                        state.early.clear();
                    }
                }
                state.handed.clone()
            };
            let _ = self.node.send_frame(&taker, "state", &handed, 0);
        }

        fn install(&self, payload: &[u8]) {
            let parsed = (|| -> Result<_> {
                let mut pos = 0;
                let token = get_text(payload, &mut pos)?;
                let failure = get_text(payload, &mut pos)?;
                let peer = get_text(payload, &mut pos)?;
                let history = get_texts(payload, &mut pos)?;
                let out = get_seq(payload, &mut pos)?;
                let expected = get_seq(payload, &mut pos)?;
                let unacked = get_numbered(payload, &mut pos)?;
                let early = get_numbered(payload, &mut pos)?;
                let count = get_len(payload, &mut pos)?;
                let received = (0..count).map(|_| get_bytes(payload, &mut pos)).collect::<Result<Vec<_>>>()?;
                Ok((token, failure, peer, history, out, expected, unacked, early, received))
            })();
            let Ok((token, failure, peer, history, out, expected, unacked, early, received)) = parsed else { return };
            {
                let mut state = lock(&self.state);
                if state.taking.as_deref() != Some(token.as_str()) || state.taken {
                    return;
                }
                let now = Instant::now();
                let long_ago = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
                state.peer = if peer.is_empty() { None } else { Some(peer) };
                state.history = history;
                state.out = out;
                state.expected = expected;
                // Sent again from here at once, under this end's address.
                for (seq, body) in unacked {
                    let payload = self.frame(seq, &body);
                    state.unacked.insert(seq, Unacked { payload, first: now, last: long_ago, body });
                }
                state.early.extend(early);
                lock(&self.inbox).extend(received.into_iter().map(Inbox::Value));
                state.announcing = true;
                state.taken = true;
            }
            self.arrived.notify_all();
            self.changed.notify_all();
            if !failure.is_empty() {
                self.fail(&failure);
            }
        }

        /// The peer moved: from now on send to its new address.
        fn peer_moved(&self, payload: &[u8]) {
            let mut pos = 0;
            let Ok(history) = get_texts(payload, &mut pos) else { return };
            let Ok(to) = get_text(payload, &mut pos) else { return };
            let known = {
                let mut state = lock(&self.state);
                if state.peer.as_ref().is_none_or(|p| history.contains(p)) {
                    state.peer = Some(to.clone());
                }
                state.peer.as_deref() == Some(to.as_str())
            };
            if known {
                let mut answer = Vec::new();
                put_text(&mut answer, &to);
                let _ = self.node.send_frame(&to, "moved-ack", &answer, 0);
            }
        }
    }

    // Sends unacknowledged frames again until the end fails or goes away.
    fn resend(weak: Weak<EndpointInner>) {
        loop {
            std::thread::sleep(Duration::from_millis(20));
            let Some(inner) = weak.upgrade() else { return };
            if inner.node.closed() {
                return;
            }
            let now = Instant::now();
            let address = lock(&inner.address).clone();
            let (peer, due, stale, moved) = {
                let mut state = lock(&inner.state);
                if state.gone || state.moved_to.is_some() {
                    return;
                }
                if state.taking.is_some() && !state.taken {
                    continue;
                }
                let stale = state.unacked.values().any(|u| now.duration_since(u.last) > Duration::from_millis(50) && now.duration_since(u.first) > inner.deadline);
                let mut due = Vec::new();
                for u in state.unacked.values_mut() {
                    if now.duration_since(u.last) > Duration::from_millis(50) {
                        u.last = now;
                        due.push(u.payload.clone());
                    }
                }
                let mut moved = None;
                if state.announcing
                    && state.peer.is_some()
                    && !state.confirmed
                    && state.announced_at.is_none_or(|at| now.duration_since(at) > Duration::from_millis(50))
                {
                    state.announced_at = Some(now);
                    let mut payload = Vec::new();
                    put_texts(&mut payload, &state.history);
                    put_text(&mut payload, &address);
                    moved = Some(payload);
                }
                (state.peer.clone(), due, stale, moved)
            };
            if stale {
                inner.fail("the other end did not answer in time (unreachable)");
                return;
            }
            if let Some(peer) = peer {
                if let Some(moved) = moved {
                    let _ = inner.node.send_frame(&peer, "moved", &moved, 0);
                }
                for payload in due {
                    let _ = inner.node.send_frame(&peer, "chan", &payload, 0);
                }
            }
        }
    }

    impl Entity for EndpointInner {
        fn receive(&self, node: &Node, kind: &str, source: &str, id: u64, payload: Vec<u8>) {
            if kind == "take" {
                self.give(&payload);
                return;
            }
            let (forward, waiting) = {
                let state = lock(&self.state);
                (state.moved_to.clone(), state.taking.is_some() && !state.taken)
            };
            if let Some(to) = forward {
                node.forward(&to, kind, source, id, &payload);
                return;
            }
            if waiting {
                // Until the state arrives, frames are dropped: their senders
                // send them again.
                if kind == "state" {
                    self.install(&payload);
                }
                return;
            }
            let mut pos = 0;
            match kind {
                "ack" => {
                    if let Ok(seq) = get_seq(&payload, &mut pos) {
                        lock(&self.state).unacked.remove(&seq);
                    }
                    return;
                }
                "moved" => {
                    self.peer_moved(&payload);
                    return;
                }
                "moved-ack" => {
                    if get_text(&payload, &mut pos).is_ok_and(|to| to == *lock(&self.address)) {
                        lock(&self.state).confirmed = true;
                        self.changed.notify_all();
                    }
                    return;
                }
                "chan" => {}
                _ => return,
            }
            let Ok(seq) = get_seq(&payload, &mut pos) else { return };
            let Ok(sender) = get_text(&payload, &mut pos) else { return };
            let body = payload[pos..].to_vec();
            let forward = {
                let mut state = lock(&self.state);
                if state.moved_to.is_none() {
                    if seq == -1 {
                        if state.peer.is_none() {
                            state.peer = Some(sender.clone());
                        }
                    } else if seq >= state.expected && !state.early.contains_key(&seq) {
                        state.early.insert(seq, body);
                        let mut inbox = lock(&self.inbox);
                        loop {
                            let next = state.expected;
                            match state.early.remove(&next) {
                                Some(body) => {
                                    inbox.push_back(Inbox::Value(body));
                                    state.expected += 1;
                                }
                                None => break,
                            }
                        }
                    }
                }
                state.moved_to.clone()
            };
            if let Some(to) = forward {
                // Moved meanwhile: the new end acknowledges it.
                node.forward(&to, kind, source, id, &payload);
                return;
            }
            self.arrived.notify_all();
            let mut ack = Vec::new();
            put_seq(&mut ack, seq);
            let _ = node.send_frame(&sender, "ack", &ack, 0);
        }
    }

    /// Converts one session step's native value to and from its logical
    /// form, for typed ends over a network. A step that sends another
    /// protocol's first end (end_codec) carries the address the receiver
    /// takes the end over from, or a relay's for a local end, instead.
    pub struct StepCodec {
        kind: CodecKind,
    }

    /// A protocol's wire form from its first end: each step's direction and
    /// descriptor, and its codec.
    pub type Wire = fn() -> (Vec<(bool, Sexp)>, Vec<StepCodec>);

    enum CodecKind {
        Value {
            to_logical: Box<dyn Fn(super::sessions::Message) -> Value + Send + Sync>,
            from_logical: Box<dyn Fn(Value) -> Result<super::sessions::Message> + Send + Sync>,
        },
        End {
            wire: Wire,
            into_endpoint: Box<dyn Fn(super::sessions::Message) -> super::sessions::Endpoint + Send + Sync>,
            from_endpoint: Box<dyn Fn(super::sessions::Endpoint) -> super::sessions::Message + Send + Sync>,
        },
    }

    /// The codec of a step whose native type is T.
    pub fn step_codec<T: super::IntoValue + super::FromValue + Send + 'static>() -> StepCodec {
        StepCodec {
            kind: CodecKind::Value {
                to_logical: Box::new(|message| match message.downcast::<T>() {
                    Ok(value) => super::IntoValue::into_value(*value),
                    Err(_) => panic!("a session sent a value of an unexpected type"),
                }),
                from_logical: Box::new(|value| {
                    let native: T = super::FromValue::from_value(value)?;
                    Ok(Box::new(native) as super::sessions::Message)
                }),
            },
        }
    }

    /// The codec of a step that sends another protocol's first end E: wire
    /// is that protocol's wire form, and into/from turn E into its untyped
    /// endpoint and back.
    pub fn end_codec<E: Send + 'static>(
        wire: Wire,
        into: fn(E) -> super::sessions::Endpoint,
        from: fn(super::sessions::Endpoint) -> E,
    ) -> StepCodec {
        StepCodec {
            kind: CodecKind::End {
                wire,
                into_endpoint: Box::new(move |message| match message.downcast::<E>() {
                    Ok(end) => into(*end),
                    Err(_) => panic!("a session sent a channel end of an unexpected protocol"),
                }),
                from_endpoint: Box::new(move |endpoint| Box::new(from(endpoint)) as super::sessions::Message),
            },
        }
    }

    /// A typed session end's transport over a network channel end: values
    /// are converted step by step, natively on this side.
    pub struct NetSession {
        endpoint: NetEndpoint,
        codecs: Vec<StepCodec>,
        step: AtomicU64,
        moved: AtomicBool,
    }

    impl NetSession {
        pub fn new(endpoint: NetEndpoint, codecs: Vec<StepCodec>) -> Arc<NetSession> {
            Arc::new(NetSession { endpoint, codecs, step: AtomicU64::new(0), moved: AtomicBool::new(false) })
        }

        fn codec(&self) -> &StepCodec {
            let step = self.step.fetch_add(1, Ordering::SeqCst) as usize;
            self.codecs.get(step).unwrap_or_else(|| panic!("this channel's protocol has ended"))
        }
    }

    /// The text that gives an unused channel end to another node. An end
    /// that is itself between nodes moves there; a local end stays here and
    /// a relay on node carries its conversation.
    fn offer_end(node: &Node, values: &Values, end: super::sessions::Endpoint, wire: Wire) -> Result<String> {
        match end.hand_over() {
            Some(address) => Ok(address),
            None => relay_end(node, values, end, wire),
        }
    }

    /// Offers a local channel end to another node: a relay on node listens
    /// for the receiver and passes each step between it and the end, which
    /// stays here. Returns the relay's address. A failure on either side
    /// gives up the other.
    fn relay_end(node: &Node, values: &Values, end: super::sessions::Endpoint, wire: Wire) -> Result<String> {
        let (steps, codecs) = wire();
        let relay = node.listen(
            &format!("relay-{}", node.next_id()),
            steps.iter().map(|(s, d)| (!s, d.clone())).collect(),
            values.clone(),
            Duration::from_secs(5),
        )?;
        let address = relay.address();
        let relayed = NetSession::new(relay, codecs);
        let directions: Vec<bool> = steps.iter().map(|(s, _)| *s).collect();
        std::thread::spawn(move || {
            use super::sessions::{Side, Transport};
            for sends in directions {
                let passed = if sends {
                    relayed.receive(Side::First).map(|m| end.send_message(m))
                } else {
                    end.receive_message().map(|m| relayed.send(Side::First, m))
                };
                if passed.is_err() {
                    relayed.close(Side::First);
                    return;
                }
            }
        });
        Ok(address)
    }

    impl super::sessions::Transport for NetSession {
        fn send(&self, _: super::sessions::Side, message: super::sessions::Message) {
            let value = match &self.codec().kind {
                CodecKind::Value { to_logical, .. } => to_logical(message),
                CodecKind::End { wire, into_endpoint, .. } => {
                    let inner = &self.endpoint.inner;
                    match offer_end(&inner.node, &inner.values, into_endpoint(message), *wire) {
                        Ok(address) => Value::Text(address),
                        Err(e) => panic!("{e}"),
                    }
                }
            };
            if let Err(e) = self.endpoint.send(&value) {
                if !is_peer_failed(&e) {
                    panic!("{e}");
                }
            }
        }

        fn receive(&self, _: super::sessions::Side) -> std::result::Result<super::sessions::Message, super::sessions::PeerFailed> {
            let codec = self.codec();
            let value = match self.endpoint.receive(None) {
                Ok(value) => value,
                Err(e) if is_peer_failed(&e) => return Err(super::sessions::PeerFailed),
                Err(e) => panic!("{e}"),
            };
            match &codec.kind {
                CodecKind::Value { from_logical, .. } => Ok(from_logical(value).unwrap_or_else(|e| panic!("{e}"))),
                CodecKind::End { wire, from_endpoint, .. } => {
                    let Value::Text(address) = value else { panic!("a channel end arrived without its address") };
                    let (steps, codecs) = wire();
                    let inner = &self.endpoint.inner;
                    let dialed = if address.contains("?take=") {
                        inner.node.take(&address, steps, inner.values.clone(), Duration::from_secs(5))
                    } else {
                        inner.node.dial(&address, steps, inner.values.clone(), Duration::from_secs(5))
                    }
                    .unwrap_or_else(|e| panic!("{e}"));
                    let session = NetSession::new(dialed, codecs);
                    Ok(from_endpoint(super::sessions::Endpoint::on(session, super::sessions::Side::First)))
                }
            }
        }

        fn close(&self, _: super::sessions::Side) {
            if !self.moved.load(Ordering::SeqCst) {
                self.endpoint.abandon();
            }
        }

        fn hand_over(&self) -> Option<String> {
            if self.step.load(Ordering::SeqCst) != 0 {
                return None;
            }
            self.moved.store(true, Ordering::SeqCst);
            Some(self.endpoint.offer())
        }
    }

    // Keeps descriptor helpers reachable for generated code.
    pub fn parse(text: &str) -> Sexp {
        read_descriptor(text).into_iter().next().expect("a descriptor")
    }
}

#[cfg(test)]
mod net_tests {
    use super::net::*;
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    fn int64() -> Sexp {
        descriptor("(int Int64 _ _)")
    }

    #[test]
    fn wire_vectors_match_python() {
        assert_eq!(wire_encoded("(int Integer _ _)", 7, 4, 4), vec!["9ddc4f", "eefb17", "8fc703", "8ee709"]);
        assert_eq!(wire_encoded("(int Int8 -128 127)", 42, 4, 5), vec!["f901", "28", "00", "48", "5c"]);
        assert_eq!(wire_encoded("(text)", 3, 4, 3), vec!["037b5b2c", "0134", "024d54"]);
        assert_eq!(wire_encoded("(list (maybe (bool)))", 5, 3, 3), vec!["02000101", "0100", "010100"]);
        assert_eq!(wire_encoded("(either (int UInt8 0 255) (text))", 9, 3, 4), vec!["00ec02", "0000", "00d201", "0100"]);
        let shape = "(data Shape (ctor Shape::Circle (int Int32 0 100)) (ctor Shape::Box (int Int32 0 100) (int Int32 0 100)) (ctor Shape::Group (list (ref Shape))))";
        assert_eq!(wire_encoded(shape, 11, 3, 3), vec!["0030", "020201b80100015ac801", "014808"]);
        for seed in 0..20 {
            assert!(wire_round_trips(shape, seed, 5, 10));
        }
    }

    fn exercise(a: Arc<dyn Transport>, b: Arc<dyn Transport>, lossy: bool) {
        let (here, there) = (Node::new(a), Node::new(b));
        let counter = actors::Actor::new(BigInt::from(0));
        let served = counter.clone();
        let handler: Arc<dyn Fn(Vec<Value>) -> Result<Value> + Send + Sync> = Arc::new(move |args| {
            let n = args[0].integer()?;
            served.call(|s: BigInt| {
                let next = s + n;
                Ok((Value::Integer(next.clone()), next))
            })
        });
        let address = there.serve("counter", vec![("add".into(), vec![int64()], int64(), handler)], values_table("")).unwrap();
        let remote = here.remote_actor(&address, vec![("add".into(), vec![int64()], int64())], values_table(""), Duration::from_secs(3));
        let replies: Vec<String> = (1..=3).map(|n| render(&remote.call("add", &[Value::Integer(n.into())]).unwrap())).collect();
        assert_eq!(replies, vec!["1", "3", "6"]);
        let double: Arc<dyn Fn(Vec<Value>) -> Result<Value> + Send + Sync> =
            Arc::new(|args| Ok(Value::Integer(args[0].integer()? * 2)));
        there.serve_definitions(vec![("h".into(), double, vec![int64()], int64())], values_table(""), "definitions").unwrap();
        let got = here
            .evaluate(&there.address(), "h", &[Value::Integer(21.into())], &[int64()], &int64(), &values_table(""), Duration::from_secs(3), "definitions")
            .unwrap();
        assert_eq!(render(&got), "42");
        if !lossy {
            let box_ = there.mailbox("jobs", int64(), values_table("")).unwrap();
            here.remote_mailbox(&format!("{}/jobs", there.address()), int64(), values_table("")).send(&Value::Integer(41.into())).unwrap();
            assert_eq!(render(&box_.receive(Some(Duration::from_secs(3))).unwrap()), "41");
        }
        let steps = vec![(true, int64()), (false, descriptor("(text)")), (true, int64())];
        let listener = there.listen("chat", steps.clone(), values_table(""), Duration::from_secs(5)).unwrap();
        let dialer = here
            .dial(&format!("{}/chat", there.address()), steps.iter().map(|(s, d)| (!s, d.clone())).collect(), values_table(""), Duration::from_secs(5))
            .unwrap();
        let worker = std::thread::spawn(move || {
            listener.send(&Value::Integer(10.into())).unwrap();
            let said = listener.receive(Some(Duration::from_secs(5))).unwrap();
            listener.send(&Value::Integer(20.into())).unwrap();
            said
        });
        assert_eq!(render(&dialer.receive(Some(Duration::from_secs(5))).unwrap()), "10");
        dialer.send(&Value::Text("hi".into())).unwrap();
        assert_eq!(render(&dialer.receive(Some(Duration::from_secs(5))).unwrap()), "20");
        assert_eq!(render(&worker.join().unwrap()), "\"hi\"");
        here.close();
        there.close();
    }

    #[test]
    fn nodes_over_the_insecure_memory_transport() {
        // The frame layer alone; lawspec_network's tests cover secure nodes
        // over memory, TCP and HTTP.
        let net = MemoryNetwork::new(3, 0.2, 0.2, Duration::from_millis(10)).with_recording();
        exercise(net.insecure_transport_for_tests("a"), net.insecure_transport_for_tests("b"), true);
        assert!(!net.recorded().is_empty());
    }

    #[test]
    fn take_tokens_are_secure() {
        let token = secure_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(token, secure_token());
    }
}


// Abilities (docs/explanation/abilities.md). Handlers travel in the Context
// generated code passes to every definition: that context is the evidence of
// evidence-passing compilation. A law installs one handler per ability; an
// operation's bridge finds the handler of its ability there. The Fail
// ability's handlers abort, so raise panics with a Failure and attempt
// catches it.

/// A handler installed for an ability: an `Arc<dyn Trait>`, and its calls
/// when it records them.
#[derive(Clone)]
pub struct Installed {
    pub handler: Arc<dyn std::any::Any + Send + Sync>,
    pub calls: Option<Calls>,
    /// The handler's own type, when it is an ability's default (production)
    /// handler: how a reader such as lawspec.time's tells the real clock.
    pub native: Option<std::any::TypeId>,
}
impl std::fmt::Debug for Installed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.calls.is_some() { "<recording handler>" } else { "<handler>" })
    }
}

/// The calls a recording handler has seen.
#[derive(Clone, Default, Debug)]
pub struct Calls(Arc<std::sync::Mutex<Vec<(String, Vec<Value>)>>>);
impl Calls {
    pub fn record(&self, operation: &str, arguments: Vec<Value>) {
        self.0.lock().unwrap().push((operation.to_string(), arguments));
    }
}

pub fn installed<H: Send + Sync + 'static>(handler: H) -> Installed {
    Installed { handler: Arc::new(handler), calls: None, native: None }
}
/// A default handler of type N, installed as H (its `Arc<dyn Trait>`).
pub fn installed_native<N: 'static, H: Send + Sync + 'static>(handler: H) -> Installed {
    Installed { handler: Arc::new(handler), calls: None, native: Some(std::any::TypeId::of::<N>()) }
}
pub fn installed_recording<H: Send + Sync + 'static>(handler: H, calls: Calls) -> Installed {
    Installed { handler: Arc::new(handler), calls: Some(calls), native: None }
}

impl Context {
    pub fn install_handlers(&mut self, handlers: Vec<(String, Installed)>) {
        let mut table = self.handlers.write().unwrap();
        for (key, handler) in handlers {
            table.insert(key, handler);
        }
    }
    /// The handler installed for an ability, as the type its bridge needs.
    pub fn handler<H: Clone + 'static>(&self, ability: &str) -> Result<H> {
        let table = self.handlers.read().unwrap();
        let installed = table.get(ability).ok_or_else(|| {
            format!("no handler for the ability {ability}: a law names one with `using`, or runs under each lawful handler")
        })?;
        installed.handler.downcast_ref::<H>().cloned()
            .ok_or_else(|| format!("the handler for {ability} has another type"))
    }
    fn installed_for(&self, ability: &str) -> Option<Installed> {
        self.handlers.read().unwrap().get(ability).cloned()
    }
}

/// handle e with h end: the body runs with these handlers installed, then
/// the ones they replaced come back.
pub fn with_handlers(
    ctx: &mut Context,
    handlers: Vec<(String, Installed)>,
    body: impl FnOnce(&mut Context) -> Result<Value>,
) -> Result<Value> {
    let previous: Vec<(String, Option<Installed>)> =
        handlers.iter().map(|(key, _)| (key.clone(), ctx.installed_for(key))).collect();
    ctx.install_handlers(handlers);
    let restore = |ctx: &mut Context| {
        let mut table = ctx.handlers.write().unwrap();
        for (key, before) in &previous {
            match before {
                Some(handler) => { table.insert(key.clone(), handler.clone()); }
                None => { table.remove(key); }
            }
        }
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(ctx)));
    restore(ctx);
    match outcome {
        Ok(result) => result,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// What native code (an adapter, or a production handler) panics with to
/// fail with a value of the failure type its signature names:
/// `ls::fail(value)` for `fails with E`, the value a native E.
pub struct Fail(pub Box<dyn std::any::Any + Send>);

pub fn fail<E: Send + 'static>(value: E) -> ! {
    quiet_failures();
    std::panic::panic_any(Fail(Box::new(value)))
}

/// A native panic payload lawspec.json maps to a failure.
pub type MappedFailure = Box<dyn Fn(&(dyn std::any::Any + Send)) -> Option<Value>>;

/// Native code that may fail: a Fail it panics with (holding a native E), or
/// a panic payload lawspec.json maps, becomes a failure of the ability.
pub fn native_failures<E: 'static, T>(
    ability: &str,
    convert: impl FnOnce(E) -> Value,
    body: impl FnOnce() -> T,
    mapped: Vec<MappedFailure>,
) -> T {
    quiet_failures();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            let payload = match payload.downcast::<Fail>() {
                Ok(fail) => match fail.0.downcast::<E>() {
                    Ok(native) => std::panic::panic_any(Failure { ability: ability.to_string(), value: convert(*native) }),
                    Err(other) => other,
                },
                Err(other) => other,
            };
            for make in &mapped {
                if let Some(value) = make(payload.as_ref()) {
                    std::panic::panic_any(Failure { ability: ability.to_string(), value });
                }
            }
            std::panic::resume_unwind(payload)
        }
    }
}

/// A failure raised with the Fail ability.
#[derive(Debug)]
pub struct Failure {
    pub ability: String,
    pub value: Value,
}

fn quiet_failures() {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if info.payload().downcast_ref::<Failure>().is_none() && info.payload().downcast_ref::<Fail>().is_none() {
                previous(info);
            }
        }));
    });
}

pub fn raise_failure(ability: &str, value: Value) -> Value {
    quiet_failures();
    std::panic::panic_any(Failure { ability: ability.to_string(), value })
}

/// Right of the body, or Left of the failure it raised with this ability.
pub fn attempt(
    ctx: &mut Context,
    ability: &str,
    body: impl FnOnce(&mut Context) -> Result<Value>,
    right: impl FnOnce(Value) -> Result<Value>,
    left: impl FnOnce(Value) -> Result<Value>,
) -> Result<Value> {
    quiet_failures();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(ctx))) {
        Ok(result) => right(result?),
        Err(payload) => match payload.downcast::<Failure>() {
            Ok(failure) if failure.ability == ability => left(failure.value),
            Ok(failure) => std::panic::panic_any(*failure),
            Err(other) => std::panic::resume_unwind(other),
        },
    }
}

/// How many times a recording handler saw an operation (with arguments).
pub fn count_calls(
    ctx: &Context,
    ability: &str,
    operation: &str,
    matches: Option<&dyn Fn(&[Value]) -> Result<bool>>,
) -> Result<Value> {
    let installed = ctx.installed_for(ability).ok_or_else(|| format!("no handler for the ability {ability}"))?;
    let calls = installed.calls.as_ref().ok_or("calls of needs a recording handler: `using recording`")?;
    let recorded = calls.0.lock().unwrap().clone();
    let mut count = 0i64;
    for (name, arguments) in recorded {
        if name == operation && match matches { None => true, Some(test) => test(&arguments)? } {
            count += 1;
        }
    }
    Ok(Value::Integer(BigInt::from(count)))
}

/// A Pair's two fields: a stateful handler clause's result and next state.
pub fn pair_fields(pair: Value) -> Result<(Value, Value)> {
    match pair {
        Value::Data(_, fields) if fields.len() == 2 => {
            let mut fields = fields.into_iter();
            Ok((fields.next().unwrap(), fields.next().unwrap()))
        }
        _ => Err("a handler clause must give Pair result state".into()),
    }
}
