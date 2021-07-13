// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use super::{context::*, remove_fallthrough_jumps};
use crate::{
    cfgir::ast as G,
    compiled_unit::*,
    expansion::ast::{AbilitySet, Address, ModuleIdent, ModuleIdent_, SpecId, Value_},
    hlir::{
        ast::{self as H},
        translate::{display_var, DisplayVar},
    },
    naming::ast::{BuiltinTypeName_, StructTypeParameter, TParam},
    parser::ast::{
        Ability, Ability_, BinOp, BinOp_, ConstantName, Field, FunctionName, StructName, UnaryOp,
        UnaryOp_, Var, Visibility,
    },
    shared::{unique_map::UniqueMap, *},
    FullyCompiledProgram,
};
use bytecode_source_map::source_map::SourceMap;
use move_binary_format::file_format as F;
use move_core_types::account_address::AccountAddress as MoveAddress;
use move_ir_types::{ast as IR, location::*};
use std::collections::{BTreeMap, BTreeSet, HashMap};

type CollectedInfos = UniqueMap<FunctionName, CollectedInfo>;
type CollectedInfo = (
    Vec<(Var, H::SingleType)>,
    BTreeMap<SpecId, (IR::NopLabel, BTreeMap<Var, H::SingleType>)>,
);

fn extract_decls(
    compilation_env: &mut CompilationEnv,
    pre_compiled_lib: Option<&FullyCompiledProgram>,
    prog: &G::Program,
) -> (
    HashMap<ModuleIdent, usize>,
    HashMap<(ModuleIdent, StructName), (BTreeSet<IR::Ability>, Vec<IR::StructTypeParameter>)>,
    HashMap<
        (ModuleIdent, FunctionName),
        (BTreeSet<(ModuleIdent, StructName)>, IR::FunctionSignature),
    >,
) {
    let pre_compiled_modules = || {
        pre_compiled_lib
            .iter()
            .map(|pre_compiled| {
                pre_compiled
                    .cfgir
                    .modules
                    .key_cloned_iter()
                    .filter(|(mident, _m)| !prog.modules.contains_key(mident))
            })
            .flatten()
    };

    let mut max_ordering = 0;
    let mut orderings: HashMap<ModuleIdent, usize> = pre_compiled_modules()
        .map(|(m, mdef)| {
            max_ordering = std::cmp::max(max_ordering, mdef.dependency_order);
            (m, mdef.dependency_order)
        })
        .collect();
    for (m, mdef) in prog.modules.key_cloned_iter() {
        orderings.insert(m, mdef.dependency_order + 1 + max_ordering);
    }

    let all_modules = || prog.modules.key_cloned_iter().chain(pre_compiled_modules());
    let sdecls = all_modules()
        .flat_map(|(m, mdef)| {
            mdef.structs.key_cloned_iter().map(move |(s, sdef)| {
                let key = (m.clone(), s);
                let abilities = abilities(&sdef.abilities);
                let type_parameters = struct_type_parameters(sdef.type_parameters.clone());
                (key, (abilities, type_parameters))
            })
        })
        .collect();
    let context = &mut Context::new(compilation_env, &prog.addresses, None);
    let fdecls = all_modules()
        .flat_map(|(m, mdef)| {
            mdef.functions.key_cloned_iter().map(move |(f, fdef)| {
                let key = (m.clone(), f);
                let seen = seen_structs(&fdef.signature);
                let gsig = fdef.signature.clone();
                (key, (seen, gsig))
            })
        })
        .map(|(key, (seen, gsig))| (key, (seen, function_signature(context, gsig))))
        .collect();
    (orderings, sdecls, fdecls)
}

//**************************************************************************************************
// Entry
//**************************************************************************************************

pub fn program(
    compilation_env: &mut CompilationEnv,
    pre_compiled_lib: Option<&FullyCompiledProgram>,
    prog: G::Program,
) -> Vec<CompiledUnit> {
    let mut units = vec![];

    let (orderings, sdecls, fdecls) = extract_decls(compilation_env, pre_compiled_lib, &prog);
    let G::Program {
        addresses,
        modules: gmodules,
        scripts: gscripts,
    } = prog;

    let mut source_modules = gmodules
        .into_iter()
        .filter(|(_, mdef)| mdef.is_source_module)
        .collect::<Vec<_>>();
    source_modules.sort_by_key(|(_, mdef)| mdef.dependency_order);
    for (m, mdef) in source_modules {
        if let Some(unit) = module(
            compilation_env,
            &addresses,
            m,
            mdef,
            &orderings,
            &sdecls,
            &fdecls,
        ) {
            units.push(unit)
        }
    }
    for (key, s) in gscripts {
        let G::Script {
            attributes: _attributes,
            loc: _loc,
            constants,
            function_name,
            function,
        } = s;
        if let Some(unit) = script(
            compilation_env,
            &addresses,
            key,
            constants,
            function_name,
            function,
            &orderings,
            &sdecls,
            &fdecls,
        ) {
            units.push(unit)
        }
    }
    units
}

