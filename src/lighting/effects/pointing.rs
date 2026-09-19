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

//! Pointing math (venue-exchange design §15.3, convention §18): from a
//! fixture's place and mounting to the pan and tilt that aim it at a
//! stage point.
//!
//! Pose degrees are GDTF physical degrees, so what the resolver writes
//! through a fixture's `PhysicalFrom..PhysicalTo` means what the
//! manufacturer meant. With the mounting rotation `R = Rz·Ry·Rx` (the
//! venue's `rotation`, degrees about X, Y, Z applied in that order):
//!
//! ```text
//! direction = R · Rz(pan) · Rx(tilt) · (0, 0, −1)
//! ```
//!
//! Rest (pan 0, tilt 0) is the mounting frame's −Z — straight down for a
//! hung fixture, the way GDTF models every fixture. Positive pan is a
//! right-hand turn about +Z (counter-clockwise seen from above); positive
//! tilt a right-hand turn about +X (the beam swings from −Z toward +Y).
//! That is the reading Blender DMX gives a GDTF's physical values, and
//! the one real ranges (`tilt 131.8 → −131.8`, mid-travel straight down)
//! only make sense under.
//!
//! The inverse has two solutions, `(pan, tilt)` and `(pan + 180°, −tilt)`;
//! [`aim_solutions`] returns both and the engine picks (design §18.2).
//! No geometry-tree kinematics: a page of trigonometry, property-tested
//! and cross-checked against rig kinematics in `golden_tests`.

/// A pan/tilt pair in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pan: f64,
    pub tilt: f64,
}

/// How a fixture's joints sit inside its mounting frame, read from its
/// GDTF geometry (design §18.6). Most fixtures are the identity: the
/// yoke turns about the mounting's Z and the head about the yoke's X,
/// with the beam down −Z at rest. Some are not — the Ayrton MagicDot SX
/// yaws its yoke geometry 90° — and for those the plain math would aim
/// the head into the wrong plane. With a calibration the beam is
///
/// ```text
/// direction = R · pre · Rz(pan + pan_offset) · Rx(tilt + tilt_offset) · (0, 0, −1)
/// ```
///
/// `pre` is the rotation the geometry puts before the pan joint, and the
/// offsets are yaws between the joints and the beam's rest angle in the
/// head. The three translations put the lens where the geometry has it,
/// so the beam is aimed from the lens, not the mounting point:
///
/// ```text
/// origin = mount_to_pan + pre · Rz(pan) · (pan_to_tilt + Rz(pan_offset) · Rx(tilt) · tilt_to_lens)
/// ```
///
/// Derived by [`crate::lighting::gdtf::aim_calibration`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimCalibration {
    /// Rotation from the joint frame into the mounting frame (row-major).
    pub pre: [[f64; 3]; 3],
    /// Degrees added to the pan the geometry wants, to get the pan the
    /// fixture's own channel means.
    pub pan_offset: f64,
    /// Likewise for tilt.
    pub tilt_offset: f64,
    /// The pan joint's position in the mounting frame, meters.
    pub mount_to_pan: [f64; 3],
    /// The tilt joint's position in the pan joint's frame.
    pub pan_to_tilt: [f64; 3],
    /// The lens's position in the tilt joint's frame.
    pub tilt_to_lens: [f64; 3],
}

impl AimCalibration {
    /// A fixture whose joints are the mounting frame's axes and whose lens
    /// is at the mounting point.
    pub const IDENTITY: AimCalibration = AimCalibration {
        pre: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        pan_offset: 0.0,
        tilt_offset: 0.0,
        mount_to_pan: [0.0; 3],
        pan_to_tilt: [0.0; 3],
        tilt_to_lens: [0.0; 3],
    };

    /// Where the lens is at a pose, as an offset from the fixture's
    /// position in stage space (meters).
    pub fn origin(&self, rotation: [f64; 3], pose: Pose) -> [f64; 3] {
        // The lens offset is in the tilt joint's frame, so it turns with
        // the channel tilt; that frame sits `pan_offset` around from the
        // pan joint's, which turns with the channel pan inside `pre`.
        let head = rot_x(pose.tilt.to_radians(), self.tilt_to_lens);
        let in_pan = add(self.pan_to_tilt, rot_z(self.pan_offset.to_radians(), head));
        let in_mount = add(
            self.mount_to_pan,
            mat_vec(self.pre, rot_z(pose.pan.to_radians(), in_pan)),
        );
        out_of_frame(rotation, in_mount)
    }

    /// Whether the lens sits away from the mounting point at all.
    fn has_lens_offset(&self) -> bool {
        [self.mount_to_pan, self.pan_to_tilt, self.tilt_to_lens]
            .iter()
            .any(|t| t.iter().any(|c| c.abs() > 1e-9))
    }

