// GDTF archives are untrusted input (a venue's file arrives by email from a
// stranger); the archive layer must never panic, hang, or blow memory on
// arbitrary bytes. Run with: cargo +nightly fuzz run gdtf_archive
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = mtrack::lighting::gdtf::read_description_xml(data);
});
