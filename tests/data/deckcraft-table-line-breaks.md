# Native-generated PPTX line-break fixture

`deckcraft-table-line-breaks.pptx` is original synthetic content generated for this repository, not a third-party document. SHA256 `1e2f2506ca7eeec142f50731e14a5f3a6b0db9d2994094fcfb9f7918ba421fe5` (23,246 bytes). It contains no external font or asset; the one-pixel PNG is generated from three RGB bytes.

The public native model/edit/writer recipe below was executed against reviewed DeckCraft `c9c7fff189c808bf7266d0b888c3636a1880080d`. Its PPTX/model crates are byte-identical to parent `dd4fd70b6347dd2ce782e33f030a2b6c0abb0106`; only PDF text projection and native test/docs changed. Recipe SHA256 `36f4a8fc555336a5c1f788daeb09436ab7e0ec353f68ceacef9e78762135da2e`. ZIP timestamps may vary on regeneration; native model/readback and output behavior are the tested contract.

One ordinary table cell has a single paragraph with ALPHA, BETA and GAMMA separated by native structural breaks. The first and last labels retain URL/internal-slide links. A generated blue pixel is placed separately near the lower slide edge. The package regression imports this source and uses existing extraction/render/conversion APIs, independently of future owned-cell break authoring.

```rust
// Original synthetic spec128 fixture. Only public native model/edit/writer APIs.
use deckcraft_model::{ColorRef, Fill, Line, Presentation, Rgba, Shape, ShapeId, ShapeKind, Size, Table, Xfrm};
use deckcraft_model::style::PictureFill;
use deckcraft_model::text::{Action, Align, Bullet, Hyperlink, RunProps, Spacing};
use std::sync::Arc;

fn link(action: Action) -> Hyperlink {
    Hyperlink { action, tooltip: "Retained navigation".into(), highlight_click: false,
        sound: None, stop_sound: false, zoom_return: None }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut document: Presentation = deckcraft_model::defaults::new_presentation(None);
    document.slide_size = Size::new(576.0, 360.0);
    let layout = document.slides[0].layout;
    let slide = deckcraft_model::defaults::new_slide(&mut document, layout).ok_or("layout")?;
    document.slides.push(Arc::new(slide));
    for slide in &mut document.slides { Arc::make_mut(slide).shapes.clear(); }
    let destination = document.slides[1].id;
    let mut table = Table::new(1, 1, 500.0, 200.0);
    table.style = "none".into();
    let cell = table.cell_mut(0, 0).unwrap();
    cell.margins = Some([0.0; 4]);
    let paragraph = &mut cell.text.paragraphs[0];
    paragraph.props.bullet = Some(Bullet::None);
    paragraph.props.align = Some(Align::Left);
    paragraph.props.margin_left = Some(0.0);
    paragraph.props.margin_right = Some(0.0);
    paragraph.props.indent = Some(0.0);
    paragraph.props.line_spacing = Some(Spacing::Pct(1.0));
    for (i, label) in ["ALPHA", "BETA", "GAMMA"].iter().enumerate() {
        let rgb = [(224,0,0),(0,160,0),(0,0,224)][i];
        let props = RunProps { size: Some(20.0), bold: Some(true), underline: Some("none".into()),
            fill: Some(Fill::solid(ColorRef::rgb(Rgba::rgb(rgb.0,rgb.1,rgb.2)))),
            link: match i {0=>Some(link(Action::Url {url:"https://example.com/line-break".into()})),
                2=>Some(link(Action::Slide {slide:destination})), _=>None}, ..Default::default() };
        if i>0 {
            let start = cell.text.paragraphs[0].char_len();
            let mut marker = props.clone(); marker.link = None;
            deckcraft_model::edit::insert(&mut cell.text,(0,start),"\u{b}",Some(marker));
        }
        let start = cell.text.paragraphs[0].char_len();
        deckcraft_model::edit::insert(&mut cell.text,(0,start),label,Some(props));
    }
    let table_shape = Shape { id:ShapeId(128), kind:ShapeKind::Table(table),
        xfrm:Some(Xfrm::new(20.0,20.0,500.0,200.0)), ..Default::default() };
    let mut png = Vec::new();
    { let mut encoder=png::Encoder::new(&mut png,1,1); encoder.set_color(png::ColorType::Rgb);
      encoder.write_header()?.write_image_data(&[32,96,192])?; }
    let media = document.add_media("original-pixel.png","image/png",png).ok_or("media")?;
    let picture = Shape { id:ShapeId(129), kind:ShapeKind::Picture {fill:PictureFill {media,..Default::default()}},
        xfrm:Some(Xfrm::new(500.0,300.0,40.0,20.0)), line:Some(Line::none()),
        descr:"Original generated pixel".into(),..Default::default() };
    Arc::make_mut(&mut document.slides[0]).shapes=vec![table_shape,picture];
    assert!(document.validate().is_empty());
    let bytes=deckcraft_pptx::export(&document)?;
    let read=deckcraft_pptx::import(&bytes)?;
    assert_eq!(read.slides.len(),2);
    std::fs::write(std::env::args().nth(1).ok_or("controlled output path")?,bytes)?;
    Ok(())
}
```
