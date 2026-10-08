//! Spaces: every folder opened as a workspace, and the workspace with no folder,
//! each with an id of its own and the add-ons added to it (ADR 0019).
//!
//! An id is made the first time a space is opened and never changes, so a space
//! can be named by it (`/share ws-k3f9qa`). Like the other stores, the file is
//! the user's only (mode 0600), replaced atomically, and written only by the
//! native side. It holds add-on ids, never commands: what an add-on runs is
//! defined in the app.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::store::{Rooted, load, now, save};

/// The letters of an id after `ws-`: lowercase letters and digits that cannot be
/// misread for one another (no `l`, `o`, `0` or `1`).
const ID_ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
const ID_PREFIX: &str = "ws-";
const ID_LETTERS: usize = 6;

/// Every space opened so far.
#[derive(Debug)]
pub struct SpaceStore {
    file: PathBuf,
    entries: Vec<SpaceEntry>,
}

/// A space as the store knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub id: String,
    /// `None`: the workspace with no folder.
    pub root: Option<PathBuf>,
    /// Add-on ids, in the order they were added.
    pub addons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpaceEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root: Option<PathBuf>,
    id: String,
    #[serde(default)]
    addons: Vec<String>,
    /// Milliseconds since the Unix epoch: when the space got its id.
    at: u64,
}

impl Rooted for SpaceEntry {
    fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }
}

impl SpaceEntry {
    fn space(&self) -> Space {
        Space {
            id: self.id.clone(),
            root: self.root.clone(),
            addons: self.addons.clone(),
        }
    }
}

/// Whether `id` is shaped as a space id (`ws-k3f9qa`).
pub fn is_space_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|letters| {
        letters.len() == ID_LETTERS && letters.bytes().all(|b| ID_ALPHABET.contains(&b))
    })
}

/// Where `/new <name>` makes its folders, in the home folder.
pub const NEW_SPACES_FOLDER: &str = "Workspaces";

/// What a new space's folder may be named, for an error message.
pub const NEW_SPACE_NAME_RULE: &str =
    "a new space needs a folder name: letters, digits, spaces, - or _, without / or a leading dot";

/// `name`, trimmed, if it can name a new space's folder (`/new <name>`): one
/// visible folder name, no path, nothing a shell or Finder would trip over.
pub fn new_space_name(name: &str) -> Option<&str> {
    let name = name.trim();
    let fine = !name.is_empty()
        && name.chars().count() <= 64
        && !name.starts_with('.')
        && !name.contains('/')
        && !name.contains(':')
        && !name.chars().any(char::is_control);
    fine.then_some(name)
}

