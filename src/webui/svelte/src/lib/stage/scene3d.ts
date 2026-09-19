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

/**
 * The Stage 3D scene (design §16.3): the deck, the fixtures from their rig
 * models, live pan and tilt, translucent beams from live colour and level,
 * focus markers, and an orbit camera. three.js is loaded with this module,
 * which the page imports lazily so the rest of the UI never pays for it.
 *
 * The world is stage space as the venue file spells it: meters,
 * right-handed, Z up, origin downstage-centre, +x stage-left, +y upstage.
 * three.js does not mind which axis is up as long as the camera is told.
 * The room is dark whatever the UI theme: beams are light added to black,
 * which is how a pre-viz reads.
 */

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import type { GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";
import {
  beamLength,
  beamLook,
  deckExtent,
  genericRig,
  poseRotations,
  rootTiltX,
  trayPositions,
  type Mat4,
  type RigModel,
  type SceneryModel,
} from "./rig";
import type {
  FixtureChannels,
  FixtureMetadata,
  FixturePose,
  Vec3,
  VenueMetadata,
} from "../ws/stores";

export type CameraPreset = "foh" | "top" | "side";

const ASSETS = "/api/lighting/assets/";
const BACKGROUND = 0x0b0e13;
const BODY_COLOR = 0x9aa4b2;
const LENS_COLOR = 0x222222;

/**
 * How far and how brightly a beam kind is drawn. A spot throws to the deck;
 * a wash is shorter and fainter; an LED tile or a glow (GDTF `None`,
 * `Glow`, `Rectangle`) is a short haze at the lens, not a throw.
 */
interface BeamStyle {
  /** The longest throw drawn, meters. */
  maxLength: number;
  /** A beam pointing up or level is drawn this long. */
  skyLength: number;
  /** Opacity at the dimmest lit level, and the gain to full. */
  floor: number;
  gain: number;
}

function beamStyle(kind: string): BeamStyle {
  switch (kind.toLowerCase()) {
    case "spot":
    case "pc":
    case "fresnel":
      return { maxLength: 24, skyLength: 4, floor: 0.1, gain: 0.2 };
    case "wash":
      return { maxLength: 8, skyLength: 2.5, floor: 0.05, gain: 0.1 };
    default:
      return { maxLength: 0.9, skyLength: 0.9, floor: 0.05, gain: 0.14 };
  }
}

interface BeamActor {
  node: THREE.Object3D;
  cone: THREE.Mesh;
  material: THREE.MeshBasicMaterial;
  style: BeamStyle;
  angle: number;
}

interface FixtureActor {
  name: string;
  root: THREE.Group;
  /** Everything GPU-side the actor owns, freed when it is rebuilt. */
  owned: { dispose(): void }[];
  /** The node group that pan turns, if any. */
  pan: THREE.Group | null;
  tilt: THREE.Group | null;
  rig: RigModel;
  beams: BeamActor[];
  lens: THREE.MeshStandardMaterial[];
  label: THREE.Sprite;
  placed: boolean;
}

export interface SceneStats {
  fixtures: number;
  placed: number;
  rigs: number;
  generic: number;
  meshes: number;
}

/** What the scenery pass drew and could not. */
export interface SceneryStats {
  objects: number;
  /** Meshes placed in the scene. */
  drawn: number;
  /** Meshes the store skipped (undrawable formats) or that failed to load. */
  skipped: number;
  /** The undrawable formats, e.g. ["3ds"]. */
  formats: string[];
}

/** Fetches and caches rig models and meshes by store path. */
class RigCache {
  private rigs = new Map<string, Promise<RigModel | null>>();
  private gltf = new GLTFLoader();
  private models = new Map<string, Promise<GLTF | null>>();

  rig(path: string): Promise<RigModel | null> {
    let pending = this.rigs.get(path);
    if (!pending) {
      pending = fetch(ASSETS + path)
        .then((r) => (r.ok ? (r.json() as Promise<RigModel>) : null))
        .catch(() => null);
      this.rigs.set(path, pending);
    }
    return pending;
  }

  model(url: string): Promise<GLTF | null> {
    let pending = this.models.get(url);
    if (!pending) {
      pending = this.gltf.loadAsync(url).catch(() => null);
      this.models.set(url, pending);
    }
    return pending;
  }

  /** Forgets every loaded mesh and rig; the scene rebuilds from fresh. */
  clear() {
    this.dispose();
    this.rigs.clear();
  }

  /** Frees the loaded meshes' geometry; clones in the scene share it. */
  dispose() {
    for (const pending of this.models.values()) {
      void pending.then((gltf) =>
        gltf?.scene.traverse((o) => {
          if (o instanceof THREE.Mesh) o.geometry.dispose();
        }),
      );
    }
    this.models.clear();
  }
}

function matrixOf(node: { transform: Mat4 }): THREE.Matrix4 {
  const t = node.transform;
  // prettier-ignore
  return new THREE.Matrix4().set(
    t[0][0], t[0][1], t[0][2], t[0][3],
    t[1][0], t[1][1], t[1][2], t[1][3],
    t[2][0], t[2][1], t[2][2], t[2][3],
    t[3][0], t[3][1], t[3][2], t[3][3],
  );
}

/** A unit cone along −Z with its apex at the origin: scale (r, r, length). */
function beamGeometry(): THREE.ConeGeometry {
  const g = new THREE.ConeGeometry(1, 1, 24, 1, true);
  g.translate(0, -0.5, 0);
  g.rotateX(Math.PI / 2);
  return g;
}

function labelSprite(text: string, color: string): THREE.Sprite {
  const canvas = document.createElement("canvas");
  const scale = 2;
  const ctx = canvas.getContext("2d");
  const font = `${13 * scale}px system-ui, sans-serif`;
  if (ctx) ctx.font = font;
  const width = Math.ceil((ctx?.measureText(text).width ?? 60) + 16 * scale);
  canvas.width = width;
  canvas.height = 24 * scale;
  if (ctx) {
    ctx.font = font;
    ctx.fillStyle = "rgba(0,0,0,0.55)";
    ctx.beginPath();
    ctx.roundRect(0, 0, width, canvas.height, 6 * scale);
    ctx.fill();
    ctx.fillStyle = color;
    ctx.textBaseline = "middle";
    ctx.fillText(text, 8 * scale, canvas.height / 2);
  }
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  const material = new THREE.SpriteMaterial({
    map: texture,
    depthTest: false,
    transparent: true,
  });
  const sprite = new THREE.Sprite(material);
  const h = 0.22;
  sprite.scale.set((h * width) / canvas.height, h, 1);
  sprite.renderOrder = 10;
  return sprite;
}

function primitiveGeometry(
  kind: string,
  size: [number, number, number],
): THREE.BufferGeometry {
  const [l, w, h] = size.map((s) => Math.max(s, 0.02));
  switch (kind.toLowerCase()) {
    case "cylinder": {
      // GDTF's Height runs along Z; three's cylinder along Y.
      const g = new THREE.CylinderGeometry(l / 2, w / 2, h, 24);
      g.rotateX(Math.PI / 2);
      return g;
    }
    case "sphere":
      return new THREE.SphereGeometry(Math.max(l, w, h) / 2, 20, 14);
    default:
      return new THREE.BoxGeometry(l, w, h);
  }
}

export class StageScene {
  private renderer: THREE.WebGLRenderer;
  private scene = new THREE.Scene();
  private camera: THREE.PerspectiveCamera;
  private controls: OrbitControls;
  private deck = new THREE.Group();
  private focus = new THREE.Group();
  private fixtures = new THREE.Group();
  private scenery = new THREE.Group();
  private sceneryGeneration = 0;
  private sceneryOwned: { dispose(): void }[] = [];
  private sceneryStats: SceneryStats | null = null;
  onSceneryStats: ((stats: SceneryStats | null) => void) | null = null;
  private actors = new Map<string, FixtureActor>();
  private cache = new RigCache();
  private cone = beamGeometry();
  private frame = 0;
  private generation = 0;
  private labels = true;
  private channels: Record<string, FixtureChannels> = {};
  private poses: Record<string, FixturePose> = {};
  private extent: [number, number, number, number] = [-4, 4, 0, 6];
  private preset: CameraPreset = "foh";
  /** Primitive geometries shared by (kind, size); freed with the scene. */
  private primitives = new Map<string, THREE.BufferGeometry>();
  private focusOwned: { dispose(): void }[] = [];
  private color = new THREE.Color();
  private dim = new THREE.Color(0x223);
  private stats: SceneStats = {
    fixtures: 0,
    placed: 0,
    rigs: 0,
    generic: 0,
    meshes: 0,
  };
  onStats: ((stats: SceneStats) => void) | null = null;

  constructor(canvas: HTMLCanvasElement) {
    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: false,
      powerPreference: "low-power",
    });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    this.camera = new THREE.PerspectiveCamera(50, 1, 0.05, 200);
    this.camera.up.set(0, 0, 1);
    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.1;
    this.controls.maxPolarAngle = Math.PI / 2 - 0.02;
    this.controls.minDistance = 1;
    this.controls.maxDistance = 80;
    this.scene.add(this.deck, this.focus, this.fixtures, this.scenery);
    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x334455, 1.2));
    const key = new THREE.DirectionalLight(0xffffff, 1.4);
    key.position.set(-4, -6, 10);
    this.scene.add(key);
    this.scene.background = new THREE.Color(BACKGROUND);
    this.scene.fog = new THREE.Fog(BACKGROUND, 30, 90);
    this.setCamera("foh");
    this.buildDeck();
  }

  /** Shows or hides the fixture name labels (focus points keep theirs). */
  setLabels(show: boolean) {
    this.labels = show;
    for (const actor of this.actors.values()) actor.label.visible = show;
  }

  setCamera(preset: CameraPreset) {
    this.preset = preset;
    const [, maxX, minY, maxY] = this.extent;
    const midY = (minY + maxY) / 2;
    const target = new THREE.Vector3(0, midY, 1);
    const reach = Math.max(maxX * 2, maxY - minY) * 1.4;
    if (preset === "foh")
      this.camera.position.set(0, minY - reach, reach * 0.45);
    else if (preset === "top")
      this.camera.position.set(0, midY - 0.01, reach * 1.3);
    else this.camera.position.set(reach, midY, reach * 0.35);
    this.controls.target.copy(target);
    this.controls.update();
  }

  resize(width: number, height: number) {
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / Math.max(1, height);
    this.camera.updateProjectionMatrix();
  }

  /** The live channel values, per fixture. */
  setChannels(channels: Record<string, FixtureChannels>) {
    this.channels = channels;
  }

  /** The live poses, per mover. */
  setPoses(poses: Record<string, FixturePose>) {
    this.poses = poses;
  }

  /** Rebuilds the fixtures and focus markers from the metadata. */
  async setVenue(
    fixtures: Record<string, FixtureMetadata>,
    venue: VenueMetadata | null,
  ): Promise<void> {
    const generation = ++this.generation;
    for (const actor of this.actors.values()) this.dropActor(actor);
    this.actors.clear();
    // Another venue's meshes are no use here: free them.
    this.cache.clear();
    this.focus.clear();
    for (const owned of this.focusOwned) owned.dispose();
    this.focusOwned = [];

    const names = Object.keys(fixtures);
    const placed = names.filter((n) => fixtures[n].position);
    const tray = trayPositions(names.filter((n) => !fixtures[n].position));
    const points: Vec3[] = placed.map((n) => fixtures[n].position as Vec3);
    for (const p of Object.values(venue?.focus_points ?? {})) points.push(p);
    this.extent = deckExtent(points);
    this.buildDeck();
    this.setCamera("foh");

    for (const [name, point] of Object.entries(venue?.focus_points ?? {})) {
      const marker = new THREE.Mesh(
        new THREE.SphereGeometry(0.09, 16, 12),
        new THREE.MeshBasicMaterial({ color: 0xffb454 }),
      );
      marker.position.set(point[0], point[1], point[2]);
      const label = labelSprite(name, "#ffd28a");
      label.position.set(0, 0, 0.3);
      marker.add(label);
      this.focus.add(marker);
      this.focusOwned.push(marker.geometry, marker.material, label.material);
      if (label.material.map) this.focusOwned.push(label.material.map);
    }

    const stats: SceneStats = {
      fixtures: names.length,
      placed: placed.length,
      rigs: 0,
      generic: 0,
      meshes: 0,
    };
    await Promise.all(
      names.map(async (name) => {
        const meta = fixtures[name];
        const rig = meta.rig ? await this.cache.rig(meta.rig) : null;
        if (generation !== this.generation) return;
        const model = rig ?? genericRig(meta.type);
        if (rig) stats.rigs++;
        else stats.generic++;
        const rigDir = meta.rig
          ? meta.rig.slice(0, meta.rig.lastIndexOf("/") + 1)
          : "";
        const actor = await this.buildActor(name, model, rigDir, stats);
        if (generation !== this.generation) {
          // Built for a venue that is gone: free it rather than keep it.
          this.dropActor(actor);
          return;
        }
        const position = meta.position ?? tray[name];
        actor.placed = !!meta.position;
        actor.root.position.set(position[0], position[1], position[2]);
        const rotation = meta.rotation ?? [0, 0, 0];
        // The venue's rotation: degrees about X, Y, Z applied in that
        // order (R = Rz·Ry·Rx, as pointing.rs defines it). three names an
        // Euler by the order its matrices are written, so that is "ZYX".
        actor.root.rotation.set(
          (rotation[0] * Math.PI) / 180,
          (rotation[1] * Math.PI) / 180,
          (rotation[2] * Math.PI) / 180,
          "ZYX",
        );
        this.actors.set(name, actor);
        this.fixtures.add(actor.root);
      }),
    );
    if (generation !== this.generation) return;
    this.stats = stats;
    this.onStats?.(stats);
  }

  /**
   * Draws the venue's scenery from the store: each object at its stage
   * transform, each glTF mesh under it (Y-up turned to Z-up, then the
   * mesh's own transform). `null` clears it.
   */
  async setScenery(path: string | null): Promise<void> {
    const generation = ++this.sceneryGeneration;
    this.scenery.clear();
    for (const owned of this.sceneryOwned) owned.dispose();
    this.sceneryOwned = [];
    this.sceneryStats = null;
    if (!path) {
      this.onSceneryStats?.(null);
      return;
    }
    const model = await fetch(ASSETS + path)
      .then((r) => (r.ok ? (r.json() as Promise<SceneryModel>) : null))
      .catch(() => null);
    if (generation !== this.sceneryGeneration) return;
    if (!model) {
      this.onSceneryStats?.(null);
      return;
    }
    const dir = path.slice(0, path.lastIndexOf("/") + 1);
    const stats: SceneryStats = {
      objects: model.objects.length,
      drawn: 0,
      skipped: 0,
      formats: Object.keys(model.formats).filter((f) => f !== "glb"),
    };
    await Promise.all(
      model.objects.map(async (object) => {
        const group = new THREE.Group();
        group.name = object.name || object.kind;
        matrixOf(object).decompose(
          group.position,
          group.quaternion,
          group.scale,
        );
        stats.skipped += object.skipped.length;
        for (const mesh of object.meshes) {
          const gltf = await this.cache.model(ASSETS + dir + mesh.file);
          if (generation !== this.sceneryGeneration) return;
          if (!gltf) {
            stats.skipped++;
            continue;
          }
          const holder = new THREE.Group();
          matrixOf(mesh).decompose(
            holder.position,
            holder.quaternion,
            holder.scale,
          );
          const clone = gltf.scene.clone(true);
          // glTF is Y-up; the scene is Z-up.
          clone.rotation.x = Math.PI / 2;
          clone.traverse((o) => {
            if (o instanceof THREE.Mesh) {
              const material = new THREE.MeshStandardMaterial({
                color: 0x5c6470,
                roughness: 0.85,
                metalness: 0.05,
              });
              o.material = material;
              this.sceneryOwned.push(material);
            }
          });
          holder.add(clone);
          group.add(holder);
          stats.drawn++;
        }
        if (generation !== this.sceneryGeneration) return;
        this.scenery.add(group);
      }),
    );
    if (generation !== this.sceneryGeneration) return;
    // Scenery can reach past the fixtures (a stage house, a wide deck):
    // widen the floor and the framing to hold it.
    const bounds = new THREE.Box3().setFromObject(this.scenery);
    if (!bounds.isEmpty()) {
      const [minX, maxX, minY, maxY] = this.extent;
      const next: [number, number, number, number] = [
        Math.min(minX, bounds.min.x - 1),
        Math.max(maxX, bounds.max.x + 1),
        Math.min(minY, bounds.min.y - 1),
        Math.max(maxY, bounds.max.y + 1),
      ];
      const half = Math.max(-next[0], next[1]);
      next[0] = -half;
      next[1] = half;
      if (next.some((v, i) => v !== this.extent[i])) {
        this.extent = next;
        this.buildDeck();
        this.setCamera(this.preset);
      }
    }
    this.sceneryStats = stats;
    this.onSceneryStats?.(stats);
  }

  private async buildActor(
    name: string,
    rig: RigModel,
    rigDir: string,
    stats: SceneStats,
  ): Promise<FixtureActor> {
    const root = new THREE.Group();
    root.name = name;
    // The rig hangs in the mounting frame; a static rig is raised so its
    // rest beam runs along +y (see rootTiltX).
    const mount = new THREE.Group();
    mount.rotation.x = rootTiltX(rig);
    root.add(mount);

    const spins: THREE.Group[] = [];
    const lens: THREE.MeshStandardMaterial[] = [];
    const owned: { dispose(): void }[] = [];
    let pan: THREE.Group | null = null;
    let tilt: THREE.Group | null = null;

    for (const node of rig.nodes) {
      // Static transform, then the axis rotation (if any), then the shape
      // and the children.
      const group = new THREE.Group();
      group.name = node.name;
      matrixOf(node).decompose(group.position, group.quaternion, group.scale);
      const spin = new THREE.Group();
      group.add(spin);
      spins.push(spin);
      if (node.role.kind === "pan") pan = spin;
      if (node.role.kind === "tilt") tilt = spin;
      const parent = node.parent === null ? mount : spins[node.parent];
      parent.add(group);

      const isLens = node.role.kind === "beam" || node.role.kind === "cell";
      const material = () => {
        const m = new THREE.MeshStandardMaterial({
          color: isLens ? LENS_COLOR : BODY_COLOR,
          roughness: 0.7,
          metalness: 0.1,
        });
        if (isLens) lens.push(m);
        owned.push(m);
        return m;
      };
      if (node.shape.shape === "primitive") {
        spin.add(
          new THREE.Mesh(
            this.primitive(node.shape.kind, node.shape.size),
            material(),
          ),
        );
      } else if (node.shape.shape === "model") {
        const gltf = await this.cache.model(ASSETS + rigDir + node.shape.file);
        if (gltf) {
          // A clone shares the loaded geometry (owned by the cache, freed
          // with the scene); only the materials are this actor's.
          const mesh = gltf.scene.clone(true);
          // glTF is Y-up; the rig is Z-up.
          mesh.rotation.x = Math.PI / 2;
          mesh.traverse((o) => {
            if (o instanceof THREE.Mesh) o.material = material();
          });
          spin.add(mesh);
          stats.meshes++;
        } else {
          spin.add(
            new THREE.Mesh(
              this.primitive("Cube", [0.15, 0.15, 0.15]),
              material(),
            ),
          );
        }
      }
    }

    const beams: BeamActor[] = rig.beams
      .filter((b) => b.node >= 0 && b.node < spins.length)
      .map((b) => {
        const material = new THREE.MeshBasicMaterial({
          color: 0xffffff,
          transparent: true,
          opacity: 0,
          blending: THREE.AdditiveBlending,
          depthWrite: false,
          side: THREE.DoubleSide,
        });
        const cone = new THREE.Mesh(this.cone, material);
        cone.renderOrder = 5;
        spins[b.node].add(cone);
        owned.push(material);
        return {
          node: spins[b.node],
          cone,
          material,
          style: beamStyle(b.kind),
          angle: b.angle_deg,
        };
      });

    const label = labelSprite(name, "#ffffff");
    label.position.set(0, 0, 0.45);
    label.visible = this.labels;
    root.add(label);
    owned.push(label.material);
    if (label.material.map) owned.push(label.material.map);

    return {
      name,
      root,
      owned,
      pan,
      tilt,
      rig,
      beams,
      lens,
      label,
      placed: true,
    };
  }

  /** Takes an actor out of the scene and frees what it owns. */
  private dropActor(actor: FixtureActor) {
    this.fixtures.remove(actor.root);
    for (const owned of actor.owned) owned.dispose();
    actor.owned = [];
  }

  /** A primitive geometry, one per (kind, size) across the whole rig. */
  private primitive(
    kind: string,
    size: [number, number, number],
  ): THREE.BufferGeometry {
    const key = `${kind.toLowerCase()}:${size.join(",")}`;
    let geometry = this.primitives.get(key);
    if (!geometry) {
      geometry = primitiveGeometry(kind, size);
      this.primitives.set(key, geometry);
    }
    return geometry;
  }

  private buildDeck() {
    this.deck.clear();
    const [minX, maxX, minY, maxY] = this.extent;
    const width = maxX - minX;
    const depth = maxY - minY;
    const midY = (minY + maxY) / 2;
    const deck = new THREE.Mesh(
      new THREE.PlaneGeometry(width, depth),
      new THREE.MeshStandardMaterial({ color: 0x1a2029, roughness: 1 }),
    );
    deck.position.set(0, midY, -0.005);
    this.deck.add(deck);
    const grid = new THREE.GridHelper(
      Math.max(width, depth),
      Math.max(width, depth),
      0x3a4553,
      0x232b36,
    );
    grid.rotation.x = Math.PI / 2;
    grid.position.set(0, midY, 0);
    this.deck.add(grid);
    // The audience edge: a line along y = 0.
    const edge = new THREE.Line(
      new THREE.BufferGeometry().setFromPoints([
        new THREE.Vector3(minX, 0, 0.01),
        new THREE.Vector3(maxX, 0, 0.01),
      ]),
      new THREE.LineBasicMaterial({ color: 0xffb454 }),
    );
    this.deck.add(edge);
    const edgeLabel = labelSprite("AUDIENCE", "#ffd28a");
    edgeLabel.position.set(0, -0.5, 0.2);
    this.deck.add(edgeLabel);
  }

  /** Applies live state and renders one frame. */
  render() {
    const origin = new THREE.Vector3();
    const direction = new THREE.Vector3();
    const rotation = new THREE.Quaternion();
    const { color, dim } = this;
    for (const actor of this.actors.values()) {
      const pose = this.poses[actor.name];
      const { panZ, tiltX } = poseRotations(
        actor.rig,
        pose?.pan ?? 0,
        pose?.tilt ?? 0,
      );
      if (actor.pan) actor.pan.rotation.set(0, 0, panZ);
      if (actor.tilt) actor.tilt.rotation.set(tiltX, 0, 0);
      const look = beamLook(this.channels[actor.name] ?? {});
      const lit = look.strobeOn && look.intensity > 0.02;
      color.setRGB(look.rgb[0], look.rgb[1], look.rgb[2]);
      for (const material of actor.lens) {
        material.emissive.copy(lit ? color : dim);
        material.emissiveIntensity = lit ? 1.5 : 0;
      }
      actor.root.updateMatrixWorld(true);
      // Many lenses on one fixture (a pixel wash) share its light.
      const share = 1 / Math.sqrt(Math.max(1, actor.beams.length));
      for (const beam of actor.beams) {
        beam.node.getWorldPosition(origin);
        direction
          .set(0, 0, -1)
          .applyQuaternion(beam.node.getWorldQuaternion(rotation));
        const { length } = beamLength(
          [origin.x, origin.y, origin.z],
          [direction.x, direction.y, direction.z],
          beam.style,
        );
        const radius = length * Math.tan((beam.angle * Math.PI) / 360);
        beam.cone.scale.set(radius, radius, length);
        beam.material.color.copy(lit ? color : dim);
        beam.material.opacity = lit
          ? (beam.style.floor + beam.style.gain * look.intensity) * share
          : 0.03 * share;
        beam.cone.visible = actor.placed || lit;
      }
    }
    this.controls.update();
    this.renderer.render(this.scene, this.camera);
    this.frame++;
  }

  get framesRendered(): number {
    return this.frame;
  }

  get lastStats(): SceneStats {
    return this.stats;
  }

  dispose() {
    for (const actor of this.actors.values()) this.dropActor(actor);
    this.actors.clear();
    for (const owned of this.sceneryOwned) owned.dispose();
    for (const owned of this.focusOwned) owned.dispose();
    for (const geometry of this.primitives.values()) geometry.dispose();
    this.cache.dispose();
    this.controls.dispose();
    this.renderer.dispose();
    this.cone.dispose();
  }
}
