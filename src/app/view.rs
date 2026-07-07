use std::collections::HashSet;

use crate::git::types::BranchEntry;

use super::MainFilter;

pub enum SidebarRow<'a> {
    Header { repo_id: crate::git::types::RepoId },
    Entry(&'a crate::git::types::BranchEntry),
}

/// Which Main-view pane receives `j`/`k` input (issue #269).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PaneFocus {
    #[default]
    Sidebar,
    Detail,
}

/// What the user is looking at: the main-view entry list and its
/// cursor/filter/search/multi-select state, plus the scroll positions of the
/// other views (issue #220).
#[derive(Default)]
pub struct ViewState {
    pub entries: Vec<BranchEntry>,
    pub main_filter: MainFilter,
    pub sidebar_scroll: usize,
    pub sidebar_offset: usize,
    pub search_active: bool,
    pub search_query: String,
    pub(super) search_pre_scroll: usize, // saved scroll position before search
    /// Branch names multi-selected in the sidebar (space / `a` keys).
    pub branch_selected: HashSet<String>,
    pub log_scroll: usize,
    pub history_scroll: usize,
    pub pr_detail_scroll: usize,
    pub pane_focus: PaneFocus,
}

impl ViewState {
    pub fn adjust_sidebar_offset(&mut self, visible_height: usize, item_count: usize) {
        if visible_height == 0 {
            return;
        }
        // Clamp scroll to valid range
        if item_count > 0 {
            self.sidebar_scroll = self.sidebar_scroll.min(item_count - 1);
        } else {
            self.sidebar_scroll = 0;
        }
        // Adjust offset when cursor exceeds viewport bounds
        if self.sidebar_scroll >= self.sidebar_offset + visible_height {
            self.sidebar_offset = self.sidebar_scroll - visible_height + 1;
        }
        if self.sidebar_scroll < self.sidebar_offset {
            self.sidebar_offset = self.sidebar_scroll;
        }
        // Clamp offset so list doesn't show blank space
        let max_offset = item_count.saturating_sub(visible_height);
        self.sidebar_offset = self.sidebar_offset.min(max_offset);
    }

    /// Clamp the PR-detail scroll so the last page stays visible. Called
    /// during render (like `adjust_sidebar_offset`), where the viewport
    /// height and the wrapped line count are known.
    pub fn clamp_pr_detail_scroll(&mut self, visible_height: usize, total_lines: usize) {
        if visible_height == 0 {
            return;
        }
        self.pr_detail_scroll = self
            .pr_detail_scroll
            .min(total_lines.saturating_sub(visible_height));
    }

    pub fn filtered_entries(&self) -> Vec<&BranchEntry> {
        let search_query = if !self.search_query.is_empty() {
            Some(self.search_query.to_lowercase())
        } else {
            None
        };
        self.entries
            .iter()
            .filter(|entry| {
                let passes_filter = match self.main_filter {
                    MainFilter::Local => entry.has_local(),
                    // My PR / Review: server-side filtered, just check PR exists
                    MainFilter::MyPr | MainFilter::ReviewRequested => entry.pull_request.is_some(),
                };
                let passes_search = match &search_query {
                    Some(q) => entry.name.to_lowercase().contains(q.as_str()),
                    None => true,
                };
                passes_filter && passes_search
            })
            .collect()
    }

    /// Build the sidebar rendering rows. Returns a flat list of headers and
    /// entries; cross-repo grouping kicks in only for My PR / Review when
    /// `entries` span more than one repo.
    pub fn sidebar_rows(&self) -> Vec<SidebarRow<'_>> {
        let filtered = self.filtered_entries();
        let group_active = matches!(
            self.main_filter,
            MainFilter::MyPr | MainFilter::ReviewRequested
        );
        let repo_set: std::collections::HashSet<_> =
            filtered.iter().map(|e| e.repo_id.clone()).collect();
        let do_group = group_active && repo_set.len() > 1;

        let mut rows = Vec::with_capacity(filtered.len() + repo_set.len());
        if !do_group {
            for e in filtered {
                rows.push(SidebarRow::Entry(e));
            }
            return rows;
        }
        let mut last_repo: Option<crate::git::types::RepoId> = None;
        for e in filtered {
            if last_repo.as_ref() != Some(&e.repo_id) {
                rows.push(SidebarRow::Header {
                    repo_id: e.repo_id.clone(),
                });
                last_repo = Some(e.repo_id.clone());
            }
            rows.push(SidebarRow::Entry(e));
        }
        rows
    }

    pub fn selected_entry(&self) -> Option<&BranchEntry> {
        let rows = self.sidebar_rows();
        match rows.get(self.sidebar_scroll)? {
            SidebarRow::Entry(e) => Some(*e),
            SidebarRow::Header { .. } => None,
        }
    }

    /// If `sidebar_scroll` lands on a Header, advance to the next Entry. If past
    /// the end, clamp to the last Entry. No-op if already on an Entry.
    pub fn snap_scroll_to_entry(&mut self) {
        // Collect row kinds into a plain bool vec (true = is_header) to avoid holding
        // a borrow on self while mutating sidebar_scroll.
        let is_header: Vec<bool> = self
            .sidebar_rows()
            .iter()
            .map(|r| matches!(r, SidebarRow::Header { .. }))
            .collect();
        if is_header.is_empty() {
            self.sidebar_scroll = 0;
            return;
        }
        while self.sidebar_scroll < is_header.len() && is_header[self.sidebar_scroll] {
            self.sidebar_scroll += 1;
        }
        if self.sidebar_scroll >= is_header.len() {
            self.sidebar_scroll = is_header.len().saturating_sub(1);
        }
    }

    /// Find the next entry-row index after `from`, skipping headers. None if no entry follows.
    pub(super) fn next_entry_index(&self, from: usize) -> Option<usize> {
        let rows = self.sidebar_rows();
        let mut next = from + 1;
        while next < rows.len() && matches!(rows[next], SidebarRow::Header { .. }) {
            next += 1;
        }
        if next < rows.len() { Some(next) } else { None }
    }

    /// Find the previous entry-row index before `from`, skipping headers. None if no entry precedes.
    pub(super) fn prev_entry_index(&self, from: usize) -> Option<usize> {
        if from == 0 {
            return None;
        }
        let rows = self.sidebar_rows();
        let mut prev = from - 1;
        while matches!(rows.get(prev), Some(SidebarRow::Header { .. })) {
            if prev == 0 {
                return None;
            }
            prev -= 1;
        }
        Some(prev)
    }
}

#[cfg(test)]
mod view_state_tests {
    use super::*;

    #[test]
    fn clamp_pr_detail_scroll_bounds() {
        let mut view = ViewState::default();
        view.pr_detail_scroll = 100;
        view.clamp_pr_detail_scroll(10, 50);
        assert_eq!(view.pr_detail_scroll, 40);

        view.pr_detail_scroll = 5;
        view.clamp_pr_detail_scroll(10, 50);
        assert_eq!(view.pr_detail_scroll, 5);

        // Content shorter than the viewport pins to the top.
        view.pr_detail_scroll = 5;
        view.clamp_pr_detail_scroll(10, 3);
        assert_eq!(view.pr_detail_scroll, 0);

        // Zero-height viewport is a no-op guard.
        view.pr_detail_scroll = 7;
        view.clamp_pr_detail_scroll(0, 50);
        assert_eq!(view.pr_detail_scroll, 7);
    }
}
