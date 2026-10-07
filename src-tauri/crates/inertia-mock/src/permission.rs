//! A permission gate that answers from a policy instead of a person.
//!
//! Records every request, so a test can assert not just that a tool ran but
//! that it *asked for the right thing* - the `target` a rule would be written
//! against. A tool that asks permission for the wrong string passes every
//! behavioural test while being unconfigurable in practice, and this is the
//! only place that shows up.

use async_trait::async_trait;
use std::path::PathBuf;

use inertia_core::permission::{evaluate, evaluate_shaped, Action, Rule};
use inertia_core::tool::{Decision, PermissionGate, PermissionRequest};
use parking_lot::Mutex;

/// How the fake answers.
#[derive(Debug, Clone)]
pub enum Policy {
    /// Everything is allowed. The usual choice for tests about something else.
    AllowAll,
    /// Nothing is allowed, for testing the refusal path.
    DenyAll,
    /// Answers from a real ruleset, exercising the actual engine.
    Rules(Vec<Rule>),
    /// A queue of decisions, one per request, standing in for a person
    /// clicking. Runs out into `Deny` - an unanswered prompt is not consent.
    Scripted(Vec<Decision>),
}

#[derive(Debug)]
pub struct MockGate {
    policy: Mutex<Policy>,
    asked: Mutex<Vec<PermissionRequest>>,
    workspace: Option<PathBuf>,
}

impl MockGate {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy: Mutex::new(policy),
            asked: Mutex::new(Vec::new()),
            workspace: None,
        }
    }

    /// Stands in front of a workspace, so the tools that keep out of its
    /// secrets and rules have something to keep out of.
    pub fn guarding(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    pub fn allow_all() -> Self {
        Self::new(Policy::AllowAll)
    }

    pub fn deny_all() -> Self {
        Self::new(Policy::DenyAll)
    }

    /// Every request made, in order.
    pub fn asked(&self) -> Vec<PermissionRequest> {
        self.asked.lock().clone()
    }

    /// The targets that were asked about - usually what a test wants to
    /// assert on.
    pub fn targets(&self) -> Vec<String> {
        self.asked.lock().iter().map(|r| r.target.clone()).collect()
    }
}

#[async_trait]
impl PermissionGate for MockGate {
    async fn ask(&self, request: &PermissionRequest) -> inertia_core::Result<Decision> {
        self.asked.lock().push(request.clone());

        let mut policy = self.policy.lock();
        Ok(match &mut *policy {
            Policy::AllowAll => Decision::Allow,
            Policy::DenyAll => Decision::Deny,
            Policy::Rules(rules) => match evaluate_shaped(
                rules,
                &request.key,
                &request.target,
                &request.shape,
            )
            .action
            {
                Action::Allow => Decision::Allow,
                // Nobody is there to ask, and an unanswered question is not a
                // yes.
                Action::Ask | Action::Deny => Decision::Deny,
            },
            Policy::Scripted(queue) => {
                if queue.is_empty() {
                    Decision::Deny
                } else {
                    queue.remove(0)
                }
            }
        })
    }

    async fn verdict(&self, key: &str, target: &str) -> Action {
        match &*self.policy.lock() {
            Policy::AllowAll => Action::Allow,
            Policy::DenyAll => Action::Deny,
            Policy::Rules(rules) => evaluate(rules, key, target).action,
            Policy::Scripted(_) => Action::Ask,
        }
    }

    fn workspace(&self) -> Option<PathBuf> {
        self.workspace.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PermissionRequest {
        PermissionRequest::new("shell", "git status")
    }

    #[tokio::test]
    async fn it_records_what_was_asked() {
        let gate = MockGate::allow_all();
        gate.ask(&request()).await.unwrap();
        assert_eq!(gate.targets(), vec!["git status"]);
    }

    #[tokio::test]
    async fn allow_all_allows_and_deny_all_denies() {
        assert!(MockGate::allow_all()
            .ask(&request())
            .await
            .unwrap()
            .is_allowed());
        assert!(!MockGate::deny_all()
            .ask(&request())
            .await
            .unwrap()
            .is_allowed());
    }

    #[tokio::test]
    async fn a_scripted_gate_answers_in_order_then_refuses() {
        let gate = MockGate::new(Policy::Scripted(vec![Decision::Allow, Decision::Deny]));
        assert!(gate.ask(&request()).await.unwrap().is_allowed());
        assert!(!gate.ask(&request()).await.unwrap().is_allowed());
        // Past the end of the script: still a refusal, never a default yes.
        assert!(!gate.ask(&request()).await.unwrap().is_allowed());
    }

    // An `ask` verdict with nobody to ask is a refusal, not an allow.
    #[tokio::test]
    async fn an_unanswerable_ask_denies() {
        let gate = MockGate::new(Policy::Rules(vec![]));
        assert_eq!(gate.verdict("shell", "git status").await, Action::Ask);
        assert!(!gate.ask(&request()).await.unwrap().is_allowed());
    }

    #[tokio::test]
    async fn rules_are_actually_consulted() {
        let gate = MockGate::new(Policy::Rules(vec![
            Rule::for_any("shell", Action::Allow),
            Rule::new("shell", Action::Deny, "rm -rf *"),
        ]));
        assert!(gate.ask(&request()).await.unwrap().is_allowed());
        assert!(!gate
            .ask(&PermissionRequest::new("shell", "rm -rf /"))
            .await
            .unwrap()
            .is_allowed());
    }
}
