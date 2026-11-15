# PR #356 review follow-up

CodeRabbit identified an unnecessarily board-gated Rules ribbon action and
unreachable lower navigation rows in short wide Setup windows. The Rules action
now needs only an open project, while DRC, Measure and board-default editing
retain their physical-board requirement. Setup's wide rail scrolls independently
of its page content and reports its clipped viewport for accessibility reveal.

Both regressions failed against the prior behavior and pass with the fixes. The
navigation test visits Net classes at 1200 × 360 and 1200 × 420 without changing
saved bytes. GPU readbacks were inspected in dark and light themes; they verify
functional composition, not native compositor timing or Wayland endurance.

The review-summary Linked Issues check also requested explicit parity evidence.
`class_form_controls_create_edit_cancel_and_exact_units` compares the project
stored by the real mixed-unit GUI workflow with a canonical headless MCP update
on the same pre-edit snapshot, then verifies stale MCP rejection preserves it.
`two_classes_reach_pcb_update_split_and_native_reopen` compares GUI-authored
bindings/resolutions/source identities with `nets.inspect_classes` through both
headless dispatch and the real live-editor adapter, called in process rather
than through a network client. It also compares conflicting
`net_classes.resolve` results with the resolver used by Rules. The existing
class-consumer tests cover assigned routing defaults, exact class spacing and
DRC dimensions/provenance at this source revision.

The changes preserve one command/resolver path and introduce no asynchronous
resource or model lifecycle. Code review confirms navigation clipping/reveal is
local to the rail, page content retains its own scroll owner, and its selection
still returns to the host's existing retained-draft path. Existing app tests
cover cancellation, stale authority, separate windows and Save/Undo/reopen.
The negative and positive regression logs remain under
`setup/issue303/rabbit-review/`; verification records distinguish the reused
workspace checks from the affected checks rerun for this change.
