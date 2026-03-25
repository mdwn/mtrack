// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//
use std::sync::Arc;

use tracing::{error, info, warn};

use crate::config::MorningstarConfig;
use crate::songs::Song;

/// Implements `SongChangeNotifier` for Morningstar controllers.
/// Captures the MIDI device at construction time and sends a SysEx
/// preset-name update whenever the current song changes.
pub struct Notifier {
    config: MorningstarConfig,
    device: Arc<dyn super::Device>,
}

impl Notifier {
    pub fn new(config: MorningstarConfig, device: Arc<dyn super::Device>) -> Notifier {
        Notifier { config, device }
    }
}

impl crate::player::SongChangeNotifier for Notifier {
    fn notify(&self, song: &Song) {
        let sysex = build_update_bank_name(&self.config, song.name());
        if let Err(e) = self.device.emit_sysex(&sysex) {
            error!("Error emitting Morningstar bank name SysEx: {:?}", e);
        }
    }
}

/// Morningstar SysEx manufacturer ID prefix.
const MANUFACTURER_ID: [u8; 3] = [0x00, 0x21, 0x24];

/// Checks if a raw MIDI message is a Morningstar SysEx acknowledgement and
/// logs the result. Returns true if the message was a Morningstar ack (so
/// callers can skip further processing if desired).
pub fn check_ack(raw_event: &[u8]) -> bool {
    // Minimum ack length: F0 + 3 mfr + model + 00 + 70 + 7F + ack + ... + checksum + F7
    if raw_event.len() < 10 {
        return false;
    }
    if raw_event[0] != 0xF0 || raw_event[1..4] != MANUFACTURER_ID {
        return false;
    }
    // op2 = 0x7F indicates an ack response
    if raw_event[7] != 0x7F {
        return false;
    }

    let ack_code = raw_event[8];
    match ack_code {
        0x00 => info!("Morningstar SysEx acknowledged (success)"),
        0x01 => warn!("Morningstar SysEx rejected: wrong model ID"),
        0x02 => warn!("Morningstar SysEx rejected: wrong checksum"),
        0x03 => warn!("Morningstar SysEx rejected: wrong payload size"),
        code => warn!(code, "Morningstar SysEx rejected: unknown ack code"),
    }

    true
}

