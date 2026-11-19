//! Команды для проверки вручную.
//!
//! `pdfcalque export input.pdf output.epub output.json`
//! `pdfcalque tables input.pdf output.json`
//! Конвертер этот файл не запускает: он импортирует библиотеку как модуль Python.

use std::fs;
use std::path::Path;

use pdfcalque::write_epub;

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    let pdf = match args.next() {
        Some(path) => path,
        None => usage(),
    };
    let bytes = match fs::read(&pdf) {
        Ok(bytes) => bytes,
        Err(err) => fail(err.to_string()),
    };
    let doc = match pdfcalque::parse_pdf(&bytes) {
        Ok(doc) => doc,
        Err(err) => fail(err),
    };
    let json = match command.as_str() {
        "export" => {
            let epub = match args.next() {
                Some(path) => path,
                None => usage(),
            };
            if let Err(err) = write_epub(&doc, Path::new(&epub)) {
                fail(err);
            }
            match args.next() {
                Some(path) => path,
                None => usage(),
            }
        }
        "tables" => match args.next() {
            Some(path) => path,
            None => usage(),
        },
        _ => usage(),
    };
    if let Some(dir) = Path::new(&json).parent() {
        fs::create_dir_all(dir).ok();
    }
    if let Err(err) = fs::write(&json, pdfcalque::report_json(&doc)) {
        fail(err.to_string());
    }
}

fn usage() -> ! {
    eprintln!("usage: pdfcalque export input.pdf output.epub output.json");
    eprintln!("       pdfcalque tables input.pdf output.json");
    std::process::exit(2);
}

fn fail(message: String) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
