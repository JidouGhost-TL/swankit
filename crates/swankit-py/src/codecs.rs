use crate::{bad, error};
use formatkit_core::Error;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use swankit::compress::{self, BitOrder};

#[pyclass(frozen, module = "swankit")]
pub struct DecodeResult(compress::DecodeOutcome);
#[pymethods]
impl DecodeResult {
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.bytes)
    }
    #[getter]
    fn consumed(&self) -> usize {
        self.0.consumed
    }
}
fn order(value: &str) -> PyResult<BitOrder> {
    match value {
        "lsb-first" => Ok(BitOrder::LsbFirst),
        "msb-first" => Ok(BitOrder::MsbFirst),
        _ => Err(bad("bit_order must be lsb-first or msb-first")),
    }
}
#[pyfunction]
#[pyo3(signature = (codec, data, *, max_output=65536, history=None, decoded_size=None, bit_order="lsb-first", fill=0))]
#[allow(clippy::too_many_arguments)]
fn decode(
    codec: &str,
    data: &[u8],
    max_output: usize,
    history: Option<&[u8]>,
    decoded_size: Option<usize>,
    bit_order: &str,
    fill: u8,
) -> PyResult<DecodeResult> {
    let history = history.unwrap_or_default();
    for (resource, requested) in [
        ("codec output", max_output),
        ("codec history", history.len()),
    ] {
        if requested > compress::MAX_DECODED {
            return Err(error(Error::ResourceLimit {
                resource,
                requested: requested as u64,
                limit: compress::MAX_DECODED as u64,
            }));
        }
    }
    let bit_order = order(bit_order)?;
    let decoded = match codec {
        "lz-rle-7f" => compress::lz_rle_7f::decode_with_history(data, history, max_output),
        "lz-bits-16" => compress::lz_bits_16::decode_with_history(data, history, max_output),
        "okumura-lzss" => {
            if !history.is_empty() {
                return Err(bad("Okumura uses ring fill, not external history"));
            }
            compress::okumura::decode(
                data,
                decoded_size.ok_or_else(|| bad("Okumura requires decoded_size"))?,
                bit_order,
                fill,
                max_output,
            )
        }
        _ => return Err(bad("unknown codec")),
    };
    decoded.map(DecodeResult).map_err(error)
}
#[pyfunction]
#[pyo3(signature = (codec, data, *, bit_order="lsb-first", fill=0))]
fn encode<'py>(
    py: Python<'py>,
    codec: &str,
    data: &[u8],
    bit_order: &str,
    fill: u8,
) -> PyResult<Bound<'py, PyBytes>> {
    let bit_order = order(bit_order)?;
    if data.len() > compress::MAX_DECODED {
        return Err(error(Error::ResourceLimit {
            resource: "codec input",
            requested: data.len() as u64,
            limit: compress::MAX_DECODED as u64,
        }));
    }
    let result = match codec {
        "lz-rle-7f" => compress::lz_rle_7f::encode(data).map_err(error)?,
        "lz-bits-16" => compress::lz_bits_16::encode(data).map_err(error)?,
        "okumura-lzss" => compress::okumura::encode(data, bit_order, fill),
        _ => return Err(bad("unknown codec")),
    };
    Ok(PyBytes::new(py, &result))
}
#[pyfunction]
#[pyo3(signature = (base, patch, *, allow_growth=false, allow_overlap=false, max_output=67108864))]
fn ips_apply<'py>(
    py: Python<'py>,
    base: &[u8],
    patch: &[u8],
    allow_growth: bool,
    allow_overlap: bool,
    max_output: usize,
) -> PyResult<Bound<'py, PyBytes>> {
    use swankit::patch::{ApplyOptions, GrowthPolicy, OverlapPolicy};
    let options = ApplyOptions {
        growth: if allow_growth {
            GrowthPolicy::ZeroFill
        } else {
            GrowthPolicy::Reject
        },
        overlaps: if allow_overlap {
            OverlapPolicy::LastWriteWins
        } else {
            OverlapPolicy::Reject
        },
        max_output_size: max_output,
    };
    let result = crate::toolkit()?
        .apply_patch(patch, base, options)
        .map_err(error)?;
    Ok(PyBytes::new(py, &result))
}
#[pyfunction]
fn ips_create<'py>(py: Python<'py>, base: &[u8], target: &[u8]) -> PyResult<Bound<'py, PyBytes>> {
    let result = crate::toolkit()?
        .create_patch(base, target)
        .map_err(error)?;
    Ok(PyBytes::new(py, &result))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<DecodeResult>()?;
    m.add_function(wrap_pyfunction!(decode, m)?)?;
    m.add_function(wrap_pyfunction!(encode, m)?)?;
    m.add_function(wrap_pyfunction!(ips_apply, m)?)?;
    m.add_function(wrap_pyfunction!(ips_create, m)?)?;
    Ok(())
}
