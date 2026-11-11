//! RedOx Universalis User Interface using egui.

use super::events::EventManager;
use super::sim::Simulation;
use super::types::*;
use egui::{Align2, Color32, RichText, Vec2};

pub struct GameUi {
    pub map_mode: MapMode,
    pub selected_province: Option<ProvinceId>,
    pub selected_army: Option<u32>,
    pub show_country_window: bool,
    pub show_help_window: bool,
}

impl GameUi {
    pub fn new() -> Self {
        Self {
            map_mode: MapMode::Political,
            selected_province: Some(ProvinceId(1)), // Start with Moscow selected
            selected_army: None,
            show_country_window: false,
            show_help_window: true,
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context, sim: &mut Simulation, events: &mut EventManager) {
        self.draw_top_bar(ctx, sim);
        self.draw_map_modes_bar(ctx);
        self.draw_province_panel(ctx, sim);
        self.draw_army_panel(ctx, sim);
        self.draw_event_dialog(ctx, sim, events);
        self.draw_battle_log(ctx, sim);

        if self.show_country_window {
            self.draw_country_window(ctx, sim);
        }

        if self.show_help_window {
            self.draw_help_window(ctx);
        }
    }

    fn draw_top_bar(&mut self, ctx: &egui::Context, sim: &mut Simulation) {
        egui::Window::new("Государев престол: Русь 1380")
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 10.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let player = sim.countries.get(&sim.player_country).cloned();
                    if let Some(c) = player {
                        let col = c.tag.map_color();
                        let color32 = Color32::from_rgb(
                            (col[0] * 255.0) as u8,
                            (col[1] * 255.0) as u8,
                            (col[2] * 255.0) as u8,
                        );

                        ui.label(RichText::new("👑").size(18.0));
                        if ui
                            .button(
                                RichText::new(c.tag.name_ru())
                                    .color(color32)
                                    .strong()
                                    .size(14.0),
                            )
                            .clicked()
                        {
                            self.show_country_window = !self.show_country_window;
                        }

                        ui.separator();

                        ui.label(RichText::new(format!("💰 {:.1} руб", c.treasury)).strong());
                        ui.label(RichText::new(format!("🛡️ {} ратников", c.manpower)));
                        ui.label(RichText::new(format!("⚖️ Стаб: {:+}", c.stability)));
                        ui.label(RichText::new(format!("🦅 Престиж: {:.0}", c.prestige)));

                        if c.tribute_to_horde > 0.0 {
                            ui.separator();
                            let horde_col = if c.horde_loyalty < 40.0 {
                                Color32::RED
                            } else {
                                Color32::KHAKI
                            };
                            ui.label(
                                RichText::new(format!(
                                    "🐎 Дань: {:.0} руб/год ({:.0}%)",
                                    c.tribute_to_horde, c.horde_loyalty
                                ))
                                .color(horde_col),
                            );
                        }
                    }

                    ui.separator();

                    let pause_btn = if sim.paused {
                        RichText::new("⏸ ПАУЗА (Пробел)")
                            .color(Color32::LIGHT_RED)
                            .strong()
                    } else {
                        RichText::new("▶ ИГРА (Пробел)")
                            .color(Color32::LIGHT_GREEN)
                            .strong()
                    };
                    if ui.button(pause_btn).clicked() {
                        sim.paused = !sim.paused;
                    }

                    let speed_txt = match sim.speed {
                        1 => "1x",
                        2 => "2x",
                        3 => "3x",
                        4 => "4x",
                        _ => "5x",
                    };
                    if ui.button(RichText::new(speed_txt).strong()).clicked() {
                        sim.speed = if sim.speed >= 5 { 1 } else { sim.speed + 1 };
                    }

                    ui.separator();
                    ui.label(
                        RichText::new(format!("📅 {}", sim.date.format_ru()))
                            .strong()
                            .size(13.0),
                    );

                    ui.separator();
                    if ui.button(RichText::new("❓").size(13.0)).clicked() {
                        self.show_help_window = !self.show_help_window;
                    }
                });
            });
    }

    fn draw_map_modes_bar(&mut self, ctx: &egui::Context) {
        egui::Window::new("Режимы карты")
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-15.0, -15.0))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let modes = [
                        MapMode::Political,
                        MapMode::Demographics,
                        MapMode::TradeGoods,
                        MapMode::Control,
                    ];
                    for mode in modes {
                        let selected = self.map_mode == mode;
                        if ui.selectable_label(selected, mode.name_ru()).clicked() {
                            self.map_mode = mode;
                        }
                    }
                });
            });
    }

    fn draw_province_panel(&mut self, ctx: &egui::Context, sim: &mut Simulation) {
        let Some(prov_id) = self.selected_province else {
            return;
        };
        let Some(prov) = sim.provinces.get(&prov_id).cloned() else {
            return;
        };

        egui::Window::new(format!("Локация: {}", prov.name_ru))
            .anchor(Align2::LEFT_TOP, Vec2::new(15.0, 15.0))
            .default_width(320.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Владелец: {}", prov.owner.name_ru())).strong());
                });
                if prov.controller != prov.owner {
                    ui.label(
                        RichText::new(format!("⚠️ Оккупировано: {}", prov.controller.name_ru()))
                            .color(Color32::RED),
                    );
                }

                ui.separator();

                ui.heading("Население (POPs)");
                let total = prov.total_population();
                ui.label(format!("Всего душ: {} человек", total));

                for pop in &prov.pops {
                    let icon = match pop.class {
                        PopClass::Peasants => "🌾",
                        PopClass::Burghers => "⚒️",
                        PopClass::Clergy => "⛪",
                        PopClass::Nobles => "⚔️",
                    };
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "{} {}: {} чел.",
                            icon,
                            pop.class.name_ru(),
                            pop.count
                        ));
                        ui.label(format!(
                            "(Удовлетворенность: {:.0}%)",
                            pop.satisfaction * 100.0
                        ));
                    });
                }

                ui.separator();

                ui.heading("Экономика и ресурсы");
                ui.label(format!(
                    "Товар: {} (Баз. цена: {:.1})",
                    prov.trade_good.name_ru(),
                    prov.trade_good.base_price()
                ));
                ui.label(format!(
                    "Доход от ремесла: +{:.2} руб/мес",
                    prov.monthly_production_value()
                ));
                ui.label(format!(
                    "Налоги в казну: +{:.2} руб/мес",
                    prov.monthly_tax()
                ));
                ui.label(format!(
                    "Эффективный контроль: {:.0}%",
                    prov.control * 100.0
                ));

                if prov.fort_level > 0 {
                    ui.label(format!("🏰 Уровень укреплений: {}", prov.fort_level));
                }

                ui.separator();

                // Raise levies button (EU5 mechanic: recruits peasants into militia)
                if prov.owner == sim.player_country {
                    let peasants = prov.pop_count(PopClass::Peasants);
                    let can_raise = peasants >= 1500;
                    if ui
                        .add_enabled(
                            can_raise,
                            egui::Button::new(
                                RichText::new("⚔️ Собрать рать (1000 смердов)").strong(),
                            ),
                        )
                        .clicked()
                    {
                        let _ = sim.raise_levy(prov_id);
                    }
                    if !can_raise {
                        ui.small("Недостаточно крестьян для сбора ополчения");
                    }
                }
            });
    }

    fn draw_army_panel(&mut self, ctx: &egui::Context, sim: &mut Simulation) {
        let Some(army_id) = self.selected_army else {
            return;
        };
        let Some(army) = sim.armies.iter().find(|a| a.id == army_id).cloned() else {
            return;
        };

        egui::Window::new(format!("Войско: {}", army.name))
            .anchor(Align2::LEFT_BOTTOM, Vec2::new(15.0, -15.0))
            .default_width(320.0)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("Владелец: {}", army.owner.name_ru())).strong());
                let loc_name = sim
                    .provinces
                    .get(&army.location)
                    .map_or("Неизвестно", |p| p.name_ru);
                ui.label(format!("Местоположение: {}", loc_name));

                ui.separator();

                ui.label(
                    RichText::new(format!("Численность: {} воинов", army.total_soldiers()))
                        .strong(),
                );
                ui.label(format!("Боевой дух: {:.0}%", army.morale * 100.0));
                ui.label(format!(
                    "Содержание: {:.2} руб/мес",
                    army.monthly_maintenance()
                ));

                if army.in_battle {
                    ui.label(
                        RichText::new("⚔️ ИДЁТ СРАЖЕНИЕ!")
                            .color(Color32::RED)
                            .strong(),
                    );
                } else if !army.movement_path.is_empty() {
                    let dest = army
                        .movement_path
                        .last()
                        .and_then(|id| sim.provinces.get(id))
                        .map_or("", |p| p.name_ru);
                    ui.label(format!(
                        "Марш в: {} ({:.0}%)",
                        dest,
                        army.movement_progress * 100.0
                    ));
                }

                ui.separator();
                ui.small("Полки в войске:");
                for reg in &army.regiments {
                    ui.label(format!(
                        "• {}: {} чел.",
                        reg.regiment_type.name_ru(),
                        reg.soldiers
                    ));
                }

                ui.separator();
                ui.small("💡 Правый клик по соседней провинции отправляет рать в поход.");
            });
    }

    fn draw_event_dialog(
        &mut self,
        ctx: &egui::Context,
        sim: &mut Simulation,
        events: &mut EventManager,
    ) {
        let Some(event) = events.pending_event.clone() else {
            return;
        };

        egui::Window::new(RichText::new(event.title_ru).strong().size(18.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(450.0)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(event.description_ru).size(14.0));
                ui.separator();

                for opt in &event.options {
                    ui.vertical(|ui| {
                        if ui
                            .button(RichText::new(opt.text_ru).strong().size(14.0))
                            .clicked()
                        {
                            events.select_option(opt.action_id, sim);
                        }
                        ui.label(
                            RichText::new(opt.effect_summary_ru)
                                .color(Color32::LIGHT_GRAY)
                                .small(),
                        );
                        ui.add_space(4.0);
                    });
                }
            });
    }

    fn draw_battle_log(&mut self, ctx: &egui::Context, sim: &mut Simulation) {
        if sim.battle_logs.is_empty() {
            return;
        }

        egui::Window::new("Летопись событий")
            .anchor(Align2::RIGHT_TOP, Vec2::new(-15.0, 45.0))
            .default_width(320.0)
            .default_height(160.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for log in sim.battle_logs.iter().rev().take(15) {
                        ui.label(RichText::new(log).size(12.0));
                    }
                });
            });
    }

    fn draw_country_window(&mut self, ctx: &egui::Context, sim: &mut Simulation) {
        let Some(c) = sim.countries.get(&sim.player_country).cloned() else {
            return;
        };

        egui::Window::new(format!("Государство: {}", c.tag.name_ru()))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(420.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading(c.tag.ruler_title_ru());
                ui.separator();

                ui.label(format!("Казна: {:.1} рублей", c.treasury));
                ui.label(format!(
                    "Рекрутский запас: {} из {}",
                    c.manpower, c.max_manpower
                ));
                ui.label(format!("Стабильность: {:+}", c.stability));
                ui.label(format!("Престиж государства: {:.0}", c.prestige));

                ui.separator();
                ui.heading("Дипломатия и войны");
                if c.at_war_with.is_empty() {
                    ui.label("В мире со всеми государствами.");
                } else {
                    for enemy in &c.at_war_with {
                        ui.label(
                            RichText::new(format!("⚔️ Состояние войны с: {}", enemy.name_ru()))
                                .color(Color32::RED)
                                .strong(),
                        );
                    }
                }

                if c.tag == CountryTag::Moscow && !c.at_war_with.contains(&CountryTag::GoldenHorde)
                {
                    ui.add_space(8.0);
                    if ui
                        .button(
                            RichText::new("⚔️ Объявить войну Золотой Орде!")
                                .color(Color32::LIGHT_RED)
                                .strong(),
                        )
                        .clicked()
                    {
                        sim.declare_war(CountryTag::Moscow, CountryTag::GoldenHorde);
                    }
                }

                ui.separator();
                if ui.button("Закрыть").clicked() {
                    self.show_country_window = false;
                }
            });
    }

    fn draw_help_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("Добро пожаловать в RedOx Universalis: 1380")
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(480.0)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Лето 1380 года. Русь и Орда");
                ui.label("Вы управляете Великим княжеством Московским под началом князя Дмитрия Донского. С юга приближаются орды темника Мамая. Ваша задача — собрать русские полки, удержать экономику и победить в грядущей сече.");
                ui.separator();

                ui.heading("Управление:");
                ui.label("• WASD / Стрелки / Зажатая правая кнопка: перемещение камеры по карте");
                ui.label("• Колёсико мыши: приближение и отдаление (зум)");
                ui.label("• Левый клик по провинции: выбор провинции и просмотр населения (POPs)");
                ui.label("• Левый клик по круглому маркеру войска: выбор рати");
                ui.label("• Правый клик по соседней провинции: приказ войску выступать в поход");
                ui.label("• Пробел: активная пауза");
                ui.label("• 1 .. 5: регулировка скорости времени");
                ui.separator();

                if ui.button(RichText::new("Начать игру!").strong().size(15.0)).clicked() {
                    self.show_help_window = false;
                }
            });
    }
}