fn module(
    compilation_env: &mut CompilationEnv,
    addresses: &UniqueMap<Name, AddressBytes>,
    ident: ModuleIdent,
    mdef: G::ModuleDefinition,
    dependency_orderings: &HashMap<ModuleIdent, usize>,
    struct_declarations: &HashMap<
        (ModuleIdent, StructName),
        (BTreeSet<IR::Ability>, Vec<IR::StructTypeParameter>),
    >,
    function_declarations: &HashMap<
        (ModuleIdent, FunctionName),
        (BTreeSet<(ModuleIdent, StructName)>, IR::FunctionSignature),
    >,
) -> Option<CompiledUnit> {
    let mut context = Context::new(compilation_env, addresses, Some(&ident));
    let structs = mdef
        .structs
        .into_iter()
        .map(|(s, sdef)| struct_def(&mut context, &ident, s, sdef))
        .collect();
    let constants = mdef
        .constants
        .into_iter()
        .map(|(n, c)| constant(&mut context, Some(&ident), n, c))
        .collect();

    let mut collected_function_infos = UniqueMap::new();
    let functions = mdef
        .functions
        .into_iter()
        .map(|(f, fdef)| {
            let (res, info) = function(&mut context, Some(&ident), f.clone(), fdef);
            collected_function_infos.add(f, info).unwrap();
            res
        })
        .collect();

    let friends = mdef
        .friends
        .into_iter()
        .filter_map(|(mident, _loc)| context.translate_module_ident(mident))
        .collect();

    let addr_name = match &ident.value.address {
        Address::Anonymous(_) => None,
        Address::Named(n) => Some(n.clone()),
    };
    let addr_bytes =
        context.resolve_address(ident.loc, ident.value.address.clone(), "module declaration")?;
    let (imports, explicit_dependency_declarations) = context.materialize(
        dependency_orderings,
        struct_declarations,
        function_declarations,
    );

    let sp!(
        ident_loc,
        ModuleIdent_ {
            address: _,
            module: module_name
        }
    ) = ident;
    let ir_module = IR::ModuleDefinition {
        name: IR::ModuleName::new(module_name.0.value.clone()),
        friends,
        imports,
        explicit_dependency_declarations,
        structs,
        constants,
        functions,
        synthetics: vec![],
    };
    let deps: Vec<&F::CompiledModule> = vec![];
    let addr = MoveAddress::new(addr_bytes.into_bytes());
    let (module, source_map) = match ir_to_bytecode::compiler::compile_module(addr, ir_module, deps)
    {
        Ok(res) => res,
        Err(e) => {
            compilation_env
                .add_error_deprecated(vec![(ident_loc, format!("ICE. IR ERROR: {}", e))]);
            return None;
        }
    };
    let function_infos = module_function_infos(&module, &source_map, &collected_function_infos);
    Some(CompiledUnit::Module {
        ident: CompiledModuleIdent::new(ident_loc, addr_name, addr_bytes, module_name),
        module,
        source_map,
        function_infos,
    })
}

fn script(
    compilation_env: &mut CompilationEnv,
    addresses: &UniqueMap<Name, AddressBytes>,
    key: String,
    constants: UniqueMap<ConstantName, G::Constant>,
    name: FunctionName,
    fdef: G::Function,
    dependency_orderings: &HashMap<ModuleIdent, usize>,
    struct_declarations: &HashMap<
        (ModuleIdent, StructName),
        (BTreeSet<IR::Ability>, Vec<IR::StructTypeParameter>),
    >,
    function_declarations: &HashMap<
        (ModuleIdent, FunctionName),
        (BTreeSet<(ModuleIdent, StructName)>, IR::FunctionSignature),
    >,
) -> Option<CompiledUnit> {
    let loc = name.loc();
    let mut context = Context::new(compilation_env, addresses, None);

    let constants = constants
        .into_iter()
        .map(|(n, c)| constant(&mut context, None, n, c))
        .collect();

    let ((_, main), info) = function(&mut context, None, name, fdef);

    let (imports, explicit_dependency_declarations) = context.materialize(
        dependency_orderings,
        struct_declarations,
        function_declarations,
    );
    let ir_script = IR::Script {
        imports,
        explicit_dependency_declarations,
        constants,
        main,
    };
    let deps: Vec<&F::CompiledModule> = vec![];
    let (script, source_map) = match ir_to_bytecode::compiler::compile_script(None, ir_script, deps)
    {
        Ok(res) => res,
        Err(e) => {
            compilation_env.add_error_deprecated(vec![(loc, format!("IR ERROR: {}", e))]);
            return None;
        }
    };
    let function_info = script_function_info(&source_map, info);
    Some(CompiledUnit::Script {
        loc,
        key,
        script,
        source_map,
        function_info,
    })
}

