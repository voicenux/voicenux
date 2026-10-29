//! Names as people say them: what speech recognition hears is a little off now and then, and
//! comes with case and punctuation of its own.

/// Lowercase, letters and digits only, one space between words: "Vine Boom!" is "vine boom".
fn plain(s: &str) -> String {
    let s: String = s.chars().flat_map(char::to_lowercase).map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether two names are the same, whatever the case and the punctuation: "Vine Boom!" and
/// "vine boom" are.
pub fn same_name(a: &str, b: &str) -> bool {
    let a = plain(a);
    !a.is_empty() && a == plain(b)
}

/// How well `said` fits `name`, from 0 to 1. The same name is 1, the name among other words
/// ("the robot voice") almost, a name heard a little off ("darth vadar") less.
pub fn likeness(said: &str, name: &str) -> f32 {
    let (said, name) = (plain(said), plain(name));
    if said.is_empty() || name.is_empty() {
        return 0.0;
    }
    if said == name {
        return 1.0;
    }
    if format!(" {said} ").contains(&format!(" {name} ")) {
        return 0.9;
    }
    let (a, b): (Vec<char>, Vec<char>) = (said.chars().collect(), name.chars().collect());
    1.0 - edits(&a, &b) as f32 / a.len().max(b.len()) as f32
}

/// How many letters to add, drop or change to turn `a` into `b` (Levenshtein).
fn edits(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_whatever_the_case_and_the_punctuation() {
        assert!(same_name("Vine Boom", "vine boom."));
        assert!(same_name("What the hell (speed up)", "what the hell speed up"));
        assert!(same_name("Ärger!", "ärger"));
        assert!(!same_name("Vine Boom", "Vine Boom 2"));
        assert!(!same_name("", "  ?! "), "nothing is no name");
    }

    #[test]
    fn names_heard_a_little_off_still_fit() {
        assert_eq!(likeness("Darth Vader.", "Darth Vader"), 1.0);
        assert_eq!(likeness("the robot voice", "Robot"), 0.9);
        assert!(likeness("Darth Vadar", "Darth Vader") > 0.8);
        assert!(likeness("Chipmunk", "Robot") < 0.5);
        assert_eq!(likeness("", "Robot"), 0.0);
        assert_eq!(edits(&['a', 'b'], &['b']), 1);
    }
}
