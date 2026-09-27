//! Filter / sort logic, mirroring the JavaScript in the web UI's
//! `assets/query.html` so both front-ends order and hide rows identically.

use std::cmp::Ordering;

use crate::api::ApiItem;

/// Sort fields, in the order the web UI's Sort panel shows them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Size,
    Seeders,
    Date,
}

impl SortKey {
    pub const ALL: [SortKey; 4] = [SortKey::Name, SortKey::Size, SortKey::Seeders, SortKey::Date];

    /// Button label / tooltip noun (web: FIELDS[].label).
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Seeders => "Seeds",
            SortKey::Date => "Age",
        }
    }

    /// The web UI's field key (`data-key`), used in logs.
    pub fn id(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Seeders => "seeders",
            SortKey::Date => "date",
        }
    }

    /// Direction a newly picked field starts at: Name A-Z, the rest descending.
    pub fn default_asc(self) -> bool {
        self == SortKey::Name
    }

    /// Human description of a direction (web: FIELDS[].asc / .desc).
    pub fn describe(self, asc: bool) -> &'static str {
        match (self, asc) {
            (SortKey::Name, true) => "A–Z",
            (SortKey::Name, false) => "Z–A",
            (SortKey::Size, true) => "Smallest first",
            (SortKey::Size, false) => "Largest first",
            (SortKey::Seeders, true) => "Fewest seeds first",
            (SortKey::Seeders, false) => "Most seeds first",
            (SortKey::Date, true) => "Oldest first",
            (SortKey::Date, false) => "Newest first",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortSpec {
    pub key: SortKey,
    /// true = ascending.
    pub asc: bool,
}

impl SortSpec {
    pub const DEFAULT: SortSpec = SortSpec {
        key: SortKey::Seeders,
        asc: false,
    };

    pub fn new(key: SortKey, asc: bool) -> Self {
        SortSpec { key, asc }
    }

    /// The field at its default direction.
    pub fn default_for(key: SortKey) -> Self {
        SortSpec { key, asc: key.default_asc() }
    }

    /// e.g. "size desc" (the web UI's key/dir pair), for logs and tests.
    pub fn label(&self) -> String {
        format!("{} {}", self.key.id(), if self.asc { "asc" } else { "desc" })
    }
}

/// The Sort panel's levels, mirroring `window.anorakSort` in the web UI's
/// `assets/index.html`: 1 to 4 levels, never two with the same field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SortLevels {
    specs: Vec<SortSpec>,
}

impl Default for SortLevels {
    fn default() -> Self {
        SortLevels { specs: vec![SortSpec::DEFAULT] }
    }
}

impl SortLevels {
    pub const MAX: usize = SortKey::ALL.len();

    #[allow(dead_code)]
    pub fn from_specs(specs: Vec<SortSpec>) -> Self {
        let mut s = SortLevels::default();
        for (i, spec) in specs.into_iter().take(Self::MAX).enumerate() {
            if i == 0 {
                s.specs[0] = spec;
            } else if !s.specs.iter().any(|x| x.key == spec.key) {
                s.specs.push(spec);
            }
        }
        s
    }

