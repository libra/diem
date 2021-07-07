// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    diag,
    errors::diagnostic_codes::*,
    expansion::{
        ast::{self as E, AbilitySet, ModuleIdent},
        translate::is_valid_struct_constant_or_schema_name as is_constant_name,
    },
    naming::ast as N,
    parser::ast::{Ability_, ConstantName, Field, FunctionName, StructName, Var},
    shared::{unique_map::UniqueMap, *},
    FullyCompiledProgram,
};
use move_ir_types::location::*;
use std::collections::BTreeMap;

//**************************************************************************************************
// Context
//**************************************************************************************************

#[derive(Debug, Clone)]
enum ResolvedType {
    TParam(Loc, N::TParam),
    BuiltinType,
}

impl ResolvedType {
    fn error_msg(&self, n: &Name) -> (Loc, String) {
        match self {
            ResolvedType::TParam(loc, _) => (
                *loc,
                format!("But '{}' was declared as a type parameter here", n),
            ),
            ResolvedType::BuiltinType => (n.loc, format!("But '{}' is a builtin type", n)),
        }
    }
}

struct Context<'env> {
    env: &'env mut CompilationEnv,
    current_module: Option<ModuleIdent>,
    scoped_types: BTreeMap<ModuleIdent, BTreeMap<String, (Loc, ModuleIdent, AbilitySet, usize)>>,
    unscoped_types: BTreeMap<String, ResolvedType>,
    scoped_functions: BTreeMap<ModuleIdent, BTreeMap<String, Loc>>,
    unscoped_constants: BTreeMap<String, Loc>,
    scoped_constants: BTreeMap<ModuleIdent, BTreeMap<String, Loc>>,
}

impl<'env> Context<'env> {
    fn new(
        compilation_env: &'env mut CompilationEnv,
        pre_compiled_lib: Option<&FullyCompiledProgram>,
        prog: &E::Program,
    ) -> Self {
        use ResolvedType as RT;
        let all_modules = || {
            prog.modules.key_cloned_iter().chain(
                pre_compiled_lib
                    .iter()
                    .map(|pre_compiled| {
                        pre_compiled
                            .expansion
                            .modules
                            .key_cloned_iter()
                            .filter(|(mident, _m)| !prog.modules.contains_key(mident))
                    })
                    .flatten(),
            )
        };
        let scoped_types = all_modules()
            .map(|(mident, mdef)| {
                let mems = mdef
                    .structs
                    .key_cloned_iter()
                    .map(|(s, sdef)| {
                        let abilities = sdef.abilities.clone();
                        let arity = sdef.type_parameters.len();
                        let sname = s.value().to_string();
                        (sname, (s.loc(), mident.clone(), abilities, arity))
                    })
                    .collect();
                (mident, mems)
            })
            .collect();
        let scoped_functions = all_modules()
            .map(|(mident, mdef)| {
                let mems = mdef
                    .functions
                    .iter()
                    .map(|(nloc, n, _)| (n.clone(), nloc))
                    .collect();
                (mident, mems)
            })
            .collect();
        let scoped_constants = all_modules()
            .map(|(mident, mdef)| {
                let mems = mdef
                    .constants
                    .iter()
                    .map(|(nloc, n, _)| (n.clone(), nloc))
                    .collect();
                (mident, mems)
            })
            .collect();
        let unscoped_types = N::BuiltinTypeName_::all_names()
            .iter()
            .map(|s| (s.to_string(), RT::BuiltinType))
            .collect();
        Self {
            env: compilation_env,
            current_module: None,
            scoped_types,
            scoped_functions,
            scoped_constants,
            unscoped_types,
            unscoped_constants: BTreeMap::new(),
        }
    }

    fn resolve_module(&mut self, m: &ModuleIdent) -> bool {
        // NOTE: piggybacking on `scoped_functions` to provide a set of modules in the context。
        // TODO: a better solution would be to have a single `BTreeMap<ModuleIdent, ModuleInfo>`
        // in the context that can be used to resolve modules, types, and functions.
        let resolved = self.scoped_functions.contains_key(m);
        if !resolved {
            self.env.add_diag(diag!(
                NameResolution::UnboundModule,
                (m.loc, format!("Unbound module '{}'", m))
            ))
        }
        resolved
    }

    fn resolve_module_type(
        &mut self,
        loc: Loc,
        m: &ModuleIdent,
        n: &Name,
    ) -> Option<(Loc, StructName, AbilitySet, usize)> {
        let types = match self.scoped_types.get(m) {
            None => {
                self.env.add_diag(diag!(
                    NameResolution::UnboundModule,
                    (m.loc, format!("Unbound module '{}'", m)),
                ));
                return None;
            }
            Some(members) => members,
        };
        match types.get(&n.value) {
            None => {
                let msg = format!(
                    "Invalid module access. Unbound struct '{}' in module '{}'",
                    n, m
                );
                self.env
                    .add_diag(diag!(NameResolution::UnboundModuleMember, (loc, msg)));
                None
            }
            Some((decl_loc, _, abilities, arity)) => {
                Some((*decl_loc, StructName(n.clone()), abilities.clone(), *arity))
            }
        }
    }

