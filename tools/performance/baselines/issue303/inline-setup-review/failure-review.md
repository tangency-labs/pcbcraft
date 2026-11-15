# Embedded Board Setup review

Board defaults and class content return local state/actions to their containing
host. The host still submits SET_RULES and the existing class/assignment commands;
physical setup uses its existing command. No engine resolution or saved format
was changed.

The page-switch tests cover invalid text retention, rule saves with pending layer
and other rule edits, physical Apply with pending rules, native codec reopen,
page-local revert, Cancel and exact Undo restoration. The existing class form
rename test also checks text focus after a dirty-state change. Keeping the revert
control in a stable widget slot fixed the focus loss found during this run.

Detached viewport callback tests exercise embedded and standalone rule placement,
focus changes, saving in the source host, owner closure and an auxiliary library
workspace in the main window. They verify saved state survives host closure and
remains undoable. These tests and GPU readbacks do not certify OS compositor or
Wayland endurance behavior.

Code inspection confirms that revision rebinding occurs only after a validated
command issued by this containing host. It copies only the revision while retaining
the same source view, target and epoch, then checks acceptance. Only child drafts
matching the previous parent source can advance. The class command handler owns
its refresh; a failed refresh cannot be blessed by the parent's class-save path
or a later save. An external revision remains stale and cannot publish. The
external-rename regression verifies the ordinary stale-input path; failed snapshot
refresh branches were inspected rather than fault-injected.

Pending profile jobs and active material drafts block rule publication. Navigation
retains them, while existing profile cancellation/authority tests continue to cover
their lifecycle. Physical Apply retains the dialog over remaining rule edits even
if context refresh fails. Closing the captured view expires the dialog. Project
and auxiliary hosts route it through the source view's owner and disable background
editing while it owns input. Standalone Rules continues to use the same shared
editor content and command path.

This follow-up adds no new performance qualification. Prior timing evidence keeps
its original boundaries, and no native endurance run or user desktop restart was
performed.
