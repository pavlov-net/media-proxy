//! Typed state machine for a single stream.
//!
//! Invalid transitions are type errors rather than runtime checks.

use crate::error::StreamError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Resolving,
    Starting,
    Running,
    Reconfiguring,
    Stopping,
    Failed,
    Done,
}

impl StreamState {
    pub fn name(self) -> &'static str {
        match self {
            Self::Resolving => "resolving",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Reconfiguring => "reconfiguring",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
            Self::Done => "done",
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use StreamState as S;
        matches!(
            (self, next),
            (S::Resolving, S::Starting | S::Failed | S::Stopping)
                | (S::Starting, S::Running | S::Failed | S::Stopping)
                | (S::Running, S::Reconfiguring | S::Stopping | S::Failed)
                | (S::Reconfiguring, S::Running | S::Failed | S::Stopping)
                | (S::Stopping, S::Done | S::Failed)
                | (S::Failed, S::Done)
        )
    }

    pub fn transition(self, next: Self) -> Result<Self, StreamError> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(StreamError::InvalidTransition {
                from: self.name(),
                to: next.name(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_happy_path() {
        let s = StreamState::Resolving;
        let s = s.transition(StreamState::Starting).unwrap();
        let s = s.transition(StreamState::Running).unwrap();
        let s = s.transition(StreamState::Stopping).unwrap();
        let _ = s.transition(StreamState::Done).unwrap();
    }

    #[test]
    fn invalid_transition_errors() {
        assert!(StreamState::Resolving.transition(StreamState::Running).is_err());
        assert!(StreamState::Done.transition(StreamState::Running).is_err());
    }
}
