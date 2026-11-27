//! Codenames, titles, and the names of unnamed sessions.

use std::time::UNIX_EPOCH;

use crate::sys::{CommandSpec, Sys};

const ADJECTIVES: [&str; 48] = [
    "Boring",
    "Adventurous",
    "Sleepy",
    "Dapper",
    "Cosmic",
    "Sneaky",
    "Gentle",
    "Rowdy",
    "Prudent",
    "Spicy",
    "Mellow",
    "Frantic",
    "Stoic",
    "Jaunty",
    "Grumpy",
    "Plucky",
    "Serene",
    "Wobbly",
    "Shrewd",
    "Zesty",
    "Bashful",
    "Intrepid",
    "Crispy",
    "Solemn",
    "Feral",
    "Suave",
    "Skittish",
    "Vigilant",
    "Peppy",
    "Somber",
    "Nimble",
    "Brooding",
    "Chipper",
    "Wily",
    "Earnest",
    "Snazzy",
    "Groggy",
    "Dauntless",
    "Fussy",
    "Breezy",
    "Stalwart",
    "Quirky",
    "Placid",
    "Rambunctious",
    "Meek",
    "Swanky",
    "Restless",
    "Deft",
];

const NOUNS: [&str; 48] = [
    "Jason",
    "Possum",
    "Wombat",
    "Heron",
    "Badger",
    "Otter",
    "Lemur",
    "Marmot",
    "Falcon",
    "Walrus",
    "Gecko",
    "Bison",
    "Ferret",
    "Puffin",
    "Stoat",
    "Ibex",
    "Newt",
    "Mongoose",
    "Tapir",
    "Quokka",
    "Capybara",
    "Osprey",
    "Vole",
    "Axolotl",
    "Gary",
    "Marlene",
    "Nigel",
    "Beatrice",
    "Clarence",
    "Doreen",
    "Pangolin",
    "Numbat",
    "Kestrel",
    "Shrike",
    "Muskrat",
    "Bandicoot",
    "Gibbon",
    "Tamarin",
    "Loon",
    "Kinkajou",
    "Hoopoe",
    "Dugong",
    "Marmoset",
    "Petrel",
    "Stanley",
    "Agnes",
    "Wendell",
    "Petunia",
];

/// A session's codename: "Plucky Gary". Every session gets one; an
/// unnamed session's title is its codename.
pub fn codename(sys: &dyn Sys) -> String {
    let a = ADJECTIVES[sys.pick(ADJECTIVES.len())];
    let n = NOUNS[sys.pick(NOUNS.len())];
    format!("{a} {n}")
}

/// At most 50 characters, with an ellipsis when cut, so titles stay scannable.
pub fn truncate_title(s: &str) -> String {
    if s.chars().count() <= 50 {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(49).collect();
        t.push('…');
        t
    }
}

/// An unnamed session's name: `sesh_YYYYmmdd_HHMMSS`.
pub fn unnamed(sys: &dyn Sys) -> String {
    format!("sesh_{}", stamp(sys))
}

/// `YYYYmmdd_HHMMSS`, in local time as `date` gives it, else UTC from the
/// clock.
pub fn stamp(sys: &dyn Sys) -> String {
    let local = sys
        .cmd()
        .run(&CommandSpec::new("date").arg("+%Y%m%d_%H%M%S"))
        .ok()
        .filter(|o| o.success())
        .map(|o| o.text())
        .filter(|t| t.len() == 15 && t.chars().all(|c| c.is_ascii_digit() || c == '_'));
    local.unwrap_or_else(|| utc_stamp(sys))
}

fn utc_stamp(sys: &dyn Sys) -> String {
    let secs = sys
        .now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rem = secs % 86_400;
    let (y, m, d) = civil(days);
    format!(
        "{y:04}{m:02}{d:02}_{:02}{:02}{:02}",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a date (Howard Hinnant's `civil_from_days`).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// What an agent's tabs and codenames lead with: its title, else its name
/// with the first letter capitalized.
pub fn agent_display(name: &str, title: Option<&str>) -> String {
    if let Some(t) = title.filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, ok};

    #[test]
    fn codenames_pick_an_adjective_then_a_noun() {
        let sys = FakeSys::new("/h");
        sys.push_picks([15, 24]);
        assert_eq!(codename(&sys), "Plucky Gary");
    }

    #[test]
    fn titles_are_cut_at_fifty() {
        assert_eq!(truncate_title("short"), "short");
        let long = "é".repeat(60);
        let t = truncate_title(&long);
        assert_eq!(t.chars().count(), 50);
        assert!(t.ends_with('…'));
        assert_eq!(truncate_title(&"x".repeat(50)), "x".repeat(50));
    }

    #[test]
    fn unnamed_uses_date_else_utc() {
        let sys = FakeSys::new("/h");
        assert_eq!(unnamed(&sys), "sesh_20261126_085422");
        sys.answer_ok("date", &["+%Y%m%d_%H%M%S"], "20261126_095422\n");
        assert_eq!(unnamed(&sys), "sesh_20261126_095422");
        sys.answer("date", &["+%Y%m%d_%H%M%S"], ok("garbage\n"));
        assert_eq!(unnamed(&sys), "sesh_20261126_085422");
    }

    #[test]
    fn agent_names() {
        assert_eq!(agent_display("cindy", None), "Cindy");
        assert_eq!(agent_display("cindy", Some("CINDY")), "CINDY");
        assert_eq!(agent_display("cindy", Some("")), "Cindy");
    }
}
