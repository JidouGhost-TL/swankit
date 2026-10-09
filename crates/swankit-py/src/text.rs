use crate::{error, graphics::Image, json};
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use serde_json::json;
use swankit::text::{self, CharacterMap, TextLimits};

#[pyclass(frozen, module = "swankit")]
struct TextResult(text::DecodedText);
#[pymethods]
impl TextResult {
    #[getter]
    fn text(&self) -> &str {
        &self.0.text
    }
    #[getter]
    fn normalized(&self) -> &str {
        &self.0.normalized
    }
    #[getter]
    fn consumed(&self) -> usize {
        self.0.consumed
    }
    #[getter]
    fn terminated(&self) -> bool {
        self.0.terminated
    }
    #[getter]
    fn final_page(&self) -> u8 {
        self.0.final_page
    }
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json(
            py,
            &json!({"text": self.0.text, "normalized": self.0.normalized,
            "consumed": self.0.consumed, "terminated": self.0.terminated, "final_page": self.0.final_page,
            "tokens": self.0.tokens.iter().map(|t| json!({"bytes":t.bytes,"text":t.text})).collect::<Vec<_>>() }),
        )
    }
}
#[pyclass(frozen, module = "swankit")]
struct TextDefinition {
    definition: text::TextDefinition,
    map: CharacterMap,
}
impl TextDefinition {
    fn wrap(definition: text::TextDefinition) -> PyResult<Self> {
        let map = definition.character_map().map_err(error)?;
        Ok(Self { definition, map })
    }
}
#[pymethods]
impl TextDefinition {
    #[new]
    fn new(definition_json: &str) -> PyResult<Self> {
        Self::wrap(text::TextDefinition::from_json(definition_json.as_bytes()).map_err(error)?)
    }
    #[staticmethod]
    fn builtin(id: &str) -> PyResult<Self> {
        Self::wrap(text::builtin_definition(id).map_err(error)?)
    }
    #[getter]
    fn id(&self) -> &str {
        &self.definition.id
    }
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        json(py, &self.definition)
    }
    fn decode(&self, data: &[u8]) -> PyResult<TextResult> {
        self.map
            .decode(data, TextLimits::default())
            .map(TextResult)
            .map_err(error)
    }
    #[pyo3(signature = (text, *, max_bytes=65536))]
    fn encode<'py>(
        &self,
        py: Python<'py>,
        text: &str,
        max_bytes: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let result = self.map.encode(text, max_bytes).map_err(error)?;
        Ok(PyBytes::new(py, &result))
    }
    fn encode_tokens<'py>(
        &self,
        py: Python<'py>,
        decoded: &TextResult,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let result = self
            .map
            .encode_tokens(&decoded.0.tokens, TextLimits::default())
            .map_err(error)?;
        Ok(PyBytes::new(py, &result))
    }
    fn dump(&self, py: Python<'_>, data: &[u8], table: &str) -> PyResult<Py<PyAny>> {
        let result = crate::toolkit()?
            .extract_text(data, &self.definition, table)
            .map_err(error)?;
        json(py, &result)
    }
    #[pyo3(signature = (data, font, *, columns=16))]
    fn font(&self, data: &[u8], font: &str, columns: u32) -> PyResult<Image> {
        crate::toolkit()?
            .extract_font(data, &self.definition, font, columns)
            .map(Image)
            .map_err(error)
    }
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<TextDefinition>()?;
    m.add_class::<TextResult>()?;
    Ok(())
}
