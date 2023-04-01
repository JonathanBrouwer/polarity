use std::rc::Rc;

use data::HashMap;
use syntax::common::*;
use syntax::ctx::{BindContext, Context, LevelCtx};
use syntax::ust;

#[derive(Debug, Clone)]
pub struct Prg {
    pub map: HashMap<Ident, XData>,
    pub exp: Option<Rc<ust::Exp>>,
}

#[derive(Debug, Clone)]
pub struct XData {
    pub repr: Repr,
    pub info: ust::Info,
    pub doc: Option<DocComment>,
    pub name: Ident,
    pub typ: Rc<ust::TypAbs>,
    pub ctors: HashMap<Ident, ust::Ctor>,
    pub dtors: HashMap<Ident, ust::Dtor>,
    pub exprs: HashMap<Key, Option<Rc<ust::Exp>>>,
}

/// A key points to a matrix cell
///
/// The binding order in the matrix cell is as follors:
/// * dtor telescope
/// * ctor telescope
/// This invariant needs to be handled when translating
/// between the matrix and other representations
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub ctor: Ident,
    pub dtor: Ident,
}

#[derive(Debug, Clone, Copy)]
pub enum Repr {
    Data,
    Codata,
}

/// Take the red pill
pub fn build(prg: &ust::Prg) -> Prg {
    let mut out = Prg { map: HashMap::default(), exp: None };
    let mut ctx = Ctx::empty();
    prg.build_matrix(&mut ctx, &mut out);
    out
}

pub trait BuildMatrix {
    fn build_matrix(&self, ctx: &mut Ctx, out: &mut Prg);
}

pub struct Ctx {
    type_for_xtor: HashMap<Ident, Ident>,
}

impl Ctx {
    pub fn empty() -> Self {
        Self { type_for_xtor: HashMap::default() }
    }
}

impl BuildMatrix for ust::Prg {
    fn build_matrix(&self, ctx: &mut Ctx, out: &mut Prg) {
        let ust::Prg { decls, exp } = self;
        out.exp = exp.clone();

        for decl in decls.map.values() {
            match decl {
                ust::Decl::Data(data) => data.build_matrix(ctx, out),
                ust::Decl::Codata(codata) => codata.build_matrix(ctx, out),
                _ => (),
            }
        }

        for decl in decls.map.values() {
            match decl {
                ust::Decl::Ctor(ctor) => ctor.build_matrix(ctx, out),
                ust::Decl::Dtor(dtor) => dtor.build_matrix(ctx, out),
                ust::Decl::Def(def) => def.build_matrix(ctx, out),
                ust::Decl::Codef(codef) => codef.build_matrix(ctx, out),
                _ => (),
            }
        }
    }
}

impl BuildMatrix for ust::Data {
    fn build_matrix(&self, ctx: &mut Ctx, out: &mut Prg) {
        let ust::Data { info, doc, name, hidden: _, typ, ctors } = self;

        let xdata = XData {
            repr: Repr::Data,
            info: info.clone(),
            doc: doc.clone(),
            name: name.clone(),
            typ: typ.clone(),
            ctors: HashMap::default(),
            dtors: HashMap::default(),
            exprs: HashMap::default(),
        };

        for ctor in ctors {
            ctx.type_for_xtor.insert(ctor.name().clone(), name.clone());
        }

        out.map.insert(name.clone(), xdata);
    }
}

impl BuildMatrix for ust::Codata {
    fn build_matrix(&self, ctx: &mut Ctx, out: &mut Prg) {
        let ust::Codata { info, doc, name, hidden: _, typ, dtors } = self;

        let xdata = XData {
            repr: Repr::Codata,
            info: info.clone(),
            doc: doc.clone(),
            name: name.clone(),
            typ: typ.clone(),
            ctors: HashMap::default(),
            dtors: HashMap::default(),
            exprs: HashMap::default(),
        };

        for dtor in dtors {
            ctx.type_for_xtor.insert(dtor.name().clone(), name.clone());
        }

        out.map.insert(name.clone(), xdata);
    }
}

impl BuildMatrix for ust::Ctor {
    fn build_matrix(&self, ctx: &mut Ctx, out: &mut Prg) {
        let type_name = &ctx.type_for_xtor[&self.name];
        let xdata = out.map.get_mut(type_name).unwrap();
        xdata.ctors.insert(self.name.clone(), self.clone());
    }
}

impl BuildMatrix for ust::Dtor {
    fn build_matrix(&self, ctx: &mut Ctx, out: &mut Prg) {
        let type_name = &ctx.type_for_xtor[&self.name];
        let xdata = out.map.get_mut(type_name).unwrap();
        xdata.dtors.insert(self.name.clone(), self.clone());
    }
}

