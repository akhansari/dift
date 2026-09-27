use super::PAGE;
use crate::git::{GitError, HistoryPage, Repository};
use std::{io, sync::mpsc, thread};

pub(super) enum Request {
    Load {
        generation: u64,
        refresh: bool,
        skip: usize,
        query: String,
    },
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
                for request in requests {
                    let Request::Load {
                        generation,
                        refresh,
                        skip,
                        query,
                    } = request;
                    if refresh || pinned.is_none() {
                        pinned = None;
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
                            |tip| repo.history_page(tip, skip, PAGE, &query),
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