fn module_function_infos(
    compile_module: &F::CompiledModule,
    source_map: &SourceMap<Loc>,
    collected_function_infos: &CollectedInfos,
) -> UniqueMap<FunctionName, FunctionInfo> {
    UniqueMap::maybe_from_iter((0..compile_module.function_defs.len()).map(|i| {
        let idx = F::FunctionDefinitionIndex(i as F::TableIndex);
        function_info_map(compile_module, source_map, collected_function_infos, idx)
    }))
    .unwrap()
}

fn function_info_map(
    compile_module: &F::CompiledModule,
    source_map: &SourceMap<Loc>,
    collected_function_infos: &CollectedInfos,
    idx: F::FunctionDefinitionIndex,
) -> (FunctionName, FunctionInfo) {
    let module = compile_module;
    let handle_idx = module.function_defs[idx.0 as usize].function;
    let name_idx = module.function_handles[handle_idx.0 as usize].name;
    let name = module.identifiers[name_idx.0 as usize]
        .clone()
        .into_string();

    let function_source_map = source_map.get_function_source_map(idx).unwrap();
    let local_map = function_source_map.make_local_name_to_index_map();
    let (params, specs) = collected_function_infos.get_(&name).unwrap();
    let parameters = params
        .iter()
        .map(|(v, ty)| var_info(&local_map, v.clone(), ty.clone()))
        .collect();
    let spec_info = specs
        .iter()
        .map(|(id, (label, used_local_types))| {
            let offset = *function_source_map.nops.get(label).unwrap();
            let used_locals = used_local_info(&local_map, used_local_types);
            let info = SpecInfo {
                offset,
                used_locals,
            };
            (*id, info)
        })
        .collect();
    let function_info = FunctionInfo {
        spec_info,
        parameters,
    };

    let name_loc = *collected_function_infos.get_loc_(&name).unwrap();
    let function_name = FunctionName(sp(name_loc, name));
    (function_name, function_info)
}

fn script_function_info(
    source_map: &SourceMap<Loc>,
    (params, specs): CollectedInfo,
) -> FunctionInfo {
    let idx = F::FunctionDefinitionIndex(0);
    let function_source_map = source_map.get_function_source_map(idx).unwrap();
    let local_map = function_source_map.make_local_name_to_index_map();
    let parameters = params
        .into_iter()
        .map(|(v, ty)| var_info(&local_map, v, ty))
        .collect();
    let spec_info = specs
        .into_iter()
        .map(|(id, (label, used_local_types))| {
            let offset = *function_source_map.nops.get(&label).unwrap();
            let used_locals = used_local_info(&local_map, &used_local_types);
            let info = SpecInfo {
                offset,
                used_locals,
            };
            (id, info)
        })
        .collect();
    FunctionInfo {
        spec_info,
        parameters,
    }
}

fn used_local_info(
    local_map: &BTreeMap<&String, F::LocalIndex>,
    used_local_types: &BTreeMap<Var, H::SingleType>,
) -> UniqueMap<Var, VarInfo> {
    UniqueMap::maybe_from_iter(used_local_types.iter().map(|(v, ty)| {
        let (v, info) = var_info(&local_map, v.clone(), ty.clone());
        let v_orig_ = match display_var(&v.0.value) {
            DisplayVar::Tmp => panic!("ICE spec block captured a tmp"),
            DisplayVar::Orig(s) => s,
        };
        let v_orig = Var(sp(v.0.loc, v_orig_));
        (v_orig, info)
    }))
    .unwrap()
}

fn var_info(
    local_map: &BTreeMap<&String, F::LocalIndex>,
    v: Var,
    type_: H::SingleType,
) -> (Var, VarInfo) {
    let index = *local_map.get(&v.0.value).unwrap();
    (v, VarInfo { type_, index })
}

//**************************************************************************************************
// Structs
//**************************************************************************************************

