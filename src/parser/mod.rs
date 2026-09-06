//! Recursive-descent parser: tokens → AST ([`Item`], [`Expr`], [`Stmt`]).

mod ast;
mod parse;

pub use ast::*;
pub use parse::*;

#[cfg(test)]
mod tests;
