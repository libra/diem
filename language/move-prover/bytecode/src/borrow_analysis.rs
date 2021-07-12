// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Data flow analysis computing borrow information for preparation of memory_instrumentation.

use crate::{
    dataflow_analysis::{DataflowAnalysis, TransferFunctions},
    dataflow_domains::{AbstractDomain, JoinResult, MapDomain, SetDomain},
    function_target::{FunctionData, FunctionTarget},
    function_target_pipeline::{FunctionTargetProcessor, FunctionTargetsHolder, FunctionVariant},
    livevar_analysis::LiveVarAnnotation,
    stackless_bytecode::{AssignKind, BorrowEdge, BorrowNode, Bytecode, Operation},
    stackless_control_flow_graph::StacklessControlFlowGraph,
};
use itertools::Itertools;
use move_binary_format::file_format::CodeOffset;
use move_model::{
    ast::TempIndex,
    model::{FunctionEnv, GlobalEnv, QualifiedInstId},
    native::VECTOR_BORROW_MUT,
};
use std::{
    borrow::BorrowMut,
    collections::{BTreeMap, BTreeSet},
    fmt,
};

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Default)]
pub struct BorrowInfo {
    /// Contains the nodes which are alive. This excludes nodes which are alive because
    /// other nodes which are alive borrow from them.
    live_nodes: SetDomain<BorrowNode>,

    /// Contains the nodes which have been moved via a move instruction.
    moved_nodes: SetDomain<BorrowNode>,

    /// Forward borrow information.
    borrowed_by: MapDomain<BorrowNode, SetDomain<(BorrowNode, BorrowEdge)>>,

    /// Backward borrow information. This field is not used during analysis, but computed once
    /// analysis is done.
    borrows_from: MapDomain<BorrowNode, SetDomain<(BorrowNode, BorrowEdge)>>,
}

impl BorrowInfo {
    /// Gets the children of this node.
    pub fn get_children(&self, node: &BorrowNode) -> Vec<&BorrowNode> {
        self.borrowed_by
            .get(node)
            .map(|s| s.iter().map(|(n, _)| n).collect_vec())
            .unwrap_or_else(Vec::new)
    }

    /// Gets the parents of this node.
    pub fn get_parents(&self, node: &BorrowNode) -> Vec<&BorrowNode> {
        self.borrows_from
            .get(node)
            .map(|s| s.iter().map(|(n, _)| n).collect_vec())
            .unwrap_or_else(Vec::new)
    }

    /// Gets incoming edges (together with sources) of this node.
    pub fn get_incoming(&self, node: &BorrowNode) -> SetDomain<(BorrowNode, BorrowEdge)> {
        let mut incoming = SetDomain::default();
        if let Some(parents) = self.borrows_from.get(node) {
            for (parent, edge) in parents.iter() {
                match parent {
                    BorrowNode::GlobalRoot(_) | BorrowNode::LocalRoot(_) => {
                        incoming.insert((parent.clone(), edge.clone()));
                    }
                    BorrowNode::Reference(_) => {
                        if self.live_nodes.contains(parent) {
                            incoming.insert((parent.clone(), edge.clone()));
                        } else {
                            // concat the edges
                            for (root_node, path_root_to_parent) in self.get_incoming(parent) {
                                let combined_path = path_root_to_parent
                                    .flatten()
                                    .into_iter()
                                    .chain(edge.flatten().into_iter())
                                    .cloned()
                                    .collect();
                                incoming.insert((root_node, BorrowEdge::Hyper(combined_path)));
                            }
                        }
                    }
                    BorrowNode::ReturnPlaceholder(_) => {
                        unreachable!("unexpected transient borrow node")
                    }
                }
            }
        }
        incoming
    }

    /// Returns true if this node is conditional, that is, it borrows from multiple parents
    pub fn is_conditional(&self, node: &BorrowNode) -> bool {
        self.borrows_from
            .get(node)
            .map_or(false, |parents| parents.len() > 1)
    }

    /// Checks whether a node is in use. A node is used if it is in the live_nodes set
    /// or if it is borrowed by a node which is used.
    pub fn is_in_use(&self, node: &BorrowNode) -> bool {
        if self.live_nodes.contains(node) {
            true
        } else {
            self.get_children(node)
                .iter()
                .any(|child| self.is_in_use(child))
        }
    }

    /// Checks whether this is a moved node.
    pub fn is_moved(&self, node: &BorrowNode) -> bool {
        self.moved_nodes.contains(node)
    }

