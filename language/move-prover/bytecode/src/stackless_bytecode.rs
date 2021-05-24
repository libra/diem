// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::function_target::FunctionTarget;
use itertools::Itertools;
use move_binary_format::file_format::CodeOffset;
use move_model::{
    ast::{Exp, ExpData, MemoryLabel, TempIndex},
    exp_rewriter::{ExpRewriter, ExpRewriterFunctions, RewriteTarget},
    model::{FunId, GlobalEnv, ModuleId, NodeId, QualifiedInstId, SpecVarId, StructId},
    ty::{Type, TypeDisplayContext},
};
use num::BigUint;
use std::{collections::BTreeMap, fmt, fmt::Formatter};

/// A label for a branch destination.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct Label(u16);

impl Label {
    pub fn new(idx: usize) -> Self {
        Self(idx as u16)
    }

    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// An id for an attribute attached to an instruction.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct AttrId(u16);

impl AttrId {
    pub fn new(idx: usize) -> Self {
        Self(idx as u16)
    }

    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// An id for a spec block. A spec block can contain assumes and asserts to be enforced at a
/// program point.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub struct SpecBlockId(u16);

impl SpecBlockId {
    pub fn new(idx: usize) -> Self {
        Self(idx as u16)
    }

    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// The kind of an assignment in the bytecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssignKind {
    /// The assign copies the lhs value.
    Copy,
    /// The assign moves the lhs value.
    Move,
    /// The assign stores the lhs value.
    // TODO: figure out why we can't treat this as either copy or move. The lifetime analysis
    // currently makes a difference of this case. It originates from stack code where Copy
    // and Move push on the stack and Store pops.
    Store,
}

/// The type of variable that is being havoc-ed
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HavocKind {
    /// Havoc a value
    Value,
    /// Havoc the value part in a mutation, but keep its pointer unchanged
    MutationValue,
    /// Havoc everything in a mutation
    MutationAll,
}

/// A constant value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Constant {
    Bool(bool),
    U8(u8),
    U64(u64),
    U128(u128),
    Address(BigUint),
    ByteArray(Vec<u8>),
}

/// An operation -- target of a call. This contains user functions, builtin functions, and
/// operators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    // User function
    Function(ModuleId, FunId, Vec<Type>),

    // Markers for beginning and end of transformed
    // opaque function calls (the function call is replaced
    // by assumes/asserts/gotos, but it is necessary to
    // add more assumes/asserts later in the pipeline.
    OpaqueCallBegin(ModuleId, FunId, Vec<Type>),
    OpaqueCallEnd(ModuleId, FunId, Vec<Type>),

    // Pack/Unpack
    Pack(ModuleId, StructId, Vec<Type>),
    Unpack(ModuleId, StructId, Vec<Type>),

    // Resources
    MoveTo(ModuleId, StructId, Vec<Type>),
    MoveFrom(ModuleId, StructId, Vec<Type>),
    Exists(ModuleId, StructId, Vec<Type>),

    // Borrow
    BorrowLoc,
    BorrowField(ModuleId, StructId, Vec<Type>, usize),
    BorrowGlobal(ModuleId, StructId, Vec<Type>),

    // Get
    GetField(ModuleId, StructId, Vec<Type>, usize),
    GetGlobal(ModuleId, StructId, Vec<Type>),

    // Builtins
    Destroy,
    ReadRef,
    WriteRef,
    FreezeRef,
    Havoc(HavocKind),
    Stop,

    // Memory model
    IsParent(BorrowNode, BorrowEdge),
    WriteBack(BorrowNode, BorrowEdge),
    UnpackRef,
    PackRef,
    UnpackRefDeep,
    PackRefDeep,
    GlobalAddress,

    // Unary
    CastU8,
    CastU64,
    CastU128,
    Not,

    // Binary
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitOr,
    BitAnd,
    Xor,
    Shl,
    Shr,
    Lt,
    Gt,
    Le,
    Ge,
    Or,
    And,
    Eq,
    Neq,

    // Debugging
    TraceLocal(TempIndex),
    TraceReturn(usize),
    TraceAbort,
    TraceExp(NodeId),

    // Event
    EmitEvent,
    EventStoreDiverge,
}

