//! Runtime [`Value`] and type metadata ([`Module`], [`StructDef`], [`EnumDef`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::parser::*;

pub(crate) type ArrayRef = Rc<RefCell<Vec<Value>>>;
pub(crate) type MapRef = Rc<RefCell<HashMap<String, Value>>>;
pub(crate) type ModuleRef = Rc<RefCell<Module>>;
pub(crate) type StructDefRef = Rc<StructDef>;
pub(crate) type EnumDefRef = Rc<EnumDef>;

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

#[derive(Debug, Clone, PartialEq)]
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
    Struct {
        name: String,
        fields: Rc<RefCell<HashMap<String, Value>>>,
    },
    StructType(StructDefRef),
    EnumType(EnumDefRef),
    Signal {
        name: String,
        arity: usize,
    },
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(n) => *n != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.borrow().is_empty(),
            Value::Map(m) => !m.borrow().is_empty(),
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
            Value::Struct { .. } => true,
            Value::StructType(_) => true,
            Value::EnumType(_) => true,
            Value::Signal { .. } => true,
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
            Value::Struct { name, .. } => name.clone(),
            Value::StructType(s) => s.name.clone(),
            Value::EnumType(e) => e.name.clone(),
            Value::Signal { .. } => "Signal".to_string(),
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
                let parts: Vec<String> = a.borrow().iter().map(Value::to_string).collect();
                format!("[{}]", parts.join(", "))
            }
            Value::Map(m) => {
                let parts: Vec<String> = m
                    .borrow()
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
            Value::Module(m) => format!(
                "module({})",
                m.borrow()
                    .functions
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::NativeFn { module, name } => format!("{}.{}", module, name),
            Value::Struct { name, fields } => {
                let parts: Vec<String> = fields
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_string()))
                    .collect();
                format!("{} {{{}}}", name, parts.join(", "))
            }
            Value::StructType(s) => format!("struct {}", s.name),
            Value::EnumType(e) => format!("enum {}", e.name),
            Value::Signal { name, .. } => format!("signal {name}"),
        }
    }
}
