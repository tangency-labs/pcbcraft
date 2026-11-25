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
    NativeDatum,
    Marked { nodes: Vec<super::islands::Key>, physical: bool, electrical: bool },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Detail {
    None,
    FinalPads,
    NativeDatum,
}

struct Trace {
    ops: Vec<Op>,
    detail: Detail,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

/// Record an operation only on the fixture thread and inside an explicitly observed command.
pub(crate) fn hit(op: Op) {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            let collect = match op {
                Op::FinalPadWork { .. } => trace.detail == Detail::FinalPads,
                Op::NativeDatum => trace.detail == Detail::NativeDatum,
                _ => true,
            };
            if collect {
                trace.ops.push(op);
            }
        }
    });
}

/// Collect operation order as well as counts, with cleanup even if a fixture unwinds.
pub(crate) fn trace<T>(command: impl FnOnce() -> T) -> (T, Vec<Op>) {
    trace_with(command, Detail::None)
}

/// Opt into the final-pad work probe without changing existing exact transaction traces.
pub(super) fn trace_final_pads<T>(command: impl FnOnce() -> T) -> (T, Vec<Op>) {
    trace_with(command, Detail::FinalPads)
}

/// Opt into counting cache misses (including unavailable) at the native relation datum boundary.
pub(super) fn trace_native_datum<T>(command: impl FnOnce() -> T) -> (T, Vec<Op>) {
    trace_with(command, Detail::NativeDatum)
}

/// Use one scoped observer with the selected work detail; unwind clears thread-local state.
fn trace_with<T>(command: impl FnOnce() -> T, detail: Detail) -> (T, Vec<Op>) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TRACE.with_borrow_mut(|trace| {
                trace.take();
            });
        }
    }
    TRACE.with_borrow_mut(|trace| assert!(trace.replace(Trace { ops: Vec::new(), detail }).is_none()));
    let _reset = Reset;
    let value = command();
    let trace = TRACE.with_borrow_mut(|trace| trace.take().unwrap());
    (value, trace.ops)
}