fn struct_def(
    context: &mut Context,
    m: &ModuleIdent,
    s: StructName,
    sdef: H::StructDefinition,
) -> IR::StructDefinition {
    let H::StructDefinition {
        attributes: _attributes,
        abilities: abs,
        type_parameters: tys,
        fields,
    } = sdef;
    let loc = s.loc();
    let name = context.struct_definition_name(m, s);
    let abilities = abilities(&abs);
    let type_formals = struct_type_parameters(tys);
    let fields = struct_fields(context, loc, fields);
    sp(
        loc,
        IR::StructDefinition_ {
            name,
            abilities,
            type_formals,
            fields,
            invariants: vec![],
        },
    )
}

fn struct_fields(
    context: &mut Context,
    loc: Loc,
    gfields: H::StructFields,
) -> IR::StructDefinitionFields {
    use H::StructFields as HF;
    use IR::StructDefinitionFields as IRF;
    match gfields {
        HF::Native(_) => IRF::Native,
        HF::Defined(field_vec) if field_vec.is_empty() => {
            // empty fields are not allowed in the bytecode, add a dummy field
            let fake_field = vec![(
                Field(sp(loc, "dummy_field".to_string())),
                H::BaseType_::bool(loc),
            )];
            struct_fields(context, loc, HF::Defined(fake_field))
        }
        HF::Defined(field_vec) => {
            let fields = field_vec
                .into_iter()
                .map(|(f, ty)| (field(f), base_type(context, ty)))
                .collect();
            IRF::Move { fields }
        }
    }
}

//**************************************************************************************************
// Structs
//**************************************************************************************************

fn constant(
    context: &mut Context,
    m: Option<&ModuleIdent>,
    n: ConstantName,
    c: G::Constant,
) -> IR::Constant {
    let name = context.constant_definition_name(m, n);
    let signature = base_type(context, c.signature);
    let value = c.value.unwrap();
    IR::Constant {
        name,
        signature,
        value,
    }
}

//**************************************************************************************************
// Functions
//**************************************************************************************************

fn function(
    context: &mut Context,
    m: Option<&ModuleIdent>,
    f: FunctionName,
    fdef: G::Function,
) -> ((IR::FunctionName, IR::Function), CollectedInfo) {
    let G::Function {
        attributes: _attributes,
        visibility: v,
        signature,
        acquires,
        body,
    } = fdef;
    let v = visibility(v);
    let parameters = signature.parameters.clone();
    let signature = function_signature(context, signature);
    let acquires = acquires
        .into_iter()
        .map(|(s, _)| context.struct_definition_name(m.unwrap(), s))
        .collect();
    let body = match body.value {
        G::FunctionBody_::Native => IR::FunctionBody::Native,
        G::FunctionBody_::Defined {
            locals,
            start,
            loop_heads,
            blocks,
        } => {
            let (locals, code) = function_body(
                context,
                parameters.clone(),
                locals,
                loop_heads,
                start,
                blocks,
            );
            IR::FunctionBody::Bytecode { locals, code }
        }
    };
    let loc = f.loc();
    let name = context.function_definition_name(m, f);
    let ir_function = IR::Function_ {
        visibility: v,
        signature,
        acquires,
        specifications: vec![],
        body,
    };
    (
        (name, sp(loc, ir_function)),
        (parameters, context.finish_function()),
    )
}

fn visibility(v: Visibility) -> IR::FunctionVisibility {
    match v {
        Visibility::Public(_) => IR::FunctionVisibility::Public,
        Visibility::Script(_) => IR::FunctionVisibility::Script,
        Visibility::Friend(_) => IR::FunctionVisibility::Friend,
        Visibility::Internal => IR::FunctionVisibility::Internal,
    }
}

fn function_signature(context: &mut Context, sig: H::FunctionSignature) -> IR::FunctionSignature {
    let return_type = types(context, sig.return_type);
    let formals = sig
        .parameters
        .into_iter()
        .map(|(v, st)| (var(v), single_type(context, st)))
        .collect();
    let type_parameters = fun_type_parameters(sig.type_parameters);
    IR::FunctionSignature {
        return_type,
        formals,
        type_formals: type_parameters,
    }
}

fn seen_structs(sig: &H::FunctionSignature) -> BTreeSet<(ModuleIdent, StructName)> {
    let mut seen = BTreeSet::new();
    seen_structs_type(&mut seen, &sig.return_type);
    sig.parameters
        .iter()
        .for_each(|(_, st)| seen_structs_single_type(&mut seen, st));
    seen
}

