//! Loading script modules and tying every call to what it calls.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::catalog::{Builtin, Catalog, Namespace};
use super::fault::{Fault, Location};
use super::parse::parse;
use super::source::{SourceResolver, decode_source, normalize_module};
use super::syntax::{Call, Expr, FnDef, Link, Stmt};

/// Bumped whenever the parsed form changes meaning, so a saved fingerprint
/// from an older engine never matches.
const FORMAT_VERSION: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Site {
    Server,
    Client,
}

/// Which game's script rules a program follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Realm {
    T5,
    T6,
}

/// One loaded module and the hash of the bytes it was read from.
#[derive(Clone, Debug)]
pub struct ModuleIdentity {
    pub site: Site,
    pub realm: Realm,
    pub module: String,
    pub sha256: [u8; 32],
}

/// Every function of the loaded modules, linked and ready to run.
#[derive(Clone, Debug)]
pub struct Program {
    functions: Vec<FnDef>,
    by_name: BTreeMap<String, usize>,
    modules: Vec<ModuleIdentity>,
    natives: Vec<Builtin>,
    realm: Realm,
}

/// A fault at the very start of `module`, for problems with the module as a
/// whole rather than with a line in it.
fn module_fault(module: &str, message: impl Into<String>) -> Fault {
    let start = Location {
        module: module.to_owned(),
        function: String::new(),
        line: 1,
        column: 1,
    };
    Fault::at(&start, message)
}

fn loader_fault(message: impl Into<String>) -> Fault {
    module_fault("<loader>", message)
}

impl Program {
    /// Read `roots` (and nothing else; scripts cannot pull in other modules)
    /// and link their calls against `catalog`.
    pub fn load(
        resolver: &dyn SourceResolver,
        roots: &[&str],
        catalog: &Catalog,
    ) -> Result<Program, Fault> {
        if roots.is_empty() {
            return Err(loader_fault("no script roots"));
        }
        let wanted = roots
            .iter()
            .map(|root| normalize_module(root).map_err(loader_fault))
            .collect::<Result<BTreeSet<_>, _>>()?;

        let mut functions = Vec::new();
        let mut by_name = BTreeMap::new();
        let mut modules = Vec::new();
        for module in &wanted {
            let bytes = resolver
                .read_bytes(module)
                .map_err(|message| module_fault(module, message))?;
            modules.push(ModuleIdentity {
                site: Site::Server,
                realm: catalog.realm(),
                module: module.clone(),
                sha256: Sha256::digest(&bytes).into(),
            });
            for def in parse(module, &decode_source(&bytes))? {
                let key = format!("{module}::{}", def.name);
                if by_name.insert(key, functions.len()).is_some() {
                    let spot = Location {
                        module: module.clone(),
                        function: def.name.clone(),
                        line: def.pos.line,
                        column: def.pos.column,
                    };
                    return Err(Fault::at(&spot, "duplicate function"));
                }
                functions.push(def);
            }
        }

        let mut linker = Linker {
            by_name: &by_name,
            catalog,
            natives: Vec::new(),
        };
        for def in &mut functions {
            let module = def.module.clone();
            let function = def.name.clone();
            let scope = Scope { module: &module, function: &function };
            def.body
                .iter_mut()
                .try_for_each(|stmt| linker.stmt(&scope, stmt))?;
        }
        let natives = linker.natives;

        Ok(Program {
            functions,
            by_name,
            modules,
            natives,
            realm: catalog.realm(),
        })
    }

    pub fn modules(&self) -> &[ModuleIdentity] {
        &self.modules
    }

    pub fn rules(&self) -> Realm {
        self.realm
    }

    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    pub fn native_count(&self) -> usize {
        self.natives.len()
    }

    /// The engine commands this program calls, in the order `Link::Native`
    /// numbers them.
    pub fn natives(&self) -> &[Builtin] {
        &self.natives
    }

    pub fn function(&self, index: usize) -> &FnDef {
        &self.functions[index]
    }

    /// Find `module::name` the way a person would type it.
    pub fn find(&self, entry: &str) -> Option<usize> {
        self.by_name
            .get(&entry.replace('\\', "/").to_ascii_lowercase())
            .copied()
    }

