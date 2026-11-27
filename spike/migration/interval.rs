#![no_std]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
#[link(wasm_import_module = "sdk")]
extern "C" {
    fn last_days() -> i32;
    fn save_days(value: i32);
}
#[no_mangle]
pub extern "C" fn refresh(target: i32, today: i32) -> i32 {
    let previous = unsafe { last_days() };
    unsafe { save_days((target - today).clamp(5, 180)) }
    previous
}
