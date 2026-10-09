use crate::{error, json};
use formatkit_core::Pcm as RustPcm;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use swankit::sound::{
    self,
    render::{Action, Event},
};

#[pyclass(frozen, module = "swankit")]
struct Pcm(RustPcm);
#[pymethods]
impl Pcm {
    #[getter]
    fn sample_rate(&self) -> u32 {
        self.0.sample_rate()
    }
    #[getter]
    fn channels(&self) -> u16 {
        self.0.channels()
    }
    #[getter]
    fn frame_count(&self) -> usize {
        self.0.frame_count()
    }
    #[getter]
    fn samples(&self) -> Vec<i16> {
        self.0.samples().to_vec()
    }
    /// Signed 16-bit little-endian interleaved samples, without a WAV header.
    fn s16le<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        let bytes: Vec<u8> = self
            .0
            .samples()
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .collect();
        PyBytes::new(py, &bytes)
    }
}
#[pyclass(frozen, module = "swankit")]
struct Wave(sound::Wave);
#[pymethods]
impl Wave {
    #[new]
    fn new(data: &[u8]) -> PyResult<Self> {
        sound::Wave::decode(data).map(Self).map_err(error)
    }
    #[getter]
    fn samples<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.0.samples())
    }
    #[pyo3(signature = (sample_rate=24000))]
    fn to_pcm(&self, sample_rate: u32) -> PyResult<Pcm> {
        self.0.to_pcm(sample_rate).map(Pcm).map_err(error)
    }
}
#[pyfunction]
fn waves(data: &[u8]) -> PyResult<Vec<Wave>> {
    if data.is_empty() || !data.len().is_multiple_of(sound::WAVE_BYTES) {
        return Err(crate::bad(
            "wave extent must contain complete 16-byte waves",
        ));
    }
    sound::instrument::wave_bank(data, data.len() / sound::WAVE_BYTES)
        .map(|bank| bank.into_iter().map(Wave).collect())
        .map_err(error)
}
#[pyfunction]
fn unsigned_pcm(data: &[u8], sample_rate: u32) -> PyResult<Pcm> {
    sound::render::unsigned_pcm(data, sample_rate, sound::render::MAX_FRAMES)
        .map(Pcm)
        .map_err(error)
}
/// Bounded stereo preview from a four-wave bank and ordered register writes.
#[pyfunction]
#[pyo3(signature = (wave_bank, writes, frames, *, coupled=false))]
fn pcm_preview(
    wave_bank: &[u8],
    writes: &Bound<'_, PyAny>,
    frames: usize,
    coupled: bool,
) -> PyResult<Pcm> {
    if writes.len()? >= sound::render::MAX_EVENTS {
        return Err(crate::bad("sound event limit exceeded"));
    }
    let writes: Vec<(usize, u8, u8)> = writes.extract()?;
    let mut events = Vec::with_capacity(writes.len() + 1);
    events.push(Event {
        frame: 0,
        action: Action::Waves(sound::WaveBank::decode(wave_bank).map_err(error)?),
    });
    events.extend(writes.into_iter().map(|(frame, port, value)| Event {
        frame,
        action: Action::Write { port, value },
    }));
    let render = if coupled {
        sound::render::render_coupled
    } else {
        sound::render::render
    };
    render(&events, frames).map(Pcm).map_err(error)
}
#[pyfunction]
fn sound_extract(py: Python<'_>, data: &[u8], definition_json: &str) -> PyResult<Py<PyAny>> {
    let definition = swankit::extract::definition(definition_json.as_bytes()).map_err(error)?;
    let result = crate::toolkit()?
        .extract_sound(data, &definition)
        .map_err(error)?;
    json(py, &result)
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Wave>()?;
    m.add_class::<Pcm>()?;
    m.add_function(wrap_pyfunction!(waves, m)?)?;
    m.add_function(wrap_pyfunction!(unsigned_pcm, m)?)?;
    m.add_function(wrap_pyfunction!(pcm_preview, m)?)?;
    m.add_function(wrap_pyfunction!(sound_extract, m)?)?;
    Ok(())
}
