//! Test-only observations at the real transaction boundaries. No injected route or alternate builder.

use super::RelationStage;
use std::cell::RefCell;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    SourceEncode,
    SourceDigest,
    EcoDiff,
    NativeReportBound,
    BuildEco,
    CandidateEncode,
    CandidateDigest,
    Prepare(RelationStage),
    VerifyReferences,
    Publish(RelationStage),
    CheckedReportBound,
    RetainChecked,
    VanishedCandidates { removed: usize, queries: usize, candidates: usize },
    Marked { nodes: Vec<super::islands::Key>, physical: bool, electrical: bool },
}

thread_local! {
    static TRACE: RefCell<Option<Vec<Op>>> = const { RefCell::new(None) };
}

/// Record an operation only on the fixture thread and inside an explicitly observed command.
pub(crate) fn hit(op: Op) {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.push(op);
        }
    });
}

/// Collect operation order as well as counts, with cleanup even if a fixture unwinds.
pub(crate) fn trace<T>(command: impl FnOnce() -> T) -> (T, Vec<Op>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TRACE.with_borrow_mut(|trace| {
                trace.take();
            });
        }
    }
    TRACE.with_borrow_mut(|trace| assert!(trace.replace(Vec::new()).is_none()));
    let _reset = Reset;
    let value = command();
    let trace = TRACE.with_borrow_mut(|trace| trace.take().unwrap());
    (value, trace)
}
