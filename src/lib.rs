use pyo3::prelude::*;

mod de;
#[cfg(all(feature = "pymem-alloc", not(windows)))]
mod pymem;
mod ser;
mod simd;
mod thunk;

#[pymodule]
mod queson {
    use pyo3::{
        exceptions::PyTypeError,
        prelude::*,
        types::{PyBytes, PyFunction, PyString},
    };

    /// Deserialize a JSON-encoded value.
    #[pyfunction]
    #[pyo3(signature = (json, /, object_hook = None))]
    fn loads<'py>(
        json: &Bound<'py, PyAny>,
        object_hook: Option<&'py Bound<'py, PyFunction>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let bytes = if let Ok(s) = json.cast::<PyString>() {
            s.to_str()?.as_bytes()
        } else if let Ok(b) = json.cast::<PyBytes>() {
            b.as_bytes()
        } else {
            return Err(PyErr::new::<PyTypeError, _>("expected a str or bytes"));
        };

        crate::de::parse_json(json.py(), bytes, object_hook)
    }

    /// Serialize a value as JSON.
    #[pyfunction]
    #[pyo3(signature = (value, /, object_hook = None, check_circular = true))]
    fn dumps<'py>(
        value: &Bound<'py, PyAny>,
        object_hook: Option<&'py Bound<'py, PyFunction>>,
        check_circular: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        crate::ser::into_json(value, object_hook, check_circular)
    }

    #[pymodule_export]
    use crate::ser::Fragment;
}
