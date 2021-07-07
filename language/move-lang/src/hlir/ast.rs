// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    expansion::ast::{
        ability_modifiers_ast_debug, AbilitySet, Attribute, Friend, ModuleIdent, SpecId, Value,
    },
    naming::ast::{BuiltinTypeName, BuiltinTypeName_, StructTypeParameter, TParam},
    parser::ast::{BinOp, ConstantName, Field, FunctionName, StructName, UnaryOp, Var, Visibility},
    shared::{ast_debug::*, unique_map::UniqueMap, AddressBytes, Name},
};
use move_ir_types::location::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

// High Level IR

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
pub enum StructFields {
    Defined(Vec<(Field, BaseType)>),
    Native(Loc),
}

//**************************************************************************************************
// Constants
//**************************************************************************************************

#[derive(PartialEq, Debug, Clone)]
pub struct Constant {
    pub attributes: Vec<Attribute>,
    pub loc: Loc,
    pub signature: BaseType,
    pub value: (UniqueMap<Var, SingleType>, Block),
}

//**************************************************************************************************
// Functions
//**************************************************************************************************

#[derive(PartialEq, Debug, Clone)]
pub struct FunctionSignature {
    pub type_parameters: Vec<TParam>,
    pub parameters: Vec<(Var, SingleType)>,
    pub return_type: Type,
}

#[derive(PartialEq, Debug, Clone)]
pub enum FunctionBody_ {
    Native,
    Defined {
        locals: UniqueMap<Var, SingleType>,
        body: Block,
    },
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
// Types
//**************************************************************************************************

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub enum TypeName_ {
    Builtin(BuiltinTypeName),
    ModuleType(ModuleIdent, StructName),
}
pub type TypeName = Spanned<TypeName_>;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum BaseType_ {
    Param(TParam),
    Apply(AbilitySet, TypeName, Vec<BaseType>),
    Unreachable,
    UnresolvedError,
}
pub type BaseType = Spanned<BaseType_>;

#[derive(Debug, PartialEq, Clone)]
pub enum SingleType_ {
    Base(BaseType),
    Ref(bool, BaseType),
}
pub type SingleType = Spanned<SingleType_>;

#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Type_ {
    Unit,
    Single(SingleType),
    Multiple(Vec<SingleType>),
}
pub type Type = Spanned<Type_>;

//**************************************************************************************************
// Statements
//**************************************************************************************************

#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Statement_ {
    Command(Command),
    IfElse {
        cond: Box<Exp>,
        if_block: Block,
        else_block: Block,
    },
    While {
        cond: (Block, Box<Exp>),
        block: Block,
    },
    Loop {
        block: Block,
        has_break: bool,
    },
}
pub type Statement = Spanned<Statement_>;

pub type Block = VecDeque<Statement>;

pub type BasicBlocks = BTreeMap<Label, BasicBlock>;

pub type BasicBlock = VecDeque<Command>;

#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone, PartialOrd, Ord)]
pub struct Label(pub usize);

//**************************************************************************************************
// Commands
//**************************************************************************************************

#[derive(Debug, PartialEq, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Command_ {
    Assign(Vec<LValue>, Box<Exp>),
    Mutate(Box<Exp>, Box<Exp>),
    Abort(Exp),
    Return {
        from_user: bool,
        exp: Exp,
    },
    Break,
    Continue,
    IgnoreAndPop {
        pop_num: usize,
        exp: Exp,
    },
    Jump {
        from_user: bool,
        target: Label,
    },
    JumpIf {
        cond: Exp,
        if_true: Label,
        if_false: Label,
    },
}
pub type Command = Spanned<Command_>;

#[derive(Debug, PartialEq, Clone)]
pub enum LValue_ {
    Ignore,
    Var(Var, Box<SingleType>),
    Unpack(StructName, Vec<BaseType>, Vec<(Field, LValue)>),
}
pub type LValue = Spanned<LValue_>;

//**************************************************************************************************
// Expressions
//**************************************************************************************************

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum UnitCase {
    Trailing,
    Implicit,
    FromUser,
}

