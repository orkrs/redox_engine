use super::*;

fn lay(tree: &Node, w: f32, h: f32) -> LaidUi {
    layout(tree, Rect::new(0.0, 0.0, w, h), &|text: &str, size: f32| {
        (text.chars().count() as f32 * size, size)
    })
}

#[test]
fn a_column_stacks_children_with_spacing_and_padding() {
    let tree = Node::column(10.0)
        .padding(5.0)
        .child(Node::button("a", "ab").font_size(10.0).padding(0.0))
        .child(Node::button("b", "abcd").font_size(10.0).padding(0.0));
    let ui = lay(&tree, 500.0, 500.0);
    let a = ui.rect_of("a").unwrap();
    let b = ui.rect_of("b").unwrap();
    assert_eq!((a.x, a.y, a.w, a.h), (5.0, 5.0, 20.0, 10.0));
    assert_eq!((b.x, b.y, b.w, b.h), (5.0, 25.0, 40.0, 10.0));
}

#[test]
fn fill_children_share_the_remaining_room() {
    let tree = Node::row(0.0)
        .width(Dim::Fixed(300.0))
        .height(Dim::Fixed(40.0))
        .child(Node::spacer(100.0, 10.0))
        .child(Node::button("a", "").padding(0.0).width(Dim::Fill))
        .child(Node::button("b", "").padding(0.0).width(Dim::Fill));
    let ui = lay(&tree, 800.0, 600.0);
    assert_eq!(ui.rect_of("a").unwrap().w, 100.0);
    assert_eq!(ui.rect_of("b").unwrap().w, 100.0);
    assert_eq!(ui.rect_of("b").unwrap().x, 200.0);
}

#[test]
fn cross_alignment_centres_children() {
    let tree = Node::column(0.0)
        .width(Dim::Fixed(200.0))
        .cross_align(Align::Center)
        .child(Node::button("a", "xxxx").font_size(10.0).padding(0.0));
    let a = lay(&tree, 500.0, 500.0).rect_of("a").unwrap();
    assert_eq!(a.x, 80.0, "(200 - 40) / 2");
}

#[test]
fn stack_children_are_anchored_to_corners_and_the_centre() {
    let tree = Node::stack()
        .child(
            Node::button("tl", "")
                .width(Dim::Fixed(20.0))
                .height(Dim::Fixed(10.0))
                .padding(0.0)
                .anchored(0.0, 0.0),
        )
        .child(
            Node::button("br", "")
                .width(Dim::Fixed(20.0))
                .height(Dim::Fixed(10.0))
                .padding(0.0)
                .anchored(1.0, 1.0),
        )
        .child(
            Node::button("c", "")
                .width(Dim::Fixed(20.0))
                .height(Dim::Fixed(10.0))
                .padding(0.0)
                .anchored(0.5, 0.5)
                .offset(0.0, -5.0),
        );
    let ui = lay(&tree, 200.0, 100.0);
    assert_eq!(ui.rect_of("tl").unwrap(), Rect::new(0.0, 0.0, 20.0, 10.0));
    assert_eq!(
        ui.rect_of("br").unwrap(),
        Rect::new(180.0, 90.0, 20.0, 10.0)
    );
    assert_eq!(ui.rect_of("c").unwrap(), Rect::new(90.0, 40.0, 20.0, 10.0));
}

#[test]
fn backgrounds_are_painted_before_their_children() {
    let tree = Node::column(0.0)
        .background(Color(10, 20, 30, 255))
        .child(Node::label("hi"));
    let ui = lay(&tree, 100.0, 100.0);
    assert_eq!(ui.items[0].kind, Laid::Panel);
    assert!(matches!(ui.items[1].kind, Laid::Label(_)));
}

#[test]
fn the_scale_fits_any_window_and_keeps_the_safe_margin() {
    let wide = UiScale::new(3840.0, 1080.0);
    assert_eq!(wide.scale, 1.0);
    assert!(wide.width > 1920.0, "ultra-wide gets extra width");
    let small = UiScale::new(960.0, 540.0);
    assert!((small.scale - 0.5).abs() < 1e-6);
    assert!((small.width - 1920.0).abs() < 1e-3);
    let root = small.root(0.05);
    assert!((root.x - 96.0).abs() < 1e-3 && (root.w - 1728.0).abs() < 1e-3);
    let (sx, sy) = small.to_screen(1920.0, 1080.0);
    assert_eq!((sx, sy), (960.0, 540.0));
    assert_eq!(small.to_virtual(480.0, 270.0), (960.0, 540.0));
}

#[test]
fn button_hit_testing_returns_the_topmost_button() {
    let tree = Node::row(0.0)
        .child(Node::button("a", "x").font_size(10.0).padding(0.0))
        .child(Node::button("b", "x").font_size(10.0).padding(0.0));
    let ui = lay(&tree, 100.0, 100.0);
    assert_eq!(ui.button_at(3.0, 3.0).as_deref(), Some("a"));
    assert_eq!(ui.button_at(13.0, 3.0).as_deref(), Some("b"));
    assert_eq!(ui.button_at(50.0, 50.0), None);
}
