//! Runtime [`Value`] and type metadata ([`Module`], [`StructDef`], [`EnumDef`]).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::ThreadId;

use crate::RuntimeError;
use crate::parser::*;

pub(crate) type ArrayRef = Arc<Mutex<Vec<Value>>>;
pub(crate) type MapRef = Arc<Mutex<HashMap<String, Value>>>;
pub(crate) type ModuleRef = Arc<Mutex<Module>>;
pub(crate) type StructDefRef = Arc<StructDef>;
pub(crate) type EnumDefRef = Arc<EnumDef>;
pub(crate) type FieldMapRef = Arc<Mutex<HashMap<String, Value>>>;
pub(crate) type ScopeRef = Arc<Mutex<HashMap<String, Value>>>;

/// Anonymous function with captured environment frames.
#[derive(Clone, Debug)]
pub struct Closure {
    pub params: Vec<Param>,
    pub body: Block,
    pub captures: Vec<ScopeRef>,
    pub file: String,
    pub module: String,
}

impl PartialEq for Closure {
    fn eq(&self, other: &Self) -> bool {
        self.params == other.params && self.body == other.body
    }
}

pub(crate) fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub(crate) fn array_value(items: Vec<Value>) -> Value {
    Value::Array(Arc::new(Mutex::new(items)))
}

pub(crate) fn map_value(map: HashMap<String, Value>) -> Value {
    Value::Map(Arc::new(Mutex::new(map)))
}

#[derive(Clone)]
pub struct TaskHandle {
    pub(crate) rx: Arc<Mutex<Option<std::sync::mpsc::Receiver<Result<Value, RuntimeError>>>>>,
}

impl std::fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Task")
    }
}

pub struct LangMutex {
    state: Mutex<Option<ThreadId>>,
    cv: Condvar,
}

impl std::fmt::Debug for LangMutex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mutex")
    }
}

impl LangMutex {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(None),
            cv: Condvar::new(),
        })
    }

    pub(crate) fn lock(&self) -> Result<(), String> {
        let me = std::thread::current().id();
        let mut owner = lock(&self.state);
        loop {
            match *owner {
                Some(id) if id == me => {
                    return Err("mutex already locked by this thread".to_string());
                }
                Some(_) => {
                    owner = self.cv.wait(owner).unwrap_or_else(|p| p.into_inner());
                }
                None => {
                    *owner = Some(me);
                    return Ok(());
                }
            }
        }
    }

    pub(crate) fn unlock(&self) -> Result<(), String> {
        let me = std::thread::current().id();
        let mut owner = lock(&self.state);
        match *owner {
            Some(id) if id == me => {
                *owner = None;
                self.cv.notify_one();
                Ok(())
            }
            Some(_) => Err("mutex is locked by another thread".to_string()),
            None => Err("mutex is not locked".to_string()),
        }
    }
}

pub struct LangChannel {
    inner: Mutex<ChannelInner>,
    cv: Condvar,
}

struct ChannelInner {
    queue: VecDeque<Value>,
    closed: bool,
}

impl std::fmt::Debug for LangChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Channel")
    }
}

