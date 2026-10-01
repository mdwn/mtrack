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

//! Bring-your-own corpus report: do GDTF meshes come at the size their
//! Model declares? The spec says the declared Length/Width/Height win
//! ("The mesh is explicitly scaled to this dimension"), and Blender DMX
//! scales every mesh per axis to them; this measures how often that
//! scaling actually changes anything in real files.
//!
//! For every Model with a glTF mesh in every GDTF of `tests/gdtf-corpus/`
//! and every GDTF embedded in `tests/mvr-corpus/*.mvr` (both gitignored),
//! the mesh's own bounding box — node transforms applied, read from the
//! glb's JSON chunk (accessor min/max), no rendering — is compared per axis
//! with the declared size, glTF's Y-up turned to GDTF's Z-up as Blender's
//! importer and the 3D view turn it: x-extent ↔ Length, z-extent ↔ Width,
//! y-extent ↔ Height. Nothing is asserted about the files; the
//! distribution is printed.
//!
//! ```sh
//! cargo test --test gdtf_model_size_corpus -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, HashSet};

use mtrack::lighting::gdtf;
use mtrack::lighting::mvr;
use sha2::{Digest, Sha256};

type Mat = [[f64; 4]; 4];

const IDENTITY: Mat = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn numbers(value: Option<&serde_json::Value>) -> Option<Vec<f64>> {
    value?
        .as_array()?
        .iter()
        .map(|v| v.as_f64())
        .collect::<Option<Vec<_>>>()
}

/// A glTF node's local matrix (row-major, column vectors): `matrix`
/// (column-major in the file), or T·R·S.
fn local_matrix(node: &serde_json::Value) -> Mat {
    if let Some(m) = numbers(node.get("matrix")).filter(|m| m.len() == 16) {
        let mut out = [[0.0; 4]; 4];
        for (c, chunk) in m.chunks(4).enumerate() {
            for (r, v) in chunk.iter().enumerate() {
                out[r][c] = *v;
            }
        }
        return out;
    }
    let t = numbers(node.get("translation")).unwrap_or_else(|| vec![0.0; 3]);
    let q = numbers(node.get("rotation")).unwrap_or_else(|| vec![0.0, 0.0, 0.0, 1.0]);
    let s = numbers(node.get("scale")).unwrap_or_else(|| vec![1.0; 3]);
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let mut out = IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = r[i][j] * s[j];
        }
        out[i][3] = t[i];
    }
    out
}

/// The glb's JSON chunk.
fn glb_json(bytes: &[u8]) -> Option<serde_json::Value> {
    if bytes.len() < 20 || &bytes[0..4] != b"glTF" {
        return None;
    }
    let len = u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize;
    if &bytes[16..20] != b"JSON" || bytes.len() < 20 + len {
        return None;
    }
    serde_json::from_slice(&bytes[20..20 + len]).ok()
}

/// The mesh's bounding box in its own (glTF, Y-up) frame, every node's
/// world transform applied to its primitives' POSITION min/max corners.
fn mesh_extent(bytes: &[u8]) -> Option<[f64; 3]> {
    let json = glb_json(bytes)?;
    let nodes = json.get("nodes")?.as_array()?;
    let meshes = json.get("meshes")?.as_array()?;
    let accessors = json.get("accessors")?.as_array()?;
    let scene = json.get("scene").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
    let roots: Vec<usize> = json
        .get("scenes")
        .and_then(|s| s.as_array())
        .and_then(|s| s.get(scene))
        .and_then(|s| s.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|n| {
            n.iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_else(|| (0..nodes.len()).collect());
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    let mut stack: Vec<(usize, Mat, usize)> = roots.into_iter().map(|r| (r, IDENTITY, 0)).collect();
    while let Some((index, parent, depth)) = stack.pop() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        if depth > 64 {
            continue;
        }
        let world = mul(&parent, &local_matrix(node));
        if let Some(mesh) = node
            .get("mesh")
            .and_then(|m| m.as_u64())
            .and_then(|m| meshes.get(m as usize))
        {
            for primitive in mesh
                .get("primitives")
                .and_then(|p| p.as_array())
                .into_iter()
                .flatten()
            {
                let Some(accessor) = primitive
                    .get("attributes")
                    .and_then(|a| a.get("POSITION"))
                    .and_then(|p| p.as_u64())
                    .and_then(|p| accessors.get(p as usize))
                else {
                    continue;
                };
                let (Some(min), Some(max)) =
                    (numbers(accessor.get("min")), numbers(accessor.get("max")))
                else {
                    continue;
                };
                if min.len() < 3 || max.len() < 3 {
                    continue;
                }
                for corner in 0..8 {
                    let p = [
                        if corner & 1 == 0 { min[0] } else { max[0] },
                        if corner & 2 == 0 { min[1] } else { max[1] },
                        if corner & 4 == 0 { min[2] } else { max[2] },
                    ];
                    for axis in 0..3 {
                        let v = world[axis][0] * p[0]
                            + world[axis][1] * p[1]
                            + world[axis][2] * p[2]
                            + world[axis][3];
                        lo[axis] = lo[axis].min(v);
                        hi[axis] = hi[axis].max(v);
                    }
                }
            }
        }
        for child in node
            .get("children")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten()
            .filter_map(|c| c.as_u64())
        {
            stack.push((child as usize, world, depth + 1));
        }
    }
    lo[0]
        .is_finite()
        .then(|| [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]])
}

