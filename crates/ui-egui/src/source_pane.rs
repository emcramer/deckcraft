//! MySlides source pane: the deck's markdown beside the slide, kept in sync both ways.
//!
//! - Typing (debounced) re-parses the source and applies only what changed: geometry edits move
//!   the shape, other edits rebuild just that slide.
//! - Moving or resizing a tagged shape in the canvas writes its new box back into the source's
//!   `{#id x= y= w= h=}` block, and the file is saved.
//! - Putting the caret on an element's line selects that shape on the slide.

use crate::SlideApp;
use deckcraft_myslides as ms;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Seconds of typing quiet before the source is applied to the slides.
const DEBOUNCE: f64 = 0.4;

#[derive(Default)]
pub struct State {
    pub open: bool,
    /// The markdown file.
    pub path: String,
    pub text: String,
    /// The source as last applied to (or read back from) the document.
    applied: ms::Deck,
    /// Document revision the source last matched.
    seen_rev: u64,
    edited_at: Option<f64>,
    caret_line: Option<usize>,
    status: String,
}

fn flag(p: &Value, cur: bool) -> bool {
    p.get("on").and_then(Value::as_bool).unwrap_or(!cur)
}

/// `view.source {on?, path?}`: show/hide the pane, optionally loading a markdown file.
pub fn command(app: &mut SlideApp, p: &Value) -> Result<Value, String> {
    app.source.open = flag(p, app.source.open);
    if let Some(path) = p.get("path").and_then(Value::as_str) {
        app.source.open = true;
        load(app, path)?;
    } else if app.source.open && app.source.path.is_empty() {
        // Default: deck.deckcraft -> deck.md next to it.
        if let Some(md) = app.session.active().and_then(|d| d.path.clone()).map(|p| PathBuf::from(p).with_extension("md")) {
            if md.exists() {
                load(app, &md.to_string_lossy())?;
            }
        }
    }
    Ok(json!({"open": app.source.open, "path": app.source.path}))
}

fn revision(app: &SlideApp) -> u64 {
    app.session.active().map(|d| d.revision).unwrap_or(0)
}

fn base_dir(app: &SlideApp) -> PathBuf {
    Path::new(&app.source.path).parent().map(Path::to_path_buf).unwrap_or_default()
}

fn has_tags(app: &SlideApp) -> bool {
    app.session.active().is_some_and(|d| d.doc.slides.iter().any(|s| s.shapes.iter().any(|sh| sh.tags.iter().any(|(k, _)| k == ms::TAG_ID))))
}

/// Load a markdown file. A document without MySlides tags is rebuilt from it; otherwise the
/// document's geometry is read back into the source.
pub fn load(app: &mut SlideApp, path: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    app.source.path = path.to_string();
    app.source.text = text;
    if has_tags(app) {
        app.source.applied = ms::parse(&app.source.text);
        app.source.seen_rev = revision(app);
        pull_geometry(app);
    } else {
        rebuild_all(app);
    }
    Ok(())
}

/// Replace every slide with ones built from the source.
pub fn rebuild_all(app: &mut SlideApp) {
    let n = app.session.active().map(|d| d.doc.slides.len()).unwrap_or(0);
    let old = ms::Deck { slides: vec![ms::SlideSrc { id: String::new(), title: String::new(), attrs: vec![], line: 0, bullets: vec![], elems: vec![] }; n] };
    app.source.applied = old;
    apply(app, true);
}

fn run(app: &mut SlideApp, errs: &mut Vec<String>, (name, p): ms::Cmd) {
    if let Err(e) = app.run(&name, p) {
        errs.push(format!("{name}: {e}"));
    }
}

fn find_tagged(app: &SlideApp, id: &str) -> Option<(usize, deckcraft_model::ShapeId)> {
    let d = app.session.active()?;
    d.doc.slides.iter().enumerate().find_map(|(i, s)| {
        s.shapes.iter().find(|sh| sh.tags.iter().any(|(k, v)| k == ms::TAG_ID && v == id)).map(|sh| (i, sh.id))
    })
}