#[derive(Debug, PartialEq, Clone)]
pub struct ModuleCall {
    pub module: ModuleIdent,
    pub name: FunctionName,
    pub type_arguments: Vec<BaseType>,
    pub arguments: Box<Exp>,
    pub acquires: BTreeMap<StructName, Loc>,
}

#[derive(Debug, PartialEq, Clone)]
pub enum BuiltinFunction_ {
    MoveTo(BaseType),
    MoveFrom(BaseType),
    BorrowGlobal(bool, BaseType),
    Exists(BaseType),
}
pub type BuiltinFunction = Spanned<BuiltinFunction_>;

#[derive(Debug, PartialEq, Clone)]
pub enum UnannotatedExp_ {
    Unit { case: UnitCase },
    Value(Value),
    Move { from_user: bool, var: Var },
    Copy { from_user: bool, var: Var },
    Constant(ConstantName),

    ModuleCall(Box<ModuleCall>),
    Builtin(Box<BuiltinFunction>, Box<Exp>),
    Freeze(Box<Exp>),

    Dereference(Box<Exp>),
    UnaryExp(UnaryOp, Box<Exp>),
    BinopExp(Box<Exp>, BinOp, Box<Exp>),

    Pack(StructName, Vec<BaseType>, Vec<(Field, BaseType, Exp)>),
    ExpList(Vec<ExpListItem>),

    Borrow(bool, Box<Exp>, Field),
    BorrowLocal(bool, Var),

    Cast(Box<Exp>, BuiltinTypeName),

    Unreachable,

    Spec(SpecId, BTreeMap<Var, SingleType>),

