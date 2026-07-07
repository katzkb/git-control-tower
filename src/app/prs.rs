use std::collections::HashMap;

use crate::git::types::{BranchEntry, PrDetail, PullRequest};

use super::MainFilter;

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
    pub include_team_reviews: bool,
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
