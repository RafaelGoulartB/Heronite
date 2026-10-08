//! Expand / paginate state of the long lists (not persisted).

/// Collapsed lists show this many rows.
pub(super) const COLLAPSED_ROWS: usize = 5;
/// Rows per page once a list is expanded.
const PAGE_ROWS: usize = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ListKind {
    Hidden,
    Favourites,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Pager {
    pub(super) expanded: bool,
    pub(super) page: usize,
}

impl Pager {
    /// An expanded list showing page `page` (0-based).
    pub(super) fn expanded_at(page: usize) -> Self {
        Self { expanded: true, page }
    }

    pub(super) fn pages(total: usize) -> usize {
        total.div_ceil(PAGE_ROWS).max(1)
    }

    /// Keeps `page` valid after the list shrank (unhide, clear).
    pub(super) fn clamp(&mut self, total: usize) {
        self.page = self.page.min(Self::pages(total) - 1);
        if total <= COLLAPSED_ROWS {
            self.expanded = false;
        }
    }

    /// Rows to draw out of `total`.
    pub(super) fn range(&self, total: usize) -> std::ops::Range<usize> {
        if !self.expanded {
            return 0..total.min(COLLAPSED_ROWS);
        }
        let start = (self.page.min(Self::pages(total) - 1)) * PAGE_ROWS;
        start.min(total)..(start + PAGE_ROWS).min(total)
    }
}

#[cfg(test)]
mod tests {
    use super::Pager;

    #[test]
    fn pager_shows_five_then_ten_per_page_and_stays_valid() {
        let mut p = Pager::default();
        assert_eq!(p.range(27), 0..5);
        assert_eq!(p.range(3), 0..3);
        p.expanded = true;
        assert_eq!((Pager::pages(27), p.range(27)), (3, 0..10));
        p.page = 2;
        assert_eq!(p.range(27), 20..27);
        // Unhiding games shrinks the list: the page index is clamped.
        p.clamp(20);
        assert_eq!((p.page, p.range(20)), (1, 10..20));
        p.clamp(4);
        assert_eq!((p.page, p.expanded, p.range(4)), (0, false, 0..4));
        assert_eq!(Pager::pages(0), 1);
    }
}
