use std::collections::BTreeMap;

use super::eval::run_limited;
use super::*;

const STRUCT: &str = "initstructs()\n{\n\tlevel.struct = [];\n}\n\ncreatestruct()\n{\n\tstruct = spawnstruct();\n\tlevel.struct[level.struct.size] = struct;\n\treturn struct;\n}\n";

const KILLED_PARAMS: &str = "einflictor, eattacker, idamage, smeansofdeath, sweapon, vdir, shitloc, timeoffset, deathanimduration";
const HIT_PARAMS: &str = "einflictor, eattacker, idamage, idflags, smeansofdeath, sweapon, vpoint, vdir, shitloc, timeoffset";

pub(super) fn t6_catalog() -> Catalog {
    Catalog::new(
        Realm::T6,
        &[
            Builtin::new(Namespace::Function, "getdvar"),
            Builtin::new(Namespace::Function, "spawnstruct"),
            Builtin::new(Namespace::Method, "giveweapon"),
            Builtin::new(Namespace::Method, "setoffhandprimaryclass"),
            Builtin::new(Namespace::Method, "setweaponammoclip"),
            Builtin::new(Namespace::Method, "setspawnweapon"),
            Builtin::new(Namespace::Method, "spawn"),
        ],
    )
}

/// The start-up scripts of one stand-in map, as the engine writes them.
pub(super) fn stub_sources(zm: bool) -> BTreeMap<String, String> {
    let loadout = if zm {
        "\tself giveweapon(\"m1911_zm_mp\");\n\tself giveweapon(\"frag_grenade_zm_mp\");\n\tself setweaponammoclip(\"frag_grenade_zm_mp\", 2);\n\tself setoffhandprimaryclass(\"frag\");\n\tself setspawnweapon(\"m1911_zm_mp\");\n"
    } else {
        ""
    };
    let callbacks = format!(
        "codecallback_startgametype()\n{{\n\tlevel.gametypestarted = true;\n\tlevel notify(\"prematch_over\");\n}}\n\n\
         codecallback_playerconnect()\n{{\n\tif (getdvar(\"bo2zm_t6\") == \"1\")\n\t\treturn;\n\tself.sessionteam = \"allies\";\n\tself.sessionstate = \"playing\";\n\tself spawn((10.000, 20.000, 30.000), (0, 90.000, 0));\n{loadout}}}\n\n\
         codecallback_playerdisconnect()\n{{\n}}\n\n\
         codecallback_playerdamage({HIT_PARAMS})\n{{\n}}\n\n\
         codecallback_playerkilled({KILLED_PARAMS})\n{{\n}}\n\n\
         codecallback_playerlaststand({KILLED_PARAMS})\n{{\n}}\n\n\
         codecallback_vehicledamage({HIT_PARAMS}, modelindex, partname)\n{{\n}}\n"
    );
    [
        ("codescripts/delete", "main()\n{\n}\n".to_owned()),
        ("codescripts/struct", STRUCT.to_owned()),
        ("maps/mp/gametypes/_callbacksetup", callbacks),
        ("maps/mp/gametypes/dm", "main()\n{\n}\n".to_owned()),
    ]
    .into_iter()
    .map(|(called, text)| (called.to_owned(), text))
    .collect()
}

pub(super) const STUB_ROOTS: [&str; 4] = [
    "codescripts/delete",
    "codescripts/struct",
    "maps/mp/gametypes/_callbacksetup",
    "maps/mp/gametypes/dm",
];

#[derive(Default)]
struct Mock {
    heap: Heap,
    names: Vec<&'static str>,
    sent: Vec<(String, Value, Vec<Value>)>,
    raised: Vec<(Value, String)>,
    owned: BTreeMap<String, Value>,
    faults: Vec<Fault>,
}

impl Mock {
    fn for_program(program: &Program) -> Self {
        Self {
            names: program.natives().iter().map(|b| b.name).collect(),
            ..Self::default()
        }
    }
}

impl ScriptWorld for Mock {
    fn heap(&mut self) -> &mut Heap {
        &mut self.heap
    }

