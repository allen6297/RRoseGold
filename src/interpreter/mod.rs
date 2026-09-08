//! Tree-walking interpreter: values, imports, eval, and host dispatch.

mod dispatch;
mod eval;
pub(crate) mod ops;
mod resolver;
mod ui_host;
mod value;

pub use eval::{Environment, EvalContext};
pub use resolver::{
    CombinedResolver, FileModuleResolver, HashMapResolver, ModuleResolver, ResolverRef,
    module_lookup_hint,
};
pub use value::{EnumDef, EnumVariantDef, Module, StructDef, Value};
