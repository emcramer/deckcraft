//! MySlides: a markdown source of truth kept in sync with a DeckCraft document.
//!
//! Dialect (points on the slide, origin top-left):
//!
//! ```text
//! ## Slide title {#id layout=titleAndContent title=x,y,w,h body=x,y,w,h}
//! - bullets fill the layout's body placeholder
//! ![alt](image.png){#fig1 x=.. y=.. w=.. h=..}
//! $$ typst math $$ {#eq1 x=.. y=.. w=..}
//! ::: {#note x=.. y=.. w=..}
//! free text box
//! :::
//! ```
//!
//! Elements without coordinates are *auto* (the template places them); moving one in the GUI pins it
//! by writing coordinates into its attribute block. Nothing else in the source is rewritten.
//!
//! Shapes carry two tags: [`TAG_ID`] (the source id) and [`TAG_RECT`] (the rect MySlides placed the
//! shape at). Write-back compares the live box against `TAG_RECT`, so an auto element only becomes
//! pinned when the user actually moves it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use deckcraft_model::{Presentation, Shape};
use serde_json::{json, Value};
use std::path::Path;

pub const TAG_ID: &str = "myslides.id";
pub const TAG_RECT: &str = "myslides.rect";
const EPS: f64 = 0.05;

pub type Rect = [f64; 4];

// ------------------------------------------------------------------------------------------ source

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Image { src: String, alt: String },
    Math(String),
    Box(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Elem {
    pub id: String,
    pub kind: Kind,
    pub attrs: Vec<(String, String)>,
    /// 0-based line holding the attribute block.
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlideSrc {
    pub id: String,
    pub title: String,
    pub attrs: Vec<(String, String)>,
    pub line: usize,
    pub bullets: Vec<String>,
    pub elems: Vec<Elem>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deck {
    pub slides: Vec<SlideSrc>,
}

fn get<'a>(attrs: &'a [(String, String)], k: &str) -> Option<&'a str> {
    attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn set(attrs: &mut Vec<(String, String)>, k: &str, v: String) {
    match attrs.iter_mut().find(|(n, _)| n == k) {
        Some(e) => e.1 = v,
        None => attrs.push((k.to_string(), v)),
    }
}

impl Elem {
    /// The pinned rect, if the source gives one. A missing `h` is `None` (aspect-derived).
    pub fn pinned(&self) -> Option<[Option<f64>; 4]> {
        let n = |k| get(&self.attrs, k).and_then(|v| v.parse::<f64>().ok());
        Some([Some(n("x")?), Some(n("y")?), Some(n("w")?), n("h")])
    }
}

impl SlideSrc {
    pub fn layout(&self) -> &str {
        get(&self.attrs, "layout").unwrap_or(if self.bullets.is_empty() { "titleOnly" } else { "titleAndContent" })
    }
    /// Pinned geometry for the `title` / `body` placeholder.
    pub fn placeholder_rect(&self, role: &str) -> Option<Rect> {
        let v: Vec<f64> = get(&self.attrs, role)?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
    }
    /// Everything except pinned geometry: if this differs, the slide has to be rebuilt.
    fn content_key(&self) -> String {
        let strip = |a: &[(String, String)]| -> Vec<(String, String)> {
            a.iter().filter(|(k, _)| !matches!(k.as_str(), "x" | "y" | "w" | "h" | "title" | "body")).cloned().collect()
        };
        let elems: Vec<_> = self.elems.iter().map(|e| (e.id.clone(), e.kind.clone(), strip(&e.attrs), e.pinned().is_some())).collect();
        format!("{}|{}|{:?}|{:?}|{:?}", self.id, self.title, strip(&self.attrs), self.bullets, elems)
    }
}

fn parse_attrs(s: &str) -> (Option<String>, Vec<(String, String)>) {
    let (mut id, mut kv) = (None, Vec::new());
    for tok in s.split_whitespace() {
        if let Some(i) = tok.strip_prefix('#') {
            id = Some(i.to_string());
        } else if let Some((k, v)) = tok.split_once('=') {
            kv.push((k.to_string(), v.to_string()));
        }
    }
    (id, kv)
}

/// Split a trailing `{...}` attribute block off a line.
fn trailing_attrs(line: &str) -> (&str, Option<&str>) {
    let t = line.trim_end();
    if t.ends_with('}') {
        if let Some(open) = t.rfind('{') {
            let inner = &t[open + 1..t.len() - 1];
            if inner.trim_start().starts_with('#') || inner.contains('=') {
                return (t[..open].trim_end(), Some(inner));
            }
        }
    }
    (t, None)
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "slide".into() } else { out }
}

pub fn parse(text: &str) -> Deck {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut deck = Deck::default();
    let mut i = 0;
    while i < lines.len() {
        let ln = lines[i];
        if let Some(head) = ln.strip_prefix("## ") {
            let (title, a) = trailing_attrs(head);
            let (id, attrs) = a.map(parse_attrs).unwrap_or_default();
            deck.slides.push(SlideSrc { id: id.unwrap_or_else(|| slug(title)), title: title.to_string(), attrs, line: i, bullets: vec![], elems: vec![] });
        } else if let Some(sl) = deck.slides.last_mut() {
            let t = ln.trim_start();
            let n = sl.elems.len();
            if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                sl.bullets.push(b.trim().to_string());
            } else if t.starts_with("![") {
                let (body, a) = trailing_attrs(t);
                if let (Some(close), true) = (body.find("]("), body.ends_with(')')) {
                    let (id, attrs) = a.map(parse_attrs).unwrap_or_default();
                    let kind = Kind::Image { alt: body[2..close].to_string(), src: body[close + 2..body.len() - 1].to_string() };
                    sl.elems.push(Elem { id: id.unwrap_or_else(|| format!("{}-img{n}", sl.id)), kind, attrs, line: i });
                }
            } else if t.starts_with("$$") {
                let (body, a) = trailing_attrs(t);
                if let Some(m) = body.strip_prefix("$$").and_then(|b| b.strip_suffix("$$")) {
                    let (id, attrs) = a.map(parse_attrs).unwrap_or_default();
                    sl.elems.push(Elem { id: id.unwrap_or_else(|| format!("{}-eq{n}", sl.id)), kind: Kind::Math(m.trim().into()), attrs, line: i });
                }
            } else if let Some(rest) = t.strip_prefix(":::") {
                let (_, a) = trailing_attrs(rest);
                let (id, attrs) = a.map(parse_attrs).unwrap_or_default();
                let mut body = Vec::new();
                let start = i;
                i += 1;
                while i < lines.len() && lines[i].trim() != ":::" {
                    body.push(lines[i]);
                    i += 1;
                }
                let text = body.join("\n").trim().to_string();
                sl.elems.push(Elem { id: id.unwrap_or_else(|| format!("{}-box{n}", sl.id)), kind: Kind::Box(text), attrs, line: start });
            }
        }
        i += 1;
    }
    deck
}