    fn native(&mut self, slot_at: usize, receiver: &Value, args: &[Value]) -> Result<Value, String> {
        let name = self.names[slot_at];
        self.sent.push((name.to_owned(), receiver.clone(), args.to_vec()));
        match name {
            "getdvar" => Ok(Value::string("")),
            "spawnstruct" => self.heap.new_object().map(Value::Object),
            _ => Ok(Value::Undefined),
        }
    }

    fn load_field(&mut self, _object: u64, name: &str) -> Option<Value> {
        self.owned.get(name).cloned()
    }

    fn store_field(&mut self, _object: u64, name: &str, setting: &Value) -> Result<bool, String> {
        if name.starts_with("session") {
            self.owned.insert(name.to_owned(), setting.clone());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn raised(&mut self, receiver: &Value, event: &str) {
        self.raised.push((receiver.clone(), event.to_owned()));
    }

    fn report(&mut self, fault: Fault) {
        self.faults.push(fault);
    }
}

fn one_module(text: &str) -> (Program, Mock) {
    let sources: BTreeMap<String, String> = [("t/one".to_owned(), text.to_owned())].into();
    let program = Program::load(&sources, &["t/one"], &t6_catalog()).expect("module loads");
    let mock = Mock::for_program(&program);
    (program, mock)
}

#[test]
fn stub_scripts_have_the_expected_shape() {
    for (zm, natives) in [(false, 3), (true, 7)] {
        let program = Program::load(&stub_sources(zm), &STUB_ROOTS, &t6_catalog()).unwrap();
        assert_eq!(program.function_count(), 11);
        assert_eq!(program.native_count(), natives);
        assert_eq!(program.rules(), Realm::T6);
    }
}

#[test]
fn a_missing_module_is_named() {
    let mut sources = stub_sources(false);
    sources.remove("codescripts/struct");
    let fault = Program::load(&sources, &STUB_ROOTS, &t6_catalog()).unwrap_err();
    assert_eq!(fault.location.module, "codescripts/struct");
    assert_eq!(fault.location.line, 1);
    assert!(fault.message.contains("missing script module"), "{}", fault.message);
}

#[test]
fn start_notifies_the_level() {
    let program = Program::load(&stub_sources(false), &STUB_ROOTS, &t6_catalog()).unwrap();
    let mut mock = Mock::for_program(&program);
    let entry = program
        .find("maps/mp/gametypes/_callbacksetup::codecallback_startgametype")
        .unwrap();
    assert_eq!(play_call(&program, &mut mock, entry, Value::level(), vec![]), Some(Value::Undefined));
    assert_eq!(mock.raised, vec![(Value::level(), "prematch_over".to_owned())]);
    assert_eq!(mock.heap.field(LEVEL, "gametypestarted"), Some(&Value::Int(1)));
    assert!(mock.faults.is_empty());
}

#[test]
fn connect_spawns_the_player() {
    let program = Program::load(&stub_sources(true), &STUB_ROOTS, &t6_catalog()).unwrap();
    let mut mock = Mock::for_program(&program);
    let entry = program
        .find("maps/mp/gametypes/_callbacksetup::codecallback_playerconnect")
        .unwrap();
    play_call(&program, &mut mock, entry, Value::Object(9), vec![]);
    assert!(mock.faults.is_empty(), "{:?}", mock.faults);
    assert_eq!(mock.owned["sessionteam"], Value::string("allies"));
    assert_eq!(mock.owned["sessionstate"], Value::string("playing"));
    let sent: Vec<&str> = mock.sent.iter().map(|(name, ..)| name.as_str()).collect();
    assert_eq!(
        sent,
        [
            "getdvar",
            "spawn",
            "giveweapon",
            "giveweapon",
            "setweaponammoclip",
            "setoffhandprimaryclass",
            "setspawnweapon"
        ]
    );
    let (_, receiver, args) = &mock.sent[1];
    assert_eq!(receiver, &Value::Object(9));
    assert_eq!(args[0], Value::Vector([10.0, 20.0, 30.0]));
    assert_eq!(args[1], Value::Vector([0.0, 90.0, 0.0]));
}

#[test]
fn struct_helpers_grow_the_level_list() {
    let program = Program::load(&stub_sources(false), &STUB_ROOTS, &t6_catalog()).unwrap();
    let mut mock = Mock::for_program(&program);
    let init = program.find("codescripts/struct::initstructs").unwrap();
    let make = program.find("codescripts/struct::createstruct").unwrap();
    play_call(&program, &mut mock, init, Value::level(), vec![]);
    let first = play_call(&program, &mut mock, make, Value::level(), vec![]);
    let second = play_call(&program, &mut mock, make, Value::level(), vec![]);
    assert_ne!(first, second);
    let Some(Value::Array(list)) = mock.heap.field(LEVEL, "struct").cloned() else {
        panic!("level.struct is not an array");
    };
    assert_eq!(mock.heap.array_len(list), 2);
}

#[test]
fn arithmetic_loops_and_containers() {
    let (program, mut mock) = one_module(
        "sum()\n{\n\ttotal = 0;\n\tfor (i = 1; i <= 4; i++)\n\t{\n\t\tif (i == 3)\n\t\t\tcontinue;\n\t\ttotal += i;\n\t}\n\
         \tlist = [];\n\tlist[0] = 10;\n\tlist[\"k\"] = 20;\n\tv = (1, 2, 3) * 2;\n\
         \tself.shown = \"n\" + list.size + \":\" + total;\n\tself.corner = v[2];\n\treturn v;\n}\n",
    );
    let entry = program.find("t/one::sum").unwrap();
    let me = mock.heap.new_object().unwrap();
    let out = play_call(&program, &mut mock, entry, Value::Object(me), vec![]);
    assert_eq!(out, Some(Value::Vector([2.0, 4.0, 6.0])));
    assert!(mock.faults.is_empty(), "{:?}", mock.faults);
    assert_eq!(mock.heap.field(me, "shown"), Some(&Value::string("n2:7")));
    assert_eq!(mock.heap.field(me, "corner"), Some(&Value::Float(6.0)));
}

#[test]
fn a_script_error_does_not_stop_the_function() {
    let (program, mut mock) = one_module("f()\n{\n\tx = 1 / 0;\n\tself.after = 5;\n\treturn x;\n}\n");
    let entry = program.find("t/one::f").unwrap();
    let me = mock.heap.new_object().unwrap();
    let out = play_call(&program, &mut mock, entry, Value::Object(me), vec![]);
    assert_eq!(out, Some(Value::Undefined));
    assert_eq!(mock.faults.len(), 1);
    assert!(mock.faults[0].message.contains("division by zero"));
    assert_eq!(mock.faults[0].location.function, "f");
    assert_eq!(mock.faults[0].location.line, 3);
    assert_eq!(mock.heap.field(me, "after"), Some(&Value::Int(5)));
}

#[test]
fn a_runaway_loop_is_cut_off() {
    let (program, mut mock) = one_module("f()\n{\n\twhile (true)\n\t{\n\t}\n}\n");
    let entry = program.find("t/one::f").unwrap();
    let out = run_limited(&program, &mut mock, entry, Value::level(), vec![], 1000);
    assert_eq!(out, None);
    assert_eq!(mock.faults.len(), 1);
    assert!(mock.faults[0].message.contains("potential infinite loop"));
}

#[test]
fn runaway_recursion_is_stopped() {
    let (program, mut mock) = one_module("f()\n{\n\tf();\n}\n");
    let entry = program.find("t/one::f").unwrap();
    play_call(&program, &mut mock, entry, Value::level(), vec![]);
    assert_eq!(mock.faults.len(), 1);
    assert!(mock.faults[0].message.contains("script stack overflow"));
    assert_eq!(mock.faults[0].callers.len(), 31);
}

#[test]
fn unresolved_names_are_refused_at_load() {
    let sources: BTreeMap<String, String> =
        [("t/one".to_owned(), "f()\n{\n\tnope();\n}\n".to_owned())].into();
    let fault = Program::load(&sources, &["t/one"], &t6_catalog()).unwrap_err();
    assert!(fault.message.contains("unresolved function or unknown builtin nope"));
    let none = Program::load(&sources, &[], &t6_catalog()).unwrap_err();
    assert_eq!(none.message, "no script roots");
}