    fn resolve_module_function(
        &mut self,
        loc: Loc,
        m: &ModuleIdent,
        n: &Name,
    ) -> Option<FunctionName> {
        let functions = match self.scoped_functions.get(m) {
            None => {
                self.env.add_diag(diag!(
                    NameResolution::UnboundModule,
                    (m.loc, format!("Unbound module '{}'", m)),
                ));
                return None;
            }
            Some(members) => members,
        };
        match functions.get(&n.value).cloned() {
            None => {
                let msg = format!(
                    "Invalid module access. Unbound function '{}' in module '{}'",
                    n, m
                );
                self.env
                    .add_diag(diag!(NameResolution::UnboundModuleMember, (loc, msg)));
                None
            }
            Some(_) => Some(FunctionName(n.clone())),
        }
    }

    fn resolve_module_constant(
        &mut self,
        loc: Loc,
        m: &ModuleIdent,
        n: Name,
    ) -> Option<ConstantName> {
        let constants = match self.scoped_constants.get(m) {
            None => {
                self.env.add_diag(diag!(
                    NameResolution::UnboundModule,
                    (m.loc, format!("Unbound module '{}'", m)),
                ));
                return None;
            }
            Some(members) => members,
        };
        match constants.get(&n.value).cloned() {
            None => {
                let msg = format!(
                    "Invalid module access. Unbound constant '{}' in module '{}'",
                    n, m
                );
                self.env
                    .add_diag(diag!(NameResolution::UnboundModuleMember, (loc, msg)));
                None
            }
            Some(_) => Some(ConstantName(n)),
        }
    }

    fn resolve_unscoped_type(&mut self, n: &Name) -> Option<ResolvedType> {
        match self.unscoped_types.get(&n.value) {
            None => {
                let msg = format!("Unbound type '{}' in current scope", n);
                self.env
                    .add_diag(diag!(NameResolution::UnboundType, (n.loc, msg)));
                None
            }
            Some(rn) => Some(rn.clone()),
        }
    }

    fn resolve_struct_name(
        &mut self,
        loc: Loc,
        verb: &str,
        sp!(nloc, ma_): E::ModuleAccess,
        etys_opt: Option<Vec<E::Type>>,
    ) -> Option<(ModuleIdent, StructName, Option<Vec<N::Type>>)> {
        use E::ModuleAccess_ as EA;

        match ma_ {
            EA::Name(n) => match self.resolve_unscoped_type(&n) {
                None => {
                    assert!(self.env.has_errors());
                    None
                }
                Some(rt) => {
                    self.env.add_diag(diag!(
                        NameResolution::NamePositionMismatch,
                        (nloc, format!("Invalid {}. Expected a struct name", verb)),
                        rt.error_msg(&n),
                    ));
                    None
                }
            },
            EA::ModuleAccess(m, n) => match self.resolve_module_type(nloc, &m, &n) {
                None => {
                    assert!(self.env.has_errors());
                    None
                }
                Some((_, _, _, arity)) => {
                    let tys_opt = etys_opt.map(|etys| {
                        let tys = types(self, etys);
                        let name_f = || format!("{}::{}", &m, &n);
                        check_type_argument_arity(self, loc, name_f, tys, arity)
                    });
                    Some((m, StructName(n), tys_opt))
                }
            },
        }
    }

    fn resolve_constant(
        &mut self,
        sp!(loc, ma_): E::ModuleAccess,
    ) -> Option<(Option<ModuleIdent>, ConstantName)> {
        use E::ModuleAccess_ as EA;
        match ma_ {
            EA::Name(n) => match self.unscoped_constants.get(&n.value) {
                None => {
                    self.env.add_diag(diag!(
                        NameResolution::UnboundUnscopedName,
                        (loc, format!("Unbound constant '{}'", n)),
                    ));
                    None
                }
                Some(_) => Some((None, ConstantName(n))),
            },
            EA::ModuleAccess(m, n) => match self.resolve_module_constant(loc, &m, n) {
                None => {
                    assert!(self.env.has_errors());
                    None
                }
                Some(cname) => Some((Some(m), cname)),
            },
        }
    }

    fn bind_type(&mut self, s: String, rt: ResolvedType) {
        self.unscoped_types.insert(s, rt);
    }

    fn bind_constant(&mut self, s: String, loc: Loc) {
        self.unscoped_constants.insert(s, loc);
    }

