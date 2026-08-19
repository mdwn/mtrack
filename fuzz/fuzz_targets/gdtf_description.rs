// The description parser must never panic on arbitrary XML-ish text.
// Run with: cargo +nightly fuzz run gdtf_description
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let _ = mtrack::lighting::gdtf::parse_description(data);
});
