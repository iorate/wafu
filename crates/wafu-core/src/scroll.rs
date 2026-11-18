use std::collections::HashMap;

use crate::view::LineRanges;

// How many screen rows each line takes at the current width, as reported by
// the UI. A count is kept until the content is replaced, even after its line
// has left the ranges: the UI reports a line again only when it makes its
// element anew or the height changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LineRows {
    rows: HashMap<usize, usize>,
}

impl LineRows {
    pub(crate) fn get(&self, index: usize) -> Option<usize> {
        self.rows.get(&index).copied()
    }

    pub(crate) fn set(&mut self, index: usize, rows: usize) {
        self.rows.insert(index, rows.max(1));
    }

    pub(crate) fn clear(&mut self) {
        self.rows.clear();
    }

    pub(crate) fn drop_first(&mut self) {
        self.rows = self
            .rows
            .drain()
            .filter_map(|(index, rows)| Some((index.checked_sub(1)?, rows)))
            .collect();
    }
}

pub(crate) fn ranges(top: usize, count: usize, area: usize) -> LineRanges {
    let shown = top.min(count)..top.saturating_add(area).min(count);
    // Lines of the last page that lie above the top are already in `before`.
    let tail = count.saturating_sub(area).max(shown.end)..count;
    LineRanges {
        before: top.saturating_sub(area).min(shown.start)..shown.start,
        shown,
        tail,
    }
}

pub(crate) fn end(count: usize, area: usize, rows: impl Fn(usize) -> Option<usize>) -> usize {
    let mut used = 0;
    let mut top = count;
    while top > 0 {
        used += assumed(&rows, top - 1);
        if used > area {
            break;
        }
        top -= 1;
    }
    top.min(count.saturating_sub(1))
}

pub(crate) fn page_down(
    top: usize,
    count: usize,
    area: usize,
    rows: impl Fn(usize) -> Option<usize>,
) -> usize {
    let mut used = 0;
    let mut next = top;
    while next < count {
        used += assumed(&rows, next);
        if used > area {
            break;
        }
        next += 1;
    }
    down_to(top, next.max(top + 1), count, area, rows)
}

pub(crate) fn page_up(top: usize, area: usize, rows: impl Fn(usize) -> Option<usize>) -> usize {
    if top == 0 {
        return 0;
    }
    if assumed(&rows, top - 1) > area {
        return top - 1;
    }
    let mut used = 0;
    let mut next = top;
    while next > 0 {
        used += assumed(&rows, next - 1);
        if used > area {
            break;
        }
        next -= 1;
    }
    next
}

pub(crate) fn move_by(
    top: usize,
    delta: i64,
    count: usize,
    area: usize,
    rows: impl Fn(usize) -> Option<usize>,
) -> usize {
    let distance = usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX);
    if delta >= 0 {
        let page = page_down(top, count, area, &rows);
        down_to(
            top,
            top.saturating_add(distance).min(page),
            count,
            area,
            rows,
        )
    } else {
        top.saturating_sub(distance).max(page_up(top, area, rows))
    }
}

pub(crate) fn pull_back(
    top: usize,
    count: usize,
    area: usize,
    rows: impl Fn(usize) -> Option<usize>,
) -> usize {
    let end = end(count, area, &rows);
    // An end found with unknown lines taken as one row lies before the real
    // one, so pulling back to it would go too far.
    if top > end && (end..count).all(|index| rows(index).is_some()) {
        end
    } else {
        top
    }
}

fn assumed(rows: impl Fn(usize) -> Option<usize>, index: usize) -> usize {
    rows(index).unwrap_or(1)
}

