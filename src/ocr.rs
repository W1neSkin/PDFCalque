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
    // TSV, уровень 5: одно слово и его рамка. Рамки отдельных букв
    // не совпадают с штрихом, поэтому их не запрашиваем.
    let output = Command::new(tesseract_program())
        .arg(&path)
        .arg("stdout")
        .args(["--psm", "3", "-l"])
        .arg(ocr_lang())
        .arg("tsv")
        .output();
    let result = match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut words = words_from_tsv(&text, image);
            // Линейка под заголовком иногда прячет одно слово. Ищем чернила в пустом
            // месте уже найденной строки и распознаём только этот кусок. Рамки,
            // которые первый проход уже поставил, не двигаем.
            fill_line_gaps(&path, image, &mut words);
            (!words.is_empty()).then_some(words)
        }
        _ => None,
    };
    let _ = std::fs::remove_file(&path);
    result
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

fn words_from_tsv(tsv: &str, image: &Image) -> Vec<Element> {
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
        if conf < 45.0 || text.is_empty() || !text.chars().any(|ch| ch.is_alphanumeric()) {
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

/// Чернила в строке, которые не попали ни в одну уже найденную рамку.
fn line_gaps(width: usize, height: usize, pixels: &[u8], words: &[(i32, i32, i32, i32)]) -> Vec<(i32, i32, i32, i32)> {
    let mut lines: Vec<Vec<(i32, i32, i32, i32)>> = Vec::new();
    let mut ordered = words.to_vec();
    ordered.sort_by_key(|word| word.1);
    for word in ordered {
        if let Some(line) = lines.iter_mut().find(|line| line.iter().any(|other| same_line(*other, word))) {
            line.push(word);
        } else {
            lines.push(vec![word]);
        }
    }
    let mut gaps = Vec::new();
    for line in lines {
        let y0 = line.iter().map(|word| word.1).min().unwrap_or(0).max(0) as usize;
        let y1 = line.iter().map(|word| word.1 + word.3).max().unwrap_or(0).min(height as i32) as usize;
        if y1 <= y0 || y0 >= height {
            continue;
        }
        let mut raw = Vec::new();
        let mut start: Option<usize> = None;
        for x in 0..=width {
            let ink = x < width && column_has_ink(pixels, width, x, y0, y1);
            match (start, ink) {
                (None, true) => start = Some(x),
                (Some(from), false) => {
                    raw.push((from as i32, x as i32));
                    start = None;
                }
                _ => {}
            }
        }
        for (left, right) in merge_runs(raw) {
            let run_w = right - left;
            if run_w < 12 || covered(left, right, &line) {
                continue;
            }
            let (top, bottom) = ink_rows(pixels, width, left, right, y0, y1);
            let run_h = bottom - top;
            if run_h < 8 || run_w > run_h * 6 {
                continue;
            }
            gaps.push((left, top, run_w, run_h));
        }
    }
    gaps.sort_by_key(|gap| -(gap.2 * gap.3));
    gaps.truncate(4);
    gaps
}

fn same_line(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let overlap = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    overlap > 0 && overlap * 2 >= a.3.min(b.3)
}

fn column_has_ink(pixels: &[u8], width: usize, x: usize, y0: usize, y1: usize) -> bool {
    let mut dark = 0;
    for y in y0..y1 {
        if pixels.get(y * width + x).copied().unwrap_or(255) < 100 {
            dark += 1;
            if dark >= 2 {
                return true;
            }
        }
    }
    false
}

fn merge_runs(raw: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for (start, end) in raw {
        if let Some((_, prev_end)) = out.last_mut() {
            if start - *prev_end <= 3 {
                *prev_end = end;
                continue;
            }
        }
        out.push((start, end));
    }
    out
}

fn covered(start: i32, end: i32, words: &[(i32, i32, i32, i32)]) -> bool {
    let width = (end - start).max(1);
    words.iter().any(|word| {
        let overlap = end.min(word.0 + word.2) - start.max(word.0);
        overlap * 2 >= width
    })
}

fn ink_rows(pixels: &[u8], width: usize, left: i32, right: i32, y0: usize, y1: usize) -> (i32, i32) {
    let mut top = y1;
    let mut bottom = y0;
    for x in left.max(0)..right {
        for y in y0..y1 {
            if pixels.get(y * width + x as usize).copied().unwrap_or(255) < 100 {
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    if bottom <= top {
        return (y0 as i32, y1 as i32);
    }
    (top as i32, bottom as i32)
}

fn fill_line_gaps(path: &Path, image: &Image, words: &mut Vec<Element>) {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jpg") {
        return;
    }
    let Some((width, height, pixels)) = decode_gray(path) else { return };
    let boxes: Vec<(i32, i32, i32, i32)> = words.iter().filter_map(|word| pixel_box(word, image)).collect();
    let scale_x = image.bounds.width / image.pixel_width as f32;
    let scale_y = image.bounds.height / image.pixel_height as f32;
    for (index, gap) in line_gaps(width, height, &pixels, &boxes).into_iter().enumerate() {
        let Some(spot) = read_gap(&pixels, width, height, gap, index) else { continue };
        let word = overlay_word(&spot, image, scale_x, scale_y);
        if overlaps_any(&word, words) {
            continue;
        }
        let rank = reading_rank(&word);
        let at = words.iter().position(|found| reading_rank(found) > rank).unwrap_or(words.len());
        words.insert(at, word);
    }
}

fn decode_gray(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).ok()?;
    let mut decoder = jpeg_decoder::Decoder::new(file);
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let width = info.width as usize;
    let height = info.height as usize;
    let gray = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => pixels,
        jpeg_decoder::PixelFormat::RGB24 => {
            pixels.chunks(3).map(|pixel| {
                let red = pixel.first().copied().unwrap_or(255) as u16;
                let green = pixel.get(1).copied().unwrap_or(red as u8) as u16;
                let blue = pixel.get(2).copied().unwrap_or(red as u8) as u16;
                ((red * 3 + green * 6 + blue) / 10) as u8
            }).collect()
        }
        _ => return None,
    };
    (gray.len() >= width * height).then_some((width, height, gray))
}

fn pixel_box(element: &Element, image: &Image) -> Option<(i32, i32, i32, i32)> {
    let Element::Text(text) = element else { return None };
    let scale_x = image.bounds.width / image.pixel_width as f32;
    let scale_y = image.bounds.height / image.pixel_height as f32;
    if scale_x <= 0.0 || scale_y <= 0.0 {
        return None;
    }
    let left = ((text.bounds.x - image.bounds.x) / scale_x).round() as i32;
    let top = ((image.bounds.y + image.bounds.height - (text.bounds.y + text.bounds.height)) / scale_y).round() as i32;
    let width = (text.bounds.width / scale_x).round().max(1.0) as i32;
    let height = (text.bounds.height / scale_y).round().max(1.0) as i32;
    Some((left, top, width, height))
}

fn read_gap(pixels: &[u8], width: usize, height: usize, gap: (i32, i32, i32, i32), index: usize) -> Option<Spot> {
    let (left, top, gap_w, gap_h) = gap;
    let x0 = (left - 4).max(0);
    let y0 = (top - 4).max(0);
    let x1 = (left + gap_w + 4).min(width as i32);
    let y1 = (top + gap_h + 4).min(height as i32);
    let crop_w = (x1 - x0) as usize;
    let crop_h = (y1 - y0) as usize;
    if crop_w < 8 || crop_h < 8 {
        return None;
    }
    let mut body = Vec::with_capacity(crop_w * crop_h);
    for row in y0..y1 {
        let start = row as usize * width + x0 as usize;
        body.extend_from_slice(&pixels[start..start + crop_w]);
    }
    let path = std::env::temp_dir().join(format!("pdfcalque-gap-{}-{index}.pgm", std::process::id()));
    let mut file = format!("P5\n{crop_w} {crop_h}\n255\n").into_bytes();
    file.extend(body);
    std::fs::write(&path, file).ok()?;
    let output = tesseract_tsv(&path, "7");
    let _ = std::fs::remove_file(&path);
    let output = output?;
    let text = String::from_utf8_lossy(&output);
    best_spot(&text, x0, y0)
}

fn tesseract_tsv(path: &Path, psm: &str) -> Option<Vec<u8>> {
    let mut child = Command::new(tesseract_program())
        .arg(path)
        .arg("stdout")
        .args(["--psm", psm, "-l"])
        .arg(ocr_lang())
        .arg("tsv")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (send, recv) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut out);
        let _ = send.send(out);
    });
    let started = std::time::Instant::now();
    let limit = std::time::Duration::from_secs(8);
    loop {
        match recv.try_recv() {
            Ok(out) => {
                let status = child.wait().ok()?;
                return status.success().then_some(out);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if started.elapsed() > limit {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
        }
    }
}

fn best_spot(tsv: &str, origin_x: i32, origin_y: i32) -> Option<Spot> {
    let mut best: Option<(f32, Spot)> = None;
    for line in tsv.lines().skip(1) {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue;
        }
        let conf: f32 = cols[10].parse().unwrap_or(-1.0);
        let text = cols[11].trim();
        if conf < 45.0 || text.is_empty() || !text.chars().any(|ch| ch.is_alphanumeric()) {
            continue;
        }
        let width: f32 = cols[8].parse().unwrap_or(0.0);
        let height: f32 = cols[9].parse().unwrap_or(0.0);
        if width < 1.0 || height < 1.0 {
            continue;
        }
        let left: f32 = cols[6].parse().unwrap_or(0.0);
        let top: f32 = cols[7].parse().unwrap_or(0.0);
        let spot = Spot {
            text: text.to_string(),
            left: origin_x as f32 + left,
            top: origin_y as f32 + top,
            width,
            height,
        };
        if best.as_ref().map(|(score, _)| conf > *score).unwrap_or(true) {
            best = Some((conf, spot));
        }
    }
    best.map(|(_, spot)| spot)
}

