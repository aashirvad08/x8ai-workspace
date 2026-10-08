//! A list in the sidebar: headings, notes and rows that can be selected, as
//! the Models, MCP, Catalog and Add-ons panels show them. A row has a key
//! saying what it is, so the selection stays on the same thing when the list
//! is read again.

/// How a row's detail is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    /// Ready, on, saved.
    Good,
    /// Needs something: a key, a secret, an install.
    Wanting,
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item<K> {
    Heading(String),
    Note(String),
    Blank,
    Row {
        key: K,
        label: String,
        detail: String,
        tone: Tone,
        /// Indented under the row before it (a model under its provider).
        nested: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Listing<K> {
    pub items: Vec<Item<K>>,
    /// Which row is selected, counting rows only.
    pub selected: usize,
    /// The first item shown.
    pub offset: usize,
    /// What the list is filtered by, if anything (the Catalog).
    pub filter: String,
    /// The filter is being typed.
    pub filtering: bool,
}

impl<K> Default for Listing<K> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            offset: 0,
            filter: String::new(),
            filtering: false,
        }
    }
}

impl<K: Clone + PartialEq> Listing<K> {
    /// Replaces the items; the selection stays on the same key if it is still
    /// there.
    pub fn set(&mut self, items: Vec<Item<K>>) {
        let was = self.key().cloned();
        self.items = items;
        let rows = self.rows();
        self.selected = was
            .and_then(|was| self.keys().position(|k| *k == was))
            .unwrap_or(self.selected)
            .min(rows.saturating_sub(1));
        self.offset = self.offset.min(self.items.len().saturating_sub(1));
    }

    fn keys(&self) -> impl Iterator<Item = &K> {
        self.items.iter().filter_map(|item| match item {
            Item::Row { key, .. } => Some(key),
            _ => None,
        })
    }

    pub fn rows(&self) -> usize {
        self.keys().count()
    }

    /// The selected row's key.
    pub fn key(&self) -> Option<&K> {
        self.keys().nth(self.selected)
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    /// The item index of row `row`.
    fn item_of(&self, row: usize) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item, Item::Row { .. }))
            .nth(row)
            .map(|(index, _)| index)
    }

    /// The row shown at line `line` of the list (from its first line), if any.
    pub fn row_at(&self, line: usize) -> Option<usize> {
        let index = self.offset + line;
        if !matches!(self.items.get(index), Some(Item::Row { .. })) {
            return None;
        }
        Some(
            self.items[..index]
                .iter()
                .filter(|item| matches!(item, Item::Row { .. }))
                .count(),
        )
    }

    /// Scrolls so the selection shows in `height` lines, with the heading
    /// above the first row.
    pub fn keep_in_view(&mut self, height: usize) {
        let Some(at) = self.item_of(self.selected) else {
            self.offset = 0;
            return;
        };
        if height == 0 {
            return;
        }
        let top = if self.selected == 0 { 0 } else { at };
        if top < self.offset {
            self.offset = top;
        } else if at >= self.offset + height {
            self.offset = at + 1 - height;
        }
    }

    /// Scrolls the view by `delta` lines (the mouse wheel).
    pub fn scroll(&mut self, delta: isize, height: usize) {
        let most = self.items.len().saturating_sub(height);
        self.offset = self.offset.saturating_add_signed(delta).min(most);
    }
}

/// Whether `text` matches the filter: every word of it, ignoring case.
pub fn matches(filter: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    filter
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: u32) -> Item<u32> {
        Item::Row {
            key,
            label: format!("row {key}"),
            detail: String::new(),
            tone: Tone::Plain,
            nested: false,
        }
    }

    #[test]
    fn the_selection_follows_its_key() {
        let mut list = Listing::default();
        list.set(vec![Item::Heading("A".into()), row(1), row(2), row(3)]);
        list.move_by(2);
        assert_eq!(list.key(), Some(&3));
        list.set(vec![row(3), row(4)]);
        assert_eq!(list.key(), Some(&3));
        list.set(vec![row(5)]);
        assert_eq!(list.key(), Some(&5));
    }

    #[test]
    fn lines_map_to_rows_past_headings() {
        let mut list = Listing::default();
        list.set(vec![
            Item::Heading("A".into()),
            row(1),
            Item::Blank,
            Item::Heading("B".into()),
            row(2),
        ]);
        assert_eq!(list.row_at(0), None);
        assert_eq!(list.row_at(1), Some(0));
        assert_eq!(list.row_at(4), Some(1));
        list.move_by(1);
        list.keep_in_view(2);
        assert_eq!(list.offset, 3);
        list.move_by(-1);
        list.keep_in_view(2);
        assert_eq!(list.offset, 0);
    }

    #[test]
    fn filters_match_every_word() {
        assert!(matches("git hub", "GitHub MCP server"));
        assert!(!matches("gitlab", "GitHub"));
        assert!(matches("", "anything"));
    }
}