impl Operation {
    /// Returns true of the operation can cause abort.
    pub fn can_abort(&self) -> bool {
        match self {
            Operation::Function(_, _, _) => true,
            Operation::OpaqueCallBegin(_, _, _) => false,
            Operation::OpaqueCallEnd(_, _, _) => false,
            Operation::Pack(_, _, _) => false,
            Operation::Unpack(_, _, _) => false,
            Operation::MoveTo(_, _, _) => true,
            Operation::MoveFrom(_, _, _) => true,
            Operation::Exists(_, _, _) => false,
            Operation::BorrowLoc => false,
            Operation::BorrowField(_, _, _, _) => false,
            Operation::BorrowGlobal(_, _, _) => true,
            Operation::GetField(_, _, _, _) => false,
            Operation::GetGlobal(_, _, _) => true,
            Operation::Destroy => false,
            Operation::ReadRef => false,
            Operation::WriteRef => false,
            Operation::FreezeRef => false,
            Operation::Havoc(_) => false,
            Operation::Stop => false,
            Operation::WriteBack(..) => false,
            Operation::IsParent(..) => false,
            Operation::UnpackRef => false,
            Operation::PackRef => false,
            Operation::UnpackRefDeep => false,
            Operation::PackRefDeep => false,
            Operation::GlobalAddress => false,
            Operation::CastU8 => true,
            Operation::CastU64 => true,
            Operation::CastU128 => true,
            Operation::Not => false,
            Operation::Add => true,
            Operation::Sub => true,
            Operation::Mul => true,
            Operation::Div => true,
            Operation::Mod => true,
            Operation::BitOr => false,
            Operation::BitAnd => false,
            Operation::Xor => false,
            Operation::Shl => false,
            Operation::Shr => false,
            Operation::Lt => false,
            Operation::Gt => false,
            Operation::Le => false,
            Operation::Ge => false,
            Operation::Or => false,
            Operation::And => false,
            Operation::Eq => false,
            Operation::Neq => false,
            Operation::TraceLocal(..) => false,
            Operation::TraceAbort => false,
            Operation::TraceReturn(..) => false,
            Operation::TraceExp(..) => false,
            Operation::EmitEvent => false,
            Operation::EventStoreDiverge => false,
        }
    }
}

/// A borrow node -- used in memory operations.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum BorrowNode {
    GlobalRoot(QualifiedInstId<StructId>),
    LocalRoot(TempIndex),
    Reference(TempIndex),
    // Used in summaries to represent a returned mutation at return index. This does not
    // appear in bytecode instructions.
    ReturnPlaceholder(usize),
}

impl BorrowNode {
    pub fn get_ref(&self) -> Option<TempIndex> {
        if let BorrowNode::Reference(idx) = self {
            Some(*idx)
        } else {
            None
        }
    }
}

/// A borrow edge.
#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub enum BorrowEdge {
    /// Direct borrow.
    Direct,
    /// Field borrow with static offset.
    Field(QualifiedInstId<StructId>, usize),
    /// Vector borrow with dynamic index.
    Index,
    /// Composed sequence of edges.
    Hyper(Vec<BorrowEdge>),
}

impl BorrowEdge {
    pub fn flatten(&self) -> Vec<&BorrowEdge> {
        if let BorrowEdge::Hyper(edges) = self {
            edges.iter().collect_vec()
        } else {
            vec![self]
        }
    }
}
/// A specification property kind.
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum PropKind {
    Assert,
    Assume,
    Modifies,
}

/// Information about the action to take on abort. The label represents the
/// destination to jump to, and the temporary where to store the abort code before
/// jump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbortAction(pub Label, pub TempIndex);

/// The stackless bytecode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bytecode {
    Assign(AttrId, TempIndex, TempIndex, AssignKind),

    Call(
        AttrId,
        Vec<TempIndex>,
        Operation,
        Vec<TempIndex>,
        Option<AbortAction>,
    ),
    Ret(AttrId, Vec<TempIndex>),

    Load(AttrId, TempIndex, Constant),
    Branch(AttrId, Label, Label, TempIndex),
    Jump(AttrId, Label),
    Label(AttrId, Label),
    Abort(AttrId, TempIndex),
    Nop(AttrId),

    SaveMem(AttrId, MemoryLabel, QualifiedInstId<StructId>),
    SaveSpecVar(AttrId, MemoryLabel, QualifiedInstId<SpecVarId>),
    Prop(AttrId, PropKind, Exp),
}

