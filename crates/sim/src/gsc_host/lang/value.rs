//! Script values and the heap that holds their objects and arrays.

use std::collections::BTreeMap;
use std::sync::Arc;

/// What a script variable can hold. Objects and arrays are handles into the
/// [`Heap`]; copying one copies the handle, not the contents.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Undefined,
    Int(i32),
    Float(f32),
    String(Arc<str>),
    Vector([f32; 3]),
    Object(u64),
    Array(u64),
}

impl Value {
    pub fn string(text: &str) -> Self {
        Value::String(text.into())
    }

    /// The `level` object every script can reach.
    pub fn level() -> Self {
        Value::Object(LEVEL)
    }

    /// What an `if` or `while` sees: nothing and zero are false.
    pub fn is_true(&self) -> bool {
        match self {
            Value::Undefined => false,
            Value::Int(n) => *n != 0,
            Value::Float(x) => *x != 0.0,
            _ => true,
        }
    }

    /// How an error message names this value's type.
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Undefined => "undefined",
            Value::Int(_) => "an int",
            Value::Float(_) => "a float",
            Value::String(_) => "a string",
            Value::Vector(_) => "a vector",
            Value::Object(_) => "an object",
            Value::Array(_) => "an array",
        }
    }

    /// The text a value turns into when joined to a string.
    pub fn to_text(&self) -> Option<String> {
        match self {
            Value::String(text) => Some(text.to_string()),
            Value::Int(n) => Some(n.to_string()),
            Value::Float(x) => Some(x.to_string()),
            Value::Vector([x, y, z]) => Some(format!("({x}, {y}, {z})")),
            _ => None,
        }
    }
}

/// The three objects that always exist.
pub const LEVEL: u64 = 0;
pub const ANIM: u64 = 1;
pub const GAME: u64 = 2;

/// Where an array element lives: by number or by name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Index(i32),
    Name(Arc<str>),
}

impl Key {
    pub fn of(datum: &Value) -> Option<Key> {
        match datum {
            Value::Int(n) => Some(Key::Index(*n)),
            Value::String(name) => Some(Key::Name(name.clone())),
            _ => None,
        }
    }
}

/// Every object's fields and every array's elements.
#[derive(Clone, Debug)]
pub struct Heap {
    objects: BTreeMap<u64, BTreeMap<Arc<str>, Value>>,
    arrays: BTreeMap<u64, BTreeMap<Key, Value>>,
    next: u64,
}

impl Default for Heap {
    fn default() -> Self {
        Self {
            objects: [LEVEL, ANIM, GAME]
                .into_iter()
                .map(|root| (root, BTreeMap::new()))
                .collect(),
            arrays: BTreeMap::new(),
            next: GAME + 1,
        }
    }
}

impl Heap {
    fn fresh_id(&mut self) -> Result<u64, String> {
        let who_no = self.next;
        self.next = who_no.checked_add(1).ok_or("object identifier exhausted")?;
        Ok(who_no)
    }

    pub fn new_object(&mut self) -> Result<u64, String> {
        let id = self.fresh_id()?;
        self.objects.insert(id, BTreeMap::new());
        Ok(id)
    }

    pub fn new_array(&mut self) -> Result<u64, String> {
        let id = self.fresh_id()?;
        self.arrays.insert(id, BTreeMap::new());
        Ok(id)
    }

    pub fn field(&self, object: u64, name: &str) -> Option<&Value> {
        self.objects.get(&object)?.get(name)
    }

    pub fn set_field(&mut self, object: u64, name: &str, value: Value) {
        if let Some(fields) = self.objects.get_mut(&object) {
            fields.insert(name.into(), value);
        }
    }

    /// Forget an object; its handle then reads as empty.
    pub fn drop_object(&mut self, object: u64) {
        self.objects.remove(&object);
    }

    pub fn element(&self, array: u64, key: &Key) -> Option<&Value> {
        self.arrays.get(&array)?.get(key)
    }

    pub fn set_element(&mut self, array: u64, key: Key, value: Value) {
        if let Some(items) = self.arrays.get_mut(&array) {
            items.insert(key, value);
        }
    }

    pub fn array_len(&self, array: u64) -> usize {
        self.arrays.get(&array).map_or(0, BTreeMap::len)
    }
}