    fn save_unscoped(&self) -> (BTreeMap<String, ResolvedType>, BTreeMap<String, Loc>) {
        (self.unscoped_types.clone(), self.unscoped_constants.clone())
    }

    fn restore_unscoped(
        &mut self,
        (types, constants): (BTreeMap<String, ResolvedType>, BTreeMap<String, Loc>),
    ) {
        self.unscoped_types = types;
        self.unscoped_constants = constants;
    }
}

//**************************************************************************************************
// Entry
//**************************************************************************************************

pub fn program(
    compilation_env: &mut CompilationEnv,
    pre_compiled_lib: Option<&FullyCompiledProgram>,
    prog: E::Program,
) -> N::Program {
    let mut context = Context::new(compilation_env, pre_compiled_lib, &prog);
    let E::Program {
        addresses,
        modules: emodules,
        scripts: escripts,
    } = prog;
    let addresses = addresses.filter_map(|_name, mapping| mapping.map(|sp!(_, addr)| addr));
    let modules = modules(&mut context, emodules);
    let scripts = scripts(&mut context, escripts);
    N::Program {
        addresses,
        modules,
        scripts,
    }
}

fn modules(
    context: &mut Context,
    modules: UniqueMap<ModuleIdent, E::ModuleDefinition>,
) -> UniqueMap<ModuleIdent, N::ModuleDefinition> {
    modules.map(|ident, mdef| module(context, ident, mdef))
}

fn module(
    context: &mut Context,
    ident: ModuleIdent,
    mdef: E::ModuleDefinition,
) -> N::ModuleDefinition {
    context.current_module = Some(ident);
    let E::ModuleDefinition {
        attributes,
        loc: _loc,
        is_source_module,
        dependency_order,
        immediate_neighbors: _,
        used_addresses: _,
        friends: efriends,
        structs: estructs,
        functions: efunctions,
        constants: econstants,
        specs: _specs,
    } = mdef;
    let friends = efriends.filter_map(|mident, f| friend(context, mident, f));
    let unscoped = context.save_unscoped();
    let structs = estructs.map(|name, s| {
        context.restore_unscoped(unscoped.clone());
        struct_def(context, name, s)
    });
    let functions = efunctions.map(|name, f| {
        context.restore_unscoped(unscoped.clone());
        function(context, name, f)
    });
    let constants = econstants.map(|name, c| {
        context.restore_unscoped(unscoped.clone());
        constant(context, name, c)
    });
    context.restore_unscoped(unscoped);
    N::ModuleDefinition {
        attributes,
        is_source_module,
        dependency_order,
        friends,
        structs,
        constants,
        functions,
    }
}

fn scripts(
    context: &mut Context,
    escripts: BTreeMap<String, E::Script>,
) -> BTreeMap<String, N::Script> {
    escripts
        .into_iter()
        .map(|(n, s)| (n, script(context, s)))
        .collect()
}

fn script(context: &mut Context, escript: E::Script) -> N::Script {
    let E::Script {
        attributes,
        loc,
        immediate_neighbors: _,
        used_addresses: _,
        constants: econstants,
        function_name,
        function: efunction,
        specs: _specs,
    } = escript;
    let outer_unscoped = context.save_unscoped();
    for (loc, s, _) in &econstants {
        context.bind_constant(s.clone(), loc)
    }
    let inner_unscoped = context.save_unscoped();
    let constants = econstants.map(|name, c| {
        context.restore_unscoped(inner_unscoped.clone());
        constant(context, name, c)
    });
    context.restore_unscoped(inner_unscoped);
    let function = function(context, function_name.clone(), efunction);
    context.restore_unscoped(outer_unscoped);
    N::Script {
        attributes,
        loc,
        constants,
        function_name,
        function,
    }
}

//**************************************************************************************************
// Friends
//**************************************************************************************************

fn friend(context: &mut Context, mident: ModuleIdent, friend: E::Friend) -> Option<E::Friend> {
    let current_mident = context.current_module.as_ref().unwrap();
    if mident.value.address != current_mident.value.address {
        // NOTE: in alignment with the bytecode verifier, this constraint is a policy decision
        // rather than a technical requirement. The compiler, VM, and bytecode verifier DO NOT
        // rely on the assumption that friend modules must reside within the same account address.
        let msg = "Cannot declare modules out of the current address as a friend";
        context.env.add_diag(diag!(
            Declarations::InvalidFriendDeclaration,
            (friend.loc, "Invalid friend declaration"),
            (mident.loc, msg),
        ));
        None
    } else if &mident == current_mident {
        context.env.add_diag(diag!(
            Declarations::InvalidFriendDeclaration,
            (friend.loc, "Invalid friend declaration"),
            (mident.loc, "Cannot declare the module itself as a friend"),
        ));
        None
    } else if context.resolve_module(&mident) {
        Some(friend)
    } else {
        assert!(context.env.has_errors());
        None
    }
}

