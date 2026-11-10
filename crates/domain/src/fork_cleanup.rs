//! When the developer's fork of a third-party repository may be deleted.
//!
//! Only a real fork of that repository, owned by the developer, and only when
//! nothing would be lost: no pull request from it is still open, and every
//! branch either is exactly what a merged pull request delivered or has no
//! commit that the upstream default branch lacks.

use crate::{
    model::PrState,
    ports::{BranchPullRequest, ForgeBranch, ForgeRepository},
    values::{OwnerName, RepoSlug},
};

/// Why a fork is kept, in words for the logs.
pub type Keep = String;

/// The fork must be a fork of `upstream` that belongs to `me`.
pub fn check_ownership(fork: &ForgeRepository, upstream: &RepoSlug, me: &OwnerName) -> Result<(), Keep> {
    if !fork.is_fork {
        return Err(format!("{} is not a fork", fork.slug));
    }
    if !fork.slug.owner().same_as(me.as_str()) {
        return Err(format!("{} does not belong to {me}", fork.slug));
    }
    match &fork.parent {
        Some(parent) if parent.to_string().eq_ignore_ascii_case(&upstream.to_string()) => Ok(()),
        Some(parent) => Err(format!("{} is a fork of {parent}, not of {upstream}", fork.slug)),
        None => Err(format!("GitHub did not say which repository {} is a fork of", fork.slug)),
    }
}

/// What a branch of the fork needs before the fork can go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchVerdict {
    /// A merged pull request delivered exactly this commit.
    Delivered,
    /// No merged pull request at this commit: only safe if the upstream
    /// default branch already has all of its commits (see [`compared`]).
    NeedsComparison,
    Keep(Keep),
}

/// Judges a branch of the fork by the pull requests that came from it.
pub fn judge_branch(branch: &ForgeBranch, pulls: &[BranchPullRequest]) -> BranchVerdict {
    if let Some(open) = pulls.iter().find(|p| p.state == PrState::Open) {
        return BranchVerdict::Keep(format!("pull request #{} from {} is still open", open.number, branch.name));
    }
    if pulls.iter().any(|p| p.state == PrState::Merged && p.head == branch.head) {
        return BranchVerdict::Delivered;
    }
    BranchVerdict::NeedsComparison
}

/// A branch without a merged pull request at its commit is only safe to lose
/// when the upstream default branch has every commit it has.
pub fn compared(branch: &ForgeBranch, commits_ahead: u64) -> Result<(), Keep> {
    match commits_ahead {
        0 => Ok(()),
        n => Err(format!("branch {} has {n} commit(s) that are not merged", branch.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::{BranchName, CloneUrl, CommitSha, PrNumber};

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn repo(slug: &str, is_fork: bool, parent: Option<&str>) -> ForgeRepository {
        ForgeRepository {
            slug: RepoSlug::parse(slug).unwrap(),
            clone_url: CloneUrl::new(format!("https://github.com/{slug}.git")).unwrap(),
            default_branch: BranchName::new("main").unwrap(),
            is_fork,
            parent: parent.map(|p| RepoSlug::parse(p).unwrap()),
            read_only: false,
            can_push: true,
        }
    }

    fn branch(head: &str) -> ForgeBranch {
        ForgeBranch { name: BranchName::new("git-opt/optimize-images").unwrap(), head: CommitSha::new(head).unwrap() }
    }

    fn pull(number: i64, state: PrState, head: &str) -> BranchPullRequest {
        BranchPullRequest { number: PrNumber::new(number), state, head: CommitSha::new(head).unwrap() }
    }

    #[test]
    fn only_the_developers_own_fork_of_that_repository() {
        let upstream = RepoSlug::parse("other/lib").unwrap();
        let me = OwnerName::new("jdev").unwrap();
        assert!(check_ownership(&repo("jdev/lib", true, Some("Other/Lib")), &upstream, &me).is_ok());
        assert!(check_ownership(&repo("JDev/lib", true, Some("other/lib")), &upstream, &me).is_ok());
        assert!(check_ownership(&repo("jdev/lib", false, None), &upstream, &me).is_err(), "not a fork");
        assert!(check_ownership(&repo("acme/lib", true, Some("other/lib")), &upstream, &me).is_err(), "not mine");
        assert!(check_ownership(&repo("jdev/lib", true, Some("else/lib")), &upstream, &me).is_err(), "other parent");
        assert!(check_ownership(&repo("jdev/lib", true, None), &upstream, &me).is_err(), "parent unknown");
    }

    #[test]
    fn a_branch_is_delivered_only_by_a_merged_pull_request_at_its_commit() {
        assert_eq!(judge_branch(&branch(A), &[pull(1, PrState::Merged, A)]), BranchVerdict::Delivered);
        assert_eq!(
            judge_branch(&branch(A), &[pull(1, PrState::Closed, B), pull(2, PrState::Merged, A)]),
            BranchVerdict::Delivered
        );
        // Pushed again after the merge, rejected, or never proposed: look at the commits.
        assert_eq!(judge_branch(&branch(B), &[pull(1, PrState::Merged, A)]), BranchVerdict::NeedsComparison);
        assert_eq!(judge_branch(&branch(A), &[pull(1, PrState::Closed, A)]), BranchVerdict::NeedsComparison);
        assert_eq!(judge_branch(&branch(A), &[]), BranchVerdict::NeedsComparison);
    }

    #[test]
    fn an_open_pull_request_always_keeps_the_fork() {
        let verdict = judge_branch(&branch(A), &[pull(1, PrState::Merged, A), pull(2, PrState::Open, A)]);
        assert!(matches!(verdict, BranchVerdict::Keep(reason) if reason.contains("#2")));
    }

    #[test]
    fn unmerged_commits_keep_the_fork() {
        assert!(compared(&branch(A), 0).is_ok());
        assert!(compared(&branch(A), 3).unwrap_err().contains("3 commit(s)"));
    }
}