    UnresolvedError,
}
pub type UnannotatedExp = Spanned<UnannotatedExp_>;
#[derive(Debug, PartialEq, Clone)]
pub struct Exp {
    pub ty: Type,
    pub exp: UnannotatedExp,
}
pub fn exp(ty: Type, exp: UnannotatedExp) -> Exp {
    Exp { ty, exp }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ExpListItem {
    Single(Exp, Box<SingleType>),
    Splat(Loc, Exp, Vec<SingleType>),
}

//**************************************************************************************************
// impls
//**************************************************************************************************

impl FunctionSignature {
    pub fn is_parameter(&self, v: &Var) -> bool {
        self.parameters
            .iter()
            .any(|(parameter_name, _)| parameter_name == v)
    }
}

impl Command_ {
    pub fn is_terminal(&self) -> bool {
        use Command_::*;
        match self {
            Break | Continue => panic!("ICE break/continue not translated to jumps"),
            Assign(_, _) | Mutate(_, _) | IgnoreAndPop { .. } => false,
            Abort(_) | Return { .. } | Jump { .. } | JumpIf { .. } => true,
        }
    }

    pub fn is_exit(&self) -> bool {
        use Command_::*;
        match self {
            Break | Continue => panic!("ICE break/continue not translated to jumps"),
            Assign(_, _) | Mutate(_, _) | IgnoreAndPop { .. } | Jump { .. } | JumpIf { .. } => {
                false
            }
            Abort(_) | Return { .. } => true,
        }
    }

    pub fn is_unit(&self) -> bool {
        use Command_::*;
        match self {
            Break | Continue => panic!("ICE break/continue not translated to jumps"),
            Assign(ls, e) => ls.is_empty() && e.is_unit(),
            IgnoreAndPop { exp: e, .. } => e.is_unit(),

            Mutate(_, _) | Return { .. } | Abort(_) | JumpIf { .. } | Jump { .. } => false,
        }
    }

    pub fn successors(&self) -> BTreeSet<Label> {
        use Command_::*;

        let mut successors = BTreeSet::new();
        match self {
            Break | Continue => panic!("ICE break/continue not translated to jumps"),
            Mutate(_, _) | Assign(_, _) | IgnoreAndPop { .. } => {
                panic!("ICE Should not be last command in block")
            }
            Abort(_) | Return { .. } => (),
            Jump { target, .. } => {
                successors.insert(*target);
            }
            JumpIf {
                if_true, if_false, ..
            } => {
                successors.insert(*if_true);
                successors.insert(*if_false);
            }
        }
        successors
    }
}

impl Exp {
    pub fn is_unit(&self) -> bool {
        self.exp.value.is_unit()
    }
}

impl UnannotatedExp_ {
    pub fn is_unit(&self) -> bool {
        matches!(self, UnannotatedExp_::Unit { case: _case })
    }
}

impl BaseType_ {
    pub fn builtin(loc: Loc, b_: BuiltinTypeName_, ty_args: Vec<BaseType>) -> BaseType {
        use BuiltinTypeName_::*;

        let kind = match b_ {
            U8 | U64 | U128 | Bool | Address => AbilitySet::primitives(loc),
            Signer => AbilitySet::signer(loc),
            Vector => {
                let declared_abilities = AbilitySet::collection(loc);
                let ty_arg_abilities = {
                    assert!(ty_args.len() == 1);
                    ty_args[0].value.abilities(ty_args[0].loc)
                };
                AbilitySet::from_abilities(
                    declared_abilities
                        .into_iter()
                        .filter(|ab| ty_arg_abilities.has_ability_(ab.value.requires())),
                )
                .unwrap()
            }
        };
        let n = sp(loc, TypeName_::Builtin(sp(loc, b_)));
        sp(loc, BaseType_::Apply(kind, n, ty_args))
    }

    pub fn abilities(&self, loc: Loc) -> AbilitySet {
        match self {
            BaseType_::Apply(abilities, _, _) | BaseType_::Param(TParam { abilities, .. }) => {
                abilities.clone()
            }
            BaseType_::Unreachable | BaseType_::UnresolvedError => AbilitySet::all(loc),
        }
    }

    pub fn bool(loc: Loc) -> BaseType {
        Self::builtin(loc, BuiltinTypeName_::Bool, vec![])
    }

    pub fn address(loc: Loc) -> BaseType {
        Self::builtin(loc, BuiltinTypeName_::Address, vec![])
    }

    pub fn u8(loc: Loc) -> BaseType {
        Self::builtin(loc, BuiltinTypeName_::U8, vec![])
    }

    pub fn u64(loc: Loc) -> BaseType {
        Self::builtin(loc, BuiltinTypeName_::U64, vec![])
    }

    pub fn u128(loc: Loc) -> BaseType {
        Self::builtin(loc, BuiltinTypeName_::U128, vec![])
    }
}

impl SingleType_ {
    pub fn base(sp!(loc, b_): BaseType) -> SingleType {
        sp(loc, SingleType_::Base(sp(loc, b_)))
    }

    pub fn bool(loc: Loc) -> SingleType {
        Self::base(BaseType_::bool(loc))
    }

    pub fn address(loc: Loc) -> SingleType {
        Self::base(BaseType_::address(loc))
    }

    pub fn u8(loc: Loc) -> SingleType {
        Self::base(BaseType_::u8(loc))
    }

    pub fn u64(loc: Loc) -> SingleType {
        Self::base(BaseType_::u64(loc))
    }

    pub fn u128(loc: Loc) -> SingleType {
        Self::base(BaseType_::u128(loc))
    }

    pub fn abilities(&self, loc: Loc) -> AbilitySet {
        match self {
            SingleType_::Ref(_, _) => AbilitySet::references(loc),
            SingleType_::Base(b) => b.value.abilities(loc),
        }
    }
}

impl Type_ {
    pub fn base(b: BaseType) -> Type {
        Self::single(SingleType_::base(b))
    }

    pub fn single(sp!(loc, s_): SingleType) -> Type {
        sp(loc, Type_::Single(sp(loc, s_)))
    }

    pub fn bool(loc: Loc) -> Type {
        Self::single(SingleType_::bool(loc))
    }

    pub fn address(loc: Loc) -> Type {
        Self::single(SingleType_::address(loc))
    }

    pub fn u8(loc: Loc) -> Type {
        Self::single(SingleType_::u8(loc))
    }

    pub fn u64(loc: Loc) -> Type {
        Self::single(SingleType_::u64(loc))
    }

    pub fn u128(loc: Loc) -> Type {
        Self::single(SingleType_::u128(loc))
    }

    pub fn type_at_index(&self, idx: usize) -> &SingleType {
        match self {
            Type_::Unit => panic!("ICE type mismatch on index lookup"),
            Type_::Single(s) => {
                assert!(idx == 0);
                s
            }
            Type_::Multiple(ss) => {
                assert!(idx < ss.len());
                ss.get(idx).unwrap()
            }
        }
    }

    pub fn from_vec(loc: Loc, mut ss: Vec<SingleType>) -> Type {
        let t_ = match ss.len() {
            0 => Type_::Unit,
            1 => Type_::Single(ss.pop().unwrap()),
            _ => Type_::Multiple(ss),
        };
        sp(loc, t_)
    }
}

//**************************************************************************************************
// Display
//**************************************************************************************************

impl std::fmt::Display for TypeName_ {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        use TypeName_::*;
        match self {
            Builtin(b) => write!(f, "{}", b),
            ModuleType(m, n) => write!(f, "{}::{}", m, n),
        }
    }
}

