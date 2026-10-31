//! bo2mp: print a T6 zone's destructible defs (pieces, health, damage scales, stages).
//! `t6dest <zone.ff> [name...]` (no names: every def). Read-only.
use std::path::PathBuf;

/// The zone (first word) and the names asked for (the rest; none means all).
fn split_command(mut words: impl Iterator<Item = String>) -> Option<(PathBuf, Vec<String>)> {
    let zone = PathBuf::from(words.next()?);
    Some((zone, words.collect()))
}

/// The names to print out of `all`, in zone order; an empty request keeps every one.
fn pick<'a, T>(all: &'a [T], asked: &[String], name_of: impl Fn(&T) -> &str) -> Vec<&'a T> {
    all.iter().filter(|d| asked.is_empty() || asked.iter().any(|a| a == name_of(d))).collect()
}

fn main() {
    let Some((zone, asked)) = split_command(std::env::args().skip(1)) else {
        eprintln!("usage: t6dest <zone.ff> [name...]");
        return;
    };
    let capture = asset_t6::capture_zone(&zone).expect("capture");
    for def in pick(&capture.destructibles, &asked, |d| d.name.as_str()) {
        println!("{def:#?}");
    }
}

#[cfg(test)]
mod tests {
    use super::{split_command, pick};

    #[test]
    fn the_first_word_is_the_zone_and_the_rest_are_names() {
        let words = ["z.ff", "barrel"].map(str::to_owned).into_iter();
        let (zone, names) = split_command(words).unwrap();
        assert_eq!(zone.to_str(), Some("z.ff"));
        assert_eq!(names, ["barrel"]);
        assert!(split_command(std::iter::empty()).is_none());
    }

    #[test]
    fn picking_keeps_zone_order_and_empty_means_all() {
        let all = ["a", "b", "c"].map(str::to_owned);
        let asked = ["c", "a"].map(str::to_owned);
        let picked: Vec<&str> = pick(&all, &asked, |s| s.as_str()).into_iter().map(String::as_str).collect();
        assert_eq!(picked, ["a", "c"]);
        assert_eq!(pick(&all, &[], |s| s.as_str()).len(), 3);
        assert!(pick(&all, &["zz".to_owned()], |s| s.as_str()).is_empty());
    }
}
