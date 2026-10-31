//! Script values.
//!
//! Strings are interned (`Str`): field names, notify names and string
//! constants compare by id. Arrays have value semantics: assigning or passing
//! one copies it, done here as a shared object copied on a script's first
//! write while shared (as the engine's refcounted arrays are); the engine's
//! own array functions (`arrayremovevalue`, `arrayinsert`, ...) change the
//! shared object in place, which scripts rely on. Structs,
//! entities and the `level`/`anim` objects are references into the VM's
//! object table (`ObjRef`, with a stamp so a deleted entity reads as
//! undefined).

use std::sync;

use indexmap::IndexMap;
use rustc_hash::{FxBuildHasher, FxHashMap};

/// An interned string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Str(pub u32);

/// The string table. Every id stays valid for the VM's life.
#[derive(Default, Debug)]
pub struct Strings {
    list: Vec<sync::Arc<str>>,
    map: FxHashMap<sync::Arc<str>, Str>,
}

impl Strings {
    pub fn intern(&mut self, s: &str) -> Str {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = Str(self.list.len() as u32);
        let rc: sync::Arc<str> = sync::Arc::from(s);
        self.list.push(rc.clone());
        self.map.insert(rc, id);
        id
    }

    pub fn find(&self, text: &str) -> Option<Str> {
        self.map.get(text).copied()
    }

    pub fn get(&self, handle: Str) -> &str {
        self.list.get(handle.0 as usize).map_or("", |s| s)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// A reference to an object (struct, entity, level, anim, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjRef {
    pub index: u32,
    pub generation: u32,
}

/// A function value: a script function, a builtin, or a name nothing defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FuncRef {
    Script(u32),
    Builtin(u32),
    Missing(Str),
}

/// An array key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i32),
    Str(Str),
    /// Entities and structs can key an array in Black Ops II.
    Obj(ObjRef),
}

impl Key {
    pub fn value(self) -> ScriptValue {
        match self {
            Key::Int(i) => ScriptValue::Int(i),
            Key::Str(s) => ScriptValue::Str(s),
            Key::Obj(o) => ScriptValue::Object(o),
        }
    }
}

/// Array storage: insertion order kept (a write to a key it has keeps its
/// place). As in BO2's engine (Scr_FirstArrayKey / Scr_NextArrayKey walk
/// from the oldest; Scr_AddArrayKeys and getlastarraykey start at the
/// newest): `foreach`, `getfirstarraykey` and `getnextarraykey` run oldest
/// first, `getarraykeys` lists newest first. (Die Rise's tower legs and
/// Buried's sloth table wait in the order their arrays were built.)
pub type ArrayMap = IndexMap<Key, ScriptValue, FxBuildHasher>;

#[derive(Clone, Debug, Default)]
pub struct Array {
    pub map: ArrayMap,
}

impl Array {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, key: &Key) -> Option<&ScriptValue> {
        self.map.get(key)
    }

    /// Store; undefined removes the key (order of the rest kept).
    pub fn set(&mut self, key: Key, val: ScriptValue) {
        if matches!(val, ScriptValue::Undefined) {
            self.map.shift_remove(&key);
        } else {
            self.map.insert(key, val);
        }
    }

    pub fn push(&mut self, val: ScriptValue) {
        let next_key = Key::Int(self.map.len() as i32);
        self.set(next_key, val);
    }

    /// The first key `foreach` visits (the oldest).
    pub fn first_key(&self) -> Option<Key> {
        self.map.first().map(|(k, _)| *k)
    }

    /// The key `foreach` visits after `k` (the next newer one).
    pub fn next_key(&self, k: &Key) -> Option<Key> {
        let i = self.map.get_index_of(k)?;
        self.map.get_index(i + 1).map(|(k, _)| *k)
    }

    /// The newest key (`getlastarraykey`).
    pub fn last_key(&self) -> Option<Key> {
        self.map.last().map(|(k, _)| *k)
    }

    /// Keys as `getarraykeys` lists them: newest first.
    pub fn keys(&self) -> Box<dyn Iterator<Item = Key> + '_> {
        Box::new(self.map.keys().rev().copied())
    }

    /// Values in index order 0..n when the array is a plain list, else in
    /// insertion order.
    pub fn values_in_order(&self) -> impl Iterator<Item = &ScriptValue> + '_ {
        self.map.values()
    }
}

