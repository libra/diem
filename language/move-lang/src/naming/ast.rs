// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    expansion::ast::{
        ability_constraints_ast_debug, ability_modifiers_ast_debug, AbilitySet, Attribute, Fields,
        Friend, ModuleIdent, SpecId, Value, Value_,
    },
    parser::ast::{BinOp, ConstantName, Field, FunctionName, StructName, UnaryOp, Var, Visibility},
    shared::{ast_debug::*, unique_map::UniqueMap, *},
};
use move_ir_types::location::*;
use once_cell::sync::Lazy;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
};

//**************************************************************************************************
// Program
//**************************************************************************************************

#[derive(Debug, Clone)]
pub struct Program {
    // Map of known named address values. Not all addresses will be present
    pub addresses: UniqueMap<Name, AddressBytes>,
    pub modules: UniqueMap<ModuleIdent, ModuleDefinition>,
    pub scripts: BTreeMap<String, Script>,
}

//**************************************************************************************************
// Scripts
//**************************************************************************************************

#[derive(Debug, Clone)]
pub struct Script {
    pub attributes: Vec<Attribute>,
    pub loc: Loc,
    pub constants: UniqueMap<ConstantName, Constant>,
    pub function_name: FunctionName,
    pub function: Function,
}

//**************************************************************************************************
// Modules
//**************************************************************************************************

#[derive(Debug, Clone)]
pub struct ModuleDefinition {
    pub attributes: Vec<Attribute>,
    pub is_source_module: bool,
    /// `dependency_order` is the topological order/rank in the dependency graph.
    /// `dependency_order` is initialized at `0` and set in the uses pass
    pub dependency_order: usize,
    pub friends: UniqueMap<ModuleIdent, Friend>,
    pub structs: UniqueMap<StructName, StructDefinition>,
    pub constants: UniqueMap<ConstantName, Constant>,
    pub functions: UniqueMap<FunctionName, Function>,
}

//**************************************************************************************************
// Structs
//**************************************************************************************************

#[derive(Debug, PartialEq, Clone)]
pub struct StructDefinition {
    pub attributes: Vec<Attribute>,
    pub abilities: AbilitySet,
    pub type_parameters: Vec<StructTypeParameter>,
    pub fields: StructFields,
}

#[derive(Debug, PartialEq, Clone)]
pub struct StructTypeParameter {
    pub param: TParam,
    pub is_phantom: bool,
}

#[derive(Debug, PartialEq, Clone)]
pub enum StructFields {
    Defined(Fields<Type>),
    Native(Loc),
}

//**************************************************************************************************
// Functions
//**************************************************************************************************

#[derive(PartialEq, Debug, Clone)]
pub struct FunctionSignature {
    pub type_parameters: Vec<TParam>,
    pub parameters: Vec<(Var, Type)>,
    pub return_type: Type,
}

#[derive(PartialEq, Debug, Clone)]
pub enum FunctionBody_ {
    Defined(Sequence),
    Native,
}
pub type FunctionBody = Spanned<FunctionBody_>;

#[derive(PartialEq, Debug, Clone)]
pub struct Function {
    pub attributes: Vec<Attribute>,
    pub visibility: Visibility,
    pub signature: FunctionSignature,
    pub acquires: BTreeMap<StructName, Loc>,
    pub body: FunctionBody,
}

//**************************************************************************************************
// Constants
//**************************************************************************************************

#[derive(PartialEq, Debug, Clone)]
pub struct Constant {
    pub attributes: Vec<Attribute>,
    pub loc: Loc,
    pub signature: Type,
    pub value: Exp,
}

//**************************************************************************************************
// Types
//**************************************************************************************************

#[derive(Debug, PartialEq, Clone, PartialOrd, Eq, Ord)]
pub enum BuiltinTypeName_ {
    // address
    Address,
    // signer
    Signer,
    // u8
    U8,
    // u64
    U64,
    // u128
    U128,
    // Vector
    Vector,
    // bool
    Bool,
}
pub type BuiltinTypeName = Spanned<BuiltinTypeName_>;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum TypeName_ {
    // exp-list/tuple type
    Multiple(usize),
    Builtin(BuiltinTypeName),
    ModuleType(ModuleIdent, StructName),
}
pub type TypeName = Spanned<TypeName_>;