// A top past the end stays until the pull-back can tell where the end really is.
fn down_to(
    top: usize,
    target: usize,
    count: usize,
    area: usize,
    rows: impl Fn(usize) -> Option<usize>,
) -> usize {
    target.min(end(count, area, rows)).max(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_rows_shift_with_the_dropped_line() {
        let mut rows = LineRows::default();
        rows.set(0, 2);
        rows.set(1, 3);

        rows.drop_first();

        assert_eq!(rows.get(0), Some(3));
        assert_eq!(rows.get(1), None);
    }

    #[test]
    fn ranges_measure_a_page_before_the_top_and_the_tail_not_shown() {
        assert_eq!(
            ranges(10, 30, 4),
            LineRanges {
                shown: 10..14,
                before: 6..10,
                tail: 26..30,
            }
        );
        assert_eq!(
            ranges(2, 8, 4),
            LineRanges {
                shown: 2..6,
                before: 0..2,
                tail: 6..8,
            }
        );
    }

    #[test]
    fn the_end_is_zero_when_everything_fits() {
        assert_eq!(end(3, 4, rows(&[1, 2, 1])), 0);
        assert_eq!(end(0, 4, rows(&[])), 0);
    }

    #[test]
    fn the_end_shows_the_last_lines_that_fit_exactly() {
        assert_eq!(end(5, 4, rows(&[1, 1, 1, 2, 2])), 3);
        assert_eq!(end(5, 4, rows(&[1, 1, 2, 1, 1])), 2);
    }

    #[test]
    fn the_end_is_the_last_line_when_it_alone_is_taller_than_the_area() {
        assert_eq!(end(3, 4, rows(&[1, 1, 5])), 2);
    }

    #[test]
    fn unknown_lines_count_as_one_row() {
        assert_eq!(end(5, 4, |_| None), 1);
        assert_eq!(page_down(0, 10, 4, |_| None), 4);
    }

    #[test]
    fn page_down_starts_at_the_first_line_not_fully_shown() {
        assert_eq!(
            page_down(0, 10, 4, rows(&[1, 2, 2, 1, 1, 1, 1, 1, 1, 1])),
            2
        );
    }

    #[test]
    fn page_down_moves_on_from_a_line_that_fills_the_area() {
        assert_eq!(
            page_down(1, 10, 4, rows(&[1, 4, 1, 1, 1, 1, 1, 1, 1, 1])),
            2
        );
        assert_eq!(
            page_down(1, 10, 4, rows(&[1, 6, 1, 1, 1, 1, 1, 1, 1, 1])),
            2
        );
    }

    #[test]
    fn page_down_stops_at_the_end() {
        assert_eq!(page_down(3, 8, 4, rows(&[1, 1, 1, 1, 1, 1, 1, 1])), 4);
        assert_eq!(page_down(4, 8, 4, rows(&[1, 1, 1, 1, 1, 1, 1, 1])), 4);
    }

    #[test]
    fn page_up_shows_as_many_lines_before_the_top_as_fit() {
        assert_eq!(page_up(5, 4, rows(&[1, 1, 1, 2, 1, 1])), 2);
        assert_eq!(page_up(2, 4, rows(&[1, 1, 1])), 0);
        assert_eq!(page_up(0, 4, rows(&[1])), 0);
    }

    #[test]
    fn page_up_goes_back_one_line_when_that_line_is_taller_than_the_area() {
        assert_eq!(page_up(3, 4, rows(&[1, 1, 5, 1])), 2);
    }

    #[test]
    fn moving_by_lines_never_skips_past_a_page() {
        let tall = rows(&[1, 1, 3, 3, 3, 1, 1, 1, 1, 1]);
        assert_eq!(move_by(0, 3, 10, 4, &tall), 2);
        assert_eq!(move_by(5, -3, 10, 4, &tall), 4);
        assert_eq!(move_by(1, -3, 10, 4, &tall), 0);
        assert_eq!(move_by(5, 3, 10, 4, &tall), 6);
    }

    #[test]
    fn a_top_past_the_end_is_pulled_back_once_the_tail_is_known() {
        let known = rows(&[1, 1, 1, 1, 2, 2]);
        assert_eq!(pull_back(5, 6, 4, &known), 4);
        assert_eq!(pull_back(3, 6, 4, &known), 3);

        let unknown_tail = |index: usize| (index < 5).then_some(1);
        assert_eq!(pull_back(5, 6, 4, unknown_tail), 5);
    }

    #[test]
    fn moving_down_from_past_the_end_stays_put() {
        let unknown_tail = |index: usize| (index < 5).then_some(1);
        assert_eq!(move_by(5, 1, 6, 4, unknown_tail), 5);
    }

    fn rows(counts: &[usize]) -> impl Fn(usize) -> Option<usize> + '_ {
        |index| counts.get(index).copied()
    }
}
