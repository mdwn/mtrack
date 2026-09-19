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

//! Bring-your-own kinematic cross-check of the pointing convention
//! (venue-exchange design §18.4): for every moving head in the GDTFs
//! embedded in `tests/mvr-corpus/*.mvr` (gitignored), the pose the
//! pointing math produces, pushed through the manufacturer's geometry
//! tree exactly as a GDTF renderer turns it, must send the beam the same
//! way the math says it does.
//!
//! ```sh
//! cargo test --test pointing_corpus -- --ignored --nocapture
//! ```
//!
//! Aiming goes through each rig's own calibration (design §18.6), so a
//! yawed yoke or a pitched lens in a manufacturer's file is followed, not
//! fought. A rig whose geometry the calibration cannot reduce is printed
//! with the reason; every other rig must agree to a hundredth of a degree.

use std::collections::HashSet;

use mtrack::lighting::effects::{AimCalibration, Pose};
use mtrack::lighting::gdtf;
use mtrack::lighting::mvr;

fn corpus_files() -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mvr-corpus");
    assert!(
        dir.is_dir(),
        "no corpus at {}: drop .mvr files in (see module docs)",
        dir.display()
    );
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().is_some_and(|e| e == "mvr")).then_some(path)
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "corpus dir {} holds no .mvr files",
        dir.display()
    );
    files
}

/// The largest angle, in degrees, between the rig's beam and the math's
/// direction over a grid of poses.
fn worst_error_deg(rig: &gdtf::RigModel, calibration: &AimCalibration) -> Option<f64> {
    let mut worst: f64 = 0.0;
    for pan in (-270..=270).step_by(30) {
        for tilt in (-135..=135).step_by(15) {
            let pose = Pose {
                pan: f64::from(pan),
                tilt: f64::from(tilt),
            };
            let math = calibration.direction([0.0; 3], pose);
            let beam = gdtf::beam_direction(rig, pose.pan, pose.tilt)?;
            let dot = (math[0] * beam[0] + math[1] * beam[1] + math[2] * beam[2]).clamp(-1.0, 1.0);
            worst = worst.max(dot.acos().to_degrees());
        }
    }
    Some(worst)
}

#[test]
#[ignore = "bring-your-own corpus: put .mvr files in tests/mvr-corpus/ and run with --ignored"]
fn every_corpus_mover_points_where_the_math_says() {
    let mut rigs = 0usize;
    let mut agree = 0usize;
    let mut disagreements: Vec<(String, f64)> = Vec::new();
    let mut uncalibrated: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for path in corpus_files() {
        let bytes = std::fs::read(&path).expect("readable mvr");
        let entries = mvr::list_gdtf_entries(&bytes).expect("gdtf entries");
        for entry in entries {
            if !seen.insert(entry.clone()) {
                continue;
            }
            let gdtf_bytes = match mvr::read_gdtf_entry(&bytes, &entry) {
                Ok(b) => b,
                Err(e) => {
                    println!("{entry}: unreadable: {e}");
                    continue;
                }
            };
            let xml = match gdtf::read_description_xml(&gdtf_bytes) {
                Ok(x) => x,
                Err(e) => {
                    println!("{entry}: no description: {e}");
                    continue;
                }
            };
            let description = match gdtf::parse_description(&xml) {
                Ok(d) => d,
                Err(e) => {
                    println!("{entry}: does not parse: {e}");
                    continue;
                }
            };
            for mode in &description.modes {
                let rig = match gdtf::distill_rig(&description, &mode.name, &HashSet::new()) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                if rig.pan.is_none() || rig.tilt.is_none() {
                    continue;
                }
                rigs += 1;
                let name = format!("{} / {}", description.name, mode.name);
                // A rig the calibration refuses is aimed by the plain
                // convention in production, so that is what it is checked
                // against, and the error it would carry is reported.
                let calibration = match gdtf::aim_calibration(&rig) {
                    Ok(c) => c,
                    Err(reason) => {
                        let plain = worst_error_deg(&rig, &AimCalibration::IDENTITY);
                        uncalibrated.push((
                            name,
                            format!(
                                "{reason}; plain convention off by {:.2}°",
                                plain.unwrap_or(f64::NAN)
                            ),
                        ));
                        continue;
                    }
                };
                if !calibration.is_identity() {
                    println!(
                        "calibrated: {name} (pan offset {:.1}°, tilt offset {:.1}°, pre {:?})",
                        calibration.pan_offset, calibration.tilt_offset, calibration.pre
                    );
                }
                match worst_error_deg(&rig, &calibration) {
                    Some(err) if err < 0.01 => agree += 1,
                    Some(err) => disagreements.push((name, err)),
                    None => println!("{name}: no beam"),
                }
            }
        }
    }

    disagreements.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (name, err) in &disagreements {
        println!("disagrees by {err:.2}°: {name}");
    }
    for (name, reason) in &uncalibrated {
        println!("not calibrated ({reason}): {name}");
    }
    println!(
        "{rigs} mover rigs: {agree} agree with the pointing math, {} do not, {} could not be calibrated",
        disagreements.len(),
        uncalibrated.len()
    );
    assert!(rigs > 0, "the corpus holds no moving heads");
    assert!(
        disagreements.is_empty(),
        "calibrated rigs disagree with the pointing math: {disagreements:?}"
    );
}