//**************************************************************************************************
// Functions
//**************************************************************************************************

fn function(context: &mut Context, _name: FunctionName, f: E::Function) -> N::Function {
    let attributes = f.attributes;
    let visibility = f.visibility;
    let signature = function_signature(context, f.signature);
    let acquires = function_acquires(context, f.acquires);
    let body = function_body(context, f.body);
    N::Function {
        attributes,
        visibility,
        signature,
        acquires,
        body,
    }
}

fn function_signature(context: &mut Context, sig: E::FunctionSignature) -> N::FunctionSignature {
    let type_parameters = fun_type_parameters(context, sig.type_parameters);
    let parameters = sig
        .parameters
        .into_iter()
        .map(|(v, ty)| (v, type_(context, ty)))
        .collect();
    let return_type = type_(context, sig.return_type);
    N::FunctionSignature {
        type_parameters,
        parameters,
        return_type,
    }
}

fn function_body(context: &mut Context, sp!(loc, b_): E::FunctionBody) -> N::FunctionBody {
    match b_ {
        E::FunctionBody_::Native => sp(loc, N::FunctionBody_::Native),
        E::FunctionBody_::Defined(es) => sp(loc, N::FunctionBody_::Defined(sequence(context, es))),
    }
}

fn function_acquires(
    context: &mut Context,
    eacquires: Vec<E::ModuleAccess>,
) -> BTreeMap<StructName, Loc> {
    let mut acquires = BTreeMap::new();
    for eacquire in eacquires {
        let new_loc = eacquire.loc;
        let sn = match acquires_type(context, eacquire) {
            None => continue,
            Some(sn) => sn,
        };
        if let Some(old_loc) = acquires.insert(sn, new_loc) {
            context.env.add_diag(diag!(
                Declarations::DuplicateItem,
                (new_loc, "Duplicate acquires item"),
                (old_loc, "Item previously listed here"),
            ))
        }
    }
    acquires
}

fn acquires_type(context: &mut Context, sp!(loc, en_): E::ModuleAccess) -> Option<StructName> {
    use ResolvedType as RT;
    use E::ModuleAccess_ as EN;
    match en_ {
        EN::Name(n) => {
            let case = match context.resolve_unscoped_type(&n)? {
                RT::BuiltinType => "builtin type",
                RT::TParam(_, _) => "type parameter",
            };
            let msg = format!(
                "Invalid acquires item. Expected a struct name, but got a {}",
                case
            );
            context
                .env
                .add_diag(diag!(NameResolution::NamePositionMismatch, (loc, msg)));
            None
        }
        EN::ModuleAccess(m, n) => {
            let (decl_loc, _, abilities, _) = context.resolve_module_type(loc, &m, &n)?;
            acquires_type_struct(context, loc, decl_loc, m, StructName(n), &abilities)
        }
    }
}

fn acquires_type_struct(
    context: &mut Context,
    loc: Loc,
    decl_loc: Loc,
    declared_module: ModuleIdent,
    n: StructName,
    abilities: &AbilitySet,
) -> Option<StructName> {
    let declared_in_current = match &context.current_module {
        Some(current_module) => current_module == &declared_module,
        None => false,
    };

    let mut has_errors = false;

    if !abilities.has_ability_(Ability_::Key) {
        let msg = format!(
            "Invalid acquires item. Expected a struct with the '{}' ability.",
            Ability_::KEY
        );
        let decl_msg = format!("Declared without the '{}' ability here", Ability_::KEY);
        context.env.add_diag(diag!(
            Declarations::InvalidAcquiresItem,
            (loc, msg),
            (decl_loc, decl_msg),
        ));
        has_errors = true;
    }

    if !declared_in_current {
        let tmsg = format!(
            "The struct '{}' was not declared in the current module. Global storage access is \
             internal to the module'",
            n
        );
        context.env.add_diag(diag!(
            Declarations::InvalidAcquiresItem,
            (loc, "Invalid acquires item"),
            (decl_loc, tmsg),
        ));
        has_errors = true;
    }

    if has_errors {
        None
    } else {
        Some(n)
    }
}

//**************************************************************************************************
// Structs
//**************************************************************************************************

fn struct_def(
    context: &mut Context,
    _name: StructName,
    sdef: E::StructDefinition,
) -> N::StructDefinition {
    let attributes = sdef.attributes;
    let abilities = sdef.abilities;
    let type_parameters = struct_type_parameters(context, sdef.type_parameters);
    let fields = struct_fields(context, sdef.fields);
    N::StructDefinition {
        attributes,
        abilities,
        type_parameters,
        fields,
    }
}

