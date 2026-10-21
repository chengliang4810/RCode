//! Durable, session-scoped goal state. Execution policy belongs to the host.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Complete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_accumulates_active_segments_and_freezes_while_paused() {
        let mut goal = SessionGoal::new("g".into(), "Build".into(), 1000).unwrap();
        assert_eq!(goal.elapsed_ms(0), 0);
        assert_eq!(goal.elapsed_ms(4500), 3500);
        goal.freeze_clock(4500);
        goal.status = GoalStatus::Paused;
        assert_eq!(goal.elapsed_ms(100_000), 3500);
        goal.status = GoalStatus::Active;
        goal.active_since = Some(100_000);
        assert_eq!(goal.elapsed_ms(102_000), 5500);
        goal.freeze_clock(102_000);
        goal.status = GoalStatus::Complete;
        assert_eq!(goal.elapsed_ms(900_000), 5500);
        goal.freeze_clock(900_000);
        assert_eq!(goal.time_used_ms, 5500);
    }

    #[test]
    fn legacy_goal_without_active_clock_retains_recorded_time() {
        let goal = SessionGoal::new("g".into(), "Build".into(), 1000).unwrap();
        let mut value = serde_json::to_value(goal).unwrap();
        value.as_object_mut().unwrap().remove("activeSince");
        value["timeUsedMs"] = 42_000.into();
        let restored: SessionGoal = serde_json::from_value(value).unwrap();
        assert_eq!(restored.elapsed_ms(500_000), 42_000);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGoal {
    pub id: String,
    pub objective: String,
    #[serde(default)]
    pub revision: u64,
    pub status: GoalStatus,
    pub tokens_used: u64,
    pub time_used_ms: u64,
    /// Beginning of the active segment; frozen into time_used_ms when paused.
    #[serde(default)]
    pub active_since: Option<i64>,
    pub iteration: u64,
    pub created_at: i64,
    pub updated_at: i64,
    pub reason: Option<String>,
}

impl SessionGoal {
    pub fn elapsed_ms(&self, now: i64) -> u64 {
        self.time_used_ms
            .saturating_add(if self.status == GoalStatus::Active {
                self.active_since
                    .map(|start| now.saturating_sub(start).max(0) as u64)
                    .unwrap_or(0)
            } else {
                0
            })
    }

    pub fn freeze_clock(&mut self, now: i64) {
        self.time_used_ms = self.elapsed_ms(now);
        self.active_since = None;
    }

    pub fn new(id: String, objective: String, now: i64) -> Result<Self, String> {
        let objective = objective.trim().to_owned();
        if objective.is_empty() || objective.chars().count() > 4_000 {
            return Err("Goal must contain between 1 and 4000 characters".into());
        }
        Ok(Self {
            id,
            objective,
            revision: 0,
            status: GoalStatus::Active,
            tokens_used: 0,
            time_used_ms: 0,
            active_since: Some(now),
            iteration: 1,
            created_at: now,
            updated_at: now,
            reason: None,
        })
    }
}
