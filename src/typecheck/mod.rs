//! Lenient static checks: unknown names, arity, methods, trait impls.

mod checker;
mod helpers;

pub use checker::{typecheck, typecheck_diagnostics, typecheck_diagnostics_with};
pub use helpers::{is_stdlib_module, stdlib_arity};