fn struct_fields(context: &mut Context, efields: E::StructFields) -> N::StructFields {
    match efields {
        E::StructFields::Native(loc) => N::StructFields::Native(loc),
        E::StructFields::Defined(em) => {
            N::StructFields::Defined(em.map(|_f, (idx, t)| (idx, type_(context, t))))
        }
    }
}

//**************************************************************************************************
// Constants
//**************************************************************************************************

fn constant(context: &mut Context, _name: ConstantName, econstant: E::Constant) -> N::Constant {
    let E::Constant {
        attributes,
        loc,
        signature: esignature,
        value: evalue,
    } = econstant;
    let signature = type_(context, esignature);
    let value = exp_(context, evalue);
    N::Constant {
        attributes,
        loc,
        signature,
        value,
    }
}

//**************************************************************************************************
// Types
//**************************************************************************************************

fn fun_type_parameters(
    context: &mut Context,
    type_parameters: Vec<(Name, AbilitySet)>,
) -> Vec<N::TParam> {
    let mut unique_tparams = UniqueMap::new();
    type_parameters
        .into_iter()
        .map(|(name, abilities)| type_parameter(context, &mut unique_tparams, name, abilities))
        .collect()
}

fn struct_type_parameters(
    context: &mut Context,
    type_parameters: Vec<E::StructTypeParameter>,
) -> Vec<N::StructTypeParameter> {
    let mut unique_tparams = UniqueMap::new();
    type_parameters
        .into_iter()
        .map(|param| {
            let is_phantom = param.is_phantom;
            let param = type_parameter(context, &mut unique_tparams, param.name, param.constraints);
            N::StructTypeParameter { param, is_phantom }
        })
        .collect()
}

fn type_parameter(
    context: &mut Context,
    unique_tparams: &mut UniqueMap<Name, ()>,
    name: Name,
    abilities: AbilitySet,
) -> N::TParam {
    let id = N::TParamID::next();
    let user_specified_name = name.clone();
    let tp = N::TParam {
        id,
        user_specified_name,
        abilities,
    };
    let loc = name.loc;
    context.bind_type(
        name.value.to_string(),
        ResolvedType::TParam(loc, tp.clone()),
    );
    if let Err((name, old_loc)) = unique_tparams.add(name, ()) {
        let msg = format!("Duplicate type parameter declared with name '{}'", name);
        context.env.add_diag(diag!(
            Declarations::DuplicateItem,
            (loc, msg),
            (
                old_loc,
                "Type parameter previously defined here".to_string()
            ),
        ))
    }
    tp
}

fn types(context: &mut Context, tys: Vec<E::Type>) -> Vec<N::Type> {
    tys.into_iter().map(|t| type_(context, t)).collect()
}

fn type_(context: &mut Context, sp!(loc, ety_): E::Type) -> N::Type {
    use ResolvedType as RT;
    use E::{ModuleAccess_ as EN, Type_ as ET};
    use N::{TypeName_ as NN, Type_ as NT};
    let ty_ = match ety_ {
        ET::Unit => NT::Unit,
        ET::Multiple(tys) => {
            NT::multiple_(loc, tys.into_iter().map(|t| type_(context, t)).collect())
        }
        ET::Ref(mut_, inner) => NT::Ref(mut_, Box::new(type_(context, *inner))),
        ET::UnresolvedError => {
            assert!(context.env.has_errors());
            NT::UnresolvedError
        }
        ET::Apply(sp!(_, EN::Name(n)), tys) => match context.resolve_unscoped_type(&n) {
            None => {
                assert!(context.env.has_errors());
                NT::UnresolvedError
            }
            Some(RT::BuiltinType) => {
                let bn_ = N::BuiltinTypeName_::resolve(&n.value).unwrap();
                let name_f = || format!("{}", &bn_);
                let arity = bn_.tparam_constraints(loc).len();
                let tys = types(context, tys);
                let tys = check_type_argument_arity(context, loc, name_f, tys, arity);
                NT::builtin_(sp(loc, bn_), tys)
            }
            Some(RT::TParam(_, tp)) => {
                if !tys.is_empty() {
                    context.env.add_diag(diag!(
                        NameResolution::NamePositionMismatch,
                        (loc, "Generic type parameters cannot take type arguments"),
                    ));
                    NT::UnresolvedError
                } else {
                    NT::Param(tp)
                }
            }
        },
        ET::Apply(sp!(nloc, EN::ModuleAccess(m, n)), tys) => {
            match context.resolve_module_type(nloc, &m, &n) {
                None => {
                    assert!(context.env.has_errors());
                    NT::UnresolvedError
                }
                Some((_, _, _, arity)) => {
                    let tn = sp(nloc, NN::ModuleType(m, StructName(n)));
                    let tys = types(context, tys);
                    let name_f = || format!("{}", tn);
                    let tys = check_type_argument_arity(context, loc, name_f, tys, arity);
                    NT::Apply(None, tn, tys)
                }
            }
        }
        ET::Fun(_, _) => panic!("ICE only allowed in spec context"),
    };
    sp(loc, ty_)
}