/// A script array: shared, copied on a script write while shared.
#[derive(Clone, Debug, Default)]
pub struct ArrayRef(sync::Arc<sync::RwLock<Array>>);

impl ArrayRef {
    pub fn new(a: Array) -> Self {
        Self(sync::Arc::new(sync::RwLock::new(a)))
    }

    /// Read it (never hold the guard across a call back into the VM).
    pub fn read(&self) -> sync::RwLockReadGuard<'_, Array> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Change it in place, for everyone sharing it (the engine's array
    /// functions do this).
    pub fn write(&self) -> sync::RwLockWriteGuard<'_, Array> {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// A copy of the contents.
    pub fn snapshot(&self) -> Array {
        self.read().clone()
    }

    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        sync::Arc::ptr_eq(&a.0, &b.0)
    }

    /// Before a script writes: a private copy when shared.
    pub fn make_unique(&mut self) {
        if sync::Arc::strong_count(&self.0) > 1 {
            *self = Self::new(self.snapshot());
        }
    }

    pub fn len(&self) -> usize {
        self.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.read().is_empty()
    }

    pub fn get(&self, key: &Key) -> Option<ScriptValue> {
        self.read().get(key).cloned()
    }
}

/// A script value.
#[derive(Clone, Debug, Default)]
pub enum ScriptValue {
    #[default]
    Undefined,
    Int(i32),
    Float(f32),
    Str(Str),
    /// A localized string reference (`&"..."`).
    IStr(Str),
    Hash(u32),
    Vec3([f32; 3]),
    Object(ObjRef),
    Array(ArrayRef),
    Func(FuncRef),
    /// An animation (`%anim`): index into the program's animation table.
    Anim(u32),
    /// An animation tree (`#animtree`): index into the program's tree table.
    AnimTree(u32),
    /// Internal: the argument boundary pushed before a call or a notify.
    Marker,
}

impl ScriptValue {
    pub fn array(a: Array) -> ScriptValue {
        ScriptValue::Array(ArrayRef::new(a))
    }

    pub fn is_undefined(&self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(&Self::Undefined)
    }

    pub fn as_int(&self) -> Option<i32> {
        match *self {
            ScriptValue::Int(i) => Some(i),
            ScriptValue::Float(f) => Some(f as i32),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f32> {
        match *self {
            ScriptValue::Int(i) => Some(i as f32),
            ScriptValue::Float(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<[f32; 3]> {
        match *self {
            ScriptValue::Vec3(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<ObjRef> {
        match *self {
            ScriptValue::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_str_id(&self) -> Option<Str> {
        match *self {
            ScriptValue::Str(s) | ScriptValue::IStr(s) => Some(s),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            ScriptValue::Undefined => "undefined",
            ScriptValue::Int(_) => "int",
            ScriptValue::Float(_) => "float",
            ScriptValue::Str(_) => "string",
            ScriptValue::IStr(_) => "localized string",
            ScriptValue::Hash(_) => "hash",
            ScriptValue::Vec3(_) => "vector",
            ScriptValue::Object(_) => "object",
            ScriptValue::Array(_) => "array",
            ScriptValue::Func(_) => "function",
            ScriptValue::Anim(_) => "animation",
            ScriptValue::AnimTree(_) => "animtree",
            ScriptValue::Marker => "(marker)",
        }
    }

    pub fn bool(b: bool) -> ScriptValue {
        ScriptValue::Int(i32::from(b))
    }
}

/// What an object is. Hosts give entities and HUD elements their own kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjKind {
    Struct,
    Level,
    Anim,
    /// A host entity, by entity number.
    Entity(u32),
    /// A host HUD element, by its slot.
    HudElem(u32),
}

#[derive(Debug)]
pub struct Object {
    pub stamp: u32,
    pub living: bool,
    /// Deleted this frame: no longer defined, its fields still readable
    /// until the frame's scripts are done.
    pub dying: bool,
    pub kind: ObjKind,
    pub fields: FxHashMap<Str, ScriptValue>,
}

/// Format a float the way script string concatenation does (`%g`-like).
pub fn format_float(f: f32) -> String {
    if f.is_finite() && f == f.trunc() && f.abs() < 1e9 {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}
