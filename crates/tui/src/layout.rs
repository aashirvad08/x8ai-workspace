//! The layout of one tab: a binary tree whose leaves are panes, as in the app
//! (`src/terminal/panes.ts`), placed on the terminal's grid of cells.
//!
//! Panes side by side are separated by a column of `│`. When a tab has more
//! than one pane each gets a title row, which also separates panes one above
//! the other.
//!
//! Splits along the same axis, one inside the other, make a row (or a
//! column): splitting or closing a pane shares its row out evenly again, so
//! six panes side by side get a sixth each, not a half, a quarter, an eighth.

use ratatui::layout::Rect;

use crate::pane::PaneId;

/// Dragging a line leaves no pane less than this share of its split.
pub const MIN_RATIO: f32 = 0.1;

/// No split makes a pane narrower than this (side by side)…
pub const MIN_COLS: u16 = 20;

/// …or shorter than this, under its title row (one above the other).
pub const MIN_ROWS: u16 = 5;

/// Where the second half of a split goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// To the right of the first: panes side by side.
    Right,
    /// Below the first: panes one above the other.
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tree {
    Pane(PaneId),
    Split {
        id: u32,
        axis: Axis,
        /// The first half's share: of its row's members when evened, else
        /// where the line was dragged to.
        ratio: f32,
        first: Box<Tree>,
        second: Box<Tree>,
    },
}

/// Where a pane is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneArea {
    pub id: PaneId,
    /// Its title row, when the tab has more than one pane.
    pub title: Option<Rect>,
    /// Where the program's screen goes.
    pub body: Rect,
}

impl PaneArea {
    /// The title and the body together.
    pub fn whole(&self) -> Rect {
        self.title.map_or(self.body, |title| title.union(self.body))
    }
}

/// The line between a split's halves, which the mouse can drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divider {
    pub split: u32,
    pub axis: Axis,
    /// The whole area the split divides.
    pub area: Rect,
    /// A column of `│` (side by side), or the second half's top row.
    pub line: Rect,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Layout {
    pub panes: Vec<PaneArea>,
    pub dividers: Vec<Divider>,
}

