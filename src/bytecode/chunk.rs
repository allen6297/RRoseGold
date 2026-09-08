//! Stack-VM instruction list. Packed u8 / a register machine / JIT come after this set is boring.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::interpreter::StructDef;
use crate::{Span, Value};

#[derive(Clone, Debug)]
pub enum Op {
    Constant(u16),
    Nil,
    Void,
    Pop,
    GetLocal(u8),
    SetLocal(u8),
    /// Slot into [`Program::globals`].
    GetGlobal(u16),
    /// Peek the stack; store in [`Program::globals`].
    SetGlobal(u16),
    Add,
    Sub,
    Mul,
    Div,
    IDiv,
    Mod,
    Eq,
    Neq,
    Lt,
    Lte,
    Gt,
    Gte,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Neg,
    Not,
    BitNot,
    Truthy,
    Jump(u16),
    JumpIfFalse(u16),
    JumpIfTrue(u16),
    /// Function index into [`Program::functions`], then arg count.
    Call(u16, u8),
    /// Index into [`Program::hosts`], then arg count. Empty module is a builtin (`print`, `len`).
    Host(u16, u8),
    /// Receiver on the stack, then args. Name is [`Program::methods`].
    Invoke(u16, u8),
    /// Pop index, then object; push the element.
    GetIndex,
    /// Peek object+index (for `a[i] += 1`); push the old element.
    PeekIndex,
    /// Pop value, index, object; store; push value.
    SetIndex,
    /// Pop end, then start; push `Range`. `true` is `..=`.
    MakeRange(bool),
    /// Pop an iterable; push an Array of items (`for`).
    IterItems,
    Dup,
    /// Pop a value; format with constant spec (empty = `to_string`); push String.
    Format(u16),
    /// Unwrap `Result.Ok`, or return `Result.Err` from the current function.
    Try,
    /// Peek enum; push whether its variant name is [`Program::methods`].
    IsVariant(u16),
    /// Pop enum; push payload (`Void` if none).
    EnumPayload,
    /// Receiver on the stack. Name is [`Program::methods`].
    GetProp(u16),
    /// Pop value, then object; store field; push value. Name is [`Program::methods`].
    SetProp(u16),
    /// Index into [`Program::struct_defs`].
    NewStruct(u16),
    /// Function index, then capture count already on the stack (upvalues).
    MakeClosure(u16, u8),
    /// Pop args, then a callable (`BytecodeFn` / tree-walk closure).
    CallValue(u8),
    /// Wrap local slot as an upvalue and push the box (lambda capture).
    BoxLocal(u8),
    /// Peek enum; push named payload field. Name is [`Program::methods`].
    GetNamedField(u16),
    /// Pop args; spawn [`Op::Call`].
    SpawnCall(u16, u8),
    /// Pop args; spawn [`Op::Host`].
    SpawnHost(u16, u8),
    /// Receiver, then args; spawn [`Op::Invoke`].
    SpawnInvoke(u16, u8),
    /// Pop args, then callable; spawn [`Op::CallValue`].
    SpawnValue(u8),
    /// Pop a Task; push its result.
    Await,
    Return,
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub ops: Vec<Op>,
    pub spans: Vec<Span>,
    pub constants: Vec<Value>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            ops: Vec::new(),
            spans: Vec::new(),
            constants: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FnProto {
    pub name: String,
    pub arity: usize,
    pub local_count: usize,
    pub chunk: Chunk,
    pub is_async: bool,
    /// Source file label (`helpers.rg`), empty when unknown.
    pub file: String,
}

#[derive(Clone, Debug)]
pub struct Program {
    pub functions: Vec<FnProto>,
    /// `(module, name)` for [`Op::Host`]. Module `""` is a language builtin.
    pub hosts: Vec<(String, String)>,
    /// Names for [`Op::Invoke`], [`Op::GetProp`], [`Op::SetProp`], [`Op::IsVariant`].
    pub methods: Vec<String>,
    pub struct_defs: Vec<std::sync::Arc<StructDef>>,
    pub enums: HashMap<String, std::sync::Arc<crate::interpreter::EnumDef>>,
    /// `(type, method)` → function index. First arg is `self`.
    pub type_methods: HashMap<(String, String), u16>,
    /// `@ufcs` functions by name. Tried after inherent methods on [`Op::Invoke`].
    pub ufcs: HashMap<String, u16>,
    /// Top-level and module `var` / `const` slots. Shared across calls and `spawn`.
    pub globals: Arc<Mutex<Vec<Value>>>,
    /// Runs once before [`Program::main`] when globals need initializers.
    pub init: Option<usize>,
    pub main: usize,
}
