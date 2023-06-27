use std::rc::Rc;

use num_bigint::BigUint;

use miette_util::ToMiette;
use parser::cst;
use parser::cst::BindingSite;
use syntax::common::*;
use syntax::ctx::{BindContext, Context};
use syntax::generic::lookup_table;
use syntax::ust;

use super::ctx::*;
use super::result::*;
use super::types::*;

pub fn lower(prg: &cst::Prg) -> Result<ust::Prg, LoweringError> {
    let cst::Prg { items, exp } = prg;
    let mut ctx = Ctx::empty();

    // Register names and metadata
    register_names(&mut ctx, &items[..])?;
    let lookup_table = build_lookup_table(items);

    // Lower definitions
    for item in items {
        item.lower_in_ctx(&mut ctx)?;
    }

    let exp = exp.lower_in_ctx(&mut ctx)?;

    Ok(ust::Prg { decls: ctx.into_decls(lookup_table), exp })
}

/// Register names for all top-level declarations
/// Returns definitions whose lowering has been deferred
fn register_names(ctx: &mut Ctx, items: &[cst::Item]) -> Result<(), LoweringError> {
    for item in items {
        match item {
            cst::Item::Data(data) => {
                ctx.add_top_level_decl(&data.name, DeclMeta::from(data))?;
                for ctor in &data.ctors {
                    ctx.add_top_level_decl(
                        &ctor.name,
                        DeclMeta::Ctor { ret_typ: data.name.clone() },
                    )?;
                }
            }
            cst::Item::Codata(codata) => {
                ctx.add_top_level_decl(&codata.name, DeclMeta::from(codata))?;
                for dtor in &codata.dtors {
                    ctx.add_top_level_decl(
                        &dtor.name,
                        DeclMeta::Dtor { self_typ: codata.name.clone() },
                    )?;
                }
            }
            cst::Item::Def(def) => {
                ctx.add_top_level_decl(&def.name, DeclMeta::from(def))?;
            }
            cst::Item::Codef(codef) => {
                ctx.add_top_level_decl(&codef.name, DeclMeta::from(codef))?;
            }
        }
    }

    Ok(())
}

/// Build the structure tracking the declaration order in the source code
fn build_lookup_table(items: &[cst::Item]) -> lookup_table::LookupTable {
    let mut lookup_table = lookup_table::LookupTable::default();

    for item in items {
        match item {
            cst::Item::Data(data) => {
                let mut typ_decl = lookup_table.add_type_decl(data.name.clone());
                let xtors = data.ctors.iter().map(|ctor| ctor.name.clone());
                typ_decl.set_xtors(xtors);
            }
            cst::Item::Codata(codata) => {
                let mut typ_decl = lookup_table.add_type_decl(codata.name.clone());
                let xtors = codata.dtors.iter().map(|ctor| ctor.name.clone());
                typ_decl.set_xtors(xtors);
            }
            cst::Item::Def(def) => {
                let type_name = def.scrutinee.typ.name.clone();
                lookup_table.add_def(type_name, def.name.to_owned());
            }
            cst::Item::Codef(codef) => {
                let type_name = codef.typ.name.clone();
                lookup_table.add_def(type_name, codef.name.to_owned())
            }
        }
    }

    lookup_table
}

impl Lower for cst::Item {
    type Target = ();

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let decl = match self {
            cst::Item::Data(data) => ust::Decl::Data(data.lower_in_ctx(ctx)?),
            cst::Item::Codata(codata) => ust::Decl::Codata(codata.lower_in_ctx(ctx)?),
            cst::Item::Def(def) => ust::Decl::Def(def.lower_in_ctx(ctx)?),
            cst::Item::Codef(codef) => ust::Decl::Codef(codef.lower_in_ctx(ctx)?),
        };
        ctx.add_decl(decl)?;
        Ok(())
    }
}

impl Lower for cst::Data {
    type Target = ust::Data;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Data { info, doc, name, hidden, params, ctors } = self;

        let ctor_decls = ctors.lower_in_ctx(ctx)?.into_iter().map(ust::Decl::Ctor);

