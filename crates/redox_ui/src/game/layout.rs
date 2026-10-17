//! Widget tree and layout.
//!
//! Sizes are measured in a virtual space (1920x1080 by default, see [`UiScale`]), so a tree looks
//! the same at any resolution. Containers lay their children out; [`layout`] turns a tree into a
//! flat, paint-ordered list of rectangles ([`LaidUi`]).

use super::loc::Localization;

/// An axis-aligned rectangle (x right, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle from position and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Centre point.
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    /// Whether a point is inside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    /// The rectangle shrunk by `m` on every side (never negative).
    pub fn inset(&self, m: f32) -> Rect {
        Rect::new(
            self.x + m,
            self.y + m,
            (self.w - 2.0 * m).max(0.0),
            (self.h - 2.0 * m).max(0.0),
        )
    }
}

/// An RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    /// Opaque white.
    pub const WHITE: Color = Color(255, 255, 255, 255);
    /// Fully transparent.
    pub const CLEAR: Color = Color(0, 0, 0, 0);
}

/// How a node's width or height is decided.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dim {
    /// A fixed number of virtual pixels.
    Fixed(f32),
    /// All the room the parent offers (shared equally among `Fill` siblings).
    Fill,
    /// As big as the content.
    Fit,
}

/// Placement across a container's main axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Left / top.
    Start,
    /// Middle.
    Center,
    /// Right / bottom.
    End,
}

/// Text that is either written out or looked up in a [`Localization`].
#[derive(Clone, Debug, PartialEq)]
pub enum Text {
    /// Shown as is.
    Literal(String),
    /// Translated key with `{name}` arguments.
    Key {
        /// Key in the localisation table.
        key: String,
        /// `(name, value)` pairs substituted into the translation.
        args: Vec<(String, String)>,
    },
}

impl Text {
    fn resolve(&self, loc: Option<&Localization>) -> String {
        match (self, loc) {
            (Text::Literal(s), _) => s.clone(),
            (Text::Key { key, args }, Some(loc)) => {
                let args: Vec<(&str, &str)> =
                    args.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                loc.format(key, &args)
            }
            (Text::Key { key, .. }, None) => key.clone(),
        }
    }
}

/// Look of a node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Background fill.
    pub background: Option<Color>,
    /// Text and bar colour.
    pub color: Color,
    /// Text height in virtual pixels.
    pub font_size: f32,
    /// Colour of the focus frame around a focused button.
    pub focus_color: Color,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: None,
            color: Color::WHITE,
            font_size: 28.0,
            focus_color: Color(255, 210, 90, 255),
        }
    }
}

/// What a node is.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Children stacked top to bottom.
    Column {
        /// Gap between children.
        spacing: f32,
    },
    /// Children placed left to right.
    Row {
        /// Gap between children.
        spacing: f32,
    },
    /// Children overlap; each is placed by its anchor.
    Stack,
    /// A line of text.
    Label(Text),
    /// A focusable button.
    Button {
        /// Identifier reported when the button is activated.
        id: String,
        /// Caption.
        label: Text,
    },
    /// A horizontal progress bar (0..1).
    Bar {
        /// Fill amount.
        value: f32,
    },
    /// Empty space.
    Spacer,
}

/// A widget.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What it is.
    pub kind: Kind,
    /// Width rule.
    pub w: Dim,
    /// Height rule.
    pub h: Dim,
    /// Space between the node's edge and its content.
    pub padding: f32,
    /// Placement of children across the container's axis (columns: horizontal).
    pub cross: Align,
    /// Position inside a `Stack` parent: fractions (0..1) of the free room.
    pub anchor: (f32, f32),
    /// Offset from the anchored position.
    pub offset: (f32, f32),
    /// Look.
    pub style: Style,
    /// Children (containers only).
    pub children: Vec<Node>,
}