impl BuildMatrix for ust::Def {
    fn build_matrix(&self, _ctx: &mut Ctx, out: &mut Prg) {
        let type_name = &self.self_param.typ.name;
        let xdata = out.map.get_mut(type_name).unwrap();
        xdata.dtors.insert(self.name.clone(), self.to_dtor());

        let ust::Match { cases, .. } = &self.body;

        for case in cases {
            let ust::Case { name, body, .. } = case;
            let key = Key { dtor: self.name.clone(), ctor: name.clone() };
            xdata.exprs.insert(key, body.clone());
        }
    }
}

impl BuildMatrix for ust::Codef {
    fn build_matrix(&self, _ctx: &mut Ctx, out: &mut Prg) {
        let type_name = &self.typ.name;
        let xdata = out.map.get_mut(type_name).unwrap();
        xdata.ctors.insert(self.name.clone(), self.to_ctor());

        let ust::Comatch { cases, .. } = &self.body;

        for case in cases {
            let ust::Cocase { name, body, .. } = case;
            let key = Key { ctor: self.name.clone(), dtor: name.clone() };
            // Swap binding order to the order imposed by the matrix representation
            let body = body.as_ref().map(|body| {
                let mut ctx = LevelCtx::empty();
                // TODO: Reconsider where to swap this
                ctx.bind_iter(self.params.params.iter().map(|_| ()), |ctx| {
                    ctx.bind_iter(case.params.params.iter().map(|_| ()), |ctx| {
                        body.swap_with_ctx(ctx, 0, 1)
                    })
                })
            });
            xdata.exprs.insert(key, body.clone());
        }
    }
}

impl XData {
    pub fn as_data(&self) -> (ust::Data, Vec<ust::Ctor>, Vec<ust::Def>) {
        let XData { name, doc, typ, ctors, dtors, exprs, .. } = self;

        let data = ust::Data {
            info: ust::Info::empty(),
            doc: doc.clone(),
            name: name.clone(),
            hidden: false,
            typ: typ.clone(),
            ctors: ctors.keys().cloned().collect(),
        };

        let defs = dtors
            .values()
            .map(|dtor| {
                let cases = ctors
                    .values()
                    .map(|ctor| {
                        let key = Key { dtor: dtor.name.clone(), ctor: ctor.name.clone() };
                        ust::Case {
                            info: ust::Info::empty(),
                            name: ctor.name.clone(),
                            args: ctor.params.instantiate(),
                            body: exprs[&key].clone(),
                        }
                    })
                    .collect();

                ust::Def {
                    info: ust::Info::empty(),
                    doc: dtor.doc.clone(),
                    name: dtor.name.clone(),
                    hidden: false,
                    params: dtor.params.clone(),
                    self_param: dtor.self_param.clone(),
                    ret_typ: dtor.ret_typ.clone(),
                    body: ust::Match { cases, info: ust::Info::empty() },
                }
            })
            .collect();

        let ctors = ctors.values().cloned().collect();

        (data, ctors, defs)
    }

    pub fn as_codata(&self) -> (ust::Codata, Vec<ust::Dtor>, Vec<ust::Codef>) {
        let XData { name, doc, typ, ctors, dtors, exprs, .. } = self;

        let codata = ust::Codata {
            info: ust::Info::empty(),
            doc: doc.clone(),
            name: name.clone(),
            hidden: false,
            typ: typ.clone(),
            dtors: dtors.keys().cloned().collect(),
        };

        let codefs = ctors
            .values()
            .map(|ctor| {
                let cases = dtors
                    .values()
                    .map(|dtor| {
                        let key = Key { dtor: dtor.name.clone(), ctor: ctor.name.clone() };
                        let body = &exprs[&key];
                        // Swap binding order (which is different in the matrix representation)
                        let body = body.as_ref().map(|body| {
                            let mut ctx = LevelCtx::empty();
                            ctx.bind_iter(dtor.params.params.iter(), |ctx| {
                                ctx.bind_iter(ctor.params.params.iter(), |ctx| {
                                    body.swap_with_ctx(ctx, 0, 1)
                                })
                            })
                        });
                        ust::Cocase {
                            info: ust::Info::empty(),
                            name: dtor.name.clone(),
                            params: dtor.params.instantiate(),
                            body,
                        }
                    })
                    .collect();

                ust::Codef {
                    info: ust::Info::empty(),
                    doc: ctor.doc.clone(),
                    name: ctor.name.clone(),
                    hidden: false,
                    params: ctor.params.clone(),
                    typ: ctor.typ.clone(),
                    body: ust::Comatch { cases, info: ust::Info::empty() },
                }
            })
            .collect();

        let dtors = dtors.values().cloned().collect();

        (codata, dtors, codefs)
    }
}

trait InstantiateExt {
    fn instantiate(&self) -> ust::TelescopeInst;
}

impl InstantiateExt for ust::Telescope {
    fn instantiate(&self) -> ust::TelescopeInst {
        let params = self
            .params
            .iter()
            .map(|ust::Param { name, .. }| ust::ParamInst {
                name: name.clone(),
                info: ust::Info::empty(),
                typ: (),
            })
            .collect();
        ust::TelescopeInst { params }
    }
}