    pub fn specs(&self) -> &[SortSpec] {
        &self.specs
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn primary(&self) -> SortSpec {
        self.specs[0]
    }

    /// One level, most seeds first (drives the Sort toggle's dot).
    pub fn is_default(&self) -> bool {
        self.specs == [SortSpec::DEFAULT]
    }

    pub fn reset(&mut self) {
        *self = SortLevels::default();
    }

    /// Put `key` at level `index` (at `asc`, or the field's default); if
    /// another level already uses `key`, that level takes over the field and
    /// direction this level had, so there are never duplicates.
    pub fn assign(&mut self, index: usize, key: SortKey, asc: Option<bool>) {
        let Some(&prev) = self.specs.get(index) else { return };
        if let Some(dup) = self.specs.iter().enumerate().position(|(i, s)| i != index && s.key == key) {
            self.specs[dup] = prev;
        }
        self.specs[index] = SortSpec { key, asc: asc.unwrap_or(key.default_asc()) };
    }

    /// A field button in level `index` was clicked. Returns whether anything
    /// changed (the active field and fields used above are no-ops).
    pub fn pick(&mut self, index: usize, key: SortKey) -> bool {
        match self.specs.get(index) {
            Some(s) if s.key != key && !self.field_disabled(index, key) => {
                self.assign(index, key, None);
                true
            }
            _ => false,
        }
    }

    pub fn flip(&mut self, index: usize) {
        if let Some(s) = self.specs.get_mut(index) {
            s.asc = !s.asc;
        }
    }

    pub fn can_add(&self) -> bool {
        self.specs.len() < Self::MAX
    }

    /// "+ Then by": a new level with the first unused field at its default.
    pub fn add(&mut self) -> bool {
        if !self.can_add() {
            return false;
        }
        match SortKey::ALL.into_iter().find(|k| !self.specs.iter().any(|s| s.key == *k)) {
            Some(key) => {
                self.specs.push(SortSpec::default_for(key));
                true
            }
            None => false,
        }
    }

    /// Remove level `index` (level 1 can't be removed).
    pub fn remove(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.specs.len() {
            return false;
        }
        self.specs.remove(index);
        true
    }

    /// Column header click: sets level 1. The same field flips its
    /// direction; a new field starts at its default direction.
    pub fn header_click(&mut self, key: SortKey) {
        let p = self.primary();
        let asc = if p.key == key { !p.asc } else { key.default_asc() };
        self.assign(0, key, Some(asc));
    }

    /// Fields used by earlier levels are disabled in later ones.
    pub fn field_disabled(&self, index: usize, key: SortKey) -> bool {
        self.specs.iter().take(index).any(|s| s.key == key)
    }

    pub fn verb(index: usize) -> &'static str {
        if index == 0 { "Sort by" } else { "Then by" }
    }

    pub fn field_tip(&self, index: usize, key: SortKey) -> String {
        if self.field_disabled(index, key) {
            format!("{} (already used above)", key.label())
        } else {
            format!("{} {}", Self::verb(index), key.label())
        }
    }

    pub fn dir_tip(&self, index: usize) -> String {
        let s = self.specs[index];
        format!("{} — click for {}", s.key.describe(s.asc), s.key.describe(!s.asc))
    }

    pub fn add_tip(&self) -> &'static str {
        if self.can_add() { "Add another sort level" } else { "All fields are already used" }
    }

    pub const REMOVE_TIP: &'static str = "Remove this sort level";
    pub const CLEAR_TIP: &'static str = "Reset sort to most seeds";
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filters {
    pub name: String,
    pub min_seeds: Option<f64>,
    pub min_mb: Option<f64>,
    pub max_mb: Option<f64>,
}

impl Filters {
    pub fn is_active(&self) -> bool {
        !self.name.trim().is_empty()
            || self.min_seeds.is_some()
            || self.min_mb.is_some()
            || self.max_mb.is_some()
    }

    pub fn matches(&self, item: &ApiItem, name_lower: &str) -> bool {
        let q = self.name.trim().to_lowercase();
        if !q.is_empty() && !name_lower.contains(&q) {
            return false;
        }
        if let Some(min) = self.min_seeds {
            if (item.seeders as f64) < min {
                return false;
            }
        }
        if let Some(min) = self.min_mb {
            if (item.size as f64) < min * 1024.0 * 1024.0 {
                return false;
            }
        }
        if let Some(max) = self.max_mb {
            if (item.size as f64) > max * 1024.0 * 1024.0 {
                return false;
            }
        }
        true
    }
}

/// Parse a numeric filter field the way the web UI does ('' = unset).
pub fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<f64>().ok()
    }
}

