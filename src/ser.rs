use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{
        PyBool, PyBytes, PyDict, PyFloat, PyFunction, PyInt, PyList, PyString, PyTuple, PyType,
        iter::{BoundDictIterator, BoundListIterator, BoundTupleIterator},
    },
};

use crate::thunk_try;

type ThunkResult<'py, E> = crate::thunk::ThunkResult<(), E, Thunk<'py>>;

/// A serialization thunk.
struct Thunk<'py> {
    /// The item to be serialized next.
    item: Bound<'py, PyAny>,
    /// Continuation after the item is serialized.
    continuation: ThunkContinuation<'py>,
}

/// The continuation type of a serialization thunk.
enum ThunkContinuation<'py> {
    /// In-progress sequence serialization.
    SerializingSequence(SequenceIterator<'py>),
    /// In-progress dict serialization.
    SerializingDict(BoundDictIterator<'py>),
}

/// Wrapper for Python sequence iterators (lists and tuples).
enum SequenceIterator<'py> {
    /// List iterator.
    List(BoundListIterator<'py>),
    /// Tuple iterator.
    Tuple(BoundTupleIterator<'py>),
}

impl<'py> From<BoundListIterator<'py>> for SequenceIterator<'py> {
    fn from(value: BoundListIterator<'py>) -> Self {
        Self::List(value)
    }
}

impl<'py> From<BoundTupleIterator<'py>> for SequenceIterator<'py> {
    fn from(value: BoundTupleIterator<'py>) -> Self {
        Self::Tuple(value)
    }
}

impl<'py> Iterator for SequenceIterator<'py> {
    type Item = Bound<'py, PyAny>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::List(list) => list.next(),
            Self::Tuple(tuple) => tuple.next(),
        }
    }
}

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

    /// Pops the top value from the stack of seen objects.
    fn pop_object(&mut self) {
        if let Some(stack) = &mut self.object_stack {
            stack.pop();
        }
    }
}

/// Serialize the given value to the buffer.
fn any_to_json<'py>(state: &mut State<'py>, value: &Bound<'py, PyAny>) -> ThunkResult<'py, PyErr> {
    match (any_to_json_native(state, value), state.object_hook) {
        (ThunkResult::Err(AnyToJsonNativeError::UnsupportedType(_)), Some(hook)) => {
            thunk_try!(state.push_object(value));

            // If we have an object hook, we can try calling that and then
            // serializing the result.
            let r = any_to_json_native(state, &thunk_try!(hook.call1((value,))));

            state.pop_object();

            r
        }

        (r, _) => r,
    }
    .map_err(|e| e.into())
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
) -> ThunkResult<'py, AnyToJsonNativeError<'py>> {
    // The order here matters, as certain types will coerce in a cast.  For
    // example, we have to check bools before trying to cast to PyInt, because
    // bools will coerce to ints.
    if value.is_none() {
        state.buffer.extend(b"null");
    } else if value.is(PyBool::new(value.py(), true)) {
        state.buffer.extend(b"true");
    } else if value.is(PyBool::new(value.py(), false)) {
        state.buffer.extend(b"false");
    } else if let Ok(s) = value.cast::<PyString>() {
        string_to_json(&mut state.buffer, thunk_try!(s.to_str()));
    } else if let Ok(i) = value.cast::<PyInt>() {
        thunk_try!(int_to_json(&mut state.buffer, i));
    } else if let Ok(f) = value.cast::<PyFloat>().map(|f| f.value()) {
        if !f.is_finite() {
            return ThunkResult::Err(AnyToJsonNativeError::Serialization(PyErr::new::<
                PyValueError,
                _,
            >(
                "non-finite floating point number",
            )));
        }

        let mut buf = zmij::Buffer::new();
        state.buffer.extend(buf.format_finite(f).as_bytes());
    } else if let Ok(l) = value.cast::<PyList>() {
        thunk_try!(state.push_object(l));
        return list_to_json(state, l.iter().into()).map_err(|e| e.into());
    } else if let Ok(t) = value.cast::<PyTuple>() {
        thunk_try!(state.push_object(t));
        return list_to_json(state, t.iter().into()).map_err(|e| e.into());
    } else if let Ok(d) = value.cast::<PyDict>() {
        thunk_try!(state.push_object(d));
        return dict_to_json(state, d).map_err(|e| e.into());
    } else if let Ok(f) = value.cast::<Fragment>() {
        state.buffer.extend(f.borrow().0.as_bytes(value.py()));
    } else {
        return ThunkResult::Err(AnyToJsonNativeError::UnsupportedType(value.get_type()));
    }

    ThunkResult::Ok(())
}

fn write_native_int(buf: &mut Vec<u8>, mut v: u64) {
    if v == 0 {
        buf.push(b'0');
        return;
    }

    let mut outbuf = [0; 20];
    let mut outpos = 20;

    while v != 0 {
        outpos -= 1;

        let digit = v % 10;
        v /= 10;

        outbuf[outpos] = (digit as u8) + b'0';
    }

    buf.extend(&outbuf[outpos..]);
}