// ------------------------------------------------------------------------------- patching source

fn num(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

fn fmt_attrs(id: &str, attrs: &[(String, String)]) -> String {
    let mut s = format!("{{#{id}");
    for (k, v) in attrs {
        s.push_str(&format!(" {k}={v}"));
    }
    s.push('}');
    s
}

fn replace_attr_block(line: &str, block: &str) -> String {
    match trailing_attrs(line) {
        (head, Some(_)) => {
            let indent = &line[..line.len() - line.trim_start().len()];
            let head = head.trim_start();
            // images keep the block glued to `)`; everything else gets a space
            if head.ends_with(')') && head.starts_with("![") { format!("{indent}{head}{block}") } else { format!("{indent}{head} {block}") }
        }
        (head, None) => {
            if head.trim_start().starts_with("![") { format!("{head}{block}") } else { format!("{head} {block}") }
        }
    }
}

// ----------------------------------------------------------------------------------- write-back

fn tag<'a>(sh: &'a Shape, k: &str) -> Option<&'a str> {
    sh.tags.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn parse_rect(s: &str) -> Option<Rect> {
    let v: Vec<f64> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
}

pub fn fmt_rect(r: Rect) -> String {
    r.iter().map(|v| num(*v)).collect::<Vec<_>>().join(",")
}

/// One geometry change found in the document.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub id: String,
    pub rect: Rect,
}

