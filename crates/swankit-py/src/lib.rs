//! Thin Python adapters: definitions and results retain their Rust owner types.
mod codecs;
mod graphics;
mod saves;
mod sound;
mod text;

use formatkit_core::Error;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::Serialize;

pyo3::create_exception!(swankit, SwankitError, PyValueError);
pyo3::create_exception!(swankit, ResourceLimitError, SwankitError);
pyo3::create_exception!(swankit, UnsupportedError, SwankitError);

fn error(e: Error) -> PyErr {
    let message = e.to_string();
    match e.leaf() {
        Error::ResourceLimit { .. } => ResourceLimitError::new_err(message),
        Error::Unsupported(_) | Error::UnsupportedDialect { .. } => {
            UnsupportedError::new_err(message)
        }
        _ => SwankitError::new_err(message),
    }
}
fn bad(message: impl Into<String>) -> PyErr {
    error(Error::Malformed(message.into()))
}
fn json<T: Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    let serialized = serde_json::to_string(value).map_err(|e| bad(e.to_string()))?;
    Ok(py
        .import("json")?
        .call_method1("loads", (serialized,))?
        .unbind())
}
fn toolkit() -> PyResult<swankit::Toolkit> {
    swankit::Toolkit::new().map_err(error)
}

/// Cartridge facts from the product's checked ws-rom parser.
#[pyclass(frozen, module = "swankit")]
struct RomFacts(swankit::RomInfo);
#[pymethods]
impl RomFacts {
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json(py, &self.0)
    }
    #[getter]
    fn size_bytes(&self) -> usize {
        self.0.size_bytes
    }
    #[getter]
    fn checksum_ok(&self) -> bool {
        self.0.checksum_ok
    }
    #[getter]
    fn color(&self) -> bool {
        self.0.color
    }
    #[getter]
    fn game_id(&self) -> u8 {
        self.0.game_id
    }
    #[getter]
    fn publisher_id(&self) -> u8 {
        self.0.publisher_id
    }
    #[getter]
    fn declared_rom_bytes(&self) -> Option<usize> {
        self.0.declared_rom_bytes
    }
    #[getter]
    fn save_bytes(&self) -> Option<u32> {
        self.0.save_bytes
    }
    fn __repr__(&self) -> String {
        format!(
            "RomFacts(size_bytes={}, checksum_ok={})",
            self.0.size_bytes, self.0.checksum_ok
        )
    }
}
#[pyfunction]
fn rom_facts(data: &[u8]) -> PyResult<RomFacts> {
    toolkit()?.info(data).map(RomFacts).map_err(error)
}

#[pyclass(frozen, module = "swankit")]
struct Detection(swankit::Detection);
#[pymethods]
impl Detection {
    #[getter]
    fn selected(&self) -> Option<&str> {
        self.0.selected.as_deref()
    }
    #[getter]
    fn confidence(&self) -> Option<&str> {
        self.0.confidence.as_deref()
    }
    #[getter]
    fn candidates(&self) -> Vec<String> {
        self.0.candidates.clone()
    }
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json(py, &self.0)
    }
}
#[pyfunction]
#[pyo3(signature = (data, extension=None, *, name="save", context=None))]
fn detect(
    data: &[u8],
    extension: Option<&str>,
    name: &str,
    context: Option<&saves::SaveContext>,
) -> PyResult<Detection> {
    let t = toolkit()?;
    match context {
        Some(c) => t
            .detect_with_save_context(data, extension, name, &c.0)
            .map(Detection)
            .map_err(error),
        None => Ok(Detection(t.detect(data, extension))),
    }
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("SwankitError", m.py().get_type::<SwankitError>())?;
    m.add(
        "ResourceLimitError",
        m.py().get_type::<ResourceLimitError>(),
    )?;
    m.add("UnsupportedError", m.py().get_type::<UnsupportedError>())?;
    m.add_class::<RomFacts>()?;
    m.add_class::<Detection>()?;
    m.add_function(wrap_pyfunction!(rom_facts, m)?)?;
    m.add_function(wrap_pyfunction!(detect, m)?)?;
    codecs::register(m)?;
    graphics::register(m)?;
    text::register(m)?;
    sound::register(m)?;
    saves::register(m)?;
    Ok(())
}
