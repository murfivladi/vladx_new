//! Значения языка и окружения (скоупы).

use crate::interp::{Interp, R};
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use vladx_ast::{Block, Param, Span};

#[derive(Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    List(Rc<RefCell<Vec<Value>>>),
    /// Словарь и объект — одно и то же; у объекта есть прототип.
    Map(Rc<RefCell<MapObj>>),
    Func(Rc<Closure>),
    Native(Rc<NativeFn>),
    Module(Rc<Module>),
    /// Ошибка исполнения, пойманная через «попробуй/поймай».
    Error(Rc<ErrorVal>),
}

pub struct MapObj {
    pub entries: IndexMap<String, Value>,
    pub proto: Option<Value>,
}

pub struct Closure {
    pub name: Option<String>,
    pub params: Vec<Param>,
    pub body: Rc<Block>,
    pub env: Rc<Env>,
    /// Модуль, где функция объявлена (None — главная программа).
    pub file: Option<Rc<str>>,
}

pub type NativeImpl = fn(&mut Interp, &[Value], Span) -> R<Value>;

pub struct NativeFn {
    pub name: String,
    pub min: usize,
    pub max: usize,
    pub f: NativeImpl,
}

pub struct Module {
    pub name: String,
    /// Имена для подсказок (на языке программы); пусто — все ключи `fns`.
    pub names: Vec<String>,
    pub fns: IndexMap<String, Value>,
}

pub struct ErrorVal {
    pub code: String,
    pub message: String,
}

impl Value {
    pub fn str(s: impl AsRef<str>) -> Value {
        Value::Str(Rc::from(s.as_ref()))
    }
    pub fn list(v: Vec<Value>) -> Value {
        Value::List(Rc::new(RefCell::new(v)))
    }
    pub fn map(entries: IndexMap<String, Value>, proto: Option<Value>) -> Value {
        Value::Map(Rc::new(RefCell::new(MapObj { entries, proto })))
    }

    /// Нейтральный id типа (совпадает с id в `[[type]]` словаря).
    pub fn type_id(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::List(_) => "list",
            Value::Map(_) => "map",
            Value::Func(_) | Value::Native(_) => "function",
            Value::Module(_) => "module",
            Value::Error(_) => "error",
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }
    }
}

/// Название типа для сообщений.
pub fn type_name(id: &str) -> &'static str {
    match id {
        "null" => "ничего",
        "bool" => "логическое",
        "number" => "число",
        "string" => "строка",
        "list" => "список",
        "map" => "словарь",
        "function" => "функция",
        "module" => "модуль",
        "error" => "ошибка",
        _ => "значение",
    }
}

pub struct Binding {
    pub value: Value,
    pub mutable: bool,
}

pub struct Env {
    pub vars: RefCell<HashMap<String, Binding>>,
    pub parent: Option<Rc<Env>>,
    pub this: Option<Value>,
}

pub enum AssignErr {
    NotFound,
    Immutable,
}

impl Env {
    pub fn new(parent: Option<Rc<Env>>, this: Option<Value>) -> Rc<Env> {
        Rc::new(Env { vars: RefCell::new(HashMap::new()), parent, this })
    }

    pub fn define(&self, name: &str, value: Value, mutable: bool) {
        self.vars.borrow_mut().insert(name.to_string(), Binding { value, mutable });
    }

    pub fn has_own(&self, name: &str) -> bool {
        self.vars.borrow().contains_key(name)
    }

    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(b) = self.vars.borrow().get(name) {
            return Some(b.value.clone());
        }
        let mut cur = self.parent.clone();
        while let Some(e) = cur {
            if let Some(b) = e.vars.borrow().get(name) {
                return Some(b.value.clone());
            }
            cur = e.parent.clone();
        }
        None
    }

    /// Изменяемая ли привязка (None — имени нет).
    pub fn mutability(&self, name: &str) -> Option<bool> {
        if let Some(b) = self.vars.borrow().get(name) {
            return Some(b.mutable);
        }
        let mut cur = self.parent.clone();
        while let Some(e) = cur {
            if let Some(b) = e.vars.borrow().get(name) {
                return Some(b.mutable);
            }
            cur = e.parent.clone();
        }
        None
    }

    pub fn assign(&self, name: &str, value: Value) -> Result<(), AssignErr> {
        if let Some(b) = self.vars.borrow_mut().get_mut(name) {
            if !b.mutable {
                return Err(AssignErr::Immutable);
            }
            b.value = value;
            return Ok(());
        }
        match &self.parent {
            Some(p) => p.assign(name, value),
            None => Err(AssignErr::NotFound),
        }
    }

    pub fn this(&self) -> Option<Value> {
        if let Some(t) = &self.this {
            return Some(t.clone());
        }
        let mut cur = self.parent.clone();
        while let Some(e) = cur {
            if let Some(t) = &e.this {
                return Some(t.clone());
            }
            cur = e.parent.clone();
        }
        None
    }
}