impl Bytecode {
    pub fn get_attr_id(&self) -> AttrId {
        use Bytecode::*;
        match self {
            Assign(id, ..)
            | Call(id, ..)
            | Ret(id, ..)
            | Load(id, ..)
            | Branch(id, ..)
            | Jump(id, ..)
            | Label(id, ..)
            | Abort(id, ..)
            | Nop(id)
            | SaveMem(id, ..)
            | SaveSpecVar(id, ..)
            | Prop(id, ..) => *id,
        }
    }

    pub fn is_exit(&self) -> bool {
        matches!(
            self,
            Bytecode::Ret(..) | Bytecode::Abort(..) | Bytecode::Call(_, _, Operation::Stop, _, _)
        )
    }

    pub fn is_return(&self) -> bool {
        matches!(self, Bytecode::Ret(..))
    }

    pub fn is_unconditional_branch(&self) -> bool {
        matches!(
            self,
            Bytecode::Ret(..)
                | Bytecode::Jump(..)
                | Bytecode::Abort(..)
                | Bytecode::Call(_, _, Operation::Stop, _, _)
        )
    }

    pub fn is_conditional_branch(&self) -> bool {
        matches!(
            self,
            Bytecode::Branch(..) | Bytecode::Call(_, _, _, _, Some(_))
        )
    }

    pub fn is_branch(&self) -> bool {
        self.is_conditional_branch() || self.is_unconditional_branch()
    }

    /// Return the destination(s) if self is a branch/jump instruction
    pub fn branch_dests(&self) -> Vec<Label> {
        match self {
            Bytecode::Branch(_, then_label, else_label, _) => vec![*then_label, *else_label],
            Bytecode::Jump(_, label) | Bytecode::Call(_, _, _, _, Some(AbortAction(label, _))) => {
                vec![*label]
            }
            _ => vec![],
        }
    }

    /// Returns a mapping from labels to code offsets.
    pub fn label_offsets(code: &[Bytecode]) -> BTreeMap<Label, CodeOffset> {
        let mut res = BTreeMap::new();
        for (offs, bc) in code.iter().enumerate() {
            if let Bytecode::Label(_, label) = bc {
                assert!(res.insert(*label, offs as CodeOffset).is_none());
            }
        }
        res
    }

    /// Return the successor offsets of this instruction. In addition to the code, a map
    /// of label to code offset need to be passed in.
    pub fn get_successors(
        pc: CodeOffset,
        code: &[Bytecode],
        label_offsets: &BTreeMap<Label, CodeOffset>,
    ) -> Vec<CodeOffset> {
        let bytecode = &code[pc as usize];
        let mut v = vec![];
        if !bytecode.is_branch() {
            // Fall through situation, just return the next pc.
            v.push(pc + 1);
        } else {
            for label in bytecode.branch_dests() {
                v.push(*label_offsets.get(&label).expect("label defined"));
            }
            if matches!(bytecode, Bytecode::Call(_, _, _, _, Some(_))) {
                // Falls through.
                v.push(pc + 1);
            }
        }
        // always give successors in ascending order
        if v.len() > 1 && v[0] > v[1] {
            v.swap(0, 1);
        }
        v
    }

    /// Returns the code offsets at which the code exits(aborts or returns).
    pub fn get_exits(code: &[Bytecode]) -> Vec<CodeOffset> {
        code.iter()
            .enumerate()
            .filter(|(_, bytecode)| bytecode.is_exit())
            .map(|(idx, _)| idx as CodeOffset)
            .collect()
    }

