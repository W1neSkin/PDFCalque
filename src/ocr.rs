//! Скан без букв в файле остаётся картинкой.
//! Поверх неё кладётся невидимый текст: каждое слово — одна надпись
//! на рамке чернил, чтобы его можно было выделить, не заменяя рисунок листа.

use std::path::Path;
use std::process::Command;

use crate::html::raw_png;
use crate::{Color, Element, Font, Glyph, Image, Point, Rect, TextBlock, TextLine, TextSource, TextSpan};

/// Если на листе нет текста, а есть большая картинка, распознаёт слова
/// и добавляет их прозрачным слоем. Картинка не удаляется.
pub fn maybe_scan(elements: &mut Vec<Element>, page_w: f32, page_h: f32) {
    if page_w < 1.0 || page_h < 1.0 || has_text(elements) {
        return;
    }
    let Some(image) = covering_image(elements, page_w, page_h).cloned() else { return };
    let Some(words) = recognize(&image) else { return };
    elements.extend(words);
}

fn has_text(elements: &[Element]) -> bool {
    elements.iter().any(|element| matches!(element, Element::Text(text) if text_chars(text) > 0))
}

fn text_chars(text: &TextBlock) -> usize {
    text.lines.iter().flat_map(|line| &line.spans).map(|span| span.text.chars().count()).sum()
}

/// Картинка на большую часть листа. Мелкая иллюстрация в тексте сюда не попадает.
fn covering_image<'a>(elements: &'a [Element], page_w: f32, page_h: f32) -> Option<&'a Image> {
    elements.iter().find_map(|element| {
        let Element::Image(image) = element else { return None };
        let wide = image.bounds.width > page_w * 0.5 && image.bounds.height > page_h * 0.5;
        (wide && image.pixel_width > 0 && image.pixel_height > 0).then_some(image)
    })
}

fn recognize(image: &Image) -> Option<Vec<Element>> {
    let (bytes, ext) = raster(image)?;
    let path = std::env::temp_dir().join(format!("pdfcalque-{}-{}.{}", std::process::id(), image.pixel_width, ext));
    std::fs::write(&path, bytes).ok()?;
    let lang = ocr_lang();
    // Сначала обычный разбор. Его рамки уже совпадают с выделением, их не двигаем.
    let primary = run_tsv(&path, &lang, "3", None);
    // Над линейкой таблицы обычный разбор иногда теряет слово. Редкий проход
    // его находит. Добавляем только то, что не пересекает уже найденные рамки.
    // Долгий проход обрываем: иначе на слабой машине конвертация не заканчивается.
    let sparse = run_tsv(&path, &lang, "11", Some(std::time::Duration::from_secs(25)));
    let _ = std::fs::remove_file(&path);
    let primary = primary?;
    let mut words = words_from_tsv(&primary, image, 45.0);
    if let Some(sparse) = sparse {
        absorb_missing(&mut words, words_from_tsv(&sparse, image, 40.0));
    }
    (!words.is_empty()).then_some(words)
}