#[derive(Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub struct TParamID(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TParam {
    pub id: TParamID,
    pub user_specified_name: Name,
    pub abilities: AbilitySet,
}

#[derive(Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Copy, Clone)]
pub struct TVar(u64);

#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Type_ {
    Unit,
    Ref(bool, Box<Type>),
    Param(TParam),
    Apply(Option<AbilitySet>, TypeName, Vec<Type>),
    Var(TVar),
    Anything,
    UnresolvedError,
}
pub type Type = Spanned<Type_>;

//**************************************************************************************************
// Expressions
//**************************************************************************************************

#[derive(Debug, PartialEq, Clone)]
pub enum LValue_ {
    Ignore,
    Var(Var),
    Unpack(ModuleIdent, StructName, Option<Vec<Type>>, Fields<LValue>),
}
pub type LValue = Spanned<LValue_>;
pub type LValueList_ = Vec<LValue>;
pub type LValueList = Spanned<LValueList_>;

#[derive(Debug, PartialEq, Clone)]
pub enum ExpDotted_ {
    Exp(Box<Exp>),
    Dot(Box<ExpDotted>, Field),
}
pub type ExpDotted = Spanned<ExpDotted_>;

#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BuiltinFunction_ {
    MoveTo(Option<Type>),
    MoveFrom(Option<Type>),
    BorrowGlobal(bool, Option<Type>),
    Exists(Option<Type>),
    Freeze(Option<Type>),
    Assert,
}
pub type BuiltinFunction = Spanned<BuiltinFunction_>;

#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Exp_ {
    Value(Value),
    Move(Var),
    Copy(Var),
    Use(Var),
    Constant(Option<ModuleIdent>, ConstantName),

    ModuleCall(
        ModuleIdent,
        FunctionName,
        Option<Vec<Type>>,
        Spanned<Vec<Exp>>,
    ),
    Builtin(BuiltinFunction, Spanned<Vec<Exp>>),

    IfElse(Box<Exp>, Box<Exp>, Box<Exp>),
    While(Box<Exp>, Box<Exp>),
    Loop(Box<Exp>),
    Block(Sequence),

    Assign(LValueList, Box<Exp>),
    FieldMutate(ExpDotted, Box<Exp>),
    Mutate(Box<Exp>, Box<Exp>),

    Return(Box<Exp>),
    Abort(Box<Exp>),
    Break,
    Continue,

    Dereference(Box<Exp>),
    UnaryExp(UnaryOp, Box<Exp>),
    BinopExp(Box<Exp>, BinOp, Box<Exp>),

    Pack(ModuleIdent, StructName, Option<Vec<Type>>, Fields<Exp>),
    ExpList(Vec<Exp>),
    Unit {
        trailing: bool,
    },

    DerefBorrow(ExpDotted),
    Borrow(bool, ExpDotted),

    Cast(Box<Exp>, Type),
    Annotate(Box<Exp>, Type),

    Spec(SpecId, BTreeSet<Var>),

    UnresolvedError,
}
pub type Exp = Spanned<Exp_>;

pub type Sequence = VecDeque<SequenceItem>;
#[derive(Debug, PartialEq, Clone)]
pub enum SequenceItem_ {
    Seq(Exp),
    Declare(LValueList, Option<Type>),
    Bind(LValueList, Exp),
}
pub type SequenceItem = Spanned<SequenceItem_>;

//**************************************************************************************************
// impls
//**************************************************************************************************

static BUILTIN_TYPE_ALL_NAMES: Lazy<BTreeSet<&'static str>> = Lazy::new(|| {
    [
        BuiltinTypeName_::ADDRESS,
        BuiltinTypeName_::SIGNER,
        BuiltinTypeName_::U_8,
        BuiltinTypeName_::U_64,
        BuiltinTypeName_::U_128,
        BuiltinTypeName_::BOOL,
        BuiltinTypeName_::VECTOR,
    ]
    .iter()
    .cloned()
    .collect()
});