        let ctor_names = ctors.iter().map(|ctor| ctor.name.clone()).collect();

        ctx.add_decls(ctor_decls)?;

        Ok(ust::Data {
            info: Some(*info),
            doc: doc.clone(),
            name: name.clone(),
            hidden: *hidden,
            typ: Rc::new(ust::TypAbs { params: params.lower_in_ctx(ctx)? }),
            ctors: ctor_names,
        })
    }
}

impl Lower for cst::Codata {
    type Target = ust::Codata;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Codata { info, doc, name, hidden, params, dtors } = self;

        let dtor_decls = dtors.lower_in_ctx(ctx)?.into_iter().map(ust::Decl::Dtor);

        let dtor_names = dtors.iter().map(|dtor| dtor.name.clone()).collect();

        ctx.add_decls(dtor_decls)?;

        Ok(ust::Codata {
            info: Some(*info),
            doc: doc.clone(),
            name: name.clone(),
            hidden: *hidden,
            typ: Rc::new(ust::TypAbs { params: params.lower_in_ctx(ctx)? }),
            dtors: dtor_names,
        })
    }
}

impl Lower for cst::Ctor {
    type Target = ust::Ctor;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Ctor { info, doc, name, params, typ } = self;

        let typ_name = match ctx.lookup_top_level_decl(name, info)? {
            DeclMeta::Ctor { ret_typ } => ret_typ,
            other => {
                return Err(LoweringError::InvalidDeclarationKind {
                    name: name.clone(),
                    expected: DeclKind::Ctor,
                    actual: other.kind(),
                })
            }
        };

        let type_arity = match ctx.lookup_top_level_decl(&typ_name, info)? {
            DeclMeta::Data { arity } => arity,
            other => {
                return Err(LoweringError::InvalidDeclarationKind {
                    name: name.clone(),
                    expected: DeclKind::Data,
                    actual: other.kind(),
                })
            }
        };

        params.lower_telescope(ctx, |ctx, params| {
            // If the type constructor does not take any arguments, it can be left out
            let typ = match typ {
                Some(typ) => typ.lower_in_ctx(ctx)?,
                None => {
                    if type_arity == 0 {
                        ust::TypApp {
                            info: None,
                            name: typ_name.clone(),
                            args: ust::Args { args: vec![] },
                        }
                    } else {
                        return Err(LoweringError::MustProvideArgs {
                            xtor: name.clone(),
                            typ: typ_name.clone(),
                            span: info.to_miette(),
                        });
                    }
                }
            };

            Ok(ust::Ctor { info: Some(*info), doc: doc.clone(), name: name.clone(), params, typ })
        })
    }
}

impl Lower for cst::Dtor {
    type Target = ust::Dtor;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Dtor { info, doc, name, params, destructee, ret_typ } = self;

        let typ_name = match ctx.lookup_top_level_decl(name, info)? {
            DeclMeta::Dtor { self_typ } => self_typ,
            other => {
                return Err(LoweringError::InvalidDeclarationKind {
                    name: name.clone(),
                    expected: DeclKind::Dtor,
                    actual: other.kind(),
                })
            }
        };

        let type_arity = match ctx.lookup_top_level_decl(&typ_name, info)? {
            DeclMeta::Codata { arity } => arity,
            other => {
                return Err(LoweringError::InvalidDeclarationKind {
                    name: name.clone(),
                    expected: DeclKind::Codata,
                    actual: other.kind(),
                })
            }
        };

        params.lower_telescope(ctx, |ctx, params| {
            // If the type constructor does not take any arguments, it can be left out
            let on_typ = match &destructee.typ {
                Some(on_typ) => on_typ.clone(),
                None => {
                    if type_arity == 0 {
                        cst::TypApp {
                            info: Default::default(),
                            name: typ_name.clone(),
                            args: vec![],
                        }
                    } else {
                        return Err(LoweringError::MustProvideArgs {
                            xtor: name.clone(),
                            typ: typ_name.clone(),
                            span: info.to_miette(),
                        });
                    }
                }
            };

            let self_param = cst::SelfParam {
                info: destructee.info,
                name: destructee.name.clone(),
                typ: on_typ,
            };

            self_param.lower_telescope(ctx, |ctx, self_param| {
                Ok(ust::Dtor {
                    info: Some(*info),
                    doc: doc.clone(),
                    name: name.clone(),
                    params,
                    self_param,
                    ret_typ: ret_typ.lower_in_ctx(ctx)?,
                })
            })
        })
    }
}

