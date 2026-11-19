//! Модуль Python. Конвертер делает `import pdfcalque`, без отдельной программы.

use std::path::Path;

use pyo3::prelude::*;

use crate::report_json;

fn py_err(message: impl ToString) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(message.to_string())
}

fn load(path: &str) -> PyResult<crate::Document> {
    let bytes = std::fs::read(path).map_err(py_err)?;
    crate::parse_pdf(&bytes).map_err(py_err)
}

/// Разобрать PDF и вернуть JSON-отчёт. EPUB не пишется.
#[pyfunction]
fn report(pdf_path: &str) -> PyResult<String> {
    Ok(report_json(&load(pdf_path)?))
}

/// Записать EPUB и вернуть тот же JSON-отчёт.
#[pyfunction]
fn export_epub(pdf_path: &str, epub_path: &str) -> PyResult<String> {
    let doc = load(pdf_path)?;
    crate::write_epub(&doc, Path::new(epub_path)).map_err(py_err)?;
    Ok(report_json(&doc))
}

#[pymodule]
fn pdfcalque(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(report, m)?)?;
    m.add_function(wrap_pyfunction!(export_epub, m)?)?;
    Ok(())
}