fn seen_structs_type(seen: &mut BTreeSet<(ModuleIdent, StructName)>, sp!(_, t_): &H::Type) {
    use H::Type_ as T;
    match t_ {
        T::Unit => (),
        T::Single(st) => seen_structs_single_type(seen, st),
        T::Multiple(ss) => ss.iter().for_each(|st| seen_structs_single_type(seen, st)),
    }
}

fn seen_structs_single_type(
    seen: &mut BTreeSet<(ModuleIdent, StructName)>,
    sp!(_, st_): &H::SingleType,
) {
    use H::SingleType_ as S;
    match st_ {
        S::Base(bt) | S::Ref(_, bt) => seen_structs_base_type(seen, bt),
    }
}

fn seen_structs_base_type(
    seen: &mut BTreeSet<(ModuleIdent, StructName)>,
    sp!(_, bt_): &H::BaseType,
) {
    use H::{BaseType_ as B, TypeName_ as TN};
    match bt_ {
        B::Unreachable | B::UnresolvedError => {
            panic!("ICE should not have reached compilation if there are errors")
        }
        B::Apply(_, sp!(_, tn_), tys) => {
            if let TN::ModuleType(m, s) = tn_ {
                seen.insert((m.clone(), s.clone()));
            }
            tys.iter().for_each(|st| seen_structs_base_type(seen, st))
        }
        B::Param(TParam { .. }) => (),
    }
}

fn function_body(
    context: &mut Context,
    parameters: Vec<(Var, H::SingleType)>,
    mut locals_map: UniqueMap<Var, H::SingleType>,
    loop_heads: BTreeSet<H::Label>,
    start: H::Label,
    blocks_map: H::BasicBlocks,
) -> (Vec<(IR::Var, IR::Type)>, IR::BytecodeBlocks) {
    parameters
        .iter()
        .for_each(|(var, _)| assert!(locals_map.remove(var).is_some()));
    let locals = locals_map
        .into_iter()
        .map(|(v, ty)| (var(v), single_type(context, ty)))
        .collect();
    let mut blocks = blocks_map.into_iter().collect::<Vec<_>>();
    blocks.sort_by_key(|(lbl, _)| *lbl);

    let mut bytecode_blocks = Vec::new();
    for (idx, (lbl, basic_block)) in blocks.into_iter().enumerate() {
        // first idx should be the start label
        assert!(idx != 0 || lbl == start);
        assert!(idx == bytecode_blocks.len());

        let mut code = IR::BytecodeBlock::new();
        for cmd in basic_block {
            command(context, &mut code, cmd);
        }
        bytecode_blocks.push((label(lbl), code));
    }

    let loop_heads = loop_heads.into_iter().map(label).collect();
    remove_fallthrough_jumps::code(&loop_heads, &mut bytecode_blocks);

    (locals, bytecode_blocks)
}

//**************************************************************************************************
// Names
//**************************************************************************************************

fn type_var(sp!(loc, n): Name) -> IR::TypeVar {
    sp(loc, IR::TypeVar_::new(n))
}

fn var(v: Var) -> IR::Var {
    sp(v.0.loc, IR::Var_::new(v.0.value))
}

fn field(f: Field) -> IR::Field {
    sp(f.0.loc, IR::Field_::new(f.0.value))
}

fn struct_definition_name(
    context: &mut Context,
    sp!(_, t_): H::Type,
) -> (IR::StructName, Vec<IR::Type>) {
    match t_ {
        H::Type_::Single(st) => struct_definition_name_single(context, st),
        _ => panic!("ICE expected single type"),
    }
}

fn struct_definition_name_single(
    context: &mut Context,
    sp!(_, st_): H::SingleType,
) -> (IR::StructName, Vec<IR::Type>) {
    match st_ {
        H::SingleType_::Ref(_, bt) | H::SingleType_::Base(bt) => {
            struct_definition_name_base(context, bt)
        }
    }
}

fn struct_definition_name_base(
    context: &mut Context,
    sp!(_, bt_): H::BaseType,
) -> (IR::StructName, Vec<IR::Type>) {
    use H::{BaseType_ as B, TypeName_ as TN};
    match bt_ {
        B::Apply(_, sp!(_, TN::ModuleType(m, s)), tys) => (
            context.struct_definition_name(&m, s),
            base_types(context, tys),
        ),
        _ => panic!("ICE expected module struct type"),
    }
}

//**************************************************************************************************
// Types
//**************************************************************************************************

