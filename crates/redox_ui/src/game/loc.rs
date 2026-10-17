//! Localisation tables: `key = value` text files per language, a fallback language, `{name}`
//! arguments and plural forms (Russian has three rules, most others two).

use std::collections::HashMap;

/// Translations for any number of languages.
#[derive(Clone, Debug)]
pub struct Localization {
    tables: HashMap<String, HashMap<String, String>>,
    current: String,
    fallback: String,
}

impl Localization {
    /// Starts with `fallback` as the current language (used for keys missing elsewhere).
    pub fn new(fallback: &str) -> Self {
        Self {
            tables: HashMap::new(),
            current: fallback.into(),
            fallback: fallback.into(),
        }
    }

    /// Adds the lines of `text` (`key = value`, `#` comments, `\n` for a line break) to a
    /// language and returns how many entries were read. A malformed line is an error that names it.
    pub fn add_language(&mut self, language: &str, text: &str) -> Result<usize, String> {
        let mut count = 0;
        let table = self.tables.entry(language.into()).or_default();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("line {}: expected `key = value`", number + 1));
            };
            let key = key.trim();
            if key.is_empty() {
                return Err(format!("line {}: empty key", number + 1));
            }
            table.insert(key.into(), value.trim().replace("\\n", "\n"));
            count += 1;
        }
        Ok(count)
    }

    /// Switches language; `false` (and no change) when it has no table.
    pub fn set_language(&mut self, language: &str) -> bool {
        if self.tables.contains_key(language) {
            self.current = language.into();
            true
        } else {
            false
        }
    }

    /// The current language.
    pub fn language(&self) -> &str {
        &self.current
    }

    /// Available languages, sorted.
    pub fn languages(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tables.keys().map(String::as_str).collect();
        names.sort_unstable();
        names
    }

    /// The translation, from the current language and then the fallback.
    pub fn get(&self, key: &str) -> Option<&str> {
        [&self.current, &self.fallback]
            .into_iter()
            .find_map(|lang| self.tables.get(lang)?.get(key))
            .map(String::as_str)
    }

    /// The translation with `{name}` replaced by the matching argument. A missing key gives the
    /// key itself, so untranslated text is visible instead of blank.
    pub fn format(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.get(key).unwrap_or(key).to_string();
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    /// A counted phrase: picks `key.one` / `key.few` / `key.many` / `key.other` by the current
    /// language's rules and fills `{n}`.
    pub fn plural(&self, key: &str, n: i64, args: &[(&str, &str)]) -> String {
        let category = plural_category(&self.current, n);
        let chosen = [category, "other"]
            .into_iter()
            .map(|c| format!("{key}.{c}"))
            .find(|k| self.get(k).is_some())
            .unwrap_or_else(|| key.to_string());
        let n_text = n.to_string();
        let mut all = vec![("n", n_text.as_str())];
        all.extend_from_slice(args);
        self.format(&chosen, &all)
    }
}

fn plural_category(language: &str, n: i64) -> &'static str {
    let n = n.abs();
    match language {
        "ru" | "uk" | "be" => match (n % 10, n % 100) {
            (1, r) if r != 11 => "one",
            (2..=4, r) if !(12..=14).contains(&r) => "few",
            _ => "many",
        },
        _ if n == 1 => "one",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc() -> Localization {
        let mut loc = Localization::new("en");
        loc.add_language(
            "en",
            "# menu\nmenu.play = Play\nhud.coins = Coins: {n}\nitems.one = {n} item\nitems.other = {n} items\nonly.en = Only English\n",
        )
        .unwrap();
        loc.add_language(
            "ru",
            "menu.play = Играть\nhud.coins = Монеты: {n}\nitems.one = {n} предмет\nitems.few = {n} предмета\nitems.many = {n} предметов\n",
        )
        .unwrap();
        loc
    }

    #[test]
    fn translates_and_falls_back() {
        let mut loc = loc();
        assert_eq!(loc.format("menu.play", &[]), "Play");
        assert!(loc.set_language("ru"));
        assert_eq!(loc.format("menu.play", &[]), "Играть");
        assert_eq!(
            loc.format("only.en", &[]),
            "Only English",
            "falls back to English"
        );
        assert_eq!(loc.format("nope", &[]), "nope", "unknown keys stay visible");
        assert!(!loc.set_language("de"), "no table, no switch");
        assert_eq!(loc.language(), "ru");
        assert_eq!(loc.languages(), vec!["en", "ru"]);
    }

    #[test]
    fn arguments_are_substituted() {
        let mut loc = loc();
        loc.set_language("ru");
        assert_eq!(loc.format("hud.coins", &[("n", "12")]), "Монеты: 12");
    }

    #[test]
    fn russian_plurals_follow_the_three_rules() {
        let mut loc = loc();
        loc.set_language("ru");
        let say = |n| loc.plural("items", n, &[]);
        assert_eq!(say(1), "1 предмет");
        assert_eq!(say(2), "2 предмета");
        assert_eq!(say(5), "5 предметов");
        assert_eq!(say(11), "11 предметов");
        assert_eq!(say(21), "21 предмет");
        assert_eq!(say(22), "22 предмета");
        assert_eq!(say(112), "112 предметов");
    }

    #[test]
    fn english_plurals_have_two_forms() {
        let loc = loc();
        assert_eq!(loc.plural("items", 1, &[]), "1 item");
        assert_eq!(loc.plural("items", 0, &[]), "0 items");
        assert_eq!(loc.plural("items", 7, &[]), "7 items");
    }

    #[test]
    fn malformed_lines_are_reported_with_their_number() {
        let mut loc = Localization::new("en");
        let err = loc.add_language("en", "a = b\nbroken line\n").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
        assert!(loc.add_language("en", " = x").is_err());
        assert_eq!(loc.add_language("en", "multi = one\\ntwo").unwrap(), 1);
        assert_eq!(loc.get("multi"), Some("one\ntwo"));
    }
}
