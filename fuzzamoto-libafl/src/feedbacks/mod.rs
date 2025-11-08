use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use std::{borrow::Cow, rc::Rc};
use std::{cell::RefCell, fmt::Debug};

use libafl::observers::StdOutObserver;
use libafl_bolts::tuples::{Handle, Handled, MatchName, MatchNameRef};
use libafl_bolts::{Error, Named};

use libafl::{
    HasMetadata,
    corpus::Testcase,
    executors::ExitKind,
    feedbacks::{Feedback, StateInitializer},
    state::HasCorpus,
};

use fuzzamoto::assertions::{AssertionScope, write_assertions};

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

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AssertionFeedback {
    assertions: HashMap<String, AssertionScope>,
    o_ref: Handle<StdOutObserver>,

    #[serde(skip)]
    last_update: Option<Instant>,
    #[serde(skip)]
    update_interval: Option<Duration>,
    #[serde(skip)]
    output_file: Option<PathBuf>,

    // Only consider always assertions
    only_always_assertions: bool,
}

impl AssertionFeedback {
    fn evaluate_assertion(&mut self, new: AssertionScope) -> bool {
        if self.only_always_assertions && matches!(new, AssertionScope::Sometimes(_, _)) {
            return false;
        }

        let previous = self.assertions.get(&new.message());

        let result = match (previous, &new) {
            (None, new) => new.evaluate() || !self.only_always_assertions,
            (Some(prev), new) => {
                (!prev.evaluate() && new.evaluate()) || (prev.distance() > new.distance())
            }
        };

        if result {
            log::debug!("{:?} -> {:?}", previous, new);
            self.assertions.insert(new.message(), new);
        }

        result
    }
}

impl<S> StateInitializer<S> for AssertionFeedback {}

impl<EM, I, OT, S> Feedback<EM, I, OT, S> for AssertionFeedback
where
    OT: MatchName,
{
    fn is_interesting(
        &mut self,
        _state: &mut S,
        _manager: &mut EM,
        _input: &I,
        observers: &OT,
        _exit_kind: &ExitKind,
    ) -> Result<bool, Error> {
        let observer = observers
            .get(&self.o_ref)
            .ok_or(Error::illegal_state("StdOutObserver is missing"))?;
        let buffer = observer
            .output
            .as_ref()
            .ok_or(Error::illegal_state("StdOutObserver has no stdout"))?;
        let stdout = String::from_utf8_lossy(buffer).into_owned();

        let mut interesting = false;
        for line in stdout.lines() {
            let trimmed = line.trim().trim_matches(|c| c == '\0');
            if let Ok(assertion) = serde_json::from_str::<AssertionScope>(trimmed) {
                interesting |= self.evaluate_assertion(assertion);
            }
        }

        let now = Instant::now();
        if !self.only_always_assertions
            && self.output_file.is_some()
            && now > self.last_update.unwrap() + self.update_interval.unwrap()
        {
            log::warn!("Writing assertions to file");
            self.last_update = Some(now);

            let mut output_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(self.output_file.as_ref().unwrap())
                .map_err(|e| {
                    log::warn!("Writing assertions to file: {:?}", e);
                    libafl::Error::unknown(format!("Failed to open output file: {}", e))
                })?;
            log::warn!("Writing assertions to file 2");
            write_assertions(&mut output_file, &self.assertions).map_err(|e| {
                log::error!("oh no {:?}", e);
                libafl::Error::unknown(format!("Failed to wirte to output file: {}", e))
            })?;
            log::warn!("Writing assertions to file 3");
        }

        Ok(interesting)
    }
}

impl Named for AssertionFeedback {
    #[inline]
    fn name(&self) -> &Cow<'static, str> {
        self.o_ref.name()
    }
}

impl AssertionFeedback {
    /// Creates a new [`AssertionFeedback`].
    #[must_use]
    pub fn new(observer: &StdOutObserver, output_file: PathBuf) -> Self {
        let interval = Duration::from_secs(30);
        Self {
            o_ref: observer.handle(),
            assertions: HashMap::new(),
            output_file: Some(output_file),

            last_update: Some(Instant::now() - interval * 2),
            update_interval: Some(interval),

            only_always_assertions: false,
        }
    }
    pub fn new_only_always(observer: &StdOutObserver) -> Self {
        Self {
            o_ref: observer.handle(),
            assertions: HashMap::new(),
            output_file: None,
            last_update: None,
            update_interval: None,
            only_always_assertions: true,
        }
    }
}
