//! Tree-walking interpreter: values, imports, eval, and host dispatch.

mod dispatch;
mod eval;
mod ops;
mod resolver;
mod value;

pub use eval::{Environment, EvalContext, WorldEntry};
pub use resolver::{CombinedResolver, FileModuleResolver, HashMapResolver, ModuleResolver};
pub use value::{EnumDef, EnumVariantDef, Module, StructDef, Value};
