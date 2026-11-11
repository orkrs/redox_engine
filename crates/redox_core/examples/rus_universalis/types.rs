//! RedOx Universalis: core data types and simulation domain model.

#![allow(dead_code)]

use redox_math::Vec3;

/// Unique identifier for each province/location.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProvinceId(pub u16);

/// Country / Principality tag.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CountryTag {
    /// Великое княжество Московское
    Moscow,
    /// Новгородская вечевая республика
    Novgorod,
    /// Великое княжество Тверское
    Tver,
    /// Великое княжество Рязанское
    Ryazan,
    /// Суздальско-Нижегородское княжество
    Suzdal,
    /// Великое княжество Литовское
    Lithuania,
    /// Золотая Орда (Улус Джучи)
    GoldenHorde,
    /// Псковская вечевая республика
    Pskov,
}

impl CountryTag {
    pub fn name_ru(&self) -> &'static str {
        match self {
            Self::Moscow => "Великое княжество Московское",
            Self::Novgorod => "Новгородская республика",
            Self::Tver => "Великое княжество Тверское",
            Self::Ryazan => "Великое княжество Рязанское",
            Self::Suzdal => "Нижегородско-Суздальское княжество",
            Self::Lithuania => "Великое княжество Литовское",
            Self::GoldenHorde => "Золотая Орда",
            Self::Pskov => "Псковская республика",
        }
    }

    pub fn ruler_title_ru(&self) -> &'static str {
        match self {
            Self::Moscow => "Великий князь Дмитрий Донской",
            Self::Novgorod => "Посадник и Новгородское Вече",
            Self::Tver => "Великий князь Михаил Тверской",
            Self::Ryazan => "Великий князь Олег Рязанский",
            Self::Suzdal => "Князь Дмитрий Константинович",
            Self::Lithuania => "Великий князь Ягайло Ольгердович",
            Self::GoldenHorde => "Беклярбек Мамай",
            Self::Pskov => "Псковский посадник и Вече",
        }
    }

    /// Primary map color for the country (RGBA linear).
    pub fn map_color(&self) -> [f32; 4] {
        match self {
            Self::Moscow => [0.82, 0.16, 0.16, 1.0],      // Багряный
            Self::Novgorod => [0.15, 0.65, 0.30, 1.0],    // Изумрудно-зеленый
            Self::Tver => [0.20, 0.40, 0.85, 1.0],        // Лазурный
            Self::Ryazan => [0.85, 0.48, 0.15, 1.0],      // Охра / Терракотовый
            Self::Suzdal => [0.18, 0.55, 0.60, 1.0],      // Морская волна
            Self::Lithuania => [0.60, 0.15, 0.45, 1.0],   // Пурпурный
            Self::GoldenHorde => [0.88, 0.72, 0.15, 1.0], // Золотой
            Self::Pskov => [0.25, 0.70, 0.75, 1.0],       // Бирюзовый
        }
    }
}

/// Trade good produced in a province.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum TradeGood {
    /// Хлеб и зерно (базовая пища)
    Grain,
    /// Меха и пушнина (главный экспорт Новгорода)
    Furs,
    /// Воск и мёд (бортничество, свечи, экспорт)
    WaxAndHoney,
    /// Болотное железо и кричное дело (оружие)
    Iron,
    /// Корабельный лес и смола
    Timber,
    /// Соль (солеварни)
    Salt,
    /// Оружие и доспехи кузнецов
    Weapons,
    /// Боевые степные кони
    Warhorses,
}

impl TradeGood {
    pub fn name_ru(&self) -> &'static str {
        match self {
            Self::Grain => "Хлеб",
            Self::Furs => "Меха и пушнина",
            Self::WaxAndHoney => "Мёд и воск",
            Self::Iron => "Железо",
            Self::Timber => "Лес и смола",
            Self::Salt => "Соль",
            Self::Weapons => "Оружие и кольчуги",
            Self::Warhorses => "Боевые кони",
        }
    }

    pub fn base_price(&self) -> f32 {
        match self {
            Self::Grain => 2.0,
            Self::Furs => 8.0,
            Self::WaxAndHoney => 5.0,
            Self::Iron => 6.0,
            Self::Timber => 3.0,
            Self::Salt => 4.5,
            Self::Weapons => 10.0,
            Self::Warhorses => 7.0,
        }
    }

    pub fn color(&self) -> [f32; 4] {
        match self {
            Self::Grain => [0.90, 0.85, 0.40, 1.0],
            Self::Furs => [0.55, 0.35, 0.20, 1.0],
            Self::WaxAndHoney => [0.95, 0.65, 0.10, 1.0],
            Self::Iron => [0.50, 0.52, 0.58, 1.0],
            Self::Timber => [0.25, 0.50, 0.20, 1.0],
            Self::Salt => [0.90, 0.92, 0.95, 1.0],
            Self::Weapons => [0.75, 0.20, 0.20, 1.0],
            Self::Warhorses => [0.80, 0.40, 0.20, 1.0],
        }
    }
}

/// Social estate / class of population.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PopClass {
    /// Смерды / Пашенные крестьяне
    Peasants,
    /// Посадские люди / Ремесленники и купцы
    Burghers,
    /// Монахи / Церковный клир
    Clergy,
    /// Бояре и дети боярские (служилые люди)
    Nobles,
}

impl PopClass {
    pub fn name_ru(&self) -> &'static str {
        match self {
            Self::Peasants => "Крестьяне-смерды",
            Self::Burghers => "Посадские люди",
            Self::Clergy => "Духовенство",
            Self::Nobles => "Бояре и дружина",
        }
    }
}

/// A demographic group within a province.
#[derive(Clone, Debug)]
pub struct PopGroup {
    pub class: PopClass,
    pub count: u32,
    pub satisfaction: f32, // 0.0 .. 1.0
}