static BUILTIN_TYPE_NUMERIC: Lazy<BTreeSet<BuiltinTypeName_>> = Lazy::new(|| {
    [
        BuiltinTypeName_::U8,
        BuiltinTypeName_::U64,
        BuiltinTypeName_::U128,
    ]
    .iter()
    .cloned()
    .collect()
});

static BUILTIN_TYPE_BITS: Lazy<BTreeSet<BuiltinTypeName_>> =
    Lazy::new(|| BUILTIN_TYPE_NUMERIC.clone());

static BUILTIN_TYPE_ORDERED: Lazy<BTreeSet<BuiltinTypeName_>> =
    Lazy::new(|| BUILTIN_TYPE_BITS.clone());

impl BuiltinTypeName_ {
    pub const ADDRESS: &'static str = "address";
    pub const SIGNER: &'static str = "signer";
    pub const U_8: &'static str = "u8";
    pub const U_64: &'static str = "u64";
    pub const U_128: &'static str = "u128";
    pub const BOOL: &'static str = "bool";
    pub const VECTOR: &'static str = "vector";

    pub fn all_names() -> &'static BTreeSet<&'static str> {
        &*BUILTIN_TYPE_ALL_NAMES
    }

    pub fn numeric() -> &'static BTreeSet<BuiltinTypeName_> {
        &*BUILTIN_TYPE_NUMERIC
    }

    pub fn bits() -> &'static BTreeSet<BuiltinTypeName_> {
        &*BUILTIN_TYPE_BITS
    }

    pub fn ordered() -> &'static BTreeSet<BuiltinTypeName_> {
        &*BUILTIN_TYPE_ORDERED
    }

    pub fn is_numeric(&self) -> bool {
        Self::numeric().contains(self)
    }

    pub fn resolve(name_str: &str) -> Option<Self> {
        use BuiltinTypeName_ as BT;
        match name_str {
            BT::ADDRESS => Some(BT::Address),
            BT::SIGNER => Some(BT::Signer),
            BT::U_8 => Some(BT::U8),
            BT::U_64 => Some(BT::U64),
            BT::U_128 => Some(BT::U128),
            BT::BOOL => Some(BT::Bool),
            BT::VECTOR => Some(BT::Vector),
            _ => None,
        }
    }

    pub fn declared_abilities(&self, loc: Loc) -> AbilitySet {
        use BuiltinTypeName_ as B;
        // Match here to make sure this function is fixed when collections are added
        match self {
            B::Address | B::U8 | B::U64 | B::U128 | B::Bool => AbilitySet::primitives(loc),
            B::Signer => AbilitySet::signer(loc),
            B::Vector => AbilitySet::collection(loc),
        }
    }

    pub fn tparam_constraints(&self, _loc: Loc) -> Vec<AbilitySet> {
        use BuiltinTypeName_ as B;
        // Match here to make sure this function is fixed when collections are added
        match self {
            B::Address | B::Signer | B::U8 | B::U64 | B::U128 | B::Bool => vec![],
            B::Vector => vec![AbilitySet::empty()],
        }
    }
}

impl TParamID {
    pub fn next() -> TParamID {
        TParamID(Counter::next())
    }
}

impl TVar {
    pub fn next() -> TVar {
        TVar(Counter::next())
    }
}

static BUILTIN_FUNCTION_ALL_NAMES: Lazy<BTreeSet<&'static str>> = Lazy::new(|| {
    let mut s = BTreeSet::new();
    s.insert(BuiltinFunction_::MOVE_TO);
    s.insert(BuiltinFunction_::MOVE_FROM);
    s.insert(BuiltinFunction_::BORROW_GLOBAL);
    s.insert(BuiltinFunction_::BORROW_GLOBAL_MUT);
    s.insert(BuiltinFunction_::EXISTS);
    s.insert(BuiltinFunction_::FREEZE);
    s.insert(BuiltinFunction_::ASSERT);
    s
});

