/*
 * Copyright (c) Radical HQ Limited
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use crate::commands::list::search_query::SearchQuerySearchNodes::PullRequest;
use crate::error::Error;
use crate::error::Result;
use graphql_client::{GraphQLQuery, Response};
use tracing::debug;
use tracing::trace;

use self::search_query::PullRequestReviewState;
use self::search_query::SearchQuerySearchNodesOnPullRequestCommentsNodes;
use self::search_query::SearchQuerySearchNodesOnPullRequestReviewThreadsNodesCommentsNodes;

#[allow(clippy::upper_case_acronyms)]
type URI = String;
type DateTime = String;

type TopLevelComment = SearchQuerySearchNodesOnPullRequestCommentsNodes;
type ThreadComment =
    SearchQuerySearchNodesOnPullRequestReviewThreadsNodesCommentsNodes;

#[allow(non_camel_case_types)]
#[derive(GraphQLQuery, Debug)]
#[graphql(
    schema_path = "src/gql/schema.docs.graphql",
    query_path = "src/gql/open_reviews.graphql",
    response_derives = "Debug"
)]
pub struct SearchQuery;

pub async fn list(config: &crate::config::Config) -> Result<()> {
    let query = format!( "repo:{}/{} is:open is:pr author:@me archived:false", config.owner, config.repo);
    trace!(query, "searching for prs");
    let variables = search_query::Variables {
        query
    };
    let request_body = SearchQuery::build_query(variables);
    let response_body: Response<search_query::ResponseData> =
        octocrab::instance()
            .post(config.graphql_path(), Some(&request_body))
            .await?;

    print_pr_info(response_body).map_err(|e| {
        Error::new(format!("unexpected error printing pr info: {}", e))
    })
}

fn print_pr_info(
    response_body: Response<search_query::ResponseData>,
) -> Result<()> {
    let term = console::Term::stdout();
    let Some(data) = response_body.data else {
        trace!("no data in response");
        return Ok(());
    };
    let Some(search_nodes) = data.search.nodes else {
        trace!("no search nodes in response");
        return Ok(());
    };
    for pr in search_nodes.into_iter().flatten() {
        let pr = match pr {
            PullRequest(pr) => pr,
            _ => {
                debug!(?pr, "ignoring node in search results");
                continue;
            },
        };

        let mut state = PrState(PullRequestReviewState::PENDING);
        if let Some(reviews) = pr.reviews {
            if let Some(review_nodes) = reviews.nodes {
                for review in review_nodes.into_iter().flatten() {
                    let new_state = PrState(review.state);
                    if new_state < state {
                        state = new_state;
                    }
                }
            };
        };

        let mut most_recent_author_comment: Option<TopLevelComment> = None;
        for comment in pr.comments.nodes.into_iter().flatten().flatten() {
            if comment.viewer_did_author {
                if let Some(ref most_recent) = most_recent_author_comment {
                    if comment.updated_at > most_recent.updated_at {
                        most_recent_author_comment = Some(comment);
                    }
                } else {
                    most_recent_author_comment = Some(comment);
                }
            }
        }

        let mut most_recent_author_thread: Option<ThreadComment> = None;
        let mut most_recent_reviewer_comment: Option<ThreadComment> = None;
        for thread in pr.review_threads.nodes.into_iter().flatten().flatten() {
            for comment in thread.comments.nodes.into_iter().flatten().flatten()
            {
                if comment.is_minimized {
                    continue;
                }
                if comment.viewer_did_author {
                    if let Some(ref most_recent) = most_recent_author_thread {
                        if comment.updated_at > most_recent.updated_at {
                            most_recent_author_thread = Some(comment);
                        }
                    } else {
                        most_recent_author_thread = Some(comment);
                    }
                } else if let Some(ref most_recent) =
                    most_recent_reviewer_comment
                {
                    if comment.updated_at > most_recent.updated_at {
                        most_recent_reviewer_comment = Some(comment);
                    }
                } else {
                    most_recent_reviewer_comment = Some(comment);
                }
            }
        }
        let newest_author_response_time =
            match (&most_recent_author_comment, &most_recent_author_thread) {
                (Some(comment), Some(thread)) => {
                    if comment.updated_at > thread.updated_at {
                        Some(&comment.updated_at)
                    } else {
                        Some(&thread.updated_at)
                    }
                }
                (Some(comment), None) => Some(&comment.updated_at),
                (None, Some(thread)) => Some(&thread.updated_at),
                _ => None,
            };

        let mut has_new_comments = false;
        match (&newest_author_response_time, &most_recent_reviewer_comment) {
            (Some(author_comment_time), Some(reviewer_comment)) => {
                has_new_comments =
                    *author_comment_time < &reviewer_comment.updated_at;
            }
            (None, Some(_)) => has_new_comments = true,
            _ => {}
        }

        term.write_line(&format!(
            "{}\t{}  {} {}",
            state.styled(),
            if has_new_comments {
                "📬"
            } else if newest_author_response_time.is_some() {
                "💬️"
            } else if newest_author_response_time.is_none()  && most_recent_reviewer_comment.is_none() {
                "💤"
            } else {
                "✅"
            },
            console::style(&pr.title).bold(),
            console::style(&pr.url).dim(),
        ))?;
    }
    Ok(())
}

#[derive(Debug)]
struct PrState(PullRequestReviewState);

impl PrState {
    fn styled(&self) -> console::StyledObject<&PrState> {
        let styled = console::style(self);
        match self.0 {
            PullRequestReviewState::APPROVED => styled.green(),
            PullRequestReviewState::CHANGES_REQUESTED => styled.red(),
            PullRequestReviewState::COMMENTED => styled.yellow(),
            PullRequestReviewState::DISMISSED => styled.dim(),
            PullRequestReviewState::PENDING => styled.bold(),
            PullRequestReviewState::Other(_) => styled,
        }
    }

    fn to_number(&self) -> i32 {
        match self.0 {
            PullRequestReviewState::APPROVED => 0,
            PullRequestReviewState::CHANGES_REQUESTED => 1,
            PullRequestReviewState::COMMENTED => 2,
            PullRequestReviewState::DISMISSED => 3,
            PullRequestReviewState::PENDING => 4,
            PullRequestReviewState::Other(_) => 5,
        }
    }
}

impl std::fmt::Display for PrState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.0 {
            PullRequestReviewState::APPROVED => write!(f, "Approved"),
            PullRequestReviewState::CHANGES_REQUESTED => {
                write!(f, "Changes Requested")
            }
            PullRequestReviewState::COMMENTED => write!(f, "Commented"),
            PullRequestReviewState::DISMISSED => write!(f, "Dismissed"),
            PullRequestReviewState::PENDING => write!(f, "Unreviewed"),
            PullRequestReviewState::Other(ref s) => write!(f, "{}", s),
        }
    }
}

impl PartialEq for PrState {
    fn eq(&self, other: &Self) -> bool {
        self.to_number() == other.to_number()
    }
}

impl Eq for PrState {}

impl PartialOrd for PrState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.to_number().cmp(&other.to_number()))
    }
}

impl Ord for PrState {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}