/// Static and dynamic data for a single province/location.
#[derive(Clone, Debug)]
pub struct Province {
    pub id: ProvinceId,
    pub name_ru: &'static str,
    pub position: Vec3, // 3D coordinates on the strategic map (Y is elevation/flat)
    pub neighbors: Vec<ProvinceId>,
    pub trade_good: TradeGood,
    pub owner: CountryTag,
    pub controller: CountryTag,
    pub pops: Vec<PopGroup>,
    pub base_tax: f32,
    pub control: f32, // 0.0 .. 1.0 (effective authority from capital)
    pub fort_level: u8,
    pub is_capital: bool,
}

impl Province {
    pub fn total_population(&self) -> u32 {
        self.pops.iter().map(|p| p.count).sum()
    }

    pub fn pop_count(&self, class: PopClass) -> u32 {
        self.pops
            .iter()
            .find(|p| p.class == class)
            .map_or(0, |p| p.count)
    }

    pub fn monthly_production_value(&self) -> f32 {
        let workers = self.pop_count(PopClass::Peasants) + self.pop_count(PopClass::Burghers);
        let production = (workers as f32 / 1000.0) * self.trade_good.base_price() * 0.1;
        production * self.control
    }

    pub fn monthly_tax(&self) -> f32 {
        let peasants_tax = (self.pop_count(PopClass::Peasants) as f32 / 2000.0) * 0.5;
        let burghers_tax = (self.pop_count(PopClass::Burghers) as f32 / 1000.0) * 1.5;
        (self.base_tax + peasants_tax + burghers_tax) * self.control
    }
}

/// Country state in the simulation.
#[derive(Clone, Debug)]
pub struct Country {
    pub tag: CountryTag,
    pub treasury: f32,
    pub manpower: u32,
    pub max_manpower: u32,
    pub stability: i8, // -3 .. +3
    pub prestige: f32,
    pub capital: ProvinceId,
    pub tribute_to_horde: f32, // Ежегодный «Ордынский выход»
    pub horde_loyalty: f32,    // 0.0 .. 100.0
    pub at_war_with: Vec<CountryTag>,
}

/// Military regiment type.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RegimentType {
    /// Посошная пешая рать (крестьянское ополчение)
    Militia,
    /// Кованая боярская конница (дружина)
    BoyarCavalry,
    /// Наемные лучники / пищальники
    Archers,
    /// Степная ордынская конница
    HordeHorseArchers,
}

impl RegimentType {
    pub fn name_ru(&self) -> &'static str {
        match self {
            Self::Militia => "Пешая рать (ополчение)",
            Self::BoyarCavalry => "Боярская кованая конница",
            Self::Archers => "Лучники",
            Self::HordeHorseArchers => "Ордынская конница",
        }
    }

    pub fn attack(&self) -> f32 {
        match self {
            Self::Militia => 2.0,
            Self::BoyarCavalry => 6.0,
            Self::Archers => 3.5,
            Self::HordeHorseArchers => 5.5,
        }
    }

    pub fn defense(&self) -> f32 {
        match self {
            Self::Militia => 1.5,
            Self::BoyarCavalry => 5.0,
            Self::Archers => 2.0,
            Self::HordeHorseArchers => 3.5,
        }
    }

    pub fn monthly_cost(&self) -> f32 {
        match self {
            Self::Militia => 0.1,
            Self::BoyarCavalry => 0.6,
            Self::Archers => 0.3,
            Self::HordeHorseArchers => 0.4,
        }
    }
}

/// Single regiment in an army.
#[derive(Clone, Debug)]
pub struct Regiment {
    pub regiment_type: RegimentType,
    pub soldiers: u32, // max 1000
    pub home_province: ProvinceId,
}

/// An army entity existing on the map.
#[derive(Clone, Debug)]
pub struct Army {
    pub id: u32,
    pub name: String,
    pub owner: CountryTag,
    pub location: ProvinceId,
    pub regiments: Vec<Regiment>,
    pub morale: f32,     // 0.0 .. 1.0 (max 1.0)
    pub max_morale: f32, // e.g. 3.0
    /// Path of provinces being traversed
    pub movement_path: Vec<ProvinceId>,
    /// Progress to next province (0.0 .. 1.0)
    pub movement_progress: f32,
    /// In battle with another army
    pub in_battle: bool,
}

impl Army {
    pub fn total_soldiers(&self) -> u32 {
        self.regiments.iter().map(|r| r.soldiers).sum()
    }

    pub fn monthly_maintenance(&self) -> f32 {
        self.regiments
            .iter()
            .map(|r| r.regiment_type.monthly_cost() * (r.soldiers as f32 / 1000.0))
            .sum()
    }
}

/// Active map visualization mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MapMode {
    /// Политическая карта (цвета княжеств и Орды)
    Political,
    /// Демография (плотность населения)
    Demographics,
    /// Торговые товары и ресурсы
    TradeGoods,
    /// Контроль и автономия (удаленность от столицы)
    Control,
}

impl MapMode {
    pub fn name_ru(&self) -> &'static str {
        match self {
            Self::Political => "Политическая",
            Self::Demographics => "Население",
            Self::TradeGoods => "Товары и промыслы",
            Self::Control => "Контроль и дань",
        }
    }
}

/// Historical or random narrative event with player choices.
#[derive(Clone, Debug)]
pub struct NarrativeEvent {
    pub id: &'static str,
    pub title_ru: &'static str,
    pub description_ru: &'static str,
    pub options: Vec<EventOption>,
}

#[derive(Clone, Debug)]
pub struct EventOption {
    pub text_ru: &'static str,
    pub effect_summary_ru: &'static str,
    pub action_id: u8,
}
