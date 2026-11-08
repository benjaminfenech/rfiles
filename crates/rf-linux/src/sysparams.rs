//! Read-only Windows system parameters the UI follows.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
/// Windows' "Animation effects" (Settings → Accessibility → Visual effects). True when it can't
/// be read (the Windows default).
pub fn client_area_animation() -> bool {
    false
}