/// Compare the live document with the source and return the source with geometry written back,
/// plus what changed. `None` when nothing moved.
pub fn write_back(src: &str, pres: &Presentation) -> Option<(String, Vec<Change>)> {
    let deck = parse(src);
    let mut lines: Vec<String> = src.split('\n').map(str::to_string).collect();
    let mut seen = std::collections::HashSet::new();
    let mut changes = Vec::new();
    for slide in &pres.slides {
        for sh in &slide.shapes {
            let (Some(id), Some(x)) = (tag(sh, TAG_ID), sh.xfrm.as_ref()) else { continue };
            if !seen.insert(id.to_string()) {
                continue; // a GUI duplicate of a shape we already synced
            }
            let live = [x.x, x.y, x.w, x.h];
            let placed = tag(sh, TAG_RECT).and_then(parse_rect);
            if placed.is_some_and(|p| (0..4).all(|i| (p[i] - live[i]).abs() < EPS)) {
                continue; // still where MySlides put it
            }
            if let Some((sid, role)) = id.rsplit_once('.').filter(|(_, r)| matches!(*r, "title" | "body")) {
                let Some(sl) = deck.slides.iter().find(|s| s.id == sid) else { continue };
                if sl.placeholder_rect(role).is_some_and(|p| (0..4).all(|i| (p[i] - live[i]).abs() < EPS)) {
                    continue;
                }
                let mut attrs = sl.attrs.clone();
                set(&mut attrs, role, fmt_rect(live));
                lines[sl.line] = format!("## {} {}", sl.title, fmt_attrs(&sl.id, &attrs));
                changes.push(Change { id: id.into(), rect: live });
                continue;
            }
            let Some(e) = deck.slides.iter().flat_map(|s| &s.elems).find(|e| e.id == id) else { continue };
            let is_box = matches!(e.kind, Kind::Box(_));
            let keys: &[&str] = if is_box { &["x", "y", "w"] } else { &["x", "y", "w", "h"] };
            if let Some(p) = e.pinned() {
                let same = keys.iter().enumerate().all(|(i, _)| p[i].is_none_or(|v| (v - live[i]).abs() < EPS));
                if same {
                    continue;
                }
            }
            let mut attrs = e.attrs.clone();
            for (i, k) in keys.iter().enumerate() {
                set(&mut attrs, k, num(live[i]));
            }
            lines[e.line] = replace_attr_block(&lines[e.line], &fmt_attrs(&e.id, &attrs));
            changes.push(Change { id: id.into(), rect: live });
        }
    }
    (!changes.is_empty()).then(|| (lines.join("\n"), changes))
}

// --------------------------------------------------------------------------------------- building

pub const SLIDE_W: f64 = 960.0;
pub const SLIDE_H: f64 = 540.0;

/// PNG/JPEG pixel size, for aspect ratios. Unknown formats return `None`.
pub fn image_size(bytes: &[u8]) -> Option<(f64, f64)> {
    if bytes.len() > 24 && bytes.starts_with(b"\x89PNG") {
        let be = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as f64;
        return Some((be(16), be(20)));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let m = bytes[i + 1];
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as f64;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as f64;
                return Some((w, h));
            }
            i += 2 + len;
        }
    }
    None
}

/// Where an element goes: its pinned rect, or a slot in the right-hand column (aspect-fitted).
pub fn place(sl: &SlideSrc, idx: usize, natural: Option<(f64, f64)>) -> Rect {
    let e = &sl.elems[idx];
    if let Some([Some(x), Some(y), Some(w), h]) = e.pinned() {
        let h = h.or_else(|| natural.map(|(nw, nh)| w * nh / nw)).unwrap_or(w * 0.6);
        return [x, y, w, (h * 10.0).round() / 10.0];
    }
    let n = sl.elems.len().max(1) as f64;
    let slot_h = (SLIDE_H - 160.0) / n;
    let slot = [500.0, 140.0 + idx as f64 * slot_h, 400.0, slot_h - 10.0];
    match natural {
        Some((nw, nh)) if !matches!(e.kind, Kind::Box(_)) => {
            let k = (slot[2] / nw).min(slot[3] / nh);
            let (w, h) = ((nw * k * 10.0).round() / 10.0, (nh * k * 10.0).round() / 10.0);
            [((slot[0] + (slot[2] - w) / 2.0) * 10.0).round() / 10.0, ((slot[1] + (slot[3] - h) / 2.0) * 10.0).round() / 10.0, w, h]
        }
        _ => slot,
    }
}

/// A command for `Session::execute` / `SlideApp::run`.
pub type Cmd = (String, Value);

fn c(name: &str, p: Value) -> Cmd {
    (name.to_string(), p)
}

