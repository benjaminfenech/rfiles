//! Önizleme penceresi için Win32 yardımcıları (üstte tut, konum, görev çubuğu, çalışma alanı).
// Generated from rf-win's public API by a one-off stub generator, then edited by hand.
// Functions return safe defaults until they get a Linux implementation.
#![allow(unused)]
fn work_of(mon: crate::Opaque) -> [f32; 4] {
    Default::default()
}
/// İmlecin bulunduğu monitörün çalışma alanı, GPUI'nin mantıksal koordinatlarında (x, y, g, y).
pub fn work_area_at_cursor() -> [f32; 4] {
    Default::default()
}
/// Pencerenin bulunduğu monitörün çalışma alanı (mantıksal).
pub fn work_area_of(hwnd: isize) -> [f32; 4] {
    Default::default()
}
/// Başka bir sürecin (ör. DirectLook'tan çağrılan rustyfiles) pencereyi öne alabilmesine izin verir;
/// yoksa Windows ön plan kilidi yüzünden pencere yalnız görev çubuğunda yanıp söner.
pub fn allow_any_foreground() {}
pub fn set_topmost(hwnd: isize, on: bool) {}
/// Pencereyi mantıksal dikdörtgene taşır (`scale`: pencerenin DPI ölçeği).
pub fn set_rect(hwnd: isize, rect: [f32; 4], scale: f32) {}
thread_local! {
    #[doc =
    " Görev çubuğunda görünmeyecek pencerelerin gizli sahibi (pencerenin iş parçacığında yaratılır)."]
    static OWNER : std::cell::Cell < isize > = const { std::cell::Cell::new(0) };
}
fn hidden_owner() -> isize {
    Default::default()
}
/// Görev çubuğunda görünsün mü. Gizlemek için pencereye gizli bir sahip verilir (araç penceresi
/// stili Windows'un küçük kapat düğmesini başlık çubuğunun altında belirginleştiriyor).
pub fn set_show_in_taskbar(hwnd: isize, show: bool) {}
/// Bu adla çalışan bir süreç var mı (ör. "QuickLook.exe"; büyük/küçük harf duyarsız).
pub fn process_running(exe: &str) -> bool {
    false
}