/// Builds the SysEx message to update the current bank name on a Morningstar controller.
///
/// Message format:
/// `F0 00 21 24 <model_id> 00 70 10 00 <save_flag> 00 00 00 <txn_id=00> 00 00 <name_bytes...> <checksum> F7`
///
/// - op2: 0x10 (update current bank name)
/// - save_flag: 0x7F (save to flash) or 0x00 (temporary)
/// - checksum: XOR of all bytes from F0 through last name byte, AND 0x7F
/// - Name is truncated or padded with spaces to the model's required length.
pub fn build_update_bank_name(config: &MorningstarConfig, name: &str) -> Vec<u8> {
    let name_len = config.name_length();
    let padded = format!("{:<width$}", name, width = name_len);
    // Truncate if longer than required (format! won't truncate).
    let padded: String = padded.chars().take(name_len).collect();

    let save_flag: u8 = if config.save() { 0x7F } else { 0x00 };

    let mut msg: Vec<u8> = Vec::with_capacity(16 + name_len + 2);

    // Header
    msg.push(0xF0); // SysEx start
    msg.push(0x00); // Morningstar manufacturer ID byte 1
    msg.push(0x21); // Morningstar manufacturer ID byte 2
    msg.push(0x24); // Morningstar manufacturer ID byte 3
    msg.push(config.model_id()); // Device model ID
    msg.push(0x00); // ignore
    msg.push(0x70); // opcode 1
    msg.push(0x10); // opcode 2: update current bank name
    msg.push(0x00); // opcode 3
    msg.push(save_flag); // opcode 4: save flag
    msg.push(0x00); // opcode 5
    msg.push(0x00); // opcode 6
    msg.push(0x00); // opcode 7
    msg.push(0x00); // transaction ID
    msg.push(0x00); // ignore
    msg.push(0x00); // ignore

    // Name bytes (padded to model's required length)
    for b in padded.as_bytes() {
        msg.push(*b);
    }

    // Checksum: XOR of all bytes so far, masked to 7 bits
    let checksum = msg.iter().fold(0u8, |acc, &b| acc ^ b) & 0x7F;
    msg.push(checksum);

    // SysEx end
    msg.push(0xF7);

    msg
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::MorningstarModel;

    fn make_config(model: MorningstarModel, save: bool) -> MorningstarConfig {
        MorningstarConfig::new(model, save)
    }

    #[test]
    fn basic_message_structure() {
        let config = make_config(MorningstarModel::MC6Pro, false);
        let msg = build_update_bank_name(&config, "Test");

        assert_eq!(msg[0], 0xF0);
        assert_eq!(&msg[1..4], &[0x00, 0x21, 0x24]);
        assert_eq!(msg[4], 0x06); // MC6Pro model ID
        assert_eq!(msg[6], 0x70); // opcode 1
        assert_eq!(msg[7], 0x10); // opcode 2: update current bank name
        assert_eq!(msg[8], 0x00); // opcode 3
        assert_eq!(msg[9], 0x00); // save flag (false)
        assert_eq!(*msg.last().unwrap(), 0xF7);
    }

    #[test]
    fn save_flag_set() {
        let config = make_config(MorningstarModel::MC4Pro, true);
        let msg = build_update_bank_name(&config, "X");

        assert_eq!(msg[9], 0x7F);
    }

    #[test]
    fn name_bytes_padded_with_spaces() {
        let config = make_config(MorningstarModel::MC4Pro, false);
        let msg = build_update_bank_name(&config, "ABC");

        // Name starts at index 16 (14 header + 2 ignore bytes)
        assert_eq!(msg[16], b'A');
        assert_eq!(msg[17], b'B');
        assert_eq!(msg[18], b'C');
        // Remaining 29 bytes should be spaces (MC4Pro = 32 char name)
        for (i, &byte) in msg.iter().enumerate().take(48).skip(19) {
            assert_eq!(byte, b' ', "byte {} should be space padding", i);
        }
        // Total: 16 header + 32 name + checksum + F7 = 50
        assert_eq!(msg.len(), 50);
    }

    #[test]
    fn truncation_at_model_limit() {
        let config = make_config(MorningstarModel::MC4Pro, false);
        let long_name = "A".repeat(50);
        let msg = build_update_bank_name(&config, &long_name);

        // 16 header bytes + 32 name bytes + checksum + F7 = 50
        assert_eq!(msg.len(), 50);
        for &byte in msg.iter().take(48).skip(16) {
            assert_eq!(byte, b'A');
        }
    }

    #[test]
    fn mc3_uses_16_char_name() {
        let config = make_config(MorningstarModel::MC3, false);
        let msg = build_update_bank_name(&config, "Hi");

        // 16 header + 16 name + checksum + F7 = 34
        assert_eq!(msg.len(), 34);
        assert_eq!(msg[16], b'H');
        assert_eq!(msg[17], b'i');
        // Rest is space padding
        for &byte in msg.iter().take(32).skip(18) {
            assert_eq!(byte, b' ');
        }
    }

    #[test]
    fn mc6_uses_24_char_name() {
        let config = make_config(MorningstarModel::MC6, false);
        let msg = build_update_bank_name(&config, "Test");

        // 16 header + 24 name + checksum + F7 = 42
        assert_eq!(msg.len(), 42);
    }

    #[test]
    fn checksum_is_correct() {
        let config = make_config(MorningstarModel::MC4Pro, false);
        let msg = build_update_bank_name(&config, "Hi");

        let checksum_idx = msg.len() - 2;
        let expected = msg[..checksum_idx].iter().fold(0u8, |acc, &b| acc ^ b) & 0x7F;
        assert_eq!(msg[checksum_idx], expected);
    }

    #[test]
    fn checksum_masked_to_7_bits() {
        let config = make_config(MorningstarModel::MC4Pro, false);
        let msg = build_update_bank_name(&config, "~~~~~");

        let checksum_idx = msg.len() - 2;
        assert!(msg[checksum_idx] <= 0x7F);
    }

    #[test]
    fn all_model_ids() {
        let models = vec![
            (MorningstarModel::MC3, 0x05),
            (MorningstarModel::MC6, 0x03),
            (MorningstarModel::MC8, 0x04),
            (MorningstarModel::MC6Pro, 0x06),
            (MorningstarModel::MC8Pro, 0x08),
            (MorningstarModel::MC4Pro, 0x09),
            (
                MorningstarModel::Custom(crate::config::CustomModel { model_id: 0x0A }),
                0x0A,
            ),
        ];

        for (model, expected_id) in models {
            let config = make_config(model, false);
            let msg = build_update_bank_name(&config, "X");
            assert_eq!(msg[4], expected_id, "Model ID mismatch");
        }
    }

    #[test]
    fn empty_name_padded_to_model_length() {
        let config = make_config(MorningstarModel::MC4Pro, false);
        let msg = build_update_bank_name(&config, "");

        // 16 header + 32 spaces + checksum + F7 = 50
        assert_eq!(msg.len(), 50);
        for &byte in msg.iter().take(48).skip(16) {
            assert_eq!(byte, b' ');
        }
    }

    #[test]
    fn exactly_32_chars_not_truncated() {
        let config = make_config(MorningstarModel::MC4Pro, false);
        let name = "A".repeat(32);
        let msg = build_update_bank_name(&config, &name);

        // 16 + 32 + 1 + 1 = 50
        assert_eq!(msg.len(), 50);
        // No padding needed — name fills the slot exactly
        for &byte in msg.iter().take(48).skip(16) {
            assert_eq!(byte, b'A');
        }
    }

    #[test]
    fn config_deserialization() {
        use config::Config;
        use config::File;
        use config::FileFormat;

        let yaml = r#"
            model: mc4pro
            save: true
        "#;

        let config: MorningstarConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.model_id(), 0x09);
        assert!(config.save());
    }

    #[test]
    fn config_deserialization_defaults() {
        use config::Config;
        use config::File;
        use config::FileFormat;

        let yaml = r#"
            model: mc6pro
        "#;

        let config: MorningstarConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.model_id(), 0x06);
        assert!(!config.save());
    }

    #[test]
    fn config_deserialization_custom_model() {
        use config::Config;
        use config::File;
        use config::FileFormat;

        let yaml = r#"
            model:
              custom:
                model_id: 15
        "#;

        let config: MorningstarConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.model_id(), 15);
    }

    mod check_ack_tests {
        use super::*;

        fn make_ack(ack_code: u8) -> Vec<u8> {
            // F0 00 21 24 <model> 00 70 7F <ack> 00 00 00 00 <txn> 00 00 <checksum> F7
            vec![
                0xF0, 0x00, 0x21, 0x24, 0x09, 0x00, 0x70, 0x7F, ack_code, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0xF7,
            ]
        }

        #[test]
        fn recognizes_success_ack() {
            assert!(check_ack(&make_ack(0x00)));
        }

        #[test]
        fn recognizes_wrong_model_id() {
            assert!(check_ack(&make_ack(0x01)));
        }

        #[test]
        fn recognizes_wrong_checksum() {
            assert!(check_ack(&make_ack(0x02)));
        }

        #[test]
        fn recognizes_wrong_payload_size() {
            assert!(check_ack(&make_ack(0x03)));
        }

        #[test]
        fn ignores_non_sysex() {
            assert!(!check_ack(&[0x90, 0x3C, 0x64]));
        }

        #[test]
        fn ignores_wrong_manufacturer() {
            assert!(!check_ack(&[
                0xF0, 0x00, 0x00, 0x00, 0x09, 0x00, 0x70, 0x7F, 0x00, 0xF7,
            ]));
        }

        #[test]
        fn ignores_non_ack_sysex() {
            // op2 = 0x10 (bank name update), not 0x7F (ack)
            assert!(!check_ack(&[
                0xF0, 0x00, 0x21, 0x24, 0x09, 0x00, 0x70, 0x10, 0x00, 0xF7,
            ]));
        }

        #[test]
        fn ignores_too_short() {
            assert!(!check_ack(&[0xF0, 0x00, 0x21, 0x24]));
        }
    }
}
