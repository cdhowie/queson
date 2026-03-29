use pyo3::prelude::*;

mod de;
#[cfg(all(feature = "pymem-alloc", not(windows)))]
mod pymem;
mod ser;
mod simd;
mod thunk;

#[pymodule]
mod queson {
    use std::num::NonZeroUsize;

    use pyo3::{
        exceptions::PyTypeError,
        prelude::*,
        types::{PyBytes, PyFunction, PyString},
    };

    /// Deserialize a JSON-encoded value.
    ///
    /// This function accepts either `bytes` or `str`.  A `bytes` will be more
    /// efficient, as a `str` will be UTF-8 encoded first.
    #[pyfunction]
    #[pyo3(signature = (json, *, object_hook = None, depth_limit = None))]
    fn loads<'py>(
        json: &Bound<'py, PyAny>,
        object_hook: Option<&'py Bound<'py, PyFunction>>,
        depth_limit: Option<NonZeroUsize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let bytes = if let Ok(s) = json.cast::<PyString>() {
            s.to_str()?.as_bytes()
        } else if let Ok(b) = json.cast::<PyBytes>() {
            b.as_bytes()
        } else {
            return Err(PyErr::new::<PyTypeError, _>("expected a str or bytes"));
        };

        crate::de::parse_json(json.py(), bytes, object_hook, depth_limit)
    }

    /// Deserialize a JSON-encoded value.
    ///
    /// This function accepts either `bytes` or `str`.  A `bytes` will be more
    /// efficient, as a `str` will be UTF-8 encoded first.
    #[pyfunction]
    #[pyo3(signature = (json, *, object_hook = None, depth_limit = None))]
    fn loadb<'py>(
        json: &Bound<'py, PyAny>,
        object_hook: Option<&'py Bound<'py, PyFunction>>,
        depth_limit: Option<NonZeroUsize>,
    ) -> PyResult<Bound<'py, PyAny>> {
        loads(json, object_hook, depth_limit)
    }

    /// Serialize a value into a JSON `str`.
    ///
    /// Consider using `dumpb` instead, as it will be faster when you can use a
    /// UTF-8 encoded `bytes` instead.
    #[pyfunction]
    #[pyo3(signature = (value, *, object_hook = None, check_circular = true))]
    fn dumps<'py>(
        value: &Bound<'py, PyAny>,
        object_hook: Option<&'py Bound<'py, PyFunction>>,
        check_circular: bool,
    ) -> PyResult<Bound<'py, PyString>> {
        PyString::from_bytes(
            value.py(),
            &crate::ser::into_json(value, object_hook, check_circular)?,
        )
    }

    /// Serialize a value into a UTF-8 encoded JSON `bytes`.
    #[pyfunction]
    #[pyo3(signature = (value, *, object_hook = None, check_circular = true))]
    fn dumpb<'py>(
        value: &Bound<'py, PyAny>,
        object_hook: Option<&'py Bound<'py, PyFunction>>,
        check_circular: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(
            value.py(),
            &crate::ser::into_json(value, object_hook, check_circular)?,
        ))
    }

    #[pymodule_export]
    use crate::ser::Fragment;
}