    /// Returns nodes which are dying from this to the next state. This includes those which
    /// are directly dying plus those from which they borrow. Returns nodes in child-first order.
    pub fn dying_nodes(&self, next: &BorrowInfo) -> Vec<BorrowNode> {
        let mut visited = BTreeSet::new();
        let mut result = vec![];
        for dying in self.live_nodes.difference(&next.live_nodes) {
            self.collect_live_ancestors(&mut visited, &mut result, dying);
        }
        result
    }

    /// Collects this node and ancestors that are still alive, inserting them in child-first order
    /// into the given vector.
    fn collect_live_ancestors(
        &self,
        visited: &mut BTreeSet<BorrowNode>,
        order: &mut Vec<BorrowNode>,
        node: &BorrowNode,
    ) {
        if visited.insert(node.clone()) {
            if self.live_nodes.contains(node) {
                order.push(node.clone());
            }
            for parent in self.get_parents(node) {
                self.collect_live_ancestors(visited, order, parent);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.live_nodes.is_empty()
            && self.moved_nodes.is_empty()
            && self.borrowed_by.is_empty()
            && self.borrows_from.is_empty()
    }

    pub fn borrow_info_str(&self, func_target: &FunctionTarget<'_>) -> String {
        let mut parts = vec![];
        let mut add = |name: &str, value: String| {
            if !value.is_empty() {
                parts.push(format!("{}: {}", name, value));
            }
        };
        add(
            "live_nodes",
            self.live_nodes
                .iter()
                .map(|node| format!("{}", node.display(func_target)))
                .join(", "),
        );
        add(
            "moved_nodes",
            self.moved_nodes
                .iter()
                .map(|node| format!("{}", node.display(func_target)))
                .join(", "),
        );
        let borrows_str =
            |(node1, borrows): (&BorrowNode, &SetDomain<(BorrowNode, BorrowEdge)>)| {
                format!(
                    "{} -> {{{}}}",
                    node1.display(func_target),
                    borrows
                        .iter()
                        .map(|(node2, edge)| format!(
                            "({}, {})",
                            edge.display(func_target.global_env()),
                            node2.display(func_target)
                        ))
                        .join(", ")
                )
            };
        add(
            "borrowed_by",
            self.borrowed_by.iter().map(borrows_str).join(", "),
        );
        add(
            "borrows_from",
            self.borrows_from.iter().map(borrows_str).join(", "),
        );
        parts.iter().join("\n")
    }

    fn add_node(&mut self, node: BorrowNode) {
        self.live_nodes.insert(node);
    }

    fn remove_node(&mut self, node: &BorrowNode) {
        self.live_nodes.remove(node);
    }

    fn add_edge(&mut self, parent: BorrowNode, child: BorrowNode, weight: BorrowEdge) -> bool {
        self.borrowed_by
            .entry(parent)
            .or_default()
            .insert((child, weight))
            .is_none()
    }

    fn consolidate(&mut self) {
        for (src, outgoing) in self.borrowed_by.iter() {
            for (dst, edge) in outgoing.iter() {
                self.borrows_from
                    .entry(dst.clone())
                    .or_default()
                    .insert((src.clone(), edge.clone()));
            }
        }
    }

    /// Collect those leaves which are returned and summarize them in a hyper edge.
    /// Each of those leaves has a path `in_mut -> ref1 .. -> refn -> out_mut`.
    /// We create a hyper edge `in_mut --summarize(ref1, .., refn)-> out_mut` for it.
    fn summarize(
        &mut self,
        target: &FunctionTarget<'_>,
        ret_info: &BorrowInfo,
        ret_values: &[TempIndex],
    ) {
        for (src, outgoing) in ret_info.borrows_from.iter() {
            if let BorrowNode::Reference(idx) = src {
                if let Some(pos) = ret_values.iter().position(|i| i == idx) {
                    // Construct hyper edges for this return value.
                    let leaf = BorrowNode::ReturnPlaceholder(pos);
                    self.construct_hyper_edges(&leaf, ret_info, vec![], outgoing)
                }
            }
        }
        for (ret_idx, ret_val) in ret_values.iter().enumerate() {
            let ty = target.get_return_type(ret_idx);
            if ty.is_mutable_reference() && *ret_val < target.get_parameter_count() {
                // Special case of a &mut parameter directly returned. We do not have this in
                // the borrow graph, so synthesize an edge.
                self.add_edge(
                    BorrowNode::Reference(*ret_val),
                    BorrowNode::ReturnPlaceholder(ret_idx),
                    BorrowEdge::Direct,
                );
            }
        }
    }

    fn construct_hyper_edges(
        &mut self,
        leaf: &BorrowNode,
        ret_info: &BorrowInfo,
        prefix: Vec<BorrowEdge>,
        outgoing: &SetDomain<(BorrowNode, BorrowEdge)>,
    ) {
        for (dest, edge) in outgoing.iter() {
            let mut path = prefix.to_owned();
            path.extend(edge.flatten().into_iter().cloned());
            if let Some(succs) = ret_info.borrows_from.get(dest) {
                self.construct_hyper_edges(leaf, ret_info, path, succs);
            } else {
                // Reached a leaf.
                let edge = if path.len() == 1 {
                    path.pop().unwrap()
                } else {
                    path.reverse();
                    BorrowEdge::Hyper(path)
                };
                self.borrowed_by
                    .entry(dest.clone())
                    .or_default()
                    .insert((leaf.clone(), edge));
            }
        }
    }

    /// Instantiates the summarized borrow graph of a function call in this graph.
    fn instantiate(
        &mut self,
        callee_target: &FunctionTarget<'_>,
        callee_summary: &BorrowInfo,
        ins: &[TempIndex],
        outs: &[TempIndex],
    ) {
        let get_in = |idx: usize| {
            assert!(
                idx < ins.len(),
                "inconsistent borrow information: undefined input"
            );
            ins[idx]
        };
        for (ret_idx, out) in outs.iter().enumerate() {
            if let Some(edges) = callee_summary
                .borrows_from
                .get(&BorrowNode::ReturnPlaceholder(ret_idx))
            {
                let out_node = BorrowNode::Reference(*out);
                self.add_node(out_node.clone());
                for (in_node, edge) in edges.iter() {
                    if let BorrowNode::Reference(in_idx) = in_node {
                        let actual_in_node = BorrowNode::Reference(get_in(*in_idx));
                        self.add_edge(actual_in_node.clone(), out_node.clone(), edge.to_owned());
                        self.moved_nodes.insert(actual_in_node);
                    }
                }
            } else {
                assert!(
                    !callee_target
                        .get_return_type(ret_idx)
                        .is_mutable_reference(),
                    "inconsistent borrow information: undefined output: {}\n{}",
                    callee_target.func_env.get_full_name_str(),
                    callee_summary.borrow_info_str(callee_target)
                )
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct BorrowInfoAtCodeOffset {
    pub before: BorrowInfo,
    pub after: BorrowInfo,
}

/// Borrow annotation computed by the borrow analysis processor.
#[derive(Clone, Default)]
pub struct BorrowAnnotation {
    summary: BorrowInfo,
    code_map: BTreeMap<CodeOffset, BorrowInfoAtCodeOffset>,
}

impl BorrowAnnotation {
    pub fn get_summary(&self) -> &BorrowInfo {
        &self.summary
    }
    pub fn get_borrow_info_at(&self, code_offset: CodeOffset) -> Option<&BorrowInfoAtCodeOffset> {
        self.code_map.get(&code_offset)
    }
}

/// Borrow analysis processor.
pub struct BorrowAnalysisProcessor {}

impl BorrowAnalysisProcessor {
    pub fn new() -> Box<Self> {
        Box::new(BorrowAnalysisProcessor {})
    }
}

impl FunctionTargetProcessor for BorrowAnalysisProcessor {
    fn process(
        &self,
        targets: &mut FunctionTargetsHolder,
        func_env: &FunctionEnv<'_>,
        mut data: FunctionData,
    ) -> FunctionData {
        let borrow_annotation = if func_env.is_native() {
            native_annotation(func_env)
        } else {
            let func_target = FunctionTarget::new(func_env, &data);
            let analyzer = BorrowAnalysis::new(&func_target, targets);
            analyzer.analyze(&data.code)
        };
        // Annotate function target with computed borrow data.
        data.annotations
            .borrow_mut()
            .set::<BorrowAnnotation>(borrow_annotation);
        data.annotations.borrow_mut().remove::<LiveVarAnnotation>();
        data
    }

    fn name(&self) -> String {
        "borrow_analysis".to_string()
    }

    fn dump_result(
        &self,
        f: &mut fmt::Formatter,
        env: &GlobalEnv,
        targets: &FunctionTargetsHolder,
    ) -> fmt::Result {
        writeln!(f, "\n\n==== borrow analysis summaries ====\n")?;
        for ref module in env.get_modules() {
            for ref fun in module.get_functions() {
                for (_, ref target) in targets.get_targets(fun) {
                    if let Some(an) = target.get_annotations().get::<BorrowAnnotation>() {
                        if !an.summary.is_empty() {
                            writeln!(
                                f,
                                "fun {}[{}]",
                                fun.get_full_name_str(),
                                target.data.variant
                            )?;
                            writeln!(f, "{}\n", an.summary.borrow_info_str(target))?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn native_annotation(fun_env: &FunctionEnv) -> BorrowAnnotation {
    let pool = fun_env.symbol_pool();
    match format!(
        "{}::{}",
        fun_env.module_env.get_name().display_full(pool),
        fun_env.get_name().display(pool)
    )
    .as_str()
    {
        VECTOR_BORROW_MUT => {
            let mut an = BorrowAnnotation::default();
            let param_node = BorrowNode::Reference(0);
            let return_node = BorrowNode::ReturnPlaceholder(0);
            let edge = BorrowEdge::Index;
            an.summary
                .borrowed_by
                .entry(param_node)
                .or_default()
                .insert((return_node, edge));
            an.summary.consolidate();
            an
        }
        _ => BorrowAnnotation::default(),
    }
}

struct BorrowAnalysis<'a> {
    func_target: &'a FunctionTarget<'a>,
    livevar_annotation: &'a LiveVarAnnotation,
    targets: &'a FunctionTargetsHolder,
}

impl<'a> BorrowAnalysis<'a> {
    fn new(func_target: &'a FunctionTarget<'a>, targets: &'a FunctionTargetsHolder) -> Self {
        let livevar_annotation = func_target
            .get_annotations()
            .get::<LiveVarAnnotation>()
            .expect("livevar annotation");

        Self {
            func_target,
            livevar_annotation,
            targets,
        }
    }

    fn analyze(&self, instrs: &[Bytecode]) -> BorrowAnnotation {
        let cfg = StacklessControlFlowGraph::new_forward(instrs);

        let mut state = BorrowInfo::default();

        // Initialize state from parameters
        for idx in 0..self.func_target.get_parameter_count() {
            state.add_node(self.borrow_node(idx));
        }

        // Run the dataflow analysis
        let state_map = self.analyze_function(state, instrs, &cfg);

        // Summarize the result
        let code_map = self.state_per_instruction(state_map, instrs, &cfg, |before, after| {
            let mut before = before.clone();
            let mut after = after.clone();
            before.consolidate();
            after.consolidate();
            BorrowInfoAtCodeOffset { before, after }
        });
        let mut summary = BorrowInfo::default();
        for (offs, code) in instrs.iter().enumerate() {
            if let Bytecode::Ret(_, temps) = code {
                if let Some(info) = code_map.get(&(offs as u16)) {
                    summary.summarize(self.func_target, &info.before, temps);
                }
            }
        }
        summary.consolidate();
        BorrowAnnotation { summary, code_map }
    }

    fn borrow_node(&self, idx: TempIndex) -> BorrowNode {
        let ty = self.func_target.get_local_type(idx);
        if ty.is_reference() {
            BorrowNode::Reference(idx)
        } else {
            BorrowNode::LocalRoot(idx)
        }
    }

    fn remap_borrow_node(&self, state: &mut BorrowInfo, from: &BorrowNode, to: &BorrowNode) {
        let remap = |node: BorrowNode| if &node == from { to.clone() } else { node };
        state.live_nodes = std::mem::take(&mut state.live_nodes)
            .into_iter()
            .map(remap)
            .collect();
        state.borrowed_by = std::mem::take(&mut state.borrowed_by)
            .into_iter()
            .map(|(src, dests)| {
                (
                    remap(src),
                    dests
                        .into_iter()
                        .map(|(node, edges)| (remap(node), edges))
                        .collect(),
                )
            })
            .collect()
    }
}

impl<'a> TransferFunctions for BorrowAnalysis<'a> {
    type State = BorrowInfo;
    const BACKWARD: bool = false;

    fn execute(&self, state: &mut BorrowInfo, instr: &Bytecode, code_offset: CodeOffset) {
        use Bytecode::*;
        let livevar_annotation_at = self
            .livevar_annotation
            .get_live_var_info_at(code_offset)
            .expect("livevar annotation");
        match instr {
            Assign(_, dest, src, kind) => {
                let dest_node = self.borrow_node(*dest);
                let src_node = self.borrow_node(*src);
                match kind {
                    AssignKind::Move | AssignKind::Store => {
                        self.remap_borrow_node(state, &src_node, &dest_node);
                        state.moved_nodes.insert(src_node);
                    }
                    AssignKind::Copy => {
                        state.add_node(dest_node.clone());
                        state.add_edge(src_node, dest_node, BorrowEdge::Direct);
                    }
                }
            }
            Call(id, dests, oper, srcs, _) => {
                use Operation::*;
                match oper {
                    // In the borrows below, we only create an edge if the
                    // borrowed value is actually alive. For a dead borrow we would
                    // otherwise never end live time, because we cannot see a node
                    // being created and dying at the very same instruction.
                    BorrowLoc if livevar_annotation_at.after.contains(&dests[0]) => {
                        let dest_node = self.borrow_node(dests[0]);
                        let src_node = self.borrow_node(srcs[0]);
                        state.add_node(dest_node.clone());
                        state.add_edge(src_node, dest_node, BorrowEdge::Direct);
                    }
                    BorrowGlobal(mid, sid, inst)
                        if livevar_annotation_at.after.contains(&dests[0]) =>
                    {
                        let dest_node = self.borrow_node(dests[0]);
                        let src_node = BorrowNode::GlobalRoot(QualifiedInstId {
                            module_id: *mid,
                            id: *sid,
                            inst: inst.to_owned(),
                        });
                        state.add_node(dest_node.clone());
                        state.add_edge(src_node, dest_node, BorrowEdge::Direct);
                    }
                    BorrowField(mid, sid, inst, field)
                        if livevar_annotation_at.after.contains(&dests[0]) =>
                    {
                        let dest_node = self.borrow_node(dests[0]);
                        let src_node = self.borrow_node(srcs[0]);
                        state.add_node(dest_node.clone());
                        state.add_edge(
                            src_node,
                            dest_node,
                            BorrowEdge::Field(mid.qualified_inst(*sid, inst.to_owned()), *field),
                        );
                    }
                    Function(mid, fid, ..) => {
                        let callee_env = &self
                            .func_target
                            .global_env()
                            .get_function_qid(mid.qualified(*fid));
                        let callee_target = &self
                            .targets
                            .get_target(callee_env, &FunctionVariant::Baseline);
                        if let Some(callee_an) =
                            callee_target.get_annotations().get::<BorrowAnnotation>()
                        {
                            state.instantiate(callee_target, &callee_an.summary, srcs, dests);
                        } else {
                            // This can happen for recursive functions. Check whether the function
                            // has &mut returns, and report an error that we can't deal with it if
                            // so.
                            let has_muts = (0..callee_target.get_return_count()).any(|idx| {
                                callee_target.get_return_type(idx).is_mutable_reference()
                            });
                            if has_muts {
                                callee_target.global_env().error(&self.func_target.get_bytecode_loc(*id),
                                    "restriction: recursive functions which return `&mut` values not supported");
                            }
                        }
                    }
                    OpaqueCallBegin(_, _, _) | OpaqueCallEnd(_, _, _) => {
                        // just skip
                    }
                    _ => {
                        // Other operations do not create references.
                    }
                }
            }
            _ => {
                // Other instructions do not create references
            }
        }

        // Update live_vars.

        for idx in livevar_annotation_at
            .before
            .difference(&livevar_annotation_at.after)
        {
            if self.func_target.get_local_type(*idx).is_reference() {
                let node = self.borrow_node(*idx);
                state.remove_node(&node);
            }
        }
    }
}

impl<'a> DataflowAnalysis for BorrowAnalysis<'a> {}

impl AbstractDomain for BorrowInfo {
    fn join(&mut self, other: &Self) -> JoinResult {
        let live_changed = self.live_nodes.join(&other.live_nodes);
        let moved_changed = self.moved_nodes.join(&other.moved_nodes);
        let borrowed_changed = self.borrowed_by.join(&other.borrowed_by);
        borrowed_changed
            .combine(moved_changed)
            .combine(live_changed)
    }
}

// =================================================================================================
// Formatting

/// Format a borrow annotation.
pub fn format_borrow_annotation(
    func_target: &FunctionTarget<'_>,
    code_offset: CodeOffset,
) -> Option<String> {
    if let Some(BorrowAnnotation { code_map, .. }) =
        func_target.get_annotations().get::<BorrowAnnotation>()
    {
        if let Some(map_at) = code_map.get(&code_offset) {
            if !map_at.before.is_empty() {
                return Some(map_at.before.borrow_info_str(func_target));
            }
        }
    }
    None
}