impl BuiltinFunction_ {
    pub const MOVE_TO: &'static str = "move_to";
    pub const MOVE_FROM: &'static str = "move_from";
    pub const BORROW_GLOBAL: &'static str = "borrow_global";
    pub const BORROW_GLOBAL_MUT: &'static str = "borrow_global_mut";
    pub const EXISTS: &'static str = "exists";
    pub const FREEZE: &'static str = "freeze";
    pub const ASSERT: &'static str = "assert";

    pub fn all_names() -> &'static BTreeSet<&'static str> {
        &*BUILTIN_FUNCTION_ALL_NAMES
    }

    pub fn resolve(name_str: &str, arg: Option<Type>) -> Option<Self> {
        use BuiltinFunction_ as BF;
        match name_str {
            BF::MOVE_TO => Some(BF::MoveTo(arg)),
            BF::MOVE_FROM => Some(BF::MoveFrom(arg)),
            BF::BORROW_GLOBAL => Some(BF::BorrowGlobal(false, arg)),
            BF::BORROW_GLOBAL_MUT => Some(BF::BorrowGlobal(true, arg)),
            BF::EXISTS => Some(BF::Exists(arg)),
            BF::FREEZE => Some(BF::Freeze(arg)),
            BF::ASSERT => Some(BF::Assert),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        use BuiltinFunction_ as BF;
        match self {
            BF::MoveTo(_) => BF::MOVE_TO,
            BF::MoveFrom(_) => BF::MOVE_FROM,
            BF::BorrowGlobal(false, _) => BF::BORROW_GLOBAL,
            BF::BorrowGlobal(true, _) => BF::BORROW_GLOBAL_MUT,
            BF::Exists(_) => BF::EXISTS,
            BF::Freeze(_) => BF::FREEZE,
            BF::Assert => BF::ASSERT,
        }
    }
}

impl Type_ {
    pub fn builtin_(b: BuiltinTypeName, ty_args: Vec<Type>) -> Type_ {
        use BuiltinTypeName_ as B;
        let abilities = match &b.value {
            B::Address | B::U8 | B::U64 | B::U128 | B::Bool => Some(AbilitySet::primitives(b.loc)),
            B::Signer => Some(AbilitySet::signer(b.loc)),
            B::Vector => None,
        };
        let n = sp(b.loc, TypeName_::Builtin(b));
        Type_::Apply(abilities, n, ty_args)
    }

    pub fn builtin(loc: Loc, b: BuiltinTypeName, ty_args: Vec<Type>) -> Type {
        sp(loc, Self::builtin_(b, ty_args))
    }

    pub fn bool(loc: Loc) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::Bool), vec![])
    }

    pub fn address(loc: Loc) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::Address), vec![])
    }

    pub fn signer(loc: Loc) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::Signer), vec![])
    }

    pub fn u8(loc: Loc) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::U8), vec![])
    }

    pub fn u64(loc: Loc) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::U64), vec![])
    }

    pub fn u128(loc: Loc) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::U128), vec![])
    }

    pub fn vector(loc: Loc, elem: Type) -> Type {
        Self::builtin(loc, sp(loc, BuiltinTypeName_::Vector), vec![elem])
    }

    pub fn multiple(loc: Loc, tys: Vec<Type>) -> Type {
        sp(loc, Self::multiple_(loc, tys))
    }

    pub fn multiple_(loc: Loc, mut tys: Vec<Type>) -> Type_ {
        match tys.len() {
            0 => Type_::Unit,
            1 => tys.pop().unwrap().value,
            n => Type_::Apply(None, sp(loc, TypeName_::Multiple(n)), tys),
        }
    }

    pub fn builtin_name(&self) -> Option<&BuiltinTypeName> {
        match self {
            Type_::Apply(_, sp!(_, TypeName_::Builtin(b)), _) => Some(b),
            _ => None,
        }
    }
}

impl Value_ {
    pub fn type_(&self, loc: Loc) -> Option<Type> {
        use Value_::*;
        Some(match self {
            Address(_) => Type_::address(loc),
            InferredNum(_) => return None,
            U8(_) => Type_::u8(loc),
            U64(_) => Type_::u64(loc),
            U128(_) => Type_::u128(loc),
            Bool(_) => Type_::bool(loc),
            Bytearray(_) => Type_::vector(loc, Type_::u8(loc)),
        })
    }
}