impl Lower for cst::Def {
    type Target = ust::Def;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Def { info, doc, name, hidden, params, scrutinee, ret_typ, body } = self;

        let self_param: cst::SelfParam = scrutinee.clone().into();

        params.lower_telescope(ctx, |ctx, params| {
            let body = body.lower_in_ctx(ctx)?;

            self_param.lower_telescope(ctx, |ctx, self_param| {
                Ok(ust::Def {
                    info: Some(*info),
                    doc: doc.clone(),
                    name: name.clone(),
                    hidden: *hidden,
                    params,
                    self_param,
                    ret_typ: ret_typ.lower_in_ctx(ctx)?,
                    body,
                })
            })
        })
    }
}

impl Lower for cst::Codef {
    type Target = ust::Codef;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Codef { info, doc, name, hidden, params, typ, body, .. } = self;

        params.lower_telescope(ctx, |ctx, params| {
            Ok(ust::Codef {
                info: Some(*info),
                doc: doc.clone(),
                name: name.clone(),
                hidden: *hidden,
                params,
                typ: typ.lower_in_ctx(ctx)?,
                body: body.lower_in_ctx(ctx)?,
            })
        })
    }
}

impl Lower for cst::Match {
    type Target = ust::Match;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Match { info, cases, omit_absurd } = self;

        Ok(ust::Match {
            info: Some(*info),
            cases: cases.lower_in_ctx(ctx)?,
            omit_absurd: *omit_absurd,
        })
    }
}

impl Lower for cst::Comatch {
    type Target = ust::Comatch;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Comatch { info, cases, omit_absurd } = self;

        Ok(ust::Comatch {
            info: Some(*info),
            cases: cases.lower_in_ctx(ctx)?,
            omit_absurd: *omit_absurd,
        })
    }
}

impl Lower for cst::Case {
    type Target = ust::Case;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Case { info, name, args, body } = self;

        args.lower_telescope(ctx, |ctx, args| {
            Ok(ust::Case {
                info: Some(*info),
                name: name.clone(),
                args,
                body: body.lower_in_ctx(ctx)?,
            })
        })
    }
}

impl Lower for cst::Cocase {
    type Target = ust::Cocase;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Cocase { info, name, args, body } = self;

        args.lower_telescope(ctx, |ctx, args| {
            Ok(ust::Cocase {
                info: Some(*info),
                name: name.clone(),
                params: args,
                body: body.lower_in_ctx(ctx)?,
            })
        })
    }
}

impl Lower for cst::TypApp {
    type Target = ust::TypApp;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::TypApp { info, name, args } = self;

        Ok(ust::TypApp {
            info: Some(*info),
            name: name.clone(),
            args: ust::Args { args: args.lower_in_ctx(ctx)? },
        })
    }
}

