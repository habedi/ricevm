#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The loader should never panic on arbitrary input.
    // It may return Ok or Err, but must not crash.
    let _ = ricevm_loader::load(data);
});