fn check_type_argument_arity<F: FnOnce() -> String>(
    context: &mut Context,
    loc: Loc,
    name_f: F,
    mut ty_args: Vec<N::Type>,
    arity: usize,
) -> Vec<N::Type> {
    let args_len = ty_args.len();
    if args_len != arity {
        let diag_code = if args_len > arity {
            NameResolution::TooManyTypeArguments
        } else {
            NameResolution::TooFewTypeArguments
        };
        let msg = format!(
            "Invalid instantiation of '{}'. Expected {} type argument(s) but got {}",
            name_f(),
            arity,
            args_len
        );
        context.env.add_diag(diag!(diag_code, (loc, msg)));
    }

    while ty_args.len() > arity {
        ty_args.pop();
    }

    while ty_args.len() < arity {
        ty_args.push(sp(loc, N::Type_::UnresolvedError))
    }

    ty_args
}

//**************************************************************************************************
// Exp
//**************************************************************************************************

fn sequence(context: &mut Context, seq: E::Sequence) -> N::Sequence {
    seq.into_iter().map(|s| sequence_item(context, s)).collect()
}

fn sequence_item(context: &mut Context, sp!(loc, ns_): E::SequenceItem) -> N::SequenceItem {
    use E::SequenceItem_ as ES;
    use N::SequenceItem_ as NS;

    let s_ = match ns_ {
        ES::Seq(e) => NS::Seq(exp_(context, e)),
        ES::Declare(b, ty_opt) => {
            let bind_opt = bind_list(context, b);
            let tys = ty_opt.map(|t| type_(context, t));
            match bind_opt {
                None => {
                    assert!(context.env.has_errors());
                    NS::Seq(sp(loc, N::Exp_::UnresolvedError))
                }
                Some(bind) => NS::Declare(bind, tys),
            }
        }
        ES::Bind(b, e) => {
            let bind_opt = bind_list(context, b);
            let e = exp_(context, e);
            match bind_opt {
                None => {
                    assert!(context.env.has_errors());
                    NS::Seq(sp(loc, N::Exp_::UnresolvedError))
                }
                Some(bind) => NS::Bind(bind, e),
            }
        }
    };
    sp(loc, s_)
}

fn call_args(context: &mut Context, sp!(loc, es): Spanned<Vec<E::Exp>>) -> Spanned<Vec<N::Exp>> {
    sp(loc, exps(context, es))
}

fn exps(context: &mut Context, es: Vec<E::Exp>) -> Vec<N::Exp> {
    es.into_iter().map(|e| exp_(context, e)).collect()
}

fn exp(context: &mut Context, e: E::Exp) -> Box<N::Exp> {
    Box::new(exp_(context, e))
}

