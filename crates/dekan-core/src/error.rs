use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid phase transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: crate::phase::GamePhase,
        to: crate::phase::GamePhase,
    },

    #[error("state channel closed")]
    ChannelClosed,
}
