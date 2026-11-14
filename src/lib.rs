//! PDFCalque разбирает PDF в собственные структуры.
//!
//! По этим структурам потом собирается документ в нужном формате. Первым будет HTML.
//!
//! Обычный PDF разбирается по файлу: объекты, команды страницы, шрифты, координаты.
//! OCR используется только если страница — скан, то есть картинка без текста в файле.

mod geom;
mod model;

pub use geom::{Color, Point, Rect};
pub use model::{
    Cell, Column, Document, Element, Font, Glyph, Image, Page, PathSegment, PathShape, Row,
    Table, TextBlock, TextLine, TextSource, TextSpan,
};