/// Renders math to a PNG; supplied by the host (the UI uses Typst when available).
pub type MathRenderer<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// Commands that create slide `index` from `sl`. Every inserted shape is tagged right after insert
/// (it is the selection then). Placeholders are tagged afterwards with [`placeholder_commands`],
/// because their ids only exist once the slide does.
pub fn slide_commands(sl: &SlideSrc, index: usize, base: &Path, math: MathRenderer) -> Vec<Cmd> {
    let mut out = Vec::new();
    let mut p = json!({"layout": sl.layout(), "title": sl.title, "at": index});
    if !sl.bullets.is_empty() {
        p["body"] = json!(sl.bullets.join("\n"));
    }
    out.push(c("slide.new", p));
    for (i, e) in sl.elems.iter().enumerate() {
        let (data, name) = match &e.kind {
            Kind::Image { src, .. } => (std::fs::read(base.join(src)).ok(), src.rsplit('/').next().unwrap_or(src).to_string()),
            Kind::Math(m) => (math(m), format!("{}.png", e.id)),
            Kind::Box(_) => (None, String::new()),
        };
        let r = place(sl, i, data.as_deref().and_then(image_size));
        match (&e.kind, data) {
            (Kind::Box(t), _) => out.push(c("insert.textBox", json!({"rect": r, "text": t}))),
            (_, Some(bytes)) => out.push(c("insert.picture", json!({"data": b64(&bytes), "name": name, "rect": r}))),
            (Kind::Math(m), None) => out.push(c("insert.textBox", json!({"rect": r, "text": format!("${m}$")}))),
            (Kind::Image { src, .. }, None) => out.push(c("insert.textBox", json!({"rect": r, "text": format!("missing image: {src}")}))),
        }
        out.push(c("shape.tags", json!({"set": {TAG_ID: e.id, TAG_RECT: fmt_rect(r)}})));
    }
    out
}

/// Tag the title/body placeholders of a freshly built slide, and apply pinned placeholder geometry.
pub fn placeholder_commands(sl: &SlideSrc, shapes: &[Shape]) -> Vec<Cmd> {
    let mut out = Vec::new();
    for sh in shapes {
        let Some(ph) = &sh.ph else { continue };
        let role = match format!("{:?}", ph.kind).to_ascii_lowercase().as_str() {
            "title" | "ctrtitle" => "title",
            "body" | "obj" => "body",
            _ => continue,
        };
        let id = sh.id;
        let mut tags = json!({TAG_ID: format!("{}.{role}", sl.id)});
        if let Some(r) = sl.placeholder_rect(role) {
            out.push(c("shape.sizePosition", json!({"ids": [id], "x": r[0], "y": r[1], "w": r[2], "h": r[3]})));
            tags[TAG_RECT] = json!(fmt_rect(r));
        }
        out.push(c("shape.tags", json!({"ids": [id], "set": tags})));
    }
    out
}

fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= ch.len() {
                s.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

// ------------------------------------------------------------------------------ source -> document

/// What a source edit means for the document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// Slide indices (in the new deck) to delete and recreate.
    pub rebuild: Vec<usize>,
    /// Slides to delete from the end when the new deck is shorter.
    pub drop_tail: usize,
    /// Pinned geometry that changed in the source: (source id, rect).
    pub moves: Vec<(String, Rect)>,
}

/// Diff two parses of the source. Content changes rebuild their slide; pure geometry edits become
/// moves, so typing `x=300` nudges the shape without recreating anything.
pub fn plan(old: &Deck, new: &Deck) -> Plan {
    let mut p = Plan::default();
    for (i, ns) in new.slides.iter().enumerate() {
        let Some(os) = old.slides.get(i) else {
            p.rebuild.push(i);
            continue;
        };
        if os.content_key() != ns.content_key() {
            p.rebuild.push(i);
            continue;
        }
        for role in ["title", "body"] {
            if let Some(r) = ns.placeholder_rect(role).filter(|r| os.placeholder_rect(role) != Some(*r)) {
                p.moves.push((format!("{}.{role}", ns.id), r));
            }
        }
        for (k, (oe, ne)) in os.elems.iter().zip(&ns.elems).enumerate() {
            if oe.attrs != ne.attrs {
                if let Some([Some(x), Some(y), Some(w), h]) = ne.pinned() {
                    let h = h.unwrap_or_else(|| place(ns, k, None)[3]);
                    p.moves.push((ne.id.clone(), [x, y, w, h]));
                }
            }
        }
    }
    p.drop_tail = old.slides.len().saturating_sub(new.slides.len());
    p
}

#[cfg(test)]
mod tests;
