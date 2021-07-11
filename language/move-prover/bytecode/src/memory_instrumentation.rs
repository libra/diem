// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    borrow_analysis::BorrowAnnotation,
    function_data_builder::FunctionDataBuilder,
    function_target::FunctionData,
    function_target_pipeline::{FunctionTargetProcessor, FunctionTargetsHolder},
    stackless_bytecode::{
        BorrowEdge, BorrowNode,
        Bytecode::{self, *},
        Operation,
    },
};
use move_binary_format::file_format::CodeOffset;
use move_model::{
    ast::ConditionKind,
    exp_generator::ExpGenerator,
    model::{FunctionEnv, StructEnv},
    ty::{PrimitiveType, Type, BOOL_TYPE},
};

pub struct MemoryInstrumentationProcessor {}

impl MemoryInstrumentationProcessor {
    pub fn new() -> Box<Self> {
        Box::new(MemoryInstrumentationProcessor {})
    }
}

impl FunctionTargetProcessor for MemoryInstrumentationProcessor {
    fn process(
        &self,
        _targets: &mut FunctionTargetsHolder,
        func_env: &FunctionEnv<'_>,
        mut data: FunctionData,
    ) -> FunctionData {
        if func_env.is_native() {
            return data;
        }
        let borrow_annotation = data
            .annotations
            .remove::<BorrowAnnotation>()
            .expect("borrow annotation");
        let mut builder = FunctionDataBuilder::new(func_env, data);
        let code = std::mem::take(&mut builder.data.code);
        let mut instrumenter = Instrumenter::new(builder, &borrow_annotation);
        for (code_offset, bytecode) in code.into_iter().enumerate() {
            instrumenter.instrument(code_offset as CodeOffset, bytecode);
        }
        instrumenter.builder.data
    }

    fn name(&self) -> String {
        "memory_instr".to_string()
    }
}

struct Instrumenter<'a> {
    builder: FunctionDataBuilder<'a>,
    borrow_annotation: &'a BorrowAnnotation,
}

impl<'a> Instrumenter<'a> {
    fn new(builder: FunctionDataBuilder<'a>, borrow_annotation: &'a BorrowAnnotation) -> Self {
        Self {
            builder,
            borrow_annotation,
        }
    }

    fn instrument(&mut self, code_offset: CodeOffset, bytecode: Bytecode) {
        if bytecode.is_branch()
            || matches!(bytecode, Bytecode::Call(_, _, Operation::Destroy, _, _))
        {
            // Add memory instrumentation before instruction.
            self.memory_instrumentation(code_offset, &bytecode);
            self.builder.emit(bytecode);
        } else {
            self.builder.emit(bytecode.clone());
            self.memory_instrumentation(code_offset, &bytecode);
        }
    }

    /// Determines whether the type needs a pack ref.
    fn is_pack_ref_ty(&self, ty: &Type) -> bool {
        use Type::*;
        let env = self.builder.global_env();
        match ty.skip_reference() {
            Struct(mid, sid, inst) => {
                self.is_pack_ref_struct(&env.get_struct_qid(mid.qualified(*sid)))
                    || inst.iter().any(|t| self.is_pack_ref_ty(t))
            }
            Vector(et) => self.is_pack_ref_ty(et.as_ref()),
            _ => false,
        }
    }

    /// Determines whether the struct needs a pack ref.
    fn is_pack_ref_struct(&self, struct_env: &StructEnv<'_>) -> bool {
        struct_env.get_spec().any(|c| matches!(c.kind, ConditionKind::Invariant))
        // If any of the fields has it, it inherits to the struct.
        ||  struct_env
            .get_fields()
            .any(|fe| self.is_pack_ref_ty(&fe.get_type()))
    }

