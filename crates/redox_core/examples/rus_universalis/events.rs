//! Narrative events and historical decisions (1380 CE).

use super::sim::Simulation;
use super::types::*;

pub struct EventManager {
    pub pending_event: Option<NarrativeEvent>,
    triggered: Vec<&'static str>,
}

impl EventManager {
    pub fn new() -> Self {
        Self {
            pending_event: None,
            triggered: Vec::new(),
        }
    }

    /// Check if conditions for historical events are met.
    pub fn check_events(&mut self, sim: &mut Simulation) {
        if self.pending_event.is_some() {
            return;
        }

        // 1. Initial event: Mamai demands tribute in August 1380
        if !self.triggered.contains(&"mamai_demand") && sim.date.month >= 8 {
            self.triggered.push("mamai_demand");
            self.pending_event = Some(NarrativeEvent {
                id: "mamai_demand",
                title_ru: "Требование темника Мамая",
                description_ru: "В Москву прибыло ордынское посольство. Беклярбек Мамай требует выплаты дани в удвоенном размере, грозя в случае отказа двинуть все кочевья на разорение Русской земли. На южных рубежах уже замечены передовые разъезды кочевников.",
                options: vec![
                    EventOption {
                        text_ru: "«Златом и серебром откупиться от нечестивых»",
                        effect_summary_ru: "–35 рублей из казны. Дань выплачена, война отложена.",
                        action_id: 1,
                    },
                    EventOption {
                        text_ru: "«Лучше нам смерть славная, нежели жизнь постыдная!»",
                        effect_summary_ru: "+25 престижа, +1 стабильность. Золотая Орда объявляет войну Москве!",
                        action_id: 2,
                    },
                ],
            });
            sim.paused = true; // Pause to let player make decision
            return;
        }

        // 2. Sergeiy Radonezhsky blessing in late August
        if !self.triggered.contains(&"radonezh_blessing")
            && sim.date.month >= 8
            && sim.date.day >= 18
        {
            self.triggered.push("radonezh_blessing");
            self.pending_event = Some(NarrativeEvent {
                id: "radonezh_blessing",
                title_ru: "Благословение преподобного Сергия Радонежского",
                description_ru: "Великий князь Дмитрий посетил Троицкий монастырь. Игумен Сергий благословил всё русское воинство на брань за веру и землю Русскую и отпустил с князем двух иноков-богатырей: Александра Пересвета и Андрея Ослябю.",
                options: vec![EventOption {
                    text_ru: "«С нами Бог и правда!»",
                    effect_summary_ru: "+2 стабильности. Моральный дух всех московских армий возрастает!",
                    action_id: 3,
                }],
            });
            sim.paused = true;
            return;
        }

        // 3. Tver neutrality or alliance
        if !self.triggered.contains(&"tver_decision") && sim.date.month >= 9 {
            self.triggered.push("tver_decision");
            self.pending_event = Some(NarrativeEvent {
                id: "tver_decision",
                title_ru: "Вести из Твери",
                description_ru: "Великий князь Тверской Михаил Александрович колеблется. Послы просят подтвердить мирный договор 1375 года и не поддерживать войско темника Мамая.",
                options: vec![EventOption {
                    text_ru: "Подтвердить крестное целование с Тверью",
                    effect_summary_ru: "Тверь сохраняет нейтралитет и не открывает второй фронт.",
                    action_id: 4,
                }],
            });
            sim.paused = true;
        }
    }

    /// Apply player choice to the simulation.
    pub fn select_option(&mut self, action_id: u8, sim: &mut Simulation) {
        match action_id {
            1 => {
                // Pay tribute
                if let Some(mos) = sim.countries.get_mut(&CountryTag::Moscow) {
                    mos.treasury -= 35.0;
                    mos.horde_loyalty = (mos.horde_loyalty + 20.0).min(100.0);
                }
                sim.battle_logs
                    .push("Москва выплатила дань послу Сарыхоже. Хан временно усмирён.".into());
            }
            2 => {
                // Refuse and go to war!
                if let Some(mos) = sim.countries.get_mut(&CountryTag::Moscow) {
                    mos.prestige += 25.0;
                    mos.stability = (mos.stability + 1).min(3);
                }
                sim.declare_war(CountryTag::GoldenHorde, CountryTag::Moscow);
                sim.battle_logs.push(
                    "🛡️ Москва отвергла ханский ультиматум! Русские полки собираются в поход."
                        .into(),
                );
            }
            3 => {
                // Blessing
                if let Some(mos) = sim.countries.get_mut(&CountryTag::Moscow) {
                    mos.stability = (mos.stability + 2).min(3);
                    mos.prestige += 15.0;
                }
                for army in &mut sim.armies {
                    if army.owner == CountryTag::Moscow {
                        army.max_morale += 0.5;
                        army.morale = army.max_morale;
                    }
                }
                sim.battle_logs.push(
                    "✝️ Преподобный Сергий благословил воинов. Боевой дух рати на высоте!".into(),
                );
            }
            4 => {
                // Tver peace
                if let Some(tver) = sim.countries.get_mut(&CountryTag::Tver) {
                    tver.prestige += 5.0;
                }
                sim.battle_logs
                    .push("Тверской князь Михаил присягнул не помогать Мамаю.".into());
            }
            _ => {}
        }
        self.pending_event = None;
    }
}
