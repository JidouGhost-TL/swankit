use crate::{error, json};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde_json::{json, Value};
use swankit::saves::{self, Format};

#[pyclass(frozen, module = "swankit")]
pub struct SaveContext(pub saves::DecodeContext);
#[pymethods]
impl SaveContext {
    #[new]
    #[pyo3(signature = (definition_json=None))]
    fn new(definition_json: Option<&str>) -> PyResult<Self> {
        match definition_json {
            None => Ok(Self(saves::DecodeContext::default())),
            Some(s) => swankit::save::context_from_json(s.as_bytes())
                .map(Self)
                .map_err(error),
        }
    }
}
// Builtins borrow the owner catalog; caller-supplied definitions stay owned.
enum Owner {
    Builtin(&'static Format),
    Loaded(Box<Format>),
}
#[pyclass(frozen, module = "swankit")]
struct SaveDefinition(Owner);
impl SaveDefinition {
    fn owner(&self) -> &Format {
        match &self.0 {
            Owner::Builtin(f) => f,
            Owner::Loaded(f) => f,
        }
    }
}
#[pymethods]
impl SaveDefinition {
    #[new]
    fn new(definition_json: &str) -> PyResult<Self> {
        Format::from_json(definition_json)
            .map(|f| Self(Owner::Loaded(Box::new(f))))
            .map_err(error)
    }
    #[staticmethod]
    fn builtin(id: &str) -> PyResult<Self> {
        saves::format(id)
            .map(|f| Self(Owner::Builtin(f)))
            .map_err(error)
    }
    #[getter]
    fn id(&self) -> &str {
        &self.owner().id
    }
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json(py, self.owner().definition())
    }
    #[getter]
    fn fields(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let fields: Vec<_> = self.owner().fields.iter().enumerate().map(|(index,f)| json!({
            "index":index,"name":f.name,"medium":f.medium,"offset":f.offset,"size":f.size,"encoding":f.encoding
        })).collect();
        json(py, &fields)
    }
    fn accepts_size(&self, size: usize) -> bool {
        self.owner().accepts_size(size)
    }
    #[pyo3(signature = (data, *, name="save", context=None))]
    fn decode(
        &self,
        py: Python<'_>,
        data: &[u8],
        name: &str,
        context: Option<&SaveContext>,
    ) -> PyResult<Py<PyAny>> {
        let empty = saves::DecodeContext::default();
        let result = crate::toolkit()?
            .decode_save(self.owner(), data, name, context.map_or(&empty, |c| &c.0))
            .map_err(error)?;
        json(py, &result)
    }
    /// Positive validity evidence or None; this follows the owner's candidate policy.
    #[pyo3(signature = (data, *, name="save", context=None))]
    fn verify(
        &self,
        py: Python<'_>,
        data: &[u8],
        name: &str,
        context: Option<&SaveContext>,
    ) -> PyResult<Py<PyAny>> {
        let empty = saves::DecodeContext::default();
        let result = self
            .owner()
            .recognize_with_context(data, name, context.map_or(&empty, |c| &c.0))
            .map_err(error)?;
        json(py, &result)
    }
    #[pyo3(signature = (data, field, value, *, all_copies=false, skip_validation=false, repair=true, name="save", context=None))]
    #[allow(clippy::too_many_arguments)]
    fn set_field<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
        field: usize,
        value: &Bound<'_, PyAny>,
        all_copies: bool,
        skip_validation: bool,
        repair: bool,
        name: &str,
        context: Option<&SaveContext>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let serialized: String = py
            .import("json")?
            .call_method1("dumps", (value,))?
            .extract()?;
        let value: Value = swankit::extract::definition(serialized.as_bytes()).map_err(error)?;
        let empty = saves::DecodeContext::default();
        let options = swankit::SaveEditOptions {
            all_copies,
            skip_validation,
            repair,
        };
        let result = if repair {
            crate::toolkit()?
                .set_save_with_options(
                    self.owner(),
                    data,
                    name,
                    context.map_or(&empty, |c| &c.0),
                    field,
                    &value,
                    options,
                )
                .map_err(error)?
        } else {
            crate::toolkit()?
                .set_save_unrepaired(
                    self.owner(),
                    data,
                    name,
                    context.map_or(&empty, |c| &c.0),
                    field,
                    &value,
                    options,
                )
                .map_err(error)?
        };
        Ok(PyBytes::new(py, &result))
    }
    #[pyo3(signature = (data, *, name="save", context=None))]
    fn fix<'py>(
        &self,
        py: Python<'py>,
        data: &[u8],
        name: &str,
        context: Option<&SaveContext>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let empty = saves::DecodeContext::default();
        let result = crate::toolkit()?
            .fix_save(self.owner(), data, name, context.map_or(&empty, |c| &c.0))
            .map_err(error)?;
        Ok(PyBytes::new(py, &result))
    }
}
#[pyfunction]
fn save_definitions() -> PyResult<Vec<String>> {
    saves::formats()
        .map(|f| f.iter().map(|f| f.id.clone()).collect())
        .map_err(error)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<SaveContext>()?;
    m.add_class::<SaveDefinition>()?;
    m.add_function(wrap_pyfunction!(save_definitions, m)?)?;
    Ok(())
}