/// Serialize the given int to the buffer.
fn int_to_json(buf: &mut Vec<u8>, i: &Bound<'_, PyInt>) -> PyResult<()> {
    // Try as u64 and i64 first, since these don't require an allocation.
    // Otherwise, fall back to writing the value as a string, which creates a
    // PyString (and therefore allocates).
    if let Ok(v) = i.extract::<u64>() {
        write_native_int(buf, v);
    } else if let Ok(v) = i.extract::<i64>() {
        match v {
            0 => buf.push(b'0'),

            1.. => write_native_int(buf, v as u64),

            ..0 => {
                buf.push(b'-');
                write_native_int(buf, v.unsigned_abs());
            }
        }
    } else {
        let s = i.str()?;
        buf.extend(s.to_str()?.as_bytes());
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
                const HEX_MAP: &[u8] = b"0123456789abcdef";

                buf.extend([
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX_MAP[usize::from((b & 0xf0) >> 4)],
                    HEX_MAP[usize::from(b & 0x0f)],
                ]);
            }
        }
    }

    buf.push(b'"');
}

/// Serialize the given list to the buffer.
fn list_to_json<'py>(
    state: &mut State<'py>,
    mut items: SequenceIterator<'py>,
) -> ThunkResult<'py, PyErr> {
    state.buffer.push(b'[');

    match items.next() {
        None => {
            state.buffer.push(b']');
            state.pop_object();
            ThunkResult::Ok(())
        }

        Some(item) => ThunkResult::Thunk(Thunk {
            item,
            continuation: ThunkContinuation::SerializingSequence(items),
        }),
    }
}

/// Continue serializing a list.
fn continue_list_to_json<'py>(
    state: &mut State<'py>,
    mut items: SequenceIterator<'py>,
) -> ThunkResult<'py, PyErr> {
    match items.next() {
        None => {
            state.buffer.push(b']');
            state.pop_object();
            ThunkResult::Ok(())
        }

        Some(item) => {
            state.buffer.push(b',');
            ThunkResult::Thunk(Thunk {
                item,
                continuation: ThunkContinuation::SerializingSequence(items),
            })
        }
    }
}

/// Serialize the given pair as a JSON object key and value.
fn write_dict_key<'py>(state: &mut State<'py>, key: &Bound<'py, PyAny>) -> PyResult<()> {
    // Like any_to_json_native, we have to be careful about order here in some
    // cases.  However, none of the types here will coerce to PyString, so we
    // test that first since it's the most likely.
    //
    // While Python's json module supports float keys, we do not.  This is
    // intentional, since floats can have different representations depending on
    // which library is doing the encoding.  The result is effectively
    // "non-portable" keys, which isn't useful.
    if let Ok(s) = key.cast::<PyString>() {
        string_to_json(&mut state.buffer, s.to_str()?);
    } else if key.is(PyBool::new(key.py(), true)) {
        state.buffer.extend(b"\"true\"");
    } else if key.is(PyBool::new(key.py(), false)) {
        state.buffer.extend(b"\"false\"");
    } else if let Ok(i) = key.cast::<PyInt>() {
        // None of the characters in an int will need to be escaped.
        state.buffer.push(b'"');
        int_to_json(&mut state.buffer, i)?;
        state.buffer.push(b'"');
    } else if key.is_none() {
        state.buffer.extend(b"\"null\"");
    } else {
        return Err(PyErr::new::<PyValueError, _>(format!(
            "cannot serialize key type: {}",
            key.get_type(),
        )));
    }

    state.buffer.push(b':');

    Ok(())
}

/// Serialize the given dict to the buffer.
fn dict_to_json<'py>(state: &mut State<'py>, dict: &Bound<'py, PyDict>) -> ThunkResult<'py, PyErr> {
    state.buffer.push(b'{');

    let mut items = dict.iter();

    match items.next() {
        None => {
            state.buffer.push(b'}');
            state.pop_object();
            ThunkResult::Ok(())
        }

        Some((key, value)) => {
            thunk_try!(write_dict_key(state, &key));
            ThunkResult::Thunk(Thunk {
                item: value,
                continuation: ThunkContinuation::SerializingDict(items),
            })
        }
    }
}

/// Continue serializing a dict.
fn continue_dict_to_json<'py>(
    state: &mut State<'py>,
    mut items: BoundDictIterator<'py>,
) -> ThunkResult<'py, PyErr> {
    match items.next() {
        None => {
            state.buffer.push(b'}');
            state.pop_object();
            ThunkResult::Ok(())
        }

        Some((key, value)) => {
            state.buffer.push(b',');
            thunk_try!(write_dict_key(state, &key));
            ThunkResult::Thunk(Thunk {
                item: value,
                continuation: ThunkContinuation::SerializingDict(items),
            })
        }
    }
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

    let mut stack = vec![];

    let mut last_result = any_to_json(&mut state, value);

    loop {
        last_result = match last_result {
            ThunkResult::Err(e) => return Err(e),

            ThunkResult::Thunk(op) => {
                stack.push(op.continuation);

                any_to_json(&mut state, &op.item)
            }

            ThunkResult::Ok(()) => match stack.pop() {
                Some(ThunkContinuation::SerializingSequence(iter)) => {
                    continue_list_to_json(&mut state, iter)
                }

                Some(ThunkContinuation::SerializingDict(iter)) => {
                    continue_dict_to_json(&mut state, iter)
                }

                None => break,
            },
        };
    }

    Ok(PyBytes::new(value.py(), &state.buffer))
}
