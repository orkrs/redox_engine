//! Simulation engine: time advancement, daily ticks, monthly economy, army movement & battle.

use super::types::*;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct GameDate {
    pub day: u8,
    pub month: u8,
    pub year: u16,
}

impl GameDate {
    pub fn new(day: u8, month: u8, year: u16) -> Self {
        Self { day, month, year }
    }

    pub fn format_ru(&self) -> String {
        let month_name = match self.month {
            1 => "января",
            2 => "февраля",
            3 => "марта",
            4 => "апреля",
            5 => "мая",
            6 => "июня",
            7 => "июля",
            8 => "августа",
            9 => "сентября",
            10 => "октября",
            11 => "ноября",
            12 => "декабря",
            _ => "",
        };
        format!("{} {} {} года", self.day, month_name, self.year)
    }

    /// Advance by one day. Returns `true` if a new month has started.
    pub fn advance_day(&mut self) -> bool {
        self.day += 1;
        let days_in_month = match self.month {
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        if self.day > days_in_month {
            self.day = 1;
            self.month += 1;
            if self.month > 12 {
                self.month = 1;
                self.year += 1;
            }
            true
        } else {
            false
        }
    }
}

/// Simulation coordinator.
pub struct Simulation {
    pub date: GameDate,
    pub paused: bool,
    pub speed: u8, // 1 .. 5
    pub day_accumulator: f32,
    pub provinces: HashMap<ProvinceId, Province>,
    pub countries: HashMap<CountryTag, Country>,
    pub armies: Vec<Army>,
    pub player_country: CountryTag,
    pub next_army_id: u32,
    pub battle_logs: Vec<String>,
}

impl Simulation {
    pub fn new(
        provinces: HashMap<ProvinceId, Province>,
        countries: HashMap<CountryTag, Country>,
        armies: Vec<Army>,
        player_country: CountryTag,
    ) -> Self {
        Self {
            date: GameDate::new(8, 8, 1380), // Месяц до Куликовской битвы!
            paused: true,
            speed: 2,
            day_accumulator: 0.0,
            provinces,
            countries,
            armies,
            player_country,
            next_army_id: 100,
            battle_logs: Vec::new(),
        }
    }

    /// Advance simulation by wall-clock `dt` seconds.
    pub fn update(&mut self, dt: f32) -> bool {
        if self.paused {
            return false;
        }

        // Speed table: 1x = 1.0s/day, 2x = 0.5s/day, 3x = 0.25s/day, 4x = 0.12s/day, 5x = 0.05s/day
        let seconds_per_day = match self.speed {
            1 => 1.0,
            2 => 0.5,
            3 => 0.25,
            4 => 0.12,
            _ => 0.05,
        };

        self.day_accumulator += dt;
        let mut days_ticked = 0;
        let mut new_month = false;

        while self.day_accumulator >= seconds_per_day && days_ticked < 10 {
            self.day_accumulator -= seconds_per_day;
            days_ticked += 1;
            if self.date.advance_day() {
                self.on_monthly_settlement();
                new_month = true;
            }
            self.on_daily_tick();
        }

        new_month
    }

    /// Daily simulation tick: movement and combat.
    fn on_daily_tick(&mut self) {
        // 1. Army Movement
        for army in &mut self.armies {
            if !army.movement_path.is_empty() && !army.in_battle {
                // Takes ~5 days to travel between neighboring provinces
                army.movement_progress += 0.20;
                if army.movement_progress >= 1.0 {
                    army.movement_progress = 0.0;
                    let next_prov = army.movement_path.remove(0);
                    army.location = next_prov;
                }
            }
        }

        // 2. Province Occupation Check
        for army in &self.armies {
            if army.movement_path.is_empty() && !army.in_battle {
                if let Some(prov) = self.provinces.get_mut(&army.location) {
                    let is_hostile = self
                        .countries
                        .get(&army.owner)
                        .map_or(false, |c| c.at_war_with.contains(&prov.controller));
                    if is_hostile {
                        prov.controller = army.owner;
                    }
                }
            }
        }

        // 3. Combat resolution between hostile armies in the same province
        let mut battles: Vec<(usize, usize)> = Vec::new();
        for i in 0..self.armies.len() {
            for j in (i + 1)..self.armies.len() {
                let a1 = &self.armies[i];
                let a2 = &self.armies[j];
                if a1.location == a2.location {
                    let at_war = self
                        .countries
                        .get(&a1.owner)
                        .map_or(false, |c| c.at_war_with.contains(&a2.owner));
                    if at_war && a1.total_soldiers() > 0 && a2.total_soldiers() > 0 {
                        battles.push((i, j));
                    }
                }
            }
        }

        for (i, j) in battles {
            self.resolve_combat_round(i, j);
        }

        // Remove destroyed armies
        self.armies.retain(|a| a.total_soldiers() > 0);
    }