    /// Remaps variables in the instruction.
    pub fn remap_all_vars<F>(self, func_target: &FunctionTarget<'_>, f: &mut F) -> Self
    where
        F: FnMut(TempIndex) -> TempIndex,
    {
        self.remap_vars_internal(func_target, &mut |_, idx| f(idx))
    }

    /// Remaps variables in source position in the instruction.
    pub fn remap_src_vars<F>(self, func_target: &FunctionTarget<'_>, f: &mut F) -> Self
    where
        F: FnMut(TempIndex) -> TempIndex,
    {
        self.remap_vars_internal(func_target, &mut |is_src, idx| {
            if is_src {
                f(idx)
            } else {
                idx
            }
        })
    }

    fn remap_vars_internal<F>(self, func_target: &FunctionTarget<'_>, f: &mut F) -> Self
    where
        F: FnMut(bool, TempIndex) -> TempIndex,
    {
        use BorrowNode::*;
        use Bytecode::*;
        use Operation::*;
        let map = |is_src: bool, f: &mut F, v: Vec<TempIndex>| -> Vec<TempIndex> {
            v.into_iter().map(|i| f(is_src, i)).collect()
        };
        let map_abort = |f: &mut F, aa: Option<AbortAction>| {
            aa.map(|AbortAction(l, code)| AbortAction(l, f(false, code)))
        };
        let map_node = |f: &mut F, node: BorrowNode| match node {
            LocalRoot(tmp) => LocalRoot(f(true, tmp)),
            Reference(tmp) => Reference(f(true, tmp)),
            _ => node,
        };
        match self {
            Load(attr, dst, cons) => Load(attr, f(false, dst), cons),
            Assign(attr, dest, src, kind) => Assign(attr, f(false, dest), f(true, src), kind),
            Call(attr, _, WriteBack(node, edge), srcs, aa) => Call(
                attr,
                vec![],
                WriteBack(map_node(f, node), edge),
                map(true, f, srcs),
                map_abort(f, aa),
            ),
            Call(attr, dests, IsParent(node, edge), srcs, aa) => Call(
                attr,
                map(false, f, dests),
                IsParent(map_node(f, node), edge),
                map(true, f, srcs),
                map_abort(f, aa),
            ),
            Call(attr, dests, op, srcs, aa) => Call(
                attr,
                map(false, f, dests),
                op,
                map(true, f, srcs),
                map_abort(f, aa),
            ),
            Ret(attr, rets) => Ret(attr, map(true, f, rets)),
            Branch(attr, if_label, else_label, cond) => {
                Branch(attr, if_label, else_label, f(true, cond))
            }
            Abort(attr, cond) => Abort(attr, f(true, cond)),
            Prop(attr, kind, exp) => {
                let new_exp = Bytecode::remap_exp(func_target, &mut |idx| f(true, idx), exp);
                Prop(attr, kind, new_exp)
            }
            _ => self,
        }
    }

    fn remap_exp<F>(func_target: &FunctionTarget<'_>, f: &mut F, exp: Exp) -> Exp
    where
        F: FnMut(TempIndex) -> TempIndex,
    {
        let mut replacer = |node_id: NodeId, target: RewriteTarget| {
            if let RewriteTarget::Temporary(idx) = target {
                Some(ExpData::Temporary(node_id, f(idx)).into_exp())
            } else {
                None
            }
        };
        ExpRewriter::new(func_target.global_env(), &mut replacer).rewrite_exp(exp)
    }

    /// Return the temporaries this instruction modifies and how the temporaries are modified.
    ///
    /// For a temporary with TempIndex $t, if $t is modified by the instruction and
    /// 1) $t is a value or an immutable reference, it will show up in the first Vec
    /// 2) $t is a mutable reference and only its value is modified, not the reference itself,
    ///    it will show up in the second Vec as ($t, false).
    /// 3) $t is a mutable reference and the reference itself is modified (i.e., the location and
    ///    path it is pointing to), it will show up in the second Vec as ($t, true).
    pub fn modifies(
        &self,
        func_target: &FunctionTarget<'_>,
    ) -> (Vec<TempIndex>, Vec<(TempIndex, bool)>) {
        use BorrowNode::*;
        use Bytecode::*;
        use Operation::*;
        let add_abort = |mut res: Vec<TempIndex>, aa: &Option<AbortAction>| {
            if let Some(AbortAction(_, dest)) = aa {
                res.push(*dest)
            }
            res
        };

        match self {
            Assign(_, dest, _, _) => {
                if func_target.get_local_type(*dest).is_mutable_reference() {
                    // reference assignment completely distorts the reference (value + pointer)
                    (vec![], vec![(*dest, true)])
                } else {
                    // value assignment
                    (vec![*dest], vec![])
                }
            }
            Load(_, dest, _) => {
                // constants can only be values, hence no modifications on the reference
                (vec![*dest], vec![])
            }
            Call(_, _, Operation::WriteBack(LocalRoot(dest), ..), _, aa) => {
                // write-back to a local variable distorts the value
                (add_abort(vec![*dest], aa), vec![])
            }
            Call(_, _, Operation::WriteBack(Reference(dest), ..), _, aa) => {
                // write-back to a reference only distorts the value, but not the pointer itself
                (add_abort(vec![], aa), vec![(*dest, false)])
            }
            Call(_, _, Operation::WriteRef, srcs, aa) => {
                // write-ref only distorts the value of the reference, but not the pointer itself
                (add_abort(vec![], aa), vec![(srcs[0], false)])
            }
            Call(_, dests, Function(..), srcs, aa) => {
                let mut val_targets = vec![];
                let mut mut_targets = vec![];
                for src in srcs {
                    if func_target.get_local_type(*src).is_mutable_reference() {
                        // values in mutable references can be distorted, but pointer stays the same
                        mut_targets.push((*src, false));
                    }
                }
                for dest in dests {
                    if func_target.get_local_type(*dest).is_mutable_reference() {
                        // similar to reference assignment
                        mut_targets.push((*dest, true));
                    } else {
                        // similar to value assignment
                        val_targets.push(*dest);
                    }
                }
                (add_abort(val_targets, aa), mut_targets)
            }
            // *** Double-check that this is in Wolfgang's code
            Call(_, dests, _, _, aa) => {
                let mut val_targets = vec![];
                let mut mut_targets = vec![];
                for dest in dests {
                    if func_target.get_local_type(*dest).is_mutable_reference() {
                        // similar to reference assignment
                        mut_targets.push((*dest, true));
                    } else {
                        // similar to value assignment
                        val_targets.push(*dest);
                    }
                }
                (add_abort(val_targets, aa), mut_targets)
            }
            _ => (vec![], vec![]),
        }
    }
}

// =================================================================================================
// Formatting

impl Bytecode {
    /// Creates a format object for a bytecode in context of a function target.
    pub fn display<'env>(
        &'env self,
        func_target: &'env FunctionTarget<'env>,
        label_offsets: &'env BTreeMap<Label, CodeOffset>,
    ) -> BytecodeDisplay<'env> {
        BytecodeDisplay {
            bytecode: self,
            func_target,
            label_offsets,
        }
    }
}

/// A display object for a bytecode.
pub struct BytecodeDisplay<'env> {
    bytecode: &'env Bytecode,
    func_target: &'env FunctionTarget<'env>,
    label_offsets: &'env BTreeMap<Label, CodeOffset>,
}

impl<'env> fmt::Display for BytecodeDisplay<'env> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use Bytecode::*;
        match &self.bytecode {
            Assign(_, dst, src, AssignKind::Copy) => {
                write!(f, "{} := copy({})", self.lstr(*dst), self.lstr(*src))?
            }
            Assign(_, dst, src, AssignKind::Move) => {
                write!(f, "{} := move({})", self.lstr(*dst), self.lstr(*src))?
            }
            Assign(_, dst, src, AssignKind::Store) => {
                write!(f, "{} := {}", self.lstr(*dst), self.lstr(*src))?
            }
            Call(_, dsts, oper, args, aa) => {
                if !dsts.is_empty() {
                    self.fmt_locals(f, dsts, false)?;
                    write!(f, " := ")?;
                }
                write!(f, "{}", oper.display(self.func_target))?;
                self.fmt_locals(f, args, true)?;
                if let Some(AbortAction(label, code)) = aa {
                    write!(
                        f,
                        " on_abort goto {} with {}",
                        self.label_str(*label),
                        self.lstr(*code)
                    )?;
                }
            }
            Ret(_, srcs) => {
                write!(f, "return ")?;
                self.fmt_locals(f, srcs, false)?;
            }
            Load(_, dst, cons) => {
                write!(f, "{} := {}", self.lstr(*dst), cons)?;
            }
            Branch(_, then_label, else_label, src) => {
                write!(
                    f,
                    "if ({}) goto {} else goto {}",
                    self.lstr(*src),
                    self.label_str(*then_label),
                    self.label_str(*else_label),
                )?;
            }
            Jump(_, label) => {
                write!(f, "goto {}", self.label_str(*label))?;
            }
            Label(_, label) => {
                write!(f, "label L{}", label.as_usize())?;
            }
            Abort(_, src) => {
                write!(f, "abort({})", self.lstr(*src))?;
            }
            Nop(_) => {
                write!(f, "nop")?;
            }
            SaveMem(_, label, qid) => {
                let env = self.func_target.global_env();
                write!(f, "@{} := save_mem({})", label.as_usize(), env.display(qid))?;
            }
            SaveSpecVar(_, label, qid) => {
                let env = self.func_target.global_env();
                let module_env = env.get_module(qid.module_id);
                let spec_var = module_env.get_spec_var(qid.id);
                write!(
                    f,
                    "@{} := save_spec_var({}::{})",
                    label.as_usize(),
                    module_env.get_name().display(env.symbol_pool()),
                    spec_var.name.display(env.symbol_pool())
                )?;
            }
            Prop(_, kind, exp) => {
                let exp_display = exp.display(self.func_target.func_env.module_env.env);
                match kind {
                    PropKind::Assume => write!(f, "assume {}", exp_display)?,
                    PropKind::Assert => write!(f, "assert {}", exp_display)?,
                    PropKind::Modifies => write!(f, "modifies {}", exp_display)?,
                }
            }
        }
        Ok(())
    }
}

