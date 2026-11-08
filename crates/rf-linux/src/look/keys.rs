//! Tuş süzgeci: klavye kancasından gelen tuş olaylarını Hızlı Bakış eylemlerine çevirir.
//! Saf durum makinesi; Windows'a bağımlı değil.
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
pub const VK_RETURN: u32 = 0x0D;
pub const VK_ESCAPE: u32 = 0x1B;
pub const VK_SPACE: u32 = 0x20;
pub const VK_LEFT: u32 = 0x25;
pub const VK_UP: u32 = 0x26;
pub const VK_RIGHT: u32 = 0x27;
pub const VK_DOWN: u32 = 0x28;
pub const VK_F5: u32 = 0x74;
pub const VK_F11: u32 = 0x7A;
/// Boşluk bu kadar ya da daha uzun basılı tutulup bırakılırsa pencere kapanır.
pub const HOLD_CLOSE_MS: u64 = 750;
/// Başka bir tuşa basıldıktan sonra bu süre boyunca hiçbir tuş eylem üretmez (yazı yazarken açılmasın).
pub const GUARD_MS: u64 = 1000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookKey {
    Space,
    Enter,
    Escape,
    Up,
    Down,
    Left,
    Right,
    F5,
    F11,
}
impl LookKey {
    pub fn from_vk(vk: u32) -> Option<LookKey> {
        Some(
            match vk {
                VK_SPACE => LookKey::Space,
                VK_RETURN => LookKey::Enter,
                VK_ESCAPE => LookKey::Escape,
                VK_UP => LookKey::Up,
                VK_DOWN => LookKey::Down,
                VK_LEFT => LookKey::Left,
                VK_RIGHT => LookKey::Right,
                VK_F5 => LookKey::F5,
                VK_F11 => LookKey::F11,
                _ => return None,
            },
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookAction {
    Toggle,
    RunAndClose,
    Close,
    Reload,
    Fullscreen,
    SelectionMoved,
}
#[derive(Debug, Default)]
pub struct KeyGate {
    /// Son geçersiz tuş basma anı.
    last_invalid: Option<u64>,
    /// Boşluk basılıysa basıldığı an.
    space_down: Option<u64>,
}
impl KeyGate {
    pub fn new() -> KeyGate {
        KeyGate::default()
    }
    /// Ön plandaki pencere değişti: yazma koruması biter.
    pub fn foreground_changed(&mut self) {
        self.last_invalid = None;
    }
    pub fn on_key(
        &mut self,
        vk: u32,
        down: bool,
        modifiers: bool,
        now_ms: u64,
        target_ok: bool,
    ) -> Option<LookAction> {
        let Some(key) = LookKey::from_vk(vk) else {
            if down {
                self.last_invalid = Some(now_ms);
            }
            return None;
        };
        let released_at = if key == LookKey::Space && !down {
            self.space_down.take()
        } else {
            None
        };
        if let Some(t) = self.last_invalid {
            if now_ms.saturating_sub(t) < GUARD_MS {
                return None;
            }
            self.last_invalid = None;
        }
        if modifiers || !target_ok {
            return None;
        }
        match (key, down) {
            (LookKey::Space, true) => {
                if self.space_down.is_some() {
                    return None;
                }
                self.space_down = Some(now_ms);
                Some(LookAction::Toggle)
            }
            (LookKey::Space, false) => {
                let held = released_at?;
                (now_ms.saturating_sub(held) >= HOLD_CLOSE_MS)
                    .then_some(LookAction::Close)
            }
            (LookKey::Enter, true) => Some(LookAction::RunAndClose),
            (LookKey::Escape, false) => Some(LookAction::Close),
            (LookKey::F5, true) => Some(LookAction::Reload),
            (LookKey::F11, true) => Some(LookAction::Fullscreen),
            (LookKey::Up | LookKey::Down | LookKey::Left | LookKey::Right, false) => {
                Some(LookAction::SelectionMoved)
            }
            _ => None,
        }
    }
}