    fn resolve_combat_round(&mut self, i: usize, j: usize) {
        let (a1, a2) = if i < j {
            let (left, right) = self.armies.split_at_mut(j);
            (&mut left[i], &mut right[0])
        } else {
            let (left, right) = self.armies.split_at_mut(i);
            (&mut right[0], &mut left[j])
        };

        a1.in_battle = true;
        a2.in_battle = true;

        let str1 = a1.total_soldiers() as f32;
        let str2 = a2.total_soldiers() as f32;

        if str1 == 0.0 || str2 == 0.0 {
            a1.in_battle = false;
            a2.in_battle = false;
            return;
        }

        // Daily casualties (0.5% .. 1.5% of opposing force)
        let dmg_to_a2 = (str1 * 0.015).max(10.0) as u32;
        let dmg_to_a1 = (str2 * 0.015).max(10.0) as u32;

        Self::apply_casualties(a1, dmg_to_a1);
        Self::apply_casualties(a2, dmg_to_a2);

        // Morale loss
        a1.morale = (a1.morale - (dmg_to_a1 as f32 / str1.max(1.0)) * 2.0).max(0.0);
        a2.morale = (a2.morale - (dmg_to_a2 as f32 / str2.max(1.0)) * 2.0).max(0.0);

        // Check rout
        if a1.morale <= 0.05 || a1.total_soldiers() == 0 {
            a1.in_battle = false;
            a2.in_battle = false;
            self.battle_logs.push(format!(
                "Битва завершена! {} отступила перед {}.",
                a1.name, a2.name
            ));
        } else if a2.morale <= 0.05 || a2.total_soldiers() == 0 {
            a1.in_battle = false;
            a2.in_battle = false;
            self.battle_logs.push(format!(
                "Битва завершена! {} одержала победу над {}.",
                a1.name, a2.name
            ));
        }
    }

    fn apply_casualties(army: &mut Army, mut casualties: u32) {
        for reg in &mut army.regiments {
            if casualties == 0 {
                break;
            }
            let loss = reg.soldiers.min(casualties);
            reg.soldiers -= loss;
            casualties -= loss;
        }
    }

    /// Monthly economy, taxes, tribute and pop demographic changes.
    pub fn on_monthly_settlement(&mut self) {
        // 1. Calculate and distribute income for each country
        let mut incomes: HashMap<CountryTag, f32> = HashMap::new();
        for prov in self.provinces.values() {
            let tax = prov.monthly_tax();
            let prod = prov.monthly_production_value();
            *incomes.entry(prov.controller).or_insert(0.0) += tax + prod;
        }

        for (tag, country) in &mut self.countries {
            let income = incomes.get(tag).copied().unwrap_or(0.0);
            country.treasury += income;

            // Pay army maintenance
            let maintenance: f32 = self
                .armies
                .iter()
                .filter(|a| a.owner == *tag)
                .map(|a| a.monthly_maintenance())
                .sum();
            country.treasury -= maintenance;

            // Horde tribute payment (annually in September, or monthly fraction)
            if country.tribute_to_horde > 0.0 && self.date.month == 9 {
                if country.treasury >= country.tribute_to_horde {
                    country.treasury -= country.tribute_to_horde;
                    country.horde_loyalty = (country.horde_loyalty + 5.0).min(100.0);
                } else {
                    country.horde_loyalty = (country.horde_loyalty - 15.0).max(0.0);
                }
            }

            // Manpower growth (1% of max per month)
            country.manpower =
                (country.manpower + (country.max_manpower / 100)).min(country.max_manpower);
        }

        // 2. Recover army morale outside battles
        for army in &mut self.armies {
            if !army.in_battle && army.morale < 1.0 {
                army.morale = (army.morale + 0.15).min(1.0);
            }
        }
    }

    /// Order army to march to an adjacent province.
    pub fn move_army(&mut self, army_id: u32, target_prov: ProvinceId) -> bool {
        let Some(army) = self.armies.iter_mut().find(|a| a.id == army_id) else {
            return false;
        };
        let current = army.movement_path.last().copied().unwrap_or(army.location);
        if let Some(prov) = self.provinces.get(&current) {
            if prov.neighbors.contains(&target_prov) {
                army.movement_path.push(target_prov);
                return true;
            }
        }
        false
    }

    /// Recruit a peasant militia regiment in a province (EU5 Levies mechanic).
    pub fn raise_levy(&mut self, prov_id: ProvinceId) -> Result<(), &'static str> {
        let Some(prov) = self.provinces.get_mut(&prov_id) else {
            return Err("Локация не найдена");
        };

