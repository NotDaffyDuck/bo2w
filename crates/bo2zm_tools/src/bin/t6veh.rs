//! bo2mp: print a T6 vehicle def's drive and engine values.
//! `t6veh <zone.ff> <vehicle name>...`
use std::path::PathBuf;

/// The zone (first word) and the names asked for (the rest; none means all).
fn split_command(mut words: impl Iterator<Item = String>) -> Option<(PathBuf, Vec<String>)> {
    let zone = PathBuf::from(words.next()?);
    Some((zone, words.collect()))
}

/// Whether `name` was asked for; an empty request wants everything.
fn is_wanted(asked: &[String], wanted: &str) -> bool {
    asked.is_empty() || asked.iter().any(|a| a == wanted)
}

fn main() {
    let Some((zone, asked)) = split_command(std::env::args().skip(1)) else {
        eprintln!("usage: t6veh <zone.ff> <name>...");
        return;
    };
    let capture = asset_t6::capture_zone(&zone).expect("capture");
    capture
        .vehicles
        .iter()
        .filter(|v| is_wanted(&asked, &v.name))
        .for_each(|v| println!("{v:#?}"));
}

#[cfg(test)]
mod tests {
    use super::{split_command, is_wanted};

    fn words(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn the_first_word_is_the_zone_and_the_rest_are_names() {
        let (zone, names) = split_command(words(&["a.ff", "x", "y"])).unwrap();
        assert_eq!(zone.to_str(), Some("a.ff"));
        assert_eq!(names, ["x", "y"]);
        assert!(split_command(words(&[])).is_none());
        assert!(split_command(words(&["a.ff"])).unwrap().1.is_empty());
    }

    #[test]
    fn no_names_means_every_vehicle() {
        assert!(is_wanted(&[], "anything"));
        let asked = vec!["a".to_owned()];
        assert!(is_wanted(&asked, "a"));
        assert!(!is_wanted(&asked, "b"));
    }
}