impl Node {
    fn new(kind: Kind, w: Dim, h: Dim) -> Self {
        Self {
            kind,
            w,
            h,
            padding: 0.0,
            cross: Align::Start,
            anchor: (0.0, 0.0),
            offset: (0.0, 0.0),
            style: Style::default(),
            children: Vec::new(),
        }
    }

    /// A column that fits its content.
    pub fn column(spacing: f32) -> Self {
        Self::new(Kind::Column { spacing }, Dim::Fit, Dim::Fit)
    }

    /// A row that fits its content.
    pub fn row(spacing: f32) -> Self {
        Self::new(Kind::Row { spacing }, Dim::Fit, Dim::Fit)
    }

    /// An overlay container that fills its parent.
    pub fn stack() -> Self {
        Self::new(Kind::Stack, Dim::Fill, Dim::Fill)
    }

    /// A text label.
    pub fn label(text: &str) -> Self {
        Self::new(Kind::Label(Text::Literal(text.into())), Dim::Fit, Dim::Fit)
    }

    /// A label whose text comes from the localisation table.
    pub fn label_key(key: &str) -> Self {
        Self::new(
            Kind::Label(Text::Key {
                key: key.into(),
                args: Vec::new(),
            }),
            Dim::Fit,
            Dim::Fit,
        )
    }

    /// A button with literal caption.
    pub fn button(id: &str, label: &str) -> Self {
        let mut node = Self::new(
            Kind::Button {
                id: id.into(),
                label: Text::Literal(label.into()),
            },
            Dim::Fit,
            Dim::Fit,
        );
        node.padding = 12.0;
        node
    }

    /// A button whose caption comes from the localisation table.
    pub fn button_key(id: &str, key: &str) -> Self {
        let mut node = Self::button(id, "");
        node.kind = Kind::Button {
            id: id.into(),
            label: Text::Key {
                key: key.into(),
                args: Vec::new(),
            },
        };
        node
    }

    /// A progress bar.
    pub fn bar(value: f32) -> Self {
        Self::new(
            Kind::Bar {
                value: value.clamp(0.0, 1.0),
            },
            Dim::Fixed(240.0),
            Dim::Fixed(18.0),
        )
    }

    /// Empty space of a fixed size.
    pub fn spacer(w: f32, h: f32) -> Self {
        Self::new(Kind::Spacer, Dim::Fixed(w), Dim::Fixed(h))
    }

    /// Adds a `{name}` argument to a key-based label or button.
    pub fn arg(mut self, name: &str, value: &str) -> Self {
        match &mut self.kind {
            Kind::Label(Text::Key { args, .. })
            | Kind::Button {
                label: Text::Key { args, .. },
                ..
            } => args.push((name.into(), value.into())),
            _ => {}
        }
        self
    }

    /// Adds a child.
    pub fn child(mut self, node: Node) -> Self {
        self.children.push(node);
        self
    }

    /// Sets the width rule.
    pub fn width(mut self, w: Dim) -> Self {
        self.w = w;
        self
    }

    /// Sets the height rule.
    pub fn height(mut self, h: Dim) -> Self {
        self.h = h;
        self
    }

    /// Sets the padding.
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = padding;
        self
    }

    /// Sets how children are aligned across the container's axis.
    pub fn cross_align(mut self, cross: Align) -> Self {
        self.cross = cross;
        self
    }

    /// Anchors the node inside a `Stack` parent (fractions of the free room, 0..1).
    pub fn anchored(mut self, x: f32, y: f32) -> Self {
        self.anchor = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        self
    }

    /// Moves the node from its anchored position.
    pub fn offset(mut self, dx: f32, dy: f32) -> Self {
        self.offset = (dx, dy);
        self
    }

    /// Sets the background.
    pub fn background(mut self, color: Color) -> Self {
        self.style.background = Some(color);
        self
    }

    /// Sets the text / bar colour.
    pub fn color(mut self, color: Color) -> Self {
        self.style.color = color;
        self
    }

    /// Sets the text height.
    pub fn font_size(mut self, size: f32) -> Self {
        self.style.font_size = size;
        self
    }
}

