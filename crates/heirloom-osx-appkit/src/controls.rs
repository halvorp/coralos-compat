//! Controls (rung 4). Until they exist a view added to an open window has nothing to publish.
use heirloom_objc::runtime::Id;

/// Publishes `v`, a view added to the open window `w`, as a host control when it is one.
pub(crate) fn publish(_w: Id, _v: Id) {}

/// Events a window or the app loop does not handle itself: controls take Intents here (rung 4); everything else is counted and dropped.
pub(crate) fn dispatch(_app: Id, _event: heirloom_ui_abi::GuestUiEvent) {}

/// Registers the control classes (rung 4).
pub(crate) fn install() {}
