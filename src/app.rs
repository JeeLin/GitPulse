use crate::config::Config;
use crate::git::RepoInfo;
use crate::notifications::{NotificationDb, NotificationState};
use crate::ui::accounts::AccountState;
use crate::ui::branches::BranchState;
use crate::ui::ci::CIState;
use crate::ui::issues::IssueState;
use crate::ui::pulls::PullRequestState;
use crate::ui::remote::RemoteRepoState;
use crate::ui::search::SearchState;
use crate::ui::stars::StarsState;
use crate::ui::trending::TrendingState;
use crate::ui::worktree::WorktreeState;
use anyhow::Result;

pub struct App {
    #[expect(dead_code)]
    pub config: Config,
    pub repos: Vec<RepoInfo>,
    pub db: NotificationDb,
    pub notification_state: NotificationState,
    pub branch_state: BranchState,
    pub account_state: AccountState,
    pub remote_state: RemoteRepoState,
    pub issue_state: IssueState,
    pub pull_state: PullRequestState,
    pub ci_state: CIState,
    pub worktree_state: WorktreeState,
    pub search_state: SearchState,
    pub trending_state: TrendingState,
    pub stars_state: StarsState,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let db_dir = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("gitpulse");
        let db_path = db_dir.join("state.db");
        let _ = std::fs::create_dir_all(&db_dir);
        let db = NotificationDb::new(&db_path)?;
        let notification_state = NotificationState::new();
        let branch_state = BranchState::new();
        let account_state = AccountState::new();
        let remote_state = RemoteRepoState::new();
        let issue_state = IssueState::new();
        let pull_state = PullRequestState::new();
        let ci_state = CIState::new();
        Ok(Self {
            config,
            repos: Vec::new(),
            db,
            notification_state,
            branch_state,
            account_state,
            remote_state,
            issue_state,
            pull_state,
            ci_state,
            worktree_state: WorktreeState::new(),
            search_state: SearchState::new(),
            trending_state: TrendingState::new(),
            stars_state: StarsState::new(),
            should_quit: false,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        crate::ui::run(self).await
    }
}