//**************************************************************************************************
// Display
//**************************************************************************************************

impl fmt::Display for BuiltinTypeName_ {
    fn fmt(&self, f: &mut fmt::Formatter) -> std::fmt::Result {
        use BuiltinTypeName_ as BT;
        write!(
            f,
            "{}",
            match self {
                BT::Address => BT::ADDRESS,
                BT::Signer => BT::SIGNER,
                BT::U8 => BT::U_8,
                BT::U64 => BT::U_64,
                BT::U128 => BT::U_128,
                BT::Bool => BT::BOOL,
                BT::Vector => BT::VECTOR,
            }
        )
    }
}

impl fmt::Display for TypeName_ {
    fn fmt(&self, f: &mut fmt::Formatter) -> std::fmt::Result {
        use TypeName_::*;
        match self {
            Multiple(_) => panic!("ICE cannot display expr-list type name"),
            Builtin(b) => write!(f, "{}", b),
            ModuleType(m, n) => write!(f, "{}::{}", m, n),
        }
    }
}

//**************************************************************************************************
// Debug
//**************************************************************************************************

impl AstDebug for Program {
    fn ast_debug(&self, w: &mut AstWriter) {
        let Program {
            addresses,
            modules,
            scripts,
        } = self;
        for (_, addr, bytes) in addresses {
            w.writeln(&format!("address {} = {};", addr, bytes));
        }

        for (m, mdef) in modules.key_cloned_iter() {
            w.write(&format!("module {}", m));
            w.block(|w| mdef.ast_debug(w));
            w.new_line();
        }

        for (n, s) in scripts {
            w.write(&format!("script {}", n));
            w.block(|w| s.ast_debug(w));
            w.new_line()
        }
    }
}

impl AstDebug for Script {
    fn ast_debug(&self, w: &mut AstWriter) {
        let Script {
            attributes,
            loc: _loc,
            constants,
            function_name,
            function,
        } = self;
        attributes.ast_debug(w);
        for cdef in constants.key_cloned_iter() {
            cdef.ast_debug(w);
            w.new_line();
        }
        (function_name.clone(), function).ast_debug(w);
    }
}

impl AstDebug for ModuleDefinition {
    fn ast_debug(&self, w: &mut AstWriter) {
        let ModuleDefinition {
            attributes,
            is_source_module,
            dependency_order,
            friends,
            structs,
            constants,
            functions,
        } = self;
        attributes.ast_debug(w);
        if *is_source_module {
            w.writeln("library module")
        } else {
            w.writeln("source module")
        }
        w.writeln(&format!("dependency order #{}", dependency_order));
        for (mident, _loc) in friends.key_cloned_iter() {
            w.write(&format!("friend {};", mident));
            w.new_line();
        }
        for sdef in structs.key_cloned_iter() {
            sdef.ast_debug(w);
            w.new_line();
        }
        for cdef in constants.key_cloned_iter() {
            cdef.ast_debug(w);
            w.new_line();
        }
        for fdef in functions.key_cloned_iter() {
            fdef.ast_debug(w);
            w.new_line();
        }
    }
}

impl AstDebug for (StructName, &StructDefinition) {
    fn ast_debug(&self, w: &mut AstWriter) {
        let (
            name,
            StructDefinition {
                attributes,
                abilities,
                type_parameters,
                fields,
            },
        ) = self;
        attributes.ast_debug(w);
        if let StructFields::Native(_) = fields {
            w.write("native ");
        }
        w.write(&format!("struct {}", name));
        type_parameters.ast_debug(w);
        ability_modifiers_ast_debug(w, abilities);
        if let StructFields::Defined(fields) = fields {
            w.block(|w| {
                w.list(fields, ",", |w, (_, f, idx_st)| {
                    let (idx, st) = idx_st;
                    w.write(&format!("{}#{}: ", idx, f));
                    st.ast_debug(w);
                    true
                })
            })
        }
    }
}