impl<'env> BytecodeDisplay<'env> {
    fn fmt_locals(
        &self,
        f: &mut Formatter<'_>,
        locals: &[TempIndex],
        always_brace: bool,
    ) -> fmt::Result {
        if !always_brace && locals.len() == 1 {
            write!(f, "{}", self.lstr(locals[0]))?
        } else {
            write!(f, "(")?;
            for (i, l) in locals.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", self.lstr(*l))?;
            }
            write!(f, ")")?;
        }
        Ok(())
    }

    fn lstr(&self, idx: TempIndex) -> String {
        format!("$t{}", idx)
    }

    fn label_str(&self, label: Label) -> String {
        self.label_offsets
            .get(&label)
            .map(|offs| offs.to_string())
            .unwrap_or_else(|| format!("L{}", label.as_usize()))
    }
}

impl Operation {
    /// Creates a format object for a bytecode in context of a function target.
    pub fn display<'env>(
        &'env self,
        func_target: &'env FunctionTarget<'env>,
    ) -> OperationDisplay<'env> {
        OperationDisplay {
            oper: self,
            func_target,
        }
    }
}

/// A display object for an operation.
pub struct OperationDisplay<'env> {
    oper: &'env Operation,
    func_target: &'env FunctionTarget<'env>,
}

impl<'env> fmt::Display for OperationDisplay<'env> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use Operation::*;
        match self.oper {
            // User function
            Function(mid, fid, targs)
            | OpaqueCallBegin(mid, fid, targs)
            | OpaqueCallEnd(mid, fid, targs) => {
                let func_env = self
                    .func_target
                    .global_env()
                    .get_module(*mid)
                    .into_function(*fid);
                write!(
                    f,
                    "{}",
                    match self.oper {
                        OpaqueCallBegin(_, _, _) => "opaque begin: ",
                        OpaqueCallEnd(_, _, _) => "opaque end: ",
                        _ => "",
                    }
                )?;
                write!(
                    f,
                    "{}::{}",
                    func_env
                        .module_env
                        .get_name()
                        .display(func_env.symbol_pool()),
                    func_env.get_name().display(func_env.symbol_pool()),
                )?;
                self.fmt_type_args(f, targs)?;
            }

            // Pack/Unpack
            Pack(mid, sid, targs) => {
                write!(f, "pack {}", self.struct_str(*mid, *sid, targs))?;
            }
            Unpack(mid, sid, targs) => {
                write!(f, "unpack {}", self.struct_str(*mid, *sid, targs))?;
            }

            // Borrow
            BorrowLoc => {
                write!(f, "borrow_local")?;
            }
            BorrowField(mid, sid, targs, offset) => {
                write!(f, "borrow_field<{}>", self.struct_str(*mid, *sid, targs))?;
                let struct_env = self
                    .func_target
                    .global_env()
                    .get_module(*mid)
                    .into_struct(*sid);
                let field_env = struct_env.get_field_by_offset(*offset);
                write!(
                    f,
                    ".{}",
                    field_env.get_name().display(struct_env.symbol_pool())
                )?;
            }
            BorrowGlobal(mid, sid, targs) => {
                write!(f, "borrow_global<{}>", self.struct_str(*mid, *sid, targs))?;
            }
            GetField(mid, sid, targs, offset) => {
                write!(f, "get_field<{}>", self.struct_str(*mid, *sid, targs))?;
                let struct_env = self
                    .func_target
                    .global_env()
                    .get_module(*mid)
                    .into_struct(*sid);
                let field_env = struct_env.get_field_by_offset(*offset);
                write!(
                    f,
                    ".{}",
                    field_env.get_name().display(struct_env.symbol_pool())
                )?;
            }
            GetGlobal(mid, sid, targs) => {
                write!(f, "get_global<{}>", self.struct_str(*mid, *sid, targs))?;
            }

            // Resources
            MoveTo(mid, sid, targs) => {
                write!(f, "move_to<{}>", self.struct_str(*mid, *sid, targs))?;
            }
            MoveFrom(mid, sid, targs) => {
                write!(f, "move_from<{}>", self.struct_str(*mid, *sid, targs))?;
            }
            Exists(mid, sid, targs) => {
                write!(f, "exists<{}>", self.struct_str(*mid, *sid, targs))?;
            }

            // Builtins
            Destroy => {
                write!(f, "destroy")?;
            }
            ReadRef => {
                write!(f, "read_ref")?;
            }
            WriteRef => {
                write!(f, "write_ref")?;
            }
            FreezeRef => {
                write!(f, "freeze_ref")?;
            }

            // Memory model
            UnpackRef => {
                write!(f, "unpack_ref")?;
            }
            PackRef => {
                write!(f, "pack_ref")?;
            }
            PackRefDeep => {
                write!(f, "pack_ref_deep")?;
            }
            UnpackRefDeep => {
                write!(f, "unpack_ref_deep")?;
            }
            WriteBack(node, edge) => write!(
                f,
                "write_back[{}{}]",
                node.display(self.func_target),
                edge.display(self.func_target.global_env())
            )?,
            IsParent(node, edge) => write!(
                f,
                "is_parent[{}{}]",
                node.display(self.func_target),
                edge.display(self.func_target.global_env())
            )?,
            GlobalAddress => {
                write!(f, "global_address")?;
            }

            Havoc(kind) => {
                write!(
                    f,
                    "havoc[{}]",
                    match kind {
                        HavocKind::Value => "val",
                        HavocKind::MutationValue => "mut",
                        HavocKind::MutationAll => "mut_all",
                    }
                )?;
            }
            Stop => {
                write!(f, "stop")?;
            }
            // Unary
            CastU8 => write!(f, "(u8)")?,
            CastU64 => write!(f, "(u64)")?,
            CastU128 => write!(f, "(u128)")?,
            Not => write!(f, "!")?,

            // Binary
            Add => write!(f, "+")?,
            Sub => write!(f, "-")?,
            Mul => write!(f, "*")?,
            Div => write!(f, "/")?,
            Mod => write!(f, "%")?,
            BitOr => write!(f, "|")?,
            BitAnd => write!(f, "&")?,
            Xor => write!(f, "^")?,
            Shl => write!(f, "<<")?,
            Shr => write!(f, "<<")?,
            Lt => write!(f, "<")?,
            Gt => write!(f, ">")?,
            Le => write!(f, "<=")?,
            Ge => write!(f, ">=")?,
            Or => write!(f, "||")?,
            And => write!(f, "&&")?,
            Eq => write!(f, "==")?,
            Neq => write!(f, "!=")?,

            // Debugging
            TraceLocal(l) => {
                let name = self.func_target.get_local_name(*l);
                write!(
                    f,
                    "trace_local[{}]",
                    name.display(self.func_target.symbol_pool())
                )?
            }
            TraceAbort => write!(f, "trace_abort")?,
            TraceReturn(r) => write!(f, "trace_return[{}]", r)?,
            TraceExp(node_id) => {
                let loc = self.func_target.global_env().get_node_loc(*node_id);
                write!(
                    f,
                    "trace_exp[{}]",
                    loc.display(self.func_target.global_env())
                )?
            }
            EmitEvent => write!(f, "emit_event")?,
            EventStoreDiverge => write!(f, "event_store_diverge")?,
        }
        Ok(())
    }
}