fn exp_(context: &mut Context, e: E::Exp) -> N::Exp {
    use E::Exp_ as EE;
    use N::Exp_ as NE;
    let sp!(eloc, e_) = e;
    let ne_ = match e_ {
        EE::Unit { trailing } => NE::Unit { trailing },
        EE::Value(val) => NE::Value(val),
        EE::Move(v) => NE::Move(v),
        EE::Copy(v) => NE::Copy(v),
        EE::Name(sp!(aloc, E::ModuleAccess_::Name(v)), None) => {
            if is_constant_name(&v.value) {
                access_constant(context, sp(aloc, E::ModuleAccess_::Name(v)))
            } else {
                NE::Use(Var(v))
            }
        }
        EE::Name(ma, None) => access_constant(context, ma),

        EE::IfElse(eb, et, ef) => {
            NE::IfElse(exp(context, *eb), exp(context, *et), exp(context, *ef))
        }
        EE::While(eb, el) => NE::While(exp(context, *eb), exp(context, *el)),
        EE::Loop(el) => NE::Loop(exp(context, *el)),
        EE::Block(seq) => NE::Block(sequence(context, seq)),

        EE::Assign(a, e) => {
            let na_opt = assign_list(context, a);
            let ne = exp(context, *e);
            match na_opt {
                None => {
                    assert!(context.env.has_errors());
                    NE::UnresolvedError
                }
                Some(na) => NE::Assign(na, ne),
            }
        }
        EE::FieldMutate(edotted, er) => {
            let ndot_opt = dotted(context, *edotted);
            let ner = exp(context, *er);
            match ndot_opt {
                None => {
                    assert!(context.env.has_errors());
                    NE::UnresolvedError
                }
                Some(ndot) => NE::FieldMutate(ndot, ner),
            }
        }
        EE::Mutate(el, er) => {
            let nel = exp(context, *el);
            let ner = exp(context, *er);
            NE::Mutate(nel, ner)
        }

        EE::Return(es) => NE::Return(exp(context, *es)),
        EE::Abort(es) => NE::Abort(exp(context, *es)),
        EE::Break => NE::Break,
        EE::Continue => NE::Continue,

        EE::Dereference(e) => NE::Dereference(exp(context, *e)),
        EE::UnaryExp(uop, e) => NE::UnaryExp(uop, exp(context, *e)),
        EE::BinopExp(e1, bop, e2) => NE::BinopExp(exp(context, *e1), bop, exp(context, *e2)),

        EE::Pack(tn, etys_opt, efields) => {
            match context.resolve_struct_name(eloc, "construction", tn, etys_opt) {
                None => {
                    assert!(context.env.has_errors());
                    NE::UnresolvedError
                }
                Some((m, sn, tys_opt)) => NE::Pack(
                    m,
                    sn,
                    tys_opt,
                    efields.map(|_, (idx, e)| (idx, exp_(context, e))),
                ),
            }
        }
        EE::ExpList(es) => {
            assert!(es.len() > 1);
            NE::ExpList(exps(context, es))
        }

        EE::Borrow(mut_, inner) => match *inner {
            sp!(_, EE::ExpDotted(edot)) => match dotted(context, *edot) {
                None => {
                    assert!(context.env.has_errors());
                    NE::UnresolvedError
                }
                Some(d) => NE::Borrow(mut_, d),
            },
            e => {
                let ne = exp(context, e);
                NE::Borrow(mut_, sp(ne.loc, N::ExpDotted_::Exp(ne)))
            }
        },

        EE::ExpDotted(edot) => match dotted(context, *edot) {
            None => {
                assert!(context.env.has_errors());
                NE::UnresolvedError
            }
            Some(d) => NE::DerefBorrow(d),
        },

        EE::Cast(e, t) => NE::Cast(exp(context, *e), type_(context, t)),
        EE::Annotate(e, t) => NE::Annotate(exp(context, *e), type_(context, t)),

        EE::Call(sp!(mloc, ma_), tys_opt, rhs) => {
            use E::ModuleAccess_ as EA;
            let ty_args = tys_opt.map(|tys| types(context, tys));
            let nes = call_args(context, rhs);
            match ma_ {
                EA::Name(n) if N::BuiltinFunction_::all_names().contains(&n.value.as_str()) => {
                    match resolve_builtin_function(context, eloc, &n, ty_args) {
                        None => {
                            assert!(context.env.has_errors());
                            NE::UnresolvedError
                        }
                        Some(f) => NE::Builtin(sp(mloc, f), nes),
                    }
                }

                EA::Name(n) => {
                    context.env.add_diag(diag!(
                        NameResolution::UnboundUnscopedName,
                        (n.loc, format!("Unbound function '{}' in current scope", n)),
                    ));
                    NE::UnresolvedError
                }
                EA::ModuleAccess(m, n) => match context.resolve_module_function(mloc, &m, &n) {
                    None => {
                        assert!(context.env.has_errors());
                        NE::UnresolvedError
                    }
                    Some(_) => NE::ModuleCall(m, FunctionName(n), ty_args, nes),
                },
            }
        }
        EE::Spec(u, unbound_names) => {
            // Vars currently aren't shadowable by types/functions
            let used_locals = unbound_names.into_iter().map(Var).collect();
            NE::Spec(u, used_locals)
        }
        EE::UnresolvedError => {
            assert!(context.env.has_errors());
            NE::UnresolvedError
        }
        // `Name` matches name variants only allowed in specs (we handle the allowed ones above)
        EE::Index(..) | EE::Lambda(..) | EE::Quant(..) | EE::Name(_, Some(_)) => {
            panic!("ICE unexpected specification construct")
        }
    };
    sp(eloc, ne_)
}

fn access_constant(context: &mut Context, ma: E::ModuleAccess) -> N::Exp_ {
    match context.resolve_constant(ma) {
        None => {
            assert!(context.env.has_errors());
            N::Exp_::UnresolvedError
        }
        Some((m, c)) => N::Exp_::Constant(m, c),
    }
}

fn dotted(context: &mut Context, edot: E::ExpDotted) -> Option<N::ExpDotted> {
    let sp!(loc, edot_) = edot;
    let nedot_ = match edot_ {
        E::ExpDotted_::Exp(e) => {
            let ne = exp(context, e);
            match &ne.value {
                N::Exp_::UnresolvedError => return None,
                _ => N::ExpDotted_::Exp(ne),
            }
        }
        E::ExpDotted_::Dot(d, f) => N::ExpDotted_::Dot(Box::new(dotted(context, *d)?), Field(f)),
    };
    Some(sp(loc, nedot_))
}

