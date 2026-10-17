//! Screens of Foxfire built with the game UI toolkit, and the English / Russian tables.

use redox_ui::game::{Align, Color, Dim, Localization, Node};

pub const EN: &str = "\
title = FOXFIRE
tagline = Collect every glowing gem before the lantern burns out
menu.play = Play
menu.language = Language: English
menu.quit = Quit
menu.best = Best score: {n}
pause.title = Paused
pause.resume = Resume
pause.menu = Main menu
hud.gems = Gems {n} / {total}
hud.score = Score {n}
hud.combo = Combo x{n}
hud.time = {t} s
hud.stamina = Sprint
hud.exhausted = Out of breath!
hud.hint = WASD move · Shift sprint · Q/E camera · Esc pause · avoid the dark wisps
end.win = The glade is yours!
end.lose = The lantern went out
end.gems = Gems: {n} / {total}
end.score = Score: {n}
end.record = New best score!
end.best = Best score: {n}
end.again = Play again
credits = Fox model: PixelMannen (CC0). Animation: tomkranis (CC BY 4.0). Sky: Poly Haven (CC0)
";

pub const RU: &str = "\
title = ЛИСИЙ ОГОНЬ
tagline = Собери все светящиеся камни, пока не погас фонарь
menu.play = Играть
menu.language = Язык: русский
menu.quit = Выход
menu.best = Лучший счёт: {n}
pause.title = Пауза
pause.resume = Продолжить
pause.menu = Главное меню
hud.gems = Камни {n} / {total}
hud.score = Счёт {n}
hud.combo = Комбо x{n}
hud.time = {t} с
hud.stamina = Бег
hud.exhausted = Нет дыхания!
hud.hint = WASD — ходьба · Shift — бег · Q/E — камера · Esc — пауза · берегись тёмных огоньков
end.win = Поляна твоя!
end.lose = Фонарь погас
end.gems = Камни: {n} / {total}
end.score = Счёт: {n}
end.record = Новый рекорд!
end.best = Лучший счёт: {n}
end.again = Играть ещё раз
credits = Лиса: PixelMannen (CC0). Анимация: tomkranis (CC BY 4.0). Небо: Poly Haven (CC0)
";

pub fn localization() -> Localization {
    let mut loc = Localization::new("en");
    loc.add_language("en", EN).expect("English table");
    loc.add_language("ru", RU).expect("Russian table");
    loc
}

const PANEL: Color = Color(10, 14, 26, 220);
const ACCENT: Color = Color(255, 178, 64, 255);
const GEM: Color = Color(120, 235, 255, 255);
const DANGER: Color = Color(255, 90, 80, 255);

fn menu_column(title_key: &str) -> Node {
    Node::column(16.0)
        .padding(34.0)
        .background(PANEL)
        .cross_align(Align::Center)
        .child(Node::label_key(title_key).font_size(60.0).color(ACCENT))
        .child(Node::spacer(0.0, 8.0))
}

fn centered(content: Node) -> Node {
    Node::stack().child(content.anchored(0.5, 0.5))
}

fn wide(button: Node) -> Node {
    button.width(Dim::Fixed(420.0)).font_size(32.0)
}

pub fn main_menu(best: u32) -> Node {
    let mut column = menu_column("title").child(Node::label_key("tagline").font_size(22.0));
    if best > 0 {
        column = column.child(
            Node::label_key("menu.best")
                .arg("n", &best.to_string())
                .color(GEM),
        );
    }
    centered(
        column
            .child(wide(Node::button_key("play", "menu.play")))
            .child(wide(Node::button_key("language", "menu.language")))
            .child(wide(Node::button_key("quit", "menu.quit")))
            .child(Node::label_key("credits").font_size(15.0)),
    )
}

pub fn pause_menu() -> Node {
    centered(
        menu_column("pause.title")
            .child(wide(Node::button_key("resume", "pause.resume")))
            .child(wide(Node::button_key("menu", "pause.menu"))),
    )
}

/// The end of a run.
pub struct EndData {
    pub won: bool,
    pub gems: usize,
    pub total: usize,
    pub score: u32,
    pub best: u32,
    pub new_record: bool,
}

pub fn end_screen(data: &EndData) -> Node {
    let mut column = menu_column(if data.won { "end.win" } else { "end.lose" })
        .child(
            Node::label_key("end.gems")
                .arg("n", &data.gems.to_string())
                .arg("total", &data.total.to_string()),
        )
        .child(
            Node::label_key("end.score")
                .arg("n", &data.score.to_string())
                .font_size(34.0),
        );
    column = if data.new_record {
        column.child(Node::label_key("end.record").color(ACCENT))
    } else {
        column.child(Node::label_key("end.best").arg("n", &data.best.to_string()))
    };
    centered(
        column
            .child(wide(Node::button_key("play", "end.again")))
            .child(wide(Node::button_key("menu", "pause.menu"))),
    )
}

/// Numbers the HUD shows.
pub struct HudData {
    pub gems: usize,
    pub total: usize,
    pub score: u32,
    pub combo: u32,
    pub time_left: f32,
    pub stamina: f32,
    pub exhausted: bool,
    pub hit: bool,
}

