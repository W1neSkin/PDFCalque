//! Геометрия страницы. Единица — пункт PDF, он же пункт CSS: 1/72 дюйма.
//! Начало координат у PDF снизу слева. В структурах координаты остаются такими,
//! как в файле. Переворот в верхний левый угол делает уже запись в HTML.

/// Точка на странице.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Прямоугольник. `x` и `y` — левый нижний угол.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Цвет после приведения к RGBA. Компоненты от 0 до 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}
