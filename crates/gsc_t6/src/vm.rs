//! The virtual machine: threads, the scheduler and the interpreter.
//!
//! Semantics (Black Ops II as its compiled code shows them, measured on the
//! PC zones 2026-10-01, plus the IW-family VM conventions noted here):
//! - Arguments are pushed last first after a marker (`PreScriptCall`, or
//!   `VoidCodePos` for a notify); the callee's prologue pops them into its
//!   locals, parameters first. Locals are addressed from the newest
//!   (`EvalLocalVariableCached 0` is the last declared).
//! - `wait` rounds to 50 ms server frames (at least one); `waittillframeend`
//!   runs after every other thread this frame.
//! - `notify` ends endons at once; the threads it wakes run after the notifier
//!   waits (VM_Notify queues them at the current time). A fired endon unwinds
//!   its thread to the function that registered it; the caller sees that
//!   call return undefined.
//! - Deleting an entity ends the threads that run on it or wait on it.
//! - Runtime errors are reported once per place and the operation yields
//!   undefined; the thread goes on.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::Arc;
use std::sync::atomic;
use std::sync::atomic::Ordering::Relaxed;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::program::{BinOp, CallKind, Op, Program};
use crate::value::{
    Array, ArrayRef, FuncRef, Key, ObjKind, ObjRef, Object, Str, Strings, ScriptValue, format_float,
};

/// A builtin: `(vm, host, self, args) -> result`.
pub type Native<H> = fn(&mut Vm<H>, &mut H, &ScriptValue, &[ScriptValue]) -> Result<ScriptValue, String>;

/// Engine hooks: an engine-owned field of a host object (`Entity`,
/// `HudElem`; `None` = use the script field), writing one (`true` when
/// handled), and a builtin nobody bound.
pub type GetFieldHook<H> = fn(&mut Vm<H>, &mut H, ObjRef, ObjKind, Str) -> Option<ScriptValue>;
pub type SetFieldHook<H> = fn(&mut Vm<H>, &mut H, ObjRef, ObjKind, Str, &ScriptValue) -> bool;
pub type UnboundHook<H> = fn(&mut Vm<H>, &mut H, u32, &ScriptValue, &[ScriptValue]) -> ScriptValue;

pub struct Hooks<H> {
    pub get_field: Option<GetFieldHook<H>>,
    pub set_field: Option<SetFieldHook<H>>,
    pub unbound: Option<UnboundHook<H>>,
}

impl<H> Default for Hooks<H> {
    fn default() -> Self {
        Self {
            get_field: None,
            set_field: None,
            unbound: None,
        }
    }
}

/// The hash Black Ops II scripts use for `#"name"` (dvar names, ...):
/// djb2 over the lower-cased name (measured: `ui_zm_mapstartlocation` ->
/// 0xc955b4cd, `ui_gametype` -> 0x0041651e, as the compiled scripts carry).
pub fn hash_name(name: &str) -> u32 {
    let mut h: u32 = 5381;
    for c in name.bytes() {
        h = h
            .wrapping_mul(33)
            .wrapping_add(u32::from(c.to_ascii_lowercase()));
    }
    h
}

