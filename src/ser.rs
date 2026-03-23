use std::io::Write;

use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{
        PyBool, PyBytes, PyDict, PyFloat, PyFunction, PyInt, PyList, PyString, PyTuple, PyType,
    },
};

/// Create a JSON fragment from a bytes value, which must contain an
/// already-encoded JSON value.
///
/// If a fragment is encountered during serialization, the fragment contents are
/// emitted directly.  This can be used when one JSON-encoded value needs to be
/// placed inside of a structure that will be serialized to JSON, without the
/// overhead of deserializing it and re-serializing it.
///
/// By default, the provided bytes value is validated and an error thrown if it
/// does not contain valid JSON.  Specify `validate=False` to skip validation.
#[pyclass(frozen)]
pub struct Fragment(Py<PyBytes>);

#[pymethods]
impl Fragment {
    /// Create a new fragment.
    ///
    /// By default, the provided byte string is validated and an error thrown if
    /// validation fails.  Specify `validate=False` to skip validation.
    #[new]
    #[pyo3(signature = (bytes, /, validate = true))]
    fn new<'py>(bytes: Bound<'py, PyBytes>, validate: bool) -> PyResult<Self> {
        if validate {
            crate::de::validate_json(bytes.py(), bytes.as_bytes())?;
        }

        Ok(Self(bytes.into()))
    }
}

/// Serialization state.
struct State<'py> {
    buffer: Vec<u8>,
    object_hook: Option<&'py Bound<'py, PyFunction>>,
    object_stack: Option<Vec<usize>>,
}

impl<'py> State<'py> {
    /// Records that the given object has been seen, returning an error if it
    /// was previously seen.
    fn push_object<T>(&mut self, object: &Bound<'py, T>) -> PyResult<()> {
        if let Some(stack) = &mut self.object_stack {
            let addr = object.as_ptr().addr();

            if stack.contains(&addr) {
                return Err(PyErr::new::<PyValueError, _>("cycle detected"));
            }

            stack.push(addr);
        }

        Ok(())
    }

    /// Pops the given object from the stack of seen objects.
    fn pop_object<T>(&mut self, object: &Bound<'py, T>) {
        if let Some(stack) = &mut self.object_stack {
            let top = stack.pop();

            debug_assert_eq!(top, Some(object.as_ptr().addr()));
        }
    }
}

/// Serialize the given value to the buffer.
fn any_to_json<'py>(state: &mut State<'py>, value: &Bound<'py, PyAny>) -> PyResult<()> {
    match (any_to_json_native(state, value), state.object_hook) {
        (Err(AnyToJsonNativeError::UnsupportedType(_)), Some(hook)) => {
            state.push_object(value)?;

            // If we have an object hook, we can try calling that and then
            // serializing the result.
            let r = any_to_json_native(state, &hook.call1((value,))?);

            state.pop_object(value);

            r
        }

        (r, _) => r,
    }?;

    Ok(())
}

/// Errors that can occur in [`any_to_json_native`].
enum AnyToJsonNativeError<'py> {
    /// The type of the provided value is not supported.
    UnsupportedType(Bound<'py, PyType>),

    /// An error occurred during serialization of a supported value.
    Serialization(PyErr),
}

impl From<PyErr> for AnyToJsonNativeError<'_> {
    fn from(value: PyErr) -> Self {
        AnyToJsonNativeError::Serialization(value)
    }
}

impl From<AnyToJsonNativeError<'_>> for PyErr {
    fn from(value: AnyToJsonNativeError<'_>) -> Self {
        match value {
            AnyToJsonNativeError::UnsupportedType(ty) => {
                PyErr::new::<PyValueError, _>(format!("cannot serialize type as JSON: {ty}",))
            }

            AnyToJsonNativeError::Serialization(e) => e,
        }
    }
}

