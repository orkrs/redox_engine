//! Historical 1380 CE scenario setup for RedOx Universalis.

use super::types::*;
use redox_math::Vec3;
use std::collections::HashMap;

pub const PROV_MOSCOW: ProvinceId = ProvinceId(1);
pub const PROV_KOLOMNA: ProvinceId = ProvinceId(2);
pub const PROV_MOZHAYSK: ProvinceId = ProvinceId(3);
pub const PROV_DMITROV: ProvinceId = ProvinceId(4);
pub const PROV_VLADIMIR: ProvinceId = ProvinceId(5);
pub const PROV_KOSTROMA: ProvinceId = ProvinceId(6);
pub const PROV_TVER: ProvinceId = ProvinceId(7);
pub const PROV_KASHIN: ProvinceId = ProvinceId(8);
pub const PROV_RZHEV: ProvinceId = ProvinceId(9);
pub const PROV_NOVGOROD: ProvinceId = ProvinceId(10);
pub const PROV_LADOGA: ProvinceId = ProvinceId(11);
pub const PROV_TORZHOK: ProvinceId = ProvinceId(12);
pub const PROV_VOLOGDA: ProvinceId = ProvinceId(13);
pub const PROV_BELOOZERO: ProvinceId = ProvinceId(14);
pub const PROV_PSKOV: ProvinceId = ProvinceId(15);
pub const PROV_RYAZAN: ProvinceId = ProvinceId(16);
pub const PROV_PRONSK: ProvinceId = ProvinceId(17);
pub const PROV_NIZHNY_NOVGOROD: ProvinceId = ProvinceId(18);
pub const PROV_SUZDAL: ProvinceId = ProvinceId(19);
pub const PROV_MUROM: ProvinceId = ProvinceId(20);
pub const PROV_SMOLENSK: ProvinceId = ProvinceId(21);
pub const PROV_KULIKOVO: ProvinceId = ProvinceId(22);
pub const PROV_BULGAR: ProvinceId = ProvinceId(23);
pub const PROV_SARAI: ProvinceId = ProvinceId(24);