impl AstDebug for (FunctionName, &Function) {
    fn ast_debug(&self, w: &mut AstWriter) {
        let (
            name,
            Function {
                attributes,
                visibility,
                signature,
                acquires,
                body,
            },
        ) = self;
        attributes.ast_debug(w);
        visibility.ast_debug(w);
        if let FunctionBody_::Native = &body.value {
            w.write("native ");
        }
        w.write(&format!("fun {}", name));
        signature.ast_debug(w);
        if !acquires.is_empty() {
            w.write(" acquires ");
            w.comma(acquires.keys(), |w, s| w.write(&format!("{}", s)));
            w.write(" ")
        }
        match &body.value {
            FunctionBody_::Defined(body) => w.block(|w| body.ast_debug(w)),
            FunctionBody_::Native => w.writeln(";"),
        }
    }
}

impl AstDebug for FunctionSignature {
    fn ast_debug(&self, w: &mut AstWriter) {
        let FunctionSignature {
            type_parameters,
            parameters,
            return_type,
        } = self;
        type_parameters.ast_debug(w);
        w.write("(");
        w.comma(parameters, |w, (v, st)| {
            w.write(&format!("{}: ", v));
            st.ast_debug(w);
        });
        w.write("): ");
        return_type.ast_debug(w)
    }
}

impl AstDebug for Vec<TParam> {
    fn ast_debug(&self, w: &mut AstWriter) {
        if !self.is_empty() {
            w.write("<");
            w.comma(self, |w, tp| tp.ast_debug(w));
            w.write(">")
        }
    }
}

impl AstDebug for Vec<StructTypeParameter> {
    fn ast_debug(&self, w: &mut AstWriter) {
        if !self.is_empty() {
            w.write("<");
            w.comma(self, |w, tp| tp.ast_debug(w));
            w.write(">")
        }
    }
}

impl AstDebug for (ConstantName, &Constant) {
    fn ast_debug(&self, w: &mut AstWriter) {
        let (
            name,
            Constant {
                attributes,
                loc: _loc,
                signature,
                value,
            },
        ) = self;
        attributes.ast_debug(w);
        w.write(&format!("const {}:", name));
        signature.ast_debug(w);
        w.write(" = ");
        value.ast_debug(w);
        w.write(";");
    }
}

impl AstDebug for BuiltinTypeName_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        w.write(&format!("{}", self));
    }
}

impl AstDebug for TypeName_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            TypeName_::Multiple(len) => w.write(&format!("Multiple({})", len)),
            TypeName_::Builtin(bt) => bt.ast_debug(w),
            TypeName_::ModuleType(m, s) => w.write(&format!("{}::{}", m, s)),
        }
    }
}

impl AstDebug for TParam {
    fn ast_debug(&self, w: &mut AstWriter) {
        let TParam {
            id,
            user_specified_name,
            abilities,
        } = self;
        w.write(&format!("{}#{}", user_specified_name, id.0));
        ability_constraints_ast_debug(w, abilities);
    }
}

impl AstDebug for StructTypeParameter {
    fn ast_debug(&self, w: &mut AstWriter) {
        if self.is_phantom {
            w.write("phantom ");
        }
        self.param.ast_debug(w);
    }
}

impl AstDebug for Type_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            Type_::Unit => w.write("()"),
            Type_::Ref(mut_, s) => {
                w.write("&");
                if *mut_ {
                    w.write("mut ");
                }
                s.ast_debug(w)
            }
            Type_::Param(tp) => tp.ast_debug(w),
            Type_::Apply(abilities_opt, sp!(_, TypeName_::Multiple(_)), ss) => {
                let w_ty = move |w: &mut AstWriter| {
                    w.write("(");
                    ss.ast_debug(w);
                    w.write(")");
                };
                match abilities_opt {
                    None => w_ty(w),
                    Some(abilities) => w.annotate_gen(w_ty, abilities, |w, annot| {
                        w.list(annot, "+", |w, a| {
                            a.ast_debug(w);
                            false
                        })
                    }),
                }
            }
            Type_::Apply(abilities_opt, m, ss) => {
                let w_ty = move |w: &mut AstWriter| {
                    m.ast_debug(w);
                    if !ss.is_empty() {
                        w.write("<");
                        ss.ast_debug(w);
                        w.write(">");
                    }
                };
                match abilities_opt {
                    None => w_ty(w),
                    Some(abilities) => w.annotate_gen(w_ty, abilities, |w, annot| {
                        w.list(annot, "+", |w, a| {
                            a.ast_debug(w);
                            false
                        })
                    }),
                }
            }
            Type_::Var(tv) => w.write(&format!("#{}", tv.0)),
            Type_::Anything => w.write("_"),
            Type_::UnresolvedError => w.write("_|_"),
        }
    }
}

