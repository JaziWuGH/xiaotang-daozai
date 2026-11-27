#![no_std]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
#[link(wasm_import_module = "sdk")]
extern "C" {
    fn last_days() -> i32;
    fn save_days(days: i32);
    fn request(ptr: *const u8, len: usize) -> i32;
}
#[no_mangle]
pub extern "C" fn refresh(target: i32, today: i32) -> i32 {
    unsafe {
        let previous = last_days();
        save_days(target - today);
        previous
    }
}
#[no_mangle]
pub extern "C" fn fault(which: i32) -> i32 {
    use core::arch::wasm32;
    match which {
        1 => loop {
            core::hint::spin_loop()
        },
        2 => wasm32::unreachable(),
        3 => wasm32::memory_grow::<0>(256) as i32,
        4 => {
            let url = b"https://denied.example/test";
            unsafe { request(url.as_ptr(), url.len()) }
        }
        _ => {
            let url = b"https://allowed.example/test";
            unsafe { request(url.as_ptr(), url.len()) }
        }
    }
}
