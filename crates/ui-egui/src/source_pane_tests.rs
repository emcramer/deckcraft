#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::services::Services;
use deckcraft_engine::Session;

const SRC: &str = "# Demo\n\n## Why round-trip? {#intro}\n\n- Write the deck as markdown\n- Nudge things in the editor\n\n::: {#note x=60 y=440 w=840}\nFree text box.\n:::\n\n## Math {#math}\n\n- Equations come from Typst\n\n$$ e^(i pi) + 1 = 0 $$ {#eq1 x=520 y=180 w=360 h=80}\n";

fn harness(dir: &Path) -> egui_kittest::Harness<'static, SlideApp> {
    let md = dir.join("deck.md");
    std::fs::write(&md, SRC).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1500.0, 860.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SlideApp::new(Session::with_new(), Services::default()),
    );
    h.run_steps(2);
    h.state_mut().run("view.source", json!({"path": md.to_string_lossy()})).unwrap();
    h.run_steps(2);
    h
}

fn tagged(app: &SlideApp, id: &str) -> deckcraft_model::Shape {
    let (slide, sid) = find_tagged(app, id).unwrap_or_else(|| panic!("no shape tagged {id}"));
    let d = app.session.active().unwrap();
    d.doc.slides[slide].shapes.iter().find(|s| s.id == sid).unwrap().clone()
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("myslides-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn loading_builds_tagged_slides_and_leaves_source_alone() {
    let d = dir("load");
    let h = harness(&d);
    let app = h.state();
    assert!(app.source.open);
    assert_eq!(app.session.active().unwrap().doc.slides.len(), 2, "the starter slide is replaced");
    for id in ["intro.title", "intro.body", "note", "math.title", "eq1"] {
        tagged(app, id);
    }
    assert!(tagged(app, "intro.body").xfrm.is_none(), "unpinned placeholders stay on the layout");
    assert_eq!(app.source.text, SRC);
    assert_eq!(std::fs::read_to_string(d.join("deck.md")).unwrap(), SRC);
}

#[test]
fn moving_a_shape_writes_its_box_back_and_saves() {
    let d = dir("move");
    let mut h = harness(&d);
    let (_, sid) = find_tagged(h.state(), "eq1").unwrap();
    let s = &mut h.state_mut().session;
    s.execute("slide.go", &json!({"index": 1})).unwrap();
    s.execute("shape.sizePosition", &json!({"ids": [sid.0], "x": 480, "y": 300, "w": 420, "h": 93})).unwrap();
    h.run_steps(2);
    let text = &h.state().source.text;
    assert!(text.contains("$$ e^(i pi) + 1 = 0 $$ {#eq1 x=480 y=300 w=420 h=93}"), "{text}");
    assert!(text.contains("{#note x=60 y=440 w=840}"), "untouched blocks stay as written");
    assert_eq!(&std::fs::read_to_string(d.join("deck.md")).unwrap(), text);
}

#[test]
fn typing_coordinates_moves_the_shape_without_rebuilding() {
    let d = dir("type");
    let mut h = harness(&d);
    let before = find_tagged(h.state(), "eq1").unwrap();
    let app = h.state_mut();
    app.source.text = app.source.text.replace("x=520", "x=300");
    app.source.edited_at = Some(-10.0);
    h.run_steps(2);
    let after = find_tagged(h.state(), "eq1").unwrap();
    assert_eq!(before, after, "same shape, not a rebuilt one");
    assert_eq!(tagged(h.state(), "eq1").xfrm.unwrap().x, 300.0);
}

#[test]
fn editing_content_rebuilds_only_that_slide() {
    let d = dir("content");
    let mut h = harness(&d);
    let other = find_tagged(h.state(), "eq1").unwrap();
    let app = h.state_mut();
    app.source.text = app.source.text.replace("- Nudge things in the editor", "- Nudge things in the editor\n- See it update live");
    app.source.edited_at = Some(-10.0);
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.session.active().unwrap().doc.slides.len(), 2);
    assert_eq!(find_tagged(app, "eq1").unwrap(), other, "slide 2 was not touched");
    let body = tagged(app, "intro.body");
    let txt = format!("{:?}", body.text);
    assert!(txt.contains("See it update live"), "{txt}");
    assert_eq!(std::fs::read_to_string(d.join("deck.md")).unwrap(), app.source.text, "edits are saved");
}

#[test]
fn screenshot() {
    let d = dir("shot");
    let mut h = harness(&d);
    h.run_steps(4);
    if let Ok(img) = h.render() {
        let _ = img.save(std::env::temp_dir().join("myslides-pane.png"));
    }
}
