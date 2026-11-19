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
    FinalPadWork { pads: usize, units: usize, candidates: usize, contacts: usize },
    Marked { nodes: Vec<super::islands::Key>, physical: bool, electrical: bool },
}

struct Trace {
    ops: Vec<Op>,
    final_pads: bool,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

/// Record an operation only on the fixture thread and inside an explicitly observed command.
pub(crate) fn hit(op: Op) {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace
            && (trace.final_pads || !matches!(op, Op::FinalPadWork { .. }))
        {
            trace.ops.push(op);
        }
    });
}

/// Collect operation order as well as counts, with cleanup even if a fixture unwinds.
pub(crate) fn trace<T>(command: impl FnOnce() -> T) -> (T, Vec<Op>) {
    trace_with(command, false)
}

/// Opt into the final-pad work probe without changing existing exact transaction traces.
pub(super) fn trace_final_pads<T>(command: impl FnOnce() -> T) -> (T, Vec<Op>) {
    trace_with(command, true)
}

/// Use one scoped observer with the selected pad-work detail; unwind clears thread-local state.
fn trace_with<T>(command: impl FnOnce() -> T, final_pads: bool) -> (T, Vec<Op>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TRACE.with_borrow_mut(|trace| {
                trace.take();
            });
        }
    }
    TRACE.with_borrow_mut(|trace| assert!(trace.replace(Trace { ops: Vec::new(), final_pads }).is_none()));
    let _reset = Reset;
    let value = command();
    let trace = TRACE.with_borrow_mut(|trace| trace.take().unwrap());
    (value, trace.ops)
}