impl Lower for cst::Exp {
    type Target = ust::Exp;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        match self {
            cst::Exp::Call { info, name, args } => match ctx.lookup(name, info)? {
                Elem::Bound(lvl) => Ok(ust::Exp::Var {
                    info: Some(*info),
                    name: name.clone(),
                    ctx: (),
                    idx: ctx.level_to_index(lvl),
                }),
                Elem::Decl(meta) => match meta.kind() {
                    DeclKind::Data | DeclKind::Codata => Ok(ust::Exp::TypCtor {
                        info: Some(*info),
                        name: name.to_owned(),
                        args: ust::Args { args: args.lower_in_ctx(ctx)? },
                    }),
                    DeclKind::Def | DeclKind::Dtor => Err(LoweringError::MustUseAsDtor {
                        name: name.to_owned(),
                        span: info.to_miette(),
                    }),
                    DeclKind::Codef | DeclKind::Ctor => Ok(ust::Exp::Ctor {
                        info: Some(*info),
                        name: name.to_owned(),
                        args: ust::Args { args: args.lower_in_ctx(ctx)? },
                    }),
                },
            },
            cst::Exp::DotCall { info, exp, name, args } => match ctx.lookup(name, info)? {
                Elem::Bound(_) => Err(LoweringError::CannotUseAsDtor {
                    name: name.clone(),
                    span: info.to_miette(),
                }),
                Elem::Decl(meta) => match meta.kind() {
                    DeclKind::Def | DeclKind::Dtor => Ok(ust::Exp::Dtor {
                        info: Some(*info),
                        exp: exp.lower_in_ctx(ctx)?,
                        name: name.clone(),
                        args: ust::Args { args: args.lower_in_ctx(ctx)? },
                    }),
                    _ => Err(LoweringError::CannotUseAsDtor {
                        name: name.clone(),
                        span: info.to_miette(),
                    }),
                },
            },
            cst::Exp::Anno { info, exp, typ } => Ok(ust::Exp::Anno {
                info: Some(*info),
                exp: exp.lower_in_ctx(ctx)?,
                typ: typ.lower_in_ctx(ctx)?,
            }),
            cst::Exp::Type { info } => Ok(ust::Exp::Type { info: Some(*info) }),
            cst::Exp::Match { info, name, on_exp, motive, body } => Ok(ust::Exp::Match {
                info: Some(*info),
                ctx: (),
                name: ctx.unique_label(name.to_owned(), info)?,
                on_exp: on_exp.lower_in_ctx(ctx)?,
                motive: motive.lower_in_ctx(ctx)?,
                ret_typ: (),
                body: body.lower_in_ctx(ctx)?,
            }),
            cst::Exp::Comatch { info, name, is_lambda_sugar, body } => Ok(ust::Exp::Comatch {
                info: Some(*info),
                ctx: (),
                name: ctx.unique_label(name.to_owned(), info)?,
                is_lambda_sugar: *is_lambda_sugar,
                body: body.lower_in_ctx(ctx)?,
            }),
            cst::Exp::Hole { info, kind } => Ok(ust::Exp::Hole { info: Some(*info), kind: *kind }),
            cst::Exp::NatLit { info, val } => {
                let mut out = ust::Exp::Ctor {
                    info: Some(*info),
                    name: "Z".to_owned(),
                    args: ust::Args { args: vec![] },
                };

                let mut i = BigUint::from(0usize);

                while &i != val {
                    i += 1usize;
                    out = ust::Exp::Ctor {
                        info: Some(*info),
                        name: "S".to_owned(),
                        args: ust::Args { args: vec![Rc::new(out)] },
                    };
                }

                Ok(out)
            }
            cst::Exp::Fun { info, from, to } => Ok(ust::Exp::TypCtor {
                info: Some(*info),
                name: "Fun".to_owned(),
                args: ust::Args { args: vec![from.lower_in_ctx(ctx)?, to.lower_in_ctx(ctx)?] },
            }),
            cst::Exp::Lam { info, var, body } => {
                let comatch = cst::Exp::Comatch {
                    info: *info,
                    name: None,
                    is_lambda_sugar: true,
                    body: cst::Comatch {
                        info: *info,
                        cases: vec![cst::Cocase {
                            info: *info,
                            name: "ap".to_owned(),
                            args: cst::TelescopeInst(vec![
                                cst::ParamInst {
                                    info: Default::default(),
                                    name: BindingSite::Wildcard,
                                },
                                cst::ParamInst {
                                    info: Default::default(),
                                    name: BindingSite::Wildcard,
                                },
                                var.clone(),
                            ]),
                            body: Some(body.clone()),
                        }],
                        omit_absurd: false,
                    },
                };
                comatch.lower_in_ctx(ctx)
            }
        }
    }
}

impl Lower for cst::Motive {
    type Target = ust::Motive;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        let cst::Motive { info, param, ret_typ } = self;

        Ok(ust::Motive {
            info: Some(*info),
            param: ust::ParamInst { info: Some(param.info), name: param.name().clone(), typ: () },
            ret_typ: ctx.bind_single(param, |ctx| ret_typ.lower_in_ctx(ctx))?,
        })
    }
}