impl std::fmt::Display for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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
                w.list(fields, ";", |w, (f, bt)| {
                    w.write(&format!("{}: ", f));
                    bt.ast_debug(w);
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
            w.write(" ");
        }
        match &body.value {
            FunctionBody_::Defined { locals, body } => w.block(|w| (locals, body).ast_debug(w)),
            FunctionBody_::Native => w.writeln(";"),
        }
    }
}

impl AstDebug for (UniqueMap<Var, SingleType>, Block) {
    fn ast_debug(&self, w: &mut AstWriter) {
        let (locals, body) = self;
        (locals, body).ast_debug(w)
    }
}

impl AstDebug for (&UniqueMap<Var, SingleType>, &Block) {
    fn ast_debug(&self, w: &mut AstWriter) {
        let (locals, body) = self;
        w.write("locals:");
        w.indent(4, |w| {
            w.list(*locals, ",", |w, (_, v, st)| {
                w.write(&format!("{}: ", v));
                st.ast_debug(w);
                true
            })
        });
        w.new_line();
        body.ast_debug(w);
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
        w.block(|w| value.ast_debug(w));
        w.write(";");
    }
}

impl AstDebug for TypeName_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            TypeName_::Builtin(bt) => bt.ast_debug(w),
            TypeName_::ModuleType(m, s) => w.write(&format!("{}::{}", m, s)),
        }
    }
}

impl AstDebug for BaseType_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            BaseType_::Param(tp) => tp.ast_debug(w),
            BaseType_::Apply(abilities, m, ss) => {
                w.annotate_gen(
                    |w| {
                        m.ast_debug(w);
                        if !ss.is_empty() {
                            w.write("<");
                            ss.ast_debug(w);
                            w.write(">");
                        }
                    },
                    abilities,
                    |w, abilities| {
                        w.list(abilities, "+", |w, ab| {
                            ab.ast_debug(w);
                            false
                        })
                    },
                );
            }
            BaseType_::Unreachable => w.write("_|_"),
            BaseType_::UnresolvedError => w.write("_"),
        }
    }
}

impl AstDebug for SingleType_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            SingleType_::Base(b) => b.ast_debug(w),
            SingleType_::Ref(mut_, s) => {
                w.write("&");
                if *mut_ {
                    w.write("mut ");
                }
                s.ast_debug(w)
            }
        }
    }
}

impl AstDebug for Type_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            Type_::Unit => w.write("()"),
            Type_::Single(s) => s.ast_debug(w),
            Type_::Multiple(ss) => {
                w.write("(");
                ss.ast_debug(w);
                w.write(")")
            }
        }
    }
}

impl AstDebug for Vec<SingleType> {
    fn ast_debug(&self, w: &mut AstWriter) {
        w.comma(self, |w, s| s.ast_debug(w))
    }
}

impl AstDebug for Vec<BaseType> {
    fn ast_debug(&self, w: &mut AstWriter) {
        w.comma(self, |w, s| s.ast_debug(w))
    }
}