/// Script truth: numbers non-zero; undefined false; other values true.
pub fn truthy(v: &ScriptValue) -> bool {
    match v {
        ScriptValue::Undefined | ScriptValue::Marker => false,
        ScriptValue::Int(i) => *i != 0,
        ScriptValue::Float(f) => *f != 0.0,
        _ => true,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ThreadId {
    pub index: u32,
    pub stamp: u32,
}

#[derive(Debug)]
struct Frame {
    func: u32,
    pc: u32,
    locals: Vec<ScriptValue>,
    self_: ScriptValue,
    base: u32,
    args: Vec<ScriptValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ready,
    Timer,
    Notify,
    FrameEnd,
    Running,
}

#[derive(Debug)]
struct Thread {
    frames: Vec<Frame>,
    stack: Vec<ScriptValue>,
    /// (object, name, frame depth).
    endons: Vec<(ObjRef, Str, u32)>,
    waiting_on: Option<(ObjRef, Str)>,
    timer_seq: u64,
    state: State,
}

#[derive(Clone, Debug)]
enum WaiterKind {
    Waittill,
    Match(Vec<ScriptValue>),
    Endon(u32),
}

#[derive(Clone, Debug)]
struct Waiter {
    thread: ThreadId,
    obj_gen: u32,
    kind: WaiterKind,
}

#[derive(Clone, Debug, Default)]
enum RefBase {
    #[default]
    None,
    Local(u8),
    Field(ObjRef, Str),
    Game,
}

#[derive(Clone, Debug, Default)]
struct Ref {
    base: RefBase,
    path: Vec<Key>,
}

enum Outcome {
    Yield,
    Done,
}

const MAX_FRAMES: usize = 256;
const MAX_NEST: u32 = 48;
const STEP_LIMIT: u64 = 5_000_000;
pub const FRAME_MS: i64 = 50;

/// Script time by function and builtin (ms, calls): `Vm::profile`.
#[derive(Default)]
pub struct ScriptProfile {
    pub funcs: FxHashMap<u32, (f64, u32)>,
    pub natives: FxHashMap<u32, (f64, u32)>,
}

pub struct Vm<H> {
    pub strings: Strings,
    pub program: Arc<Program>,
    natives: Vec<Option<Native<H>>>,
    pub hooks: Hooks<H>,
    objects: Vec<Object>,
    free_objects: Vec<u32>,
    /// Objects freed this frame, their fields kept until it ends.
    dying: Vec<u32>,
    pub level: ObjRef,
    pub anim: ObjRef,
    pub game: ScriptValue,
    threads: Vec<Option<Box<Thread>>>,
    thread_gens: Vec<u32>,
    free_threads: Vec<u32>,
    /// (due ms, !seq, thread index, stamp): soonest first, and of the
    /// threads due on the same tick the newest `wait` first, as BO2 runs
    /// them (OP_wait puts a thread at the head of its time's list).
    timers: BinaryHeap<Reverse<(i64, u64, u32, u32)>>,
    upcoming_serial: u64,
    frame_end: Vec<ThreadId>,
    ready: VecDeque<ThreadId>,
    waiters: FxHashMap<(u32, Str), Vec<Waiter>>,
    running: Vec<ThreadId>,
    pending_unwind: FxHashMap<ThreadId, u32>,
    nest: u32,
    /// Woken threads wait in `ready` instead of running at once.
    queue_wakes: bool,
    pub clock_stamp_ms: i64,
    pub dvars: FxHashMap<String, String>,
    reported: FxHashSet<String>,
    /// Messages for the host to show (errors, unbound builtins).
    pub messages: Vec<String>,
    /// A thread that runs longer than this before it waits names itself
    /// in `messages` (ms; 0 = off): which script holds a frame up.
    pub slow_thread_ms: f64,
    /// While set: the time spent in each script function a thread resumed
    /// in, and in each builtin (`profile_report`).
    pub profile: Option<ScriptProfile>,
    pub steps: u64,
    pub rng: u64,
    /// Builtins whose calls are logged (debugging): id -> calls left.
    pub trace: FxHashMap<u32, u32>,
    /// Script functions whose calls are logged (`trace_functions`).
    pub ftrace: FxHashSet<u32>,
}

/// Hang watch (read from another thread when the sim stops ticking): the
/// builtin running now (id + 1, 0 = none), the script function that called
/// it, and a count of instructions run (moves while scripts loop).
static NATIVE_NOW: atomic::AtomicU32 = atomic::AtomicU32::new(0);
static NATIVE_CALLER: atomic::AtomicU32 = atomic::AtomicU32::new(u32::MAX);
static STEPS_RUN: atomic::AtomicU64 = atomic::AtomicU64::new(0);
/// Sampled every 4096 instructions: the function running and its thread's
/// first function; and how many times a thread was resumed.
static SAMPLE_FUNC: atomic::AtomicU32 = atomic::AtomicU32::new(u32::MAX);
static SAMPLE_ROOT: atomic::AtomicU32 = atomic::AtomicU32::new(u32::MAX);
static RESUMES: atomic::AtomicU64 = atomic::AtomicU64::new(0);
static NAMES: std::sync::Mutex<(Vec<String>, Vec<String>)> =
    std::sync::Mutex::new((Vec::new(), Vec::new()));

/// What the scripts are doing now, for a hang report: the builtin being
/// run (and by which function), or none, and the instructions run so far.
pub fn hang_report() -> String {
    let (id, caller, steps) = (
        NATIVE_NOW.load(Relaxed),
        NATIVE_CALLER.load(Relaxed),
        STEPS_RUN.load(Relaxed),
    );
    let names = NAMES.lock().map(|n| n.clone()).unwrap_or_default();
    let get = |v: &Vec<String>, i: u32| {
        v.get(i as usize)
            .cloned()
            .unwrap_or_else(|| format!("#{i}"))
    };
    let sample = format!(
        "{} instructions run, {} resumes; sampled {} in thread {}",
        steps,
        RESUMES.load(Relaxed),
        get(&names.1, SAMPLE_FUNC.load(Relaxed)),
        get(&names.1, SAMPLE_ROOT.load(Relaxed))
    );
    if id == 0 {
        format!("no builtin running; {sample}")
    } else {
        format!(
            "builtin {} called from {}; {sample}",
            get(&names.0, id - 1),
            get(&names.1, caller)
        )
    }
}

impl<H> Vm<H> {
    pub fn new(program: Program, strings: Strings) -> Self {
        let n = program.builtins.len();
        // The biggest program's names (the server's, not the client's).
        if let Ok(mut names) = NAMES.lock()
            && names.1.len() < program.functions.len()
        {
            names.0 = program
                .builtins
                .iter()
                .map(|(b, _)| strings.get(*b).to_owned())
                .collect();
            names.1 = program
                .functions
                .iter()
                .map(|f| {
                    format!(
                        "{}::{}",
                        program.scripts[f.script as usize].name,
                        strings.get(f.name)
                    )
                })
                .collect();
        }
        let mut vm = Self {
            strings,
            program: Arc::new(program),
            natives: vec![None; n],
            hooks: Hooks::default(),
            objects: Vec::new(),
            free_objects: Vec::new(),
            dying: Vec::new(),
            level: ObjRef {
                index: 0,
                generation: 0,
            },
            anim: ObjRef {
                index: 0,
                generation: 0,
            },
            game: ScriptValue::array(Array::new()),
            threads: Vec::new(),
            thread_gens: Vec::new(),
            free_threads: Vec::new(),
            timers: BinaryHeap::new(),
            upcoming_serial: 0,
            frame_end: Vec::new(),
            ready: VecDeque::new(),
            waiters: FxHashMap::default(),
            running: Vec::new(),
            pending_unwind: FxHashMap::default(),
            nest: 0,
            queue_wakes: false,
            clock_stamp_ms: 0,
            dvars: FxHashMap::default(),
            reported: FxHashSet::default(),
            messages: Vec::new(),
            slow_thread_ms: 0.0,
            profile: None,
            steps: 0,
            rng: 0x2545_f491_4f6c_dd1d,
            trace: FxHashMap::default(),
            ftrace: FxHashSet::default(),
        };
        vm.level = vm.alloc_object(ObjKind::Level);
        vm.anim = vm.alloc_object(ObjKind::Anim);
        vm
    }

    // ---- builtins -------------------------------------------------------

    /// Bind a builtin by name (`method` for `self name(...)` calls). Returns
    /// whether the program uses it.
    pub fn bind(&mut self, name: &str, method: bool, native: Native<H>) -> bool {
        let Some(s) = self.strings.find(name) else {
            return false;
        };
        match self.program.builtin_index.get(&(s, method)) {
            Some(&id) => {
                self.natives[id as usize] = Some(native);
                true
            }
            None => false,
        }
    }

    /// The builtin bound under a name now, if any (so a later bind can wrap it).
    pub fn bound(&self, name: &str, method: bool) -> Option<Native<H>> {
        let s = self.strings.find(name)?;
        let &id = self.program.builtin_index.get(&(s, method))?;
        self.natives[id as usize]
    }

    /// Builtins the program calls that nothing bound: (name, method).
    pub fn unbound_builtins(&self) -> Vec<(String, bool)> {
        self.program
            .builtins
            .iter()
            .enumerate()
            .filter(|(i, _)| self.natives[*i].is_none())
            .map(|(_, (n, m))| (self.strings.get(*n).to_owned(), *m))
            .collect()
    }

    pub fn intern(&mut self, s: &str) -> Str {
        self.strings.intern(s)
    }

    pub fn str(&self, s: Str) -> &str {
        self.strings.get(s)
    }

    pub fn report_once(&mut self, msg: String) {
        if self.reported.insert(msg.clone()) {
            self.messages.push(msg);
        }
    }

    // ---- objects --------------------------------------------------------

    pub fn alloc_object(&mut self, kind: ObjKind) -> ObjRef {
        if let Some(i) = self.free_objects.pop() {
            let o = &mut self.objects[i as usize];
            o.stamp = o.stamp.wrapping_add(1);
            o.living = true;
            o.dying = false;
            o.kind = kind;
            o.fields.clear();
            return ObjRef {
                index: i,
                generation: o.stamp,
            };
        }
        let i = self.objects.len() as u32;
        self.objects.push(Object {
            stamp: 0,
            living: true,
            dying: false,
            kind,
            fields: FxHashMap::default(),
        });
        ObjRef {
            index: i,
            generation: 0,
        }
    }

    pub fn new_struct(&mut self) -> ScriptValue {
        ScriptValue::Object(self.alloc_object(ObjKind::Struct))
    }

    pub fn alive(&self, o: ObjRef) -> bool {
        self.objects
            .get(o.index as usize)
            .is_some_and(|x| x.living && x.stamp == o.generation)
    }

    /// Deleted this frame (its fields still read).
    fn dying(&self, o: ObjRef) -> bool {
        self.objects
            .get(o.index as usize)
            .is_some_and(|x| x.dying && x.stamp == o.generation)
    }

    pub fn kind(&self, obj: ObjRef) -> Option<ObjKind> {
        self.objects
            .get(obj.index as usize)
            .filter(|x| x.living && x.stamp == obj.generation)
            .map(|x| x.kind)
    }

    /// The script field (not the engine's). A deleted object's last until
    /// the frame's scripts are done (BO2's `destroyhelicopter` deletes the
    /// chopper, then reads its `hardpointtype` and `killstreak_id`).
    pub fn raw_field(&self, o: ObjRef, f: Str) -> ScriptValue {
        match self.objects.get(o.index as usize) {
            Some(x) if (x.living || x.dying) && x.stamp == o.generation => {
                x.fields.get(&f).cloned().unwrap_or_default()
            }
            _ => ScriptValue::Undefined,
        }
    }

    pub fn set_raw_field(&mut self, o: ObjRef, f: Str, v: ScriptValue) {
        if let Some(x) = self.objects.get_mut(o.index as usize)
            && (x.living || x.dying)
            && x.stamp == o.generation
        {
            if v.is_undefined() {
                x.fields.remove(&f);
            } else {
                x.fields.insert(f, v);
            }
        }
    }

    pub fn field_names(&self, o: ObjRef) -> Vec<Str> {
        match self.objects.get(o.index as usize) {
            Some(x) if x.living && x.stamp == o.generation => {
                x.fields.keys().copied().collect()
            }
            _ => Vec::new(),
        }
    }

    pub fn get_field(&mut self, host: &mut H, o: ObjRef, f: Str) -> ScriptValue {
        let Some(kind) = self.kind(o) else {
            return self.raw_field(o, f);
        };
        if matches!(kind, ObjKind::Entity(_) | ObjKind::HudElem(_))
            && let Some(hook) = self.hooks.get_field
            && let Some(v) = hook(self, host, o, kind, f)
        {
            return v;
        }
        self.raw_field(o, f)
    }

    pub fn set_field(&mut self, host: &mut H, o: ObjRef, f: Str, v: ScriptValue) {
        let Some(kind) = self.kind(o) else {
            if self.dying(o) {
                self.set_raw_field(o, f, v);
            } else {
                self.report_once(format!("field '{}' set on a deleted object", self.str(f)));
            }
            return;
        };
        if matches!(kind, ObjKind::Entity(_) | ObjKind::HudElem(_))
            && let Some(hook) = self.hooks.set_field
            && hook(self, host, o, kind, f, &v)
        {
            return;
        }
        self.set_raw_field(o, f, v);
    }

    /// Free an object (an entity deleted), as BO2 cancels a dead entity's
    /// notify list (Scr_FreeEntityList -> Scr_CancelNotifyList): a thread
    /// waiting on it ends, an endon on it is dropped and its thread runs on,
    /// and threads that run on it run on (nothing ends them for that; a
    /// `delete` ends `endon("death")` threads by notifying "death" first).
    pub fn free_object(&mut self, host: &mut H, o: ObjRef) {
        if !self.alive(o) {
            return;
        }
        // Endons on it are dropped (their threads run on) and waits on it end.
        // Threads running on it (its `self`) go on, as in Black Ops II: a care
        // package's crate thread runs on the helicopter that dropped it, long
        // after the helicopter is gone (a call on a removed entity is that
        // thread's own error).
        let keys: Vec<(u32, Str)> = self
            .waiters
            .keys()
            .filter(|k| k.0 == o.index)
            .copied()
            .collect();
        let mut kill: Vec<ThreadId> = Vec::new();
        let mut dropped: Vec<ThreadId> = Vec::new();
        for k in keys {
            if let Some(list) = self.waiters.remove(&k) {
                for w in list {
                    if w.obj_gen != o.generation {
                        continue;
                    }
                    match w.kind {
                        WaiterKind::Endon(_) => dropped.push(w.thread),
                        _ => kill.push(w.thread),
                    }
                }
            }
        }
        // A running thread's own list keeps its stale entry; its waiter is
        // gone, so nothing can fire it.
        for t in dropped {
            if self.thread_gens.get(t.index as usize) == Some(&t.stamp)
                && let Some(Some(th)) = self.threads.get_mut(t.index as usize)
            {
                th.endons.retain(|e| e.0 != o);
            }
        }
        let x = &mut self.objects[o.index as usize];
        x.living = false;
        x.dying = true;
        self.dying.push(o.index);
        for t in kill {
            self.unwind(host, t, 0);
        }
    }

    /// The level restarts (Black Ops II's `map_restart`), between frames:
    /// every thread ends and every object goes (a host's references to them
    /// read as gone); `game`, the program, the strings, the natives, the
    /// dvars and the dice stay. A new `level` and `anim`.
    pub fn restart(&mut self) {
        debug_assert!(self.running.is_empty());
        self.collect_deleted();
        for (i, t) in self.threads.iter_mut().enumerate() {
            if t.take().is_some() {
                self.free_threads.push(i as u32);
            }
        }
        self.timers.clear();
        self.frame_end.clear();
        self.ready.clear();
        self.waiters.clear();
        self.pending_unwind.clear();
        for (i, o) in self.objects.iter_mut().enumerate() {
            if o.living {
                o.living = false;
                o.fields.clear();
                self.free_objects.push(i as u32);
            }
        }
        self.level = self.alloc_object(ObjKind::Level);
        self.anim = self.alloc_object(ObjKind::Anim);
    }

    // ---- threads --------------------------------------------------------

    fn new_thread(&mut self, func: u32, self_: ScriptValue, args: Vec<ScriptValue>) -> ThreadId {
        let th = Box::new(Thread {
            frames: vec![Frame {
                func,
                pc: 0,
                locals: Vec::new(),
                self_,
                base: 0,
                args,
            }],
            stack: Vec::with_capacity(16),
            endons: Vec::new(),
            waiting_on: None,
            timer_seq: 0,
            state: State::Ready,
        });
        if let Some(i) = self.free_threads.pop() {
            self.thread_gens[i as usize] = self.thread_gens[i as usize].wrapping_add(1);
            self.threads[i as usize] = Some(th);
            return ThreadId {
                index: i,
                stamp: self.thread_gens[i as usize],
            };
        }
        let i = self.threads.len() as u32;
        self.threads.push(Some(th));
        self.thread_gens.push(0);
        ThreadId {
            index: i,
            stamp: 0,
        }
    }

    /// Is thread `t` still running (not returned, not ended by an endon)?
    pub fn thread_live(&self, t: ThreadId) -> bool {
        self.thread_gens.get(t.index as usize) == Some(&t.stamp)
            && (self.threads[t.index as usize].is_some() || self.running.contains(&t))
    }

    /// Is the thread still running (not returned or ended)?
    pub fn thread_running(&self, t: ThreadId) -> bool {
        self.thread_live(t)
    }

    /// Start `func` as a new thread on `self_` and run it until it yields.
    pub fn spawn(&mut self, host: &mut H, entry: u32, owner: ScriptValue, args: Vec<ScriptValue>) -> ThreadId {
        let t = self.new_thread(entry, owner, args);
        self.exec(host, t);
        t
    }

    /// Start a script function by path and name; `None` if it does not exist.
    pub fn spawn_named(
        &mut self,
        host: &mut H,
        file: &str,
        name: &str,
        self_: ScriptValue,
        args: Vec<ScriptValue>,
    ) -> Option<ThreadId> {
        let f = self.program.find(&self.strings, file, name)?;
        Some(self.spawn(host, f, self_, args))
    }

    /// Number of live threads.
    pub fn thread_count(&self) -> usize {
        self.threads.iter().filter(|t| t.is_some()).count() + self.running.len()
    }

    fn free_thread(&mut self, t: ThreadId, th: &mut Thread) {
        self.cancel_wait(t, th);
        for (o, n, _) in th.endons.drain(..) {
            if let Some(list) = self.waiters.get_mut(&(o.index, n)) {
                list.retain(|w| w.thread != t);
                if list.is_empty() {
                    self.waiters.remove(&(o.index, n));
                }
            }
        }
        self.threads[t.index as usize] = None;
        self.free_threads.push(t.index);
        self.thread_gens[t.index as usize] = self.thread_gens[t.index as usize].wrapping_add(1);
    }

    fn cancel_wait(&mut self, t: ThreadId, th: &mut Thread) {
        if let Some((o, n)) = th.waiting_on.take()
            && let Some(list) = self.waiters.get_mut(&(o.index, n))
        {
            list.retain(|w| !(w.thread == t && !matches!(w.kind, WaiterKind::Endon(_))));
            if list.is_empty() {
                self.waiters.remove(&(o.index, n));
            }
        }
        th.timer_seq = th.timer_seq.wrapping_add(1);
        if th.state == State::FrameEnd {
            self.frame_end.retain(|x| *x != t);
        }
        th.state = State::Ready;
    }

    /// Unwind thread `t` to frame `depth` (0 ends it); the caller of the
    /// unwound frame sees undefined and the thread runs on now.
    fn unwind(&mut self, host: &mut H, t: ThreadId, depth: u32) {
        if self.running.contains(&t) {
            let d = self.pending_unwind.entry(t).or_insert(depth);
            *d = (*d).min(depth);
            return;
        }
        if self.thread_gens.get(t.index as usize) != Some(&t.stamp) {
            return;
        }
        let Some(mut th) = self.threads[t.index as usize].take() else {
            return;
        };
        self.cancel_wait(t, &mut th);
        let alive = self.apply_unwind(t, &mut th, depth);
        if !alive {
            self.threads[t.index as usize] = Some(th);
            let mut th = self.threads[t.index as usize].take().unwrap();
            self.free_thread(t, &mut th);
            return;
        }
        th.state = State::Ready;
        self.threads[t.index as usize] = Some(th);
        self.run_or_defer(host, t);
    }

    /// Pop frames down to `depth`; false when nothing is left.
    fn apply_unwind(&mut self, t: ThreadId, th: &mut Thread, depth: u32) -> bool {
        let depth = depth as usize;
        if depth >= th.frames.len() {
            return true;
        }
        let base = th.frames[depth].base as usize;
        th.frames.truncate(depth);
        th.stack.truncate(base);
        let mut dropped_endons = Vec::new();
        th.endons.retain(|e| {
            if e.2 as usize >= depth {
                dropped_endons.push((e.0, e.1));
                false
            } else {
                true
            }
        });
        for (o, n) in dropped_endons {
            if let Some(list) = self.waiters.get_mut(&(o.index, n)) {
                list.retain(|w| {
                    !(w.thread == t
                        && matches!(w.kind, WaiterKind::Endon(d) if d as usize >= depth))
                });
                if list.is_empty() {
                    self.waiters.remove(&(o.index, n));
                }
            }
        }
        if th.frames.is_empty() {
            return false;
        }
        th.stack.push(ScriptValue::Undefined);
        true
    }

    fn run_or_defer(&mut self, host: &mut H, t: ThreadId) {
        if self.nest >= MAX_NEST || self.queue_wakes {
            self.ready.push_back(t);
        } else {
            self.exec(host, t);
        }
    }

    /// Notify `name` on `obj` with `args`: endons fire, then waiting threads
    /// run.
    pub fn notify(&mut self, host: &mut H, obj: ObjRef, name: Str, args: &[ScriptValue]) {
        let Some(list) = self.waiters.remove(&(obj.index, name)) else {
            return;
        };
        let mut keep = Vec::new();
        let mut endons: Vec<(ThreadId, u32)> = Vec::new();
        let mut wakes: Vec<ThreadId> = Vec::new();
        for w in list {
            if w.obj_gen != obj.generation {
                continue;
            }
            if !self.thread_live(w.thread) {
                continue;
            }
            match &w.kind {
                WaiterKind::Endon(d) => {
                    match endons.iter_mut().find(|e| e.0 == w.thread) {
                        Some(e) => e.1 = e.1.min(*d),
                        None => endons.push((w.thread, *d)),
                    }
                    keep.push(w);
                }
                WaiterKind::Waittill => wakes.push(w.thread),
                WaiterKind::Match(vals) => {
                    let ok = vals
                        .iter()
                        .enumerate()
                        .all(|(i, v)| args.get(i).is_some_and(|a| self.equal(a, v)));
                    if ok {
                        wakes.push(w.thread);
                    } else {
                        keep.push(w);
                    }
                }
            }
        }
        if !keep.is_empty() {
            self.waiters.insert((obj.index, name), keep);
        }
        for (t, d) in endons {
            self.unwind(host, t, d);
        }
        // The newest waiter wakes first, as in the engine: the zombies
        // scripts rely on it (_zm::init_player_levelvars starts waiting for
        // "start_zombie_round_logic" a frame-end after _zm_powerups'
        // watch_for_drop, yet must set the score values it reads first).
        for t in wakes.into_iter().rev() {
            self.wake(host, t, obj, name, args);
        }
    }

    /// Notify `name` on `obj` as BO2's VM_Notify does: endons end at once,
    /// waiting threads are queued to run after the caller (on the current
    /// time's list). `delete` notifies "death" so: its waiters run after the
    /// entity is freed and find `self` undefined (Die Rise's corpse-room
    /// count returns on a deleted zombie).
    pub fn notify_queued(&mut self, host: &mut H, obj: ObjRef, name: &str, args: &[ScriptValue]) {
        let n = self.intern(name);
        let was = std::mem::replace(&mut self.queue_wakes, true);
        self.notify(host, obj, n, args);
        self.queue_wakes = was;
    }

    /// Notify `name` on `obj` to one thread only, the others waiting on it
    /// left asleep: a script started late (another map's weapon set up on
    /// its first give) past the `waittill( "spawned_player" )` it opens with.
    pub fn notify_thread(&mut self, host: &mut H, t: ThreadId, obj: ObjRef, name: &str, args: &[ScriptValue]) {
        let n = self.intern(name);
        if let Some(list) = self.waiters.get_mut(&(obj.index, n)) {
            list.retain(|w| !(w.thread == t && matches!(w.kind, WaiterKind::Waittill)));
            if list.is_empty() {
                self.waiters.remove(&(obj.index, n));
            }
        }
        self.wake(host, t, obj, n, args);
    }

    /// Notify by string.
    pub fn notify_str(&mut self, host: &mut H, obj: ObjRef, name: &str, args: &[ScriptValue]) {
        let n = self.intern(name);
        self.notify(host, obj, n, args);
    }

    fn wake(&mut self, host: &mut H, t: ThreadId, obj: ObjRef, name: Str, args: &[ScriptValue]) {
        if self.thread_gens.get(t.index as usize) != Some(&t.stamp) {
            return;
        }
        let Some(th) = self.threads[t.index as usize].as_mut() else {
            return;
        };
        // Still waiting on this?
        if th.state != State::Notify || th.waiting_on != Some((obj, name)) {
            return;
        }
        th.waiting_on = None;
        th.state = State::Ready;
        th.stack.push(ScriptValue::Marker);
        for a in args.iter().rev() {
            th.stack.push(a.clone());
        }
        self.run_or_defer(host, t);
    }

    /// Advance one server frame: due timers, then frame-end threads.
    pub fn run_frame(&mut self, host: &mut H) {
        self.clock_stamp_ms += FRAME_MS;
        self.run_due(host);
    }

    /// Run everything due at the current time.
    pub fn run_due(&mut self, host: &mut H) {
        // `wait 0` comes back this frame: a script looping on it alone
        // would never let the frame end, so past this many wakes the rest
        // wait for the next.
        let mut wakes = 0u32;
        loop {
            while let Some(t) = self.ready.pop_front() {
                self.exec(host, t);
            }
            let Some(&Reverse((at, newest, index, stamp))) = self.timers.peek() else {
                break;
            };
            let seq = !newest;
            if at > self.clock_stamp_ms || wakes >= 100_000 {
                break;
            }
            wakes += 1;
            self.timers.pop();
            let t = ThreadId { index, stamp };
            let ok = self.thread_gens.get(index as usize) == Some(&stamp)
                && self.threads[index as usize]
                    .as_ref()
                    .is_some_and(|th| th.state == State::Timer && th.timer_seq == seq);
            if ok {
                if let Some(th) = self.threads[index as usize].as_mut() {
                    th.state = State::Ready;
                }
                self.exec(host, t);
            }
        }
        for _ in 0..8 {
            while let Some(t) = self.ready.pop_front() {
                self.exec(host, t);
            }
            if self.frame_end.is_empty() {
                break;
            }
            let list = std::mem::take(&mut self.frame_end);
            for t in list {
                if let Some(Some(th)) = self.threads.get_mut(t.index as usize)
                    && self.thread_gens[t.index as usize] == t.stamp
                    && th.state == State::FrameEnd
                {
                    th.state = State::Ready;
                    self.exec(host, t);
                    // Threads it woke run as soon as it waits, before the next
                    // frame-end thread (_globallogic_player's "connected" sets
                    // up _visionset_mgr's player state before its monitor reads it).
                    while let Some(t) = self.ready.pop_front() {
                        self.exec(host, t);
                    }
                }
            }
        }
        self.collect_deleted();
    }

    /// The frame's scripts are done: objects deleted in it lose their
    /// fields and their slots can be reused.
    fn collect_deleted(&mut self) {
        for i in std::mem::take(&mut self.dying) {
            let x = &mut self.objects[i as usize];
            if x.dying {
                x.dying = false;
                x.fields.clear();
                self.free_objects.push(i);
            }
        }
    }

    fn exec(&mut self, host: &mut H, t: ThreadId) {
        if self.thread_gens.get(t.index as usize) != Some(&t.stamp) {
            return;
        }
        let Some(mut th) = self.threads[t.index as usize].take() else {
            return;
        };
        if th.state != State::Ready {
            self.threads[t.index as usize] = Some(th);
            return;
        }
        th.state = State::Running;
        self.running.push(t);
        self.nest += 1;
        let program = self.program.clone();
        let timed = (self.slow_thread_ms > 0.0 || self.profile.is_some()).then(|| {
            let func = |f: Option<&Frame>| f.map(|f| f.func);
            (std::time::Instant::now(), func(th.frames.first()), func(th.frames.last()))
        });
        let step_result = self.interp(host, t, &mut th, &program);
        if let Some((started, entry, at)) = timed {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if let (Some(p), Some(f)) = (self.profile.as_mut(), at) {
                let row = p.funcs.entry(f).or_default();
                row.0 += ms;
                row.1 += 1;
            }
            if self.slow_thread_ms > 0.0 && ms > self.slow_thread_ms {
                let name = |f: Option<u32>| {
                    f.and_then(|f| program.functions.get(f as usize)).map_or_else(String::new, |func| {
                        format!("{}::{}", program.scripts[func.script as usize].name, self.strings.get(func.name))
                    })
                };
                let line = format!("slow thread: {ms:.1} ms in {} (from {})", name(at), name(entry));
                self.messages.push(line);
            }
        }
        self.nest -= 1;
        self.running.pop();
        match step_result {
            Outcome::Done => {
                self.threads[t.index as usize] = Some(th);
                let mut th = self.threads[t.index as usize].take().unwrap();
                self.free_thread(t, &mut th);
            }
            Outcome::Yield => {
                self.threads[t.index as usize] = Some(th);
            }
        }
    }

    fn fail_thread(&mut self, program: &Program, th: &Thread, msg: &str) {
        let (f, pc) = th.frames.last().map_or((0, 0), |f| (f.func, f.pc));
        let func = &program.functions[f as usize];
        let script = &program.scripts[func.script as usize].name;
        let at = func
            .addrs
            .get(pc.saturating_sub(1) as usize)
            .copied()
            .unwrap_or(0);
        let m = format!("{script}::{} @{at:#x}: {msg}", self.strings.get(func.name));
        self.report_once(m);
    }

    // ---- values ---------------------------------------------------------

    pub fn equal(&self, a: &ScriptValue, b: &ScriptValue) -> bool {
        match (a, b) {
            (ScriptValue::Undefined, ScriptValue::Undefined) => true,
            (ScriptValue::Int(x), ScriptValue::Int(y)) => x == y,
            (ScriptValue::Int(_) | ScriptValue::Float(_), ScriptValue::Int(_) | ScriptValue::Float(_)) => {
                a.as_float() == b.as_float()
            }
            (ScriptValue::Str(x) | ScriptValue::IStr(x), ScriptValue::Str(y) | ScriptValue::IStr(y)) => x == y,
            (ScriptValue::Hash(x), ScriptValue::Hash(y)) => x == y,
            (ScriptValue::Vec3(x), ScriptValue::Vec3(y)) => x == y,
            (ScriptValue::Object(x), ScriptValue::Object(y)) => x == y && self.alive(*x),
            (ScriptValue::Object(x), ScriptValue::Undefined) | (ScriptValue::Undefined, ScriptValue::Object(x)) => {
                !self.alive(*x)
            }
            (ScriptValue::Array(x), ScriptValue::Array(y)) => ArrayRef::ptr_eq(x, y),
            (ScriptValue::Func(x), ScriptValue::Func(y)) => x == y,
            (ScriptValue::Anim(x), ScriptValue::Anim(y)) | (ScriptValue::AnimTree(x), ScriptValue::AnimTree(y)) => x == y,
            _ => false,
        }
    }

    /// A value made fit to read: dead objects read as undefined (one
    /// deleted this frame stays, for its fields; it is not defined).
    fn live(&self, v: ScriptValue) -> ScriptValue {
        match v {
            ScriptValue::Object(o) if !self.alive(o) && !self.dying(o) => ScriptValue::Undefined,
            v => v,
        }
    }

    pub fn is_defined(&self, v: &ScriptValue) -> bool {
        match v {
            ScriptValue::Undefined | ScriptValue::Marker => false,
            ScriptValue::Object(o) => self.alive(*o),
            _ => true,
        }
    }

    pub fn to_key(&self, v: &ScriptValue) -> Result<Key, String> {
        match v {
            ScriptValue::Int(i) => Ok(Key::Int(*i)),
            ScriptValue::Float(f) => Ok(Key::Int(*f as i32)),
            ScriptValue::Str(s) | ScriptValue::IStr(s) => Ok(Key::Str(*s)),
            ScriptValue::Object(o) => Ok(Key::Obj(*o)),
            ScriptValue::Hash(h) => Ok(Key::Int(*h as i32)),
            other => Err(format!("{} is not an array index", other.type_name())),
        }
    }

    pub fn to_text(&self, v: &ScriptValue) -> String {
        match v {
            ScriptValue::Undefined => "undefined".into(),
            ScriptValue::Int(i) => i.to_string(),
            ScriptValue::Float(f) => format_float(*f),
            ScriptValue::Str(s) | ScriptValue::IStr(s) => self.strings.get(*s).to_owned(),
            ScriptValue::Vec3(v) => format!(
                "({}, {}, {})",
                format_float(v[0]),
                format_float(v[1]),
                format_float(v[2])
            ),
            ScriptValue::Hash(h) => format!("#{h:08x}"),
            ScriptValue::Object(_) => "[object]".into(),
            ScriptValue::Array(_) => "[array]".into(),
            ScriptValue::Func(_) => "[function]".into(),
            ScriptValue::Anim(_) | ScriptValue::AnimTree(_) => "[anim]".into(),
            ScriptValue::Marker => "[marker]".into(),
        }
    }

    pub fn string(&mut self, s: &str) -> ScriptValue {
        ScriptValue::Str(self.strings.intern(s))
    }

    fn binop(&mut self, op: BinOp, a: ScriptValue, b: ScriptValue) -> Result<ScriptValue, String> {
        use ScriptValue::{Float, Int, Vec3};
        let num = |v: &ScriptValue| v.as_float();
        Ok(match op {
            BinOp::Eq => ScriptValue::bool(self.equal(&a, &b)),
            BinOp::Ne => ScriptValue::bool(!self.equal(&a, &b)),
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                use std::cmp::Ordering::{Greater, Less};
                let ord = match (&a, &b) {
                    (Int(x), Int(y)) => x.partial_cmp(y),
                    // bo2mp: two strings (or two localized strings) go by
                    // BO2's `I_strcmp`, case-sensitive over signed chars;
                    // `_sharpshooter` orders two attachment names this way.
                    (ScriptValue::Str(x), ScriptValue::Str(y)) | (ScriptValue::IStr(x), ScriptValue::IStr(y)) => {
                        let key = |s| {
                            self.strings
                                .get(s)
                                .bytes()
                                .map(|c| c as i8)
                                .chain(std::iter::once(0))
                        };
                        Some(key(*x).cmp(key(*y)))
                    }
                    _ => {
                        let (Some(x), Some(y)) = (num(&a), num(&b)) else {
                            return Err(format!(
                                "cannot compare {} with {}",
                                a.type_name(),
                                b.type_name()
                            ));
                        };
                        x.partial_cmp(&y)
                    }
                };
                // bo2mp: BO2's `Scr_EvalLessEqual` is not-greater and its
                // `Scr_EvalGreaterEqual` is not-less, so NaN reads true for both.
                ScriptValue::bool(match op {
                    BinOp::Lt => ord == Some(Less),
                    BinOp::Gt => ord == Some(Greater),
                    BinOp::Le => ord != Some(Greater),
                    _ => ord != Some(Less),
                })
            }
            BinOp::Or | BinOp::Xor | BinOp::And | BinOp::Shl | BinOp::Shr => {
                let (Some(lhs), Some(rhs)) = (a.as_int(), b.as_int()) else {
                    return Err(format!(
                        "bit operation on {} and {}",
                        a.type_name(),
                        b.type_name()
                    ));
                };
                Int(match op {
                    BinOp::Or => lhs | rhs,
                    BinOp::Xor => lhs ^ rhs,
                    BinOp::And => lhs & rhs,
                    BinOp::Shl => lhs.wrapping_shl(rhs as u32 & 31),
                    _ => lhs.wrapping_shr(rhs as u32 & 31),
                })
            }
            BinOp::Add => match (&a, &b) {
                (Int(x), Int(y)) => Int(x.wrapping_add(*y)),
                // bo2mp: a hashed dvar name plus an index (`_class`'s
                // exclusion lists walk `#"..." + n` until one is unset): a
                // hash no dvar of ours has, so it reads as unset.
                (ScriptValue::Hash(h), Int(y)) | (Int(y), ScriptValue::Hash(h)) => {
                    ScriptValue::Hash(h.wrapping_add(*y as u32))
                }
                (Vec3(x), Vec3(y)) => Vec3([x[0] + y[0], x[1] + y[1], x[2] + y[2]]),
                (ScriptValue::Str(_) | ScriptValue::IStr(_), _) | (_, ScriptValue::Str(_) | ScriptValue::IStr(_)) => {
                    let s = format!("{}{}", self.to_text(&a), self.to_text(&b));
                    self.string(&s)
                }
                _ => match (num(&a), num(&b)) {
                    (Some(x), Some(y)) => Float(x + y),
                    _ => {
                        return Err(format!(
                            "cannot add {} and {}",
                            a.type_name(),
                            b.type_name()
                        ));
                    }
                },
            },
            BinOp::Sub => match (&a, &b) {
                (Int(x), Int(y)) => Int(x.wrapping_sub(*y)),
                (Vec3(x), Vec3(y)) => Vec3([x[0] - y[0], x[1] - y[1], x[2] - y[2]]),
                // A number with a vector is first made (n, n, n), as BO2's
                // own VM does (the Jet Gun's smoke: tag_flash - view height).
                (Vec3(v), s) if num(s).is_some() => {
                    let k = num(s).unwrap_or(0.0);
                    Vec3([v[0] - k, v[1] - k, v[2] - k])
                }
                (s, Vec3(v)) if num(s).is_some() => {
                    let k = num(s).unwrap_or(0.0);
                    Vec3([k - v[0], k - v[1], k - v[2]])
                }
                _ => match (num(&a), num(&b)) {
                    (Some(x), Some(y)) => Float(x - y),
                    _ => {
                        return Err(format!(
                            "cannot subtract {} and {}",
                            a.type_name(),
                            b.type_name()
                        ));
                    }
                },
            },
            BinOp::Mul => match (&a, &b) {
                (Int(x), Int(y)) => Int(x.wrapping_mul(*y)),
                (Vec3(x), Vec3(y)) => Vec3([x[0] * y[0], x[1] * y[1], x[2] * y[2]]),
                (Vec3(v), s) | (s, Vec3(v)) => match num(s) {
                    Some(k) => Vec3([v[0] * k, v[1] * k, v[2] * k]),
                    None => return Err(format!("cannot multiply a vector by {}", s.type_name())),
                },
                _ => match (num(&a), num(&b)) {
                    (Some(x), Some(y)) => Float(x * y),
                    _ => {
                        return Err(format!(
                            "cannot multiply {} and {}",
                            a.type_name(),
                            b.type_name()
                        ));
                    }
                },
            },
            // bo2mp: BO2's `Scr_EvalDivide` after `Scr_CastWeakerPair2`: a
            // number meeting a vector divides as (k, k, k), two vectors part
            // by part, two ints as floats. By 0 is `by_zero`'s.
            BinOp::Div => match (&a, &b) {
                (Vec3(_), _) | (_, Vec3(_)) => {
                    let splat = |v: &ScriptValue| match v {
                        Vec3(x) => Some(*x),
                        _ => num(v).map(|k| [k; 3]),
                    };
                    match (splat(&a), splat(&b)) {
                        (Some(_), Some(y)) if y.contains(&0.0) => {
                            return Err("divide by 0".into());
                        }
                        (Some(x), Some(y)) => Vec3([x[0] / y[0], x[1] / y[1], x[2] / y[2]]),
                        _ => {
                            return Err(format!(
                                "cannot divide {} by {}",
                                a.type_name(),
                                b.type_name()
                            ));
                        }
                    }
                }
                _ => match (num(&a), num(&b)) {
                    (Some(_), Some(y)) if y == 0.0 => return Err("divide by 0".into()),
                    (Some(x), Some(y)) => Float(x / y),
                    _ => {
                        return Err(format!(
                            "cannot divide {} by {}",
                            a.type_name(),
                            b.type_name()
                        ));
                    }
                },
            },
            // bo2mp: BO2's `Scr_EvalMod` takes two ints and nothing else (no
            // cast first): any other pair is its unmatching-types error, and
            // undefined.
            BinOp::Mod => match (&a, &b) {
                (Int(_), Int(0)) => return Err("divide by 0".into()),
                (Int(x), Int(y)) => Int(x.wrapping_rem(*y)),
                _ => {
                    return Err(format!(
                        "pair '{}' and '{}' has unmatching types '{}' and '{}'",
                        self.to_text(&a),
                        self.to_text(&b),
                        a.type_name(),
                        b.type_name()
                    ));
                }
            },
        })
    }

    /// bo2mp: what BO2 leaves for a divide by 0 (`Scr_EvalDivide`, and
    /// `Scr_EvalMod` on two ints): 0, (0, 0, 0) when a vector is in it (any
    /// part of a vector divisor 0). `None` when `b` does not divide by 0.
    fn by_zero(op: BinOp, a: &ScriptValue, b: &ScriptValue) -> Option<ScriptValue> {
        use ScriptValue::{Float, Int, Vec3};
        let zero = |v: &ScriptValue| v.as_float() == Some(0.0);
        match (op, a, b) {
            (BinOp::Mod, Int(_), Int(0)) => Some(Int(0)),
            (BinOp::Div, Vec3(_) | Int(_) | Float(_), Vec3(d)) if d.contains(&0.0) => {
                Some(Vec3([0.0; 3]))
            }
            (BinOp::Div, Vec3(_), d) if zero(d) => Some(Vec3([0.0; 3])),
            (BinOp::Div, Int(_) | Float(_), d) if zero(d) => Some(Float(0.0)),
            _ => None,
        }
    }

    /// `a[i]`, with BO2's own error lines (`Scr_EvalArray`).
    fn index(&mut self, container: &ScriptValue, key: &ScriptValue) -> Result<ScriptValue, String> {
        match container {
            ScriptValue::Array(a) => {
                let k = self.to_key(key)?;
                Ok(self.live(a.get(&k).unwrap_or_default()))
            }
            ScriptValue::Object(_) => Err(format!("{} is not an array", container.type_name())),
            ScriptValue::Str(s) | ScriptValue::IStr(s) => {
                let i = key
                    .as_int()
                    .ok_or_else(|| format!("{} is not a string index", key.type_name()))?;
                let text = self.strings.get(*s);
                let c = usize::try_from(i)
                    .ok()
                    .and_then(|i| text.chars().nth(i))
                    .ok_or_else(|| format!("string index {i} out of range"))?;
                Ok(self.string(&c.to_string()))
            }
            ScriptValue::Vec3(v) => {
                let i = key
                    .as_int()
                    .ok_or_else(|| format!("{} is not a vector index", key.type_name()))?;
                usize::try_from(i)
                    .ok()
                    .and_then(|i| v.get(i))
                    .map(|x| ScriptValue::Float(*x))
                    .ok_or_else(|| format!("vector index {i} out of range"))
            }
            bad => Err(format!(
                "{} is not an array, string, or vector",
                bad.type_name()
            )),
        }
    }

    fn write_path(slot: &mut ScriptValue, path: &[Key], v: ScriptValue) -> Result<(), String> {
        let Some((&k, rest)) = path.split_first() else {
            *slot = v;
            return Ok(());
        };
        if slot.is_undefined() {
            *slot = ScriptValue::array(Array::new());
        }
        let ScriptValue::Array(rc) = slot else {
            return Err(format!("{} is not an array", slot.type_name()));
        };
        rc.make_unique();
        let mut arr = rc.write();
        if rest.is_empty() {
            arr.set(k, v);
            return Ok(());
        }
        let child = arr.map.entry(k).or_insert(ScriptValue::Undefined);
        let r = Self::write_path(child, rest, v);
        if child.is_undefined() {
            arr.map.shift_remove(&k);
        }
        r
    }

    fn read_path(&mut self, mut v: ScriptValue, path: &[Key]) -> ScriptValue {
        for k in path {
            v = match &v {
                ScriptValue::Array(a) => a.get(k).unwrap_or_default(),
                _ => return ScriptValue::Undefined,
            };
        }
        self.live(v)
    }

    fn read_ref(&mut self, host: &mut H, th: &Thread, r: &Ref) -> ScriptValue {
        let base = match &r.base {
            RefBase::None => ScriptValue::Undefined,
            RefBase::Local(k) => {
                let f = th.frames.last().unwrap();
                let n = f.locals.len();
                f.locals
                    .get(n.wrapping_sub(1 + *k as usize))
                    .cloned()
                    .unwrap_or_default()
            }
            RefBase::Field(o, f) => self.get_field(host, *o, *f),
            RefBase::Game => self.game.clone(),
        };
        self.read_path(base, &r.path)
    }

    fn write_ref(
        &mut self,
        host: &mut H,
        th: &mut Thread,
        r: &Ref,
        v: ScriptValue,
    ) -> Result<(), String> {
        match &r.base {
            RefBase::None => Err("assignment with no target".into()),
            RefBase::Local(k) => {
                let f = th.frames.last_mut().unwrap();
                let n = f.locals.len();
                let slot = f
                    .locals
                    .get_mut(n.wrapping_sub(1 + *k as usize))
                    .ok_or("local variable out of range")?;
                Self::write_path(slot, &r.path, v)
            }
            RefBase::Field(o, f) => {
                if r.path.is_empty() {
                    self.set_field(host, *o, *f, v);
                    return Ok(());
                }
                let mut cur = self.get_field(host, *o, *f);
                // Take the script field out so the array is not shared.
                self.set_raw_field(*o, *f, ScriptValue::Undefined);
                let res = Self::write_path(&mut cur, &r.path, v);
                self.set_field(host, *o, *f, cur);
                res
            }
            RefBase::Game => {
                let mut g = std::mem::take(&mut self.game);
                let res = Self::write_path(&mut g, &r.path, v);
                self.game = g;
                res
            }
        }
    }

    fn pop_args(th: &mut Thread) -> Vec<ScriptValue> {
        let mut args = Vec::new();
        while let Some(v) = th.stack.pop() {
            if matches!(v, ScriptValue::Marker) {
                return args;
            }
            args.push(v);
        }
        args
    }

    /// Log the next `count` calls of each named builtin (both forms).
    /// Log every call of these script functions (by bare name).
    pub fn trace_functions(&mut self, names: &str) {
        for n in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let want = n.to_ascii_lowercase();
            for (i, f) in self.program.functions.iter().enumerate() {
                if self.strings.get(f.name) == want {
                    self.ftrace.insert(i as u32);
                }
            }
        }
    }

    pub fn trace_builtins(&mut self, names: &str, limit: u32) {
        for n in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let Some(s) = self.strings.find(&n.to_ascii_lowercase()) else {
                continue;
            };
            for method in [false, true] {
                if let Some(&id) = self.program.builtin_index.get(&(s, method)) {
                    self.trace.insert(id, limit);
                }
            }
        }
    }

    fn call_native(&mut self, host: &mut H, id: u32, self_: &ScriptValue, args: &[ScriptValue]) -> ScriptValue {
        if let Some(left) = self.trace.get_mut(&id)
            && *left > 0
        {
            *left -= 1;
            let r = self.call_native_inner(host, id, self_, args);
            let (n, m) = self.program.builtins[id as usize];
            let a: Vec<String> = args.iter().map(|a| self.to_text(a)).collect();
            let who = match self_ {
                ScriptValue::Object(o) => match self.kind(*o) {
                    Some(ObjKind::Entity(e)) => format!("ent{e} "),
                    Some(ObjKind::Level) => "level ".to_owned(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            let line = format!(
                "trace {who}{}{}({}) = {}",
                if m { "." } else { "" },
                self.strings.get(n),
                a.join(", "),
                self.to_text(&r)
            );
            self.messages.push(line);
            return r;
        }
        if self.profile.is_some() {
            let clock = std::time::Instant::now();
            let r = self.call_native_inner(host, id, self_, args);
            let ms = clock.elapsed().as_secs_f64() * 1000.0;
            if let Some(p) = self.profile.as_mut() {
                let row = p.natives.entry(id).or_default();
                row.0 += ms;
                row.1 += 1;
            }
            return r;
        }
        self.call_native_inner(host, id, self_, args)
    }

    /// The costliest script functions and builtins since the last report
    /// (ms in all, calls), and a fresh count; `None` while not profiling.
    /// Times include what they call.
    pub fn profile_report(&mut self, top: usize) -> Option<String> {
        let p = std::mem::take(self.profile.as_mut()?);
        let program = self.program.clone();
        let mut funcs: Vec<(u32, (f64, u32))> = p.funcs.into_iter().collect();
        funcs.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        let mut natives: Vec<(u32, (f64, u32))> = p.natives.into_iter().collect();
        natives.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
        let f: Vec<String> = funcs
            .iter()
            .take(top)
            .filter_map(|(f, (ms, n))| {
                let func = program.functions.get(*f as usize)?;
                let script = program.scripts[func.script as usize].name.rsplit('/').next().unwrap_or("");
                Some(format!("{script}::{} {ms:.1} ({n})", self.strings.get(func.name)))
            })
            .collect();
        let b: Vec<String> = natives
            .iter()
            .take(top)
            .filter_map(|(key, (ms, n))| {
                let (name, _) = program.builtins.get(*key as usize)?;
                Some(format!("{} {ms:.1} ({n})", self.strings.get(*name)))
            })
            .collect();
        Some(format!("scripts: {}; builtins: {}", f.join(", "), b.join(", ")))
    }

    fn call_native_inner(&mut self, host: &mut H, id: u32, self_: &ScriptValue, args: &[ScriptValue]) -> ScriptValue {
        match self.natives.get(id as usize).copied().flatten() {
            Some(f) => match f(self, host, self_, args) {
                Ok(v) => v,
                Err(e) => {
                    let (n, m) = self.program.builtins[id as usize];
                    let text = format!(
                        "{}{}: {e}",
                        if m { "method " } else { "" },
                        self.strings.get(n)
                    );
                    self.report_once(text);
                    ScriptValue::Undefined
                }
            },
            None => match self.hooks.unbound {
                Some(hook) => hook(self, host, id, self_, args),
                None => {
                    let (name, method) = self.program.builtins[id as usize];
                    let n = self.strings.get(name).to_owned();
                    self.report_once(format!(
                        "unbound builtin {}{n}",
                        if method { "method " } else { "" }
                    ));
                    ScriptValue::Undefined
                }
            },
        }
    }

    /// Call a function value now with `self_` and `args` (from a builtin):
    /// a script function runs as a new thread until it yields.
    pub fn call_value(&mut self, host: &mut H, f: &ScriptValue, self_: ScriptValue, args: Vec<ScriptValue>) -> ScriptValue {
        match f {
            ScriptValue::Func(FuncRef::Script(fi)) => {
                self.spawn(host, *fi, self_, args);
                ScriptValue::Undefined
            }
            ScriptValue::Func(FuncRef::Builtin(b)) => self.call_native(host, *b, &self_, &args),
            _ => ScriptValue::Undefined,
        }
    }

    fn register(&mut self, t: ThreadId, obj: ObjRef, name: Str, kind: WaiterKind) {
        self.waiters
            .entry((obj.index, name))
            .or_default()
            .push(Waiter {
                thread: t,
                obj_gen: obj.generation,
                kind,
            });
    }

    /// bo2mp: BO2's server VM rounds a wait to frames, a non-zero one to at
    /// least one, but `wait 0` (exactly) resumes this same frame. So
    /// `fadetoblackforxsec(0, 0, 0, 0)` (the RC-XD's detonate, whose dvars
    /// BO2 no longer sets) never shows its black.
    fn wait_frames(seconds: f32) -> i64 {
        if seconds == 0.0 {
            return 0;
        }
        let frames = (seconds * 20.0 + 0.5).floor() as i64;
        frames.max(1)
    }

    fn timer(&mut self, t: ThreadId, th: &mut Thread, ms: i64) {
        self.upcoming_serial += 1;
        th.timer_seq = self.upcoming_serial;
        th.state = State::Timer;
        self.timers.push(Reverse((
            self.clock_stamp_ms + ms,
            !self.upcoming_serial,
            t.index,
            t.stamp,
        )));
    }

    /// The thread's call chain, outermost first: `script::func@addr > ...`.
    fn frame_chain(&self, program: &Program, th: &Thread) -> String {
        let chain: Vec<String> = th
            .frames
            .iter()
            .map(|fr| {
                let func = &program.functions[fr.func as usize];
                let at = func
                    .addrs
                    .get(fr.pc.saturating_sub(1) as usize)
                    .copied()
                    .unwrap_or(0);
                format!(
                    "{}::{}@{at:#x}",
                    program.scripts[func.script as usize].name,
                    self.strings.get(func.name)
                )
            })
            .collect();
        chain.join(" > ")
    }

    #[allow(clippy::too_many_lines, reason = "one arm per opcode")]
    fn interp(&mut self, host: &mut H, t: ThreadId, th: &mut Thread, program: &Program) -> Outcome {
        let mut r = Ref::default();
        let mut obj: Option<ObjRef> = None;
        let mut steps: u64 = 0;
        let resumed_at = std::time::Instant::now();
        let mut slow_named = false;
        RESUMES.fetch_add(1, Relaxed);
        macro_rules! pop {
            () => {
                th.stack.pop().unwrap_or_default()
            };
        }
        macro_rules! fail {
            ($($parts:tt)*) => {{
                let m = format!($($parts)*);
                self.fail_thread(program, th, &m);
            }};
        }
        loop {
            if let Some(d) = self.pending_unwind.remove(&t) {
                if !self.apply_unwind(t, th, d) {
                    return Outcome::Done;
                }
            }
            steps += 1;
            if steps & 0xFFF == 0 {
                STEPS_RUN.fetch_add(0x1000, Relaxed);
                SAMPLE_FUNC.store(th.frames.last().map_or(u32::MAX, |f| f.func), Relaxed);
                SAMPLE_ROOT.store(th.frames.first().map_or(u32::MAX, |f| f.func), Relaxed);
            }
            // Slow-thread watchdog: a thread that has run 2 s without waiting
            // names itself on stderr once (the step limit may be far off when
            // its builtins are slow, and messages only drain after the tick).
            if steps & 0xFFFF == 0 && !slow_named && resumed_at.elapsed().as_secs() >= 2 {
                slow_named = true;
                eprintln!(
                    "bo2zm gsc: thread running {} ms, {steps} instructions without waiting: {}",
                    resumed_at.elapsed().as_millis(),
                    self.frame_chain(program, th)
                );
            }
            if steps > STEP_LIMIT {
                let chain = self.frame_chain(program, th);
                fail!("thread ran {STEP_LIMIT} instructions without waiting; ended ({chain})");
                return Outcome::Done;
            }
            let Some(frame) = th.frames.last_mut() else {
                return Outcome::Done;
            };
            let func = &program.functions[frame.func as usize];
            let pc = frame.pc as usize;
            let op = func.code.get(pc).copied().unwrap_or(Op::End);
            frame.pc += 1;
            match op {
                Op::End | Op::Return => {
                    let v = if matches!(op, Op::Return) {
                        pop!()
                    } else {
                        ScriptValue::Undefined
                    };
                    let f = th.frames.pop().unwrap();
                    th.stack.truncate(f.base as usize);
                    let depth = th.frames.len() as u32;
                    if th.endons.iter().any(|e| e.2 >= depth) {
                        let mut dropped_endons = Vec::new();
                        th.endons.retain(|e| {
                            if e.2 >= depth {
                                dropped_endons.push((e.0, e.1));
                                false
                            } else {
                                true
                            }
                        });
                        for (o, n) in dropped_endons {
                            if let Some(list) = self.waiters.get_mut(&(o.index, n)) {
                                list.retain(|w| {
                                    !(w.thread == t
                                        && matches!(w.kind, WaiterKind::Endon(d) if d >= depth))
                                });
                                if list.is_empty() {
                                    self.waiters.remove(&(o.index, n));
                                }
                            }
                        }
                    }
                    if th.frames.is_empty() {
                        return Outcome::Done;
                    }
                    th.stack.push(v);
                    obj = None;
                }
                Op::Undefined => th.stack.push(ScriptValue::Undefined),
                Op::Int(i) => th.stack.push(ScriptValue::Int(i)),
                Op::Real(f) => th.stack.push(ScriptValue::Float(f)),
                Op::Str(s) => th.stack.push(ScriptValue::Str(s)),
                Op::IStr(s) => th.stack.push(ScriptValue::IStr(s)),
                Op::Vector(i) => th.stack.push(ScriptValue::Vec3(program.vectors[i as usize])),
                Op::Hash(h) => th.stack.push(ScriptValue::Hash(h)),
                Op::LevelObject => obj = Some(self.level),
                Op::AnimObject => obj = Some(self.anim),
                Op::SelfObject => {
                    let s = th.frames.last().unwrap().self_.clone();
                    match s {
                        ScriptValue::Object(o) => obj = Some(o),
                        other => {
                            obj = None;
                            fail!("self is {}, not an object", other.type_name());
                        }
                    }
                }
                Op::SelfValue => {
                    let s = th.frames.last().unwrap().self_.clone();
                    let s = self.live(s);
                    th.stack.push(s);
                }
                Op::Level => th.stack.push(ScriptValue::Object(self.level)),
                Op::Game => th.stack.push(self.game.clone()),
                Op::AnimValue => th.stack.push(ScriptValue::Object(self.anim)),
                Op::GameRef => {
                    r.base = RefBase::Game;
                    r.path.clear();
                }
                Op::Anim(i) => th.stack.push(ScriptValue::Anim(i)),
                Op::AnimTree(i) => th.stack.push(ScriptValue::AnimTree(i)),
                Op::FuncRef(site) => th
                    .stack
                    .push(ScriptValue::Func(program.sites[site as usize].callee)),
                Op::Locals(n) => {
                    let params = usize::from(func.params);
                    let frame = th.frames.last_mut().unwrap();
                    let args = std::mem::take(&mut frame.args);
                    let mut locals = vec![ScriptValue::Undefined; usize::from(n)];
                    for (i, a) in args
                        .into_iter()
                        .enumerate()
                        .take(params.min(usize::from(n)))
                    {
                        locals[i] = a;
                    }
                    frame.locals = locals;
                }
                Op::CheckClearParams => {
                    th.frames.last_mut().unwrap().args.clear();
                }
                Op::EvalLocal(k) => {
                    let f = th.frames.last().unwrap();
                    let n = f.locals.len();
                    let v = f
                        .locals
                        .get(n.wrapping_sub(1 + k as usize))
                        .cloned()
                        .unwrap_or_default();
                    let v = self.live(v);
                    th.stack.push(v);
                }
                Op::EvalLocalRef(k) | Op::EvalLocalArrayRef(k) => {
                    r.base = RefBase::Local(k);
                    r.path.clear();
                }
                Op::SetLocal(k) => {
                    let v = pop!();
                    let f = th.frames.last_mut().unwrap();
                    let n = f.locals.len();
                    if let Some(slot) = f.locals.get_mut(n.wrapping_sub(1 + k as usize)) {
                        *slot = v;
                    }
                }
                Op::EvalArray => {
                    let a = pop!();
                    let k = pop!();
                    match self.index(&a, &k) {
                        Ok(v) => th.stack.push(v),
                        Err(e) => {
                            th.stack.push(ScriptValue::Undefined);
                            if !a.is_undefined() {
                                fail!("{e}");
                            }
                        }
                    }
                }
                Op::EvalArrayRef => {
                    let k = pop!();
                    match self.to_key(&k) {
                        Ok(k) => r.path.push(k),
                        Err(e) => {
                            fail!("{e}");
                            r.base = RefBase::None;
                        }
                    }
                }
                Op::ClearArray => {
                    let k = pop!();
                    if let Ok(k) = self.to_key(&k) {
                        let cur = self.read_ref(host, th, &r);
                        if let ScriptValue::Array(mut rc) = cur
                            && rc.get(&k).is_some()
                        {
                            rc.make_unique();
                            rc.write().set(k, ScriptValue::Undefined);
                            let rr = r.clone();
                            let _ = self.write_ref(host, th, &rr, ScriptValue::Array(rc));
                        }
                    }
                }
                Op::EmptyArray => th.stack.push(ScriptValue::array(Array::new())),
                Op::EvalField(f) => {
                    let v = match obj {
                        Some(o) => self.get_field(host, o, f),
                        None => ScriptValue::Undefined,
                    };
                    let v = self.live(v);
                    th.stack.push(v);
                }
                Op::EvalFieldRef(f) => {
                    match obj {
                        Some(o) => r.base = RefBase::Field(o, f),
                        None => {
                            r.base = RefBase::None;
                            fail!("field '{}' of a non-object", self.strings.get(f));
                        }
                    }
                    r.path.clear();
                }
                Op::ClearField(f) => {
                    if let Some(o) = obj {
                        self.set_field(host, o, f, ScriptValue::Undefined);
                    }
                }
                Op::WaittillVar(k) => {
                    let v = if matches!(th.stack.last(), Some(ScriptValue::Marker) | None) {
                        ScriptValue::Undefined
                    } else {
                        pop!()
                    };
                    let f = th.frames.last_mut().unwrap();
                    let n = f.locals.len();
                    if let Some(slot) = f.locals.get_mut(n.wrapping_sub(1 + k as usize)) {
                        *slot = v;
                    }
                }
                Op::ClearParams => {
                    while let Some(v) = th.stack.pop() {
                        if matches!(v, ScriptValue::Marker) {
                            break;
                        }
                    }
                }
                Op::Set => {
                    let v = pop!();
                    let rr = std::mem::take(&mut r);
                    if let Err(e) = self.write_ref(host, th, &rr, v) {
                        fail!("{e}");
                    }
                    r = rr;
                }
                Op::Wait | Op::RealWait => {
                    let v = pop!();
                    let secs = v.as_float().unwrap_or_else(|| 0.05);
                    let ms = Self::wait_frames(secs) * FRAME_MS;
                    self.timer(t, th, ms);
                    return Outcome::Yield;
                }
                Op::FrameEnd => {
                    th.state = State::FrameEnd;
                    self.frame_end.push(t);
                    return Outcome::Yield;
                }
                Op::PreCall | Op::VoidCodePos => th.stack.push(ScriptValue::Marker),
                Op::Call(site, kind) => {
                    let callee = program.sites[site as usize].callee;
                    let self_v = if kind.method() {
                        pop!()
                    } else {
                        th.frames.last().unwrap().self_.clone()
                    };
                    let args = Self::pop_args(th);
                    if let Some(o) = self.call(host, t, th, program, callee, kind, self_v, args) {
                        return o;
                    }
                    obj = None;
                }
                Op::CallPointer(kind) => {
                    let f = pop!();
                    let self_v = if kind.method() {
                        pop!()
                    } else {
                        th.frames.last().unwrap().self_.clone()
                    };
                    let args = Self::pop_args(th);
                    match f {
                        ScriptValue::Func(callee) => {
                            if let Some(o) =
                                self.call(host, t, th, program, callee, kind, self_v, args)
                            {
                                return o;
                            }
                        }
                        other => {
                            fail!("call through a {}, not a function", other.type_name());
                            th.stack.push(ScriptValue::Undefined);
                        }
                    }
                    obj = None;
                }
                Op::DecTop => {
                    th.stack.pop();
                }
                Op::CastObject => {
                    let v = pop!();
                    match v {
                        ScriptValue::Object(o) if self.alive(o) || self.dying(o) => obj = Some(o),
                        other => {
                            obj = None;
                            if !matches!(other, ScriptValue::Object(_)) {
                                fail!("{} is not an object", other.type_name());
                            }
                        }
                    }
                }
                Op::CastBool => {
                    let v = pop!();
                    th.stack.push(ScriptValue::bool(truthy(&v)));
                }
                Op::Not => {
                    let v = pop!();
                    th.stack.push(ScriptValue::bool(!truthy(&v)));
                }
                Op::BitNot => {
                    let v = pop!();
                    th.stack.push(ScriptValue::Int(!v.as_int().unwrap_or(0)));
                }
                Op::JumpFalse(to) => {
                    let v = pop!();
                    if !truthy(&v) {
                        th.frames.last_mut().unwrap().pc = to;
                    }
                }
                Op::JumpTrue(to) => {
                    let v = pop!();
                    if truthy(&v) {
                        th.frames.last_mut().unwrap().pc = to;
                    }
                }
                Op::JumpFalseExpr(to) => {
                    if th.stack.last().is_some_and(|v| !truthy(v)) {
                        th.frames.last_mut().unwrap().pc = to;
                    } else {
                        th.stack.pop();
                    }
                }
                Op::JumpTrueExpr(to) => {
                    if th.stack.last().is_some_and(truthy) {
                        th.frames.last_mut().unwrap().pc = to;
                    } else {
                        th.stack.pop();
                    }
                }
                Op::Jump(to) => th.frames.last_mut().unwrap().pc = to,
                Op::Inc | Op::Dec => {
                    let cur = self.read_ref(host, th, &r);
                    let d = if matches!(op, Op::Inc) { 1 } else { -1 };
                    let nv = match cur {
                        ScriptValue::Int(i) => ScriptValue::Int(i.wrapping_add(d)),
                        ScriptValue::Float(f) => ScriptValue::Float(f + d as f32),
                        ScriptValue::Undefined => {
                            fail!("++/-- on undefined");
                            ScriptValue::Int(d)
                        }
                        other => {
                            fail!("++/-- on a {}", other.type_name());
                            other
                        }
                    };
                    let rr = r.clone();
                    if let Err(e) = self.write_ref(host, th, &rr, nv) {
                        fail!("{e}");
                    }
                }
                Op::Bin(b) => {
                    let y = pop!();
                    let x = pop!();
                    // bo2mp: BO2 says "divide by 0" and its thread goes on
                    // with a 0 ((0, 0, 0) for a vector), not undefined.
                    if let Some(z) = Self::by_zero(b, &x, &y) {
                        fail!("divide by 0");
                        th.stack.push(z);
                    } else {
                        match self.binop(b, x, y) {
                            Ok(v) => th.stack.push(v),
                            Err(e) => {
                                fail!("{e}");
                                th.stack.push(ScriptValue::Undefined);
                            }
                        }
                    }
                }
                Op::Size => {
                    let v = pop!();
                    let n = match &v {
                        ScriptValue::Array(a) => a.len() as i32,
                        ScriptValue::Str(s) | ScriptValue::IStr(s) => {
                            self.strings.get(*s).chars().count() as i32
                        }
                        ScriptValue::Undefined => {
                            fail!("size of undefined");
                            0
                        }
                        other => {
                            fail!("size of a {}", other.type_name());
                            0
                        }
                    };
                    th.stack.push(ScriptValue::Int(n));
                }
                Op::Waittill | Op::WaittillMatch(_) => {
                    let o = pop!();
                    let n = pop!();
                    let mut vals = Vec::new();
                    if let Op::WaittillMatch(k) = op {
                        for _ in 0..k {
                            vals.push(pop!());
                        }
                    }
                    match (o, n.as_str_id()) {
                        (ScriptValue::Object(o), Some(n)) if self.alive(o) => {
                            let kind = if matches!(op, Op::Waittill) {
                                WaiterKind::Waittill
                            } else {
                                WaiterKind::Match(vals)
                            };
                            self.register(t, o, n, kind);
                            th.waiting_on = Some((o, n));
                            th.state = State::Notify;
                            return Outcome::Yield;
                        }
                        (ScriptValue::Object(_), Some(..)) => {
                            // Waiting on a deleted object ends the thread.
                            return Outcome::Done;
                        }
                        (o, n) => {
                            fail!(
                                "waittill on {} for {:?}",
                                o.type_name(),
                                n.map(|s| self.strings.get(s).to_owned())
                            );
                            th.stack.push(ScriptValue::Marker);
                        }
                    }
                }
                Op::Notify => {
                    let o = pop!();
                    let n = pop!();
                    let args = Self::pop_args(th);
                    match (o, n.as_str_id()) {
                        (ScriptValue::Object(o), Some(n)) => {
                            // BO2's notify op (VM_Notify): endons end at once, the
                            // woken threads go on the current time's list and run
                            // after this one waits. Buried's Head Chopper reads
                            // is_slicing after "end" and must see it cleared.
                            if self.alive(o) {
                                let was = std::mem::replace(&mut self.queue_wakes, true);
                                self.notify(host, o, n, &args);
                                self.queue_wakes = was;
                            }
                        }
                        // A thing this thread just deleted reads as undefined;
                        // BO2 goes on past the notify (_zm_powerups: powerup_delete
                        // then notify "powerup_grabbed").
                        (ScriptValue::Undefined, Some(..)) => {}
                        (o, _) => fail!("notify on a {}", o.type_name()),
                    }
                }
                Op::Endon => {
                    let o = pop!();
                    let n = pop!();
                    match (o, n.as_str_id()) {
                        (ScriptValue::Object(o), Some(n)) if self.alive(o) => {
                            let depth = th.frames.len() as u32 - 1;
                            th.endons.push((o, n, depth));
                            self.register(t, o, n, WaiterKind::Endon(depth));
                        }
                        (ScriptValue::Object(_), Some(_)) => return Outcome::Done,
                        (o, _) => fail!("endon on a {}", o.type_name()),
                    }
                }
                Op::Switch(i) => {
                    let v = pop!();
                    let table = &program.switches[i as usize];
                    let hit = match &v {
                        ScriptValue::Int(n) => table.ints.get(n).copied(),
                        ScriptValue::Float(f) if f.fract() == 0.0 => {
                            table.ints.get(&(*f as i32)).copied()
                        }
                        ScriptValue::Str(s) | ScriptValue::IStr(s) => table.strs.get(s).copied(),
                        _ => None,
                    };
                    let to = hit.or(table.default).unwrap_or(table.end_addr);
                    th.frames.last_mut().unwrap().pc = to;
                }
                Op::MakeVector => {
                    let x = pop!();
                    let y = pop!();
                    let z = pop!();
                    match (x.as_float(), y.as_float(), z.as_float()) {
                        (Some(x), Some(y), Some(z)) => th.stack.push(ScriptValue::Vec3([x, y, z])),
                        _ => {
                            fail!("vector of non-numbers");
                            th.stack.push(ScriptValue::Undefined);
                        }
                    }
                }
                Op::VectorConst(flags) => {
                    let c = |b: u8| match b & 3 {
                        1 => -1.0,
                        2 => 1.0,
                        _ => 0.0,
                    };
                    th.stack
                        .push(ScriptValue::Vec3([c(flags >> 4), c(flags >> 2), c(flags)]));
                }
                Op::IsDefined => {
                    let v = pop!();
                    th.stack.push(ScriptValue::bool(self.is_defined(&v)));
                }
                Op::VectorScale => {
                    let v = pop!();
                    let s = pop!();
                    match (v.as_vec3(), s.as_float()) {
                        (Some(v), Some(s)) => {
                            th.stack.push(ScriptValue::Vec3([v[0] * s, v[1] * s, v[2] * s]))
                        }
                        _ => {
                            fail!("vectorscale of {} by {}", v.type_name(), s.type_name());
                            th.stack.push(ScriptValue::Undefined);
                        }
                    }
                }
                Op::AnglesToUp | Op::AnglesToRight | Op::AnglesToForward => {
                    let v = pop!();
                    match v.as_vec3() {
                        Some(angles) => {
                            let (f, rgt, up) = crate::math::facing_axes(angles);
                            th.stack.push(ScriptValue::Vec3(match op {
                                Op::AnglesToUp => up,
                                Op::AnglesToRight => rgt,
                                _ => f,
                            }));
                        }
                        None => {
                            fail!("angles of a {}", v.type_name());
                            th.stack.push(ScriptValue::Undefined);
                        }
                    }
                }
                Op::AngleClamp180 => {
                    let v = pop!();
                    let a = v.as_float().unwrap_or(0.0);
                    th.stack.push(ScriptValue::Float(crate::math::angle_clamp180(a)));
                }
                Op::VectorToAngles => {
                    let v = pop!();
                    match v.as_vec3() {
                        Some(d) => th.stack.push(ScriptValue::Vec3(crate::math::vector_to_angles(d))),
                        None => {
                            fail!("vectortoangles of a {}", v.type_name());
                            th.stack.push(ScriptValue::Undefined);
                        }
                    }
                }
                Op::Abs => {
                    let v = pop!();
                    th.stack.push(match v {
                        ScriptValue::Int(i) => ScriptValue::Int(i.wrapping_abs()),
                        ScriptValue::Float(f) => ScriptValue::Float(f.abs()),
                        _ => ScriptValue::Undefined,
                    });
                }
                Op::GetTime => th.stack.push(ScriptValue::Int(self.clock_stamp_ms as i32)),
                Op::GetDvar
                | Op::GetDvarInt
                | Op::GetDvarFloat
                | Op::GetDvarVector
                | Op::GetDvarColor(_) => {
                    let n = pop!();
                    let text = self.dvar_text(&n);
                    let v = match op {
                        Op::GetDvar => self.string(&text),
                        Op::GetDvarInt => ScriptValue::Int(crate::math::parse_int(&text)),
                        Op::GetDvarFloat => ScriptValue::Float(text.trim().parse::<f32>().unwrap_or(0.0)),
                        Op::GetDvarVector => {
                            let p: Vec<f32> = text
                                .split(char::is_whitespace)
                                .filter(|w| !w.is_empty())
                                .filter_map(|x| x.parse().ok())
                                .collect();
                            ScriptValue::Vec3([
                                p.first().copied().unwrap_or(0.0),
                                p.get(1).copied().unwrap_or(0.0),
                                p.get(2).copied().unwrap_or(0.0),
                            ])
                        }
                        Op::GetDvarColor(c) => {
                            let p: Vec<f32> = text
                                .split(char::is_whitespace)
                                .filter(|w| !w.is_empty())
                                .filter_map(|x| x.parse().ok())
                                .collect();
                            ScriptValue::Float(p.get(usize::from(c)).copied().unwrap_or(if c == 3 {
                                1.0
                            } else {
                                0.0
                            }))
                        }
                        _ => ScriptValue::Undefined,
                    };
                    th.stack.push(v);
                }
                Op::FirstKey => {
                    let a = pop!();
                    let v = match &a {
                        ScriptValue::Array(a) => a.read().first_key().map(Key::value).unwrap_or_default(),
                        ScriptValue::Undefined => {
                            fail!("foreach over undefined");
                            ScriptValue::Undefined
                        }
                        other => {
                            fail!("foreach over a {}", other.type_name());
                            ScriptValue::Undefined
                        }
                    };
                    th.stack.push(v);
                }
                Op::NextKey => {
                    let a = pop!();
                    let k = pop!();
                    let v = match (&a, self.to_key(&k)) {
                        (ScriptValue::Array(a), Ok(k)) => {
                            a.read().next_key(&k).map(Key::value).unwrap_or_default()
                        }
                        _ => ScriptValue::Undefined,
                    };
                    th.stack.push(v);
                }
                Op::Nop => {}
                Op::Bad(c) => {
                    fail!("unsupported opcode {}", crate::object::op_name(c));
                }
            }
        }
    }

    /// Perform a call; `Some(outcome)` if the thread must stop here.
    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        host: &mut H,
        t: ThreadId,
        th: &mut Thread,
        program: &Program,
        callee: FuncRef,
        kind: CallKind,
        self_v: ScriptValue,
        args: Vec<ScriptValue>,
    ) -> Option<Outcome> {
        match callee {
            FuncRef::Script(fi) => {
                if !self.ftrace.is_empty() && self.ftrace.contains(&fi) {
                    let func = &program.functions[fi as usize];
                    let a: Vec<String> = args.iter().map(|v| self.to_text(v)).collect();
                    // The calling function, so a trace says who asked.
                    let from = th.frames.last().map_or(String::from("-"), |f| {
                        let cf = &program.functions[f.func as usize];
                        format!(
                            "{}::{}",
                            program.scripts[cf.script as usize].name,
                            self.strings.get(cf.name)
                        )
                    });
                    let m = format!(
                        "ftrace {}::{}({}) self {} from {from}",
                        program.scripts[func.script as usize].name,
                        self.strings.get(func.name),
                        a.join(", "),
                        self.to_text(&self_v)
                    );
                    self.messages.push(m);
                }
                if kind.thread() {
                    self.spawn(host, fi, self_v, args);
                    th.stack.push(ScriptValue::Undefined);
                } else {
                    if th.frames.len() >= MAX_FRAMES {
                        self.fail_thread(program, th, "call stack too deep");
                        th.stack.push(ScriptValue::Undefined);
                        return None;
                    }
                    let base = th.stack.len() as u32;
                    th.frames.push(Frame {
                        func: fi,
                        pc: 0,
                        locals: Vec::new(),
                        self_: self_v,
                        base,
                        args,
                    });
                }
            }
            FuncRef::Builtin(b) => {
                NATIVE_CALLER.store(th.frames.last().map_or(u32::MAX, |f| f.func), Relaxed);
                NATIVE_NOW.store(b + 1, Relaxed);
                let v = self.call_native(host, b, &self_v, &args);
                NATIVE_NOW.store(0, Relaxed);
                th.stack
                    .push(if kind.thread() { ScriptValue::Undefined } else { v });
            }
            FuncRef::Missing(n) => {
                let m = format!("missing function {}", self.strings.get(n));
                self.report_once(m);
                th.stack.push(ScriptValue::Undefined);
            }
        }
        let _ = t;
        None
    }

    /// One line per waiting thread: where it is and what it waits for
    /// (debugging: "what are the scripts stuck on?").
    pub fn describe_threads(&self) -> Vec<String> {
        let mut out = Vec::new();
        for th in self.threads.iter().flatten() {
            let Some(f) = th.frames.last() else { continue };
            let func = &self.program.functions[f.func as usize];
            let script = &self.program.scripts[func.script as usize].name;
            let chain: Vec<String> = th
                .frames
                .iter()
                .map(|fr| {
                    self.strings
                        .get(self.program.functions[fr.func as usize].name)
                        .to_owned()
                })
                .collect();
            let what = match th.state {
                State::Timer => "wait".to_owned(),
                State::FrameEnd => "waittillframeend".to_owned(),
                State::Notify => match th.waiting_on {
                    Some((o, n)) => {
                        let who = match self.kind(o) {
                            Some(ObjKind::Level) => "level".to_owned(),
                            Some(ObjKind::Entity(e)) => format!("ent{e}"),
                            Some(k) => format!("{k:?}"),
                            None => "dead".to_owned(),
                        };
                        format!("waittill {who} \"{}\"", self.strings.get(n))
                    }
                    None => "notify?".to_owned(),
                },
                s => format!("{s:?}"),
            };
            out.push(format!(
                "{script}::{} [{}] {what}",
                self.strings.get(func.name),
                chain.join(" > ")
            ));
        }
        out.sort();
        out
    }

    /// The frame-0 self of a thread (tests and tools).
    pub fn thread_self(&self, t: ThreadId) -> Option<ScriptValue> {
        self.threads
            .get(t.index as usize)?
            .as_ref()
            .and_then(|th| th.frames.first())
            .map(|f| f.self_.clone())
    }

    /// A dvar's text by name or by `#"name"` hash ("" when unset).
    pub fn dvar_text(&self, name: &ScriptValue) -> String {
        match name {
            ScriptValue::Hash(h) => self
                .dvars
                .iter()
                .find(|(k, _)| hash_name(k) == *h)
                .map(|(_, v)| v.clone())
                .unwrap_or_default(),
            v => self
                .dvars
                .get(&self.to_text(v).to_ascii_lowercase())
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// The dvar name a value names (a hash maps to a set dvar's name).
    pub fn dvar_name(&self, name: &ScriptValue) -> String {
        match name {
            ScriptValue::Hash(h) => self
                .dvars
                .keys()
                .find(|k| hash_name(k) == *h)
                .cloned()
                .unwrap_or_else(|| format!("#{h:08x}")),
            v => self.to_text(v).to_ascii_lowercase(),
        }
    }

    /// Next pseudo-random u32 (xorshift; the host may reseed `rng`).
    pub fn rand_u32(&mut self) -> u32 {
        let mut bits = self.rng;
        bits ^= bits << 13;
        bits ^= bits >> 7;
        bits ^= bits << 17;
        self.rng = bits;
        (bits >> 32) as u32
    }

    /// A pseudo-random float in [0, 1).
    pub fn rand_unit(&mut self) -> f32 {
        (self.rand_u32() >> 8) as f32 / 16_777_216.0
    }
}