impl AstDebug for Vec<Type> {
    fn ast_debug(&self, w: &mut AstWriter) {
        w.comma(self, |w, s| s.ast_debug(w))
    }
}

impl AstDebug for VecDeque<SequenceItem> {
    fn ast_debug(&self, w: &mut AstWriter) {
        w.semicolon(self, |w, item| item.ast_debug(w))
    }
}

impl AstDebug for SequenceItem_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use SequenceItem_ as I;
        match self {
            I::Seq(e) => e.ast_debug(w),
            I::Declare(sp!(_, bs), ty_opt) => {
                w.write("let ");
                bs.ast_debug(w);
                if let Some(ty) = ty_opt {
                    ty.ast_debug(w)
                }
            }
            I::Bind(sp!(_, bs), e) => {
                w.write("let ");
                bs.ast_debug(w);
                w.write(" = ");
                e.ast_debug(w);
            }
        }
    }
}

impl AstDebug for Exp_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use Exp_ as E;
        match self {
            E::Unit { trailing } if !trailing => w.write("()"),
            E::Unit {
                trailing: _trailing,
            } => w.write("/*()*/"),
            E::Value(v) => v.ast_debug(w),
            E::Move(v) => w.write(&format!("move {}", v)),
            E::Copy(v) => w.write(&format!("copy {}", v)),
            E::Use(v) => w.write(&format!("{}", v)),
            E::Constant(None, c) => w.write(&format!("{}", c)),
            E::Constant(Some(m), c) => w.write(&format!("{}::{}", m, c)),
            E::ModuleCall(m, f, tys_opt, sp!(_, rhs)) => {
                w.write(&format!("{}::{}", m, f));
                if let Some(ss) = tys_opt {
                    w.write("<");
                    ss.ast_debug(w);
                    w.write(">");
                }
                w.write("(");
                w.comma(rhs, |w, e| e.ast_debug(w));
                w.write(")");
            }
            E::Builtin(bf, sp!(_, rhs)) => {
                bf.ast_debug(w);
                w.write("(");
                w.comma(rhs, |w, e| e.ast_debug(w));
                w.write(")");
            }
            E::Pack(m, s, tys_opt, fields) => {
                w.write(&format!("{}::{}", m, s));
                if let Some(ss) = tys_opt {
                    w.write("<");
                    ss.ast_debug(w);
                    w.write(">");
                }
                w.write("{");
                w.comma(fields, |w, (_, f, idx_e)| {
                    let (idx, e) = idx_e;
                    w.write(&format!("{}#{}: ", idx, f));
                    e.ast_debug(w);
                });
                w.write("}");
            }
            E::IfElse(b, t, f) => {
                w.write("if (");
                b.ast_debug(w);
                w.write(") ");
                t.ast_debug(w);
                w.write(" else ");
                f.ast_debug(w);
            }
            E::While(b, e) => {
                w.write("while (");
                b.ast_debug(w);
                w.write(")");
                e.ast_debug(w);
            }
            E::Loop(e) => {
                w.write("loop ");
                e.ast_debug(w);
            }
            E::Block(seq) => w.block(|w| seq.ast_debug(w)),
            E::ExpList(es) => {
                w.write("(");
                w.comma(es, |w, e| e.ast_debug(w));
                w.write(")");
            }

            E::Assign(sp!(_, lvalues), rhs) => {
                lvalues.ast_debug(w);
                w.write(" = ");
                rhs.ast_debug(w);
            }
            E::FieldMutate(ed, rhs) => {
                ed.ast_debug(w);
                w.write(" = ");
                rhs.ast_debug(w);
            }
            E::Mutate(lhs, rhs) => {
                w.write("*");
                lhs.ast_debug(w);
                w.write(" = ");
                rhs.ast_debug(w);
            }

            E::Return(e) => {
                w.write("return ");
                e.ast_debug(w);
            }
            E::Abort(e) => {
                w.write("abort ");
                e.ast_debug(w);
            }
            E::Break => w.write("break"),
            E::Continue => w.write("continue"),
            E::Dereference(e) => {
                w.write("*");
                e.ast_debug(w)
            }
            E::UnaryExp(op, e) => {
                op.ast_debug(w);
                w.write(" ");
                e.ast_debug(w);
            }
            E::BinopExp(l, op, r) => {
                l.ast_debug(w);
                w.write(" ");
                op.ast_debug(w);
                w.write(" ");
                r.ast_debug(w)
            }
            E::Borrow(mut_, e) => {
                w.write("&");
                if *mut_ {
                    w.write("mut ");
                }
                e.ast_debug(w);
            }
            E::DerefBorrow(ed) => {
                w.write("(&*)");
                ed.ast_debug(w)
            }
            E::Cast(e, ty) => {
                w.write("(");
                e.ast_debug(w);
                w.write(" as ");
                ty.ast_debug(w);
                w.write(")");
            }
            E::Annotate(e, ty) => {
                w.write("(");
                e.ast_debug(w);
                w.write(": ");
                ty.ast_debug(w);
                w.write(")");
            }
            E::Spec(u, used_locals) => {
                w.write(&format!("spec #{}", u));
                if !used_locals.is_empty() {
                    w.write("uses [");
                    w.comma(used_locals, |w, n| w.write(&format!("{}", n)));
                    w.write("]");
                }
            }
            E::UnresolvedError => w.write("_|_"),
        }
    }
}