/// Every GDTF archive of the corpus, deduplicated by content, with a name.
fn corpus_archives() -> Vec<(String, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut out = Vec::new();
    let mut add = |name: String, bytes: Vec<u8>| {
        if seen.insert(Sha256::digest(&bytes).to_vec()) {
            out.push((name, bytes));
        }
    };
    let list = |dir: &std::path::Path, ext: &str| -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map(|d| {
                d.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == ext))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    };
    for path in list(&root.join("gdtf-corpus"), "gdtf") {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        add(name, std::fs::read(&path).expect("readable gdtf"));
    }
    for path in list(&root.join("mvr-corpus"), "mvr") {
        let bytes = std::fs::read(&path).expect("readable mvr");
        let Ok(entries) = mvr::list_gdtf_entries(&bytes) else {
            continue;
        };
        for entry in entries {
            if let Ok(gdtf_bytes) = mvr::read_gdtf_entry(&bytes, &entry) {
                add(entry, gdtf_bytes);
            }
        }
    }
    out
}

struct Row {
    archive: String,
    model: String,
    declared: [f64; 3],
    mesh: [f64; 3],
    /// The worst per-axis factor between the two, ≥ 1.
    worst: f64,
}

#[test]
#[ignore = "bring-your-own corpus: put .gdtf/.mvr files in tests/gdtf-corpus/ and tests/mvr-corpus/ and run with --ignored"]
fn corpus_meshes_against_their_declared_model_size() {
    let archives = corpus_archives();
    assert!(!archives.is_empty(), "no GDTF archives in the corpus");

    let mut rows: Vec<Row> = Vec::new();
    let (mut unparsed, mut no_mesh, mut unmeasured, mut undeclared) = (0, 0, 0, 0);
    let mut undeclared_names: Vec<String> = Vec::new();
    let mut seen_models = 0usize;
    for (archive, bytes) in &archives {
        let Ok(description) = gdtf::parse_archive(bytes) else {
            unparsed += 1;
            continue;
        };
        let available = gdtf::list_model_files(bytes).unwrap_or_default();
        let wanted: HashSet<String> = description
            .models
            .iter()
            .filter_map(|m| m.file.as_ref().map(|f| f.to_lowercase()))
            .filter(|f| available.contains(f))
            .collect();
        let meshes: BTreeMap<String, Vec<u8>> = gdtf::read_assets(bytes, &wanted, None)
            .map(|a| a.models.into_iter().collect())
            .unwrap_or_default();
        for model in &description.models {
            let Some(file) = &model.file else {
                continue;
            };
            seen_models += 1;
            let Some(glb) = meshes.get(&file.to_lowercase()) else {
                no_mesh += 1;
                continue;
            };
            let Some(e) = mesh_extent(glb) else {
                unmeasured += 1;
                continue;
            };
            // glTF Y-up → GDTF Z-up: x ↔ Length, z ↔ Width, y ↔ Height.
            let mesh = [e[0], e[2], e[1]];
            if model.size.iter().any(|s| *s <= 0.0) {
                undeclared += 1;
                undeclared_names.push(format!(
                    "{archive} / {}: declared {:?}, mesh {:.4?}",
                    model.name, model.size, mesh
                ));
                continue;
            }
            let worst = (0..3)
                .map(|i| {
                    let (a, b) = (model.size[i], mesh[i]);
                    if b <= 1e-9 {
                        f64::INFINITY
                    } else {
                        (a / b).max(b / a)
                    }
                })
                .fold(1.0, f64::max);
            rows.push(Row {
                archive: archive.clone(),
                model: model.name.clone(),
                declared: model.size,
                mesh,
                worst,
            });
        }
    }

    let buckets: [(&str, f64, f64); 6] = [
        ("within 2%", 1.0, 1.02),
        ("2–10%", 1.02, 1.10),
        ("10–50%", 1.10, 1.50),
        ("×1.5–×10", 1.50, 10.0),
        ("×10–×500", 10.0, 500.0),
        (">×500 (mm vs m, flat axis)", 500.0, f64::INFINITY),
    ];
    println!("GDTF archives: {} ({} unparsed)", archives.len(), unparsed);
    println!(
        "Models naming a file: {seen_models}; no glb in the archive (3ds only or missing): {no_mesh}; \
         glb without POSITION bounds: {unmeasured}; a declared dimension of 0: {undeclared}"
    );
    // The same fixture type at another revision (another archive hash)
    // repeats its models: count each (archive name, model) once.
    let mut once: HashSet<(String, String)> = HashSet::new();
    rows.retain(|r| once.insert((r.archive.clone(), r.model.clone())));
    println!(
        "Compared (all three declared > 0, distinct): {}",
        rows.len()
    );
    for (label, lo, hi) in buckets {
        let n = rows
            .iter()
            .filter(|r| r.worst >= lo && (r.worst < hi || hi.is_infinite()))
            .count();
        let share = 100.0 * n as f64 / rows.len().max(1) as f64;
        println!("  {label:>28}: {n:5} ({share:5.1}%)");
    }
    // Uniform vs per-axis: a mesh off by the same factor on every axis is
    // a unit or authoring-scale slip; unequal factors change its shape.
    let off: Vec<&Row> = rows.iter().filter(|r| r.worst >= 1.02).collect();
    let uniform = off
        .iter()
        .filter(|r| {
            let f: Vec<f64> = (0..3)
                .map(|i| r.declared[i] / r.mesh[i].max(1e-12))
                .collect();
            let (mn, mx) = f
                .iter()
                .fold((f64::INFINITY, 0.0f64), |(a, b), v| (a.min(*v), b.max(*v)));
            mx / mn < 1.02
        })
        .count();
    println!(
        "Of the {} off by ≥ 2%: {} by one uniform factor, {} by different factors per axis",
        off.len(),
        uniform,
        off.len() - uniform
    );

    let mut types_off: Vec<&str> = rows
        .iter()
        .filter(|r| r.worst >= 1.10)
        .map(|r| r.archive.as_str())
        .collect();
    types_off.sort();
    types_off.dedup();
    let types: HashSet<&str> = rows.iter().map(|r| r.archive.as_str()).collect();
    println!(
        "Fixture types with a measured mesh: {}; with any mesh off by ≥ 10%: {} ({})",
        types.len(),
        types_off.len(),
        types_off.join(", ")
    );

    rows.sort_by(|a, b| b.worst.total_cmp(&a.worst));
    println!("Worst 25:");
    for r in rows.iter().take(25) {
        println!(
            "  ×{:<10.3} {} / {}: declared [{:.4}, {:.4}, {:.4}] mesh [{:.4}, {:.4}, {:.4}]",
            r.worst,
            r.archive,
            r.model,
            r.declared[0],
            r.declared[1],
            r.declared[2],
            r.mesh[0],
            r.mesh[1],
            r.mesh[2]
        );
    }
    println!("Declared 0 (first 10):");
    for line in undeclared_names.iter().take(10) {
        println!("  {line}");
    }
}
