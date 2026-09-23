use std::collections::HashMap;

use crate::git::types::{BranchEntry, PrDetail, PullRequest};

use super::MainFilter;

/// Which review requests the Review view includes. Cycled with `t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewScope {
    /// Personal requests, PRs reviewed by me, and requests to the teams
    /// listed in `review.teams`.
    Custom,
    /// Personal requests and PRs reviewed by me.
    #[default]
    OnlyMe,
    /// Personal requests, PRs reviewed by me, and requests to any team I
    /// belong to.
    All,
}

impl ReviewScope {
    /// Startup scope: Custom when teams are configured, otherwise OnlyMe.
    pub fn initial(has_teams: bool) -> Self {
        if has_teams {
            Self::Custom
        } else {
            Self::OnlyMe
        }
    }

    /// Next scope in the `t` cycle (Custom → OnlyMe → All → Custom).
    /// Custom is skipped when no teams are configured.
    pub fn next(self, has_teams: bool) -> Self {
        match self {
            Self::Custom => Self::OnlyMe,
            Self::OnlyMe => Self::All,
            Self::All => Self::initial(has_teams),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::OnlyMe => "me",
            Self::All => "all",
        }
    }
}

/// Per-filter PR list caches keyed by `MainFilter`, their fetch parameters,
/// and the PR-detail cache (issue #220).
#[derive(Default)]
pub struct PrCaches {
    pub local: Vec<PullRequest>,
    pub my: Vec<PullRequest>,
    pub review: Vec<PullRequest>,
    pub local_loaded: bool,
    pub my_loaded: bool,
    pub review_loaded: bool,
    pub show_merged: bool,
    pub review_scope: ReviewScope,
    /// PR detail bodies for the detail pane, cached by `(RepoId, PR number)`.
    pub detail: HashMap<(crate::git::types::RepoId, u64), PrDetail>,
}

impl PrCaches {
    /// Cached PR detail for `entry`, if the entry has a PR and it is cached.
    pub fn detail_for(&self, entry: &BranchEntry) -> Option<&PrDetail> {
        let pr_num = entry.pr_number()?;
        self.detail.get(&(entry.repo_id.clone(), pr_num))
    }

    pub fn current(&self, filter: MainFilter) -> &[PullRequest] {
        match filter {
            MainFilter::Local => &self.local,
            MainFilter::MyPr => &self.my,
            MainFilter::ReviewRequested => &self.review,
        }
    }

    /// A filter's list counts as loading until its first fetch lands.
    pub fn is_loading(&self, filter: MainFilter) -> bool {
        match filter {
            MainFilter::Local => !self.local_loaded,
            MainFilter::MyPr => !self.my_loaded,
            MainFilter::ReviewRequested => !self.review_loaded,
        }
    }

    /// Store a fetched PR list for `filter` and mark it loaded.
    pub fn set(&mut self, filter: MainFilter, prs: Vec<PullRequest>) {
        match filter {
            MainFilter::Local => {
                self.local = prs;
                self.local_loaded = true;
            }
            MainFilter::MyPr => {
                self.my = prs;
                self.my_loaded = true;
            }
            MainFilter::ReviewRequested => {
                self.review = prs;
                self.review_loaded = true;
            }
        }
    }

    /// Drop a filter's cached list so the next fetch is forced.
    pub fn invalidate(&mut self, filter: MainFilter) {
        match filter {
            MainFilter::Local => {
                self.local.clear();
                self.local_loaded = false;
            }
            MainFilter::MyPr => {
                self.my.clear();
                self.my_loaded = false;
            }
            MainFilter::ReviewRequested => {
                self.review.clear();
                self.review_loaded = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_scope_follows_team_config() {
        assert_eq!(ReviewScope::initial(true), ReviewScope::Custom);
        assert_eq!(ReviewScope::initial(false), ReviewScope::OnlyMe);
    }

    #[test]
    fn cycle_with_teams_visits_all_three() {
        let s = ReviewScope::Custom;
        let s = s.next(true);
        assert_eq!(s, ReviewScope::OnlyMe);
        let s = s.next(true);
        assert_eq!(s, ReviewScope::All);
        assert_eq!(s.next(true), ReviewScope::Custom);
    }

    #[test]
    fn cycle_without_teams_skips_custom() {
        let s = ReviewScope::OnlyMe.next(false);
        assert_eq!(s, ReviewScope::All);
        assert_eq!(s.next(false), ReviewScope::OnlyMe);
    }
}
