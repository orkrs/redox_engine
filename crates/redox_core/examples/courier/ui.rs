//! Screens of the game built with the game UI toolkit, and the localisation tables.

use redox_ui::game::{Align, Color, Dim, Localization, Node};

pub const EN: &str = "\
title = COURIER
menu.play = Play
menu.language = Language: English
menu.quit = Quit
pause.title = Paused
pause.resume = Resume
pause.menu = Main menu
hud.coins = Coins
hud.coins.one = {n} coin
hud.coins.other = {n} coins
hud.time = Time {t} s
hud.state = Animation: {state}
hud.streaming = Ground texture: mip {mip} ({kib} KiB resident)
hud.hint = WASD move · Q/E camera · Space wave · Esc pause
win.title = All coins collected!
win.time = Your time: {t} s
win.again = Play again
win.record = New record!
win.best = Best time: {t} s
menu.best = Best time: {t} s
";

pub const RU: &str = "\
title = КУРЬЕР
menu.play = Играть
menu.language = Язык: русский
menu.quit = Выход
pause.title = Пауза
pause.resume = Продолжить
pause.menu = Главное меню
hud.coins = Монеты
hud.coins.one = {n} монета
hud.coins.few = {n} монеты
hud.coins.many = {n} монет
hud.time = Время {t} с
hud.state = Анимация: {state}
hud.streaming = Текстура земли: mip {mip} ({kib} КиБ в памяти)
hud.hint = WASD — ходьба · Q/E — камера · Пробел — помахать · Esc — пауза
win.title = Все монеты собраны!
win.time = Ваше время: {t} с
win.again = Играть ещё раз
win.record = Новый рекорд!
win.best = Лучшее время: {t} с
menu.best = Лучшее время: {t} с
";

pub fn localization() -> Localization {
    let mut loc = Localization::new("en");
    loc.add_language("en", EN).expect("English table");
    loc.add_language("ru", RU).expect("Russian table");
    loc
}

const PANEL: Color = Color(12, 16, 28, 215);
const ACCENT: Color = Color(255, 210, 90, 255);

fn menu_column(title_key: &str) -> Node {
    Node::column(18.0)
        .padding(36.0)
        .background(PANEL)
        .cross_align(Align::Center)
        .child(Node::label_key(title_key).font_size(64.0).color(ACCENT))
        .child(Node::spacer(0.0, 12.0))
}

fn centered(content: Node) -> Node {
    Node::stack().child(content.anchored(0.5, 0.5))
}

fn wide(button: Node) -> Node {
    button.width(Dim::Fixed(420.0)).font_size(34.0)
}

pub fn main_menu(best: Option<f32>) -> Node {
    let mut column = menu_column("title");
    if let Some(seconds) = best {
        column = column.child(Node::label_key("menu.best").arg("t", &format!("{seconds:.1}")));
    }
    centered(
        column
            .child(wide(Node::button_key("play", "menu.play")))
            .child(wide(Node::button_key("language", "menu.language")))
            .child(wide(Node::button_key("quit", "menu.quit"))),
    )
}

pub fn pause_menu() -> Node {
    centered(
        menu_column("pause.title")
            .child(wide(Node::button_key("resume", "pause.resume")))
            .child(wide(Node::button_key("menu", "pause.menu"))),
    )
}

pub fn win_screen(seconds: f32, best: Option<f32>, new_record: bool) -> Node {
    let mut column = menu_column("win.title")
        .child(Node::label_key("win.time").arg("t", &format!("{seconds:.1}")));
    if new_record {
        column = column.child(Node::label_key("win.record").color(ACCENT));
    } else if let Some(best) = best {
        column = column.child(Node::label_key("win.best").arg("t", &format!("{best:.1}")));
    }
    centered(
        column
            .child(wide(Node::button_key("play", "win.again")))
            .child(wide(Node::button_key("menu", "pause.menu"))),
    )
}

/// Numbers and strings the HUD shows.
pub struct HudData {
    pub coins: usize,
    pub total: usize,
    pub seconds: f32,
    pub state: String,
    pub mip: u32,
    pub resident_kib: usize,
}

pub fn hud(data: &HudData, loc: &Localization) -> Node {
    let coins_text = loc.plural("hud.coins", data.coins as i64, &[]);
    let progress = data.coins as f32 / data.total.max(1) as f32;
    Node::stack()
        .padding(8.0)
        .child(
            Node::column(8.0)
                .padding(14.0)
                .background(Color(0, 0, 0, 120))
                .child(Node::label_key("hud.coins").font_size(22.0).color(ACCENT))
                .child(Node::label(&format!("{coins_text} / {}", data.total)).font_size(34.0))
                .child(Node::bar(progress).color(ACCENT))
                .child(
                    Node::label_key("hud.time")
                        .arg("t", &format!("{:.1}", data.seconds))
                        .font_size(24.0),
                )
                .anchored(0.0, 0.0),
        )
        .child(
            Node::column(4.0)
                .padding(10.0)
                .background(Color(0, 0, 0, 120))
                .child(
                    Node::label_key("hud.state")
                        .arg("state", &data.state)
                        .font_size(20.0),
                )
                .child(
                    Node::label_key("hud.streaming")
                        .arg("mip", &data.mip.to_string())
                        .arg("kib", &data.resident_kib.to_string())
                        .font_size(20.0),
                )
                .anchored(1.0, 0.0),
        )
        .child(
            Node::label_key("hud.hint")
                .font_size(22.0)
                .anchored(0.5, 1.0),
        )
}