impl<'env> OperationDisplay<'env> {
    fn fmt_type_args(&self, f: &mut Formatter<'_>, targs: &[Type]) -> fmt::Result {
        if !targs.is_empty() {
            let tctx = TypeDisplayContext::WithEnv {
                env: self.func_target.global_env(),
                type_param_names: None,
            };
            write!(f, "<")?;
            for (i, ty) in targs.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", ty.display(&tctx))?;
            }
            write!(f, ">")?;
        }
        Ok(())
    }

    fn struct_str(&self, mid: ModuleId, sid: StructId, targs: &[Type]) -> String {
        let ty = Type::Struct(mid, sid, targs.to_vec());
        let tctx = TypeDisplayContext::WithEnv {
            env: self.func_target.global_env(),
            type_param_names: None,
        };
        format!("{}", ty.display(&tctx))
    }
}

impl fmt::Display for Constant {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use Constant::*;
        match self {
            Bool(x) => write!(f, "{}", x)?,
            U8(x) => write!(f, "{}", x)?,
            U64(x) => write!(f, "{}", x)?,
            U128(x) => write!(f, "{}", x)?,
            Address(x) => write!(f, "0x{}", x.to_str_radix(16))?,
            ByteArray(x) => write!(f, "{:?}", x)?,
        }
        Ok(())
    }
}

