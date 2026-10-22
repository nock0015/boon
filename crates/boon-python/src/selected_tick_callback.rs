//! Python delivery for measured selected-tick counts, separate from spool pressure.
use pyo3::prelude::*;
pub(super) fn emit(callback: &Py<PyAny>, counts: (usize, usize), phase: &str) -> PyResult<()> {
    Python::attach(|py| callback.call1(py, (counts.0, counts.1, phase)).map(|_| ()))
}
