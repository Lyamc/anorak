//! Filter / sort logic, mirroring the JavaScript in the web UI's
//! `assets/query.html` so both front-ends order and hide rows identically.

use std::cmp::Ordering;

use crate::api::ApiItem;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Seeders,
    Size,
    Date,
    Name,
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

    /// Same eight choices, same order and labels as the web UI's <select>.
    pub const OPTIONS: [SortSpec; 8] = [
        SortSpec { key: SortKey::Seeders, asc: false },
        SortSpec { key: SortKey::Seeders, asc: true },
        SortSpec { key: SortKey::Size, asc: false },
        SortSpec { key: SortKey::Size, asc: true },
        SortSpec { key: SortKey::Date, asc: false },
        SortSpec { key: SortKey::Date, asc: true },
        SortSpec { key: SortKey::Name, asc: true },
        SortSpec { key: SortKey::Name, asc: false },
    ];

    pub fn label(&self) -> &'static str {
        match (self.key, self.asc) {
            (SortKey::Seeders, false) => "Most seeds",
            (SortKey::Seeders, true) => "Least seeds",
            (SortKey::Size, false) => "Largest",
            (SortKey::Size, true) => "Smallest",
            (SortKey::Date, false) => "Newest",
            (SortKey::Date, true) => "Oldest",
            (SortKey::Name, true) => "Title A–Z",
            (SortKey::Name, false) => "Title Z–A",
        }
    }
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

/// Indices of rows to show, filtered then stably sorted (primary, then
/// secondary), starting from the server's default order like the web UI.
#[allow(dead_code)]
pub fn compute_view(
    items: &[ApiItem],
    names_lower: &[String],
    filters: &Filters,
    primary: SortSpec,
    secondary: Option<SortSpec>,
) -> Vec<usize> {
    compute_view_by(items.len(), |i| &items[i], names_lower, filters, primary, secondary)
}

/// `compute_view` over any indexable item storage (avoids cloning rows).
pub fn compute_view_by<'a>(
    len: usize,
    get: impl Fn(usize) -> &'a ApiItem,
    names_lower: &[String],
    filters: &Filters,
    primary: SortSpec,
    secondary: Option<SortSpec>,
) -> Vec<usize> {
    let mut view: Vec<usize> = (0..len)
        .filter(|&i| filters.matches(get(i), &names_lower[i]))
        .collect();
    view.sort_by(|&a, &b| {
        let (ia, ib) = (get(a), get(b));
        let (na, nb) = (&names_lower[a], &names_lower[b]);
        cmp_one(ia, na, ib, nb, primary).then_with(|| {
            secondary
                .map(|s| cmp_one(ia, na, ib, nb, s))
                .unwrap_or(Ordering::Equal)
        })
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
        assert_eq!(compute_view(&items, &names, &f, SortSpec::DEFAULT, None), vec![1, 0, 2]);
        let size_desc = SortSpec { key: SortKey::Size, asc: false };
        assert_eq!(
            compute_view(&items, &names, &f, SortSpec::DEFAULT, Some(size_desc)),
            vec![1, 2, 0]
        );
        let f = Filters { name: "1080".into(), ..Default::default() };
        let name_asc = SortSpec { key: SortKey::Name, asc: true };
        assert_eq!(compute_view(&items, &names, &f, name_asc, None), vec![0, 2]);
        let f = Filters { min_mb: Some(2048.0), max_mb: Some(2048.0), ..Default::default() };
        assert_eq!(compute_view(&items, &names, &f, SortSpec::DEFAULT, None), vec![0]);
    }
}