fn overlaps_any(extra: &Element, words: &[Element]) -> bool {
    let Some(extra_box) = text_bounds(extra) else { return true };
    words.iter().any(|word| text_bounds(word).map(|found| overlap_ratio(extra_box, found) > 0.3).unwrap_or(false))
}

fn overlap_ratio(a: Rect, b: Rect) -> f32 {
    let overlap_x = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let overlap_y = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    if overlap_x <= 0.0 || overlap_y <= 0.0 {
        return 0.0;
    }
    let smaller = (a.width * a.height).min(b.width * b.height);
    if smaller <= 0.0 { 0.0 } else { overlap_x * overlap_y / smaller }
}

fn text_bounds(element: &Element) -> Option<Rect> {
    match element {
        Element::Text(text) => Some(text.bounds),
        _ => None,
    }
}

fn reading_rank(element: &Element) -> (i32, i32) {
    let Some(bounds) = text_bounds(element) else { return (i32::MAX, 0) };
    let top = bounds.y + bounds.height;
    ((-top / 12.0).round() as i32, bounds.x.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::line_gaps;

    fn paint(pixels: &mut [u8], width: usize, x: usize, y: usize, w: usize, h: usize) {
        for row in y..y + h {
            for col in x..x + w {
                pixels[row * width + col] = 0;
            }
        }
    }

    #[test]
    fn uncovered_ink_on_a_line_is_a_gap() {
        let width = 200;
        let height = 40;
        let mut pixels = vec![255u8; width * height];
        paint(&mut pixels, width, 10, 8, 30, 16);
        paint(&mut pixels, width, 90, 8, 40, 16);
        let gaps = line_gaps(width, height, &pixels, &[(10, 8, 30, 16)]);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].0 >= 85 && gaps[0].0 <= 95, "{:?}", gaps[0]);
        assert!(gaps[0].2 >= 35, "{:?}", gaps[0]);
    }

    #[test]
    fn header_gap_reads_total_when_the_sample_is_present() {
        let path = std::path::Path::new(r"E:\temp\scan-page1.jpg");
        if !path.is_file() {
            return;
        }
        let Some((width, height, pixels)) = super::decode_gray(path) else { return };
        let words = [(223, 608, 52, 24), (466, 605, 53, 30), (536, 605, 55, 30)];
        let gaps = line_gaps(width, height, &pixels, &words);
        let gap = gaps.iter().copied().max_by_key(|item| item.0);
        let Some(gap) = gap else { panic!("no gap"); };
        let spot = super::read_gap(&pixels, width, height, gap, 0).expect("ocr");
        assert_eq!(spot.text, "Total", "gap={gap:?}");
    }
}
