//! Builtins that need no engine: numbers, strings, arrays, structs, dvars.

use crate::math;
use crate::value::{Array, Key, ScriptValue};
use crate::vm::{Vm, truthy};

type Vec3 = [f32; 3];

fn arg<'a>(args: &'a [ScriptValue], i: usize) -> &'a ScriptValue {
    args.get(i).unwrap_or(&ScriptValue::Undefined)
}

fn num(args: &[ScriptValue], i: usize, what: &str) -> Result<f32, String> {
    arg(args, i).as_float().ok_or_else(|| {
        format!(
            "{what}: argument {} is {}, not a number",
            i + 1,
            arg(args, i).type_name()
        )
    })
}

fn vec3(args: &[ScriptValue], i: usize, what: &str) -> Result<Vec3, String> {
    arg(args, i).as_vec3().ok_or_else(|| {
        format!(
            "{what}: argument {} is {}, not a vector",
            i + 1,
            arg(args, i).type_name()
        )
    })
}

/// A copy of an array argument's contents.
fn array(args: &[ScriptValue], i: usize, what: &str) -> Result<Array, String> {
    match arg(args, i) {
        ScriptValue::Array(a) => Ok(a.snapshot()),
        bad => Err(format!(
            "{what}: argument {} is {}, not an array",
            i + 1,
            bad.type_name()
        )),
    }
}

fn text<H>(vm: &Vm<H>, args: &[ScriptValue], i: usize) -> String {
    match arg(args, i) {
        ScriptValue::Undefined => String::new(),
        v => vm.to_text(v),
    }
}

fn flag(args: &[ScriptValue], i: usize, default: bool) -> bool {
    match arg(args, i) {
        ScriptValue::Undefined => default,
        v => truthy(v),
    }
}

fn list(values: Vec<ScriptValue>) -> ScriptValue {
    let mut a = Array::new();
    for v in values {
        a.push(v);
    }
    ScriptValue::array(a)
}

/// Array values with int keys re-numbered 0.. and other keys kept, in
/// insertion order.
/// An entry `arrayremovevalue` takes out (BO2's ArrayRemoveVariable): one
/// of the same type and value. An object matches itself even once it is
/// deleted: `watchmissiledeath` takes a deleted missile out of
/// `level.missileentities` from its `death` waiter, after the free.
fn same_entry<H>(vm: &Vm<H>, entry: &ScriptValue, v: &ScriptValue) -> bool {
    match (entry, v) {
        (ScriptValue::Object(x), ScriptValue::Object(y)) => x == y,
        _ => vm.equal(entry, v),
    }
}

fn renumber(src: impl Iterator<Item = (Key, ScriptValue)>, keep_keys: bool) -> Array {
    let mut a = Array::new();
    let mut n = 0;
    for (k, v) in src {
        match k {
            Key::Int(_) if !keep_keys => {
                a.set(Key::Int(n), v);
                n += 1;
            }
            k => a.set(k, v),
        }
    }
    a
}

