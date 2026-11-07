//! A very small Java bytecode interpreter.
//!
//! The shapes of mobs and block entities aren't data files in the game: they're built by
//! code in the client jar. Official textures only line up with official shapes, so on start
//! Lattice runs that model-building code, from the player's own downloaded jar, and reads the
//! result. Nothing of it is stored in this project. This interpreter only knows what that code
//! needs: plain arithmetic, objects, and a few collection classes from outside the jar.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::rc::Rc;

pub type VmResult<T> = Result<T, String>;

#[derive(Debug, Clone, PartialEq)]
pub enum V {
    Null,
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    Ref(usize),
}

impl V {
    fn wide(&self) -> bool {
        matches!(self, V::Long(_) | V::Double(_))
    }
    fn int(&self) -> VmResult<i32> {
        match self {
            V::Int(v) => Ok(*v),
            other => Err(format!("expected an int, found {other:?}")),
        }
    }
    fn float(&self) -> VmResult<f32> {
        match self {
            V::Float(v) => Ok(*v),
            V::Int(v) => Ok(*v as f32),
            V::Double(v) => Ok(*v as f32),
            other => Err(format!("expected a float, found {other:?}")),
        }
    }
    fn double(&self) -> VmResult<f64> {
        match self {
            V::Double(v) => Ok(*v),
            V::Float(v) => Ok(*v as f64),
            V::Int(v) => Ok(*v as f64),
            other => Err(format!("expected a double, found {other:?}")),
        }
    }
    fn long(&self) -> VmResult<i64> {
        match self {
            V::Long(v) => Ok(*v),
            other => Err(format!("expected a long, found {other:?}")),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Obj {
    Instance { class: String, fields: HashMap<String, V> },
    Array(Vec<V>),
    Str(String),
    List(Vec<V>),
    Map(Vec<(V, V)>),
    Set(Vec<V>),
    Entry(V, V),
    Iter(Vec<V>, usize),
    /// A lambda or method reference: the method it runs and the values it captured.
    Lambda { kind: u8, class: String, name: String, desc: String, captured: Vec<V> },
    Class(String),
    /// A value from a class this interpreter doesn't run; only its name matters.
    Opaque(String),
}

enum Cp {
    None,
    Utf8(String),
    Int(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    Class(u16),
    Str(u16),
    Member(u16, u16),
    NameType(u16, u16),
    Handle(u8, u16),
    MethodType,
    Dynamic(u16, u16),
}

struct Method {
    access: u16,
    name: String,
    desc: String,
    max_locals: usize,
    code: Option<Rc<Vec<u8>>>,
}

struct Field {
    access: u16,
    name: String,
    constant: Option<u16>,
}

pub struct Class {
    name: String,
    superclass: Option<String>,
    interfaces: Vec<String>,
    cp: Vec<Cp>,
    fields: Vec<Field>,
    methods: Vec<Rc<Method>>,
    bootstrap: Vec<(u16, Vec<u16>)>,
}

struct Bytes<'a> {
    b: &'a [u8],
    at: usize,
}

impl Bytes<'_> {
    fn u8(&mut self) -> VmResult<u8> {
        let v = *self.b.get(self.at).ok_or("class file ended early")?;
        self.at += 1;
        Ok(v)
    }
    fn u16(&mut self) -> VmResult<u16> {
        Ok((self.u8()? as u16) << 8 | self.u8()? as u16)
    }
    fn u32(&mut self) -> VmResult<u32> {
        Ok((self.u16()? as u32) << 16 | self.u16()? as u32)
    }
    fn take(&mut self, n: usize) -> VmResult<&[u8]> {
        let s = self.b.get(self.at..self.at + n).ok_or("class file ended early")?;
        self.at += n;
        Ok(s)
    }
}

fn parse_class(bytes: &[u8]) -> VmResult<Class> {
    let mut r = Bytes { b: bytes, at: 0 };
    if r.u32()? != 0xCAFE_BABE {
        return Err("not a class file".into());
    }
    r.u32()?;
    let n = r.u16()? as usize;
    let mut cp = Vec::with_capacity(n);
    cp.push(Cp::None);
    while cp.len() < n {
        let tag = r.u8()?;
        let e = match tag {
            1 => {
                let len = r.u16()? as usize;
                Cp::Utf8(String::from_utf8_lossy(r.take(len)?).into_owned())
            }
            3 => Cp::Int(r.u32()? as i32),
            4 => Cp::Float(f32::from_bits(r.u32()?)),
            5 => Cp::Long(((r.u32()? as u64) << 32 | r.u32()? as u64) as i64),
            6 => Cp::Double(f64::from_bits((r.u32()? as u64) << 32 | r.u32()? as u64)),
            7 => Cp::Class(r.u16()?),
            8 => Cp::Str(r.u16()?),
            9..=11 => Cp::Member(r.u16()?, r.u16()?),
            12 => Cp::NameType(r.u16()?, r.u16()?),
            15 => Cp::Handle(r.u8()?, r.u16()?),
            16 => {
                r.u16()?;
                Cp::MethodType
            }
            17 | 18 => Cp::Dynamic(r.u16()?, r.u16()?),
            19 | 20 => {
                r.u16()?;
                Cp::None
            }
            t => return Err(format!("unknown constant pool tag {t}")),
        };
        let two = matches!(e, Cp::Long(_) | Cp::Double(_));
        cp.push(e);
        if two {
            cp.push(Cp::None);
        }
    }
    let utf = |cp: &[Cp], i: u16| -> VmResult<String> {
        match cp.get(i as usize) {
            Some(Cp::Utf8(s)) => Ok(s.clone()),
            _ => Err(format!("constant {i} isn't text")),
        }
    };
    let class_name = |cp: &[Cp], i: u16| -> VmResult<String> {
        match cp.get(i as usize) {
            Some(Cp::Class(n)) => utf(cp, *n),
            _ => Err(format!("constant {i} isn't a class")),
        }
    };
    r.u16()?;
    let name = class_name(&cp, r.u16()?)?;
    let sup = r.u16()?;
    let superclass = if sup == 0 { None } else { Some(class_name(&cp, sup)?) };
    let n_if = r.u16()?;
    let mut interfaces = Vec::new();
    for _ in 0..n_if {
        interfaces.push(class_name(&cp, r.u16()?)?);
    }
    let n_f = r.u16()?;
    let mut fields = Vec::new();
    for _ in 0..n_f {
        let access = r.u16()?;
        let fname = utf(&cp, r.u16()?)?;
        r.u16()?;
        let mut constant = None;
        for _ in 0..r.u16()? {
            let an = utf(&cp, r.u16()?)?;
            let len = r.u32()? as usize;
            if an == "ConstantValue" {
                constant = Some(r.u16()?);
            } else {
                r.take(len)?;
            }
        }
        fields.push(Field { access, name: fname, constant });
    }
    let n_m = r.u16()?;
    let mut methods = Vec::new();
    for _ in 0..n_m {
        let access = r.u16()?;
        let mname = utf(&cp, r.u16()?)?;
        let desc = utf(&cp, r.u16()?)?;
        let mut code = None;
        let mut max_locals = 0;
        for _ in 0..r.u16()? {
            let an = utf(&cp, r.u16()?)?;
            let len = r.u32()? as usize;
            if an == "Code" {
                r.u16()?;
                max_locals = r.u16()? as usize;
                let n = r.u32()? as usize;
                code = Some(Rc::new(r.take(n)?.to_vec()));
                let table = r.u16()? as usize;
                r.take(table * 8)?;
                for _ in 0..r.u16()? {
                    r.u16()?;
                    let l = r.u32()? as usize;
                    r.take(l)?;
                }
            } else {
                r.take(len)?;
            }
        }
        methods.push(Rc::new(Method { access, name: mname, desc, max_locals, code }));
    }
    let mut bootstrap = Vec::new();
    for _ in 0..r.u16()? {
        let an = utf(&cp, r.u16()?)?;
        let len = r.u32()? as usize;
        if an == "BootstrapMethods" {
            for _ in 0..r.u16()? {
                let h = r.u16()?;
                let mut args = Vec::new();
                for _ in 0..r.u16()? {
                    args.push(r.u16()?);
                }
                bootstrap.push((h, args));
            }
        } else {
            r.take(len)?;
        }
    }
    Ok(Class { name, superclass, interfaces, cp, fields, methods, bootstrap })
}

impl Class {
    fn utf(&self, i: u16) -> VmResult<&str> {
        match self.cp.get(i as usize) {
            Some(Cp::Utf8(s)) => Ok(s),
            _ => Err(format!("{}: constant {i} isn't text", self.name)),
        }
    }
    fn class_at(&self, i: u16) -> VmResult<&str> {
        match self.cp.get(i as usize) {
            Some(Cp::Class(n)) => self.utf(*n),
            _ => Err(format!("{}: constant {i} isn't a class", self.name)),
        }
    }
    fn member(&self, i: u16) -> VmResult<(&str, &str, &str)> {
        let Some(Cp::Member(c, nt)) = self.cp.get(i as usize) else { return Err(format!("{}: constant {i} isn't a member", self.name)) };
        let Some(Cp::NameType(n, d)) = self.cp.get(*nt as usize) else { return Err("bad name and type".into()) };
        Ok((self.class_at(*c)?, self.utf(*n)?, self.utf(*d)?))
    }
}

/// Number of arguments in a method descriptor, and whether it returns anything.
fn parse_args(desc: &str) -> (Vec<char>, bool) {
    let b = desc.as_bytes();
    let mut i = 1;
    let mut out = Vec::new();
    while b[i] != b')' {
        let mut kind = b[i] as char;
        while b[i] == b'[' {
            i += 1;
            kind = '[';
        }
        if b[i] == b'L' {
            while b[i] != b';' {
                i += 1;
            }
            if kind != '[' {
                kind = 'L';
            }
        }
        i += 1;
        out.push(kind);
    }
    (out, b[i + 1] != b'V')
}

const STEP_LIMIT: u64 = 80_000_000;

pub struct Vm {
    archive: zip::ZipArchive<std::fs::File>,
    classes: HashMap<String, Option<Rc<Class>>>,
    statics: HashMap<String, HashMap<String, V>>,
    inited: HashSet<String>,
    pub heap: Vec<Obj>,
    strings: HashMap<String, usize>,
    steps: u64,
    depth: u32,
}

/// Classes whose static setup pulls in far more of the game than models need. What
/// model code wants from them is answered in `native_call` or as opaque names.
fn skip_clinit(class: &str) -> bool {
    !class.starts_with("net/minecraft/client/model") && !class.starts_with("net/minecraft/client/animation/definitions")
}

impl Vm {
    pub fn open(jar: &std::path::Path) -> VmResult<Vm> {
        let f = std::fs::File::open(jar).map_err(|e| e.to_string())?;
        let archive = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        Ok(Vm { archive, classes: HashMap::new(), statics: HashMap::new(), inited: HashSet::new(), heap: Vec::new(), strings: HashMap::new(), steps: 0, depth: 0 })
    }

    fn class(&mut self, name: &str) -> Option<Rc<Class>> {
        if let Some(c) = self.classes.get(name) {
            return c.clone();
        }
        let path = format!("{name}.class");
        let parsed = (|| {
            let mut e = self.archive.by_name(&path).ok()?;
            let mut buf = Vec::new();
            e.read_to_end(&mut buf).ok()?;
            parse_class(&buf).ok().map(Rc::new)
        })();
        self.classes.insert(name.to_string(), parsed.clone());
        parsed
    }

    pub fn alloc(&mut self, o: Obj) -> V {
        self.heap.push(o);
        V::Ref(self.heap.len() - 1)
    }

    pub fn str(&mut self, s: &str) -> V {
        if let Some(i) = self.strings.get(s) {
            return V::Ref(*i);
        }
        let v = self.alloc(Obj::Str(s.to_string()));
        if let V::Ref(i) = v {
            self.strings.insert(s.to_string(), i);
        }
        v
    }

    pub fn string_of(&self, v: &V) -> String {
        match v {
            V::Ref(i) => match &self.heap[*i] {
                Obj::Str(s) => s.clone(),
                Obj::Opaque(s) => s.clone(),
                Obj::Instance { class, fields } => {
                    // Enums print as their name.
                    match fields.get("$name") {
                        Some(n) => self.string_of(n),
                        None => class.clone(),
                    }
                }
                other => format!("{other:?}"),
            },
            V::Int(v) => v.to_string(),
            V::Long(v) => v.to_string(),
            V::Float(v) => format!("{v:?}"),
            V::Double(v) => format!("{v:?}"),
            V::Null => "null".into(),
        }
    }

    fn eq(&self, a: &V, b: &V) -> bool {
        match (a, b) {
            (V::Ref(x), V::Ref(y)) => {
                if x == y {
                    return true;
                }
                match (&self.heap[*x], &self.heap[*y]) {
                    (Obj::Str(p), Obj::Str(q)) | (Obj::Opaque(p), Obj::Opaque(q)) => p == q,
                    _ => false,
                }
            }
            _ => a == b,
        }
    }

    /// Runs a class's static setup once.
    fn init_class(&mut self, class: &str) -> VmResult<()> {
        if !self.inited.insert(class.to_string()) {
            return Ok(());
        }
        let Some(c) = self.class(class) else { return Ok(()) };
        if let Some(sup) = c.superclass.clone() {
            self.init_class(&sup)?;
        }
        let mut st = HashMap::new();
        for f in &c.fields {
            if f.access & 0x0008 == 0 {
                continue;
            }
            if let Some(i) = f.constant {
                let v = match &c.cp[i as usize] {
                    Cp::Int(v) => V::Int(*v),
                    Cp::Float(v) => V::Float(*v),
                    Cp::Long(v) => V::Long(*v),
                    Cp::Double(v) => V::Double(*v),
                    Cp::Str(s) => {
                        let text = c.utf(*s)?.to_string();
                        self.str(&text)
                    }
                    _ => V::Null,
                };
                st.insert(f.name.clone(), v);
            }
        }
        self.statics.insert(class.to_string(), st);
        if skip_clinit(class) {
            return Ok(());
        }
        if let Some(m) = c.methods.iter().find(|m| m.name == "<clinit>").cloned() {
            self.run_method(&c, &m, Vec::new())?;
        }
        Ok(())
    }

    fn find_method(&mut self, class: &str, name: &str, desc: &str) -> Option<(Rc<Class>, Rc<Method>)> {
        let mut at = Some(class.to_string());
        let mut interfaces = Vec::new();
        while let Some(n) = at {
            let c = self.class(&n)?;
            if let Some(m) = c.methods.iter().find(|m| m.name == name && m.desc == desc) {
                return Some((c.clone(), m.clone()));
            }
            interfaces.extend(c.interfaces.iter().cloned());
            at = c.superclass.clone();
        }
        // Default methods on interfaces.
        let mut seen = HashSet::new();
        while let Some(i) = interfaces.pop() {
            if !seen.insert(i.clone()) {
                continue;
            }
            let Some(c) = self.class(&i) else { continue };
            if let Some(m) = c.methods.iter().find(|m| m.name == name && m.desc == desc && m.code.is_some()) {
                return Some((c.clone(), m.clone()));
            }
            interfaces.extend(c.interfaces.iter().cloned());
        }
        None
    }

    /// Calls a static or instance method by name.
    pub fn call(&mut self, class: &str, name: &str, desc: &str, args: Vec<V>) -> VmResult<V> {
        // Answered directly rather than by running the game's own version.
        if class == "net/minecraft/resources/Identifier"
            || (class == "net/minecraft/util/Mth" && (name == "sin" || name == "cos"))
            || (class == "net/minecraft/util/Util" && name == "allOfEnumExcept")
            || (class == "net/minecraft/core/Direction" && name == "values")
        {
            return self.native_call(class, name, desc, args);
        }
        self.init_class(class)?;
        let Some((c, m)) = self.find_method(class, name, desc) else {
            return self.native_call(class, name, desc, args);
        };
        self.run_method(&c, &m, args)
    }

    fn instance_class(&self, v: &V) -> Option<String> {
        match v {
            V::Ref(i) => match &self.heap[*i] {
                Obj::Instance { class, .. } => Some(class.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// A method reference or lambda being run.
    pub fn call_lambda(&mut self, f: &V, args: Vec<V>) -> VmResult<V> {
        let V::Ref(i) = f else { return Err(format!("calling {f:?}")) };
        let Obj::Lambda { kind, class, name, desc, captured } = self.heap[*i].clone() else {
            return Err(format!("calling something that isn't a lambda: {:?}", self.heap[*i]));
        };
        let mut all = captured;
        all.extend(args);
        match kind {
            6 => self.call(&class, &name, &desc, all),
            8 => {
                let obj = self.new_object(&class)?;
                let mut with = vec![obj.clone()];
                with.extend(all);
                self.call(&class, "<init>", &desc, with)?;
                Ok(obj)
            }
            _ => {
                let recv = all.first().cloned().unwrap_or(V::Null);
                let dyn_class = self.instance_class(&recv).unwrap_or(class);
                self.call(&dyn_class, &name, &desc, all)
            }
        }
    }

    /// Sets up a class, then returns the values of its static fields by name.
    pub fn statics_of(&mut self, class: &str) -> VmResult<Vec<(String, V)>> {
        self.init_class(class)?;
        Ok(self.statics.get(class).map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default())
    }

    /// Classes in the jar whose path starts with `prefix`, nested classes left out.
    pub fn classes_under(&self, prefix: &str) -> Vec<String> {
        self.archive
            .file_names()
            .filter_map(Result::ok)
            .filter(|n| n.starts_with(prefix) && n.ends_with(".class") && !n.contains('$') && !n.ends_with("package-info.class"))
            .map(|n| n.trim_end_matches(".class").to_string())
            .collect()
    }

    pub fn new_object(&mut self, class: &str) -> VmResult<V> {
        self.init_class(class)?;
        Ok(self.alloc(Obj::Instance { class: class.to_string(), fields: HashMap::new() }))
    }

    pub fn get(&self, obj: &V, field: &str) -> V {
        if let V::Ref(i) = obj {
            if let Obj::Instance { fields, .. } = &self.heap[*i] {
                return fields.get(field).cloned().unwrap_or(V::Null);
            }
        }
        V::Null
    }

    pub fn list_of(&self, v: &V) -> Vec<V> {
        match v {
            V::Ref(i) => match &self.heap[*i] {
                Obj::List(l) | Obj::Set(l) | Obj::Array(l) => l.clone(),
                Obj::Map(m) => m.iter().map(|(k, _)| k.clone()).collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    pub fn map_of(&self, v: &V) -> Vec<(V, V)> {
        match v {
            V::Ref(i) => match &self.heap[*i] {
                Obj::Map(m) => m.clone(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn run_method(&mut self, class: &Rc<Class>, m: &Rc<Method>, args: Vec<V>) -> VmResult<V> {
        let Some(code) = m.code.clone() else {
            return Err(format!("{}.{} has no code", class.name, m.name));
        };
        let _ = m.access;
        self.depth += 1;
        if self.depth > 200 {
            self.depth -= 1;
            return Err("calls nested too deeply".into());
        }
        let mut locals = vec![V::Null; m.max_locals.max(args.len() * 2) + 2];
        let mut at = 0;
        for a in args {
            let wide = a.wide();
            locals[at] = a;
            at += if wide { 2 } else { 1 };
        }
        let r = self.interpret(class, &code, locals).map_err(|e| format!("{e}\n  in {}.{}{}", class.name, m.name, m.desc));
        self.depth -= 1;
        r
    }

    fn interpret(&mut self, class: &Rc<Class>, code: &[u8], mut locals: Vec<V>) -> VmResult<V> {
        let mut stack: Vec<V> = Vec::with_capacity(16);
        let mut pc = 0usize;
        macro_rules! pop {
            () => {
                stack.pop().ok_or("operand stack empty")?
            };
        }
        macro_rules! u1 {
            ($o:expr) => {
                code[pc + $o] as usize
            };
        }
        macro_rules! u2 {
            ($o:expr) => {
                ((code[pc + $o] as usize) << 8 | code[pc + $o + 1] as usize) as u16
            };
        }
        macro_rules! s2 {
            ($o:expr) => {
                ((code[pc + $o] as i16) << 8 | code[pc + $o + 1] as i16) as isize
            };
        }
        macro_rules! jump {
            ($cond:expr) => {
                if $cond {
                    pc = (pc as isize + s2!(1)) as usize;
                    continue;
                }
                pc += 3;
                continue;
            };
        }
        loop {
            self.steps += 1;
            if self.steps > STEP_LIMIT {
                return Err("ran for too long".into());
            }
            let op = code[pc];
            match op {
                0 => pc += 1,
                1 => {
                    stack.push(V::Null);
                    pc += 1
                }
                2..=8 => {
                    stack.push(V::Int(op as i32 - 3));
                    pc += 1
                }
                9 | 10 => {
                    stack.push(V::Long(op as i64 - 9));
                    pc += 1
                }
                11..=13 => {
                    stack.push(V::Float((op - 11) as f32));
                    pc += 1
                }
                14 | 15 => {
                    stack.push(V::Double((op - 14) as f64));
                    pc += 1
                }
                16 => {
                    stack.push(V::Int(code[pc + 1] as i8 as i32));
                    pc += 2
                }
                17 => {
                    stack.push(V::Int(s2!(1) as i32));
                    pc += 3
                }
                18..=20 => {
                    let i = if op == 18 { u1!(1) as u16 } else { u2!(1) };
                    let v = match &class.cp[i as usize] {
                        Cp::Int(v) => V::Int(*v),
                        Cp::Float(v) => V::Float(*v),
                        Cp::Long(v) => V::Long(*v),
                        Cp::Double(v) => V::Double(*v),
                        Cp::Str(s) => {
                            let t = class.utf(*s)?.to_string();
                            self.str(&t)
                        }
                        Cp::Class(_) => {
                            let t = class.class_at(i)?.to_string();
                            self.alloc(Obj::Class(t))
                        }
                        _ => return Err(format!("unsupported constant {i}")),
                    };
                    stack.push(v);
                    pc += if op == 18 { 2 } else { 3 };
                }
                21..=25 => {
                    stack.push(locals[u1!(1)].clone());
                    pc += 2
                }
                26..=45 => {
                    let n = (op as usize - 26) % 4;
                    stack.push(locals[n].clone());
                    pc += 1
                }
                46..=53 => {
                    let i = pop!().int()?;
                    let a = pop!();
                    let V::Ref(r) = a else { return Err("indexing null".into()) };
                    let Obj::Array(d) = &self.heap[r] else { return Err("indexing a non-array".into()) };
                    stack.push(d.get(i as usize).cloned().ok_or("array index out of bounds")?);
                    pc += 1
                }
                54..=58 => {
                    locals[u1!(1)] = pop!();
                    pc += 2
                }
                59..=78 => {
                    let n = (op as usize - 59) % 4;
                    locals[n] = pop!();
                    pc += 1
                }
                79..=86 => {
                    let v = pop!();
                    let i = pop!().int()?;
                    let a = pop!();
                    let V::Ref(r) = a else { return Err("storing into null".into()) };
                    let Obj::Array(d) = &mut self.heap[r] else { return Err("storing into a non-array".into()) };
                    *d.get_mut(i as usize).ok_or("array index out of bounds")? = v;
                    pc += 1
                }
                87 => {
                    pop!();
                    pc += 1
                }
                88 => {
                    let a = pop!();
                    if !a.wide() {
                        pop!();
                    }
                    pc += 1
                }
                89 => {
                    let a = stack.last().ok_or("empty")?.clone();
                    stack.push(a);
                    pc += 1
                }
                90 => {
                    let a = pop!();
                    let b = pop!();
                    stack.push(a.clone());
                    stack.push(b);
                    stack.push(a);
                    pc += 1
                }
                91 => {
                    let a = pop!();
                    let b = pop!();
                    if b.wide() {
                        stack.push(a.clone());
                        stack.push(b);
                        stack.push(a);
                    } else {
                        let c = pop!();
                        stack.push(a.clone());
                        stack.push(c);
                        stack.push(b);
                        stack.push(a);
                    }
                    pc += 1
                }
                92 => {
                    let a = pop!();
                    if a.wide() {
                        stack.push(a.clone());
                        stack.push(a);
                    } else {
                        let b = pop!();
                        stack.push(b.clone());
                        stack.push(a.clone());
                        stack.push(b);
                        stack.push(a);
                    }
                    pc += 1
                }
                93 | 94 => {
                    let a = pop!();
                    if a.wide() {
                        let b = pop!();
                        stack.push(a.clone());
                        stack.push(b);
                        stack.push(a);
                    } else {
                        let b = pop!();
                        let c = pop!();
                        stack.push(b.clone());
                        stack.push(a.clone());
                        stack.push(c);
                        stack.push(b);
                        stack.push(a);
                    }
                    pc += 1
                }
                95 => {
                    let a = pop!();
                    let b = pop!();
                    stack.push(a);
                    stack.push(b);
                    pc += 1
                }
                96..=115 => {
                    let b = pop!();
                    let a = pop!();
                    let kind = (op - 96) % 4;
                    let which = (op - 96) / 4;
                    stack.push(match kind {
                        0 => {
                            let (x, y) = (a.int()?, b.int()?);
                            V::Int(match which {
                                0 => x.wrapping_add(y),
                                1 => x.wrapping_sub(y),
                                2 => x.wrapping_mul(y),
                                3 => {
                                    if y == 0 {
                                        return Err("divide by zero".into());
                                    }
                                    x.wrapping_div(y)
                                }
                                _ => {
                                    if y == 0 {
                                        return Err("divide by zero".into());
                                    }
                                    x.wrapping_rem(y)
                                }
                            })
                        }
                        1 => {
                            let (x, y) = (a.long()?, b.long()?);
                            V::Long(match which {
                                0 => x.wrapping_add(y),
                                1 => x.wrapping_sub(y),
                                2 => x.wrapping_mul(y),
                                3 => x.wrapping_div(y),
                                _ => x.wrapping_rem(y),
                            })
                        }
                        2 => {
                            let (x, y) = (a.float()?, b.float()?);
                            V::Float(match which {
                                0 => x + y,
                                1 => x - y,
                                2 => x * y,
                                3 => x / y,
                                _ => x % y,
                            })
                        }
                        _ => {
                            let (x, y) = (a.double()?, b.double()?);
                            V::Double(match which {
                                0 => x + y,
                                1 => x - y,
                                2 => x * y,
                                3 => x / y,
                                _ => x % y,
                            })
                        }
                    });
                    pc += 1
                }
                116 => {
                    let a = pop!().int()?;
                    stack.push(V::Int(a.wrapping_neg()));
                    pc += 1
                }
                117 => {
                    let a = pop!().long()?;
                    stack.push(V::Long(a.wrapping_neg()));
                    pc += 1
                }
                118 => {
                    let a = pop!().float()?;
                    stack.push(V::Float(-a));
                    pc += 1
                }
                119 => {
                    let a = pop!().double()?;
                    stack.push(V::Double(-a));
                    pc += 1
                }
                120..=131 => {
                    let b = pop!();
                    let a = pop!();
                    let r = match op {
                        120 => V::Int(a.int()? << (b.int()? & 31)),
                        121 => V::Long(a.long()? << (b.int()? & 63)),
                        122 => V::Int(a.int()? >> (b.int()? & 31)),
                        123 => V::Long(a.long()? >> (b.int()? & 63)),
                        124 => V::Int(((a.int()? as u32) >> (b.int()? & 31)) as i32),
                        125 => V::Long(((a.long()? as u64) >> (b.int()? & 63)) as i64),
                        126 => V::Int(a.int()? & b.int()?),
                        127 => V::Long(a.long()? & b.long()?),
                        128 => V::Int(a.int()? | b.int()?),
                        129 => V::Long(a.long()? | b.long()?),
                        130 => V::Int(a.int()? ^ b.int()?),
                        _ => V::Long(a.long()? ^ b.long()?),
                    };
                    stack.push(r);
                    pc += 1
                }
                132 => {
                    let i = u1!(1);
                    let c = code[pc + 2] as i8 as i32;
                    locals[i] = V::Int(locals[i].int()?.wrapping_add(c));
                    pc += 3
                }
                133..=147 => {
                    let a = pop!();
                    stack.push(match op {
                        133 => V::Long(a.int()? as i64),
                        134 => V::Float(a.int()? as f32),
                        135 => V::Double(a.int()? as f64),
                        136 => V::Int(a.long()? as i32),
                        137 => V::Float(a.long()? as f32),
                        138 => V::Double(a.long()? as f64),
                        139 => V::Int(a.float()? as i32),
                        140 => V::Long(a.float()? as i64),
                        141 => V::Double(a.float()? as f64),
                        142 => V::Int(a.double()? as i32),
                        143 => V::Long(a.double()? as i64),
                        144 => V::Float(a.double()? as f32),
                        145 => V::Int(a.int()? as i8 as i32),
                        146 => V::Int((a.int()? as u16) as i32),
                        _ => V::Int(a.int()? as i16 as i32),
                    });
                    pc += 1
                }
                148 => {
                    let b = pop!().long()?;
                    let a = pop!().long()?;
                    stack.push(V::Int((a > b) as i32 - (a < b) as i32));
                    pc += 1
                }
                149..=152 => {
                    let b = pop!().double()?;
                    let a = pop!().double()?;
                    let nan = if op == 149 || op == 151 { -1 } else { 1 };
                    stack.push(V::Int(if a.is_nan() || b.is_nan() { nan } else { (a > b) as i32 - (a < b) as i32 }));
                    pc += 1
                }
                153..=158 => {
                    let v = pop!().int()?;
                    jump!(match op {
                        153 => v == 0,
                        154 => v != 0,
                        155 => v < 0,
                        156 => v >= 0,
                        157 => v > 0,
                        _ => v <= 0,
                    });
                }
                159..=164 => {
                    let b = pop!().int()?;
                    let a = pop!().int()?;
                    jump!(match op {
                        159 => a == b,
                        160 => a != b,
                        161 => a < b,
                        162 => a >= b,
                        163 => a > b,
                        _ => a <= b,
                    });
                }
                165 | 166 => {
                    let b = pop!();
                    let a = pop!();
                    let same = self.eq(&a, &b);
                    jump!(if op == 165 { same } else { !same });
                }
                167 => {
                    pc = (pc as isize + s2!(1)) as usize;
                }
                170 | 171 => {
                    let v = pop!().int()?;
                    let base = pc;
                    let mut p = (pc + 4) & !3;
                    let rd = |p: usize| i32::from_be_bytes([code[p], code[p + 1], code[p + 2], code[p + 3]]);
                    let default = rd(p);
                    p += 4;
                    if op == 170 {
                        let (low, high) = (rd(p), rd(p + 4));
                        p += 8;
                        if v >= low && v <= high {
                            pc = (base as i64 + rd(p + (v - low) as usize * 4) as i64) as usize;
                        } else {
                            pc = (base as i64 + default as i64) as usize;
                        }
                    } else {
                        let n = rd(p) as usize;
                        p += 4;
                        pc = (base as i64 + default as i64) as usize;
                        for k in 0..n {
                            if rd(p + k * 8) == v {
                                pc = (base as i64 + rd(p + k * 8 + 4) as i64) as usize;
                                break;
                            }
                        }
                    }
                }
                172..=176 => return Ok(pop!()),
                177 => return Ok(V::Null),
                178 => {
                    let (c, n, _) = class.member(u2!(1))?;
                    let (c, n) = (c.to_string(), n.to_string());
                    stack.push(self.get_static(&c, &n)?);
                    pc += 3
                }
                179 => {
                    let (c, n, _) = class.member(u2!(1))?;
                    let (c, n) = (c.to_string(), n.to_string());
                    let v = pop!();
                    self.init_class(&c)?;
                    self.statics.entry(c).or_default().insert(n, v);
                    pc += 3
                }
                180 => {
                    let (_, n, d) = class.member(u2!(1))?;
                    let (n, d) = (n.to_string(), d.to_string());
                    let o = pop!();
                    let mut v = self.get_field(&o, &n)?;
                    // A field nothing has written yet holds its type's zero.
                    if v == V::Null {
                        v = match d.as_str() {
                            "I" | "Z" | "B" | "S" | "C" => V::Int(0),
                            "F" => V::Float(0.0),
                            "D" => V::Double(0.0),
                            "J" => V::Long(0),
                            _ => V::Null,
                        };
                    }
                    stack.push(v);
                    pc += 3
                }
                181 => {
                    let (_, n, _) = class.member(u2!(1))?;
                    let n = n.to_string();
                    let v = pop!();
                    let o = pop!();
                    let V::Ref(r) = o else { return Err(format!("setting {n} on null")) };
                    match &mut self.heap[r] {
                        Obj::Instance { fields, .. } => {
                            fields.insert(n, v);
                        }
                        other => return Err(format!("setting {n} on {other:?}")),
                    }
                    pc += 3
                }
                182..=185 => {
                    let (owner, name, desc) = class.member(u2!(1))?;
                    let (owner, name, desc) = (owner.to_string(), name.to_string(), desc.to_string());
                    let (argt, returns) = parse_args(&desc);
                    let mut args = Vec::with_capacity(argt.len() + 1);
                    for _ in 0..argt.len() {
                        args.push(pop!());
                    }
                    if op != 184 {
                        args.push(pop!());
                    }
                    args.reverse();
                    let r = if op == 184 {
                        self.call(&owner, &name, &desc, args)?
                    } else if op == 183 {
                        self.invoke_special(&owner, &name, &desc, args)?
                    } else {
                        self.invoke_virtual(&owner, &name, &desc, args)?
                    };
                    if returns {
                        stack.push(r);
                    }
                    pc += if op == 185 { 5 } else { 3 };
                }
                186 => {
                    let Cp::Dynamic(bs, nt) = class.cp[u2!(1) as usize] else { return Err("bad invokedynamic".into()) };
                    let Cp::NameType(_, d) = class.cp[nt as usize] else { return Err("bad name and type".into()) };
                    let desc = class.utf(d)?.to_string();
                    let (argt, _) = parse_args(&desc);
                    let mut args = Vec::new();
                    for _ in 0..argt.len() {
                        args.push(pop!());
                    }
                    args.reverse();
                    let v = self.invoke_dynamic(class, bs, args)?;
                    stack.push(v);
                    pc += 5;
                }
                187 => {
                    let c = class.class_at(u2!(1))?.to_string();
                    let v = self.new_object(&c)?;
                    stack.push(v);
                    pc += 3
                }
                188 | 189 => {
                    let n = pop!().int()?;
                    if n < 0 {
                        return Err("negative array size".into());
                    }
                    let zero = if op == 188 {
                        match code[pc + 1] {
                            6 => V::Float(0.0),
                            7 => V::Double(0.0),
                            11 => V::Long(0),
                            _ => V::Int(0),
                        }
                    } else {
                        V::Null
                    };
                    let v = self.alloc(Obj::Array(vec![zero; n as usize]));
                    stack.push(v);
                    pc += if op == 188 { 2 } else { 3 };
                }
                190 => {
                    let a = pop!();
                    let V::Ref(r) = a else { return Err("length of null".into()) };
                    let Obj::Array(d) = &self.heap[r] else { return Err("length of a non-array".into()) };
                    stack.push(V::Int(d.len() as i32));
                    pc += 1
                }
                191 => return Err("the code threw an exception".into()),
                192 => pc += 3,
                193 => {
                    let c = class.class_at(u2!(1))?.to_string();
                    let a = pop!();
                    let yes = match self.instance_class(&a) {
                        Some(mut at) => {
                            let mut found = false;
                            loop {
                                if at == c {
                                    found = true;
                                    break;
                                }
                                let Some(k) = self.class(&at) else { break };
                                if k.interfaces.contains(&c) {
                                    found = true;
                                    break;
                                }
                                match k.superclass.clone() {
                                    Some(s) => at = s,
                                    None => break,
                                }
                            }
                            found
                        }
                        None => false,
                    };
                    stack.push(V::Int(yes as i32));
                    pc += 3;
                }
                194 | 195 => {
                    pop!();
                    pc += 1
                }
                197 => {
                    let dims = u1!(3);
                    let mut sizes = Vec::new();
                    for _ in 0..dims {
                        sizes.push(pop!().int()?);
                    }
                    sizes.reverse();
                    let v = self.multi_array(&sizes);
                    stack.push(v);
                    pc += 4;
                }
                198 | 199 => {
                    let a = pop!();
                    let is_null = a == V::Null;
                    jump!(if op == 198 { is_null } else { !is_null });
                }
                196 => {
                    // wide
                    let inner = code[pc + 1];
                    let i = ((code[pc + 2] as usize) << 8) | code[pc + 3] as usize;
                    match inner {
                        21..=25 => {
                            stack.push(locals[i].clone());
                            pc += 4
                        }
                        54..=58 => {
                            locals[i] = pop!();
                            pc += 4
                        }
                        132 => {
                            let c = (((code[pc + 4] as i16) << 8) | code[pc + 5] as i16) as i32;
                            locals[i] = V::Int(locals[i].int()?.wrapping_add(c));
                            pc += 6
                        }
                        _ => return Err(format!("wide opcode {inner}")),
                    }
                }
                other => return Err(format!("opcode {other} isn't supported")),
            }
        }
    }

    fn multi_array(&mut self, sizes: &[i32]) -> V {
        let n = sizes[0].max(0) as usize;
        if sizes.len() == 1 {
            return self.alloc(Obj::Array(vec![V::Int(0); n]));
        }
        let mut d = Vec::with_capacity(n);
        for _ in 0..n {
            d.push(self.multi_array(&sizes[1..]));
        }
        self.alloc(Obj::Array(d))
    }

    fn get_static(&mut self, class: &str, name: &str) -> VmResult<V> {
        self.init_class(class)?;
        let mut at = Some(class.to_string());
        while let Some(c) = at {
            if let Some(v) = self.statics.get(&c).and_then(|s| s.get(name)) {
                return Ok(v.clone());
            }
            at = self.class(&c).and_then(|k| k.superclass.clone());
        }
        // A static of a class that wasn't set up: an opaque name is all models need of it.
        let v = self.alloc(Obj::Opaque(format!("{class}.{name}")));
        self.statics.entry(class.to_string()).or_default().insert(name.to_string(), v.clone());
        Ok(v)
    }

    fn get_field(&mut self, o: &V, name: &str) -> VmResult<V> {
        let V::Ref(r) = o else { return Err(format!("reading {name} from null")) };
        match &self.heap[*r] {
            Obj::Instance { fields, .. } => Ok(fields.get(name).cloned().unwrap_or(V::Null)),
            Obj::Opaque(_) => Ok(o.clone()),
            other => Err(format!("reading {name} from {other:?}")),
        }
    }

    fn invoke_special(&mut self, owner: &str, name: &str, desc: &str, args: Vec<V>) -> VmResult<V> {
        if let Some((c, m)) = self.find_method(owner, name, desc) {
            return self.run_method(&c, &m, args);
        }
        self.native_call(owner, name, desc, args)
    }

    fn invoke_virtual(&mut self, owner: &str, name: &str, desc: &str, args: Vec<V>) -> VmResult<V> {
        if owner == "net/minecraft/resources/Identifier" {
            return self.native_call(owner, name, desc, args);
        }
        let recv = args.first().cloned().unwrap_or(V::Null);
        if let V::Ref(r) = &recv {
            match &self.heap[*r] {
                Obj::Lambda { .. } => {
                    // A functional interface's method, unless it's a default method on it.
                    if let Some((c, m)) = self.find_method(owner, name, desc) {
                        if m.code.is_some() {
                            return self.run_method(&c, &m, args);
                        }
                    }
                    return self.call_lambda(&recv, args[1..].to_vec());
                }
                Obj::Instance { class, .. } => {
                    let class = class.clone();
                    if let Some((c, m)) = self.find_method(&class, name, desc) {
                        if m.code.is_some() {
                            return self.run_method(&c, &m, args);
                        }
                    }
                }
                _ => {}
            }
        }
        self.native_call(owner, name, desc, args)
    }

    fn invoke_dynamic(&mut self, class: &Rc<Class>, bs: u16, args: Vec<V>) -> VmResult<V> {
        let (handle, bargs) = &class.bootstrap[bs as usize];
        let Cp::Handle(_, m) = class.cp[*handle as usize] else { return Err("bad bootstrap".into()) };
        let (owner, name, _) = class.member(m)?;
        match (owner, name) {
            ("java/lang/invoke/LambdaMetafactory", _) => {
                let Cp::Handle(kind, target) = class.cp[bargs[1] as usize] else { return Err("bad lambda".into()) };
                let (c, n, d) = class.member(target)?;
                Ok(self.alloc(Obj::Lambda { kind, class: c.to_string(), name: n.to_string(), desc: d.to_string(), captured: args }))
            }
            ("java/lang/invoke/StringConcatFactory", "makeConcatWithConstants") => {
                let recipe = match &class.cp[bargs[0] as usize] {
                    Cp::Str(s) => class.utf(*s)?.to_string(),
                    _ => String::new(),
                };
                let mut out = String::new();
                let mut it = args.iter();
                for ch in recipe.chars() {
                    if ch == '\u{1}' {
                        if let Some(a) = it.next() {
                            out.push_str(&self.string_of(a));
                        }
                    } else {
                        out.push(ch);
                    }
                }
                Ok(self.str(&out))
            }
            _ => Err(format!("unsupported invokedynamic {owner}.{name}")),
        }
    }

    /// Classes from outside the jar, and the few inside it that are simpler answered here.
    fn native_call(&mut self, owner: &str, name: &str, desc: &str, args: Vec<V>) -> VmResult<V> {
        let list_new = |vm: &mut Vm, items: Vec<V>| vm.alloc(Obj::List(items));
        let collect = |vm: &Vm, v: &V| vm.list_of(v);
        match (owner, name) {
            ("java/lang/Object", "<init>") | ("java/lang/Record", "<init>") => return Ok(V::Null),
            ("java/lang/Enum", "<init>") => {
                if let V::Ref(r) = &args[0] {
                    if let Obj::Instance { fields, .. } = &mut self.heap[*r] {
                        fields.insert("$name".into(), args[1].clone());
                        fields.insert("$ordinal".into(), args[2].clone());
                    }
                }
                return Ok(V::Null);
            }
            (_, "ordinal") => return Ok(self.get(&args[0], "$ordinal")),
            (_, "name") if desc == "()Ljava/lang/String;" => {
                let n = self.get(&args[0], "$name");
                if n != V::Null {
                    return Ok(n);
                }
                let s = self.string_of(&args[0]);
                let short = s.rsplit('.').next().unwrap_or(&s).to_string();
                return Ok(self.str(&short));
            }
            ("java/lang/Class", "desiredAssertionStatus") => return Ok(V::Int(0)),
            ("java/lang/Math" | "java/lang/StrictMath", _) => return self.math(name, desc, &args),
            ("net/minecraft/util/Mth", "sin") => {
                let x = args[0].double()?;
                let i = ((x * 10430.378350470453) as i64 & 65535) as f64;
                return Ok(V::Float((i * std::f64::consts::TAU / 65536.0).sin() as f32));
            }
            ("net/minecraft/util/Mth", "cos") => {
                let x = args[0].double()?;
                let i = (((x * 10430.378350470453) as i64 + 16384) & 65535) as f64;
                return Ok(V::Float((i * std::f64::consts::TAU / 65536.0).sin() as f32));
            }
            ("java/lang/Float", "valueOf") | ("java/lang/Integer", "valueOf") | ("java/lang/Double", "valueOf") | ("java/lang/Boolean", "valueOf") | ("java/lang/Long", "valueOf") => {
                return Ok(args[0].clone());
            }
            (_, "floatValue") => return Ok(V::Float(args[0].float()?)),
            (_, "doubleValue") => return Ok(V::Double(args[0].double()?)),
            (_, "intValue") => return Ok(V::Int(args[0].int()?)),
            (_, "booleanValue") => return Ok(args[0].clone()),
            ("java/lang/String", "valueOf") => {
                let t = self.string_of(&args[0]);
                return Ok(self.str(&t));
            }
            ("java/lang/StringBuilder", "<init>") => return Ok(V::Null),
            ("java/util/Objects", "requireNonNull") => return Ok(args[0].clone()),
            ("java/util/Optional", "empty") => return Ok(V::Null),
            // The identifiers models use are only ever names.
            ("net/minecraft/resources/Identifier", "withDefaultNamespace") => {
                let t = format!("minecraft:{}", self.string_of(&args[0]));
                return Ok(self.str(&t));
            }
            ("net/minecraft/resources/Identifier", "fromNamespaceAndPath") => {
                let t = format!("{}:{}", self.string_of(&args[0]), self.string_of(&args[1]));
                return Ok(self.str(&t));
            }
            ("net/minecraft/resources/Identifier", "parse") => return Ok(args[0].clone()),
            ("net/minecraft/resources/Identifier", "getPath") => {
                let t = self.string_of(&args[0]);
                let path = t.split(':').next_back().unwrap_or("").to_string();
                return Ok(self.str(&path));
            }
            ("net/minecraft/resources/Identifier", "getNamespace") => {
                let t = self.string_of(&args[0]);
                let ns = t.split(':').next().unwrap_or("minecraft").to_string();
                return Ok(self.str(&ns));
            }
            // Vectors from the maths library the game uses.
            ("org/joml/Vector3f", "<init>") => {
                if let V::Ref(r) = &args[0] {
                    if let Obj::Instance { fields, .. } = &mut self.heap[*r] {
                        let f = |i: usize| args.get(i).cloned().unwrap_or(V::Float(0.0));
                        fields.insert("x".into(), f(1));
                        fields.insert("y".into(), f(2));
                        fields.insert("z".into(), f(3));
                    }
                }
                return Ok(V::Null);
            }
            ("org/joml/Vector3fc" | "org/joml/Vector3f", "x" | "y" | "z") => return Ok(self.get(&args[0], name)),
            ("com/google/common/collect/ImmutableMap", "builder") => return Ok(self.alloc(Obj::Map(Vec::new()))),
            ("com/google/common/collect/Maps", "newEnumMap") | ("com/google/common/collect/ImmutableMap", "copyOf") | ("java/util/Map", "copyOf") => {
                let m = self.map_of(&args[0]);
                return Ok(self.alloc(Obj::Map(m)));
            }
            ("com/google/common/collect/ImmutableMap$Builder", "put") | (_, "put") if matches!(args.first(), Some(V::Ref(r)) if matches!(self.heap[*r], Obj::Map(_))) => {
                let old = self.map_put(&args[0], args[1].clone(), args[2].clone())?;
                return Ok(if owner.contains("Builder") { args[0].clone() } else { old });
            }
            ("com/google/common/collect/ImmutableMap$Builder", "build") | ("com/google/common/collect/ImmutableMap$Builder", "buildOrThrow") => return Ok(args[0].clone()),
            ("com/google/common/collect/Maps", "newHashMap" | "newLinkedHashMap") | ("java/util/HashMap" | "java/util/LinkedHashMap", "<init>") if args.len() <= 1 => {
                if owner.starts_with("java/util") {
                    if let V::Ref(r) = &args[0] {
                        self.heap[*r] = Obj::Map(Vec::new());
                    }
                    return Ok(V::Null);
                }
                return Ok(self.alloc(Obj::Map(Vec::new())));
            }
            ("java/util/Map", "of") | ("com/google/common/collect/ImmutableMap", "of") => {
                let pairs = args.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect();
                return Ok(self.alloc(Obj::Map(pairs)));
            }
            (_, "get") if matches!(args.first(), Some(V::Ref(r)) if matches!(self.heap[*r], Obj::Map(_))) => {
                let m = self.map_of(&args[0]);
                return Ok(m.iter().find(|(k, _)| self.eq(k, &args[1])).map_or(V::Null, |(_, v)| v.clone()));
            }
            (_, "computeIfAbsent") => {
                let m = self.map_of(&args[0]);
                if let Some((_, v)) = m.iter().find(|(k, _)| self.eq(k, &args[1])) {
                    return Ok(v.clone());
                }
                let v = self.call_lambda(&args[2], vec![args[1].clone()])?;
                self.map_put(&args[0], args[1].clone(), v.clone())?;
                return Ok(v);
            }
            (_, "containsKey") => {
                let m = self.map_of(&args[0]);
                return Ok(V::Int(m.iter().any(|(k, _)| self.eq(k, &args[1])) as i32));
            }
            (_, "entrySet") => {
                let m = self.map_of(&args[0]);
                let mut items = Vec::with_capacity(m.len());
                for (k, v) in m {
                    items.push(self.alloc(Obj::Entry(k, v)));
                }
                return Ok(self.alloc(Obj::Set(items)));
            }
            (_, "keySet") => {
                let items = self.map_of(&args[0]).into_iter().map(|(k, _)| k).collect();
                return Ok(self.alloc(Obj::Set(items)));
            }
            (_, "values") if matches!(args.first(), Some(V::Ref(r)) if matches!(self.heap[*r], Obj::Map(_))) => {
                let items = self.map_of(&args[0]).into_iter().map(|(_, v)| v).collect();
                return Ok(self.alloc(Obj::List(items)));
            }
            (_, "putAll") => {
                for (k, v) in self.map_of(&args[1]) {
                    self.map_put(&args[0], k, v)?;
                }
                return Ok(V::Null);
            }
            (_, "forEach") if matches!(args.first(), Some(V::Ref(r)) if matches!(self.heap[*r], Obj::Map(_))) => {
                for (k, v) in self.map_of(&args[0]) {
                    self.call_lambda(&args[1], vec![k, v])?;
                }
                return Ok(V::Null);
            }
            ("java/util/Map$Entry", "getKey") => {
                if let V::Ref(r) = &args[0] {
                    if let Obj::Entry(k, _) = &self.heap[*r] {
                        return Ok(k.clone());
                    }
                }
                return Err("getKey on a non-entry".into());
            }
            ("java/util/Map$Entry", "getValue") => {
                if let V::Ref(r) = &args[0] {
                    if let Obj::Entry(_, v) = &self.heap[*r] {
                        return Ok(v.clone());
                    }
                }
                return Err("getValue on a non-entry".into());
            }
            ("java/util/ArrayList" | "java/util/HashSet" | "java/util/LinkedHashSet", "<init>") => {
                if let V::Ref(r) = &args[0] {
                    let from = args.get(1).filter(|a| !matches!(a, V::Int(_))).map(|a| collect(self, a)).unwrap_or_default();
                    self.heap[*r] = if owner.ends_with("ArrayList") { Obj::List(from) } else { Obj::Set(from) };
                }
                return Ok(V::Null);
            }
            ("com/google/common/collect/Lists", "newArrayList") | ("com/google/common/collect/ImmutableList", "of") | ("java/util/List", "of") | ("java/util/Arrays", "asList") => {
                let items = if args.len() == 1 && desc.starts_with("([") { collect(self, &args[0]) } else { args.clone() };
                return Ok(list_new(self, items));
            }
            ("com/google/common/collect/ImmutableList", "copyOf") | ("java/util/List", "copyOf") => {
                let items = collect(self, &args[0]);
                return Ok(list_new(self, items));
            }
            ("com/google/common/collect/Sets", "newHashSet") | ("java/util/Set", "of") => {
                let items = if args.len() == 1 && desc.starts_with("([") { collect(self, &args[0]) } else { args.clone() };
                return Ok(self.alloc(Obj::Set(items)));
            }
            ("java/util/EnumSet", "allOf") => {
                let cn = match &args[0] {
                    V::Ref(r) => match &self.heap[*r] {
                        Obj::Class(c) => c.clone(),
                        _ => String::new(),
                    },
                    _ => String::new(),
                };
                let items = self.enum_values(&cn)?;
                return Ok(self.alloc(Obj::Set(items)));
            }
            ("java/util/EnumSet", "noneOf") => return Ok(self.alloc(Obj::Set(Vec::new()))),
            ("java/util/EnumSet", "of") => {
                let mut items = Vec::new();
                for (i, a) in args.iter().enumerate() {
                    if i + 1 == args.len() && desc.contains("[Ljava/lang/Enum;") {
                        items.extend(collect(self, a));
                    } else {
                        items.push(a.clone());
                    }
                }
                return Ok(self.alloc(Obj::Set(items)));
            }
            ("java/util/EnumSet", "copyOf") => {
                let items = collect(self, &args[0]);
                return Ok(self.alloc(Obj::Set(items)));
            }
            ("net/minecraft/util/Util", "allOfEnumExcept") => {
                let single = vec![args[0].clone()];
                let cn = self.string_of(&args[0]);
                let class = cn.rsplit_once('.').map_or(String::new(), |(c, _)| c.to_string());
                let items = self.enum_values(&class)?.into_iter().filter(|v| !single.iter().any(|s| self.eq(s, v))).collect();
                return Ok(self.alloc(Obj::Set(items)));
            }
            ("net/minecraft/core/Direction", "values") => {
                let items = self.enum_values("net/minecraft/core/Direction")?;
                return Ok(self.alloc(Obj::Array(items)));
            }
            (_, "values") if desc.starts_with("()[") && owner.contains("WoodType") => return Ok(self.alloc(Obj::List(Vec::new()))),
            (_, "clone") => {
                if let V::Ref(r) = &args[0] {
                    let c = self.heap[*r].clone();
                    return Ok(self.alloc(c));
                }
                return Ok(V::Null);
            }
            _ => {}
        }
        // Collection methods by shape, whichever interface they're called through.
        if let Some(V::Ref(r)) = args.first() {
            let r = *r;
            let is_collection = matches!(self.heap[r], Obj::List(_) | Obj::Set(_) | Obj::Array(_) | Obj::Map(_) | Obj::Iter(..));
            if is_collection {
                match name {
                    "add" => {
                        if let Obj::List(l) | Obj::Set(l) = &mut self.heap[r] {
                            l.push(args[1].clone());
                        }
                        return Ok(V::Int(1));
                    }
                    "addAll" => {
                        let more = collect(self, &args[1]);
                        if let Obj::List(l) | Obj::Set(l) = &mut self.heap[r] {
                            l.extend(more);
                        }
                        return Ok(V::Int(1));
                    }
                    "size" => return Ok(V::Int(collect(self, &args[0]).len() as i32)),
                    "isEmpty" => return Ok(V::Int(collect(self, &args[0]).is_empty() as i32)),
                    "contains" => {
                        let l = collect(self, &args[0]);
                        return Ok(V::Int(l.iter().any(|x| self.eq(x, &args[1])) as i32));
                    }
                    "get" => {
                        let l = collect(self, &args[0]);
                        return l.get(args[1].int()? as usize).cloned().ok_or_else(|| "list index out of bounds".to_string());
                    }
                    "iterator" => {
                        let l = collect(self, &args[0]);
                        return Ok(self.alloc(Obj::Iter(l, 0)));
                    }
                    "hasNext" => {
                        if let Obj::Iter(l, p) = &self.heap[r] {
                            return Ok(V::Int((*p < l.len()) as i32));
                        }
                    }
                    "next" => {
                        if let Obj::Iter(l, p) = &mut self.heap[r] {
                            let v = l.get(*p).cloned().ok_or("iterator ran out")?;
                            *p += 1;
                            return Ok(v);
                        }
                    }
                    "stream" | "toList" | "sequential" | "parallel" | "distinct" => return Ok(args[0].clone()),
                    "filter" => {
                        let mut kept = Vec::new();
                        for v in collect(self, &args[0]) {
                            if self.call_lambda(&args[1], vec![v.clone()])? == V::Int(1) {
                                kept.push(v);
                            }
                        }
                        return Ok(list_new(self, kept));
                    }
                    "map" => {
                        let mut out = Vec::new();
                        for v in collect(self, &args[0]) {
                            out.push(self.call_lambda(&args[1], vec![v])?);
                        }
                        return Ok(list_new(self, out));
                    }
                    "forEach" => {
                        for v in collect(self, &args[0]) {
                            self.call_lambda(&args[1], vec![v])?;
                        }
                        return Ok(V::Null);
                    }
                    "toArray" => {
                        let l = collect(self, &args[0]);
                        return Ok(self.alloc(Obj::Array(l)));
                    }
                    _ => {}
                }
            }
        }
        // A static stream of an array.
        if (owner == "java/util/Arrays" && name == "stream") || (owner == "java/util/stream/Stream" && name == "of") {
            let items = if desc.starts_with("([") { collect(self, &args[0]) } else { args.clone() };
            return Ok(list_new(self, items));
        }
        if owner == "java/util/function/Function" && name == "identity" || owner == "java/util/function/UnaryOperator" && name == "identity" {
            return Ok(self.alloc(Obj::Opaque("identity".into())));
        }
        if name == "apply" {
            if let Some(V::Ref(r)) = args.first() {
                if matches!(&self.heap[*r], Obj::Opaque(s) if s == "identity") {
                    return Ok(args[1].clone());
                }
            }
        }
        // Anything else from the rest of the game: an opaque stand-in that goes nowhere.
        let (_, returns) = parse_args(desc);
        if !returns {
            return Ok(V::Null);
        }
        let ret = &desc[desc.find(')').unwrap_or(0) + 1..];
        Ok(match ret {
            "I" | "Z" | "B" | "S" | "C" => V::Int(0),
            "F" => V::Float(0.0),
            "D" => V::Double(0.0),
            "J" => V::Long(0),
            _ => self.alloc(Obj::Opaque(format!("{owner}.{name}"))),
        })
    }

    fn enum_values(&mut self, class: &str) -> VmResult<Vec<V>> {
        if class == "net/minecraft/core/Direction" {
            let mut out = Vec::new();
            for n in ["DOWN", "UP", "NORTH", "SOUTH", "WEST", "EAST"] {
                out.push(self.get_static(class, n)?);
            }
            return Ok(out);
        }
        Err(format!("values of {class}"))
    }

    /// Returns what the key held before.
    fn map_put(&mut self, map: &V, k: V, v: V) -> VmResult<V> {
        let V::Ref(r) = map else { return Err("putting into null".into()) };
        let same: Option<usize> = match &self.heap[*r] {
            Obj::Map(m) => m.iter().position(|(a, _)| self.eq(a, &k)),
            _ => return Err("putting into a non-map".into()),
        };
        let mut old = V::Null;
        if let Obj::Map(m) = &mut self.heap[*r] {
            match same {
                Some(i) => old = std::mem::replace(&mut m[i].1, v),
                None => m.push((k, v)),
            }
        }
        Ok(old)
    }

    fn math(&mut self, name: &str, desc: &str, a: &[V]) -> VmResult<V> {
        let d = |i: usize| a[i].double();
        Ok(match name {
            "sin" => V::Double(d(0)?.sin()),
            "cos" => V::Double(d(0)?.cos()),
            "sqrt" => V::Double(d(0)?.sqrt()),
            "floor" => V::Double(d(0)?.floor()),
            "ceil" => V::Double(d(0)?.ceil()),
            "atan2" => V::Double(d(0)?.atan2(d(1)?)),
            "pow" => V::Double(d(0)?.powf(d(1)?)),
            "abs" => match desc {
                "(I)I" => V::Int(a[0].int()?.abs()),
                "(F)F" => V::Float(a[0].float()?.abs()),
                _ => V::Double(d(0)?.abs()),
            },
            "max" => match desc {
                "(II)I" => V::Int(a[0].int()?.max(a[1].int()?)),
                "(FF)F" => V::Float(a[0].float()?.max(a[1].float()?)),
                _ => V::Double(d(0)?.max(d(1)?)),
            },
            "min" => match desc {
                "(II)I" => V::Int(a[0].int()?.min(a[1].int()?)),
                "(FF)F" => V::Float(a[0].float()?.min(a[1].float()?)),
                _ => V::Double(d(0)?.min(d(1)?)),
            },
            other => return Err(format!("Math.{other}")),
        })
    }
}