impl Tree {
    /// Pane ids in reading order: left to right, top to bottom.
    pub fn panes(&self) -> Vec<PaneId> {
        match self {
            Self::Pane(id) => vec![*id],
            Self::Split { first, second, .. } => {
                let mut ids = first.panes();
                ids.extend(second.panes());
                ids
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Self::Pane(pane) => *pane == id,
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    /// Splits pane `target` in two: `added` goes right of it or below it, and
    /// the row they are in is shared out evenly. Returns whether `target` was
    /// found.
    pub fn split(&mut self, target: PaneId, added: PaneId, axis: Axis, id: u32) -> bool {
        if !self.split_in_two(target, added, axis, id) {
            return false;
        }
        if let Some((row, _)) = self.row_of(added) {
            self.even(row);
        }
        true
    }

    fn split_in_two(&mut self, target: PaneId, added: PaneId, axis: Axis, id: u32) -> bool {
        match self {
            Self::Pane(pane) if *pane == target => {
                *self = Self::Split {
                    id,
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Pane(target)),
                    second: Box::new(Self::Pane(added)),
                };
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.split_in_two(target, added, axis, id)
                    || second.split_in_two(target, added, axis, id)
            }
        }
    }

    /// Puts pane `added` where `target` is. Returns whether `target` was found.
    pub fn replace(&mut self, target: PaneId, added: PaneId) -> bool {
        match self {
            Self::Pane(pane) if *pane == target => {
                *pane = added;
                true
            }
            Self::Pane(_) => false,
            Self::Split { first, second, .. } => {
                first.replace(target, added) || second.replace(target, added)
            }
        }
    }

    /// Removes pane `target`; its sibling takes the space, and what is left of
    /// its row is shared out evenly. Returns false when `target` was the whole
    /// tree, which is then left as it was.
    pub fn remove(&mut self, target: PaneId) -> bool {
        // The row's first split stays unless it was the pane's own.
        let row = self
            .row_of(target)
            .and_then(|(row, own)| (row != own).then_some(row));
        let removed = self.take_out(target);
        if let Some(row) = row {
            self.even(row);
        }
        removed
    }

    fn take_out(&mut self, target: PaneId) -> bool {
        let Self::Split { first, second, .. } = self else {
            return *self != Self::Pane(target);
        };
        let gone = Self::Pane(target);
        let kept = if **first == gone {
            Some(std::mem::replace(&mut **second, gone))
        } else if **second == gone {
            Some(std::mem::replace(&mut **first, gone))
        } else {
            None
        };
        match kept {
            Some(kept) => *self = kept,
            None => {
                first.take_out(target);
                second.take_out(target);
            }
        }
        true
    }

    /// The splits from the top down to pane `target`, its own last.
    fn path_to(&self, target: PaneId) -> Option<Vec<(u32, Axis)>> {
        match self {
            Self::Pane(id) => (*id == target).then(Vec::new),
            Self::Split {
                id,
                axis,
                first,
                second,
                ..
            } => {
                let mut path = first.path_to(target).or_else(|| second.path_to(target))?;
                path.insert(0, (*id, *axis));
                Some(path)
            }
        }
    }

    /// The row pane `target` is in, as the first of the splits along its own
    /// split's axis right above it, and its own split.
    fn row_of(&self, target: PaneId) -> Option<(u32, u32)> {
        let path = self.path_to(target)?;
        let &(own, axis) = path.last()?;
        let (row, _) = path.iter().rev().take_while(|(_, a)| *a == axis).last()?;
        Some((*row, own))
    }

    /// Shares out the row whose first split is `id` evenly among its members.
    fn even(&mut self, id: u32) {
        match self {
            Self::Split { id: this, axis, .. } if *this == id => {
                let axis = *axis;
                self.balance(axis);
            }
            Self::Split { first, second, .. } => {
                first.even(id);
                second.even(id);
            }
            Self::Pane(_) => {}
        }
    }

    /// Gives each half of a row along `along` its members' share, and returns
    /// how many members it has: panes, or splits along the other axis.
    fn balance(&mut self, along: Axis) -> u16 {
        match self {
            Self::Split {
                axis,
                ratio,
                first,
                second,
                ..
            } if *axis == along => {
                let (a, b) = (first.balance(along), second.balance(along));
                *ratio = f32::from(a) / f32::from(a + b);
                a + b
            }
            _ => 1,
        }
    }

    /// Panes `ids` in an even grid, in reading order: rows of as many as
    /// there are rows, or one more, the last row taking what is left. Each
    /// split's id is `next_id`, counted up first. `None` with no panes.
    pub fn grid(ids: &[PaneId], next_id: &mut u32) -> Option<Self> {
        let mut cols = 1;
        while cols * cols < ids.len() {
            cols += 1;
        }
        let rows = ids
            .chunks(cols)
            .filter_map(|row| {
                let panes = row.iter().map(|&id| Self::Pane(id)).collect();
                Self::row(panes, Axis::Right, next_id)
            })
            .collect();
        Self::row(rows, Axis::Down, next_id)
    }

    /// `members` one after the other along `axis`, each with an even share.
    fn row(members: Vec<Self>, axis: Axis, next_id: &mut u32) -> Option<Self> {
        let mut members = members.into_iter().rev();
        let last = members.next()?;
        let mut row = members.fold(last, |rest, member| {
            *next_id += 1;
            Self::Split {
                id: *next_id,
                axis,
                ratio: 0.5,
                first: Box::new(member),
                second: Box::new(rest),
            }
        });
        row.balance(axis);
        Some(row)
    }

    /// Sets split `id`'s share, kept between MIN_RATIO and 1 - MIN_RATIO.
    pub fn resize(&mut self, id: u32, to: f32) {
        if let Self::Split {
            id: this,
            ratio,
            first,
            second,
            ..
        } = self
        {
            if *this == id {
                let to = if to.is_finite() { to } else { 0.5 };
                *ratio = to.clamp(MIN_RATIO, 1.0 - MIN_RATIO);
            } else {
                first.resize(id, to);
                second.resize(id, to);
            }
        }
    }

    /// Places the tree in `area`.
    pub fn layout(&self, area: Rect) -> Layout {
        let mut layout = Layout::default();
        let titled = matches!(self, Self::Split { .. });
        self.place(area, titled, &mut layout);
        layout
    }

    fn place(&self, area: Rect, titled: bool, out: &mut Layout) {
        match self {
            Self::Pane(id) => {
                let (title, body) = if titled && area.height >= 2 {
                    (
                        Some(Rect { height: 1, ..area }),
                        Rect {
                            y: area.y + 1,
                            height: area.height - 1,
                            ..area
                        },
                    )
                } else {
                    (None, area)
                };
                out.panes.push(PaneArea {
                    id: *id,
                    title,
                    body,
                });
            }
            Self::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => {
                let (first_area, line, second_area) = match axis {
                    Axis::Right => {
                        // One column for the line between them.
                        let width = beside(area.width, *ratio);
                        let line_width = u16::from(area.width > width);
                        (
                            Rect { width, ..area },
                            Rect {
                                x: area.x + width,
                                width: line_width,
                                ..area
                            },
                            Rect {
                                x: area.x + width + line_width,
                                width: area.width - width - line_width,
                                ..area
                            },
                        )
                    }
                    Axis::Down => {
                        let height = share(area.height, *ratio);
                        let second = Rect {
                            y: area.y + height,
                            height: area.height - height,
                            ..area
                        };
                        (
                            Rect { height, ..area },
                            Rect {
                                height: second.height.min(1),
                                ..second
                            },
                            second,
                        )
                    }
                };
                out.dividers.push(Divider {
                    split: *id,
                    axis: *axis,
                    area,
                    line,
                });
                first.place(first_area, titled, out);
                second.place(second_area, titled, out);
            }
        }
    }
}

/// The first pane's width of `total` columns at `ratio`, the line after it
/// taking one. Every pane of a row but the last has its line after it, so the
/// shares are of one column more, and the panes of an even row come out the
/// same width. At least one column each when there are three.
fn beside(total: u16, ratio: f32) -> u16 {
    if total < 3 {
        return total / 2;
    }
    let first = (f32::from(total + 1) * ratio).round() as u16;
    first.saturating_sub(1).clamp(1, total - 2)
}

/// The first half of `total` cells at `ratio`, leaving at least one for each
/// half when there are two to share.
fn share(total: u16, ratio: f32) -> u16 {
    if total < 2 {
        return total;
    }
    let first = (f32::from(total) * ratio).round() as u16;
    first.clamp(1, total - 1)
}

impl Layout {
    pub fn area_of(&self, id: PaneId) -> Option<&PaneArea> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// The pane at a cell, if any.
    pub fn pane_at(&self, x: u16, y: u16) -> Option<&PaneArea> {
        self.panes
            .iter()
            .find(|p| p.whole().contains((x, y).into()))
    }

    /// The divider at a cell, if any: the `│` column, or a lower pane's title.
    pub fn divider_at(&self, x: u16, y: u16) -> Option<&Divider> {
        self.dividers
            .iter()
            .find(|d| d.line.contains((x, y).into()))
    }

    /// The nearest pane in `direction` from `from` that lies alongside it.
    pub fn neighbor(&self, from: PaneId, direction: Direction) -> Option<PaneId> {
        let a = self.area_of(from)?.whole();
        let overlap = |a0: u16, a1: u16, b0: u16, b1: u16| a1.min(b1).saturating_sub(a0.max(b0));
        self.panes
            .iter()
            .filter(|p| p.id != from)
            .filter_map(|p| {
                let b = p.whole();
                let (gap, shared) = match direction {
                    Direction::Left => (
                        i32::from(a.x) - i32::from(b.right()),
                        overlap(a.y, a.bottom(), b.y, b.bottom()),
                    ),
                    Direction::Right => (
                        i32::from(b.x) - i32::from(a.right()),
                        overlap(a.y, a.bottom(), b.y, b.bottom()),
                    ),
                    Direction::Up => (
                        i32::from(a.y) - i32::from(b.bottom()),
                        overlap(a.x, a.right(), b.x, b.right()),
                    ),
                    Direction::Down => (
                        i32::from(b.y) - i32::from(a.bottom()),
                        overlap(a.x, a.right(), b.x, b.right()),
                    ),
                };
                (gap >= 0 && shared > 0).then_some((gap, std::cmp::Reverse(shared), p.id))
            })
            .min_by_key(|&(gap, shared, _)| (gap, shared))
            .map(|(_, _, id)| id)
    }
}

/// The ratio that puts split `divider`'s line at cell `at` (x or y).
pub fn ratio_at(divider: &Divider, x: u16, y: u16) -> f32 {
    match divider.axis {
        // As `beside` shares: the first pane and its line, of one more column.
        Axis::Right => {
            let width = x.saturating_sub(divider.area.x);
            f32::from(width + 1) / f32::from(divider.area.width + 1)
        }
        Axis::Down if divider.area.height == 0 => 0.5,
        Axis::Down => f32::from(y.saturating_sub(divider.area.y)) / f32::from(divider.area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: PaneId = PaneId(1);
    const B: PaneId = PaneId(2);
    const C: PaneId = PaneId(3);

    /// A | B, with C below B.
    fn three() -> Tree {
        let mut tree = Tree::Pane(A);
        assert!(tree.split(A, B, Axis::Right, 10));
        assert!(tree.split(B, C, Axis::Down, 11));
        tree
    }

    #[test]
    fn splitting_adds_a_pane_beside_or_below_its_target() {
        let tree = three();
        assert_eq!(tree.panes(), [A, B, C]);
        assert!(!tree.clone().split(PaneId(9), PaneId(8), Axis::Right, 12));
    }

    #[test]
    fn a_pane_can_be_replaced_in_place() {
        let mut tree = three();
        let before = tree.layout(Rect::new(0, 0, 81, 20));
        assert!(tree.replace(B, PaneId(4)));
        assert_eq!(tree.panes(), [A, PaneId(4), C]);
        let after = tree.layout(Rect::new(0, 0, 81, 20));
        assert_eq!(
            after.area_of(PaneId(4)).unwrap().body,
            before.area_of(B).unwrap().body
        );
        assert!(!tree.replace(B, PaneId(5)));
    }

    #[test]
    fn removing_a_pane_gives_its_space_to_its_sibling() {
        let mut tree = three();
        assert!(tree.remove(B));
        assert_eq!(tree.panes(), [A, C]);
        assert!(tree.remove(A));
        assert_eq!(tree, Tree::Pane(C));
        // The last pane is not removed.
        assert!(!tree.remove(C));
        assert_eq!(tree, Tree::Pane(C));
    }

    #[test]
    fn a_single_pane_takes_the_whole_area_without_a_title() {
        let layout = Tree::Pane(A).layout(Rect::new(0, 1, 80, 20));
        assert_eq!(
            layout.panes,
            [PaneArea {
                id: A,
                title: None,
                body: Rect::new(0, 1, 80, 20)
            }]
        );
        assert!(layout.dividers.is_empty());
    }

    #[test]
    fn split_panes_get_titles_and_a_line_between_them() {
        let layout = three().layout(Rect::new(0, 0, 81, 20));
        let a = layout.area_of(A).unwrap();
        let b = layout.area_of(B).unwrap();
        let c = layout.area_of(C).unwrap();
        // 80 usable columns, halved; the line is column 40.
        assert_eq!(a.title, Some(Rect::new(0, 0, 40, 1)));
        assert_eq!(a.body, Rect::new(0, 1, 40, 19));
        assert_eq!(b.whole(), Rect::new(41, 0, 40, 10));
        assert_eq!(c.title, Some(Rect::new(41, 10, 40, 1)));
        assert_eq!(c.body, Rect::new(41, 11, 40, 9));
        assert_eq!(layout.divider_at(40, 5).unwrap().split, 10);
        assert_eq!(layout.divider_at(60, 10).unwrap().split, 11);
        assert_eq!(layout.pane_at(45, 15).unwrap().id, C);
    }

    #[test]
    fn neighbors_are_found_by_direction() {
        let layout = three().layout(Rect::new(0, 0, 81, 20));
        assert_eq!(layout.neighbor(A, Direction::Right), Some(B));
        assert_eq!(layout.neighbor(C, Direction::Left), Some(A));
        assert_eq!(layout.neighbor(B, Direction::Down), Some(C));
        assert_eq!(layout.neighbor(C, Direction::Up), Some(B));
        assert_eq!(layout.neighbor(A, Direction::Left), None);
        assert_eq!(layout.neighbor(B, Direction::Up), None);
    }

    #[test]
    fn resizing_follows_the_mouse_and_keeps_both_halves() {
        let mut tree = three();
        let area = Rect::new(0, 0, 81, 20);
        let divider = tree.layout(area).dividers[0];
        tree.resize(divider.split, ratio_at(&divider, 20, 5));
        assert_eq!(tree.layout(area).area_of(A).unwrap().body.width, 20);
        tree.resize(divider.split, 0.0);
        assert_eq!(tree.layout(area).area_of(A).unwrap().body.width, 7);
        tree.resize(divider.split, f32::NAN);
        assert_eq!(tree.layout(area).area_of(A).unwrap().body.width, 40);
    }

    fn widths(tree: &Tree, area: Rect) -> Vec<u16> {
        let layout = tree.layout(area);
        tree.panes()
            .iter()
            .map(|&id| layout.area_of(id).unwrap().body.width)
            .collect()
    }

    #[test]
    fn a_row_is_shared_out_evenly_as_it_grows_and_shrinks() {
        // Splitting the newest pane each time: a quarter each, not a half, a
        // quarter, an eighth. 83 columns: four panes and three lines.
        let area = Rect::new(0, 0, 83, 20);
        let mut tree = Tree::Pane(A);
        tree.split(A, B, Axis::Right, 10);
        tree.split(B, C, Axis::Right, 11);
        tree.split(C, PaneId(4), Axis::Right, 12);
        assert_eq!(widths(&tree, area), [20, 20, 20, 20]);
        // Splitting the first pane: it joins the same row.
        let mut first = Tree::Pane(A);
        first.split(A, B, Axis::Right, 10);
        first.split(A, C, Axis::Right, 11);
        assert_eq!(first.panes(), [A, C, B]);
        assert_eq!(widths(&first, Rect::new(0, 0, 32, 20)), [10, 10, 10]);
        // Closing one shares the rest out again, wherever it was.
        tree.remove(C);
        assert_eq!(widths(&tree, area), [27, 27, 27]);
        tree.remove(A);
        assert_eq!(widths(&tree, area), [41, 41]);
        // A column, too: each pane is a title row and its body.
        let mut column = Tree::Pane(A);
        column.split(A, B, Axis::Down, 10);
        column.split(B, C, Axis::Down, 11);
        column.split(C, PaneId(4), Axis::Down, 12);
        let layout = column.layout(Rect::new(0, 0, 80, 40));
        for id in column.panes() {
            assert_eq!(layout.area_of(id).unwrap().whole().height, 10);
        }
    }

    #[test]
    fn a_row_inside_another_is_shared_out_on_its_own() {
        // A | B, then C below B, then D beside C: A keeps half the width, and
        // C and D share B's half below it.
        let mut tree = three();
        tree.split(C, PaneId(4), Axis::Right, 12);
        let layout = tree.layout(Rect::new(0, 0, 81, 20));
        assert_eq!(layout.area_of(A).unwrap().body.width, 40);
        assert_eq!(layout.area_of(B).unwrap().body.width, 40);
        assert_eq!(layout.area_of(C).unwrap().body.width, 20);
        assert_eq!(layout.area_of(PaneId(4)).unwrap().body.width, 19);
        // A dragged line elsewhere is kept by a split in another row.
        let mut tree = three();
        tree.resize(10, 0.3);
        let before = tree
            .layout(Rect::new(0, 0, 81, 20))
            .area_of(A)
            .unwrap()
            .body;
        tree.split(C, PaneId(4), Axis::Right, 12);
        let after = tree
            .layout(Rect::new(0, 0, 81, 20))
            .area_of(A)
            .unwrap()
            .body;
        assert_eq!(before, after);
    }

    #[test]
    fn panes_are_tiled_in_reading_order() {
        let ids: Vec<PaneId> = (1..=5).map(PaneId).collect();
        let mut next = 0;
        let tree = Tree::grid(&ids, &mut next).unwrap();
        assert_eq!(tree.panes(), ids);
        assert_eq!(next, 4);
        // Three above, two below, each row even, the rows the same height.
        let layout = tree.layout(Rect::new(0, 0, 62, 20));
        let at = |n: u32| layout.area_of(PaneId(n)).unwrap().whole();
        assert_eq!([at(1).y, at(2).y, at(3).y], [0, 0, 0]);
        assert_eq!([at(4).y, at(5).y], [10, 10]);
        assert_eq!([at(1).width, at(2).width, at(3).width], [20, 20, 20]);
        assert_eq!([at(4).width, at(5).width], [31, 30]);
        // Two side by side, one alone, none at all.
        let two = Tree::grid(&[A, B], &mut next).unwrap();
        assert_eq!(widths(&two, Rect::new(0, 0, 81, 20)), [40, 40]);
        assert_eq!(Tree::grid(&[A], &mut next), Some(Tree::Pane(A)));
        assert_eq!(Tree::grid(&[], &mut next), None);
    }

    #[test]
    fn a_tiny_area_still_lays_out() {
        let layout = three().layout(Rect::new(0, 0, 2, 1));
        assert_eq!(layout.panes.len(), 3);
        let layout = three().layout(Rect::new(0, 0, 0, 0));
        assert!(layout.panes.iter().all(|p| p.body.area() == 0));
    }
}