/// Serialize the given value to the buffer.
///
/// The type of the value must be one of the types natively supported by this
/// library: None, bool, str, int, float, list, tuple, dict, or Fragment.
fn any_to_json_native<'py>(
    state: &mut State<'py>,
    value: &Bound<'py, PyAny>,
) -> Result<(), AnyToJsonNativeError<'py>> {
    if value.is_none() {
        state.buffer.extend(b"null");
    } else if value.is(PyBool::new(value.py(), true)) {
        state.buffer.extend(b"true");
    } else if value.is(PyBool::new(value.py(), false)) {
        state.buffer.extend(b"false");
    } else if let Ok(s) = value.cast::<PyString>() {
        string_to_json(&mut state.buffer, s.to_str()?);
    } else if let Ok(i) = value.cast::<PyInt>() {
        write!(state.buffer, "{i}").unwrap();
    } else if let Ok(f) = value.cast::<PyFloat>().map(|f| f.value()) {
        if !f.is_finite() {
            return Err(AnyToJsonNativeError::Serialization(PyErr::new::<
                PyValueError,
                _,
            >(
                "non-finite floating point number",
            )));
        }

        write!(state.buffer, "{f}").unwrap();
    } else if let Ok(l) = value.cast::<PyList>() {
        state.push_object(l)?;
        list_to_json(state, l.iter())?;
        state.pop_object(l);
    } else if let Ok(t) = value.cast::<PyTuple>() {
        state.push_object(t)?;
        list_to_json(state, t.iter())?;
        state.pop_object(t);
    } else if let Ok(d) = value.cast::<PyDict>() {
        state.push_object(d)?;
        dict_to_json(state, d)?;
        state.pop_object(d);
    } else if let Ok(f) = value.cast::<Fragment>() {
        state.buffer.extend(f.borrow().0.as_bytes(value.py()));
    } else {
        return Err(AnyToJsonNativeError::UnsupportedType(value.get_type()));
    }

    Ok(())
}

/// Serialize the given string to the buffer.
fn string_to_json(buf: &mut Vec<u8>, s: &str) {
    // We are going to push at least this many more bytes, but maybe more if
    // escape sequences are required.
    buf.reserve(s.len() + 2);

    buf.push(b'"');

    for &b in s.as_bytes() {
        match b {
            b'\\' | b'"' => buf.extend([b'\\', b]),

            b' '.. => buf.push(b),

            0..b' ' => {
                buf.extend(b"\\u00");

                const HEX_MAP: &[u8] = b"0123456789abcdef";

                buf.push(HEX_MAP[usize::from((b & 0xf0) >> 4)]);
                buf.push(HEX_MAP[usize::from(b & 0x0f)]);
            }
        }
    }

    buf.push(b'"');
}

/// Serialize the given list to the buffer.
fn list_to_json<'py>(
    state: &mut State<'py>,
    list: impl IntoIterator<Item = Bound<'py, PyAny>>,
) -> PyResult<()> {
    state.buffer.push(b'[');

    let mut items = list.into_iter();

    if let Some(i) = items.next() {
        any_to_json(state, &i)?;
        drop(i);

        for i in items {
            state.buffer.push(b',');
            any_to_json(state, &i)?;
        }
    }

    state.buffer.push(b']');

    Ok(())
}

/// Serialize the given pair as a JSON object key and value.
fn dict_item_to_json<'py>(
    state: &mut State<'py>,
    key: &Bound<'py, PyAny>,
    item: &Bound<'py, PyAny>,
) -> PyResult<()> {
    let key = key.str()?;
    string_to_json(&mut state.buffer, key.to_str()?);

    state.buffer.push(b':');

    any_to_json(state, item)?;

    Ok(())
}

/// Serialize the given dict to the buffer.
fn dict_to_json<'py>(state: &mut State<'py>, dict: &Bound<'py, PyDict>) -> PyResult<()> {
    state.buffer.push(b'{');

    let mut items = dict.iter();

    if let Some((key, value)) = items.next() {
        dict_item_to_json(state, &key, &value)?;

        drop((key, value));

        for (key, value) in items {
            state.buffer.push(b',');

            dict_item_to_json(state, &key, &value)?;
        }
    }

    state.buffer.push(b'}');

    Ok(())
}

/// Serialize the given value as JSON.
pub fn into_json<'py>(
    value: &Bound<'py, PyAny>,
    object_hook: Option<&'py Bound<'py, PyFunction>>,
    check_circular: bool,
) -> PyResult<Bound<'py, PyBytes>> {
    let mut state = State {
        buffer: vec![],
        object_hook,
        object_stack: check_circular.then(Vec::new),
    };

    any_to_json(&mut state, value)?;

    Ok(PyBytes::new(value.py(), &state.buffer))
}