fn run_tsv(path: &Path, lang: &str, psm: &str, limit: Option<std::time::Duration>) -> Option<String> {
    let mut child = Command::new(tesseract_program())
        .arg(path)
        .arg("stdout")
        .args(["--psm", psm, "-l", lang, "tsv"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (send, recv) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        let _ = send.send(out);
    });
    let started = std::time::Instant::now();
    loop {
        match recv.try_recv() {
            Ok(text) => {
                let status = child.wait().ok()?;
                return status.success().then_some(text);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if limit.is_some_and(|limit| started.elapsed() > limit) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}

fn raster(image: &Image) -> Option<(Vec<u8>, &'static str)> {
    // JPEG Tesseract читает как есть. Остальное собираем в PNG.
    if image.filter.contains("DCTDecode") {
        return Some((image.data.clone(), "jpg"));
    }
    let pixels = if image.filter.contains("FlateDecode") { inflate(&image.data)? } else { image.data.clone() };
    let png = raw_png(image.pixel_width, image.pixel_height, &image.color_space, &pixels)?;
    Some((png, "png"))
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut decoder, &mut out).ok()?;
    Some(out)
}

fn tesseract_program() -> String {
    if let Ok(path) = std::env::var("PDFCALQUE_TESSERACT") {
        if Path::new(&path).is_file() {
            return path;
        }
    }
    let windows = r"C:\Program Files\Tesseract-OCR\tesseract.exe";
    if Path::new(windows).is_file() {
        return windows.to_string();
    }
    "tesseract".to_string()
}

fn ocr_lang() -> String {
    std::env::var("PDFCALQUE_OCR_LANG").unwrap_or_else(|_| "eng".to_string())
}

struct Spot {
    text: String,
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

fn words_from_tsv(tsv: &str, image: &Image, min_conf: f32) -> Vec<Element> {
    let scale_x = image.bounds.width / image.pixel_width as f32;
    let scale_y = image.bounds.height / image.pixel_height as f32;
    let mut out = Vec::new();
    for line in tsv.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let conf: f32 = cols[10].parse().unwrap_or(-1.0);
        let text = cols[11].trim();
        // Уровень 5 — одно слово. Мусор с низкой уверенностью не кладём на лист.
        if conf < min_conf || text.is_empty() || !text.chars().any(|ch| ch.is_alphanumeric()) {
            continue;
        }
        let spot = Spot {
            text: text.to_string(),
            left: cols[6].parse().unwrap_or(0.0),
            top: cols[7].parse().unwrap_or(0.0),
            width: cols[8].parse().unwrap_or(0.0),
            height: cols[9].parse().unwrap_or(0.0),
        };
        if spot.width < 1.0 || spot.height < 1.0 {
            continue;
        }
        out.push(overlay_word(&spot, image, scale_x, scale_y));
    }
    out
}

/// Слова второго прохода. Рамку, которая уже есть, не заменяем и не сдвигаем.
fn absorb_missing(words: &mut Vec<Element>, extra: Vec<Element>) {
    for word in extra {
        if overlaps_any(&word, words) {
            continue;
        }
        let rank = reading_rank(&word);
        let index = words.iter().position(|found| reading_rank(found) > rank).unwrap_or(words.len());
        words.insert(index, word);
    }
}

fn overlaps_any(extra: &Element, words: &[Element]) -> bool {
    let Some(extra_box) = text_bounds(extra) else { return true };
    words.iter().any(|word| {
        text_bounds(word).map(|found| overlap_ratio(extra_box, found) > 0.3).unwrap_or(false)
    })
}

fn overlap_ratio(a: Rect, b: Rect) -> f32 {
    let ix = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let iy = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    let smaller = (a.width * a.height).min(b.width * b.height);
    if smaller <= 0.0 { 0.0 } else { ix * iy / smaller }
}

fn text_bounds(element: &Element) -> Option<Rect> {
    match element {
        Element::Text(text) => Some(text.bounds),
        _ => None,
    }
}

/// Сверху вниз, слева направо. 12 пунктов — одна строка, чтобы дрожание
/// базовой линии не переставляло уже найденные слова.
fn reading_rank(element: &Element) -> (i32, i32) {
    let Some(bounds) = text_bounds(element) else { return (i32::MAX, 0) };
    let top = bounds.y + bounds.height;
    (((-top) / 12.0).round() as i32, bounds.x.round() as i32)
}

/// Одна надпись на всю рамку слова. Буквы внутри не режутся,
/// иначе при выделении между ними появляются щели.
fn overlay_word(spot: &Spot, image: &Image, scale_x: f32, scale_y: f32) -> Element {
    let x = image.bounds.x + spot.left * scale_x;
    let width = spot.width * scale_x;
    let height = (spot.height * scale_y).max(1.0);
    let top = image.bounds.y + image.bounds.height - spot.top * scale_y;
    let bottom = top - height;
    let bounds = Rect { x, y: bottom, width, height };
    // Times ближе к печатному шрифту счёта, чем рубленый запасной.
    let font = Font { name: Some("Times New Roman".into()), size: height * 0.8, bold: false, italic: false };
    let span = TextSpan {
        bounds,
        text: spot.text.clone(),
        font,
        color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
        glyphs: vec![Glyph {
            bounds,
            origin: Point { x, y: bottom + height * 0.2 },
            rotation: 0.0,
            ch: None,
            glyph_id: None,
        }],
    };
    Element::Text(TextBlock {
        bounds,
        source: TextSource::Ocr,
        visible: true,
        lines: vec![TextLine { bounds, spans: vec![span] }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> Image {
        Image {
            bounds: Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 },
            pixel_width: 100,
            pixel_height: 100,
            filter: String::new(),
            color_space: String::new(),
            data: Vec::new(),
        }
    }

    fn tsv(rows: &[&str]) -> String {
        let mut out = String::from("level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n");
        out.push_str(&rows.join("\n"));
        out
    }

    #[test]
    fn missing_word_is_inserted_and_the_old_box_stays() {
        let image = sheet();
        let mut words = words_from_tsv(&tsv(&["5\t1\t1\t1\t1\t1\t10\t10\t20\t10\t90\tItem"]), &image, 45.0);
        let item = text_bounds(&words[0]).unwrap();
        let sparse = tsv(&[
            "5\t1\t1\t1\t1\t1\t12\t10\t20\t10\t90\tItem",
            "5\t1\t1\t1\t1\t2\t50\t10\t20\t10\t46\tTotal",
            "5\t1\t1\t1\t1\t3\t80\t10\t10\t10\t10\tjunk",
        ]);
        absorb_missing(&mut words, words_from_tsv(&sparse, &image, 40.0));
        assert_eq!(words.len(), 2);
        assert_eq!(text_bounds(&words[0]).unwrap(), item);
        let Element::Text(total) = &words[1] else { panic!("total") };
        assert_eq!(total.lines[0].spans[0].text, "Total");
    }
}
