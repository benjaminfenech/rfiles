//! Keyboard-layout helpers. GPUI turns Shift+digit/punctuation into the layout's shifted
//! character and drops Shift (Turkish Q: Shift+2 → `'`, US: Shift+2 → `@`), so a binding written
//! as `ctrl-shift-2` never matches. `shifted_char` tells the app which key to bind instead.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
/// Character the current keyboard layout produces for `vk` with Shift held, if printable.
pub fn shifted_char(_vk: u16) -> Option<String> {
    None
}