/// What a laid-out item draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Laid {
    /// A filled rectangle (container background).
    Panel,
    /// Text.
    Label(String),
    /// A button with its caption.
    Button {
        /// Identifier of the button.
        id: String,
        /// Caption.
        label: String,
    },
    /// A progress bar.
    Bar(f32),
}

/// One paint-ordered item.
#[derive(Clone, Debug, PartialEq)]
pub struct LaidItem {
    /// Position and size in virtual pixels.
    pub rect: Rect,
    /// What to draw.
    pub kind: Laid,
    /// Its style.
    pub style: Style,
}

/// A laid-out tree: items in paint order.
#[derive(Clone, Debug, Default)]
pub struct LaidUi {
    /// Items, back to front.
    pub items: Vec<LaidItem>,
}

impl LaidUi {
    /// Rectangle of the button with this id.
    pub fn rect_of(&self, id: &str) -> Option<Rect> {
        self.items.iter().find_map(|item| match &item.kind {
            Laid::Button { id: b, .. } if b == id => Some(item.rect),
            _ => None,
        })
    }

    /// `(id, rect)` of every button, in tree order.
    pub fn buttons(&self) -> Vec<(String, Rect)> {
        self.items
            .iter()
            .filter_map(|item| match &item.kind {
                Laid::Button { id, .. } => Some((id.clone(), item.rect)),
                _ => None,
            })
            .collect()
    }

    /// Id of the button under a point, if any (topmost wins).
    pub fn button_at(&self, x: f32, y: f32) -> Option<String> {
        self.items.iter().rev().find_map(|item| match &item.kind {
            Laid::Button { id, .. } if item.rect.contains(x, y) => Some(id.clone()),
            _ => None,
        })
    }
}

/// Maps the virtual layout space onto a window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiScale {
    /// Virtual pixels to window pixels.
    pub scale: f32,
    /// Width of the window in virtual pixels.
    pub width: f32,
    /// Height of the window in virtual pixels.
    pub height: f32,
}

impl UiScale {
    /// Reference size the designs are made for.
    pub const REFERENCE: (f32, f32) = (1920.0, 1080.0);

    /// Scale for a window of `screen_w` x `screen_h` pixels: the smaller ratio wins, so
    /// everything fits; the other axis gets extra virtual room.
    pub fn new(screen_w: f32, screen_h: f32) -> Self {
        let scale = (screen_w / Self::REFERENCE.0)
            .min(screen_h / Self::REFERENCE.1)
            .max(1e-3);
        Self {
            scale,
            width: screen_w / scale,
            height: screen_h / scale,
        }
    }

    /// The whole window in virtual pixels, shrunk by a safe-area margin (fraction of each side,
    /// e.g. 0.03 keeps content out of TV overscan).
    pub fn root(&self, safe_fraction: f32) -> Rect {
        let mx = self.width * safe_fraction;
        let my = self.height * safe_fraction;
        Rect::new(mx, my, self.width - 2.0 * mx, self.height - 2.0 * my)
    }

    /// Window pixels of a virtual point.
    pub fn to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale, y * self.scale)
    }

    /// Virtual point of window pixels.
    pub fn to_virtual(&self, x: f32, y: f32) -> (f32, f32) {
        (x / self.scale, y / self.scale)
    }
}

/// A rough text measure (about half the font size per character); real fonts replace it.
pub fn approx_text(text: &str, size: f32) -> (f32, f32) {
    (text.chars().count() as f32 * size * 0.55, size * 1.25)
}

struct Ctx<'a> {
    measure: &'a dyn Fn(&str, f32) -> (f32, f32),
    loc: Option<&'a Localization>,
    out: Vec<LaidItem>,
}