#[derive(Clone, Copy)]
enum LValueCase {
    Bind,
    Assign,
}

fn lvalue(context: &mut Context, case: LValueCase, sp!(loc, l_): E::LValue) -> Option<N::LValue> {
    use LValueCase as C;
    use E::LValue_ as EL;
    use N::LValue_ as NL;
    let nl_ = match l_ {
        EL::Var(sp!(_, E::ModuleAccess_::Name(n)), None) => {
            let v = Var(n);
            if v.is_underscore() {
                NL::Ignore
            } else {
                NL::Var(v)
            }
        }
        EL::Unpack(tn, etys_opt, efields) => {
            let msg = match case {
                C::Bind => "deconstructing binding",
                C::Assign => "deconstructing assignment",
            };
            let (m, sn, tys_opt) = context.resolve_struct_name(loc, msg, tn, etys_opt)?;
            let nfields = UniqueMap::maybe_from_opt_iter(
                efields
                    .into_iter()
                    .map(|(k, (idx, inner))| Some((k, (idx, lvalue(context, case, inner)?)))),
            )?;
            NL::Unpack(
                m,
                sn,
                tys_opt,
                nfields.expect("ICE fields were already unique"),
            )
        }
        EL::Var(_, _) => panic!("unexpected specification construct"),
    };
    Some(sp(loc, nl_))
}

fn bind_list(context: &mut Context, ls: E::LValueList) -> Option<N::LValueList> {
    lvalue_list(context, LValueCase::Bind, ls)
}

fn assign_list(context: &mut Context, ls: E::LValueList) -> Option<N::LValueList> {
    lvalue_list(context, LValueCase::Assign, ls)
}

fn lvalue_list(
    context: &mut Context,
    case: LValueCase,
    sp!(loc, b_): E::LValueList,
) -> Option<N::LValueList> {
    Some(sp(
        loc,
        b_.into_iter()
            .map(|inner| lvalue(context, case, inner))
            .collect::<Option<_>>()?,
    ))
}

fn resolve_builtin_function(
    context: &mut Context,
    loc: Loc,
    b: &Name,
    ty_args: Option<Vec<N::Type>>,
) -> Option<N::BuiltinFunction_> {
    use N::{BuiltinFunction_ as B, BuiltinFunction_::*};
    Some(match b.value.as_str() {
        B::MOVE_TO => MoveTo(check_builtin_ty_arg(context, loc, b, ty_args)),
        B::MOVE_FROM => MoveFrom(check_builtin_ty_arg(context, loc, b, ty_args)),
        B::BORROW_GLOBAL => BorrowGlobal(false, check_builtin_ty_arg(context, loc, b, ty_args)),
        B::BORROW_GLOBAL_MUT => BorrowGlobal(true, check_builtin_ty_arg(context, loc, b, ty_args)),
        B::EXISTS => Exists(check_builtin_ty_arg(context, loc, b, ty_args)),
        B::FREEZE => Freeze(check_builtin_ty_arg(context, loc, b, ty_args)),
        B::ASSERT => {
            check_builtin_ty_args(context, loc, b, 0, ty_args);
            Assert
        }
        _ => {
            context.env.add_diag(diag!(
                NameResolution::UnboundUnscopedName,
                (b.loc, format!("Unbound function: '{}'", b)),
            ));
            return None;
        }
    })
}

fn check_builtin_ty_arg(
    context: &mut Context,
    loc: Loc,
    b: &Name,
    ty_args: Option<Vec<N::Type>>,
) -> Option<N::Type> {
    let res = check_builtin_ty_args(context, loc, b, 1, ty_args);
    res.map(|mut v| {
        assert!(v.len() == 1);
        v.pop().unwrap()
    })
}

fn check_builtin_ty_args(
    context: &mut Context,
    loc: Loc,
    b: &Name,
    arity: usize,
    ty_args: Option<Vec<N::Type>>,
) -> Option<Vec<N::Type>> {
    ty_args.map(|mut args| {
        let args_len = args.len();
        if args_len != arity {
            let diag_code = if args_len > arity {
                NameResolution::TooManyTypeArguments
            } else {
                NameResolution::TooFewTypeArguments
            };
            context.env.add_diag(diag!(
                diag_code,
                (b.loc, format!("Invalid call to builtin function: '{}'", b)),
                (
                    loc,
                    format!("Expected {} type argument(s) but got {}", arity, args_len),
                ),
            ));
        }

        while args.len() > arity {
            args.pop();
        }

        while args.len() < arity {
            args.push(sp(loc, N::Type_::UnresolvedError));
        }

        args
    })
}