        let peasant_pop = prov.pops.iter_mut().find(|p| p.class == PopClass::Peasants);
        let Some(peasants) = peasant_pop else {
            return Err("В провинции нет крестьян");
        };

        if peasants.count < 1500 {
            return Err("Недостаточно крестьян для сбора ополчения!");
        }

        // Deduct 1000 peasants from the real population
        peasants.count -= 1000;
        let owner = prov.owner;

        // Find existing army in this province or create a new host
        let army = self
            .armies
            .iter_mut()
            .find(|a| a.location == prov_id && a.owner == owner && !a.in_battle);

        if let Some(existing) = army {
            existing.regiments.push(Regiment {
                regiment_type: RegimentType::Militia,
                soldiers: 1000,
                home_province: prov_id,
            });
        } else {
            let id = self.next_army_id;
            self.next_army_id += 1;
            self.armies.push(Army {
                id,
                name: format!("Посошная рать ({})", prov.name_ru),
                owner,
                location: prov_id,
                regiments: vec![Regiment {
                    regiment_type: RegimentType::Militia,
                    soldiers: 1000,
                    home_province: prov_id,
                }],
                morale: 0.8,
                max_morale: 2.5,
                movement_path: vec![],
                movement_progress: 0.0,
                in_battle: false,
            });
        }

        Ok(())
    }

    /// Declare war between two countries.
    pub fn declare_war(&mut self, attacker: CountryTag, defender: CountryTag) {
        if let Some(c) = self.countries.get_mut(&attacker) {
            if !c.at_war_with.contains(&defender) {
                c.at_war_with.push(defender);
            }
        }
        if let Some(c) = self.countries.get_mut(&defender) {
            if !c.at_war_with.contains(&attacker) {
                c.at_war_with.push(attacker);
            }
        }
        self.battle_logs.push(format!(
            "⚔️ ВОЙНА! {} объявляет войну {}!",
            attacker.name_ru(),
            defender.name_ru()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::build_historical_scenario;

    #[test]
    fn scenario_initialization() {
        let (provinces, countries, armies) = build_historical_scenario();
        assert_eq!(provinces.len(), 24);
        assert_eq!(countries.len(), 8);
        assert!(armies.len() >= 4);

        let moscow = provinces.get(&ProvinceId(1)).unwrap();
        assert_eq!(moscow.name_ru, "Москва");
        assert_eq!(moscow.owner, CountryTag::Moscow);
        assert!(moscow.total_population() > 50000);
    }

    #[test]
    fn levy_recruitment_reduces_peasants() {
        let (provinces, countries, armies) = build_historical_scenario();
        let mut sim = Simulation::new(provinces, countries, armies, CountryTag::Moscow);

        let initial_peasants = sim
            .provinces
            .get(&ProvinceId(1))
            .unwrap()
            .pop_count(PopClass::Peasants);
        let initial_armies = sim.armies.len();

        let res = sim.raise_levy(ProvinceId(1));
        assert!(res.is_ok());

        let new_peasants = sim
            .provinces
            .get(&ProvinceId(1))
            .unwrap()
            .pop_count(PopClass::Peasants);
        assert_eq!(new_peasants, initial_peasants - 1000);
        assert!(sim.armies.len() >= initial_armies);
    }

    #[test]
    fn army_movement_validation() {
        let (provinces, countries, armies) = build_historical_scenario();
        let mut sim = Simulation::new(provinces, countries, armies, CountryTag::Moscow);

        // Army 1 is in Moscow (Prov 1). Kolomna (Prov 2) is neighbor.
        let army_id = sim.armies[0].id;
        let moved = sim.move_army(army_id, ProvinceId(2));
        assert!(moved);

        // Sarai (Prov 24) is NOT a direct neighbor of Moscow
        let invalid_move = sim.move_army(army_id, ProvinceId(24));
        assert!(!invalid_move);
    }

    #[test]
    fn time_advancement_and_monthly_tick() {
        let (provinces, countries, armies) = build_historical_scenario();
        let mut sim = Simulation::new(provinces, countries, armies, CountryTag::Moscow);
        sim.paused = false;

        let initial_treasury = sim.countries.get(&CountryTag::Moscow).unwrap().treasury;

        // Advance by 30 days (simulate 30.0 seconds at speed 1)
        for _ in 0..30 {
            sim.update(1.0);
        }

        let new_treasury = sim.countries.get(&CountryTag::Moscow).unwrap().treasury;
        // Moscow earns monthly tax and production
        assert!(new_treasury != initial_treasury);
    }
}