    /// Whether this is the identity: no geometry correction at all.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Whether the joints sit on the mounting frame's axes with no
    /// offsets — the plain convention, whatever the lens's position.
    pub fn frame_is_identity(&self) -> bool {
        self.pre == Self::IDENTITY.pre
            && self.pan_offset.abs() < 1e-9
            && self.tilt_offset.abs() < 1e-9
    }

    /// The direction (stage space, unit) a pose looks along through this
    /// geometry: see [`direction`].
    pub fn direction(&self, rotation: [f64; 3], pose: Pose) -> [f64; 3] {
        let joint = joint_direction(Pose {
            pan: pose.pan + self.pan_offset,
            tilt: pose.tilt + self.tilt_offset,
        });
        out_of_frame(rotation, mat_vec(self.pre, joint))
    }

    /// Both poses aiming at a stage point through this geometry: see
    /// [`aim_solutions`]. The beam leaves the lens, which moves with the
    /// pose, so the aim is refined from the lens's position a few times;
    /// each step corrects a head's length over metres of throw, and the
    /// solutions settle to well under a hundredth of a degree.
    pub fn aim_solutions(
        &self,
        position: [f64; 3],
        rotation: [f64; 3],
        target: [f64; 3],
    ) -> [Pose; 2] {
        let mut solutions = self.aim_from(position, rotation, target);
        if !self.has_lens_offset() {
            return solutions;
        }
        for _ in 0..4 {
            solutions = [0, 1].map(|i| {
                let lens = add(position, self.origin(rotation, solutions[i]));
                self.aim_from(lens, rotation, target)[i]
            });
        }
        solutions
    }

    /// Both poses sending the beam from `from` to `target`, ignoring the
    /// lens offset.
    fn aim_from(&self, from: [f64; 3], rotation: [f64; 3], target: [f64; 3]) -> [Pose; 2] {
        let position = from;
        let d = [
            target[0] - position[0],
            target[1] - position[1],
            target[2] - position[2],
        ];
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if length < 1e-9 {
            return [Pose {
                pan: 0.0,
                tilt: 0.0,
            }; 2];
        }
        let mounting = into_frame(rotation, [d[0] / length, d[1] / length, d[2] / length]);
        let local = mat_t_vec(self.pre, mounting);
        joint_solutions(local).map(|pose| Pose {
            pan: wrap_pan(pose.pan - self.pan_offset),
            tilt: pose.tilt - self.tilt_offset,
        })
    }
}

/// The beam a joint pose sends, in the joint frame:
/// `Rz(pan)·Rx(tilt)·(0,0,−1)`.
fn joint_direction(pose: Pose) -> [f64; 3] {
    let (pan, tilt) = (pose.pan.to_radians(), pose.tilt.to_radians());
    [-pan.sin() * tilt.sin(), pan.cos() * tilt.sin(), -tilt.cos()]
}

/// Both joint poses sending the beam along a unit direction in the joint
/// frame: the principal (tilt in `0..=180`) and its flip.
fn joint_solutions(local: [f64; 3]) -> [Pose; 2] {
    let tilt = (-local[2]).clamp(-1.0, 1.0).acos().to_degrees();
    let pan = if local[0].hypot(local[1]) < 1e-9 {
        0.0
    } else {
        (-local[0]).atan2(local[1]).to_degrees()
    };
    let flipped = if pan > 0.0 { pan - 180.0 } else { pan + 180.0 };
    [
        Pose { pan, tilt },
        Pose {
            pan: flipped,
            tilt: -tilt,
        },
    ]
}