fn ability(sp!(_, a_): Ability) -> IR::Ability {
    use Ability_ as A;
    use IR::Ability as IRA;
    match a_ {
        A::Copy => IRA::Copy,
        A::Drop => IRA::Drop,
        A::Store => IRA::Store,
        A::Key => IRA::Key,
    }
}

fn abilities(set: &AbilitySet) -> BTreeSet<IR::Ability> {
    set.iter().map(ability).collect()
}

fn fun_type_parameters(tps: Vec<TParam>) -> Vec<(IR::TypeVar, BTreeSet<IR::Ability>)> {
    tps.into_iter()
        .map(|tp| (type_var(tp.user_specified_name), abilities(&tp.abilities)))
        .collect()
}

fn struct_type_parameters(tps: Vec<StructTypeParameter>) -> Vec<IR::StructTypeParameter> {
    tps.into_iter()
        .map(|tp| {
            let name = type_var(tp.param.user_specified_name);
            let constraints = abilities(&tp.param.abilities);
            (tp.is_phantom, name, constraints)
        })
        .collect()
}

fn base_types(context: &mut Context, bs: Vec<H::BaseType>) -> Vec<IR::Type> {
    bs.into_iter().map(|b| base_type(context, b)).collect()
}

fn base_type(context: &mut Context, sp!(_, bt_): H::BaseType) -> IR::Type {
    use BuiltinTypeName_ as BT;
    use H::{BaseType_ as B, TypeName_ as TN};
    use IR::Type as IRT;
    match bt_ {
        B::Unreachable | B::UnresolvedError => {
            panic!("ICE should not have reached compilation if there are errors")
        }
        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::Address))), _) => IRT::Address,
        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::Signer))), _) => IRT::Signer,
        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::U8))), _) => IRT::U8,
        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::U64))), _) => IRT::U64,
        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::U128))), _) => IRT::U128,

        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::Bool))), _) => IRT::Bool,
        B::Apply(_, sp!(_, TN::Builtin(sp!(_, BT::Vector))), mut args) => {
            assert!(
                args.len() == 1,
                "ICE vector must have exactly 1 type argument"
            );
            IRT::Vector(Box::new(base_type(context, args.pop().unwrap())))
        }
        B::Apply(_, sp!(_, TN::ModuleType(m, s)), tys) => {
            let n = context.qualified_struct_name(&m, s);
            let tys = base_types(context, tys);
            IRT::Struct(n, tys)
        }
        B::Param(TParam {
            user_specified_name,
            ..
        }) => IRT::TypeParameter(type_var(user_specified_name).value),
    }
}

fn single_type(context: &mut Context, sp!(_, st_): H::SingleType) -> IR::Type {
    use H::SingleType_ as S;
    use IR::Type as IRT;
    match st_ {
        S::Base(bt) => base_type(context, bt),
        S::Ref(mut_, bt) => IRT::Reference(mut_, Box::new(base_type(context, bt))),
    }
}

fn types(context: &mut Context, sp!(_, t_): H::Type) -> Vec<IR::Type> {
    use H::Type_ as T;
    match t_ {
        T::Unit => vec![],
        T::Single(st) => vec![single_type(context, st)],
        T::Multiple(ss) => ss.into_iter().map(|st| single_type(context, st)).collect(),
    }
}

//**************************************************************************************************
// Commands
//**************************************************************************************************

fn label(lbl: H::Label) -> IR::BlockLabel {
    IR::BlockLabel(format!("{}", lbl))
}

fn command(context: &mut Context, code: &mut IR::BytecodeBlock, sp!(loc, cmd_): H::Command) {
    use H::Command_ as C;
    use IR::Bytecode_ as B;
    match cmd_ {
        C::Assign(ls, e) => {
            exp(context, code, e);
            lvalues(context, code, ls);
        }
        C::Mutate(eref, ervalue) => {
            exp(context, code, ervalue);
            exp(context, code, eref);
            code.push(sp(loc, B::WriteRef));
        }
        C::Abort(ecode) => {
            exp_(context, code, ecode);
            code.push(sp(loc, B::Abort));
        }
        C::Return { exp: e, .. } => {
            exp_(context, code, e);
            code.push(sp(loc, B::Ret));
        }
        C::IgnoreAndPop { pop_num, exp: e } => {
            exp_(context, code, e);
            for _ in 0..pop_num {
                code.push(sp(loc, B::Pop));
            }
        }
        C::Jump { target, .. } => code.push(sp(loc, B::Branch(label(target)))),
        C::JumpIf {
            cond,
            if_true,
            if_false,
        } => {
            exp_(context, code, cond);
            code.push(sp(loc, B::BrTrue(label(if_true))));
            code.push(sp(loc, B::Branch(label(if_false))));
        }
        C::Break | C::Continue => panic!("ICE break/continue not translated to jumps"),
    }
}