impl AstDebug for VecDeque<Statement> {
    fn ast_debug(&self, w: &mut AstWriter) {
        w.semicolon(self, |w, stmt| stmt.ast_debug(w))
    }
}

impl AstDebug for (Block, Box<Exp>) {
    fn ast_debug(&self, w: &mut AstWriter) {
        let (block, exp) = self;
        if block.is_empty() {
            exp.ast_debug(w);
        } else {
            w.block(|w| {
                block.ast_debug(w);
                w.writeln(";");
                exp.ast_debug(w);
            })
        }
    }
}

impl AstDebug for Statement_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use Statement_ as S;
        match self {
            S::Command(cmd) => cmd.ast_debug(w),
            S::IfElse {
                cond,
                if_block,
                else_block,
            } => {
                w.write("if (");
                cond.ast_debug(w);
                w.write(") ");
                w.block(|w| if_block.ast_debug(w));
                w.write(" else ");
                w.block(|w| else_block.ast_debug(w));
            }
            S::While { cond, block } => {
                w.write("while (");
                cond.ast_debug(w);
                w.write(")");
                w.block(|w| block.ast_debug(w))
            }
            S::Loop { block, has_break } => {
                w.write("loop");
                if *has_break {
                    w.write("#has_break");
                }
                w.write(" ");
                w.block(|w| block.ast_debug(w))
            }
        }
    }
}

impl AstDebug for Command_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use Command_ as C;
        match self {
            C::Assign(lvalues, rhs) => {
                lvalues.ast_debug(w);
                w.write(" = ");
                rhs.ast_debug(w);
            }
            C::Mutate(lhs, rhs) => {
                w.write("*");
                lhs.ast_debug(w);
                w.write(" = ");
                rhs.ast_debug(w);
            }
            C::Abort(e) => {
                w.write("abort ");
                e.ast_debug(w);
            }
            C::Return { exp: e, from_user } if *from_user => {
                w.write("return@");
                e.ast_debug(w);
            }
            C::Return { exp: e, .. } => {
                w.write("return ");
                e.ast_debug(w);
            }
            C::Break => w.write("break"),
            C::Continue => w.write("continue"),
            C::IgnoreAndPop { pop_num, exp } => {
                w.write("pop ");
                w.comma(0..*pop_num, |w, _| w.write("_"));
                w.write(" = ");
                exp.ast_debug(w);
            }
            C::Jump { target, from_user } if *from_user => w.write(&format!("jump@{}", target.0)),
            C::Jump { target, .. } => w.write(&format!("jump {}", target.0)),
            C::JumpIf {
                cond,
                if_true,
                if_false,
            } => {
                w.write("jump_if(");
                cond.ast_debug(w);
                w.write(&format!(") {} else {}", if_true.0, if_false.0));
            }
        }
    }
}

impl AstDebug for Exp {
    fn ast_debug(&self, w: &mut AstWriter) {
        let Exp { ty, exp } = self;
        w.annotate(|w| exp.ast_debug(w), ty)
    }
}

