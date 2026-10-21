//! Single writer policy for a session goal; Loro remains its durable owner.
use crate::doc_host::ChatDocHandle;
use rcode_agent::goal::{GoalChange, GoalStore};
use rcode_proto::goal::{GoalStatus, SessionGoal};
use std::sync::{Arc, Mutex};

pub(crate) struct DocGoalStore {
    doc: Arc<ChatDocHandle>,
    mutation: Mutex<()>,
    activity: Arc<tokio::sync::Notify>,
    _activity_subscription: loro::Subscription,
}

impl DocGoalStore {
    pub fn new(doc: Arc<ChatDocHandle>) -> Self {
        let activity = Arc::new(tokio::sync::Notify::new());
        let notify = activity.clone();
        let subscription = doc.doc().doc().subscribe_root(Arc::new(move |event| {
            if event.events.iter().any(|change| matches!(change.path.first(), Some((_, loro::Index::Key(key))) if key.as_str() == "queue" || key.as_str() == "meta")) {
                notify.notify_one();
            }
        }));
        Self { doc, mutation: Mutex::new(()), activity, _activity_subscription: subscription }
    }

    pub fn user_action(
        &self,
        action: &str,
        objective: Option<&str>,
    ) -> Result<Option<SessionGoal>, String> {
        self.user_action_if_current(action, objective, None)
    }

    pub fn user_action_if_current(
        &self,
        action: &str,
        objective: Option<&str>,
        expected_id: Option<&str>,
    ) -> Result<Option<SessionGoal>, String> {
        self.user_action_if_revision(action, objective, expected_id, None)
    }

    pub fn user_action_if_revision(
        &self,
        action: &str,
        objective: Option<&str>,
        expected_id: Option<&str>,
        revision: Option<u64>,
    ) -> Result<Option<SessionGoal>, String> {
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let old = self.doc.doc().goal();
        if expected_id.is_some_and(|id| old.as_ref().is_none_or(|goal| goal.id != id)) {
            return Err("The goal changed; reload its current state before retrying".into());
        }
        if revision
            .is_some_and(|revision| old.as_ref().is_none_or(|goal| goal.revision != revision))
        {
            return Err("The goal changed; reload its current state before retrying".into());
        }
        let now = crate::now_ms();
        let next = match action {
            "read" => return Ok(old),
            "clear" => None,
            "create" | "replace" => {
                if action == "create"
                    && old
                        .as_ref()
                        .is_some_and(|goal| goal.status != GoalStatus::Complete)
                {
                    return Err(
                        "A goal already exists. Use /goal replace <objective> to replace it."
                            .into(),
                    );
                }
                Some(SessionGoal::new(
                    crate::new_id(),
                    objective.unwrap_or_default().into(),
                    now,
                )?)
            }
            "pause" | "resume" => {
                let mut goal = old.ok_or("There is no session goal")?;
                if action == "resume" {
                    goal.freeze_clock(now);
                    // 新执行代次使用新 ID，旧运行的取消回调或校验结果不能回写。
                    goal.id = crate::new_id();
                    goal.status = GoalStatus::Active;
                    goal.active_since = Some(now);
                    goal.reason = None;
                } else {
                    goal.freeze_clock(now);
                    goal.status = GoalStatus::Paused;
                    goal.reason = Some("Paused by the user".into());
                }
                goal.updated_at = now;
                Some(goal)
            }
            "edit" => {
                // Keep the execution generation and active clock. The runner picks up
                // the new objective revision at its next model-call boundary.
                let mut goal = old.ok_or("There is no session goal")?;
                let validated =
                    SessionGoal::new(String::new(), objective.unwrap_or_default().into(), now)?;
                goal.revision = goal
                    .revision
                    .checked_add(1)
                    .ok_or("Goal revision overflow")?;
                goal.objective = validated.objective;
                goal.reason = None;
                goal.updated_at = now;
                Some(goal)
            }
            _ => return Err("Unknown goal action".into()),
        };
        self.doc
            .doc()
            .update_goal(next.as_ref())
            .map_err(|error| error.to_string())?;
        Ok(next)
    }