fn cmp_one(a: &ApiItem, a_name: &str, b: &ApiItem, b_name: &str, spec: SortSpec) -> Ordering {
    let ord = match spec.key {
        SortKey::Seeders => a.seeders.cmp(&b.seeders),
        SortKey::Size => a.size.cmp(&b.size),
        SortKey::Date => a.date.unwrap_or(0).cmp(&b.date.unwrap_or(0)),
        SortKey::Name => a_name.cmp(b_name),
    };
    if spec.asc { ord } else { ord.reverse() }
}

/// Indices of rows to show, filtered then stably sorted by each level in
/// turn, starting from the server's order.
#[allow(dead_code)]
pub fn compute_view(
    items: &[ApiItem],
    names_lower: &[String],
    filters: &Filters,
    sorts: &[SortSpec],
) -> Vec<usize> {
    compute_view_by(items.len(), |i| &items[i], names_lower, filters, sorts)
}

/// `compute_view` over any indexable item storage (avoids cloning rows).
pub fn compute_view_by<'a>(
    len: usize,
    get: impl Fn(usize) -> &'a ApiItem,
    names_lower: &[String],
    filters: &Filters,
    sorts: &[SortSpec],
) -> Vec<usize> {
    let mut view: Vec<usize> = (0..len)
        .filter(|&i| filters.matches(get(i), &names_lower[i]))
        .collect();
    view.sort_by(|&a, &b| {
        let (ia, ib) = (get(a), get(b));
        let (na, nb) = (&names_lower[a], &names_lower[b]);
        sorts
            .iter()
            .map(|&s| cmp_one(ia, na, ib, nb, s))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    });
    view
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, seeders: u32, size: u64, date: i64) -> ApiItem {
        ApiItem {
            title: title.into(),
            guid: String::new(),
            magnet: "magnet:?xt=urn:btih:x".into(),
            category: String::new(),
            seeders,
            peers: 0,
            size,
            size_format: String::new(),
            date: Some(date),
            date_format: String::new(),
            already_added: false,
        }
    }

    #[test]
    fn filter_and_sort() {
        let items = vec![
            item("B 1080p", 5, 2 << 30, 3),
            item("a 720p", 50, 1 << 20, 1),
            item("C 1080p", 5, 3 << 30, 2),
        ];
        let names: Vec<String> = items.iter().map(|i| i.title.to_lowercase()).collect();
        let f = Filters::default();
        let d = [SortSpec::DEFAULT];
        assert_eq!(compute_view(&items, &names, &f, &d), vec![1, 0, 2]);
        let size_desc = SortSpec::new(SortKey::Size, false);
        assert_eq!(compute_view(&items, &names, &f, &[SortSpec::DEFAULT, size_desc]), vec![1, 2, 0]);
        let f = Filters { name: "1080".into(), ..Default::default() };
        let name_asc = SortSpec::new(SortKey::Name, true);
        assert_eq!(compute_view(&items, &names, &f, &[name_asc]), vec![0, 2]);
        let f = Filters { min_mb: Some(2048.0), max_mb: Some(2048.0), ..Default::default() };
        assert_eq!(compute_view(&items, &names, &f, &d), vec![0]);
    }

    #[test]
    fn multi_key_is_stable() {
        let items = vec![
            item("x", 5, 10, 1),
            item("y", 5, 20, 1),
            item("z", 5, 10, 2),
            item("w", 7, 10, 1),
        ];
        let names: Vec<String> = items.iter().map(|i| i.title.to_lowercase()).collect();
        let f = Filters::default();
        // seeds desc, size asc: ties (0 and 2) keep server order.
        let s = [SortSpec::DEFAULT, SortSpec::new(SortKey::Size, true)];
        assert_eq!(compute_view(&items, &names, &f, &s), vec![3, 0, 2, 1]);
        // + age newest first breaks the 0/2 tie.
        let s = [SortSpec::DEFAULT, SortSpec::new(SortKey::Size, true), SortSpec::new(SortKey::Date, false)];
        assert_eq!(compute_view(&items, &names, &f, &s), vec![3, 2, 0, 1]);
    }

    fn keys(l: &SortLevels) -> Vec<String> {
        l.specs().iter().map(|s| s.label()).collect()
    }

    #[test]
    fn levels_like_web() {
        let mut l = SortLevels::default();
        assert!(l.is_default());
        assert_eq!(keys(&l), ["seeders desc"]);
        // New field starts at its default direction.
        assert!(l.pick(0, SortKey::Name));
        assert_eq!(keys(&l), ["name asc"]);
        assert!(!l.is_default());
        assert!(!l.pick(0, SortKey::Name));
        l.flip(0);
        assert_eq!(keys(&l), ["name desc"]);
        // + Then by: first unused field in Name, Size, Seeds, Age order.
        assert!(l.add());
        assert_eq!(keys(&l), ["name desc", "size desc"]);
        assert!(l.add());
        assert!(l.add());
        assert_eq!(keys(&l), ["name desc", "size desc", "seeders desc", "date desc"]);
        assert!(!l.can_add());
        assert!(!l.add());
        assert_eq!(l.add_tip(), "All fields are already used");
        // Fields used above are disabled below.
        assert!(l.field_disabled(2, SortKey::Name) && l.field_disabled(2, SortKey::Size));
        assert!(!l.field_disabled(2, SortKey::Date) && !l.field_disabled(0, SortKey::Date));
        assert!(!l.pick(2, SortKey::Name));
        assert_eq!(l.field_tip(2, SortKey::Name), "Name (already used above)");
        assert_eq!(l.field_tip(2, SortKey::Date), "Then by Age");
        assert_eq!(l.field_tip(0, SortKey::Size), "Sort by Size");
        // Picking a later level's field in an earlier level swaps them.
        l.flip(3); // date asc
        assert!(l.pick(1, SortKey::Date));
        assert_eq!(keys(&l), ["name desc", "date desc", "seeders desc", "size desc"]);
        assert!(l.pick(0, SortKey::Seeders));
        assert_eq!(keys(&l), ["seeders desc", "date desc", "name desc", "size desc"]);
        // Remove.
        assert!(!l.remove(0));
        assert!(l.remove(1));
        assert_eq!(keys(&l), ["seeders desc", "name desc", "size desc"]);
        assert_eq!(l.add_tip(), "Add another sort level");
        assert!(l.add());
        assert_eq!(keys(&l), ["seeders desc", "name desc", "size desc", "date desc"]);
        l.reset();
        assert!(l.is_default());
    }

    #[test]
    fn header_click_and_tips() {
        let mut l = SortLevels::default();
        l.header_click(SortKey::Seeders);
        assert_eq!(keys(&l), ["seeders asc"]);
        l.header_click(SortKey::Seeders);
        assert_eq!(keys(&l), ["seeders desc"]);
        l.header_click(SortKey::Name);
        assert_eq!(keys(&l), ["name asc"]);
        l.header_click(SortKey::Name);
        assert_eq!(keys(&l), ["name desc"]);
        l.header_click(SortKey::Size);
        assert_eq!(keys(&l), ["size desc"]);
        assert_eq!(l.dir_tip(0), "Largest first — click for Smallest first");
        // Header on a field a later level uses: swap.
        l.add(); // name asc
        l.header_click(SortKey::Name);
        assert_eq!(keys(&l), ["name asc", "size desc"]);
        assert_eq!(l.dir_tip(0), "A–Z — click for Z–A");
        l.header_click(SortKey::Date);
        assert_eq!(keys(&l), ["date desc", "size desc"]);
        assert_eq!(l.dir_tip(0), "Newest first — click for Oldest first");
        l.flip(0);
        assert_eq!(l.dir_tip(0), "Oldest first — click for Newest first");
        let l = SortLevels::default();
        assert_eq!(l.dir_tip(0), "Most seeds first — click for Fewest seeds first");
    }
}
