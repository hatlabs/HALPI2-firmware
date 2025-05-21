use core::ops::Range;

unsafe extern "C" {
    static mut _bootloader_state_start: u32;
    static mut _bootloader_state_end: u32;

    static mut __bootloader_dfu_start: u32;
    static mut __bootloader_dfu_end: u32;

    static mut __bootloader_appdata_start: u32;
    static mut __bootloader_appdata_end: u32;
}

/// The size of a page in bytes
pub const PAGE_SIZE: u32 = 0x0000_1000;

pub fn get_bootloader_state_range() -> Range<u32> {
    unsafe { _bootloader_state_start.._bootloader_state_end }
}
pub fn get_bootloader_dfu_range() -> Range<u32> {
    unsafe { __bootloader_dfu_start..__bootloader_dfu_end }
}
pub fn get_bootloader_appdata_range() -> Range<u32> {
    unsafe { __bootloader_appdata_start..__bootloader_appdata_end }
}
pub fn get_bootloader_state_size() -> u32 {
    get_bootloader_state_range().end - get_bootloader_state_range().start
}
pub fn get_bootloader_dfu_size() -> u32 {
    get_bootloader_dfu_range().end - get_bootloader_dfu_range().start
}
pub fn get_bootloader_appdata_size() -> u32 {
    get_bootloader_appdata_range().end - get_bootloader_appdata_range().start
}