impl LangChannel {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(ChannelInner {
                queue: VecDeque::new(),
                closed: false,
            }),
            cv: Condvar::new(),
        })
    }

    pub(crate) fn send(&self, value: Value) -> Result<(), String> {
        let mut st = lock(&self.inner);
        if st.closed {
            return Err("send on closed channel".to_string());
        }
        st.queue.push_back(value);
        self.cv.notify_one();
        Ok(())
    }

    pub(crate) fn close(&self) {
        let mut st = lock(&self.inner);
        st.closed = true;
        self.cv.notify_all();
    }

    pub(crate) fn recv(&self) -> Value {
        let mut st = lock(&self.inner);
        loop {
            if let Some(v) = st.queue.pop_front() {
                return v;
            }
            if st.closed {
                return Value::None;
            }
            st = self.cv.wait(st).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// `None` on timeout or closed-and-empty.
    pub(crate) fn recv_timeout(&self, secs: f64) -> Option<Value> {
        use std::time::Instant;
        let wait = duration_secs(secs);
        let deadline = Instant::now() + wait;
        let mut st = lock(&self.inner);
        loop {
            if let Some(v) = st.queue.pop_front() {
                return Some(v);
            }
            if st.closed {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, result) = self
                .cv
                .wait_timeout(st, deadline.saturating_duration_since(now))
                .unwrap_or_else(|p| p.into_inner());
            st = guard;
            if result.timed_out() && st.queue.is_empty() && !st.closed {
                return None;
            }
        }
    }
}

pub(crate) fn duration_secs(secs: f64) -> std::time::Duration {
    if !secs.is_finite() || secs <= 0.0 {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_secs_f64(secs.min(86_400.0))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub functions: HashMap<String, FnDecl>,
    pub values: HashMap<String, Value>,
    pub native: Option<String>,
}

impl Module {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            values: HashMap::new(),
            native: None,
        }
    }

    pub fn native(module: impl Into<String>) -> Self {
        Self {
            functions: HashMap::new(),
            values: HashMap::new(),
            native: Some(module.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<String>,
    pub defaults: Vec<(String, Expr)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDef {
    pub name: String,
    pub variants: HashMap<String, EnumVariantDef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariantDef {
    pub arity: usize,
    pub field_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Void,
    None,
    Array(ArrayRef),
    Map(MapRef),
    Range(i64, i64, bool),
    Enum {
        module: String,
        variant: String,
        value: Option<Box<Value>>,
    },
    Module(ModuleRef),
    NativeFn {
        module: String,
        name: String,
    },
    /// Free function used as a value (`collected.connect(on_coin)`).
    FnRef {
        name: String,
    },
    /// Lambda / closure (`fn() { ... }`).
    Closure(Arc<Closure>),
    Struct {
        name: String,
        fields: FieldMapRef,
    },
    StructType(StructDefRef),
    EnumType(EnumDefRef),
    Signal {
        name: String,
        arity: usize,
    },
    Task(TaskHandle),
    Mutex(Arc<LangMutex>),
    Channel(Arc<LangChannel>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Void, Value::Void) => true,
            (Value::None, Value::None) => true,
            (Value::Array(a), Value::Array(b)) => {
                if Arc::ptr_eq(a, b) {
                    true
                } else {
                    let a = lock(a).clone();
                    let b = lock(b).clone();
                    a == b
                }
            }
            (Value::Map(a), Value::Map(b)) => {
                if Arc::ptr_eq(a, b) {
                    true
                } else {
                    let a = lock(a).clone();
                    let b = lock(b).clone();
                    a == b
                }
            }
            (Value::Range(a1, a2, a3), Value::Range(b1, b2, b3)) => {
                a1 == b1 && a2 == b2 && a3 == b3
            }
            (
                Value::Enum {
                    module: ma,
                    variant: va,
                    value: a,
                },
                Value::Enum {
                    module: mb,
                    variant: vb,
                    value: b,
                },
            ) => ma == mb && va == vb && a == b,
            (Value::Module(a), Value::Module(b)) => {
                if Arc::ptr_eq(a, b) {
                    true
                } else {
                    *lock(a) == *lock(b)
                }
            }
            (
                Value::NativeFn {
                    module: ma,
                    name: na,
                },
                Value::NativeFn {
                    module: mb,
                    name: nb,
                },
            ) => ma == mb && na == nb,
            (Value::FnRef { name: a }, Value::FnRef { name: b }) => a == b,
            (Value::Closure(a), Value::Closure(b)) => Arc::ptr_eq(a, b) || **a == **b,
            (
                Value::Struct {
                    name: na,
                    fields: fa,
                },
                Value::Struct {
                    name: nb,
                    fields: fb,
                },
            ) => {
                na == nb
                    && (Arc::ptr_eq(fa, fb) || {
                        let a = lock(fa).clone();
                        let b = lock(fb).clone();
                        a == b
                    })
            }
            (Value::StructType(a), Value::StructType(b)) => a == b,
            (Value::EnumType(a), Value::EnumType(b)) => a == b,
            (
                Value::Signal {
                    name: na,
                    arity: aa,
                },
                Value::Signal {
                    name: nb,
                    arity: ab,
                },
            ) => na == nb && aa == ab,
            (Value::Task(a), Value::Task(b)) => Arc::ptr_eq(&a.rx, &b.rx),
            (Value::Mutex(a), Value::Mutex(b)) => Arc::ptr_eq(a, b),
            (Value::Channel(a), Value::Channel(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !lock(a).is_empty(),
            Value::Map(m) => !lock(m).is_empty(),
            Value::Range(start, end, inclusive) => {
                if *inclusive {
                    start <= end
                } else {
                    start < end
                }
            }
            Value::Enum {
                module, variant, ..
            } => {
                !(module == "Option" && variant == "None" || module == "Result" && variant == "Err")
            }
            Value::Module(_) => true,
            Value::NativeFn { .. } => true,
            Value::FnRef { .. } => true,
            Value::Closure(_) => true,
            Value::Struct { .. } => true,
            Value::StructType(_) => true,
            Value::EnumType(_) => true,
            Value::Signal { .. } => true,
            Value::Task(_) => true,
            Value::Mutex(_) => true,
            Value::Channel(_) => true,
            Value::None => false,
            Value::Void => false,
        }
    }

    pub fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "Int".to_string(),
            Value::Float(_) => "Float".to_string(),
            Value::String(_) => "String".to_string(),
            Value::Bool(_) => "Bool".to_string(),
            Value::Void => "Void".to_string(),
            Value::None => "none".to_string(),
            Value::Array(_) => "Array".to_string(),
            Value::Map(_) => "Map".to_string(),
            Value::Range(_, _, _) => "Range".to_string(),
            Value::Enum {
                module, variant, ..
            } => format!("{}.{}", module, variant),
            Value::Module(_) => "Module".to_string(),
            Value::NativeFn { module, name } => format!("{}.{}", module, name),
            Value::FnRef { name } => format!("fn {name}"),
            Value::Closure(_) => "Fn".to_string(),
            Value::Struct { name, .. } => name.clone(),
            Value::StructType(s) => s.name.clone(),
            Value::EnumType(e) => e.name.clone(),
            Value::Signal { .. } => "Signal".to_string(),
            Value::Task(_) => "Task".to_string(),
            Value::Mutex(_) => "Mutex".to_string(),
            Value::Channel(_) => "Channel".to_string(),
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Float(n) => n.to_string(),
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Void => "void".to_string(),
            Value::None => "none".to_string(),
            Value::Array(a) => {
                let items: Vec<Value> = lock(a).clone();
                let parts: Vec<String> = items.iter().map(Value::to_string).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Map(m) => {
                let entries: Vec<(String, Value)> = lock(m)
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_string()))
                    .collect();
                format!("{{{}}}", parts.join(", "))
            }
            Value::Range(start, end, inclusive) => {
                if *inclusive {
                    format!("{}..={}", start, end)
                } else {
                    format!("{}..{}", start, end)
                }
            }
            Value::Enum {
                module,
                variant,
                value,
            } => {
                if let Some(v) = value {
                    format!("{}.{}({})", module, variant, v.to_string())
                } else {
                    format!("{}.{}", module, variant)
                }
            }
            Value::Module(m) => {
                let keys: Vec<String> = lock(m).functions.keys().cloned().collect();
                format!("module({})", keys.join(", "))
            }
            Value::NativeFn { module, name } => format!("{}.{}", module, name),
            Value::FnRef { name } => format!("fn {name}"),
            Value::Closure(_) => "fn()".to_string(),
            Value::Struct { name, fields } => {
                let entries: Vec<(String, Value)> = lock(fields)
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let parts: Vec<String> = entries
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_string()))
                    .collect();
                format!("{} {{{}}}", name, parts.join(", "))
            }
            Value::StructType(s) => format!("struct {}", s.name),
            Value::EnumType(e) => format!("enum {}", e.name),
            Value::Signal { name, .. } => format!("signal {name}"),
            Value::Task(_) => "task".to_string(),
            Value::Mutex(_) => "mutex".to_string(),
            Value::Channel(_) => "channel".to_string(),
        }
    }
}