    /// Identifies exactly which script text was loaded.
    pub fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(FORMAT_VERSION.to_le_bytes());
        for module in &self.modules {
            hash.update([module.site as u8, module.realm as u8]);
            hash.update((module.module.len() as u64).to_le_bytes());
            hash.update(module.module.as_bytes());
            hash.update(module.sha256);
        }
        hash.finalize().into()
    }
}

struct Scope<'a> {
    module: &'a str,
    function: &'a str,
}

struct Linker<'a> {
    by_name: &'a BTreeMap<String, usize>,
    catalog: &'a Catalog,
    natives: Vec<Builtin>,
}

impl Linker<'_> {
    fn stmt(&mut self, scope: &Scope, stmt: &mut Stmt) -> Result<(), Fault> {
        match stmt {
            Stmt::Eval(expr) => self.expr(scope, expr),
            Stmt::Assign { target, value, .. } => {
                self.expr(scope, target)?;
                self.expr(scope, value)
            }
            Stmt::If { cond, then, otherwise } => {
                self.expr(scope, cond)?;
                self.stmt(scope, then)?;
                otherwise.as_mut().map_or(Ok(()), |other| self.stmt(scope, other))
            }
            Stmt::While { cond, body } => {
                self.expr(scope, cond)?;
                self.stmt(scope, body)
            }
            Stmt::For { init, cond, step, body } => {
                init.as_mut().map_or(Ok(()), |s| self.stmt(scope, s))?;
                cond.as_mut().map_or(Ok(()), |e| self.expr(scope, e))?;
                step.as_mut().map_or(Ok(()), |s| self.stmt(scope, s))?;
                self.stmt(scope, body)
            }
            Stmt::Return(value) => value.as_mut().map_or(Ok(()), |e| self.expr(scope, e)),
            Stmt::Break | Stmt::Continue => Ok(()),
            Stmt::Block(list) => list.iter_mut().try_for_each(|s| self.stmt(scope, s)),
        }
    }

    fn expr(&mut self, scope: &Scope, expr: &mut Expr) -> Result<(), Fault> {
        match expr {
            Expr::Lit(_) | Expr::Local(_) | Expr::Me | Expr::Root(_) | Expr::NewArray => Ok(()),
            Expr::Vector(parts) => parts.iter_mut().try_for_each(|e| self.expr(scope, e)),
            Expr::Field(inner, ..) | Expr::Not(inner) | Expr::Neg(inner, _) => {
                self.expr(scope, inner)
            }
            Expr::Index(left, right, _) | Expr::Binary(_, left, right, _) => {
                self.expr(scope, left)?;
                self.expr(scope, right)
            }
            Expr::Call(call) => self.call(scope, call),
        }
    }

    fn call(&mut self, scope: &Scope, call: &mut Call) -> Result<(), Fault> {
        if let Some(receiver) = &mut call.receiver {
            self.expr(scope, receiver)?;
        }
        call.args
            .iter_mut()
            .try_for_each(|arg| self.expr(scope, arg))?;
        let spot = Location {
            module: scope.module.to_owned(),
            function: scope.function.to_owned(),
            line: call.pos.line,
            column: call.pos.column,
        };

        if let Some(module) = &call.module {
            let key = format!("{}::{}", module.replace('\\', "/"), call.name);
            let found = self.by_name.get(&key);
            call.link = Link::Script(*found.ok_or_else(|| {
                Fault::at(&spot, format!("unresolved function {module}::{}", call.name))
            })?);
            return Ok(());
        }
        if let Some(&local) = self.by_name.get(&format!("{}::{}", scope.module, call.name)) {
            call.link = Link::Script(local);
            return Ok(());
        }
        if call.receiver.is_some() && call.name == "notify" {
            call.link = Link::Notify;
            return Ok(());
        }
        let namespace = if call.receiver.is_some() { Namespace::Method } else { Namespace::Function };
        let Some(builtin) = self.catalog.get(namespace, &call.name) else {
            let what = if call.receiver.is_some() { "method" } else { "function" };
            return Err(Fault::at(
                &spot,
                format!("unresolved {what} or unknown builtin {}", call.name),
            ));
        };
        if call.args.len() >= 256 {
            return Err(Fault::at(&spot, "parameter count exceeds 256"));
        }
        let slot = self
            .natives
            .iter()
            .position(|known| *known == builtin)
            .unwrap_or_else(|| {
                self.natives.push(builtin);
                self.natives.len() - 1
            });
        call.link = Link::Native(slot);
        Ok(())
    }
}