pub fn build_historical_scenario() -> (
    HashMap<ProvinceId, Province>,
    HashMap<CountryTag, Country>,
    Vec<Army>,
) {
    let mut provinces = HashMap::new();
    let mut countries = HashMap::new();
    let mut armies = Vec::new();

    // ── Helper to build POP list ──────────────────────────────────────────────
    fn pops(peasants: u32, burghers: u32, clergy: u32, nobles: u32) -> Vec<PopGroup> {
        vec![
            PopGroup {
                class: PopClass::Peasants,
                count: peasants,
                satisfaction: 0.85,
            },
            PopGroup {
                class: PopClass::Burghers,
                count: burghers,
                satisfaction: 0.80,
            },
            PopGroup {
                class: PopClass::Clergy,
                count: clergy,
                satisfaction: 0.90,
            },
            PopGroup {
                class: PopClass::Nobles,
                count: nobles,
                satisfaction: 0.75,
            },
        ]
    }

    // ── Provinces (X: West(-) to East(+), Z: South(-) to North(+)) ────────────
    let prov_list = vec![
        // Московское княжество
        Province {
            id: PROV_MOSCOW,
            name_ru: "Москва",
            position: Vec3::new(0.0, 0.0, 0.0),
            neighbors: vec![
                PROV_MOZHAYSK,
                PROV_DMITROV,
                PROV_KOLOMNA,
                PROV_VLADIMIR,
                PROV_TVER,
            ],
            trade_good: TradeGood::Grain,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(35000, 12000, 3000, 2500),
            base_tax: 6.0,
            control: 1.0,
            fort_level: 2, // Белокаменный Кремль Дмитрия Донского (1367)
            is_capital: true,
        },
        Province {
            id: PROV_KOLOMNA,
            name_ru: "Коломна",
            position: Vec3::new(1.0, 0.0, -1.3),
            neighbors: vec![PROV_MOSCOW, PROV_RYAZAN, PROV_VLADIMIR, PROV_KULIKOVO],
            trade_good: TradeGood::Weapons,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(18000, 4500, 1000, 800),
            base_tax: 3.5,
            control: 0.92,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_MOZHAYSK,
            name_ru: "Можайск",
            position: Vec3::new(-1.6, 0.0, -0.3),
            neighbors: vec![PROV_MOSCOW, PROV_RZHEV, PROV_SMOLENSK],
            trade_good: TradeGood::Timber,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(14000, 2500, 700, 600),
            base_tax: 2.5,
            control: 0.88,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_DMITROV,
            name_ru: "Дмитров",
            position: Vec3::new(-0.2, 0.0, 1.4),
            neighbors: vec![PROV_MOSCOW, PROV_KASHIN, PROV_VLADIMIR, PROV_TVER],
            trade_good: TradeGood::Grain,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(16000, 3200, 900, 700),
            base_tax: 2.8,
            control: 0.90,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_VLADIMIR,
            name_ru: "Владимир",
            position: Vec3::new(2.4, 0.0, 0.5),
            neighbors: vec![
                PROV_MOSCOW,
                PROV_SUZDAL,
                PROV_KOLOMNA,
                PROV_MUROM,
                PROV_DMITROV,
            ],
            trade_good: TradeGood::Grain,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(22000, 5000, 2000, 1200),
            base_tax: 4.5,
            control: 0.85,
            fort_level: 2,
            is_capital: false,
        },
        Province {
            id: PROV_KOSTROMA,
            name_ru: "Кострома",
            position: Vec3::new(2.8, 0.0, 2.6),
            neighbors: vec![PROV_SUZDAL, PROV_VOLOGDA, PROV_NIZHNY_NOVGOROD],
            trade_good: TradeGood::WaxAndHoney,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(15000, 3000, 800, 600),
            base_tax: 2.6,
            control: 0.78,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_BELOOZERO,
            name_ru: "Белоозеро",
            position: Vec3::new(0.0, 0.0, 4.8),
            neighbors: vec![PROV_VOLOGDA, PROV_DMITROV, PROV_NOVGOROD],
            trade_good: TradeGood::Furs,
            owner: CountryTag::Moscow,
            controller: CountryTag::Moscow,
            pops: pops(12000, 2000, 1100, 500),
            base_tax: 2.2,
            control: 0.72,
            fort_level: 1,
            is_capital: false,
        },
        // Тверское княжество
        Province {
            id: PROV_TVER,
            name_ru: "Тверь",
            position: Vec3::new(-1.8, 0.0, 1.8),
            neighbors: vec![
                PROV_MOSCOW,
                PROV_KASHIN,
                PROV_RZHEV,
                PROV_TORZHOK,
                PROV_DMITROV,
            ],
            trade_good: TradeGood::Weapons,
            owner: CountryTag::Tver,
            controller: CountryTag::Tver,
            pops: pops(28000, 8500, 2200, 1800),
            base_tax: 5.0,
            control: 1.0,
            fort_level: 2,
            is_capital: true,
        },
        Province {
            id: PROV_KASHIN,
            name_ru: "Кашин",
            position: Vec3::new(-0.8, 0.0, 2.5),
            neighbors: vec![PROV_TVER, PROV_DMITROV, PROV_VOLOGDA],
            trade_good: TradeGood::Salt,
            owner: CountryTag::Tver,
            controller: CountryTag::Tver,
            pops: pops(14000, 2800, 800, 600),
            base_tax: 2.5,
            control: 0.88,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_RZHEV,
            name_ru: "Ржев",
            position: Vec3::new(-2.8, 0.0, 0.8),
            neighbors: vec![PROV_TVER, PROV_MOZHAYSK, PROV_SMOLENSK, PROV_TORZHOK],
            trade_good: TradeGood::Timber,
            owner: CountryTag::Tver,
            controller: CountryTag::Tver,
            pops: pops(13000, 2200, 600, 500),
            base_tax: 2.2,
            control: 0.85,
            fort_level: 1,
            is_capital: false,
        },
        // Новгородская республика
        Province {
            id: PROV_NOVGOROD,
            name_ru: "Новгород Великий",
            position: Vec3::new(-4.2, 0.0, 4.0),
            neighbors: vec![PROV_LADOGA, PROV_TORZHOK, PROV_PSKOV, PROV_BELOOZERO],
            trade_good: TradeGood::Furs,
            owner: CountryTag::Novgorod,
            controller: CountryTag::Novgorod,
            pops: pops(42000, 22000, 4000, 2000), // Богатейший торговый город
            base_tax: 8.5,
            control: 1.0,
            fort_level: 2,
            is_capital: true,
        },
        Province {
            id: PROV_LADOGA,
            name_ru: "Ладога",
            position: Vec3::new(-3.8, 0.0, 5.5),
            neighbors: vec![PROV_NOVGOROD, PROV_BELOOZERO],
            trade_good: TradeGood::Timber,
            owner: CountryTag::Novgorod,
            controller: CountryTag::Novgorod,
            pops: pops(15000, 3500, 800, 500),
            base_tax: 3.0,
            control: 0.90,
            fort_level: 2, // Каменная Ладожская крепость
            is_capital: false,
        },
        Province {
            id: PROV_TORZHOK,
            name_ru: "Торжок",
            position: Vec3::new(-2.9, 0.0, 2.7),
            neighbors: vec![PROV_NOVGOROD, PROV_TVER, PROV_RZHEV],
            trade_good: TradeGood::Grain,
            owner: CountryTag::Novgorod,
            controller: CountryTag::Novgorod,
            pops: pops(18000, 4200, 900, 700),
            base_tax: 3.8,
            control: 0.88,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_VOLOGDA,
            name_ru: "Вологда",
            position: Vec3::new(1.8, 0.0, 4.2),
            neighbors: vec![PROV_BELOOZERO, PROV_KOSTROMA, PROV_KASHIN],
            trade_good: TradeGood::Salt,
            owner: CountryTag::Novgorod,
            controller: CountryTag::Novgorod,
            pops: pops(16000, 3000, 1000, 600),
            base_tax: 3.2,
            control: 0.80,
            fort_level: 1,
            is_capital: false,
        },
        // Псковская республика
        Province {
            id: PROV_PSKOV,
            name_ru: "Псков",
            position: Vec3::new(-6.0, 0.0, 3.2),
            neighbors: vec![PROV_NOVGOROD, PROV_SMOLENSK],
            trade_good: TradeGood::WaxAndHoney,
            owner: CountryTag::Pskov,
            controller: CountryTag::Pskov,
            pops: pops(26000, 9000, 2000, 1100),
            base_tax: 4.8,
            control: 1.0,
            fort_level: 2, // Мощный Псковский Кром
            is_capital: true,
        },
        // Рязанское княжество
        Province {
            id: PROV_RYAZAN,
            name_ru: "Переяславль-Рязанский",
            position: Vec3::new(2.2, 0.0, -1.8),
            neighbors: vec![PROV_KOLOMNA, PROV_PRONSK, PROV_MUROM, PROV_KULIKOVO],
            trade_good: TradeGood::Grain,
            owner: CountryTag::Ryazan,
            controller: CountryTag::Ryazan,
            pops: pops(24000, 6000, 1600, 1400),
            base_tax: 4.2,
            control: 1.0,
            fort_level: 1,
            is_capital: true,
        },
        Province {
            id: PROV_PRONSK,
            name_ru: "Пронск",
            position: Vec3::new(2.0, 0.0, -3.0),
            neighbors: vec![PROV_RYAZAN, PROV_KULIKOVO],
            trade_good: TradeGood::Iron,
            owner: CountryTag::Ryazan,
            controller: CountryTag::Ryazan,
            pops: pops(12000, 2200, 600, 500),
            base_tax: 2.0,
            control: 0.85,
            fort_level: 1,
            is_capital: false,
        },
        // Нижегородско-Суздальское княжество
        Province {
            id: PROV_NIZHNY_NOVGOROD,
            name_ru: "Нижний Новгород",
            position: Vec3::new(4.5, 0.0, 0.2),
            neighbors: vec![PROV_SUZDAL, PROV_MUROM, PROV_KOSTROMA, PROV_BULGAR],
            trade_good: TradeGood::Salt,
            owner: CountryTag::Suzdal,
            controller: CountryTag::Suzdal,
            pops: pops(25000, 7500, 1800, 1300),
            base_tax: 5.2,
            control: 1.0,
            fort_level: 2,
            is_capital: true,
        },
        Province {
            id: PROV_SUZDAL,
            name_ru: "Суздаль",
            position: Vec3::new(3.0, 0.0, 1.3),
            neighbors: vec![PROV_VLADIMIR, PROV_NIZHNY_NOVGOROD, PROV_KOSTROMA],
            trade_good: TradeGood::WaxAndHoney,
            owner: CountryTag::Suzdal,
            controller: CountryTag::Suzdal,
            pops: pops(17000, 3500, 1500, 900),
            base_tax: 3.5,
            control: 0.90,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_MUROM,
            name_ru: "Муром",
            position: Vec3::new(3.8, 0.0, -0.9),
            neighbors: vec![PROV_VLADIMIR, PROV_RYAZAN, PROV_NIZHNY_NOVGOROD],
            trade_good: TradeGood::Timber,
            owner: CountryTag::Suzdal,
            controller: CountryTag::Suzdal,
            pops: pops(14000, 2800, 800, 600),
            base_tax: 2.6,
            control: 0.84,
            fort_level: 1,
            is_capital: false,
        },
        // Великое княжество Литовское
        Province {
            id: PROV_SMOLENSK,
            name_ru: "Смоленск",
            position: Vec3::new(-4.2, 0.0, -1.2),
            neighbors: vec![PROV_MOZHAYSK, PROV_RZHEV, PROV_PSKOV, PROV_KULIKOVO],
            trade_good: TradeGood::Iron,
            owner: CountryTag::Lithuania,
            controller: CountryTag::Lithuania,
            pops: pops(30000, 8000, 2000, 1600),
            base_tax: 5.5,
            control: 0.88,
            fort_level: 2,
            is_capital: false,
        },
        // Золотая Орда и Степь
        Province {
            id: PROV_KULIKOVO,
            name_ru: "Куликово Поле (Верхний Дон)",
            position: Vec3::new(0.6, 0.0, -3.8),
            neighbors: vec![
                PROV_KOLOMNA,
                PROV_RYAZAN,
                PROV_PRONSK,
                PROV_SARAI,
                PROV_SMOLENSK,
            ],
            trade_good: TradeGood::Warhorses,
            owner: CountryTag::GoldenHorde,
            controller: CountryTag::GoldenHorde,
            pops: pops(8000, 1000, 300, 3500), // Кочевья темника Мамая
            base_tax: 2.0,
            control: 0.70,
            fort_level: 0,
            is_capital: false,
        },
        Province {
            id: PROV_BULGAR,
            name_ru: "Булгар (Волжский Улус)",
            position: Vec3::new(6.2, 0.0, -0.7),
            neighbors: vec![PROV_NIZHNY_NOVGOROD, PROV_SARAI],
            trade_good: TradeGood::Furs,
            owner: CountryTag::GoldenHorde,
            controller: CountryTag::GoldenHorde,
            pops: pops(22000, 7000, 1500, 1500),
            base_tax: 4.8,
            control: 0.80,
            fort_level: 1,
            is_capital: false,
        },
        Province {
            id: PROV_SARAI,
            name_ru: "Сарай-Берке (Столица Орды)",
            position: Vec3::new(4.8, 0.0, -4.6),
            neighbors: vec![PROV_KULIKOVO, PROV_BULGAR],
            trade_good: TradeGood::Warhorses,
            owner: CountryTag::GoldenHorde,
            controller: CountryTag::GoldenHorde,
            pops: pops(38000, 18000, 3000, 5000),
            base_tax: 9.0,
            control: 1.0,
            fort_level: 2,
            is_capital: true,
        },
    ];

    for prov in prov_list {
        provinces.insert(prov.id, prov);
    }

    // ── Countries ─────────────────────────────────────────────────────────────
    countries.insert(
        CountryTag::Moscow,
        Country {
            tag: CountryTag::Moscow,
            treasury: 120.0,
            manpower: 14000,
            max_manpower: 25000,
            stability: 2,
            prestige: 45.0,
            capital: PROV_MOSCOW,
            tribute_to_horde: 25.0, // Ежегодная дань
            horde_loyalty: 35.0,    // На грани мятежа в 1380 г.!
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::Novgorod,
        Country {
            tag: CountryTag::Novgorod,
            treasury: 350.0,
            manpower: 10000,
            max_manpower: 18000,
            stability: 1,
            prestige: 50.0,
            capital: PROV_NOVGOROD,
            tribute_to_horde: 15.0,
            horde_loyalty: 60.0,
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::Tver,
        Country {
            tag: CountryTag::Tver,
            treasury: 85.0,
            manpower: 7000,
            max_manpower: 12000,
            stability: 1,
            prestige: 30.0,
            capital: PROV_TVER,
            tribute_to_horde: 12.0,
            horde_loyalty: 70.0,
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::Ryazan,
        Country {
            tag: CountryTag::Ryazan,
            treasury: 55.0,
            manpower: 5500,
            max_manpower: 9000,
            stability: 0,
            prestige: 25.0,
            capital: PROV_RYAZAN,
            tribute_to_horde: 10.0,
            horde_loyalty: 80.0, // Лавирует между Мамаем и Москвой
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::Suzdal,
        Country {
            tag: CountryTag::Suzdal,
            treasury: 75.0,
            manpower: 6500,
            max_manpower: 11000,
            stability: 1,
            prestige: 28.0,
            capital: PROV_NIZHNY_NOVGOROD,
            tribute_to_horde: 10.0,
            horde_loyalty: 55.0,
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::Lithuania,
        Country {
            tag: CountryTag::Lithuania,
            treasury: 180.0,
            manpower: 16000,
            max_manpower: 28000,
            stability: 1,
            prestige: 60.0,
            capital: PROV_SMOLENSK,
            tribute_to_horde: 0.0,
            horde_loyalty: 100.0, // Союзник Мамая против Москвы
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::GoldenHorde,
        Country {
            tag: CountryTag::GoldenHorde,
            treasury: 400.0,
            manpower: 35000,
            max_manpower: 50000,
            stability: 0,
            prestige: 80.0,
            capital: PROV_SARAI,
            tribute_to_horde: 0.0,
            horde_loyalty: 100.0,
            at_war_with: vec![],
        },
    );

    countries.insert(
        CountryTag::Pskov,
        Country {
            tag: CountryTag::Pskov,
            treasury: 90.0,
            manpower: 4500,
            max_manpower: 7000,
            stability: 2,
            prestige: 20.0,
            capital: PROV_PSKOV,
            tribute_to_horde: 5.0,
            horde_loyalty: 75.0,
            at_war_with: vec![],
        },
    );

    // ── Initial Armies ────────────────────────────────────────────────────────
    armies.push(Army {
        id: 1,
        name: "Московская великокняжеская рать".to_string(),
        owner: CountryTag::Moscow,
        location: PROV_MOSCOW,
        regiments: vec![
            Regiment {
                regiment_type: RegimentType::BoyarCavalry,
                soldiers: 3000,
                home_province: PROV_MOSCOW,
            },
            Regiment {
                regiment_type: RegimentType::Militia,
                soldiers: 4500,
                home_province: PROV_MOSCOW,
            },
            Regiment {
                regiment_type: RegimentType::Militia,
                soldiers: 3500,
                home_province: PROV_KOLOMNA,
            },
            Regiment {
                regiment_type: RegimentType::Archers,
                soldiers: 1500,
                home_province: PROV_VLADIMIR,
            },
        ],
        morale: 1.0,
        max_morale: 3.5,
        movement_path: vec![],
        movement_progress: 0.0,
        in_battle: false,
    });

    armies.push(Army {
        id: 2,
        name: "Владимиро-Суздальская дружина".to_string(),
        owner: CountryTag::Moscow,
        location: PROV_VLADIMIR,
        regiments: vec![
            Regiment {
                regiment_type: RegimentType::BoyarCavalry,
                soldiers: 1200,
                home_province: PROV_VLADIMIR,
            },
            Regiment {
                regiment_type: RegimentType::Militia,
                soldiers: 2500,
                home_province: PROV_VLADIMIR,
            },
        ],
        morale: 1.0,
        max_morale: 3.0,
        movement_path: vec![],
        movement_progress: 0.0,
        in_battle: false,
    });

    armies.push(Army {
        id: 3,
        name: "Орда темника Мамая".to_string(),
        owner: CountryTag::GoldenHorde,
        location: PROV_KULIKOVO,
        regiments: vec![
            Regiment {
                regiment_type: RegimentType::HordeHorseArchers,
                soldiers: 6000,
                home_province: PROV_SARAI,
            },
            Regiment {
                regiment_type: RegimentType::HordeHorseArchers,
                soldiers: 4000,
                home_province: PROV_KULIKOVO,
            },
            Regiment {
                regiment_type: RegimentType::Militia,
                soldiers: 5000,
                home_province: PROV_BULGAR,
            },
        ],
        morale: 1.0,
        max_morale: 3.8,
        movement_path: vec![],
        movement_progress: 0.0,
        in_battle: false,
    });

    armies.push(Army {
        id: 4,
        name: "Новгородская вечевая рать".to_string(),
        owner: CountryTag::Novgorod,
        location: PROV_NOVGOROD,
        regiments: vec![
            Regiment {
                regiment_type: RegimentType::BoyarCavalry,
                soldiers: 1500,
                home_province: PROV_NOVGOROD,
            },
            Regiment {
                regiment_type: RegimentType::Militia,
                soldiers: 4000,
                home_province: PROV_NOVGOROD,
            },
        ],
        morale: 1.0,
        max_morale: 2.8,
        movement_path: vec![],
        movement_progress: 0.0,
        in_battle: false,
    });

    (provinces, countries, armies)
}