impl AstDebug for BuiltinFunction_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use BuiltinFunction_ as F;
        let (n, bt) = match self {
            F::MoveTo(bt) => (F::MOVE_TO, bt),
            F::MoveFrom(bt) => (F::MOVE_FROM, bt),
            F::BorrowGlobal(true, bt) => (F::BORROW_GLOBAL_MUT, bt),
            F::BorrowGlobal(false, bt) => (F::BORROW_GLOBAL, bt),
            F::Exists(bt) => (F::EXISTS, bt),
            F::Freeze(bt) => (F::FREEZE, bt),
            F::Assert => (F::ASSERT, &None),
        };
        w.write(n);
        if let Some(bt) = bt {
            w.write("<");
            bt.ast_debug(w);
            w.write(">");
        }
    }
}

impl AstDebug for ExpDotted_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use ExpDotted_ as D;
        match self {
            D::Exp(e) => e.ast_debug(w),
            D::Dot(e, n) => {
                e.ast_debug(w);
                w.write(&format!(".{}", n))
            }
        }
    }
}

impl AstDebug for Vec<LValue> {
    fn ast_debug(&self, w: &mut AstWriter) {
        let parens = self.len() != 1;
        if parens {
            w.write("(");
        }
        w.comma(self, |w, b| b.ast_debug(w));
        if parens {
            w.write(")");
        }
    }
}

impl AstDebug for LValue_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use LValue_ as L;
        match self {
            L::Ignore => w.write("_"),
            L::Var(v) => w.write(&format!("{}", v)),
            L::Unpack(m, s, tys_opt, fields) => {
                w.write(&format!("{}::{}", m, s));
                if let Some(ss) = tys_opt {
                    w.write("<");
                    ss.ast_debug(w);
                    w.write(">");
                }
                w.write("{");
                w.comma(fields, |w, (_, f, idx_b)| {
                    let (idx, b) = idx_b;
                    w.write(&format!("{}#{}: ", idx, f));
                    b.ast_debug(w);
                });
                w.write("}");
            }
        }
    }
}