impl AstDebug for UnannotatedExp_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use UnannotatedExp_ as E;
        match self {
            E::Unit {
                case: UnitCase::FromUser,
            } => w.write("()"),
            E::Unit {
                case: UnitCase::Implicit,
            } => w.write("/*()*/"),
            E::Unit {
                case: UnitCase::Trailing,
            } => w.write("/*;()*/"),
            E::Value(v) => v.ast_debug(w),
            E::Move {
                from_user: false,
                var: v,
            } => w.write(&format!("move {}", v)),
            E::Move {
                from_user: true,
                var: v,
            } => w.write(&format!("move@{}", v)),
            E::Copy {
                from_user: false,
                var: v,
            } => w.write(&format!("copy {}", v)),
            E::Copy {
                from_user: true,
                var: v,
            } => w.write(&format!("copy@{}", v)),
            E::Constant(c) => w.write(&format!("{}", c)),
            E::ModuleCall(mcall) => {
                mcall.ast_debug(w);
            }
            E::Builtin(bf, rhs) => {
                bf.ast_debug(w);
                w.write("(");
                rhs.ast_debug(w);
                w.write(")");
            }
            E::Freeze(e) => {
                w.write("freeze(");
                e.ast_debug(w);
                w.write(")");
            }
            E::Pack(s, tys, fields) => {
                w.write(&format!("{}", s));
                w.write("<");
                tys.ast_debug(w);
                w.write(">");
                w.write("{");
                w.comma(fields, |w, (f, bt, e)| {
                    w.annotate(|w| w.write(&format!("{}", f)), bt);
                    w.write(": ");
                    e.ast_debug(w);
                });
                w.write("}");
            }

            E::ExpList(es) => {
                w.write("(");
                w.comma(es, |w, e| e.ast_debug(w));
                w.write(")");
            }

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
            E::Borrow(mut_, e, f) => {
                w.write("&");
                if *mut_ {
                    w.write("mut ");
                }
                e.ast_debug(w);
                w.write(&format!(".{}", f));
            }
            E::BorrowLocal(mut_, v) => {
                w.write("&");
                if *mut_ {
                    w.write("mut ");
                }
                w.write(&format!("{}", v));
            }
            E::Cast(e, bt) => {
                w.write("(");
                e.ast_debug(w);
                w.write(" as ");
                bt.ast_debug(w);
                w.write(")");
            }
            E::Spec(u, used_locals) => {
                w.write(&format!("spec #{}", u));
                if !used_locals.is_empty() {
                    w.write("uses [");
                    w.comma(used_locals, |w, (n, st)| {
                        w.annotate(|w| w.write(&format!("{}", n)), st)
                    });
                    w.write("]");
                }
            }
            E::UnresolvedError => w.write("_|_"),
            E::Unreachable => w.write("unreachable"),
        }
    }
}

impl AstDebug for ModuleCall {
    fn ast_debug(&self, w: &mut AstWriter) {
        let ModuleCall {
            module,
            name,
            type_arguments,
            acquires,
            arguments,
        } = self;
        w.write(&format!("{}::{}", module, name));
        if !acquires.is_empty() {
            w.write("[acquires: [");
            w.comma(acquires.keys(), |w, s| w.write(&format!("{}", s)));
            w.write("]], ");
        }
        w.write("<");
        type_arguments.ast_debug(w);
        w.write(">");
        w.write("(");
        arguments.ast_debug(w);
        w.write(")");
    }
}

impl AstDebug for BuiltinFunction_ {
    fn ast_debug(&self, w: &mut AstWriter) {
        use crate::naming::ast::BuiltinFunction_ as NF;
        use BuiltinFunction_ as F;
        let (n, bt) = match self {
            F::MoveTo(bt) => (NF::MOVE_TO, bt),
            F::MoveFrom(bt) => (NF::MOVE_FROM, bt),
            F::BorrowGlobal(true, bt) => (NF::BORROW_GLOBAL_MUT, bt),
            F::BorrowGlobal(false, bt) => (NF::BORROW_GLOBAL, bt),
            F::Exists(bt) => (NF::EXISTS, bt),
        };
        w.write(n);
        w.write("<");
        bt.ast_debug(w);
        w.write(">");
    }
}

impl AstDebug for ExpListItem {
    fn ast_debug(&self, w: &mut AstWriter) {
        match self {
            ExpListItem::Single(e, st) => w.annotate(|w| e.ast_debug(w), st),
            ExpListItem::Splat(_, e, ss) => {
                w.write("~");
                w.annotate(|w| e.ast_debug(w), ss)
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
        w.comma(self, |w, a| a.ast_debug(w));
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
            L::Var(v, st) => {
                w.write(&format!("({}: ", v));
                st.ast_debug(w);
                w.write(")");
            }
            L::Unpack(s, tys, fields) => {
                w.write(&format!("{}", s));
                w.write("<");
                tys.ast_debug(w);
                w.write(">");
                w.write("{");
                w.comma(fields, |w, (f, l)| {
                    w.write(&format!("{}: ", f));
                    l.ast_debug(w)
                });
                w.write("}");
            }
        }
    }
}