    pub fn pause_interrupted(&self, expected_id: &str) -> Result<(), String> {
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(mut goal) = self
            .doc
            .doc()
            .goal()
            .filter(|goal| goal.id == expected_id && goal.status == GoalStatus::Active)
        {
            // Offline time after a crash is not runtime. Keep the last durable checkpoint.
            goal.freeze_clock(goal.updated_at);
            goal.status = GoalStatus::Paused;
            goal.reason = Some("Run interrupted by engine restart".into());
            goal.updated_at = crate::now_ms();
            self.doc
                .doc()
                .update_goal(Some(&goal))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

impl GoalStore for DocGoalStore {
    fn activity(&self) -> Option<Arc<tokio::sync::Notify>> { Some(self.activity.clone()) }
    fn read(&self) -> Result<Option<SessionGoal>, String> {
        Ok(self.doc.doc().goal())
    }
    fn should_yield(&self) -> Result<bool, String> {
        self.doc
            .doc()
            .read_queue()
            .map(|queue| !queue.is_empty())
            .map_err(|error| error.to_string())
    }

    fn change_if_current(
        &self,
        expected_id: &str,
        revision: Option<u64>,
        change: GoalChange,
    ) -> Result<Option<SessionGoal>, String> {
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(mut goal) = self.doc.doc().goal() else {
            return Ok(None);
        };
        // A delayed verifier or child result cannot finish or charge a new goal.
        if goal.id != expected_id || revision.is_some_and(|revision| revision != goal.revision) {
            return Ok(None);
        }
        match change {
            GoalChange::Usage { tokens, elapsed_ms } => {
                goal.tokens_used = goal.tokens_used.saturating_add(tokens);
                goal.time_used_ms = goal.time_used_ms.saturating_add(elapsed_ms);
            }
            GoalChange::Advance { reason } if goal.status == GoalStatus::Active => {
                goal.iteration = goal.iteration.saturating_add(1);
                goal.reason = Some(reason);
            }
            GoalChange::Complete { reason } if goal.status == GoalStatus::Active => {
                goal.freeze_clock(crate::now_ms());
                goal.status = GoalStatus::Complete;
                goal.reason = Some(reason);
            }
            GoalChange::Pause { reason } if goal.status == GoalStatus::Active => {
                goal.freeze_clock(crate::now_ms());
                goal.status = GoalStatus::Paused;
                goal.reason = Some(reason);
            }
            _ => return Ok(Some(goal)),
        }
        goal.updated_at = crate::now_ms();
        self.doc
            .doc()
            .update_goal(Some(&goal))
            .map_err(|error| error.to_string())?;
        Ok(Some(goal))
    }
}

pub(crate) fn parse_command(prompt: &str) -> Option<Result<(&str, String), String>> {
    let text = rcode_proto::invocation::harness_prompt(prompt, rcode_proto::HarnessId::Native);
    // Return owned objective data; the action comes from a fixed vocabulary.
    let command = text.strip_prefix("/goal")?;
    if !command.is_empty() && !command.starts_with(char::is_whitespace) {
        return None;
    }
    let command = command.trim();
    let (first, tail) = command
        .split_once(char::is_whitespace)
        .unwrap_or((command, ""));
    let (action, rest) = match first {
        "" => ("read", ""),
        "pause" => ("pause", tail.trim()),
        "resume" => ("resume", tail.trim()),
        "clear" => ("clear", tail.trim()),
        "replace" => ("replace", tail.trim()),
        _ => ("create", command),
    };
    if matches!(action, "pause" | "clear" | "read" | "resume") && !rest.is_empty() {
        return Some(Err("Unexpected arguments for goal action".into()));
    }
    Some(Ok((action, rest.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> (tempfile::TempDir, crate::EngineCore, DocGoalStore) {
        let root = tempfile::tempdir().unwrap();
        let core = crate::EngineCore::assemble(
            root.path(),
            Arc::new(crate::HarnessRegistry::new()),
            rcode_proto::HarnessId::Native,
            None,
        )
        .unwrap();
        core.workspace
            .create_chat("goal-test", None, Some(&core.device_id), None, None)
            .unwrap();
        let doc = core.doc_host.open("goal-test").unwrap();
        let store = DocGoalStore::new(doc);
        (root, core, store)
    }
    #[tokio::test]
    async fn goal_lifecycle_roundtrips_and_stale_callbacks_cannot_touch_a_resumed_or_replaced_goal()
    {
        let (_root, core, store) = store();
        let first = store
            .user_action("create", Some("Implement the feature"))
            .unwrap()
            .unwrap();
        assert!(
            store
                .user_action("create", Some("A different goal"))
                .is_err()
        );
        store
            .change(
                &first.id,
                GoalChange::Usage {
                    tokens: 123,
                    elapsed_ms: 10,
                },
            )
            .unwrap();
        store.user_action("pause", None).unwrap();
        store
            .change(
                &first.id,
                GoalChange::Complete {
                    reason: "late verifier".into(),
                },
            )
            .unwrap();
        assert_eq!(store.read().unwrap().unwrap().status, GoalStatus::Paused);
        let resumed = store.user_action("resume", None).unwrap().unwrap();
        assert_ne!(resumed.id, first.id);
        assert_eq!(resumed.tokens_used, 123);
        assert!(
            store
                .change(
                    &first.id,
                    GoalChange::Pause {
                        reason: "old cancellation".into()
                    }
                )
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .user_action_if_current("clear", None, Some(&first.id))
                .is_err()
        );
        let replacement = store
            .user_action("replace", Some("New objective"))
            .unwrap()
            .unwrap();
        assert!(
            store
                .change(
                    &resumed.id,
                    GoalChange::Usage {
                        tokens: 999,
                        elapsed_ms: 1
                    }
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(store.read().unwrap().unwrap().tokens_used, 0);
        let exported = core
            .doc_host
            .open("goal-test")
            .unwrap()
            .doc()
            .export_snapshot()
            .unwrap();
        let imported = rcode_doc::SessionDoc::init("replica").unwrap();
        imported.doc().import(&exported).unwrap();
        assert_eq!(imported.goal(), Some(replacement));
        store.user_action("clear", None).unwrap();
        assert!(store.read().unwrap().is_none());
    }
    #[tokio::test]
    async fn restart_recovery_pauses_an_active_goal_instead_of_dispatching_it() {
        let (root, core, store) = store();
        store
            .user_action("create", Some("Finish the interrupted work"))
            .unwrap();
        let journal =
            crate::RunJournal::open(root.path().join("orgs/dev-org/dev-user/journals")).unwrap();
        journal
            .append(
                "goal-test",
                &rcode_proto::AgentEvent::TextDelta {
                    text: "partial".into(),
                },
            )
            .unwrap();
        assert_eq!(core.sessions.recover_stale().unwrap(), 1);
        let goal = store.read().unwrap().unwrap();
        assert_eq!(goal.status, GoalStatus::Paused);
        assert!(goal.reason.unwrap().contains("restart"));
        assert!(!core.sessions.turn_in_flight("goal-test"));
        assert!(journal.stale_sessions().unwrap().is_empty());
    }

    #[test]
    fn only_explicit_goal_commands_change_goal_state() {
        assert!(parse_command("Please create a goal").is_none());
        assert!(parse_command("    /goal literal code").is_none());
        assert!(parse_command("/goals").is_none());
        assert_eq!(
            parse_command("/goal Build it").unwrap().unwrap(),
            ("create", "Build it".into())
        );
        assert_eq!(
            parse_command("/goal resume").unwrap().unwrap(),
            ("resume", String::new())
        );
        let chip = rcode_proto::invocation::Invocation::Command {
            name: "goal".into(),
        }
        .link();
        assert_eq!(
            parse_command(&format!("{chip} Build it")).unwrap().unwrap(),
            ("create", "Build it".into())
        );
    }

    #[tokio::test]
    async fn editing_keeps_running_preserves_usage_and_rejects_old_verifiers_and_drafts() {
        let (_root, core, store) = store();
        let first = store
            .user_action("create", Some("Original objective"))
            .unwrap()
            .unwrap();
        store
            .change(
                &first.id,
                GoalChange::Usage {
                    tokens: 123,
                    elapsed_ms: 0,
                },
            )
            .unwrap();
        assert!(
            core.sessions
                .update_goal_if_current(
                    "goal-test",
                    "edit",
                    None,
                    Some("New objective"),
                    Some(first.revision)
                )
                .await
                .is_err()
        );
        let edited = core
            .sessions
            .update_goal_if_current(
                "goal-test",
                "edit",
                Some(&first.id),
                Some("  New objective  "),
                Some(first.revision),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(edited.objective, "New objective");
        assert_eq!(edited.status, GoalStatus::Active);
        assert_eq!(edited.tokens_used, 123);
        assert_eq!(edited.active_since, first.active_since);
        assert!(edited.elapsed_ms(crate::now_ms()) >= first.time_used_ms);
        assert_eq!(first.id, edited.id);
        assert_eq!(edited.revision, first.revision + 1);
        assert!(
            store
                .change_if_current(
                    &first.id,
                    Some(first.revision),
                    GoalChange::Complete {
                        reason: "old verifier".into()
                    }
                )
                .unwrap()
                .is_none()
        );
        assert!(
            core.sessions
                .update_goal_if_current(
                    "goal-test",
                    "edit",
                    Some(&first.id),
                    Some("Stale draft"),
                    Some(first.revision)
                )
                .await
                .is_err()
        );
        assert_eq!(store.read().unwrap().unwrap(), edited);
        assert!(!core.sessions.turn_in_flight("goal-test"));
        for invalid in ["".to_owned(), " ".to_owned(), "a".repeat(4001)] {
            assert!(
                store
                    .user_action_if_revision(
                        "edit",
                        Some(&invalid),
                        Some(&edited.id),
                        Some(edited.revision)
                    )
                    .is_err()
            );
            assert_eq!(store.read().unwrap().unwrap(), edited);
        }
    }

    #[tokio::test]
    async fn restart_freezes_at_last_durable_activity_and_excludes_offline_time() {
        let (_root, core, store) = store();
        let mut goal = SessionGoal::new("g".into(), "Work".into(), 1000).unwrap();
        goal.updated_at = 6000;
        goal.time_used_ms = 2000;
        core.doc_host
            .open("goal-test")
            .unwrap()
            .doc()
            .update_goal(Some(&goal))
            .unwrap();
        store.pause_interrupted(&goal.id).unwrap();
        let paused = store.read().unwrap().unwrap();
        assert_eq!(paused.time_used_ms, 7000);
        assert_eq!(paused.elapsed_ms(crate::now_ms()), 7000);
        assert_eq!(paused.status, GoalStatus::Paused);
        let resumed = store.user_action("resume", None).unwrap().unwrap();
        assert_eq!(resumed.time_used_ms, 7000);
        assert!(resumed.active_since.is_some());
    }
}