/// Lays `tree` out inside `root` with literal text only (keys show as themselves).
pub fn layout(tree: &Node, root: Rect, measure: &dyn Fn(&str, f32) -> (f32, f32)) -> LaidUi {
    run(tree, root, measure, None)
}

/// Lays `tree` out inside `root`, translating keys with `loc`.
pub fn layout_localized(
    tree: &Node,
    root: Rect,
    measure: &dyn Fn(&str, f32) -> (f32, f32),
    loc: &Localization,
) -> LaidUi {
    run(tree, root, measure, Some(loc))
}

fn run(
    tree: &Node,
    root: Rect,
    measure: &dyn Fn(&str, f32) -> (f32, f32),
    loc: Option<&Localization>,
) -> LaidUi {
    let mut ctx = Ctx {
        measure,
        loc,
        out: Vec::new(),
    };
    let size = resolve(tree, (root.w, root.h), &ctx);
    place(tree, Rect::new(root.x, root.y, size.0, size.1), &mut ctx);
    LaidUi { items: ctx.out }
}

fn own_text(node: &Node, ctx: &Ctx) -> Option<String> {
    match &node.kind {
        Kind::Label(t) | Kind::Button { label: t, .. } => Some(t.resolve(ctx.loc)),
        _ => None,
    }
}

/// Natural (content) size of a node, padding included.
fn natural(node: &Node, ctx: &Ctx) -> (f32, f32) {
    let pad = node.padding * 2.0;
    match &node.kind {
        Kind::Label(_) | Kind::Button { .. } => {
            let text = own_text(node, ctx).unwrap_or_default();
            let (w, h) = (ctx.measure)(&text, node.style.font_size);
            (w + pad, h + pad)
        }
        Kind::Bar { .. } => (240.0 + pad, 18.0 + pad),
        Kind::Spacer => (0.0, 0.0),
        Kind::Column { spacing } => {
            let sizes: Vec<(f32, f32)> =
                node.children.iter().map(|c| resolve_fit(c, ctx)).collect();
            let gaps = spacing * sizes.len().saturating_sub(1) as f32;
            (
                sizes.iter().map(|s| s.0).fold(0.0, f32::max) + pad,
                sizes.iter().map(|s| s.1).sum::<f32>() + gaps + pad,
            )
        }
        Kind::Row { spacing } => {
            let sizes: Vec<(f32, f32)> =
                node.children.iter().map(|c| resolve_fit(c, ctx)).collect();
            let gaps = spacing * sizes.len().saturating_sub(1) as f32;
            (
                sizes.iter().map(|s| s.0).sum::<f32>() + gaps + pad,
                sizes.iter().map(|s| s.1).fold(0.0, f32::max) + pad,
            )
        }
        Kind::Stack => {
            let sizes: Vec<(f32, f32)> =
                node.children.iter().map(|c| resolve_fit(c, ctx)).collect();
            (
                sizes.iter().map(|s| s.0).fold(0.0, f32::max) + pad,
                sizes.iter().map(|s| s.1).fold(0.0, f32::max) + pad,
            )
        }
    }
}

/// Size of a child when it is asked to fit (Fill counts as zero: it takes what is left).
fn resolve_fit(node: &Node, ctx: &Ctx) -> (f32, f32) {
    let nat = natural(node, ctx);
    let pick = |dim: Dim, natural: f32| match dim {
        Dim::Fixed(v) => v,
        Dim::Fit => natural,
        Dim::Fill => 0.0,
    };
    (pick(node.w, nat.0), pick(node.h, nat.1))
}

/// Size of a node inside `avail` (Fill takes all of it).
fn resolve(node: &Node, avail: (f32, f32), ctx: &Ctx) -> (f32, f32) {
    let nat = natural(node, ctx);
    let pick = |dim: Dim, natural: f32, room: f32| match dim {
        Dim::Fixed(v) => v,
        Dim::Fit => natural,
        Dim::Fill => room,
    };
    (pick(node.w, nat.0, avail.0), pick(node.h, nat.1, avail.1))
}

