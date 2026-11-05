use super::*;

const SRC: &str = "# Demo\n\n## Why? {#intro}\n\n- one\n- two\n\n![Chart](chart.png){#fig1}\n\n## Math {#math layout=titleAndContent}\n\n$$ e^(i pi) + 1 = 0 $$ {#eq1 x=520 y=180 w=360}\n\n::: {#note x=60 y=440 w=840 h=50}\nFree text.\n:::\n";

#[test]
fn parses_slides_and_elements() {
    let d = parse(SRC);
    assert_eq!(d.slides.len(), 2);
    assert_eq!(d.slides[0].id, "intro");
    assert_eq!(d.slides[0].bullets, ["one", "two"]);
    assert_eq!(d.slides[0].elems[0].kind, Kind::Image { src: "chart.png".into(), alt: "Chart".into() });
    assert!(d.slides[0].elems[0].pinned().is_none());
    assert_eq!(d.slides[1].elems[0].pinned(), Some([Some(520.0), Some(180.0), Some(360.0), None]));
    assert_eq!(d.slides[1].elems[1].kind, Kind::Box("Free text.".into()));
    assert_eq!(slug("Why round-trip slides?"), "why-round-trip-slides");
}

fn shape(id: u32, tags: &[(&str, &str)], r: Rect) -> Shape {
    Shape {
        id: deckcraft_model::ShapeId(id),
        xfrm: Some(deckcraft_model::Xfrm { x: r[0], y: r[1], w: r[2], h: r[3], ..Default::default() }),
        tags: tags.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        ..Default::default()
    }
}

fn pres(shapes: Vec<Shape>) -> Presentation {
    let mut p = Presentation::default();
    p.slides.push(std::sync::Arc::new(deckcraft_model::Slide { shapes, ..Default::default() }));
    p
}

#[test]
fn unmoved_shapes_write_nothing() {
    let p = pres(vec![
        shape(1, &[(TAG_ID, "fig1"), (TAG_RECT, "500,160,400,250")], [500.0, 160.0, 400.0, 250.0]),
        shape(2, &[(TAG_ID, "eq1"), (TAG_RECT, "520,180,360,84.2")], [520.0, 180.0, 360.0, 84.2]),
        shape(3, &[(TAG_ID, "note"), (TAG_RECT, "60,440,840,50")], [60.0, 440.0, 840.0, 28.8]),
    ]);
    assert_eq!(write_back(SRC, &p), None);
}

#[test]
fn moved_shapes_patch_only_their_blocks() {
    let p = pres(vec![
        shape(1, &[(TAG_ID, "fig1"), (TAG_RECT, "500,160,400,250")], [540.0, 200.0, 360.0, 225.0]),
        shape(4, &[(TAG_ID, "fig1")], [0.0, 0.0, 10.0, 10.0]), // GUI duplicate: ignored
        shape(5, &[(TAG_ID, "intro.body")], [48.0, 140.0, 460.0, 330.0]),
    ]);
    let (out, ch) = write_back(SRC, &p).unwrap_or_default();
    assert_eq!(ch.len(), 2);
    assert!(out.contains("![Chart](chart.png){#fig1 x=540 y=200 w=360 h=225}"));
    assert!(out.contains("## Why? {#intro body=48,140,460,330}"));
    assert!(out.contains("{#eq1 x=520 y=180 w=360}"), "untouched lines stay byte-identical");
    // and the patched source parses back to the same geometry, so a second pass is a no-op
    let d = parse(&out);
    assert_eq!(d.slides[0].placeholder_rect("body"), Some([48.0, 140.0, 460.0, 330.0]));
}

#[test]
fn text_box_height_is_not_synced() {
    let p = pres(vec![shape(3, &[(TAG_ID, "note")], [60.0, 440.0, 840.0, 28.8])]);
    assert_eq!(write_back(SRC, &p), None);
}

#[test]
fn plan_separates_moves_from_rebuilds() {
    let old = parse(SRC);
    let moved = parse(&SRC.replace("x=520", "x=300"));
    let pl = plan(&old, &moved);
    assert!(pl.rebuild.is_empty());
    assert_eq!(pl.moves.len(), 1);
    assert_eq!(pl.moves[0].0, "eq1");
    let edited = parse(&SRC.replace("- two", "- two\n- three"));
    assert_eq!(plan(&old, &edited).rebuild, [0]);
    let shorter = parse(&SRC[..SRC.find("## Math").unwrap_or(SRC.len())]);
    assert_eq!(plan(&old, &shorter).drop_tail, 1);
    assert_eq!(plan(&old, &old), Plan::default());
}

#[test]
fn builds_tagged_commands() {
    let d = parse(SRC);
    let cmds = slide_commands(&d.slides[1], 1, Path::new("."), &|_| None);
    assert_eq!(cmds[0].0, "slide.new");
    assert_eq!(cmds[0].1["at"], 1);
    let tags: Vec<_> = cmds.iter().filter(|c| c.0 == "shape.tags").map(|c| c.1["set"][TAG_ID].as_str().unwrap_or("")).collect();
    assert_eq!(tags, ["eq1", "note"]);
}

#[test]
fn reads_png_size() {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend(800u32.to_be_bytes());
    png.extend(500u32.to_be_bytes());
    png.extend([0; 8]);
    assert_eq!(image_size(&png), Some((800.0, 500.0)));
}