fn lvalues(context: &mut Context, code: &mut IR::BytecodeBlock, ls: Vec<H::LValue>) {
    lvalues_(context, code, ls.into_iter())
}

fn lvalues_(
    context: &mut Context,
    code: &mut IR::BytecodeBlock,
    ls: impl std::iter::DoubleEndedIterator<Item = H::LValue>,
) {
    for l in ls.rev() {
        lvalue(context, code, l)
    }
}

fn lvalue(context: &mut Context, code: &mut IR::BytecodeBlock, sp!(loc, l_): H::LValue) {
    use H::LValue_ as L;
    use IR::Bytecode_ as B;
    match l_ {
        L::Ignore => {
            code.push(sp(loc, B::Pop));
        }
        L::Var(v, _) => {
            code.push(sp(loc, B::StLoc(var(v))));
        }
        L::Unpack(s, tys, field_ls) if field_ls.is_empty() => {
            let n = context.struct_definition_name(context.current_module().unwrap(), s);
            code.push(sp(loc, B::Unpack(n, base_types(context, tys))));
            // Pop off false
            code.push(sp(loc, B::Pop));
        }

        L::Unpack(s, tys, field_ls) => {
            let n = context.struct_definition_name(context.current_module().unwrap(), s);
            code.push(sp(loc, B::Unpack(n, base_types(context, tys))));

            lvalues_(context, code, field_ls.into_iter().map(|(_, l)| l));
        }
    }
}

//**************************************************************************************************
// Expressions
//**************************************************************************************************

fn exp(context: &mut Context, code: &mut IR::BytecodeBlock, e: Box<H::Exp>) {
    exp_(context, code, *e)
}

fn exp_(context: &mut Context, code: &mut IR::BytecodeBlock, e: H::Exp) {
    use Value_ as V;
    use H::UnannotatedExp_ as E;
    use IR::Bytecode_ as B;
    let sp!(loc, e_) = e.exp;
    match e_ {
        E::Unreachable => panic!("ICE should not compile dead code"),
        E::UnresolvedError => panic!("ICE should not have reached compilation if there are errors"),
        E::Unit { .. } => (),
        // remember to switch to orig_name
        E::Spec(id, used_locals) => code.push(sp(loc, B::Nop(Some(context.spec(id, used_locals))))),
        E::Value(v) => {
            code.push(sp(
                loc,
                match v.value {
                    V::InferredNum(_) => panic!("ICE inferred num should have been expanded"),
                    V::Address(a) => {
                        let addr_bytes = context
                            .resolve_address(loc, a, "address value")
                            .unwrap_or(AddressBytes::DEFAULT_ERROR_BYTES);
                        B::LdAddr(MoveAddress::new(addr_bytes.into_bytes()))
                    }
                    V::Bytearray(bytes) => B::LdByteArray(bytes),
                    V::U8(u) => B::LdU8(u),
                    V::U64(u) => B::LdU64(u),
                    V::U128(u) => B::LdU128(u),
                    V::Bool(b) => {
                        if b {
                            B::LdTrue
                        } else {
                            B::LdFalse
                        }
                    }
                },
            ));
        }
        E::Move { var: v, .. } => {
            code.push(sp(loc, B::MoveLoc(var(v))));
        }
        E::Copy { var: v, .. } => code.push(sp(loc, B::CopyLoc(var(v)))),

        E::Constant(c) => code.push(sp(loc, B::LdConst(context.constant_name(c)))),

        E::ModuleCall(mcall) => {
            exp(context, code, mcall.arguments);
            module_call(
                context,
                loc,
                code,
                mcall.module,
                mcall.name,
                mcall.type_arguments,
            );
        }

        E::Builtin(b, arg) => {
            exp(context, code, arg);
            builtin(context, code, *b);
        }

        E::Freeze(er) => {
            exp(context, code, er);
            code.push(sp(loc, B::FreezeRef));
        }

        E::Dereference(er) => {
            exp(context, code, er);
            code.push(sp(loc, B::ReadRef));
        }

        E::UnaryExp(op, er) => {
            exp(context, code, er);
            unary_op(code, op);
        }

        E::BinopExp(el, op, er) => {
            exp(context, code, el);
            exp(context, code, er);
            binary_op(code, op);
        }

        E::Pack(s, tys, field_args) if field_args.is_empty() => {
            // empty fields are not allowed in the bytecode, add a dummy field
            // empty structs have a dummy field of type 'bool' added

            // Push on fake field
            code.push(sp(loc, B::LdFalse));

            let n = context.struct_definition_name(context.current_module().unwrap(), s);
            code.push(sp(loc, B::Pack(n, base_types(context, tys))))
        }

        E::Pack(s, tys, field_args) => {
            for (_, _, earg) in field_args {
                exp_(context, code, earg);
            }
            let n = context.struct_definition_name(context.current_module().unwrap(), s);
            code.push(sp(loc, B::Pack(n, base_types(context, tys))))
        }

        E::ExpList(items) => {
            for item in items {
                let ei = match item {
                    H::ExpListItem::Single(ei, _) | H::ExpListItem::Splat(_, ei, _) => ei,
                };
                exp_(context, code, ei);
            }
        }

        E::Borrow(mut_, el, f) => {
            let (n, tys) = struct_definition_name(context, el.ty.clone());
            exp(context, code, el);
            let instr = if mut_ {
                B::MutBorrowField(n, tys, field(f))
            } else {
                B::ImmBorrowField(n, tys, field(f))
            };
            code.push(sp(loc, instr));
        }

        E::BorrowLocal(mut_, v) => {
            let instr = if mut_ {
                B::MutBorrowLoc(var(v))
            } else {
                B::ImmBorrowLoc(var(v))
            };
            code.push(sp(loc, instr));
        }

        E::Cast(el, sp!(_, bt_)) => {
            use BuiltinTypeName_ as BT;
            exp(context, code, el);
            let instr = match bt_ {
                BT::U8 => B::CastU8,
                BT::U64 => B::CastU64,
                BT::U128 => B::CastU128,
                _ => panic!("ICE type checking failed. unexpected cast"),
            };
            code.push(sp(loc, instr));
        }
    }
}