/// Whether `id` is shaped as an add-on id (`starship`, `nerd-font`). Which ids
/// exist is the add-on registry's business.
fn is_addon_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl SpaceStore {
    /// Loads the spaces. A missing file is none; see `store::load` for a damaged
    /// one. Entries with a malformed id, or a root or id seen before, are dropped,
    /// and so are malformed add-on ids.
    pub fn load(file: PathBuf) -> (Self, Option<String>) {
        let (loaded, warning) = load::<SpaceEntry>(&file);
        let mut entries: Vec<SpaceEntry> = Vec::with_capacity(loaded.len());
        for mut entry in loaded {
            if !is_space_id(&entry.id)
                || entries
                    .iter()
                    .any(|e| e.id == entry.id || e.root == entry.root)
            {
                continue;
            }
            let mut addons: Vec<String> = Vec::new();
            for addon in entry.addons.drain(..) {
                if is_addon_id(&addon) && !addons.contains(&addon) {
                    addons.push(addon);
                }
            }
            entry.addons = addons;
            entries.push(entry);
        }
        (Self { file, entries }, warning)
    }

    /// The space of `root` (`None`: no folder), if it has an id yet.
    pub fn get(&self, root: Option<&Path>) -> Option<Space> {
        self.entries
            .iter()
            .find(|e| e.root.as_deref() == root)
            .map(SpaceEntry::space)
    }

    pub fn by_id(&self, id: &str) -> Option<Space> {
        self.entries
            .iter()
            .find(|e| e.id == id)
            .map(SpaceEntry::space)
    }

    /// Every space, the workspace with no folder first, then in the order they
    /// were first opened.
    pub fn list(&self) -> Vec<Space> {
        let mut spaces: Vec<Space> = self.entries.iter().map(SpaceEntry::space).collect();
        spaces.sort_by_key(|s| s.root.is_some());
        spaces
    }

    /// The space of `root`, given an id now if it has none.
    pub fn ensure(&mut self, root: Option<&Path>) -> Result<Space, Error> {
        if let Some(space) = self.get(root) {
            return Ok(space);
        }
        let entry = SpaceEntry {
            root: root.map(Path::to_owned),
            id: self.new_id(),
            addons: Vec::new(),
            at: now(),
        };
        let space = entry.space();
        self.entries.push(entry);
        self.save()?;
        Ok(space)
    }

    /// Adds the add-ons to the space of `root`, in order, after those it has.
    pub fn add(&mut self, root: Option<&Path>, addons: &[&str]) -> Result<Space, Error> {
        let id = self.ensure(root)?.id;
        self.add_to(&id, addons)
    }

    /// Adds the add-ons to the space with this id, as sharing does.
    pub fn add_to(&mut self, id: &str, addons: &[&str]) -> Result<Space, Error> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| Error::NotFound(id.to_owned()))?;
        let before = entry.addons.len();
        for addon in addons {
            if is_addon_id(addon) && !entry.addons.iter().any(|a| a == addon) {
                entry.addons.push((*addon).to_owned());
            }
        }
        let space = entry.space();
        if space.addons.len() != before {
            self.save()?;
        }
        Ok(space)
    }

    /// Takes the add-on out of the space of `root`. Other spaces keep it.
    pub fn remove(&mut self, root: Option<&Path>, addon: &str) -> Result<Space, Error> {
        let id = self.ensure(root)?.id;
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .expect("just ensured");
        let before = entry.addons.len();
        entry.addons.retain(|a| a != addon);
        let space = entry.space();
        if space.addons.len() != before {
            self.save()?;
        }
        Ok(space)
    }

    fn save(&self) -> Result<(), Error> {
        save(&self.file, &self.entries)
    }

    /// A new id, unlike every other: six letters from the process's random hash
    /// keys, which the standard library seeds from the operating system.
    fn new_id(&self) -> String {
        let random = RandomState::new();
        (0u64..)
            .map(|attempt| {
                let mut hasher = random.build_hasher();
                hasher.write_u64(now());
                hasher.write_u64(attempt);
                let mut bits = hasher.finish();
                let mut id = String::from(ID_PREFIX);
                for _ in 0..ID_LETTERS {
                    id.push(char::from(ID_ALPHABET[(bits % 32) as usize]));
                    bits /= 32;
                }
                id
            })
            .find(|id| self.by_id(id).is_none())
            .expect("an unused id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SpaceStore) {
        let dir = tempfile::tempdir().unwrap();
        let (store, warning) = SpaceStore::load(dir.path().join("spaces.json"));
        assert!(warning.is_none());
        (dir, store)
    }

    #[test]
    fn a_new_space_is_named_by_one_folder_name() {
        assert_eq!(new_space_name("  demo app "), Some("demo app"));
        assert_eq!(new_space_name("gym-RL_2"), Some("gym-RL_2"));
        for bad in [
            "",
            "  ",
            ".hidden",
            "..",
            "a/b",
            "../etc",
            "a:b",
            "x\u{0}y",
            &"n".repeat(65),
        ] {
            assert_eq!(new_space_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_space_gets_one_id_for_good() {
        let (dir, mut spaces) = store();
        let root = Path::new("/Users/me/gymRL");
        let first = spaces.ensure(Some(root)).unwrap();
        assert!(is_space_id(&first.id), "{}", first.id);
        assert_eq!(spaces.ensure(Some(root)).unwrap().id, first.id);

        let (reloaded, _) = SpaceStore::load(dir.path().join("spaces.json"));
        assert_eq!(reloaded.get(Some(root)).unwrap().id, first.id);
    }

    #[test]
    fn every_space_has_its_own_id_and_the_home_space_is_one_of_them() {
        let (_dir, mut spaces) = store();
        let home = spaces.ensure(None).unwrap();
        let a = spaces.ensure(Some(Path::new("/a"))).unwrap();
        let b = spaces.ensure(Some(Path::new("/b"))).unwrap();
        assert!(home.root.is_none());
        assert!(home.id != a.id && a.id != b.id && home.id != b.id);
        assert_eq!(spaces.list()[0].id, home.id);
    }

    #[test]
    fn add_ons_belong_to_one_space() {
        let (_dir, mut spaces) = store();
        let a = Path::new("/a");
        let b = Path::new("/b");
        spaces
            .add(Some(a), &["starship", "fzf", "starship"])
            .unwrap();
        spaces.ensure(Some(b)).unwrap();
        assert_eq!(spaces.get(Some(a)).unwrap().addons, ["starship", "fzf"]);
        assert!(spaces.get(Some(b)).unwrap().addons.is_empty());

        spaces.remove(Some(a), "starship").unwrap();
        assert_eq!(spaces.get(Some(a)).unwrap().addons, ["fzf"]);
    }

    #[test]
    fn sharing_adds_to_what_the_other_space_has() {
        let (_dir, mut spaces) = store();
        spaces
            .add(Some(Path::new("/a")), &["starship", "fzf"])
            .unwrap();
        let b = spaces.add(Some(Path::new("/b")), &["zoxide"]).unwrap();
        let shared = spaces.add_to(&b.id, &["starship", "fzf"]).unwrap();
        assert_eq!(shared.addons, ["zoxide", "starship", "fzf"]);
        assert!(matches!(
            spaces.add_to("ws-zzzzzz", &["fzf"]),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn a_damaged_or_tampered_file_keeps_only_well_formed_entries() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("spaces.json");
        std::fs::write(
            &file,
            r#"{"version":1,"workspaces":[
                {"root":"/a","id":"ws-abcdef","addons":["starship","; rm -rf ~","starship"],"at":1},
                {"root":"/b","id":"ws-abcdef","addons":[],"at":2},
                {"root":"relative","id":"ws-ghijkm","addons":[],"at":3},
                {"root":"/c","id":"not-an-id","addons":[],"at":4}
            ]}"#,
        )
        .unwrap();
        let (spaces, warning) = SpaceStore::load(file);
        assert!(warning.is_none());
        let list = spaces.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].addons, ["starship"]);
    }

    #[test]
    fn ids_are_shaped_as_ids() {
        assert!(is_space_id("ws-k3f9qa"));
        for bad in [
            "ws-k3f9q",
            "ws-k3f9qa1",
            "WS-k3f9qa",
            "ws-k3f9q0",
            "k3f9qa",
            "ws-../..x",
        ] {
            assert!(!is_space_id(bad), "{bad}");
        }
    }
}
