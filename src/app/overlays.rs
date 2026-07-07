use crate::ui::confirm_dialog::ConfirmDialog;
use crate::ui::notification::Notification;

use super::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionItem {
    CreateWorktree,
    CdIntoWorktree,
    DeleteWorktree,
    CreateBranch,
    DeleteBranch,
    OpenPrInBrowser,
    CopyBranchName,
}

impl ActionItem {
    /// Short label shown inside the action menu; the group header
    /// (`group_label`) carries the "worktree"/"branch" context.
    pub fn label(&self) -> &'static str {
        match self {
            Self::CreateWorktree => "Create",
            Self::CdIntoWorktree => "Go to (cd)",
            Self::DeleteWorktree => "Delete",
            Self::CreateBranch => "Create from this",
            Self::DeleteBranch => "Delete",
            Self::OpenPrInBrowser => "Open PR in browser",
            Self::CopyBranchName => "Copy branch name",
        }
    }

    /// Section header shown above this item's group in the action menu;
    /// None = ungrouped top section.
    pub fn group_label(&self) -> Option<&'static str> {
        match self {
            Self::OpenPrInBrowser | Self::CopyBranchName => None,
            Self::CdIntoWorktree | Self::CreateWorktree | Self::DeleteWorktree => Some("Worktree"),
            Self::CreateBranch | Self::DeleteBranch => Some("Branch"),
        }
    }
}

/// A confirm dialog plus the command to run when the user accepts it.
/// Carrying the consequence with the dialog means `handle_confirm_key`
/// needs no out-of-band staging state to know what `y` applies to.
pub struct PendingConfirm {
    pub dialog: ConfirmDialog,
    pub on_confirm: Command,
}

pub struct ActionMenu {
    pub items: Vec<ActionItem>,
    pub scroll: usize,
    /// `(repo_id, branch_name)` captured when the menu was opened. Carrying
    /// both fields prevents wrong-repo lookup when two repos share a branch name.
    pub target: (crate::git::types::RepoId, String),
    pub footer: Option<String>,
}

pub struct BranchCreateInput {
    pub source: String,
    pub name: String,
    pub cursor: usize,
}

/// Modal/overlay UI state, listed in `handle_key` priority order (issue #220).
#[derive(Default)]
pub struct Overlays {
    pub show_help: bool,
    pub confirm_dialog: Option<PendingConfirm>,
    pub action_menu: Option<ActionMenu>,
    pub branch_create_input: Option<BranchCreateInput>,
    pub notification: Option<Notification>,
}

pub(super) fn insert_char_at(s: &mut String, char_idx: usize, c: char) {
    let byte_idx = char_to_byte_idx(s, char_idx);
    s.insert(byte_idx, c);
}

pub(super) fn remove_char_at(s: &mut String, char_idx: usize) {
    let byte_idx = char_to_byte_idx(s, char_idx);
    if byte_idx < s.len() {
        s.remove(byte_idx);
    }
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

#[cfg(test)]
mod text_edit_tests {
    use super::*;

    #[test]
    fn insert_at_start() {
        let mut s = String::from("bc");
        insert_char_at(&mut s, 0, 'a');
        assert_eq!(s, "abc");
    }

    #[test]
    fn insert_at_middle() {
        let mut s = String::from("ac");
        insert_char_at(&mut s, 1, 'b');
        assert_eq!(s, "abc");
    }

    #[test]
    fn insert_at_end() {
        let mut s = String::from("ab");
        insert_char_at(&mut s, 2, 'c');
        assert_eq!(s, "abc");
    }

    #[test]
    fn insert_unicode() {
        let mut s = String::from("αγ");
        insert_char_at(&mut s, 1, 'β');
        assert_eq!(s, "αβγ");
    }

    #[test]
    fn remove_at_start() {
        let mut s = String::from("abc");
        remove_char_at(&mut s, 0);
        assert_eq!(s, "bc");
    }

    #[test]
    fn remove_at_middle() {
        let mut s = String::from("abc");
        remove_char_at(&mut s, 1);
        assert_eq!(s, "ac");
    }

    #[test]
    fn remove_at_end_is_noop() {
        let mut s = String::from("abc");
        remove_char_at(&mut s, 3);
        assert_eq!(s, "abc");
    }

    #[test]
    fn remove_unicode() {
        let mut s = String::from("αβγ");
        remove_char_at(&mut s, 1);
        assert_eq!(s, "αγ");
    }
}