/// Apply the source to the document. `force` rebuilds every slide.
fn apply(app: &mut SlideApp, force: bool) {
    let new = ms::parse(&app.source.text);
    let mut plan = ms::plan(&app.source.applied, &new);
    if force {
        plan.rebuild = (0..new.slides.len()).collect();
        plan.moves.clear();
    }
    let here = app.session.active().map(|d| d.selection.slide).unwrap_or(0);
    let base = base_dir(app);
    let mut errs = Vec::new();
    for _ in 0..plan.drop_tail {
        let last = app.session.active().map(|d| d.doc.slides.len()).unwrap_or(0);
        if last == 0 {
            break;
        }
        run(app, &mut errs, ("slide.go".into(), json!({"index": last - 1})));
        run(app, &mut errs, ("slide.delete".into(), json!({})));
    }
    let old_len = app.source.applied.slides.len();
    for &i in &plan.rebuild {
        let Some(sl) = new.slides.get(i) else { continue };
        let exists = i < old_len && app.session.active().is_some_and(|d| i < d.doc.slides.len());
        if exists {
            run(app, &mut errs, ("slide.go".into(), json!({"index": i})));
            run(app, &mut errs, ("slide.delete".into(), json!({})));
        }
        for c in ms::slide_commands(sl, i, &base, &|_| None) {
            run(app, &mut errs, c);
        }
        let shapes = app.session.active().and_then(|d| d.doc.slides.get(i).map(|s| s.shapes.clone())).unwrap_or_default();
        run(app, &mut errs, ("slide.go".into(), json!({"index": i})));
        for c in ms::placeholder_commands(sl, &shapes) {
            run(app, &mut errs, c);
        }
    }
    // A forced rebuild of a document that had more slides than the source: drop the rest.
    if force {
        while app.session.active().is_some_and(|d| d.doc.slides.len() > new.slides.len().max(1)) {
            let last = app.session.active().map(|d| d.doc.slides.len()).unwrap_or(1) - 1;
            run(app, &mut errs, ("slide.go".into(), json!({"index": last})));
            run(app, &mut errs, ("slide.delete".into(), json!({})));
        }
    }
    for (id, r) in &plan.moves {
        let Some((slide, sid)) = find_tagged(app, id) else { continue };
        run(app, &mut errs, ("slide.go".into(), json!({"index": slide})));
        run(app, &mut errs, ("edit.select".into(), json!({"ids": [sid]})));
        run(app, &mut errs, ("shape.sizePosition".into(), json!({"x": r[0], "y": r[1], "w": r[2], "h": r[3]})));
        run(app, &mut errs, ("shape.tags".into(), json!({"ids": [sid], "set": {ms::TAG_RECT: ms::fmt_rect(*r)}})));
    }
    let n = app.session.active().map(|d| d.doc.slides.len()).unwrap_or(1);
    if !plan.rebuild.is_empty() || !plan.moves.is_empty() || plan.drop_tail > 0 {
        let _ = app.run("slide.go", json!({"index": here.min(n.saturating_sub(1))}));
    }
    app.source.applied = new;
    app.source.seen_rev = revision(app);
    app.source.status = if errs.is_empty() {
        format!("applied: {} slide(s) rebuilt, {} moved", plan.rebuild.len(), plan.moves.len())
    } else {
        errs.join("; ")
    };
}

/// Read moved/resized shapes back into the source and save it.
fn pull_geometry(app: &mut SlideApp) {
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    if let Some((text, changes)) = ms::write_back(&app.source.text, &doc) {
        app.source.text = text;
        app.source.applied = ms::parse(&app.source.text);
        app.source.status = format!("from slide: {}", changes.iter().map(|c| format!("#{}", c.id)).collect::<Vec<_>>().join(", "));
        save(app);
    }
    app.source.seen_rev = revision(app);
}

fn save(app: &mut SlideApp) {
    if app.source.path.is_empty() {
        return;
    }
    if let Err(e) = std::fs::write(&app.source.path, &app.source.text) {
        app.source.status = format!("save failed: {e}");
    }
}

/// Once per frame while the pane is open.
fn poll(app: &mut SlideApp, now: f64) {
    if let Some(t) = app.source.edited_at {
        if now - t >= DEBOUNCE {
            app.source.edited_at = None;
            apply(app, false);
            save(app);
        }
    } else if revision(app) != app.source.seen_rev {
        pull_geometry(app);
    }
}

/// Source -> slide selection: the element (or slide heading) on the caret's line.
fn follow_caret(app: &mut SlideApp, line: usize) {
    let deck = &app.source.applied;
    let target = deck.slides.iter().enumerate().find_map(|(i, s)| {
        if s.line == line {
            return Some((i, None));
        }
        s.elems.iter().find(|e| e.line == line).map(|e| (i, Some(e.id.clone())))
    });
    match target {
        Some((i, None)) => {
            let _ = app.run("slide.go", json!({"index": i}));
        }
        Some((_, Some(id))) => {
            if let Some((slide, sid)) = find_tagged(app, &id) {
                let _ = app.run("slide.go", json!({"index": slide}));
                let _ = app.run("edit.select", json!({"ids": [sid]}));
            }
        }
        None => {}
    }
    app.source.seen_rev = revision(app);
}

pub fn show(app: &mut SlideApp, ui: &mut egui::Ui) {
    let now = ui.input(|i| i.time);
    poll(app, now);
    let t = crate::theme::Tokens::get(ui.ctx());
    egui::SidePanel::left("myslides.source")
        .resizable(true)
        .default_width(440.0)
        .min_width(220.0)
        .frame(egui::Frame::NONE.fill(t.card).inner_margin(egui::Margin::same(8)))
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Markdown source").font(crate::theme::bold(15.0)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Hide (Cmd+Shift+M)").clicked() {
                        app.source.open = false;
                    }
                    if ui.small_button("Rebuild").on_hover_text("Rebuild every slide from the source").clicked() {
                        rebuild_all(app);
                    }
                });
            });
            ui.horizontal(|ui| {
                let resp = ui.add(egui::TextEdit::singleline(&mut app.source.path).hint_text("path/to/deck.md").desired_width(ui.available_width() - 50.0));
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Load").clicked() || enter {
                    let p = app.source.path.clone();
                    if let Err(e) = load(app, &p) {
                        app.source.status = e;
                    }
                }
            });
            if !app.source.status.is_empty() {
                ui.label(egui::RichText::new(&app.source.status).small().color(t.text_dim));
            }
            ui.separator();
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                let out = egui::TextEdit::multiline(&mut app.source.text)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(40)
                    .lock_focus(true)
                    .show(ui);
                if out.response.changed() {
                    app.source.edited_at = Some(now);
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(DEBOUNCE));
                }
                if out.response.has_focus() {
                    if let Some(r) = out.cursor_range {
                        let idx = r.primary.index;
                        let line = app.source.text.chars().take(idx).filter(|&c| c == '\n').count();
                        if app.source.caret_line != Some(line) {
                            app.source.caret_line = Some(line);
                            if app.source.edited_at.is_none() {
                                follow_caret(app, line);
                            }
                        }
                    }
                }
            });
        });
}

#[cfg(test)]
#[path = "source_pane_tests.rs"]
mod tests;
