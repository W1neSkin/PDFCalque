# PDFCalque

PDFCalque reads a PDF into its own document model, then writes that model to other formats. The writers do not parse the PDF again. They only read pages, text, images, paths, tables, fonts, and the bookmark tree.

A normal PDF is taken from the file: operators, fonts, and coordinates. A scan stays the picture already stored in the PDF. Letters are not recognized from that picture.

## What it writes

- HTML. One SVG per page, and one HTML file that stacks those pages with a break after each page.
- EPUB 3 fixed layout. Each page stays a page. The bookmark tree becomes the table of contents, with a link to the place on the page. Embedded Type 1 fonts are converted to OpenType and stored in `OEBPS/fonts`.
- Tables as CSV and as an `.xlsx` workbook.

Text that is already in the PDF is positioned glyph by glyph. A ruled table is built from the drawn grid. A table without rules is built when short words line up in columns. Two columns of book text stay ordinary paragraphs.

## What it does not do

- It does not OCR a scan.
- It does not paint shadings or blend modes. A page that is mostly vector art can look plainer than a fresh render of the PDF.
- It does not rebuild merged table cells. A header that crosses two columns is left out rather than stored as one wrong cell.

## Library

```rust
let bytes = std::fs::read("book.pdf")?;
let doc = pdfcalque::parse_pdf(&bytes)?;

pdfcalque::write_html(&doc, std::path::Path::new("pages"))?;
pdfcalque::write_epub(&doc, std::path::Path::new("book.epub"))?;

for page in &doc.pages {
    for element in &page.elements {
        if let pdfcalque::Element::Table(table) = element {
            let csv = pdfcalque::table_to_csv(table);
            let xlsx = pdfcalque::table_to_xlsx(table)?;
            let _ = (csv, xlsx);
        }
    }
}
```

`parse_pdf_page` reads one page. Page numbers start at 1.

The model keeps PDF coordinates: the origin is the bottom left of the page. HTML and EPUB flip that to the top left when they draw.

## Command line

```bash
cargo build --release --bin pdfcalque
```

Write an EPUB and a JSON report:

```bash
pdfcalque export input.pdf output.epub output.json
```

Write only the report, including any tables:

```bash
pdfcalque tables input.pdf output.json
```

The report has the page count, text size, language, image and path counts, and table rows. Another program can compare those rows with its own extraction and keep the better cells.

## Build and test

Rust 1.80 or newer is enough.

```bash
cargo test
```

The tests use small PDFs in `tests/fixtures`. They check text positions, a ruled grid, a borderless word table, a fax image, and a fixed-layout EPUB.

## License

MIT