/// A pan folded into `-180..=180`.
fn wrap_pan(pan: f64) -> f64 {
    let wrapped = (pan + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped == -180.0 && pan > 0.0 {
        180.0
    } else {
        wrapped
    }
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn rot_z(angle: f64, v: [f64; 3]) -> [f64; 3] {
    let (c, s) = (angle.cos(), angle.sin());
    [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

fn rot_x(angle: f64, v: [f64; 3]) -> [f64; 3] {
    let (c, s) = (angle.cos(), angle.sin());
    [v[0], c * v[1] - s * v[2], s * v[1] + c * v[2]]
}

fn mat_vec(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn mat_t_vec(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

/// Rotates a stage-space vector into a mounting frame given by degrees
/// about X, Y and Z applied in that order — the inverse (transpose) of
/// `R = Rz·Ry·Rx`.
fn into_frame(rotation_deg: [f64; 3], v: [f64; 3]) -> [f64; 3] {
    let (rx, ry, rz) = (
        rotation_deg[0].to_radians(),
        rotation_deg[1].to_radians(),
        rotation_deg[2].to_radians(),
    );
    // Apply Rzᵀ, then Ryᵀ, then Rxᵀ.
    let (cz, sz) = (rz.cos(), rz.sin());
    let v = [cz * v[0] + sz * v[1], -sz * v[0] + cz * v[1], v[2]];
    let (cy, sy) = (ry.cos(), ry.sin());
    let v = [cy * v[0] - sy * v[2], v[1], sy * v[0] + cy * v[2]];
    let (cx, sx) = (rx.cos(), rx.sin());
    [v[0], cx * v[1] + sx * v[2], -sx * v[1] + cx * v[2]]
}

/// Rotates a mounting-frame vector out into stage space: `R = Rz·Ry·Rx` —
/// where a cell's offset in the fixture's frame lands on the stage.
pub fn out_of_frame(rotation_deg: [f64; 3], v: [f64; 3]) -> [f64; 3] {
    let (rx, ry, rz) = (
        rotation_deg[0].to_radians(),
        rotation_deg[1].to_radians(),
        rotation_deg[2].to_radians(),
    );
    let (cx, sx) = (rx.cos(), rx.sin());
    let v = [v[0], cx * v[1] - sx * v[2], sx * v[1] + cx * v[2]];
    let (cy, sy) = (ry.cos(), ry.sin());
    let v = [cy * v[0] + sy * v[2], v[1], -sy * v[0] + cy * v[2]];
    let (cz, sz) = (rz.cos(), rz.sin());
    [cz * v[0] - sz * v[1], sz * v[0] + cz * v[1], v[2]]
}

/// Both poses that aim a fixture at `position` with mounting `rotation`
/// (degrees about X, Y, Z) at a stage point: first the principal one
/// (tilt in `0..=180`), then its flip (`pan + 180°`, `−tilt`). Pans are in
/// `-180..=180`; [`nearest_pan`] picks the turn. A target on the pan axis
/// (straight down or up) has no pan of its own and gets 0 here — the
/// engine holds the head's current pan for it; a target on top of the
/// fixture has no direction at all and gets rest.
pub fn aim_solutions(position: [f64; 3], rotation: [f64; 3], target: [f64; 3]) -> [Pose; 2] {
    AimCalibration::IDENTITY.aim_solutions(position, rotation, target)
}

/// The principal pose aiming at a stage point: tilt in `0..=180`. See
/// [`aim_solutions`] for the flip.
pub fn aim(position: [f64; 3], rotation: [f64; 3], target: [f64; 3]) -> Pose {
    aim_solutions(position, rotation, target)[0]
}

/// The direction (stage space, unit) a pose looks along: the beam of a
/// fixture at rest points down its mounting frame's −Z, pan turns it
/// about +Z, tilt about +X.
pub fn direction(rotation: [f64; 3], pose: Pose) -> [f64; 3] {
    AimCalibration::IDENTITY.direction(rotation, pose)
}

/// Picks, among the pans equivalent to `pan` modulo 360° that lie within
/// `range`, the one nearest `current` — what a desk does, and what stops a
/// mover flipping through 500° to reach a point 10° away. Falls back to
/// the principal solution clamped to the range when none fits (the
/// resolver reports the clamp).
pub fn nearest_pan(pan: f64, current: f64, range: (f64, f64)) -> f64 {
    let (low, high) = if range.0 <= range.1 {
        range
    } else {
        (range.1, range.0)
    };
    let mut best: Option<f64> = None;
    for turn in -2..=2 {
        let candidate = pan + f64::from(turn) * 360.0;
        if candidate < low || candidate > high {
            continue;
        }
        match best {
            Some(b) if (b - current).abs() <= (candidate - current).abs() => {}
            _ => best = Some(candidate),
        }
    }
    best.unwrap_or_else(|| pan.clamp(low, high))
}

/// Interpolates a pose along the shortest path in degrees.
pub fn lerp(from: Pose, to: Pose, t: f64) -> Pose {
    Pose {
        pan: from.pan + (to.pan - from.pan) * t,
        tilt: from.tilt + (to.tilt - from.tilt) * t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_hung_fixture_aims_by_the_gdtf_convention() {
        // Hung at 4 m, 3 m upstage, unrotated: at rest it points straight
        // down. The drummer 1 m further upstage on the deck is 1 m over
        // for 4 m down: tilt +atan(1/4) toward +y, pan 0.
        let pose = aim([0.0, 3.0, 4.0], [0.0; 3], [0.0, 4.0, 0.0]);
        assert!(close(pose.pan, 0.0), "{pose:?}");
        assert!(
            close(pose.tilt, (1.0f64).atan2(4.0).to_degrees()),
            "{pose:?}"
        );

        // Straight down is rest.
        let down = aim([0.0, 3.0, 4.0], [0.0; 3], [0.0, 3.0, 0.0]);
        assert!(close(down.pan, 0.0) && close(down.tilt, 0.0), "{down:?}");

        // A target at +x, level: tilt 90° brings the beam to +y, and a
        // right-hand turn of −90° about +Z takes +y to +x.
        let left = aim([0.0, 0.0, 4.0], [0.0; 3], [5.0, 0.0, 4.0]);
        assert!(close(left.pan, -90.0), "{left:?}");
        assert!(close(left.tilt, 90.0), "{left:?}");
    }

    #[test]
    fn the_seeded_rear_fixture_faces_the_audience() {
        // rotation (0, 0, 180) — a hung rear-truss mover yawed to face
        // downstage: its local +y points downstage, so a level downstage
        // target is pan 0, tilt 90.
        let pose = aim([0.0, 3.5, 4.2], [0.0, 0.0, 180.0], [0.0, 0.0, 4.2]);
        assert!(close(pose.pan, 0.0), "{pose:?}");
        assert!(close(pose.tilt, 90.0), "{pose:?}");
        // Stage-left (+x) is its local −x: pan +90 (counter-clockwise from
        // above takes local +y to local −x).
        let pose = aim([0.0, 3.5, 4.2], [0.0, 0.0, 180.0], [3.0, 3.5, 4.2]);
        assert!(close(pose.pan, 90.0), "{pose:?}");
    }

    #[test]
    fn both_solutions_look_the_same_way() {
        let [a, b] = aim_solutions([1.0, 2.0, 4.0], [10.0, -20.0, 35.0], [-2.0, 5.0, 0.5]);
        assert!(close(b.tilt, -a.tilt), "{a:?} {b:?}");
        let turn = (b.pan - a.pan).abs();
        assert!(close(turn, 180.0), "{a:?} {b:?}");
        let da = direction([10.0, -20.0, 35.0], a);
        let db = direction([10.0, -20.0, 35.0], b);
        for i in 0..3 {
            assert!(close(da[i], db[i]), "{da:?} {db:?}");
        }
    }

    #[test]
    fn aim_round_trips_through_direction_for_random_poses() {
        // A cheap deterministic LCG: no rand dependency in this crate's
        // unit tests is needed for a few hundred samples.
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64) / ((1u64 << 53) as f64)
        };
        for _ in 0..500 {
            let rotation = [
                (next() - 0.5) * 180.0,
                (next() - 0.5) * 180.0,
                (next() - 0.5) * 360.0,
            ];
            let position = [next() * 10.0 - 5.0, next() * 8.0, next() * 6.0];
            let target = [next() * 10.0 - 5.0, next() * 8.0, next() * 6.0];
            let d = [
                target[0] - position[0],
                target[1] - position[1],
                target[2] - position[2],
            ];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if len < 0.1 {
                continue;
            }
            let [principal, flipped] = aim_solutions(position, rotation, target);
            for pose in [principal, flipped] {
                let back = direction(rotation, pose);
                for i in 0..3 {
                    assert!(
                        (back[i] - d[i] / len).abs() < 1e-6,
                        "{rotation:?} {position:?} {target:?}: {pose:?} → {back:?}"
                    );
                }
                assert!(pose.pan >= -180.0 && pose.pan <= 180.0);
            }
            assert!(principal.tilt >= 0.0 && principal.tilt <= 180.0);
            assert!(flipped.tilt <= 0.0);
        }
    }

    #[test]
    fn the_nearest_turn_is_chosen_within_the_range() {
        let range = (-270.0, 270.0);
        // Sitting at 200°, a target at −170° is 10° away through 190°.
        assert!(close(nearest_pan(-170.0, 200.0, range), 190.0));
        // Sitting at 0°, the principal solution is nearest.
        assert!(close(nearest_pan(-170.0, 0.0, range), -170.0));
        // A range that cannot hold any turn clamps.
        assert!(close(nearest_pan(100.0, 0.0, (-45.0, 45.0)), 45.0));
        // Always the closest of the in-range equivalents, by brute force.
        for current in [-260.0, -100.0, 0.0, 130.0, 265.0] {
            for pan in [-179.0, -90.0, 0.0, 45.0, 179.0] {
                let chosen = nearest_pan(pan, current, range);
                let best = (-2..=2)
                    .map(|turn| pan + f64::from(turn) * 360.0)
                    .filter(|c| (range.0..=range.1).contains(c))
                    .min_by(|a, b| (a - current).abs().total_cmp(&(b - current).abs()))
                    .unwrap();
                assert!(
                    close(chosen, best),
                    "{pan} from {current} → {chosen}, best {best}"
                );
            }
        }
    }
}
