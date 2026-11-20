//! Модуль Python. Конвертер сначала разбирает PDF, потом пишет результат из этого разбора.

use std::path::Path;

use pyo3::prelude::*;

use crate::report_json;

fn py_err(message: impl ToString) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(message.to_string())
}

/// Уже разобранный PDF. Повторно файл не читается.
#[pyclass]
struct Book {
    doc: crate::Document,
}

#[pymethods]
impl Book {
    /// JSON: страницы, слова, картинки, таблицы.
    fn report(&self) -> String {
        report_json(&self.doc)
    }

    /// EPUB из уже разобранных страниц.
    fn write_epub(&self, path: &str) -> PyResult<()> {
        crate::write_epub(&self.doc, Path::new(path)).map_err(py_err)
    }
}

/// Разобрать PDF библиотекой и вернуть документ.
/// `progress(текущая, всего)` вызывается перед каждой страницей.
#[pyfunction]
#[pyo3(signature = (pdf_path, progress=None))]
fn parse(py: Python<'_>, pdf_path: &str, progress: Option<PyObject>) -> PyResult<Book> {
    let bytes = std::fs::read(pdf_path).map_err(py_err)?;
    let doc = py.allow_threads(|| {
        crate::parse::parse_pdf_reporting(&bytes, |current, total| {
            let Some(callback) = &progress else { return };
            Python::with_gil(|py| {
                let _ = callback.call1(py, (current, total));
            });
        })
    }).map_err(py_err)?;
    Ok(Book { doc })
}

#[pymodule]
fn pdfcalque(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse, m)?)?;
    m.add_class::<Book>()?;
    Ok(())
}
