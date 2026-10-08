//! The layout of one tab: a binary tree whose leaves are panes, as in the app
//! (`src/terminal/panes.ts`), placed on the terminal's grid of cells.
//!
//! Panes side by side are separated by a column of `│`. When a tab has more
//! than one pane each gets a title row, which also separates panes one above
//! the other.

use ratatui::layout::Rect;

use crate::pane::PaneId;

/// No pane gets less than this share of its split.
pub const MIN_RATIO: f32 = 0.1;

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
        /// The first half's share, between MIN_RATIO and 1 - MIN_RATIO.
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

    /// Splits pane `target` in two: `added` goes right of it or below it.
    /// Returns whether `target` was found.
    pub fn split(&mut self, target: PaneId, added: PaneId, axis: Axis, id: u32) -> bool {
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
                first.split(target, added, axis, id) || second.split(target, added, axis, id)
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

    /// Removes pane `target`; its sibling takes the space. Returns false when
    /// `target` was the whole tree, which is then left as it was.
    pub fn remove(&mut self, target: PaneId) -> bool {
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
                first.remove(target);
                second.remove(target);
            }
        }
        true
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
                        let usable = area.width.saturating_sub(1);
                        let width = share(usable, *ratio);
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
                                width: usable - width,
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
    let (start, length, at) = match divider.axis {
        Axis::Right => (divider.area.x, divider.area.width.saturating_sub(1), x),
        Axis::Down => (divider.area.y, divider.area.height, y),
    };
    if length == 0 {
        return 0.5;
    }
    f32::from(at.saturating_sub(start)) / f32::from(length)
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
        assert_eq!(tree.layout(area).area_of(A).unwrap().body.width, 8);
        tree.resize(divider.split, f32::NAN);
        assert_eq!(tree.layout(area).area_of(A).unwrap().body.width, 40);
    }

    #[test]
    fn a_tiny_area_still_lays_out() {
        let layout = three().layout(Rect::new(0, 0, 2, 1));
        assert_eq!(layout.panes.len(), 3);
        let layout = three().layout(Rect::new(0, 0, 0, 0));
        assert!(layout.panes.iter().all(|p| p.body.area() == 0));
    }
}