fn module_call(
    context: &mut Context,
    loc: Loc,
    code: &mut IR::BytecodeBlock,
    mident: ModuleIdent,
    fname: FunctionName,
    tys: Vec<H::BaseType>,
) {
    use IR::Bytecode_ as B;
    let (m, n) = context.qualified_function_name(&mident, fname);
    code.push(sp(loc, B::Call(m, n, base_types(context, tys))))
}

fn builtin(context: &mut Context, code: &mut IR::BytecodeBlock, sp!(loc, b_): H::BuiltinFunction) {
    use H::BuiltinFunction_ as HB;
    use IR::Bytecode_ as B;
    code.push(sp(
        loc,
        match b_ {
            HB::MoveTo(bt) => {
                let (n, tys) = struct_definition_name_base(context, bt);
                B::MoveTo(n, tys)
            }
            HB::MoveFrom(bt) => {
                let (n, tys) = struct_definition_name_base(context, bt);
                B::MoveFrom(n, tys)
            }
            HB::BorrowGlobal(false, bt) => {
                let (n, tys) = struct_definition_name_base(context, bt);
                B::ImmBorrowGlobal(n, tys)
            }
            HB::BorrowGlobal(true, bt) => {
                let (n, tys) = struct_definition_name_base(context, bt);
                B::MutBorrowGlobal(n, tys)
            }
            HB::Exists(bt) => {
                let (n, tys) = struct_definition_name_base(context, bt);
                B::Exists(n, tys)
            }
        },
    ))
}

fn unary_op(code: &mut IR::BytecodeBlock, sp!(loc, op_): UnaryOp) {
    use UnaryOp_ as O;
    use IR::Bytecode_ as B;
    code.push(sp(
        loc,
        match op_ {
            O::Not => B::Not,
        },
    ));
}

fn binary_op(code: &mut IR::BytecodeBlock, sp!(loc, op_): BinOp) {
    use BinOp_ as O;
    use IR::Bytecode_ as B;
    code.push(sp(
        loc,
        match op_ {
            O::Add => B::Add,
            O::Sub => B::Sub,
            O::Mul => B::Mul,
            O::Mod => B::Mod,
            O::Div => B::Div,
            O::BitOr => B::BitOr,
            O::BitAnd => B::BitAnd,
            O::Xor => B::Xor,
            O::Shl => B::Shl,
            O::Shr => B::Shr,

            O::And => B::And,
            O::Or => B::Or,

            O::Eq => B::Eq,
            O::Neq => B::Neq,

            O::Lt => B::Lt,
            O::Gt => B::Gt,

            O::Le => B::Le,
            O::Ge => B::Ge,

            O::Range | O::Implies | O::Iff => panic!("specification operator unexpected"),
        },
    ));
}