pub fn hud(data: &HudData) -> Node {
    let time_color = if data.time_left < 10.0 {
        DANGER
    } else {
        ACCENT
    };
    let mut left = Node::column(6.0)
        .padding(14.0)
        .background(Color(0, 0, 0, 120))
        .child(
            Node::label_key("hud.gems")
                .arg("n", &data.gems.to_string())
                .arg("total", &data.total.to_string())
                .font_size(30.0)
                .color(GEM),
        )
        .child(Node::bar(data.gems as f32 / data.total.max(1) as f32).color(GEM))
        .child(
            Node::label_key("hud.score")
                .arg("n", &data.score.to_string())
                .font_size(24.0),
        );
    if data.combo > 1 {
        left = left.child(
            Node::label_key("hud.combo")
                .arg("n", &data.combo.to_string())
                .font_size(26.0)
                .color(ACCENT),
        );
    }
    let time = Node::column(2.0)
        .padding(10.0)
        .background(Color(0, 0, 0, if data.hit { 190 } else { 120 }))
        .child(
            Node::label_key("hud.time")
                .arg("t", &format!("{:.1}", data.time_left.max(0.0)))
                .font_size(46.0)
                .color(time_color),
        );
    let stamina = Node::column(4.0)
        .padding(10.0)
        .background(Color(0, 0, 0, 120))
        .child(
            Node::label_key(if data.exhausted {
                "hud.exhausted"
            } else {
                "hud.stamina"
            })
            .font_size(18.0)
            .color(if data.exhausted {
                DANGER
            } else {
                Color(230, 230, 230, 255)
            }),
        )
        .child(Node::bar(data.stamina).color(if data.exhausted { DANGER } else { ACCENT }));
    Node::stack()
        .padding(8.0)
        .child(left.anchored(0.0, 0.0))
        .child(time.anchored(0.5, 0.0))
        .child(stamina.anchored(0.5, 0.93))
        .child(
            Node::label_key("hud.hint")
                .font_size(18.0)
                .anchored(0.5, 1.0),
        )
}

#[cfg(test)]
mod tests {
    use redox_ui::game::{Laid, LaidUi, UiScale, approx_text, layout_localized};

    use super::*;

    const SIZES: [(f32, f32); 4] = [
        (1920.0, 1080.0),
        (1280.0, 720.0),
        (1024.0, 768.0),
        (3440.0, 1440.0),
    ];

    fn fits(tree: &Node, loc: &Localization, w: f32, h: f32) -> LaidUi {
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
                "{:?} leaves the safe area at {w}x{h}",
                item.kind
            );
        }
        ui
    }

    fn labels(ui: &LaidUi) -> Vec<String> {
        ui.items
            .iter()
            .filter_map(|i| match &i.kind {
                Laid::Label(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn hud_data() -> HudData {
        HudData {
            gems: 7,
            total: 24,
            score: 1800,
            combo: 3,
            time_left: 42.34,
            stamina: 0.6,
            exhausted: false,
            hit: false,
        }
    }

    #[test]
    fn every_screen_fits_every_window_in_both_languages() {
        let mut loc = localization();
        let end = EndData {
            won: true,
            gems: 24,
            total: 24,
            score: 9000,
            best: 8000,
            new_record: true,
        };
        for language in ["en", "ru"] {
            loc.set_language(language);
            for (w, h) in SIZES {
                assert_eq!(fits(&main_menu(1234), &loc, w, h).buttons().len(), 3);
                assert_eq!(fits(&pause_menu(), &loc, w, h).buttons().len(), 2);
                assert_eq!(fits(&end_screen(&end), &loc, w, h).buttons().len(), 2);
                fits(&hud(&hud_data()), &loc, w, h);
            }
        }
    }

    #[test]
    fn the_hud_shows_numbers_and_the_combo_only_when_it_matters() {
        let loc = localization();
        let ui = fits(&hud(&hud_data()), &loc, 1920.0, 1080.0);
        let text = labels(&ui);
        assert!(text.contains(&"Gems 7 / 24".to_string()), "{text:?}");
        assert!(text.contains(&"Score 1800".to_string()), "{text:?}");
        assert!(text.contains(&"Combo x3".to_string()), "{text:?}");
        assert!(text.contains(&"42.3 s".to_string()), "{text:?}");
        let calm = HudData {
            combo: 1,
            ..hud_data()
        };
        let text = labels(&fits(&hud(&calm), &loc, 1920.0, 1080.0));
        assert!(!text.iter().any(|t| t.starts_with("Combo")), "{text:?}");
    }

    #[test]
    fn exhaustion_is_announced() {
        let loc = localization();
        let tired = HudData {
            exhausted: true,
            ..hud_data()
        };
        let text = labels(&fits(&hud(&tired), &loc, 1920.0, 1080.0));
        assert!(text.contains(&"Out of breath!".to_string()), "{text:?}");
    }

    #[test]
    fn the_end_screen_reports_a_record_or_the_best_score() {
        let loc = localization();
        let mut end = EndData {
            won: false,
            gems: 5,
            total: 24,
            score: 700,
            best: 1500,
            new_record: false,
        };
        let text = labels(&fits(&end_screen(&end), &loc, 1920.0, 1080.0));
        assert!(
            text.contains(&"The lantern went out".to_string()),
            "{text:?}"
        );
        assert!(text.contains(&"Best score: 1500".to_string()), "{text:?}");
        end.won = true;
        end.new_record = true;
        let text = labels(&fits(&end_screen(&end), &loc, 1920.0, 1080.0));
        assert!(text.contains(&"New best score!".to_string()), "{text:?}");
    }

    #[test]
    fn the_menu_shows_the_best_score_only_when_there_is_one() {
        let loc = localization();
        let with = labels(&fits(&main_menu(900), &loc, 1920.0, 1080.0));
        assert!(with.contains(&"Best score: 900".to_string()), "{with:?}");
        let without = labels(&fits(&main_menu(0), &loc, 1920.0, 1080.0));
        assert!(!without.iter().any(|t| t.starts_with("Best score")));
    }
}
