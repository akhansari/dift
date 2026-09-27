use super::PAGE;
use crate::git::{CommitInfo, GitError, HistoryPage, Repository};
use std::{io, sync::mpsc, thread};

pub(super) struct Request {
    pub(super) generation: u64,
    pub(super) refresh: bool,
    pub(super) skip: usize,
    pub(super) query: String,
}
pub(super) struct ResultPage {
    pub(super) generation: u64,
    pub(super) append: bool,
    pub(super) result: Result<(Option<String>, String, HistoryPage), GitError>,
}
pub(super) struct Worker {
    pub(super) sender: mpsc::Sender<Request>,
    pub(super) updates: mpsc::Receiver<ResultPage>,
}

impl Worker {
    pub(super) fn spawn(repo: Repository) -> io::Result<Self> {
        let (sender, requests) = mpsc::channel();
        let (updates, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("git-history".into())
            .spawn(move || {
                let mut pinned: Option<(Option<String>, String)> = None;
                let mut search_cache: Option<Vec<CommitInfo>> = None;
                for incoming in &requests {
                    // The UI rejects stale generations; skip their expensive Git work too.
                    let Request {
                        generation,
                        refresh,
                        skip,
                        query,
                    } = newest_request(incoming, requests.try_iter());
                    if refresh {
                        pinned = None;
                        search_cache = None;
                    }
                    let result = (|| {
                        if pinned.is_none() {
                            pinned = Some(repo.history_head()?);
                        }
                        let (tip, branch) = pinned
                            .as_ref()
                            .ok_or_else(|| GitError::InvalidOutput("Missing history tip".into()))?;
                        let page = tip.as_deref().map_or_else(
                            || {
                                Ok(HistoryPage {
                                    commits: Vec::new(),
                                    more: false,
                                })
                            },
                            |tip| {
                                if query.is_empty() {
                                    repo.history_page(tip, skip, PAGE, "")
                                } else {
                                    let query = query.to_lowercase();
                                    let commits = match &search_cache {
                                        Some(commits) => commits,
                                        None => search_cache.insert(repo.history_all(tip)?),
                                    };
                                    Ok(HistoryPage {
                                        commits: commits
                                            .iter()
                                            .filter(|commit| commit.matches_lowercase(&query))
                                            .cloned()
                                            .collect(),
                                        more: false,
                                    })
                                }
                            },
                        )?;
                        Ok((tip.clone(), branch.clone(), page))
                    })();
                    if updates
                        .send(ResultPage {
                            generation,
                            append: skip > 0,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self {
            sender,
            updates: receiver,
        })
    }
}

fn newest_request(first: Request, pending: impl IntoIterator<Item = Request>) -> Request {
    pending.into_iter().fold(first, |previous, mut next| {
        next.refresh |= previous.refresh;
        next
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppResult, ui::fixtures::Fixture};
    use std::time::Duration;

    #[test]
    fn newer_search_replaces_pending_work_without_losing_a_refresh() {
        // Given: a refresh, followed by two edits to the search query.
        let refresh = Request {
            generation: 1,
            refresh: true,
            skip: 0,
            query: String::new(),
        };
        let old = Request {
            generation: 2,
            refresh: false,
            skip: 0,
            query: "ol".into(),
        };
        let latest = Request {
            generation: 3,
            refresh: false,
            skip: 0,
            query: "older".into(),
        };
        // When: coalescing queued worker requests.
        let request = newest_request(refresh, [old, latest]);
        // Then: only the newest query is evaluated against a refreshed tip.
        assert_eq!(request.generation, 3);
        assert_eq!(request.query, "older");
        assert!(request.refresh);
    }

    #[test]
    fn searches_reuse_the_pinned_history_until_refresh() -> AppResult<()> {
        // Given: a worker pinned to the initial commit.
        let fixture = Fixture::new("")?;
        let worker = Worker::spawn(fixture.repo()?)?;
        worker.sender.send(Request {
            generation: 1,
            refresh: false,
            skip: 0,
            query: String::new(),
        })?;
        let initial = worker.updates.recv_timeout(Duration::from_secs(4))?;
        let (tip, _, _) = initial.result?;
        fixture.write("tracked.txt", "new commit\n")?;
        fixture.git(&["add", "tracked.txt"])?;
        fixture.git(&["commit", "-m", "new commit"])?;
        // When: searching the pinned history and then explicitly refreshing it.
        worker.sender.send(Request {
            generation: 2,
            refresh: false,
            skip: 0,
            query: "initial".into(),
        })?;
        let pinned = worker.updates.recv_timeout(Duration::from_secs(4))?;
        worker.sender.send(Request {
            generation: 3,
            refresh: true,
            skip: 0,
            query: "new commit".into(),
        })?;
        let refreshed = worker.updates.recv_timeout(Duration::from_secs(4))?;
        // Then: search stays on the old tip until refresh, then sees the new commit.
        let (pinned_tip, _, pinned_page) = pinned.result?;
        let (new_tip, _, new_page) = refreshed.result?;
        assert_eq!(pinned_tip, tip);
        assert_eq!(pinned_page.commits[0].subject(), "initial");
        assert_ne!(new_tip, tip);
        assert_eq!(new_page.commits[0].subject(), "new commit");
        Ok(())
    }
}
