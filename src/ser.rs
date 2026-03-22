use std::io::Write;

use pyo3::{
    exceptions::PyValueError,
    prelude::*,
    types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString},
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

/// Serialize the given value to the buffer.
fn any_to_json<'py>(buf: &mut Vec<u8>, value: &Bound<'py, PyAny>) -> PyResult<()> {
    if value.is_none() {
        buf.extend(b"null");
    } else if value.is(PyBool::new(value.py(), true)) {
        buf.extend(b"true");
    } else if value.is(PyBool::new(value.py(), false)) {
        buf.extend(b"false");
    } else if let Ok(s) = value.cast::<PyString>() {
        string_to_json(buf, s.to_str()?);
    } else if let Ok(i) = value.cast::<PyInt>() {
        write!(buf, "{i}").unwrap();
    } else if let Ok(f) = value.cast::<PyFloat>() {
        write!(buf, "{}", f.value()).unwrap();
    } else if let Ok(l) = value.cast::<PyList>() {
        list_to_json(buf, l)?;
    } else if let Ok(d) = value.cast::<PyDict>() {
        dict_to_json(buf, d)?;
    } else if let Ok(f) = value.cast::<Fragment>() {
        buf.extend(f.borrow().0.as_bytes(value.py()));
    } else {
        return Err(PyErr::new::<PyValueError, _>(format!(
            "cannot serialize type as JSON: {}",
            value.get_type()
        )));
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
fn list_to_json(buf: &mut Vec<u8>, list: &Bound<'_, PyList>) -> PyResult<()> {
    buf.push(b'[');

    let mut items = list.iter();

    if let Some(i) = items.next() {
        any_to_json(buf, &i)?;
        drop(i);

        for i in items {
            buf.push(b',');
            any_to_json(buf, &i)?;
        }
    }

    buf.push(b']');

    Ok(())
}

/// Serialize the given pair as a JSON object key and value.
fn dict_item_to_json(
    buf: &mut Vec<u8>,
    key: &Bound<'_, PyAny>,
    item: &Bound<'_, PyAny>,
) -> PyResult<()> {
    let key = key.str()?;
    string_to_json(buf, key.to_str()?);

    buf.push(b':');

    any_to_json(buf, item)?;

    Ok(())
}

/// Serialize the given dict to the buffer.
fn dict_to_json(buf: &mut Vec<u8>, dict: &Bound<'_, PyDict>) -> PyResult<()> {
    buf.push(b'{');

    let mut items = dict.iter();

    if let Some((key, value)) = items.next() {
        dict_item_to_json(buf, &key, &value)?;

        drop((key, value));

        for (key, value) in items {
            buf.push(b',');

            dict_item_to_json(buf, &key, &value)?;
        }
    }

    buf.push(b'}');

    Ok(())
}

/// Serialize the given value as JSON.
pub fn into_json<'py>(value: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyBytes>> {
    let mut buf = vec![];

    any_to_json(&mut buf, value)?;

    Ok(PyBytes::new(value.py(), &buf))
}