#[cfg(test)]
mod tests {
    use redox_ui::game::{Rect, UiScale, approx_text, layout_localized};

    use super::*;

    fn fits(tree: &Node, loc: &Localization, w: f32, h: f32) -> redox_ui::game::LaidUi {
        let scale = UiScale::new(w, h);
        let root = scale.root(0.03);
        let ui = layout_localized(tree, root, &approx_text, loc);
        for item in &ui.items {
            let r = item.rect;
            assert!(
                r.x >= root.x - 0.5
                    && r.y >= root.y - 0.5
                    && r.x + r.w <= root.x + root.w + 0.5
                    && r.y + r.h <= root.y + root.h + 0.5,
                "{:?} leaves the safe area {root:?} at {w}x{h}",
                item.kind
            );
        }
        ui
    }

    const SIZES: [(f32, f32); 4] = [
        (1920.0, 1080.0),
        (1280.0, 720.0),
        (1024.0, 768.0),
        (3440.0, 1440.0),
    ];

    #[test]
    fn every_menu_fits_every_window_in_both_languages() {
        let mut loc = localization();
        for language in ["en", "ru"] {
            loc.set_language(language);
            for (w, h) in SIZES {
                let main = fits(&main_menu(Some(12.3)), &loc, w, h);
                assert_eq!(main.buttons().len(), 3);
                let pause = fits(&pause_menu(), &loc, w, h);
                assert_eq!(pause.buttons().len(), 2);
                let win = fits(&win_screen(42.5, Some(40.0), false), &loc, w, h);
                let record = fits(&win_screen(42.5, Some(42.5), true), &loc, w, h);
                assert!(record.rect_of("play").is_some());
                assert!(win.rect_of("play").is_some());
            }
        }
    }

    #[test]
    fn menu_buttons_do_not_overlap() {
        let loc = localization();
        let ui = fits(&main_menu(None), &loc, 1920.0, 1080.0);
        let buttons = ui.buttons();
        for (i, (_, a)) in buttons.iter().enumerate() {
            for (_, b) in &buttons[i + 1..] {
                let overlap =
                    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
                assert!(!overlap, "{a:?} overlaps {b:?}");
            }
        }
    }

    fn labels(ui: &redox_ui::game::LaidUi) -> Vec<String> {
        ui.items
            .iter()
            .filter_map(|i| match &i.kind {
                redox_ui::game::Laid::Label(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_best_time_and_the_record_are_shown() {
        let mut loc = localization();
        let menu = fits(&main_menu(Some(31.25)), &loc, 1920.0, 1080.0);
        assert!(
            labels(&menu)
                .iter()
                .any(|t| t == "Best time: 31.2 s" || t == "Best time: 31.3 s")
        );
        let none = fits(&main_menu(None), &loc, 1920.0, 1080.0);
        assert!(!labels(&none).iter().any(|t| t.starts_with("Best time")));

        let record = fits(&win_screen(20.0, Some(20.0), true), &loc, 1920.0, 1080.0);
        assert!(labels(&record).iter().any(|t| t == "New record!"));
        let slower = fits(&win_screen(25.0, Some(20.0), false), &loc, 1920.0, 1080.0);
        assert!(labels(&slower).iter().any(|t| t == "Best time: 20.0 s"));
        loc.set_language("ru");
        let ru = fits(&win_screen(20.0, Some(20.0), true), &loc, 1920.0, 1080.0);
        assert!(labels(&ru).iter().any(|t| t == "Новый рекорд!"));
    }

    #[test]
    fn the_hud_fills_corners_and_shows_translated_counts() {
        let mut loc = localization();
        loc.set_language("ru");
        let data = HudData {
            coins: 3,
            total: 10,
            seconds: 12.34,
            state: "move".into(),
            mip: 2,
            resident_kib: 340,
        };
        for (w, h) in SIZES {
            let ui = fits(&hud(&data, &loc), &loc, w, h);
            let text: Vec<String> = ui
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    redox_ui::game::Laid::Label(t) => Some(t.clone()),
                    _ => None,
                })
                .collect();
            assert!(text.iter().any(|t| t.starts_with("3 монеты /")), "{text:?}");
            assert!(text.iter().any(|t| t.contains("Время 12.3")), "{text:?}");
            let _ = Rect::default();
        }
    }
}