    fn memory_instrumentation(&mut self, code_offset: CodeOffset, bytecode: &Bytecode) {
        let borrow_annotation_at = self
            .borrow_annotation
            .get_borrow_info_at(code_offset)
            .unwrap();
        let before = &borrow_annotation_at.before;
        let after = &borrow_annotation_at.after;

        // Generate UnpackRef from Borrow instructions.
        if let Call(attr_id, dests, op, _, _) = bytecode {
            use Operation::*;
            match op {
                BorrowLoc | BorrowField(..) | BorrowGlobal(..) => {
                    let ty = &self
                        .builder
                        .get_target()
                        .get_local_type(dests[0])
                        .to_owned();
                    let node = BorrowNode::Reference(dests[0]);
                    if self.is_pack_ref_ty(ty) && after.is_in_use(&node) {
                        self.builder.set_loc_from_attr(*attr_id);
                        self.builder.emit_with(|id| {
                            Bytecode::Call(id, vec![], Operation::UnpackRef, vec![dests[0]], None)
                        });
                    }
                }
                _ => {}
            }
        }

        // Generate PackRef for nodes which go out of scope, as well as WriteBack.
        let attr_id = bytecode.get_attr_id();
        for node in before.dying_nodes(after) {
            if let BorrowNode::Reference(idx) = &node {
                // Generate write_back for this reference.
                let incoming = before.get_incoming(&node);
                let is_conditional = incoming.len() > 1;
                for (parent, edge) in before.get_incoming(&node) {
                    self.builder.set_loc_from_attr(attr_id);

                    // instrument IsParent checks if there may be multiple parents
                    let skip_label_opt = if is_conditional {
                        let temp = self.builder.new_temp(BOOL_TYPE.clone());
                        self.builder.emit_with(|id| {
                            Bytecode::Call(
                                id,
                                vec![temp],
                                Operation::IsParent(parent.clone(), edge.clone()),
                                vec![*idx],
                                None,
                            )
                        });
                        let update_label = self.builder.new_label();
                        let skip_label = self.builder.new_label();
                        self.builder
                            .emit_with(|id| Bytecode::Branch(id, update_label, skip_label, temp));
                        self.builder
                            .emit_with(|id| Bytecode::Label(id, update_label));
                        Some(skip_label)
                    } else {
                        None
                    };

                    // handle the write-back
                    let (should_pack_ref, final_ref_idx) = match &parent {
                        BorrowNode::LocalRoot(root_idx) => {
                            let ref_ty = Type::Reference(
                                true,
                                Box::new(self.builder.get_local_type(*root_idx)),
                            );
                            let should_pack_ref =
                                self.is_pack_ref_ty(&ref_ty) && !after.is_in_use(&parent);

                            let from_idx = match edge {
                                BorrowEdge::Direct => *idx,
                                BorrowEdge::Hyper(mut edges) => {
                                    assert!(matches!(edges.remove(0), BorrowEdge::Direct));
                                    // generate a shadow ref just for the purpose of write-back
                                    let temp = self.builder.new_temp(ref_ty);
                                    self.builder.emit_with(|id| {
                                        Bytecode::Call(
                                            id,
                                            vec![temp],
                                            Operation::BorrowLoc,
                                            vec![*root_idx],
                                            None,
                                        )
                                    });
                                    // write-back to the shadow ref
                                    let shadow_parent = BorrowNode::Reference(temp);
                                    self.builder.emit_with(|id| {
                                        Bytecode::Call(
                                            id,
                                            vec![],
                                            Operation::WriteBack(
                                                shadow_parent,
                                                BorrowEdge::Hyper(edges),
                                            ),
                                            vec![*idx],
                                            None,
                                        )
                                    });
                                    temp
                                }
                                _ => unreachable!(
                                    "only direct or hyper edges allowed for local root"
                                ),
                            };

                            self.builder.emit_with(|id| {
                                Bytecode::Call(
                                    id,
                                    vec![],
                                    Operation::WriteBack(parent, BorrowEdge::Direct),
                                    vec![from_idx],
                                    None,
                                )
                            });
                            (should_pack_ref, from_idx)
                        }
                        BorrowNode::GlobalRoot(inst_id) => {
                            let ref_ty = Type::Reference(true, Box::new(inst_id.to_type()));
                            let should_pack_ref =
                                self.is_pack_ref_ty(&ref_ty) && !after.is_in_use(&parent);

                            let from_idx = match edge {
                                BorrowEdge::Direct => *idx,
                                BorrowEdge::Hyper(mut edges) => {
                                    assert!(matches!(edges.remove(0), BorrowEdge::Direct));
                                    // generate a shadow ref just for the purpose of write-back
                                    let addr = self
                                        .builder
                                        .new_temp(Type::Primitive(PrimitiveType::Address));
                                    self.builder.emit_with(|id| {
                                        Bytecode::Call(
                                            id,
                                            vec![addr],
                                            Operation::GlobalAddress,
                                            vec![*idx],
                                            None,
                                        )
                                    });
                                    let temp = self.builder.new_temp(ref_ty);
                                    self.builder.emit_with(|id| {
                                        Bytecode::Call(
                                            id,
                                            vec![temp],
                                            Operation::BorrowGlobal(
                                                inst_id.module_id,
                                                inst_id.id,
                                                inst_id.inst.to_vec(),
                                            ),
                                            vec![addr],
                                            None,
                                        )
                                    });
                                    // write-back to the shadow ref
                                    let shadow_parent = BorrowNode::Reference(temp);
                                    self.builder.emit_with(|id| {
                                        Bytecode::Call(
                                            id,
                                            vec![],
                                            Operation::WriteBack(
                                                shadow_parent,
                                                BorrowEdge::Hyper(edges),
                                            ),
                                            vec![*idx],
                                            None,
                                        )
                                    });
                                    temp
                                }
                                _ => unreachable!(
                                    "only direct or hyper edges allowed for global root"
                                ),
                            };

                            self.builder.emit_with(|id| {
                                Bytecode::Call(
                                    id,
                                    vec![],
                                    Operation::WriteBack(parent, BorrowEdge::Direct),
                                    vec![from_idx],
                                    None,
                                )
                            });
                            (should_pack_ref, from_idx)
                        }
                        BorrowNode::Reference(ref_idx) => {
                            let ref_idx = *ref_idx;
                            self.builder.emit_with(|id| {
                                Bytecode::Call(
                                    id,
                                    vec![],
                                    Operation::WriteBack(parent, edge),
                                    vec![*idx],
                                    None,
                                )
                            });
                            (false, ref_idx)
                        }
                        BorrowNode::ReturnPlaceholder(..) => {
                            unreachable!("unexpected transient borrow node")
                        }
                    };

                    // On write-back to a local or global root, "pack" the reference,
                    // i.e. validate all its invariants.
                    if should_pack_ref {
                        self.builder.emit_with(|id| {
                            Bytecode::Call(
                                id,
                                vec![],
                                Operation::PackRefDeep,
                                vec![final_ref_idx],
                                None,
                            )
                        });
                    }

                    // add the skip label
                    if let Some(label) = skip_label_opt {
                        self.builder.emit_with(|id| Bytecode::Label(id, label));
                    }
                }
            }
        }
    }
}