fn aligned(align: Align, start: f32, room: f32, size: f32) -> f32 {
    match align {
        Align::Start => start,
        Align::Center => start + (room - size) * 0.5,
        Align::End => start + room - size,
    }
}

fn place(node: &Node, rect: Rect, ctx: &mut Ctx) {
    if let Some(bg) = node.style.background {
        let mut style = node.style;
        style.background = Some(bg);
        ctx.out.push(LaidItem {
            rect,
            kind: Laid::Panel,
            style,
        });
    }
    match &node.kind {
        Kind::Label(_) => {
            let text = own_text(node, ctx).unwrap_or_default();
            ctx.out.push(LaidItem {
                rect: rect.inset(node.padding),
                kind: Laid::Label(text),
                style: Style {
                    background: None,
                    ..node.style
                },
            });
        }
        Kind::Button { id, .. } => {
            let label = own_text(node, ctx).unwrap_or_default();
            ctx.out.push(LaidItem {
                rect,
                kind: Laid::Button {
                    id: id.clone(),
                    label,
                },
                style: node.style,
            });
        }
        Kind::Bar { value } => ctx.out.push(LaidItem {
            rect,
            kind: Laid::Bar(*value),
            style: node.style,
        }),
        Kind::Spacer => {}
        Kind::Column { spacing } => place_axis(node, rect, *spacing, true, ctx),
        Kind::Row { spacing } => place_axis(node, rect, *spacing, false, ctx),
        Kind::Stack => {
            let inner = rect.inset(node.padding);
            for child in &node.children {
                let (cw, ch) = resolve(child, (inner.w, inner.h), ctx);
                let x = inner.x + (inner.w - cw) * child.anchor.0 + child.offset.0;
                let y = inner.y + (inner.h - ch) * child.anchor.1 + child.offset.1;
                place(child, Rect::new(x, y, cw, ch), ctx);
            }
        }
    }
}

fn place_axis(node: &Node, rect: Rect, spacing: f32, vertical: bool, ctx: &mut Ctx) {
    let inner = rect.inset(node.padding);
    let main_room = if vertical { inner.h } else { inner.w };
    let cross_room = if vertical { inner.w } else { inner.h };
    let main_dim = |c: &Node| if vertical { c.h } else { c.w };

    let fits: Vec<(f32, f32)> = node.children.iter().map(|c| resolve_fit(c, ctx)).collect();
    let used: f32 = fits
        .iter()
        .map(|s| if vertical { s.1 } else { s.0 })
        .sum::<f32>()
        + spacing * node.children.len().saturating_sub(1) as f32;
    let fills = node
        .children
        .iter()
        .filter(|c| main_dim(c) == Dim::Fill)
        .count();
    let share = if fills > 0 {
        ((main_room - used) / fills as f32).max(0.0)
    } else {
        0.0
    };

    let mut cursor = if vertical { inner.y } else { inner.x };
    for (child, fit) in node.children.iter().zip(&fits) {
        let main = if main_dim(child) == Dim::Fill {
            share
        } else if vertical {
            fit.1
        } else {
            fit.0
        };
        let cross_dim = if vertical { child.w } else { child.h };
        let cross = match cross_dim {
            Dim::Fill => cross_room,
            Dim::Fixed(v) => v,
            Dim::Fit => {
                let nat = natural(child, ctx);
                if vertical { nat.0 } else { nat.1 }
            }
        };
        let cross_pos = aligned(
            node.cross,
            if vertical { inner.x } else { inner.y },
            cross_room,
            cross,
        );
        let child_rect = if vertical {
            Rect::new(cross_pos, cursor, cross, main)
        } else {
            Rect::new(cursor, cross_pos, main, cross)
        };
        place(child, child_rect, ctx);
        cursor += main + spacing;
    }
}

#[cfg(test)]
mod tests;
