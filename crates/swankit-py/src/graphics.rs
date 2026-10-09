use crate::{bad, error};
use formatkit_core::Image as RustImage;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use swankit::gfx::{self, Arrangement, AtlasLayout, Order, TileFormat};

#[pyclass(frozen, module = "swankit")]
pub struct Image(pub RustImage);
#[pymethods]
impl Image {
    #[getter]
    fn width(&self) -> u32 {
        self.0.width
    }
    #[getter]
    fn height(&self) -> u32 {
        self.0.height
    }
    #[getter]
    fn rgba<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.rgba)
    }
}
fn tile_format(value: &str) -> PyResult<TileFormat> {
    match value {
        "2bpp" => Ok(TileFormat::Bpp2),
        "4bpp-planar" => Ok(TileFormat::Bpp4Planar),
        "4bpp-packed" => Ok(TileFormat::Bpp4Packed),
        _ => Err(bad("unknown tile format")),
    }
}
#[pyfunction]
#[pyo3(signature = (data, format="2bpp", *, palette=None, columns=16, scale=1, tiles_wide=1, tiles_high=1, column_major=false, max_bytes=67108864))]
#[allow(clippy::too_many_arguments)]
fn tiles(
    data: &[u8],
    format: &str,
    palette: Option<&[u8]>,
    columns: u32,
    scale: u32,
    tiles_wide: u32,
    tiles_high: u32,
    column_major: bool,
    max_bytes: usize,
) -> PyResult<Image> {
    let format = tile_format(format)?;
    let colors = match palette {
        Some(bytes) => {
            if bytes.len() != format.colors() * 4 {
                return Err(bad("RGBA palette has wrong color count"));
            }
            bytes
                .chunks_exact(4)
                .map(|c| [c[0], c[1], c[2], c[3]])
                .collect()
        }
        None => swankit::extract::grayscale(format.colors()).map_err(error)?,
    };
    crate::toolkit()?
        .tiles(
            data,
            format,
            &colors,
            AtlasLayout {
                arrangement: Arrangement {
                    tiles_wide,
                    tiles_high,
                    order: if column_major {
                        Order::ColumnMajor
                    } else {
                        Order::RowMajor
                    },
                },
                columns,
                scale,
                max_bytes,
            },
        )
        .map(Image)
        .map_err(error)
}
#[pyfunction]
fn grayscale<'py>(py: Python<'py>, colors: usize) -> PyResult<Bound<'py, PyBytes>> {
    let colors = swankit::extract::grayscale(colors).map_err(error)?;
    Ok(PyBytes::new(py, colors.as_flattened()))
}
#[pyfunction]
#[pyo3(signature = (data, index=0))]
fn color_palette<'py>(py: Python<'py>, data: &[u8], index: u8) -> PyResult<Bound<'py, PyBytes>> {
    let palettes = gfx::ColorPalettes::parse(data).map_err(error)?;
    Ok(PyBytes::new(
        py,
        palettes.palette(index).map_err(error)?.as_flattened(),
    ))
}
#[pyfunction]
#[pyo3(signature = (ports, index=0))]
fn mono_palette<'py>(py: Python<'py>, ports: &[u8], index: u8) -> PyResult<Bound<'py, PyBytes>> {
    let palettes = gfx::MonoPalettes::from_ports(ports).map_err(error)?;
    Ok(PyBytes::new(
        py,
        palettes.palette(index).map_err(error)?.as_flattened(),
    ))
}
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Image>()?;
    m.add_function(wrap_pyfunction!(tiles, m)?)?;
    m.add_function(wrap_pyfunction!(grayscale, m)?)?;
    m.add_function(wrap_pyfunction!(color_palette, m)?)?;
    m.add_function(wrap_pyfunction!(mono_palette, m)?)?;
    Ok(())
}