/// A display object for a borrow node.
pub struct BorrowNodeDisplay<'env> {
    node: &'env BorrowNode,
    func_target: &'env FunctionTarget<'env>,
}

impl BorrowNode {
    /// Creates a format object for a borrow node in context of a function target.
    pub fn display<'env>(
        &'env self,
        func_target: &'env FunctionTarget<'env>,
    ) -> BorrowNodeDisplay<'env> {
        BorrowNodeDisplay {
            node: self,
            func_target,
        }
    }
}

impl<'env> fmt::Display for BorrowNodeDisplay<'env> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use BorrowNode::*;
        match self.node {
            GlobalRoot(s) => {
                let ty = Type::Struct(s.module_id, s.id, s.inst.to_owned());
                let tctx = TypeDisplayContext::WithEnv {
                    env: self.func_target.global_env(),
                    type_param_names: None,
                };
                write!(f, "{}", ty.display(&tctx))?;
            }
            LocalRoot(idx) => {
                write!(f, "LocalRoot($t{})", idx)?;
            }
            Reference(idx) => {
                write!(f, "Reference($t{})", idx)?;
            }
            ReturnPlaceholder(idx) => {
                write!(f, "Return({})", idx)?;
            }
        }
        Ok(())
    }
}
impl BorrowEdge {
    pub fn display<'a>(&'a self, env: &'a GlobalEnv) -> BorrowEdgeDisplay<'a> {
        BorrowEdgeDisplay { env, edge: self }
    }
}

pub struct BorrowEdgeDisplay<'a> {
    env: &'a GlobalEnv,
    edge: &'a BorrowEdge,
}

impl<'a> std::fmt::Display for BorrowEdgeDisplay<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use BorrowEdge::*;
        match self.edge {
            Field(qid, field) => {
                let struct_env = self.env.get_struct(qid.to_qualified_id());
                let field_env = struct_env.get_field_by_offset(*field);
                write!(
                    f,
                    ".{}",
                    field_env.get_name().display(self.env.symbol_pool())
                )
            }
            Index => write!(f, "[]"),
            Direct => write!(f, "@"),
            Hyper(es) => {
                write!(
                    f,
                    "{}",
                    es.iter()
                        .map(|e| format!("{}", e.display(self.env)))
                        .join("/")
                )
            }
        }
    }
}