pub fn bind_core<H>(vm: &mut Vm<H>) {
    macro_rules! f {
        ($name:literal, $body:expr) => {
            vm.bind($name, false, $body);
        };
    }
    f!("int", |vm, _, _, a| Ok(match arg(a, 0) {
        ScriptValue::Int(i) => ScriptValue::Int(*i),
        ScriptValue::Float(f) => ScriptValue::Int(*f as i32),
        ScriptValue::Str(_) | ScriptValue::IStr(_) => ScriptValue::Int(math::parse_int(&vm.to_text(arg(a, 0)))),
        ScriptValue::Undefined => ScriptValue::Int(0),
        other => return Err(format!("int of a {}", other.type_name())),
    }));
    f!("float", |vm, _, _, a| Ok(match arg(a, 0) {
        ScriptValue::Int(i) => ScriptValue::Float(*i as f32),
        ScriptValue::Float(f) => ScriptValue::Float(*f),
        ScriptValue::Str(_) | ScriptValue::IStr(_) =>
            ScriptValue::Float(vm.to_text(arg(a, 0)).trim().parse::<f32>().unwrap_or(0.0)),
        ScriptValue::Undefined => ScriptValue::Float(0.0),
        other => return Err(format!("float of a {}", other.type_name())),
    }));
    f!("randomint", |vm, _, _, a| {
        let n = num(a, 0, "randomint")? as i32;
        Ok(ScriptValue::Int(if n <= 0 {
            0
        } else {
            (vm.rand_u32() % n as u32) as i32
        }))
    });
    f!("randomintrange", |vm, _, _, a| {
        let lo = num(a, 0, "randomintrange")? as i32;
        let hi = num(a, 1, "randomintrange")? as i32;
        Ok(ScriptValue::Int(if hi <= lo {
            lo
        } else {
            lo + (vm.rand_u32() % (hi - lo) as u32) as i32
        }))
    });
    f!("randomfloat", |vm, _, _, a| {
        let n = num(a, 0, "randomfloat")?;
        Ok(ScriptValue::Float(vm.rand_unit() * n))
    });
    f!("randomfloatrange", |vm, _, _, a| {
        let lo = num(a, 0, "randomfloatrange")?;
        let hi = num(a, 1, "randomfloatrange")?;
        Ok(ScriptValue::Float(lo + vm.rand_unit() * (hi - lo)))
    });
    f!("distance", |_, _, _, a| Ok(ScriptValue::Float(math::length(
        math::sub(vec3(a, 0, "distance")?, vec3(a, 1, "distance")?)
    ))));
    f!("distancesquared", |_, _, _, a| {
        let d = math::sub(
            vec3(a, 0, "distancesquared")?,
            vec3(a, 1, "distancesquared")?,
        );
        Ok(ScriptValue::Float(math::dot(d, d)))
    });
    f!("distance2d", |_, _, _, a| {
        let d = math::sub(vec3(a, 0, "distance2d")?, vec3(a, 1, "distance2d")?);
        Ok(ScriptValue::Float((d[0] * d[0] + d[1] * d[1]).sqrt()))
    });
    f!("distance2dsquared", |_, _, _, a| {
        let d = math::sub(
            vec3(a, 0, "distance2dsquared")?,
            vec3(a, 1, "distance2dsquared")?,
        );
        Ok(ScriptValue::Float(d[0] * d[0] + d[1] * d[1]))
    });
    f!("vectornormalize", |_, _, _, a| Ok(ScriptValue::Vec3(
        math::normalize(vec3(a, 0, "vectornormalize")?)
    )));
    f!("vectordot", |_, _, _, a| Ok(ScriptValue::Float(math::dot(
        vec3(a, 0, "vectordot")?,
        vec3(a, 1, "vectordot")?
    ))));
    // pointonsegmentnearesttopoint(a, b, p): the point of segment a..b
    // nearest p (Buried's Leroy picks the barricade he faces with it).
    f!("pointonsegmentnearesttopoint", |_, _, _, a| {
        let name = "pointonsegmentnearesttopoint";
        let (s, e, p) = (vec3(a, 0, name)?, vec3(a, 1, name)?, vec3(a, 2, name)?);
        let d = math::sub(e, s);
        let len2 = math::dot(d, d);
        let t = if len2 > 0.0 {
            (math::dot(math::sub(p, s), d) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Ok(ScriptValue::Vec3(std::array::from_fn(|i| s[i] + d[i] * t)))
    });
    f!("vectorcross", |_, _, _, a| Ok(ScriptValue::Vec3(math::cross(
        vec3(a, 0, "vectorcross")?,
        vec3(a, 1, "vectorcross")?
    ))));
    f!("length", |_, _, _, a| Ok(ScriptValue::Float(math::length(vec3(
        a, 0, "length"
    )?))));
    f!("lengthsquared", |_, _, _, a| {
        let v = vec3(a, 0, "lengthsquared")?;
        Ok(ScriptValue::Float(math::dot(v, v)))
    });
    f!("length2d", |_, _, _, a| {
        let v = vec3(a, 0, "length2d")?;
        Ok(ScriptValue::Float((v[0] * v[0] + v[1] * v[1]).sqrt()))
    });
    f!("vectorlerp", |_, _, _, a| {
        let x = vec3(a, 0, "vectorlerp")?;
        let y = vec3(a, 1, "vectorlerp")?;
        let t = num(a, 2, "vectorlerp")?;
        Ok(ScriptValue::Vec3(math::add(x, math::scale(math::sub(y, x), t))))
    });
    f!("vectorscale", |_, _, _, a| Ok(ScriptValue::Vec3(math::scale(
        vec3(a, 0, "vectorscale")?,
        num(a, 1, "vectorscale")?
    ))));
    f!("anglestoforward", |_, _, _, a| Ok(ScriptValue::Vec3(
        math::facing_axes(vec3(a, 0, "anglestoforward")?).0
    )));
    f!("anglestoright", |_, _, _, a| Ok(ScriptValue::Vec3(
        math::facing_axes(vec3(a, 0, "anglestoright")?).1
    )));
    f!("anglestoup", |_, _, _, a| Ok(ScriptValue::Vec3(
        math::facing_axes(vec3(a, 0, "anglestoup")?).2
    )));
    f!("vectortoangles", |_, _, _, a| Ok(ScriptValue::Vec3(
        math::vector_to_angles(vec3(a, 0, "vectortoangles")?)
    )));
    f!("vectortoyaw", |_, _, _, a| Ok(ScriptValue::Float(
        math::vector_to_angles(vec3(a, 0, "vectortoyaw")?)[1]
    )));
    f!("angleclamp180", |_, _, _, a| Ok(ScriptValue::Float(
        math::angle_clamp180(num(a, 0, "angleclamp180")?)
    )));
    f!("angleclamp", |_, _, _, a| Ok(ScriptValue::Float(
        math::angle_clamp(num(a, 0, "angleclamp")?)
    )));
    f!("absangleclamp180", |_, _, _, a| Ok(ScriptValue::Float(
        math::angle_clamp180(num(a, 0, "absangleclamp180")?).abs()
    )));
    // BO2's Scr_AbsAngleClamp360: AngleNormalize360, an angle in [0, 360)
    // (Cargo's crane turns for absangleclamp360(yaw change) / 25 seconds).
    f!("absangleclamp360", |_, _, _, a| Ok(ScriptValue::Float(
        math::angle_clamp(num(a, 0, "absangleclamp360")?)
    )));
    f!("combineangles", |_, _, _, a| {
        let x = vec3(a, 0, "combineangles")?;
        let y = vec3(a, 1, "combineangles")?;
        Ok(ScriptValue::Vec3([x[0] + y[0], x[1] + y[1], x[2] + y[2]]))
    });
    f!("sin", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "sin")?.to_radians().sin()
    )));
    f!("cos", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "cos")?.to_radians().cos()
    )));
    f!("tan", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "tan")?.to_radians().tan()
    )));
    f!("asin", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "asin")?.clamp(-1.0, 1.0).asin().to_degrees()
    )));
    f!("acos", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "acos")?.clamp(-1.0, 1.0).acos().to_degrees()
    )));
    f!("atan", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "atan")?.atan().to_degrees()
    )));
    f!("sqrt", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "sqrt")?.max(0.0).sqrt()
    )));
    f!("pow", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "pow")?.powf(num(a, 1, "pow")?)
    )));
    f!("exp", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "exp")?.exp()
    )));
    f!("log", |_, _, _, a| Ok(ScriptValue::Float(num(a, 0, "log")?.ln())));
    f!("floor", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "floor")?.floor()
    )));
    f!("ceil", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "ceil")?.ceil()
    )));
    f!("round", |_, _, _, a| Ok(ScriptValue::Float(
        num(a, 0, "round")?.round()
    )));
    // lerpfloat(from, to, fraction) (Buried's generator lights, Origins).
    f!("lerpfloat", |_, _, _, a| {
        let from = num(a, 0, "lerpfloat")?;
        let to = num(a, 1, "lerpfloat")?;
        Ok(ScriptValue::Float(from + (to - from) * num(a, 2, "lerpfloat")?))
    });
    f!("abs", |_, _, _, a| Ok(match arg(a, 0) {
        ScriptValue::Int(i) => ScriptValue::Int(i.wrapping_abs()),
        v => ScriptValue::Float(v.as_float().unwrap_or(0.0).abs()),
    }));
    f!("min", |_, _, _, a| Ok(match (arg(a, 0), arg(a, 1)) {
        (ScriptValue::Int(x), ScriptValue::Int(y)) => ScriptValue::Int(*x.min(y)),
        _ => ScriptValue::Float(num(a, 0, "min")?.min(num(a, 1, "min")?)),
    }));
    f!("max", |_, _, _, a| Ok(match (arg(a, 0), arg(a, 1)) {
        (ScriptValue::Int(x), ScriptValue::Int(y)) => ScriptValue::Int(*x.max(y)),
        _ => ScriptValue::Float(num(a, 0, "max")?.max(num(a, 1, "max")?)),
    }));
    f!("clamp", |_, _, _, a| {
        let v = num(a, 0, "clamp")?;
        let lo = num(a, 1, "clamp")?;
        let hi = num(a, 2, "clamp")?;
        Ok(ScriptValue::Float(v.max(lo).min(hi)))
    });
    f!("isstring", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::Str(_)
    ))));
    f!("isistring", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::IStr(_)
    ))));
    f!("isint", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::Int(_)
    ))));
    f!("isfloat", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::Float(_)
    ))));
    f!("isarray", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::Array(_)
    ))));
    f!("isvec", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::Vec3(_)
    ))));
    f!("isfunctionptr", |_, _, _, a| Ok(ScriptValue::bool(matches!(
        arg(a, 0),
        ScriptValue::Func(_)
    ))));
    f!("issubstr", |vm, _, _, a| {
        let s = text(vm, a, 0);
        let sub = text(vm, a, 1);
        Ok(ScriptValue::bool(s.contains(&sub)))
    });
    f!("getsubstr", |vm, _, _, a| {
        let s: Vec<char> = text(vm, a, 0).chars().collect();
        let start = arg(a, 1).as_int().unwrap_or(0).clamp(0, s.len() as i32) as usize;
        let end = arg(a, 2)
            .as_int()
            .map_or(s.len(), |e| e.clamp(0, s.len() as i32) as usize);
        let out: String = if end > start {
            s[start..end].iter().collect()
        } else {
            String::new()
        };
        Ok(vm.string(&out))
    });
    f!("strtok", |vm, _, _, a| {
        let s = text(vm, a, 0);
        let delims = text(vm, a, 1);
        let parts: Vec<String> = s
            .split(|c| delims.contains(c))
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect();
        let vals = parts.iter().map(|p| vm.string(p)).collect();
        Ok(list(vals))
    });
    f!("tolower", |vm, _, _, a| {
        let s = text(vm, a, 0).to_ascii_lowercase();
        Ok(vm.string(&s))
    });
    f!("toupper", |vm, _, _, a| {
        let s = text(vm, a, 0).to_ascii_uppercase();
        Ok(vm.string(&s))
    });
    f!("strstartswith", |vm, _, _, a| Ok(ScriptValue::bool(
        text(vm, a, 0).starts_with(&text(vm, a, 1))
    )));
    f!("strendswith", |vm, _, _, a| Ok(ScriptValue::bool(
        text(vm, a, 0).ends_with(&text(vm, a, 1))
    )));
    f!("spawnstruct", |vm, _, _, _| Ok(vm.new_struct()));
    f!("array", |_, _, _, a| Ok(list(a.to_vec())));
    f!("getarraykeys", |_, _, _, a| {
        let arr = array(a, 0, "getarraykeys")?;
        Ok(list(arr.keys().map(Key::value).collect()))
    });
    f!("getfirstarraykey", |_, _, _, a| {
        let arr = array(a, 0, "getfirstarraykey")?;
        Ok(arr.first_key().map(Key::value).unwrap_or_default())
    });
    f!("getnextarraykey", |vm, _, _, a| {
        let arr = array(a, 0, "getnextarraykey")?;
        let k = vm.to_key(arg(a, 1))?;
        Ok(arr.next_key(&k).map(Key::value).unwrap_or_default())
    });
    f!("getlastarraykey", |_, _, _, a| {
        let arr = array(a, 0, "getlastarraykey")?;
        Ok(arr.last_key().map(Key::value).unwrap_or_default())
    });
    f!("arraycopy", |_, _, _, a| {
        let arr = array(a, 0, "arraycopy")?;
        Ok(ScriptValue::array(arr))
    });
    f!("isinarray", |vm, _, _, a| {
        let arr = array(a, 0, "isinarray")?;
        let v = arg(a, 1);
        Ok(ScriptValue::bool(arr.values_in_order().any(|x| vm.equal(x, v))))
    });
    f!("arraycombine", |vm, _, _, a| {
        let x = array(a, 0, "arraycombine")?;
        let y = array(a, 1, "arraycombine")?;
        let dupes = flag(a, 2, true);
        let keep_keys = flag(a, 3, false);
        let mut items: Vec<(Key, ScriptValue)> = x.map.iter().map(|(k, v)| (*k, v.clone())).collect();
        for (k, v) in &y.map {
            if !dupes && items.iter().any(|(_, w)| vm.equal(w, v)) {
                continue;
            }
            items.push((*k, v.clone()));
        }
        Ok(ScriptValue::array(renumber(items.into_iter(), keep_keys)))
    });
    // The engine's array edits change the array in place (scripts call
    // them without using the result) and return it.
    f!("arrayinsert", |_, _, _, a| {
        let ScriptValue::Array(rc) = arg(a, 0) else {
            return Err(format!(
                "arrayinsert: argument 1 is {}, not an array",
                arg(a, 0).type_name()
            ));
        };
        let arr = rc.snapshot();
        let v = arg(a, 1).clone();
        let at = arg(a, 2).as_int().unwrap_or(arr.len() as i32).max(0) as usize;
        let mut vals: Vec<ScriptValue> = arr.values_in_order().cloned().collect();
        vals.insert(at.min(vals.len()), v);
        let ScriptValue::Array(new) = list(vals) else {
            unreachable!()
        };
        *rc.write() = new.snapshot();
        Ok(ScriptValue::Array(rc.clone()))
    });
    f!("arrayremoveindex", |vm, _, _, a| {
        let ScriptValue::Array(rc) = arg(a, 0) else {
            return Err(format!(
                "arrayremoveindex: argument 1 is {}, not an array",
                arg(a, 0).type_name()
            ));
        };
        let arr = rc.snapshot();
        let k = vm.to_key(arg(a, 1))?;
        let keep_keys = flag(a, 2, false);
        let items = arr
            .map
            .iter()
            .filter(|(kk, _)| **kk != k)
            .map(|(k, v)| (*k, v.clone()));
        *rc.write() = renumber(items, keep_keys);
        Ok(ScriptValue::Array(rc.clone()))
    });
    f!("arrayremovevalue", |vm, _, _, a| {
        let ScriptValue::Array(rc) = arg(a, 0) else {
            return Err(format!(
                "arrayremovevalue: argument 1 is {}, not an array",
                arg(a, 0).type_name()
            ));
        };
        let arr = rc.snapshot();
        let v = arg(a, 1).clone();
        let keep_keys = flag(a, 2, false);
        let items: Vec<(Key, ScriptValue)> = arr
            .map
            .iter()
            .filter(|entry| !same_entry(vm, entry.1, &v))
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        *rc.write() = renumber(items.into_iter(), keep_keys);
        Ok(ScriptValue::Array(rc.clone()))
    });
    f!("arraysort", |vm, host, _, a| {
        let arr = array(a, 0, "arraysort")?;
        let origin = vec3(a, 1, "arraysort")?;
        let closest_first = flag(a, 2, true);
        let max = arg(a, 3).as_int().map(|n| n.max(0) as usize);
        let radius = arg(a, 4).as_float();
        let origin_s = vm.intern("origin");
        let mut items: Vec<(f32, ScriptValue)> = Vec::new();
        for v in arr.values_in_order() {
            let p = match v {
                ScriptValue::Object(o) => vm.get_field(host, *o, origin_s).as_vec3(),
                ScriptValue::Vec3(p) => Some(*p),
                _ => None,
            };
            let Some(p) = p else { continue };
            let d = math::length(math::sub(p, origin));
            if radius.is_some_and(|r| d > r) {
                continue;
            }
            items.push((d, v.clone()));
        }
        items.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        if !closest_first {
            items.reverse();
        }
        if let Some(m) = max {
            items.truncate(m);
        }
        Ok(list(items.into_iter().map(|(_, v)| v).collect()))
    });
    f!("getdvar", |vm, _, _, a| {
        let v = vm.dvar_text(arg(a, 0));
        Ok(vm.string(&v))
    });
    f!("getdvarint", |vm, _, _, a| Ok(ScriptValue::Int(math::parse_int(
        &vm.dvar_text(arg(a, 0))
    ))));
    f!("getdvarfloat", |vm, _, _, a| {
        Ok(ScriptValue::Float(
            vm.dvar_text(arg(a, 0)).trim().parse().unwrap_or(0.0),
        ))
    });
    f!("setdvar", |vm, _, _, a| {
        let n = vm.dvar_name(arg(a, 0));
        let v = text(vm, a, 1);
        vm.dvars.insert(n, v);
        Ok(ScriptValue::Undefined)
    });
    f!("setdvarint", |vm, _, _, a| {
        let n = vm.dvar_name(arg(a, 0));
        let v = math::parse_int(&text(vm, a, 1)).to_string();
        vm.dvars.insert(n, v);
        Ok(ScriptValue::Undefined)
    });
    f!("gettime", |vm, _, _, _| Ok(ScriptValue::Int(vm.clock_stamp_ms as i32)));
    for name in [
        "assert",
        "assertmsg",
        "assertex",
        "println",
        "print",
        "profilelog_begintiming",
        "profilelog_endtiming",
        "pixbeginevent",
        "pixendevent",
        "pixmarker",
        "incrementcounter",
        "bbprint",
        "adddebugcommand",
        "print3d",
        "line",
        "circle",
        "box",
        "sphere",
        "debugstar",
        "recordline",
        "recordsphere",
        "recordenttext",
    ] {
        vm.bind(name, false, |_, _, _, _| Ok(ScriptValue::Undefined));
    }
}
