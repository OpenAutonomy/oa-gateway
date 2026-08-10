use serde_json::{json, Map, Number, Value};

use crate::instance::{Complex, Field, Message, Node, Simple};
use crate::schema::Schema;
use crate::{UciError, MAX_DEPTH};

pub fn from_json(text: &str, schema: &Schema) -> Result<Message, UciError> {
    let value: Value = serde_json::from_str(text).map_err(|e| UciError::Json(e.to_string()))?;
    let obj = value
        .as_object()
        .ok_or_else(|| UciError::Json("root must be an object".into()))?;
    if obj.len() != 1 {
        return Err(UciError::Json(
            "root object must have exactly one member".into(),
        ));
    }
    let (name, body) = obj.iter().next().expect("len == 1");
    let declared = schema
        .global_type(name)
        .ok_or_else(|| UciError::UnknownElement(name.clone()))?;
    let node = read_node(body, schema, declared, name, 0)?;
    Ok(Message {
        name: name.clone(),
        body: node,
    })
}

pub fn to_json(message: &Message, schema: &Schema) -> Result<String, UciError> {
    let declared = schema
        .global_type(&message.name)
        .unwrap_or(message.name.as_str());
    let body = write_node(&message.body, schema, declared, &message.name, 0)?;
    serde_json::to_string(&json!({ &message.name: body }))
        .map_err(|e| UciError::Json(e.to_string()))
}

fn read_node(
    value: &Value,
    schema: &Schema,
    type_name: &str,
    path: &str,
    depth: usize,
) -> Result<Node, UciError> {
    if depth > MAX_DEPTH {
        return Err(UciError::too_deep(path));
    }
    if schema.is_simple(type_name) || !schema.is_complex(type_name) {
        return Ok(Node::Simple(read_simple(
            value,
            schema.primitive(type_name),
            path,
        )?));
    }

    let obj = value.as_object().ok_or_else(|| {
        UciError::at(
            path,
            format!("expected object for complex type {type_name}"),
        )
    })?;

    let actual = obj
        .get("$type")
        .and_then(Value::as_str)
        .unwrap_or(type_name);
    if !schema.is_complex(actual) {
        return Err(UciError::UnknownType(actual.to_owned()));
    }

    let decls: Vec<_> = schema.flatten(actual)?;
    let mut fields = Vec::new();
    for (key, val) in obj {
        if key == "$type" {
            continue;
        }
        let decl = decls.iter().copied().find(|e| e.name == *key);
        let child_type = decl.map(|e| e.type_name.as_str()).unwrap_or("xs:string");
        let array = decl.is_some_and(|e| e.max_occurs.is_array());
        let child_path = format!("{path}.{key}");
        if array {
            let items = match val {
                Value::Array(arr) => arr
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        read_node(
                            v,
                            schema,
                            child_type,
                            &format!("{child_path}[{i}]"),
                            depth + 1,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                other => vec![read_node(
                    other,
                    schema,
                    child_type,
                    &child_path,
                    depth + 1,
                )?],
            };
            fields.push((key.clone(), Field::Many(items)));
        } else {
            fields.push((
                key.clone(),
                Field::One(read_node(val, schema, child_type, &child_path, depth + 1)?),
            ));
        }
    }

    let type_name = if actual != type_name {
        Some(actual.to_owned())
    } else {
        None
    };
    Ok(Node::Complex(Complex { type_name, fields }))
}

fn read_simple(value: &Value, type_name: &str, path: &str) -> Result<Simple, UciError> {
    match (type_name, value) {
        ("xs:boolean", Value::Bool(b)) => Ok(Simple::Bool(*b)),
        ("xs:boolean", Value::String(s)) if s == "true" || s == "1" => Ok(Simple::Bool(true)),
        ("xs:boolean", Value::String(s)) if s == "false" || s == "0" => Ok(Simple::Bool(false)),
        (_, Value::Bool(b)) => Ok(Simple::Bool(*b)),
        (_, Value::Number(n)) => Ok(Simple::Number(n.clone())),
        (_, Value::String(s)) => {
            Ok(parse_numeric_string(type_name, s).unwrap_or_else(|_| Simple::String(s.clone())))
        }
        (_, Value::Null) => Err(UciError::at(path, "null is not allowed")),
        _ => Err(UciError::at(
            path,
            format!("cannot map JSON {value} to {type_name}"),
        )),
    }
}

fn parse_numeric_string(type_name: &str, s: &str) -> Result<Simple, UciError> {
    if matches!(
        type_name,
        "xs:int"
            | "xs:integer"
            | "xs:long"
            | "xs:short"
            | "xs:byte"
            | "xs:decimal"
            | "xs:double"
            | "xs:float"
    ) {
        if let Ok(n) = s.parse::<i64>() {
            return Ok(Simple::Number(n.into()));
        }
        if let Ok(f) = s.parse::<f64>() {
            if let Some(n) = Number::from_f64(f) {
                return Ok(Simple::Number(n));
            }
        }
    }
    Err(UciError::Json("not numeric".into()))
}

fn write_node(
    node: &Node,
    schema: &Schema,
    type_name: &str,
    path: &str,
    depth: usize,
) -> Result<Value, UciError> {
    if depth > MAX_DEPTH {
        return Err(UciError::too_deep(path));
    }
    match node {
        Node::Simple(s) => Ok(write_simple(s)),
        Node::Complex(c) => {
            let actual = c.type_name.as_deref().unwrap_or(type_name);
            let decls = if schema.is_complex(actual) {
                schema.flatten(actual)?
            } else {
                Vec::new()
            };
            let mut map = Map::new();
            if let Some(tn) = &c.type_name {
                map.insert("$type".into(), Value::String(tn.clone()));
            }
            for (name, field) in &c.fields {
                let decl = decls.iter().copied().find(|e| e.name == *name);
                let child_type = decl.map(|e| e.type_name.as_str()).unwrap_or("xs:string");
                let child_path = format!("{path}.{name}");
                let value = match field {
                    Field::One(n) => write_node(n, schema, child_type, &child_path, depth + 1)?,
                    Field::Many(items) => Value::Array(
                        items
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                write_node(
                                    n,
                                    schema,
                                    child_type,
                                    &format!("{child_path}[{i}]"),
                                    depth + 1,
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                };
                map.insert(name.clone(), value);
            }
            Ok(Value::Object(map))
        }
    }
}

fn write_simple(s: &Simple) -> Value {
    match s {
        Simple::String(v) => Value::String(v.clone()),
        Simple::Bool(b) => Value::Bool(*b),
        Simple::Number(n) => Value::Number(n.clone()),
    }
}
