//! Moving things in a list the way a drag and drop does: next to another one, before or after it.

/// Moves the items with these ids next to `target`, before or after it, in the order they had.
/// The target may be one of them: then they go where it was.
pub fn move_group<T>(list: &mut Vec<T>, id: impl Fn(&T) -> &str, ids: &[String], target: &str, after: bool) {
    let Some(at) = list.iter().position(|x| id(x) == target) else { return };
    let moving = |x: &T| ids.iter().any(|i| i == id(x));
    // The place counts only the items that stay.
    let mut to = list[..at].iter().filter(|x| !moving(x)).count();
    if after && !moving(&list[at]) {
        to += 1;
    }
    let (group, rest): (Vec<T>, Vec<T>) = std::mem::take(list).into_iter().partition(|x| moving(x));
    *list = rest;
    let to = to.min(list.len());
    list.splice(to..to, group);
}

/// `name`, or with a number after it when the list has that name already.
pub fn free_name(list: &[String], name: &str) -> String {
    let name = name.trim();
    (1..).map(|i| if i == 1 { name.to_string() } else { format!("{name} {i}") }).find(|n| !list.contains(n)).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn groups_keep_their_order() {
        let mut l = list("a b c d e");
        move_group(&mut l, |x| x.as_str(), &list("d b"), "e", true);
        assert_eq!(l, list("a c e b d"));
        move_group(&mut l, |x| x.as_str(), &list("b d"), "a", false);
        assert_eq!(l, list("b d a c e"));
        move_group(&mut l, |x| x.as_str(), &list("b d"), "d", true);
        assert_eq!(l, list("b d a c e"));
        move_group(&mut l, |x| x.as_str(), &list("b"), "gone", true);
        assert_eq!(l, list("b d a c e"));
    }
}