impl<T: Lower> Lower for Option<T> {
    type Target = Option<T::Target>;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        self.as_ref().map(|x| x.lower_in_ctx(ctx)).transpose()
    }
}

impl<T: Lower> Lower for Vec<T> {
    type Target = Vec<T::Target>;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        self.iter().map(|x| x.lower_in_ctx(ctx)).collect()
    }
}

impl<T: Lower> Lower for Rc<T> {
    type Target = Rc<T::Target>;

    fn lower_in_ctx(&self, ctx: &mut Ctx) -> Result<Self::Target, LoweringError> {
        Ok(Rc::new((**self).lower_in_ctx(ctx)?))
    }
}

impl LowerTelescope for cst::SelfParam {
    type Target = ust::SelfParam;

    fn lower_telescope<T, F: FnOnce(&mut Ctx, Self::Target) -> Result<T, LoweringError>>(
        &self,
        ctx: &mut Ctx,
        f: F,
    ) -> Result<T, LoweringError> {
        let cst::SelfParam { info, name, typ } = self;
        let typ_out = typ.lower_in_ctx(ctx)?;
        ctx.bind_single(name.clone().unwrap_or_default(), |ctx| {
            f(ctx, ust::SelfParam { info: Some(*info), name: name.clone(), typ: typ_out })
        })
    }
}

fn desugar_telescope(tel: &cst::Telescope) -> cst::Telescope {
    let cst::Telescope(params) = tel;
    let params: Vec<cst::Param> = params.iter().flat_map(desugar_param).collect();
    cst::Telescope(params)
}
fn desugar_param(param: &cst::Param) -> Vec<cst::Param> {
    let cst::Param { name, names, typ } = param;
    let mut params: Vec<cst::Param> =
        vec![cst::Param { name: name.clone(), names: vec![], typ: typ.clone() }];
    for extra_name in names {
        params.push(cst::Param { name: extra_name.clone(), names: vec![], typ: typ.clone() });
    }
    params
}

impl LowerTelescope for cst::Telescope {
    type Target = ust::Telescope;

    /// Lower a telescope
    ///
    /// Execute a function `f` under the context where all binders
    /// of the telescope are in scope.
    fn lower_telescope<T, F>(&self, ctx: &mut Ctx, f: F) -> Result<T, LoweringError>
    where
        F: FnOnce(&mut Ctx, Self::Target) -> Result<T, LoweringError>,
    {
        let tel = desugar_telescope(self);
        ctx.bind_fold(
            tel.0.iter(),
            Ok(vec![]),
            |ctx, params_out, param| {
                let mut params_out = params_out?;
                let cst::Param { name, names: _, typ } = param; // The `names` field has been removed by `desugar_telescope`.
                let typ_out = typ.lower_in_ctx(ctx)?;
                let name = match name {
                    BindingSite::Var { name } => name.clone(),
                    BindingSite::Wildcard => "_".to_owned(),
                };
                let param_out = ust::Param { name, typ: typ_out };
                params_out.push(param_out);
                Ok(params_out)
            },
            |ctx, params| f(ctx, params.map(|params| ust::Telescope { params })?),
        )
    }
}

impl LowerTelescope for cst::TelescopeInst {
    type Target = ust::TelescopeInst;

    fn lower_telescope<T, F: FnOnce(&mut Ctx, Self::Target) -> Result<T, LoweringError>>(
        &self,
        ctx: &mut Ctx,
        f: F,
    ) -> Result<T, LoweringError> {
        ctx.bind_fold(
            self.0.iter(),
            Ok(vec![]),
            |_ctx, params_out, param| {
                let mut params_out = params_out?;
                let cst::ParamInst { info, name } = param;
                let param_out =
                    ust::ParamInst { info: Some(*info), name: name.name().clone(), typ: () };
                params_out.push(param_out);
                Ok(params_out)
            },
            |ctx, params| f(ctx, params.map(|params| ust::TelescopeInst { params })?),
        )
    }
}
