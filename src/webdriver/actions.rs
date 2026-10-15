//! W3C actions (`POST /actions`), checked and cut into ticks before any of
//! them runs.
//!
//! The page simulates one mouse and one keyboard with DOM events. The input
//! sources served are `none` (pauses), `key` (key down and up) and `pointer`
//! with the pointer type `mouse` (move, button down and up, pauses). Wheel
//! input, pen and touch pointers and `pointerCancel` answer `unsupported
//! operation`.

use std::time::Duration;

use serde_json::{Value, json};

use super::{ELEMENT_KEY, WebDriverError, invalid, milliseconds, unsupported};

/// One tick of an action sequence: the i-th action of every input source.
#[derive(Debug, PartialEq)]
pub(super) struct Tick {
    /// The input the page dispatches, in the order of the input sources. A
    /// pause dispatches nothing.
    pub inputs: Vec<Value>,
    /// How long the tick lasts: its longest pause or pointer move.
    pub duration: Duration,
}

/// One checked action of an input source.
struct Step {
    /// What the page dispatches, or `None` for a pause.
    input: Option<Value>,
    /// The duration of a pause or a pointer move, in milliseconds.
    duration: u64,
}

/// Checks the body of Perform Actions and cuts it into ticks.
///
/// # Errors
///
/// `invalid argument` for a body that is not an action sequence, and
/// `unsupported operation` for input the page does not simulate.
pub(super) fn ticks(body: &Value) -> Result<Vec<Tick>, WebDriverError> {
    let sources = body
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("`actions` must be an array of input sources"))?
        .iter()
        .map(source)
        .collect::<Result<Vec<_>, _>>()?;
    let length = sources.iter().map(Vec::len).max().unwrap_or(0);
    Ok((0..length)
        .map(|index| {
            let mut tick = Tick {
                inputs: Vec::new(),
                duration: Duration::ZERO,
            };
            for step in sources.iter().filter_map(|steps| steps.get(index)) {
                tick.duration = tick.duration.max(Duration::from_millis(step.duration));
                if let Some(input) = &step.input {
                    tick.inputs.push(input.clone());
                }
            }
            tick
        })
        .collect())
}

/// The checked actions of one input source.
fn source(source: &Value) -> Result<Vec<Step>, WebDriverError> {
    let id = source
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("an input source needs an `id`"))?;
    let kind = source
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("input source `{id}` needs a `type`")))?;
    match kind {
        "none" | "key" => {}
        "pointer" => {
            let pointer = source
                .pointer("/parameters/pointerType")
                .and_then(Value::as_str)
                .unwrap_or("mouse");
            if pointer != "mouse" {
                return Err(unsupported(format!(
                    "a {pointer} pointer is not simulated, only the mouse"
                )));
            }
        }
        "wheel" => return Err(unsupported("wheel input (scrolling) is not simulated")),
        other => return Err(invalid(format!("`{other}` is not a type of input source"))),
    }
    source
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("input source `{id}` needs an array of `actions`")))?
        .iter()
        .map(|action| step(id, kind, action))
        .collect()
}

/// One checked action of the input source `id` of type `kind`.
fn step(id: &str, kind: &str, action: &Value) -> Result<Step, WebDriverError> {
    let subtype = action
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("an action of `{id}` needs a `type`")))?;
    // Only a pause and a pointer move last; other actions may carry a
    // duration (thirtyfour sends one on button actions), which is ignored.
    let duration = match (subtype, action.get("duration")) {
        ("pause" | "pointerMove", Some(value)) if !value.is_null() => {
            milliseconds("duration", value)?
        }
        _ => 0,
    };
    let input = match (kind, subtype) {
        (_, "pause") => None,
        ("key", "keyDown" | "keyUp") => {
            let value = action
                .get("value")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| invalid("the `value` of a key action must be a key"))?;
            Some(json!({ "source": id, "type": subtype, "value": value }))
        }
        ("pointer", "pointerDown" | "pointerUp") => {
            let button = action
                .get("button")
                .and_then(Value::as_u64)
                .filter(|button| *button <= 4)
                .ok_or_else(|| invalid("`button` must be a mouse button from 0 to 4"))?;
            Some(json!({ "source": id, "type": subtype, "button": button }))
        }
        ("pointer", "pointerMove") => Some(json!({
            "source": id,
            "type": subtype,
            "origin": origin(action)?,
            "x": coordinate(action, "x")?,
            "y": coordinate(action, "y")?,
        })),
        ("pointer", "pointerCancel") => {
            return Err(unsupported("`pointerCancel` is not simulated"));
        }
        _ => {
            return Err(invalid(format!(
                "`{subtype}` is not an action of a {kind} input source"
            )));
        }
    };
    Ok(Step { input, duration })
}

/// Where a pointer move starts from: `viewport` (the default), `pointer`, or
/// an element, for the page as `{ "element": id }`.
fn origin(action: &Value) -> Result<Value, WebDriverError> {
    match action.get("origin") {
        None => Ok(json!("viewport")),
        Some(Value::String(origin)) if origin == "viewport" || origin == "pointer" => {
            Ok(json!(origin))
        }
        Some(Value::Object(reference)) => reference
            .get(ELEMENT_KEY)
            .and_then(Value::as_str)
            .map(|element| json!({ "element": element }))
            .ok_or_else(|| invalid("`origin` is an object but not an element reference")),
        Some(_) => Err(invalid(
            "`origin` must be `viewport`, `pointer` or an element",
        )),
    }
}

/// The `x` or `y` of a pointer move: a number, 0 when left out.
fn coordinate(action: &Value, key: &str) -> Result<Value, WebDriverError> {
    match action.get(key) {
        None => Ok(json!(0)),
        Some(value) if value.is_number() => Ok(value.clone()),
        Some(_) => Err(invalid(format!("`{key}` must be a number"))),
    }
}
