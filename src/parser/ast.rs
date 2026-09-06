//! AST types for programs, types, statements, and expressions.

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Import(Import),
    FnDecl(FnDecl),
    VarDecl(VarDecl),
    ConstDecl(ConstDecl),
    StructDecl(StructDecl),
    ClassDecl(ClassDecl),
    TraitDecl(TraitDecl),
    EnumDecl(EnumDecl),
    ImplDecl {
        type_name: String,
        trait_name: Option<String>,
        methods: Vec<FnDecl>,
        trailing: Vec<String>,
        span: crate::Span,
    },
    Mod(ModDecl),
    SignalDecl(SignalDecl),
    Comment(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModDecl {
    pub name: String,
    pub items: Vec<Item>,
    pub span: crate::Span,
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDecl {
    pub name: String,
    pub ty: Type,
    pub leading: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NestedImpl {
    pub trait_name: String,
    pub methods: Vec<FnDecl>,
    pub leading: Vec<String>,
    pub trailing: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub trailing: Vec<String>,
    pub doc: Option<String>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassDecl {
    pub name: String,
    pub parent: Option<String>,
    pub fields: Vec<FieldDecl>,
    pub defaults: Vec<(String, Expr)>,
    pub methods: Vec<FnDecl>,
    /// `class Foo impl Named, Drawable` — traits this class claims.
    pub impl_traits: Vec<String>,
    /// Nested `impl Trait { … }` inside the class body.
    pub trait_impls: Vec<NestedImpl>,
    pub trailing: Vec<String>,
    pub doc: Option<String>,
    pub span: crate::Span,
    pub is_pub: bool,
    /// `@node` — this class is a scene node the editor can insert.
    pub is_node: bool,
    /// `@export var` fields in this class (Inspector).
    pub exported_fields: Vec<VarDecl>,
}

impl ClassDecl {
    pub fn all_methods(&self) -> impl Iterator<Item = &FnDecl> {
        self.methods
            .iter()
            .chain(self.trait_impls.iter().flat_map(|b| b.methods.iter()))
    }

    /// `impl Trait` on the class header plus nested `impl Trait { … }`.
    pub fn implemented_traits(&self) -> Vec<String> {
        let mut names = self.impl_traits.clone();
        for b in &self.trait_impls {
            if !names.iter().any(|n| n == &b.trait_name) {
                names.push(b.trait_name.clone());
            }
        }
        names
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitDecl {
    pub name: String,
    pub methods: Vec<TraitMethod>,
    /// `signal` contract: implementing types expose these to Inspector / `.emit`.
    pub signals: Vec<SignalDecl>,
    pub trailing: Vec<String>,
    pub doc: Option<String>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub leading: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub name: String,
    pub variants: Vec<EnumVariant>,
    pub trailing: Vec<String>,
    pub doc: Option<String>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub value_types: Vec<Type>,
    /// Parallel to `value_types`. Empty string when the field is positional-only.
    pub field_names: Vec<String>,
    pub leading: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: Vec<String>,
    pub alias: Option<String>,
    /// `true` for `from module import item`, `false` for `import module.path`
    pub is_from: bool,
    pub span: crate::Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<Type>,
    pub body: Block,
    pub is_test: bool,
    pub is_ufcs: bool,
    pub doc: Option<String>,
    pub leading: Vec<String>,
    pub is_pub: bool,
}

/// Parameters after an explicit `self`. Class / trait methods may omit `self`;
/// the receiver is still bound as `self` when the method is called.
pub fn params_after_self(params: &[Param]) -> &[Param] {
    if params.first().is_some_and(|p| p.name == "self") {
        &params[1..]
    } else {
        params
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignalDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub span: crate::Span,
    pub doc: Option<String>,
    pub leading: Vec<String>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub name: String,
    pub args: Vec<Type>,
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VarDecl {
    pub name: String,
    pub ty: Type,
    pub value: Option<Expr>,
    pub exported: bool,
    pub export_group: Option<String>,
    pub doc: Option<String>,
    pub leading: Vec<String>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<Type>,
    pub value: Expr,
    pub doc: Option<String>,
    pub is_pub: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: crate::Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Expr(Expr),
    Return(Option<Expr>),
    If {
        cond: Expr,
        then_block: Block,
        elif_blocks: Vec<(Expr, Block)>,
        else_block: Option<Block>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    For {
        name: String,
        iter: Expr,
        body: Block,
    },
    VarDecl(VarDecl),
    ConstDecl(ConstDecl),
    Break,
    Continue,
    Pass,
    Comment(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: crate::Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Literal(Literal),
    Ident(String),
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Member {
        object: Box<Expr>,
        name: String,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    StructLiteral {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    Assign {
        op: AssignOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    FString(Vec<FStringPart>),
    Range {
        start: Box<Expr>,
        end: Box<Expr>,
        inclusive: bool,
    },
    Match {
        expr: Box<Expr>,
        arms: Vec<MatchArm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard,
    Variant {
        name: String,
        binds: Vec<String>,
        field_binds: Vec<(String, String)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FStringPart {
    Text(String),
    Expr { expr: Expr, format: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum BinOp {
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
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
}
