use std::{borrow::Cow, rc::Rc};
use std::{cell::RefCell, fmt::Debug};

use libafl_bolts::{Error, Named};

use libafl::{
    HasMetadata,
    corpus::Testcase,
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
    state::HasCorpus,
};

pub mod recv;
pub use recv::RecvFeedback;

use crate::input::IrInput;
use crate::stages::TimeoutsToVerify;

/// A Feedback that captures all timeouts and stores them in State for re-evaluation later.
/// Use in conjunction with `VerifyTimeoutsStage`
#[derive(Debug)]
pub struct CaptureTimeoutFeedback {
    enabled: Rc<RefCell<bool>>,
}

impl CaptureTimeoutFeedback {
    /// Create a new [`CaptureTimeoutFeedback`].
    pub fn new(enabled: Rc<RefCell<bool>>) -> Self {
        Self { enabled }
    }
}

impl Named for CaptureTimeoutFeedback {
    fn name(&self) -> &Cow<'static, str> {
        static NAME: Cow<'static, str> = Cow::Borrowed("CaptureTimeoutFeedback");
        &NAME
    }
}

impl<S> StateInitializer<S> for CaptureTimeoutFeedback {}

impl<EM, OT, S> Feedback<EM, IrInput, OT, S> for CaptureTimeoutFeedback
where
    S: HasCorpus<IrInput> + HasMetadata,
{
    #[inline]
    fn is_interesting(
        &mut self,
        state: &mut S,
        _manager: &mut EM,
        input: &IrInput,
        _observers: &OT,
        exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        if *self.enabled.borrow() && matches!(exit_kind, ExitKind::Timeout) {
            let timeouts = state.metadata_or_insert_with(TimeoutsToVerify::new);
            log::info!("Timeout detected, adding to verification queue!");
            timeouts.push(input.clone());
            return Ok(false);
        }
        Ok(matches!(exit_kind, ExitKind::Timeout))
    }

    fn append_metadata(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _observers: &OT,
        _testcase: &mut Testcase<IrInput>,
    ) -> Result<(), Error> {
        Ok(())
    }
}
